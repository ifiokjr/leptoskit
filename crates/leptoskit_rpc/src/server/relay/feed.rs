//! Taking commits from the relay's stream, in version order, once each,
//! and from the database log for whatever the stream can't bring.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use async_nats::jetstream;
use futures::StreamExt;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::live::LogEntry;

use super::Batch;
use super::Names;
use super::Relay;
use super::RelayError;
use super::entry_version;

/// How long reading a gap from the database may take before the feed
/// tries again.
const FILL: Duration = Duration::from_secs(10);

/// How long to wait before subscribing again when the relay is down.
const RESUBSCRIBE: Duration = Duration::from_millis(500);

/// The relay's commits, after a version: what a [`Live`](crate::server::Live)
/// with a relay reads instead of [`Db::change_log`].
pub(crate) struct Feed {
	relay: Relay,
	db: Db,
	names: Names,
	/// The newest version handed out.
	version: i64,
	/// Entries ready to hand out, after `version`, up to `queued`.
	ready: VecDeque<LogEntry>,
	/// The newest version in `ready`, or `version` when it's empty.
	queued: i64,
	/// The stream's messages, while subscribed.
	messages: Option<jetstream::consumer::pull::Ordered>,
	/// The stream sequence of the last message taken.
	sequence: u64,
	/// When the database was first seen ahead of the feed with no message
	/// coming.
	behind_since: Option<tokio::time::Instant>,
	/// Entries read from the database rather than the stream.
	filled: Arc<AtomicU64>,
}

impl Feed {
	/// A feed after `version`, from the stream's newest message on, and from
	/// the database for anything before it.
	pub(crate) async fn start(
		relay: Relay,
		db: Db,
		version: i64,
		filled: Arc<AtomicU64>,
	) -> Result<Self, RelayError> {
		let names = relay.names(&db).await?.clone();
		let mut stream = relay.stream(&names).await?;
		let sequence = stream
			.info()
			.await
			.map_err(RelayError::new)?
			.state
			.last_sequence;

		Ok(Self {
			relay,
			db,
			names,
			version,
			ready: VecDeque::new(),
			queued: version,
			messages: None,
			sequence,
			behind_since: None,
			filled,
		})
	}

	/// The newest version handed out.
	pub(crate) fn version(&self) -> i64 {
		self.version
	}

	/// The next entry already to hand.
	pub(crate) fn next_ready(&mut self) -> Option<LogEntry> {
		let entry = self.ready.pop_front()?;
		self.version = entry_version(&entry);
		Some(entry)
	}

	/// The next entry, waiting for one.
	///
	/// # Errors
	///
	/// When neither the stream nor the database can be read; the next call
	/// tries again from the same place.
	pub(crate) async fn next(&mut self) -> Result<LogEntry, DbError> {
		loop {
			if let Some(entry) = self.next_ready() {
				return Ok(entry);
			}

			if self.messages.is_none() {
				if let Ok(messages) = subscribe(&self.relay, &self.names, self.sequence).await {
					self.messages = Some(messages);
				} else {
					// No relay: the database still has every commit.
					self.check_database().await?;
					tokio::time::sleep(self.relay.settings().stall.min(RESUBSCRIBE)).await;
					continue;
				}
			}

			let Some(messages) = &mut self.messages else {
				continue;
			};
			let next = tokio::time::timeout(self.relay.settings().stall, messages.next()).await;

			match next {
				Ok(Some(Ok(message))) => {
					self.behind_since = None;

					if let Ok(info) = message.info() {
						self.sequence = info.stream_sequence;
					}

					// A message that isn't ours to read is skipped: anything it
					// held comes from the database once the next message, or a
					// stall, says so.
					if let Ok(batch) = serde_json::from_slice::<Batch>(&message.payload) {
						self.take(batch).await?;
					}
				}
				// The subscription ended or failed: start a new one after
				// the last message taken.
				Ok(Some(Err(_)) | None) => self.messages = None,
				// Nothing for a while.
				Err(_) => self.check_database().await?,
			}
		}
	}

	/// Queues what `batch` brings that the feed doesn't have, reading what
	/// comes before it from the database first when there's a gap.
	async fn take(&mut self, batch: Batch) -> Result<(), DbError> {
		if batch.to <= self.queued {
			return Ok(());
		}

		if batch.from > self.queued {
			self.fill(batch.from).await?;
		}

		match batch.entries {
			Some(entries) => {
				for entry in entries {
					if entry_version(&entry) > self.queued {
						self.queued = entry_version(&entry);
						self.ready.push_back(entry);
					}
				}
			}
			None => self.fill(batch.to).await?,
		}

		Ok(())
	}

	/// Reads the database log from where the feed is up to `to`.
	async fn fill(&mut self, to: i64) -> Result<(), DbError> {
		let mut after = self.queued;

		while after < to {
			let page = tokio::time::timeout(FILL, self.db.change_log_after(after))
				.await
				.map_err(|_| {
					DbError::Connection(Box::new(RelayError::new("reading a gap timed out")))
				})??;

			if page.is_empty() {
				// The log has nothing up to `to` any more: it was pruned
				// under the reader, so it may have missed changes.
				self.push(LogEntry::Pruned { through: to });
				break;
			}

			for entry in page {
				let version = entry_version(&entry);

				if version > to {
					return Ok(());
				}

				after = version;
				self.push(match entry {
					LogEntry::Pruned { through } => {
						LogEntry::Pruned {
							through: through.min(to),
						}
					}
					commit @ LogEntry::Commit(_) => commit,
				});
			}
		}

		Ok(())
	}

	fn push(&mut self, entry: LogEntry) {
		let version = entry_version(&entry);

		if version > self.queued {
			self.queued = version;
			self.filled.fetch_add(1, Ordering::Relaxed);
			self.ready.push_back(entry);
		}
	}

	/// With no message for a while: when the database has moved on and
	/// stays ahead for another while, no publisher is running, so read it
	/// directly.
	async fn check_database(&mut self) -> Result<(), DbError> {
		let newest = self.db.live_version().await?;

		if newest <= self.queued {
			self.behind_since = None;
			return Ok(());
		}

		let now = tokio::time::Instant::now();
		let since = *self.behind_since.get_or_insert(now);

		if now.duration_since(since) >= self.relay.settings().stall {
			self.behind_since = None;
			self.fill(newest).await?;
		}

		Ok(())
	}
}

/// An ordered consumer of the stream after the message at `sequence`.
async fn subscribe(
	relay: &Relay,
	names: &Names,
	sequence: u64,
) -> Result<jetstream::consumer::pull::Ordered, RelayError> {
	let stream = relay.stream(names).await?;
	let consumer = stream
		.create_consumer(jetstream::consumer::pull::OrderedConfig {
			filter_subject: names.subject.clone(),
			deliver_policy: jetstream::consumer::DeliverPolicy::ByStartSequence {
				start_sequence: sequence + 1,
			},
			..Default::default()
		})
		.await
		.map_err(RelayError::new)?;
	consumer.messages().await.map_err(RelayError::new)
}

impl std::fmt::Debug for Feed {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Feed")
			.field("version", &self.version)
			.field("sequence", &self.sequence)
			.finish_non_exhaustive()
	}
}
