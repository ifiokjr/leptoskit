//! The live relay: one server reads the database's change log and publishes
//! it to a NATS JetStream stream, and every [`Live`](super::Live) takes its
//! commits from there instead of each reading the database.
//!
//! ```ignore
//! let relay = rpc::server::Relay::connect("nats://127.0.0.1:4222").await?;
//!
//! // On the servers that hold sockets (`--role live`):
//! let live = rpc.live().relay(&relay);
//!
//! // On the servers that capture (`--role capture`); several may run it,
//! // and one publishes at a time:
//! let publisher = relay.publish(&db);
//! ```
//!
//! # Why
//!
//! Without a relay, every server holding live sockets reads each commit
//! from the database itself, which is fine up to a few dozen servers. With
//! one, the database is read once per commit however many there are, and
//! channel signals and presence cross between servers over NATS.
//!
//! # What it guarantees
//!
//! - **One publisher.** Every server that runs [`Relay::publish`] competes for
//!   a lease in a JetStream key-value bucket; the holder publishes, and when it
//!   stops renewing, another takes over within the lease.
//! - **In order, once.** Each message holds the commits after one version up to
//!   another, and names both. It is published only if the stream's last message
//!   is still the one the publisher saw, so two publishers can never
//!   interleave, and its version is its `Nats-Msg-Id`, so a retried publish is
//!   dropped as a duplicate.
//! - **No gaps.** A consumer that finds a message starting past where it is,
//!   because it reconnected, the stream's retention dropped what it missed, or
//!   a commit was too big for a message, reads the gap from the database log.
//!   One that hears nothing while the database moves on, because no publisher
//!   is running, reads the database log itself until messages come again.

mod feed;
mod publisher;

use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Read;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use leptoskit_db::live::LogEntry;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::OnceCell;

pub(crate) use self::feed::Feed;
pub use self::publisher::PublisherStats;
pub use self::publisher::RelayPublisher;

/// A connection to the NATS server the live relay runs through, with its
/// settings. Cheap to clone; every clone shares the connection.
#[derive(Clone)]
pub struct Relay {
	inner: Arc<Inner>,
}

struct Inner {
	client: async_nats::Client,
	jetstream: jetstream::Context,
	settings: RelaySettings,
	/// The names for the database the relay carries, once it's been asked.
	names: OnceCell<Names>,
}

/// How the relay keeps its stream and its lease.
#[derive(Clone, Debug)]
pub struct RelaySettings {
	/// How long the stream keeps commits: a consumer further behind reads
	/// from the database instead. The change log's own retention is the
	/// natural choice, and the default: 15 minutes.
	pub retention: Duration,
	/// How long the stream remembers a message's version, to drop a
	/// publish retried after a lost acknowledgement.
	pub duplicate_window: Duration,
	/// How long a publisher's lease lasts without renewal: how soon another
	/// server takes over when the publisher stops.
	pub lease: Duration,
	/// How long a consumer hears nothing before it checks the database for
	/// commits the relay hasn't brought.
	pub stall: Duration,
	/// The largest message the publisher sends; a batch of commits bigger
	/// than this is split, and a single commit bigger than it tells
	/// consumers to read it from the database.
	pub max_message_bytes: usize,
	/// Keep the stream in memory rather than on disk, as tests do.
	pub in_memory: bool,
	/// The name the relay's subjects, stream and bucket use for the
	/// database, instead of the database's own name.
	pub name: Option<String>,
}

impl Default for RelaySettings {
	fn default() -> Self {
		Self {
			retention: Duration::from_mins(15),
			duplicate_window: Duration::from_mins(2),
			lease: Duration::from_secs(5),
			stall: Duration::from_secs(2),
			max_message_bytes: 512 * 1024,
			in_memory: false,
			name: None,
		}
	}
}

/// The subjects, stream and bucket for one database.
#[derive(Clone, Debug)]
pub(crate) struct Names {
	/// Where commits are published: `leptoskit.live.<database>`.
	pub(crate) subject: String,
	/// The stream that keeps them: `LEPTOSKIT_LIVE_<database>`.
	pub(crate) stream: String,
	/// The bucket the publisher's lease lives in.
	pub(crate) bucket: String,
	/// Where channel signals and presence go: `leptoskit.channels.<database>`.
	pub(crate) channels: String,
}

impl Names {
	fn new(database: &str) -> Self {
		let token: String = database
			.chars()
			.map(|c| {
				if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
					c
				} else {
					'_'
				}
			})
			.collect();
		let token = if token.is_empty() {
			"app".to_owned()
		} else {
			token
		};

		Self {
			subject: format!("leptoskit.live.{token}"),
			stream: format!("LEPTOSKIT_LIVE_{token}"),
			bucket: format!("leptoskit_live_{token}"),
			channels: format!("leptoskit.channels.{token}"),
		}
	}
}

/// One message on the relay's stream: the log entries after `from`, up to
/// and including `to`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Batch {
	/// The version the previous message ended at: a consumer at it can take
	/// this one as it is.
	pub(crate) from: i64,
	/// The newest version in it.
	pub(crate) to: i64,
	/// The entries; `None` when they were too big for a message, so a
	/// consumer reads them from the database.
	#[serde(default)]
	pub(crate) entries: Option<Vec<LogEntry>>,
}

/// Couldn't reach, or use, the relay.
#[derive(Debug, thiserror::Error)]
#[error("the live relay: {0}")]
pub struct RelayError(String);

impl RelayError {
	pub(crate) fn new(error: impl std::fmt::Display) -> Self {
		Self(error.to_string())
	}
}

impl Relay {
	/// Connects to the NATS server at `url`, such as `nats://127.0.0.1:4222`,
	/// with the default settings. NATS reconnects by itself when the
	/// connection drops.
	///
	/// # Errors
	///
	/// When `url` isn't one NATS can connect to.
	pub async fn connect(url: &str) -> Result<Self, RelayError> {
		Self::connect_with(url, RelaySettings::default()).await
	}

	/// [`Relay::connect`] with `settings`.
	///
	/// # Errors
	///
	/// As [`Relay::connect`].
	pub async fn connect_with(url: &str, settings: RelaySettings) -> Result<Self, RelayError> {
		let client = async_nats::ConnectOptions::new()
			.name("leptoskit live relay")
			.retry_on_initial_connect()
			.connect(url)
			.await
			.map_err(RelayError::new)?;

		Ok(Self {
			inner: Arc::new(Inner {
				jetstream: jetstream::new(client.clone()),
				client,
				settings,
				names: OnceCell::new(),
			}),
		})
	}

	/// The relay's settings.
	pub fn settings(&self) -> &RelaySettings {
		&self.inner.settings
	}

	/// Starts publishing `db`'s change log, on this server whenever it holds
	/// the lease; see [`RelayPublisher`]. Run it on every server with the
	/// `capture` role, or on one: either way, one publishes at a time.
	pub fn publish(&self, db: &Db) -> RelayPublisher {
		RelayPublisher::start(
			self.clone(),
			db.system("the live relay publishes the change log"),
		)
	}

	pub(crate) fn client(&self) -> &async_nats::Client {
		&self.inner.client
	}

	pub(crate) fn jetstream(&self) -> &jetstream::Context {
		&self.inner.jetstream
	}

	/// The names for `db`: from the settings, or the database's own name.
	pub(crate) async fn names(&self, db: &Db) -> Result<&Names, RelayError> {
		self.inner
			.names
			.get_or_try_init(|| {
				async {
					let name = match &self.inner.settings.name {
						Some(name) => name.clone(),
						None => database_name(db).await?,
					};
					Ok(Names::new(&name))
				}
			})
			.await
	}

	/// The stream that keeps the commits, made if it's missing.
	pub(crate) async fn stream(
		&self,
		names: &Names,
	) -> Result<jetstream::stream::Stream, RelayError> {
		let settings = &self.inner.settings;
		self.inner
			.jetstream
			.get_or_create_stream(jetstream::stream::Config {
				name: names.stream.clone(),
				subjects: vec![names.subject.clone()],
				max_age: settings.retention,
				duplicate_window: settings.duplicate_window,
				storage: if settings.in_memory {
					jetstream::stream::StorageType::Memory
				} else {
					jetstream::stream::StorageType::File
				},
				..Default::default()
			})
			.await
			.map_err(RelayError::new)
	}
}

impl std::fmt::Debug for Relay {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Relay")
			.field("settings", &self.inner.settings)
			.finish_non_exhaustive()
	}
}

/// The database's name, which keeps different databases' relays apart on
/// one NATS server: Postgres's `current_database()`, or `sqlite`.
async fn database_name(db: &Db) -> Result<String, RelayError> {
	if db.dialect() == Dialect::Sqlite {
		return Ok("sqlite".to_owned());
	}

	let rows = db
		.fetch(Statement {
			sql: "SELECT current_database()".to_owned(),
			args: Vec::new(),
		})
		.await
		.map_err(RelayError::new)?;

	match rows.first().and_then(|row| row.first()) {
		Some(Value::Text(name)) => Ok(name.clone()),
		_ => Err(RelayError::new("the database didn't say its name")),
	}
}

/// The version a log entry brings a reader to.
pub(crate) fn entry_version(entry: &LogEntry) -> i64 {
	match entry {
		LogEntry::Commit(commit) => commit.version,
		LogEntry::Pruned { through } => *through,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_are_valid_subjects_streams_and_buckets() {
		let names = Names::new("my app.db");
		assert_eq!(names.subject, "leptoskit.live.my_app_db");
		assert_eq!(names.stream, "LEPTOSKIT_LIVE_my_app_db");
		assert_eq!(names.bucket, "leptoskit_live_my_app_db");
		assert_eq!(names.channels, "leptoskit.channels.my_app_db");
		assert_eq!(Names::new("").subject, "leptoskit.live.app");
	}

	#[test]
	fn a_batch_without_entries_says_so() {
		let batch: Batch = serde_json::from_str(r#"{"from":3,"to":9}"#).unwrap();
		assert_eq!((batch.from, batch.to), (3, 9));
		assert!(batch.entries.is_none());
	}
}
