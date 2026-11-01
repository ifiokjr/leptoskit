//! Publishing the change log to the relay's stream, from whichever server
//! holds the lease.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use async_nats::jetstream;
use async_nats::jetstream::context::PublishErrorKind;
use async_nats::jetstream::kv;
use leptoskit_db::Db;
use leptoskit_db::live::ChangeLog;
use leptoskit_db::live::LogEntry;
use tokio::sync::watch;

use super::Batch;
use super::Names;
use super::Relay;
use super::RelayError;
use super::entry_version;
use super::instance_id;

/// The key the lease is held under.
const LEASE_KEY: &str = "publisher";

/// How long to wait before trying again after the relay or the database
/// failed.
const RETRY: Duration = Duration::from_millis(500);

/// Publishes the change log to the relay while this server holds the
/// lease, from [`Relay::publish`]. Dropping it, or [`stop`](Self::stop),
/// gives the lease up, so another server takes over at once.
pub struct RelayPublisher {
	stop: watch::Sender<bool>,
	stats: Arc<Stats>,
	task: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Default)]
struct Stats {
	leading: AtomicBool,
	following: AtomicBool,
	commits: AtomicU64,
	messages: AtomicU64,
	version: AtomicI64,
	leases: AtomicU64,
}

/// What a [`RelayPublisher`] has done.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PublisherStats {
	/// Whether this server holds the lease and publishes.
	pub leading: bool,
	/// Whether it holds the lease and is reading the change log from where
	/// the stream ends: every commit from now on will be published.
	pub following: bool,
	/// Log entries published, by this server.
	pub commits: u64,
	/// Messages published, by this server.
	pub messages: u64,
	/// The newest version this server published.
	pub version: i64,
	/// How many times this server took the lease.
	pub leases: u64,
}

impl RelayPublisher {
	pub(super) fn start(relay: Relay, db: Db) -> Self {
		let (stop, stopped) = watch::channel(false);
		let stats = Arc::new(Stats::default());
		let task = tokio::spawn(run(relay, db, Arc::clone(&stats), stopped));
		Self {
			stop,
			stats,
			task: Some(task),
		}
	}

	/// What it has done so far.
	pub fn stats(&self) -> PublisherStats {
		PublisherStats {
			leading: self.stats.leading.load(Ordering::Relaxed),
			following: self.stats.following.load(Ordering::Relaxed),
			commits: self.stats.commits.load(Ordering::Relaxed),
			messages: self.stats.messages.load(Ordering::Relaxed),
			version: self.stats.version.load(Ordering::Relaxed),
			leases: self.stats.leases.load(Ordering::Relaxed),
		}
	}

	/// Stops publishing and gives the lease up.
	pub fn stop(&self) {
		let _ = self.stop.send(true);
	}

	/// [`stop`](Self::stop), and waits until the lease is given up.
	pub async fn shutdown(mut self) {
		self.stop();

		if let Some(task) = self.task.take() {
			let _ = task.await;
		}
	}
}

impl Drop for RelayPublisher {
	fn drop(&mut self) {
		let _ = self.stop.send(true);
	}
}

impl std::fmt::Debug for RelayPublisher {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("RelayPublisher")
			.field("stats", &self.stats())
			.finish_non_exhaustive()
	}
}

/// Competes for the lease until stopped, publishing while it holds it.
async fn run(relay: Relay, db: Db, stats: Arc<Stats>, mut stopped: watch::Receiver<bool>) {
	let instance = instance_id();

	while !*stopped.borrow() {
		let outcome = async {
			let names = relay.names(&db).await?.clone();
			let stream = relay.stream(&names).await?;
			let bucket = lease_bucket(&relay, &names).await?;
			Ok::<_, RelayError>((names, stream, bucket))
		}
		.await;

		let Ok((names, stream, bucket)) = outcome else {
			pause(&mut stopped, RETRY).await;
			continue;
		};

		match bucket.create(LEASE_KEY, instance.clone().into()).await {
			Ok(revision) => {
				stats.leading.store(true, Ordering::Relaxed);
				stats.leases.fetch_add(1, Ordering::Relaxed);
				let lease = Lease {
					bucket: &bucket,
					instance: &instance,
					revision,
				};
				lead(&relay, &db, &names, &stream, lease, &stats, &mut stopped).await;
				stats.following.store(false, Ordering::Relaxed);
				stats.leading.store(false, Ordering::Relaxed);
			}
			// Someone else holds it: try again before it would expire.
			Err(error) if error.kind() == kv::CreateErrorKind::AlreadyExists => {
				pause(&mut stopped, relay.settings().lease / 3).await;
			}
			Err(_) => pause(&mut stopped, RETRY).await,
		}
	}
}

/// The lease this server holds.
struct Lease<'a> {
	bucket: &'a kv::Store,
	instance: &'a str,
	revision: u64,
}

impl Lease<'_> {
	/// Renews it, or says it's been lost.
	async fn renew(&mut self) -> bool {
		match self
			.bucket
			.update(LEASE_KEY, self.instance.to_owned().into(), self.revision)
			.await
		{
			Ok(revision) => {
				self.revision = revision;
				true
			}
			Err(_) => false,
		}
	}

	/// Gives it up, so another server needn't wait for it to expire.
	async fn release(&self) {
		let _ = self
			.bucket
			.delete_expect_revision(LEASE_KEY, Some(self.revision))
			.await;
	}
}

/// How a stretch of publishing ended.
enum Ended {
	/// Asked to stop: give the lease up.
	Stopped,
	/// The lease was lost: another server publishes now.
	Lost,
	/// The relay or the database failed, or another server published:
	/// start again from what the stream holds.
	Resync,
}

/// A server holding the lease, and what it publishes with.
struct Leading<'a> {
	relay: &'a Relay,
	db: &'a Db,
	names: &'a Names,
	stream: &'a jetstream::stream::Stream,
	stats: &'a Stats,
	lease: Lease<'a>,
	renew: tokio::time::Interval,
	stopped: &'a mut watch::Receiver<bool>,
}

/// Publishes while the lease holds: from the stream's newest message on,
/// each batch the log read fetched together.
async fn lead(
	relay: &Relay,
	db: &Db,
	names: &Names,
	stream: &jetstream::stream::Stream,
	lease: Lease<'_>,
	stats: &Stats,
	stopped: &mut watch::Receiver<bool>,
) {
	let mut renew = tokio::time::interval(relay.settings().lease / 3);
	renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
	renew.tick().await;
	let mut leading = Leading {
		relay,
		db,
		names,
		stream,
		stats,
		lease,
		renew,
		stopped,
	};

	loop {
		match leading.follow().await {
			Ended::Stopped => break,
			Ended::Lost => return,
			Ended::Resync => {
				if !leading.wait(RETRY).await {
					break;
				}
			}
		}
	}

	leading.lease.release().await;
}

impl Leading<'_> {
	/// Where the stream ends, as its last message's sequence and version,
	/// and the change log from there.
	async fn start(&self) -> Option<(u64, i64, ChangeLog)> {
		let last = last_published(self.stream, self.names).await.ok()?;
		let (sequence, version) = if let Some(last) = last {
			last
		} else {
			let mut stream = self.stream.clone();
			let sequence = stream.info().await.ok()?.state.last_sequence;
			(sequence, self.db.live_version().await.ok()?)
		};
		let log = self.db.change_log(Some(version)).await.ok()?;
		self.stats.version.store(version, Ordering::Relaxed);
		self.stats.following.store(true, Ordering::Relaxed);
		Some((sequence, version, log))
	}

	/// Publishes from where the stream ends until something stops it.
	async fn follow(&mut self) -> Ended {
		let Some((mut sequence, mut version, mut log)) = self.start().await else {
			return Ended::Resync;
		};

		loop {
			let next = tokio::select! {
				biased;
				_ = self.stopped.changed() => return Ended::Stopped,
				_ = self.renew.tick() => {
					if self.lease.renew().await {
						continue;
					}
					return Ended::Lost;
				}
				next = log.next() => next,
			};

			let Ok(entry) = next else {
				return Ended::Resync;
			};
			let mut entries = vec![entry];

			while let Some(Ok(entry)) = log.next_ready() {
				entries.push(entry);
			}

			for batch in batches(version, entries, self.relay.settings().max_message_bytes) {
				let Ok(published) = publish(self.relay, self.names, &batch, sequence).await else {
					return Ended::Resync;
				};
				sequence = published;
				version = batch.to;
				let count = batch.entries.as_ref().map_or(1, Vec::len);
				self.stats
					.commits
					.fetch_add(count as u64, Ordering::Relaxed);
				self.stats.messages.fetch_add(1, Ordering::Relaxed);
				self.stats.version.store(version, Ordering::Relaxed);
			}
		}
	}

	/// Waits `delay`, renewing the lease; `false` when stopped or the lease
	/// is lost.
	async fn wait(&mut self, delay: Duration) -> bool {
		wait_holding(&mut self.lease, &mut self.renew, self.stopped, delay).await
	}
}

/// Waits `delay`, renewing the lease; `false` when stopped or the lease is
/// lost.
async fn wait_holding(
	lease: &mut Lease<'_>,
	renew: &mut tokio::time::Interval,
	stopped: &mut watch::Receiver<bool>,
	delay: Duration,
) -> bool {
	let wake = tokio::time::sleep(delay);
	tokio::pin!(wake);

	loop {
		tokio::select! {
			() = &mut wake => return !*stopped.borrow(),
			_ = stopped.changed() => return false,
			_ = renew.tick() => {
				if !lease.renew().await {
					return false;
				}
			}
		}
	}
}

/// The stream's last message: its sequence and the version it ended at.
async fn last_published(
	stream: &jetstream::stream::Stream,
	names: &Names,
) -> Result<Option<(u64, i64)>, RelayError> {
	match stream.get_last_raw_message_by_subject(&names.subject).await {
		Ok(message) => {
			let batch: Batch = serde_json::from_slice(&message.payload).map_err(RelayError::new)?;
			Ok(Some((message.sequence, batch.to)))
		}
		Err(error)
			if error.kind() == jetstream::stream::LastRawMessageErrorKind::NoMessageFound =>
		{
			Ok(None)
		}
		Err(error) => Err(RelayError::new(error)),
	}
}

/// Publishes `batch` only if the stream's last message is still
/// `sequence`, and returns its own.
async fn publish(
	relay: &Relay,
	names: &Names,
	batch: &Batch,
	sequence: u64,
) -> Result<u64, RelayError> {
	let payload = serde_json::to_vec(batch).map_err(RelayError::new)?;
	let ack = relay
		.jetstream()
		.send_publish(
			names.subject.clone(),
			jetstream::message::PublishMessage::build()
				.payload(payload.into())
				.message_id(batch.to.to_string())
				.expected_last_sequence(sequence),
		)
		.await
		.map_err(RelayError::new)?
		.await
		.map_err(|error| {
			match error.kind() {
				PublishErrorKind::WrongLastSequence => {
					RelayError::new("another server published first")
				}
				_ => RelayError::new(error),
			}
		})?;

	if ack.duplicate {
		return Err(RelayError::new("another server published this already"));
	}

	Ok(ack.sequence)
}

/// `entries`, read after `from`, as messages no bigger than `limit`: one
/// alone that's bigger says to read it from the database.
fn batches(from: i64, entries: Vec<LogEntry>, limit: usize) -> Vec<Batch> {
	let mut batches = Vec::new();
	let mut current: Vec<LogEntry> = Vec::new();
	let mut size = 0;
	let mut start = from;

	for entry in entries {
		let bytes = serde_json::to_vec(&entry).map_or(usize::MAX, |bytes| bytes.len());

		if !current.is_empty() && size + bytes > limit {
			let to = current.last().map_or(start, entry_version);
			batches.push(Batch {
				from: start,
				to,
				entries: Some(std::mem::take(&mut current)),
			});
			start = to;
			size = 0;
		}

		if bytes > limit {
			let to = entry_version(&entry);
			batches.push(Batch {
				from: start,
				to,
				entries: None,
			});
			start = to;
			continue;
		}

		size += bytes;
		current.push(entry);
	}

	if let Some(last) = current.last() {
		batches.push(Batch {
			from: start,
			to: entry_version(last),
			entries: Some(current),
		});
	}

	batches
}

/// The bucket the lease lives in, made if it's missing: one value, gone
/// once its holder stops renewing it.
async fn lease_bucket(relay: &Relay, names: &Names) -> Result<kv::Store, RelayError> {
	if let Ok(bucket) = relay.jetstream().get_key_value(&names.bucket).await {
		return Ok(bucket);
	}

	relay
		.jetstream()
		.create_key_value(kv::Config {
			bucket: names.bucket.clone(),
			history: 1,
			max_age: relay.settings().lease,
			storage: jetstream::stream::StorageType::Memory,
			..Default::default()
		})
		.await
		.map_err(RelayError::new)
}

/// Waits `delay`, or until stopped.
async fn pause(stopped: &mut watch::Receiver<bool>, delay: Duration) {
	tokio::select! {
		() = tokio::time::sleep(delay) => {}
		_ = stopped.changed() => {}
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_db::live::Commit;

	use super::*;

	fn commit(version: i64, body: usize) -> LogEntry {
		LogEntry::Commit(Commit {
			version,
			committed_at: chrono::Utc::now(),
			actor: None,
			origin: "x".repeat(body),
			changes: Vec::new(),
			lsn: None,
		})
	}

	#[test]
	fn batches_chain_and_split_by_size() {
		let entries = vec![commit(2, 10), commit(3, 10), commit(5, 10)];
		let size = serde_json::to_vec(&entries[0]).unwrap().len();
		let batches = batches(1, entries, size * 2);
		let spans: Vec<(i64, i64, Option<usize>)> = batches
			.iter()
			.map(|batch| (batch.from, batch.to, batch.entries.as_ref().map(Vec::len)))
			.collect();
		assert_eq!(spans, [(1, 3, Some(2)), (3, 5, Some(1))]);
	}

	#[test]
	fn a_commit_too_big_for_a_message_is_read_from_the_database() {
		let batches = batches(
			1,
			vec![commit(2, 10), commit(3, 5_000), commit(4, 10)],
			1_000,
		);
		let spans: Vec<(i64, i64, Option<usize>)> = batches
			.iter()
			.map(|batch| (batch.from, batch.to, batch.entries.as_ref().map(Vec::len)))
			.collect();
		assert_eq!(spans, [(1, 2, Some(1)), (2, 3, None), (3, 4, Some(1))]);
	}

	#[test]
	fn pruned_markers_travel_too() {
		let batches = batches(0, vec![LogEntry::Pruned { through: 40 }], 1_000);
		assert_eq!((batches[0].from, batches[0].to), (0, 40));
	}
}
