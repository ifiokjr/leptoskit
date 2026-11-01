//! Replication capture, Postgres only: decoding the write-ahead log into the
//! change log, for `[db] live_capture = "replication"`.
//!
//! Migrations then give live tables no capture triggers. They publish them
//! instead, whole, with `REPLICA IDENTITY FULL`, so an update or delete
//! carries the row as it was. One server at a time captures, holding a
//! session-scoped advisory lock on a connection of its own, and every step
//! of capturing happens on that connection: when it drops, the lock goes
//! with it and another server takes over. The capturing server:
//!
//! 1. holds a logical replication slot, `leptoskit_live_<database>`, made with
//!    the built-in `pgoutput` plugin if it's missing;
//! 2. reads committed transactions from it with
//!    `pg_logical_slot_peek_binary_changes`, which leaves the slot where it
//!    was;
//! 3. writes each one that touched a live table to `live.commits` and
//!    `live.changes`, taking versions under the same commit lock triggers use,
//!    in the order the transactions committed, with the commit's write-ahead
//!    log position, then sends `NOTIFY`, all in one transaction;
//! 4. and only once that has committed, moves the slot past what it wrote.
//!
//! A server that stops between the last two steps leaves the slot behind
//! what the log holds; whoever captures next skips every transaction at or
//! before the newest recorded position, so nothing is recorded twice. A
//! transaction that rolled back never reaches the write-ahead log's decoded
//! output at all.
//!
//! A transaction's actor and origin are settings, which logical decoding
//! can't see, so [`Tx::commit`](crate::Tx::commit) emits a transactional
//! logical message with them when the database publishes live tables.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptoskit_db_schema::Publication;
use leptoskit_db_schema::PublishedTable;
use leptoskit_db_schema::TableName;
use leptoskit_db_schema::live;
use tokio::sync::watch;
use uuid::Uuid;

use super::pgoutput;
use super::pgoutput::Datum;
use super::pgoutput::Message;
use crate::Db;
use crate::DbError;
use crate::Read;
use crate::Result;
use crate::postgres::driver_error;

/// How replication capture paces itself.
///
/// ```ignore
/// let capture = db.capture_changes_with(
/// 	CaptureSettings::default().poll(Duration::from_millis(50)),
/// );
/// ```
#[derive(Clone, Debug)]
pub struct CaptureSettings {
	poll: Duration,
	standby: Duration,
	idle: Duration,
	changes_per_read: u32,
	faults: Faults,
}

impl Default for CaptureSettings {
	fn default() -> Self {
		Self {
			poll: Duration::from_millis(100),
			standby: Duration::from_secs(1),
			idle: Duration::from_secs(10),
			changes_per_read: 10_000,
			faults: Faults::default(),
		}
	}
}

impl CaptureSettings {
	/// How long the capturing server waits before reading the slot again
	/// when the last read found nothing: the most a change waits to be
	/// recorded, beyond the time to record it. A tenth of a second unless
	/// set.
	#[must_use]
	pub fn poll(mut self, poll: Duration) -> Self {
		self.poll = poll.max(Duration::from_millis(1));
		self
	}

	/// How often a server that isn't capturing checks whether it can take
	/// over: how long capture pauses when the capturing server dies, once
	/// Postgres notices its connection is gone. A second unless set.
	#[must_use]
	pub fn standby(mut self, standby: Duration) -> Self {
		self.standby = standby.max(Duration::from_millis(1));
		self
	}

	/// How often a database that captures with triggers is checked for a
	/// switch to replication, and for a slot a switch back left behind. Ten
	/// seconds unless set.
	#[must_use]
	pub fn idle(mut self, idle: Duration) -> Self {
		self.idle = idle.max(Duration::from_millis(1));
		self
	}

	/// The most changes one read of the slot decodes, rounded up to whole
	/// transactions. Ten thousand unless set.
	#[must_use]
	pub fn changes_per_read(mut self, changes: u32) -> Self {
		self.changes_per_read = changes.max(1);
		self
	}

	/// Makes the next `count` writes to the change log fail just before
	/// they commit, as if the server died then. For tests.
	#[doc(hidden)]
	#[must_use]
	pub fn fail_writes(self, count: usize) -> Self {
		self.faults.fail_writes.store(count, Ordering::SeqCst);
		self
	}

	/// Makes the next `count` moves of the slot not happen, as if the server
	/// died after recording what it read. For tests.
	#[doc(hidden)]
	#[must_use]
	pub fn skip_advances(self, count: usize) -> Self {
		self.faults.skip_advances.store(count, Ordering::SeqCst);
		self
	}
}

/// Failures tests ask for.
#[derive(Clone, Debug, Default)]
struct Faults {
	fail_writes: Arc<AtomicUsize>,
	skip_advances: Arc<AtomicUsize>,
}

impl Faults {
	/// Whether the next of `counter`'s failures is due, using it up.
	fn take(counter: &AtomicUsize) -> bool {
		counter
			.try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
				left.checked_sub(1)
			})
			.is_ok()
	}
}

/// What a server's replication capture has done since it started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureStats {
	/// Whether this server is the one capturing now.
	pub capturing: bool,
	/// The commits it has recorded.
	pub commits: u64,
	/// The write-ahead log position of the newest commit it recorded.
	pub lsn: Option<u64>,
}

#[derive(Default)]
struct Shared {
	capturing: AtomicBool,
	commits: AtomicU64,
	lsn: AtomicU64,
}

/// A server's replication capture, running until it's stopped or dropped:
/// what [`Db::capture_changes`] starts.
pub struct Capture {
	stop: watch::Sender<bool>,
	shared: Arc<Shared>,
	task: Option<tokio::task::JoinHandle<()>>,
}

impl Capture {
	/// What it has done so far.
	pub fn stats(&self) -> CaptureStats {
		let lsn = self.shared.lsn.load(Ordering::Relaxed);

		CaptureStats {
			capturing: self.shared.capturing.load(Ordering::Relaxed),
			commits: self.shared.commits.load(Ordering::Relaxed),
			lsn: (lsn > 0).then_some(lsn),
		}
	}

	/// Asks it to stop, after whatever it's recording now.
	pub fn stop(&self) {
		self.stop.send_replace(true);
	}

	/// Stops it and waits until it has. Its connection closes, so another
	/// server can take over.
	pub async fn shutdown(mut self) {
		self.stop();

		if let Some(task) = self.task.take() {
			let _ = task.await;
		}
	}
}

impl Drop for Capture {
	fn drop(&mut self) {
		self.stop();
	}
}

impl std::fmt::Debug for Capture {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Capture")
			.field("stats", &self.stats())
			.finish_non_exhaustive()
	}
}

/// The change log's state, for metrics and checks: [`Db::live_stats`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveStats {
	/// How the database captures writes to live tables, as its migrations
	/// made it.
	pub capture: crate::LiveCapture,
	/// The newest committed version.
	pub version: i64,
	/// Replication capture: how many bytes of write-ahead log the slot
	/// holds that capture hasn't recorded yet. Postgres keeps them until it
	/// has, so a number that only grows means nothing is capturing. `None`
	/// with triggers, or before the slot exists.
	pub live_capture_lag: Option<u64>,
	/// Replication capture: whether a server is reading the slot right now.
	pub slot_active: bool,
}

impl Db {
	/// Starts replication capture on this server, which records writes to
	/// live tables in the change log when the migrations publish them
	/// (`[db] live_capture = "replication"`), and otherwise waits for them
	/// to. Every server may run it: one at a time captures, and another
	/// takes over within [`CaptureSettings::standby`] when that one dies.
	///
	/// It needs a direct connection, as `LISTEN` does, made by a role with
	/// the `REPLICATION` attribute, to a Postgres with `wal_level =
	/// logical`. While no server captures, the slot keeps the write-ahead
	/// log from being recycled, so stopping it for good means switching back
	/// to triggers.
	///
	/// On SQLite it does nothing.
	pub fn capture_changes(&self) -> Capture {
		self.capture_changes_with(CaptureSettings::default())
	}

	/// [`Db::capture_changes`], paced by `settings`.
	pub fn capture_changes_with(&self, settings: CaptureSettings) -> Capture {
		let (stop, stopped) = watch::channel(false);
		let shared = Arc::new(Shared::default());
		let task = self.postgres_url().map(|url| {
			tokio::spawn(run(
				self.system("capture live changes"),
				url.to_string(),
				settings,
				Arc::clone(&shared),
				stopped,
			))
		});

		Capture { stop, shared, task }
	}

	/// Waits until the change log holds every commit made before the call.
	/// With triggers, and on SQLite, it always does: a commit is recorded as
	/// it happens. With replication capture, a commit is recorded a moment
	/// after it, so this waits until the capturing server has read past
	/// where the write-ahead log was, checking every few milliseconds. It
	/// waits as long as that takes, so bound it with a timeout when nothing
	/// may be capturing.
	///
	/// ```ignore
	/// post.save(&db).await?;
	/// db.live_caught_up().await?;
	/// // Every feed started from here on starts after the save.
	/// let feed = db.changes().await?;
	/// ```
	pub async fn live_caught_up(&self) -> Result<()> {
		if self.postgres_url().is_none() {
			return Ok(());
		}

		let rows = self
			.fetch(crate::Statement {
				sql: format!(
					"SELECT (pg_current_wal_insert_lsn() - '0/0'::pg_lsn)::bigint, \
					 current_database() FROM pg_catalog.pg_publication WHERE pubname = '{}'",
					live::PUBLICATION
				),
				args: Vec::new(),
			})
			.await?;
		let Some((target, database)) = rows.into_iter().next().and_then(|row| {
			let mut row = row.into_iter();
			match (row.next(), row.next()) {
				(Some(crate::Value::Int(target)), Some(crate::Value::Text(database))) => {
					Some((target, database))
				}
				_ => None,
			}
		}) else {
			return Ok(());
		};
		let slot = live::slot_name(&database);

		loop {
			let rows = self
				.fetch(crate::Statement {
					sql: "SELECT (confirmed_flush_lsn - '0/0'::pg_lsn)::bigint >= $1 FROM \
					      pg_catalog.pg_replication_slots WHERE slot_name = $2"
						.to_string(),
					args: vec![crate::Value::Int(target), crate::Value::Text(slot.clone())],
				})
				.await?;

			if matches!(
				rows.first().and_then(|row| row.first()),
				Some(crate::Value::Bool(true))
			) {
				return Ok(());
			}

			tokio::time::sleep(Duration::from_millis(5)).await;
		}
	}

	/// The change log's state: how writes are captured, the newest version,
	/// and, with replication, how far capture lags behind the write-ahead
	/// log.
	pub async fn live_stats(&self) -> Result<LiveStats> {
		let version = super::log::latest_version(self).await?;
		let mut stats = LiveStats {
			capture: crate::LiveCapture::Triggers,
			version,
			live_capture_lag: None,
			slot_active: false,
		};

		let Some(url) = self.postgres_url() else {
			return Ok(stats);
		};
		let client = crate::postgres::session_client(url).await?;
		let mode = mode(&client).await?;
		stats.capture = if mode.published {
			crate::LiveCapture::Replication
		} else {
			crate::LiveCapture::Triggers
		};

		if let Some(slot) = slot(&client, &mode.slot).await? {
			let current = query_i64(
				&client,
				"SELECT (pg_current_wal_lsn() - '0/0'::pg_lsn)::bigint",
			)
			.await?;
			stats.live_capture_lag = Some(
				u64::try_from(current)
					.unwrap_or(0)
					.saturating_sub(slot.confirmed),
			);
			stats.slot_active = slot.active;
		}

		Ok(stats)
	}
}

/// Captures until stopped, reconnecting after failures with a growing
/// delay.
async fn run(
	db: Db,
	url: String,
	settings: CaptureSettings,
	shared: Arc<Shared>,
	mut stop: watch::Receiver<bool>,
) {
	let mut delay = Duration::from_millis(100);
	let mut replicating = false;

	while !*stop.borrow() {
		let outcome = session(&db, &url, &settings, &shared, &mut stop, &mut replicating).await;
		shared.capturing.store(false, Ordering::Relaxed);

		match outcome {
			Ok(()) => delay = Duration::from_millis(100),
			Err(error) => {
				eprintln!("warning: live capture: {error}; trying again in {delay:?}");

				if pause(&mut stop, delay).await {
					break;
				}

				delay = (delay * 2).min(Duration::from_secs(30));
			}
		}
	}

	if replicating {
		eprintln!(
			"warning: live capture stopped on this server. Until a server captures again, \
			 Postgres keeps the write-ahead log the replication slot hasn't read, which grows \
			 without bound; switch `[db] live_capture` back to \"triggers\" to stop capturing for \
			 good"
		);
	}
}

/// Waits for `duration`, or until asked to stop; whether it was.
async fn pause(stop: &mut watch::Receiver<bool>, duration: Duration) -> bool {
	if *stop.borrow() {
		return true;
	}

	tokio::time::timeout(duration, stop.changed()).await.is_ok() || *stop.borrow()
}

/// How the database captures, and what its slot is called.
struct Mode {
	published: bool,
	slot: String,
	logical: bool,
}

async fn mode(client: &tokio_postgres::Client) -> Result<Mode> {
	let row = client
		.query_one(
			"SELECT EXISTS (SELECT FROM pg_catalog.pg_publication WHERE pubname = $1), \
			 current_database(), current_setting('wal_level') = 'logical'",
			&[&live::PUBLICATION],
		)
		.await
		.map_err(driver_error)?;
	let database: String = row.get(1);

	Ok(Mode {
		published: row.get(0),
		slot: live::slot_name(&database),
		logical: row.get(2),
	})
}

/// A replication slot as Postgres reports it.
struct Slot {
	confirmed: u64,
	active: bool,
}

/// The slot named `name` in this database, if there is one.
async fn slot(client: &tokio_postgres::Client, name: &str) -> Result<Option<Slot>> {
	let row = client
		.query_opt(
			"SELECT coalesce((confirmed_flush_lsn - '0/0'::pg_lsn)::bigint, 0), active, database \
			 = current_database(), plugin FROM pg_catalog.pg_replication_slots WHERE slot_name = \
			 $1",
			&[&name],
		)
		.await
		.map_err(driver_error)?;
	let Some(row) = row else {
		return Ok(None);
	};

	let here: Option<bool> = row.get(2);
	let plugin: Option<String> = row.get(3);

	if here != Some(true) || plugin.as_deref() != Some("pgoutput") {
		return Err(DbError::connection(SlotTaken(name.to_string())));
	}

	let confirmed: i64 = row.get(0);
	Ok(Some(Slot {
		confirmed: u64::try_from(confirmed).unwrap_or(0),
		active: row.get(1),
	}))
}

/// A slot with capture's name that belongs to something else.
#[derive(Debug, thiserror::Error)]
#[error(
	"the replication slot `{0}` belongs to another database or plugin; drop it or rename the \
	 database"
)]
struct SlotTaken(String);

/// Replication capture needs `wal_level = logical`.
#[derive(Debug, thiserror::Error)]
#[error(
	"the database publishes live tables for replication capture, but Postgres runs with \
	 `wal_level` below `logical`; set `wal_level = logical` and restart it"
)]
struct NotLogical;

async fn query_i64(client: &tokio_postgres::Client, sql: &str) -> Result<i64> {
	let row = client.query_one(sql, &[]).await.map_err(driver_error)?;
	Ok(row.get(0))
}

/// One connection's work: waits while the database captures with
/// triggers, and once it publishes live tables, takes part in the
/// election and captures while it leads. `Ok` when asked to stop, or when
/// the database stopped publishing.
async fn session(
	db: &Db,
	url: &str,
	settings: &CaptureSettings,
	shared: &Shared,
	stop: &mut watch::Receiver<bool>,
	replicating: &mut bool,
) -> Result<()> {
	let mut client = crate::postgres::session_client(url).await?;
	client
		.batch_execute(
			"SET application_name = 'leptoskit live capture'; SET TimeZone = 'UTC'; SET DateStyle \
			 = 'ISO, YMD'; SET extra_float_digits = 3; SET bytea_output = 'hex'; SET \
			 IntervalStyle = 'postgres'",
		)
		.await
		.map_err(driver_error)?;

	loop {
		let mode = mode(&client).await?;
		*replicating = mode.published;

		if !mode.published {
			forget_slot(db, &client, &mode.slot).await?;

			if pause(stop, settings.idle).await {
				return Ok(());
			}

			continue;
		}

		if !mode.logical {
			return Err(DbError::connection(NotLogical));
		}

		let leads: bool = client
			.query_one("SELECT pg_try_advisory_lock($1)", &[&live::CAPTURE_LOCK])
			.await
			.map_err(driver_error)?
			.get(0);

		if !leads {
			if pause(stop, settings.standby).await {
				return Ok(());
			}

			continue;
		}

		let leader = Leader::start(&mut client, mode.slot, settings, shared).await?;
		shared.capturing.store(true, Ordering::Relaxed);
		return leader.capture(stop).await;
	}
}

/// With triggers, a slot a switch back from replication left behind: what
/// it didn't capture is lost, so a marker tells readers they may have missed
/// changes, then the slot goes, so Postgres stops keeping the write-ahead
/// log for it.
async fn forget_slot(db: &Db, client: &tokio_postgres::Client, name: &str) -> Result<()> {
	let Some(slot) = slot(client, name).await? else {
		return Ok(());
	};

	if slot.active || !super::log::has_log(db).await? {
		return Ok(());
	}

	mark_gap(client).await?;
	client
		.execute("SELECT pg_drop_replication_slot($1)", &[&name])
		.await
		.map_err(driver_error)?;
	Ok(())
}

/// Records a commit that tells every reader behind it that it may have
/// missed changes: what a reader of a pruned log hears.
async fn mark_gap(client: &tokio_postgres::Client) -> Result<()> {
	client
		.query_one(
			&format!(
				"WITH locked AS (SELECT pg_advisory_xact_lock({lock})), marker AS (INSERT INTO \
				 live.commits (version, committed_at, origin) SELECT nextval('live.version_seq'), \
				 clock_timestamp(), $1 FROM locked RETURNING version) SELECT pg_notify($2, \
				 version::text) FROM marker",
				lock = live::COMMIT_LOCK
			),
			&[&super::log::PRUNED, &live::CHANNEL],
		)
		.await
		.map_err(driver_error)?;
	Ok(())
}

/// The capturing server's state.
struct Leader<'a> {
	client: &'a mut tokio_postgres::Client,
	slot: String,
	settings: &'a CaptureSettings,
	shared: &'a Shared,
	/// Where the slot was last confirmed to.
	confirmed: u64,
	/// The commit position of the newest commit the log holds.
	recorded: u64,
	relations: std::collections::HashMap<u32, pgoutput::Relation>,
}

impl<'a> Leader<'a> {
	/// Takes over: makes the slot if there's none, which marks a gap, since
	/// whatever committed before it was made is out of its reach.
	async fn start(
		client: &'a mut tokio_postgres::Client,
		slot_name: String,
		settings: &'a CaptureSettings,
		shared: &'a Shared,
	) -> Result<Self> {
		let confirmed = if let Some(slot) = slot(client, &slot_name).await? {
			slot.confirmed
		} else {
			let lsn: i64 = client
				.query_one(
					"SELECT (lsn - '0/0'::pg_lsn)::bigint FROM \
					 pg_create_logical_replication_slot($1, 'pgoutput')",
					&[&slot_name],
				)
				.await
				.map_err(driver_error)?
				.get(0);
			mark_gap(client).await?;
			u64::try_from(lsn).unwrap_or(0)
		};
		let recorded: Option<i64> = client
			.query_opt(
				"SELECT lsn FROM live.commits WHERE lsn IS NOT NULL ORDER BY version DESC LIMIT 1",
				&[],
			)
			.await
			.map_err(driver_error)?
			.map(|row| row.get(0));

		Ok(Self {
			client,
			slot: slot_name,
			settings,
			shared,
			confirmed,
			recorded: recorded
				.and_then(|lsn| u64::try_from(lsn).ok())
				.unwrap_or(0),
			relations: std::collections::HashMap::new(),
		})
	}

	/// Reads, records, and moves the slot on, until asked to stop.
	async fn capture(mut self, stop: &mut watch::Receiver<bool>) -> Result<()> {
		loop {
			if *stop.borrow() {
				return Ok(());
			}

			let found = self.step().await?;

			if !found && pause(stop, self.settings.poll).await {
				return Ok(());
			}
		}
	}

	/// One read of the slot, up to where the log has been flushed: records
	/// what it found and moves the slot past it. Whether it found anything.
	async fn step(&mut self) -> Result<bool> {
		let target = u64::try_from(
			query_i64(
				self.client,
				"SELECT (pg_current_wal_flush_lsn() - '0/0'::pg_lsn)::bigint",
			)
			.await?,
		)
		.unwrap_or(0);
		let limit = i32::try_from(self.settings.changes_per_read).unwrap_or(i32::MAX);
		let rows = self
			.client
			.query(
				"SELECT data FROM pg_logical_slot_peek_binary_changes($1, $2::text::pg_lsn, $3, \
				 'proto_version', '1', 'publication_names', $4, 'messages', 'true')",
				&[&self.slot, &lsn_text(target), &limit, &live::PUBLICATION],
			)
			.await
			.map_err(driver_error)?;
		let read_all = rows.len() < limit as usize;
		let messages = rows
			.iter()
			.map(|row| pgoutput::decode(row.get::<_, &[u8]>(0)))
			.collect::<Result<Vec<_>>>()?;
		let transactions = self.transactions(messages)?;
		let mut end = transactions
			.last()
			.map_or(0, |transaction| transaction.end_lsn);
		let fresh: Vec<Transaction> = transactions
			.into_iter()
			.filter(|transaction| transaction.commit_lsn > self.recorded)
			.collect();

		if fresh.iter().any(Transaction::matters) {
			self.record(&fresh).await?;
		}

		if read_all {
			end = end.max(target);
		}

		self.advance(end).await?;
		Ok(!rows.is_empty())
	}

	/// Groups messages into committed transactions.
	fn transactions(&mut self, messages: Vec<Message>) -> Result<Vec<Transaction>> {
		let mut done = Vec::new();
		let mut open: Option<Transaction> = None;

		for message in messages {
			match message {
				Message::Begin { xid } => open = Some(Transaction::new(xid)),
				Message::Relation(relation) => {
					self.relations.insert(relation.id, relation);
				}
				Message::Commit {
					commit_lsn,
					end_lsn,
					timestamp,
				} => {
					if let Some(mut transaction) = open.take() {
						transaction.commit_lsn = commit_lsn;
						transaction.end_lsn = end_lsn;
						transaction.committed_at = pgoutput::timestamp(timestamp);
						done.push(transaction);
					}
				}
				Message::Other => {}
				message => {
					let transaction = open.as_mut().ok_or_else(|| DbError::connection(Unframed))?;
					transaction.add(message, &self.relations);
				}
			}
		}

		Ok(done)
	}

	/// Records `transactions`, in order, in one transaction: each that
	/// touched a live table gets the next version, then `NOTIFY` wakes the
	/// readers.
	async fn record(&mut self, transactions: &[Transaction]) -> Result<()> {
		let publication = published(self.client).await?;
		let tx = self.client.transaction().await.map_err(driver_error)?;
		tx.execute("SELECT pg_advisory_xact_lock($1)", &[&live::COMMIT_LOCK])
			.await
			.map_err(driver_error)?;
		let current: i64 = tx
			.query_one("SELECT pg_current_xact_id()::text::bigint", &[])
			.await
			.map_err(driver_error)?
			.get(0);
		let mut newest = None;
		let mut recorded = 0;

		for transaction in transactions
			.iter()
			.filter(|transaction| transaction.matters())
		{
			let (origin, changes) = transaction.changes(&publication);
			let txid = full_xid(current, transaction.xid);
			let version: i64 = tx
				.query_one(
					"INSERT INTO live.commits (version, txid, committed_at, actor, origin, lsn) \
					 VALUES (nextval('live.version_seq'), $1, $2, $3, $4, $5) RETURNING version",
					&[
						&txid,
						&transaction.committed_at,
						&transaction.actor,
						&origin,
						&(transaction.commit_lsn as i64),
					],
				)
				.await
				.map_err(driver_error)?
				.get(0);

			if !changes.is_empty() {
				tx.execute(
					"INSERT INTO live.changes (seq, txid, version, table_name, op, key, old, new, \
					 changed) SELECT nextval('live.change_seq'), $1, $2, c.table_name, c.op, \
					 c.key, c.old, c.new, c.changed FROM jsonb_to_recordset($3::jsonb) AS \
					 c(table_name text, op text, key jsonb, old jsonb, new jsonb, changed jsonb)",
					&[&txid, &version, &serde_json::Value::Array(changes)],
				)
				.await
				.map_err(driver_error)?;
			}

			newest = Some((version, transaction.commit_lsn));
			recorded += 1;
		}

		let Some((version, lsn)) = newest else {
			return Ok(());
		};

		tx.execute(
			"SELECT pg_notify($1, $2)",
			&[&live::CHANNEL, &version.to_string()],
		)
		.await
		.map_err(driver_error)?;

		if Faults::take(&self.settings.faults.fail_writes) {
			return Err(DbError::connection(Injected("a write to the change log")));
		}

		tx.commit().await.map_err(driver_error)?;
		self.recorded = lsn;
		self.shared.commits.fetch_add(recorded, Ordering::Relaxed);
		self.shared.lsn.store(lsn, Ordering::Relaxed);
		Ok(())
	}

	/// Moves the slot to `to`, now that everything before it is recorded.
	async fn advance(&mut self, to: u64) -> Result<()> {
		if to <= self.confirmed {
			return Ok(());
		}

		if Faults::take(&self.settings.faults.skip_advances) {
			return Ok(());
		}

		self.client
			.execute(
				"SELECT pg_replication_slot_advance($1, $2::text::pg_lsn)",
				&[&self.slot, &lsn_text(to)],
			)
			.await
			.map_err(driver_error)?;
		self.confirmed = to;
		Ok(())
	}
}

/// A failure a test asked for.
#[derive(Debug, thiserror::Error)]
#[error("{0} failed, as the test asked")]
struct Injected(&'static str);

/// A change arrived outside a transaction.
#[derive(Debug, thiserror::Error)]
#[error("pgoutput sent a change outside a transaction")]
struct Unframed;

/// What each published table may copy, from the publication's comment.
async fn published(client: &tokio_postgres::Client) -> Result<Vec<PublishedTable>> {
	let comment: Option<String> = client
		.query_opt(
			"SELECT obj_description(oid, 'pg_publication') FROM pg_catalog.pg_publication WHERE \
			 pubname = $1",
			&[&live::PUBLICATION],
		)
		.await
		.map_err(driver_error)?
		.and_then(|row| row.get(0));

	Ok(comment
		.as_deref()
		.and_then(Publication::tables_in_comment)
		.unwrap_or_default())
}

/// A position in the write-ahead log as Postgres writes one: `16/B374D848`.
fn lsn_text(lsn: u64) -> String {
	format!("{:X}/{:X}", lsn >> 32, lsn & 0xffff_ffff)
}

/// The 64-bit ID of a transaction pgoutput names by its 32 bits, which
/// started at most two billion transactions before `current`, the 64-bit
/// ID of one running now.
fn full_xid(current: i64, xid: u32) -> i64 {
	let epoch = current >> 32;
	let low = current & 0xffff_ffff;
	let xid = i64::from(xid);

	if xid > low {
		((epoch - 1) << 32) | xid
	} else {
		(epoch << 32) | xid
	}
}

/// One decoded transaction.
struct Transaction {
	xid: u32,
	commit_lsn: u64,
	end_lsn: u64,
	committed_at: crate::Timestamp,
	actor: Option<Uuid>,
	origin: Option<String>,
	rows: Vec<Row>,
	/// It emptied a published table, so which rows changed isn't known.
	truncated: bool,
}

/// One written row, with the whole row before and after.
struct Row {
	namespace: String,
	table: String,
	op: &'static str,
	old: Option<serde_json::Map<String, serde_json::Value>>,
	new: Option<serde_json::Map<String, serde_json::Value>>,
}

impl Transaction {
	fn new(xid: u32) -> Self {
		Self {
			xid,
			commit_lsn: 0,
			end_lsn: 0,
			committed_at: crate::Timestamp::default(),
			actor: None,
			origin: None,
			rows: Vec::new(),
			truncated: false,
		}
	}

	/// Whether it touched a live table, and so takes a version.
	fn matters(&self) -> bool {
		self.truncated || !self.rows.is_empty()
	}

	fn add(
		&mut self,
		message: Message,
		relations: &std::collections::HashMap<u32, pgoutput::Relation>,
	) {
		let mut row = |relation: u32, op, old: Option<Vec<Datum>>, new: Option<Vec<Datum>>| {
			let Some(relation) = relations.get(&relation) else {
				// Every change follows its table's description; without one,
				// nobody can be told what changed.
				self.truncated = true;
				return;
			};
			let old = old.map(|old| image(relation, &old, None));
			let new = new.map(|new| image(relation, &new, old.as_ref()));
			self.rows.push(Row {
				namespace: relation.namespace.clone(),
				table: relation.name.clone(),
				op,
				old,
				new,
			});
		};

		match message {
			Message::Insert { relation, new } => row(relation, "I", None, Some(new)),
			Message::Update { relation, old, new } => row(relation, "U", old, Some(new)),
			Message::Delete { relation, old } => row(relation, "D", Some(old), None),
			Message::Truncate { .. } => self.truncated = true,
			Message::Logical {
				transactional: true,
				prefix,
				content,
			} if prefix == live::MESSAGE_PREFIX => self.describe(&content),
			_ => {}
		}
	}

	/// Takes the actor and origin from the message the writer emitted.
	fn describe(&mut self, content: &[u8]) {
		let Ok(serde_json::Value::Object(said)) = serde_json::from_slice(content) else {
			return;
		};

		self.actor = said
			.get("actor")
			.and_then(serde_json::Value::as_str)
			.and_then(|actor| Uuid::parse_str(actor).ok());
		self.origin = said
			.get("origin")
			.and_then(serde_json::Value::as_str)
			.map(ToString::to_string);
	}

	/// The commit's origin, and its changes as the change log holds them,
	/// copying only what `publication` lets each table copy. A commit that
	/// emptied a table, or touched one the publication doesn't describe,
	/// becomes a marker that tells readers they may have missed changes.
	fn changes(&self, publication: &[PublishedTable]) -> (String, Vec<serde_json::Value>) {
		let origin = self
			.origin
			.clone()
			.unwrap_or_else(|| live::DEFAULT_ORIGIN.to_string());
		let mut changes = Vec::with_capacity(self.rows.len());

		if self.truncated {
			return (super::log::PRUNED.to_string(), Vec::new());
		}

		for row in &self.rows {
			let Some(table) = described(publication, &row.namespace, &row.table) else {
				return (super::log::PRUNED.to_string(), Vec::new());
			};
			changes.push(row.change(table));
		}

		(origin, changes)
	}
}

/// What the publication says about the table `namespace.name`, which it
/// names without a namespace when the table is in the default one.
fn described<'a>(
	publication: &'a [PublishedTable],
	namespace: &str,
	name: &str,
) -> Option<&'a PublishedTable> {
	let namespaced = TableName::namespaced(namespace, name);
	let plain = TableName::new(name);

	publication
		.iter()
		.find(|table| table.table == namespaced)
		.or_else(|| publication.iter().find(|table| table.table == plain))
}

impl Row {
	/// The change as a capture trigger would have written it.
	fn change(&self, table: &PublishedTable) -> serde_json::Value {
		let whole = self.new.as_ref().or(self.old.as_ref());
		let key: Vec<serde_json::Value> = table
			.key
			.iter()
			.map(|column| {
				whole
					.and_then(|row| row.get(column))
					.cloned()
					.unwrap_or_default()
			})
			.collect();
		let copy = |row: &serde_json::Map<String, serde_json::Value>| {
			let copied: serde_json::Map<String, serde_json::Value> = table
				.columns
				.iter()
				.map(|column| (column.clone(), row.get(column).cloned().unwrap_or_default()))
				.collect();
			serde_json::Value::Object(copied)
		};
		// Every column whose value changed, `secret` ones included, so a
		// query filtering on one still re-runs.
		let changed: Vec<String> = match (&self.old, &self.new) {
			(Some(old), Some(new)) => {
				let mut names: Vec<String> = new
					.iter()
					.filter(|(column, value)| old.get(*column) != Some(value))
					.map(|(column, _)| column.clone())
					.collect();
				names.sort_unstable();
				names
			}
			(None, Some(new)) if self.op == "U" => new.keys().cloned().collect(),
			_ => Vec::new(),
		};

		serde_json::json!({
			"table_name": table.table.to_string(),
			"op": self.op,
			"key": key,
			"old": self.old.as_ref().map(copy),
			"new": self.new.as_ref().map(copy),
			"changed": changed,
		})
	}
}

/// A row as JSON, each column as `to_jsonb` writes it. A large value an
/// update left alone, which isn't sent again, is the one in `before`.
fn image(
	relation: &pgoutput::Relation,
	row: &[Datum],
	before: Option<&serde_json::Map<String, serde_json::Value>>,
) -> serde_json::Map<String, serde_json::Value> {
	relation
		.columns
		.iter()
		.zip(row)
		.map(|(column, datum)| {
			let value = match datum {
				Datum::Null => serde_json::Value::Null,
				Datum::Text(text) => pgoutput::json(column.type_oid, text),
				Datum::Unchanged => {
					before
						.and_then(|before| before.get(&column.name))
						.cloned()
						.unwrap_or_default()
				}
			};
			(column.name.clone(), value)
		})
		.collect()
}

/// What a transaction that said who acts in it, or what made it, runs just
/// before `COMMIT`: when the database publishes live tables and the
/// transaction wrote anything, a transactional logical message with the
/// actor and origin, which replication capture reads alongside its changes.
/// Otherwise it does nothing.
pub(crate) fn describe_commit() -> String {
	format!(
		"SELECT pg_catalog.pg_logical_emit_message(true, '{prefix}', \
		 pg_catalog.json_build_object('actor', nullif(current_setting('{actor}', true), ''), \
		 'origin', nullif(current_setting('{origin}', true), ''))::text) WHERE \
		 pg_catalog.pg_current_xact_id_if_assigned() IS NOT NULL AND EXISTS (SELECT FROM \
		 pg_catalog.pg_publication WHERE pubname = '{publication}')",
		prefix = live::MESSAGE_PREFIX,
		actor = live::ACTOR_SETTING,
		origin = live::ORIGIN_SETTING,
		publication = live::PUBLICATION,
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn positions_and_transaction_ids_read_as_postgres_writes_them() {
		assert_eq!(lsn_text(0x16_B374_D848), "16/B374D848");
		assert_eq!(lsn_text(0), "0/0");
		assert_eq!(full_xid((3 << 32) + 100, 90), (3 << 32) + 90);
		assert_eq!(
			full_xid((3 << 32) + 100, 4_000_000_000),
			(2 << 32) + 4_000_000_000
		);
	}

	#[test]
	fn a_change_copies_only_what_the_publication_allows() {
		let table = PublishedTable {
			table: TableName::new("notes"),
			key: vec!["id".into()],
			columns: vec!["id".into(), "body".into()],
		};
		let row = |pairs: serde_json::Value| pairs.as_object().cloned();
		let update = Row {
			namespace: "public".into(),
			table: "notes".into(),
			op: "U",
			old: row(serde_json::json!({ "id": 1, "body": "a", "token": "x" })),
			new: row(serde_json::json!({ "id": 1, "body": "a", "token": "y" })),
		};
		assert_eq!(
			update.change(&table),
			serde_json::json!({
				"table_name": "notes",
				"op": "U",
				"key": [1],
				"old": { "id": 1, "body": "a" },
				"new": { "id": 1, "body": "a" },
				"changed": ["token"],
			})
		);
		assert!(described(std::slice::from_ref(&table), "public", "notes").is_some());
		assert!(described(&[table], "other", "posts").is_none());
	}
}
