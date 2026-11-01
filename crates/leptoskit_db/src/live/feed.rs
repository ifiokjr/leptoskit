//! Reading the change log as it grows: each subscriber's view of it, and
//! the whole of it for a server's own use.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use leptoskit_db_schema::TableName;
use tokio::sync::watch;

use super::ChangedRows;
use super::Invalidation;
use super::VersionedInvalidation;
use super::log::Change;
use super::log::Commit;
use super::log::LogEntry;
use super::log::RawChange;
use super::log::RawEntry;
use super::log::Reader;
use crate::Db;
use crate::DbError;
use crate::Read;
use crate::Result;
use crate::Value;
use crate::policy::Access;

/// Wakes a database's readers when the change log may have grown: a
/// `NOTIFY` from any server sharing a Postgres database, a commit through
/// this process's SQLite pool, or the listening connection coming back,
/// after which readers catch up on what they missed. Readers always read
/// the log itself; a wake only says when.
pub(crate) struct Hub {
	pulse: watch::Sender<u64>,
	/// Whether the database has the `live` pack, once it's been seen.
	has_log: AtomicBool,
	#[cfg(feature = "postgres")]
	pub(crate) listener: tokio::sync::OnceCell<crate::postgres::Listener>,
}

impl Hub {
	pub(crate) fn new() -> Self {
		Self {
			pulse: watch::channel(0).0,
			has_log: AtomicBool::new(false),
			#[cfg(feature = "postgres")]
			listener: tokio::sync::OnceCell::new(),
		}
	}

	/// Wakes every reader.
	pub(crate) fn wake(&self) {
		self.pulse
			.send_modify(|pulse| *pulse = pulse.wrapping_add(1));
	}

	pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
		self.pulse.subscribe()
	}

	pub(crate) fn has_log(&self) -> bool {
		self.has_log.load(Ordering::Relaxed)
	}

	pub(crate) fn found_log(&self) {
		self.has_log.store(true, Ordering::Relaxed);
	}
}

/// The live changes one handle may know about, read from the change log in
/// version order: [`Db::changes`] starts one at the newest version, and
/// [`Db::changes_since`] resumes one.
///
/// Each change is checked against the handle's `select` policy, as the
/// row was before and after it, so a subscriber learns of a change only
/// when it could read the row on one side of it.
pub struct Feed {
	reader: Reader,
}

impl Feed {
	pub(crate) fn new(reader: Reader) -> Self {
		Self { reader }
	}

	/// The newest version this feed has passed: give it to
	/// [`Db::changes_since`] to carry on from here, such as after a client
	/// reconnects.
	pub fn version(&self) -> i64 {
		self.reader.version()
	}

	/// Waits for the next change this handle may know about.
	///
	/// [`Invalidation::Everything`] says changes may have been missed: the
	/// log was pruned past this feed's version, or a change came from a
	/// table this build doesn't know, as during a rolling deploy.
	///
	/// # Errors
	///
	/// When the log can't be read, such as when the database is
	/// unreachable; the next call reads on from the same place. When
	/// checking a change against the policy fails, the feed moves past it,
	/// so a client should refetch everything.
	pub async fn next(&mut self) -> Result<Invalidation> {
		Ok(self.next_versioned().await?.invalidation)
	}

	/// [`Feed::next`], with the version of the commit it came from, which a
	/// client can resume from or wait for.
	///
	/// # Errors
	///
	/// As for [`Feed::next`].
	pub async fn next_versioned(&mut self) -> Result<VersionedInvalidation> {
		loop {
			let invalidation = match self.reader.next().await? {
				RawEntry::Pruned { .. } => Some(Invalidation::Everything),
				RawEntry::Commit(commit) => self.visible(&commit.changes).await?,
			};

			if let Some(invalidation) = invalidation {
				return Ok(VersionedInvalidation {
					version: self.reader.version(),
					invalidation,
				});
			}
		}
	}

	/// The changes in one commit this handle may know about, by table, or
	/// `None` when it may know of none.
	async fn visible(&self, changes: &[RawChange]) -> Result<Option<Invalidation>> {
		let mut tables: Vec<(&TableName, Vec<&RawChange>)> = Vec::new();

		for change in changes {
			match tables.iter_mut().find(|(table, _)| *table == &change.table) {
				Some((_, rows)) => rows.push(change),
				None => tables.push((&change.table, vec![change])),
			}
		}

		let mut invalidated = Vec::new();

		for (table, rows) in tables {
			let Some(entry) = crate::table::live_entry(table) else {
				// No policy to check it against, so nobody can be told what
				// changed, only that something did.
				return Ok(Some(Invalidation::Everything));
			};
			let decoded = Decoded::new(entry, self.reader.db().dialect(), &rows)?;
			let visible = self.readable(entry, &decoded).await?;
			let mut seen = HashSet::new();
			let keys: Vec<Vec<String>> = decoded
				.keys
				.iter()
				.zip(visible)
				.filter(|(_, visible)| *visible)
				.map(|(key, _)| key.iter().map(key_text).collect::<Vec<_>>())
				.filter(|key| seen.insert(key.clone()))
				.collect();

			if !keys.is_empty() {
				invalidated.push(ChangedRows {
					table: table.to_string(),
					keys,
				});
			}
		}

		Ok((!invalidated.is_empty()).then_some(Invalidation::Rows(invalidated)))
	}

	/// Whether this handle may read each change's row, before or after.
	async fn readable(
		&self,
		entry: &crate::table::TableEntry,
		decoded: &Decoded,
	) -> Result<Vec<bool>> {
		let db = self.reader.db();
		let images: Vec<&[Value]> = decoded.images.iter().map(Vec::as_slice).collect();
		let access: &Access = db.access();
		let Some(statements) = entry.visible_images(access, db.dialect(), &images)? else {
			return Ok(vec![true; decoded.keys.len()]);
		};
		let mut visible = vec![false; decoded.keys.len()];

		for statement in statements {
			for row in db.fetch(statement).await? {
				let owner = match row.first() {
					Some(Value::Int(number)) => {
						usize::try_from(*number)
							.ok()
							.and_then(|number| decoded.owners.get(number))
					}
					_ => None,
				};

				match owner {
					Some(owner) => visible[*owner] = true,
					None => {
						return Err(DbError::Decode(crate::DecodeError::custom(
							"a live change's policy check returned no image number",
						)));
					}
				}
			}
		}

		Ok(visible)
	}
}

impl std::fmt::Debug for Feed {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Feed")
			.field("version", &self.version())
			.finish_non_exhaustive()
	}
}

/// One table's changes in a commit, read back as values: each change's
/// key, and its images numbered in order with the change each belongs to.
struct Decoded {
	keys: Vec<Vec<Value>>,
	images: Vec<Vec<Value>>,
	owners: Vec<usize>,
}

impl Decoded {
	fn new(
		entry: &crate::table::TableEntry,
		dialect: leptoskit_db_schema::Dialect,
		rows: &[&RawChange],
	) -> Result<Self> {
		let definition = (entry.definition)();
		let columns = entry.subject.columns;
		let mut decoded = Self {
			keys: Vec::with_capacity(rows.len()),
			images: Vec::with_capacity(rows.len() * 2),
			owners: Vec::with_capacity(rows.len() * 2),
		};

		for (index, change) in rows.iter().enumerate() {
			decoded.keys.push(super::images::logged_key(
				&definition,
				&change.key,
				dialect,
			)?);

			for image in [&change.old, &change.new].into_iter().flatten() {
				decoded.images.push(super::images::logged_values(
					&definition,
					columns,
					image,
					dialect,
				)?);
				decoded.owners.push(index);
			}
		}

		Ok(decoded)
	}
}

/// Every commit to a live table, read from the change log in version
/// order, with each written row's key and images: for a server's own use,
/// such as a hub that keeps one log for all its subscribers and decides
/// itself what each may see. [`Db::change_log`] starts one; it needs a
/// [`Db::system`] handle, since it bypasses every policy.
///
/// Nothing here is for clients: the images hold every column but `secret`
/// and `live = skip` ones, whoever may read the row.
pub struct ChangeLog {
	reader: Reader,
}

impl ChangeLog {
	pub(crate) fn new(reader: Reader) -> Self {
		Self { reader }
	}

	/// The newest version this log has passed.
	pub fn version(&self) -> i64 {
		self.reader.version()
	}

	/// Waits for the next commit, or for word that the log was pruned past
	/// this reader.
	///
	/// # Errors
	///
	/// When the log can't be read, or holds a value its column can't; the
	/// next call reads on from the same place, or past that commit.
	pub async fn next(&mut self) -> Result<LogEntry> {
		let entry = self.reader.next().await?;
		self.entry(entry)
	}

	/// The next commit when the last read of the log already fetched it,
	/// without reading the log again: for a reader that handles commits in
	/// batches, taking whatever has arrived together. `None` when the next
	/// entry needs a read; call [`ChangeLog::next`] for it.
	///
	/// # Errors
	///
	/// As [`ChangeLog::next`], for a commit holding a value its column
	/// can't.
	pub fn next_ready(&mut self) -> Option<Result<LogEntry>> {
		let entry = self.reader.next_ready()?;
		Some(self.entry(entry))
	}

	fn entry(&self, entry: RawEntry) -> Result<LogEntry> {
		match entry {
			RawEntry::Pruned { through } => Ok(LogEntry::Pruned { through }),
			RawEntry::Commit(commit) => {
				let dialect = self.reader.db().dialect();
				let changes = commit
					.changes
					.into_iter()
					.map(|change| normalize(change, dialect))
					.collect::<Result<_>>()?;

				Ok(LogEntry::Commit(Commit {
					version: commit.version,
					committed_at: commit.committed_at,
					actor: commit.actor,
					origin: commit.origin,
					changes,
				}))
			}
		}
	}
}

impl std::fmt::Debug for ChangeLog {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ChangeLog")
			.field("version", &self.version())
			.finish_non_exhaustive()
	}
}

/// A logged change with its values written the same way on both databases,
/// when this build knows its table.
fn normalize(change: RawChange, dialect: leptoskit_db_schema::Dialect) -> Result<Change> {
	let Some(entry) = crate::table::entries().get(&change.table) else {
		return Ok(Change {
			key: change.key.iter().map(json_text).collect(),
			table: change.table,
			op: change.op,
			old: change.old,
			new: change.new,
			changed: change.changed,
		});
	};
	let definition = (entry.definition)();
	let image = |image: serde_json::Map<String, serde_json::Value>| -> Result<_> {
		let names: Vec<&str> = image.keys().map(String::as_str).collect();
		let values = super::images::logged_values(&definition, &names, &image, dialect)?;
		Ok(names
			.iter()
			.zip(values)
			.map(|(name, value)| {
				// A value this build can't read back, such as a Postgres
				// shape, stays as the database wrote it.
				let json = match value {
					Value::Null => image[*name].clone(),
					value => crate::audit::json(&value),
				};

				((*name).to_string(), json)
			})
			.collect())
	};
	let key = super::images::logged_key(&definition, &change.key, dialect)?;

	Ok(Change {
		table: change.table,
		op: change.op,
		key: key.iter().map(key_text).collect(),
		old: change.old.map(image).transpose()?,
		new: change.new.map(image).transpose()?,
		changed: change.changed,
	})
}

/// A key column's value as a client names it: an `Id` as its hyphenated
/// UUID, numbers in decimal, text as it is.
fn key_text(value: &Value) -> String {
	json_text(&crate::audit::json(value))
}

fn json_text(json: &serde_json::Value) -> String {
	match json {
		serde_json::Value::String(text) => text.clone(),
		other => other.to_string(),
	}
}

impl Db {
	/// The changes to `#[db::table(live)]` tables this handle may know
	/// about, from now on: each committed insert, update, and delete whose
	/// row this handle's `select` policy allows, before or after the change.
	/// Make one per subscriber, from the request's handle.
	///
	/// ```ignore
	/// let mut changes = request_db.changes().await?;
	/// while let Ok(invalidation) = changes.next().await {
	/// 	events.send(invalidation).await?;
	/// }
	/// ```
	///
	/// Changes come from the change log that migrations' capture triggers
	/// fill, so raw SQL and other services' writes arrive too. On Postgres
	/// the first call opens a connection that listens for every server's
	/// commits, and returns once it's listening.
	pub async fn changes(&self) -> Result<Feed> {
		Ok(Feed::new(Reader::start(self.clone(), None).await?))
	}

	/// [`Db::changes`], carrying on after `version`: what a client that
	/// reconnects hasn't heard yet, from [`Feed::version`] or
	/// [`VersionedInvalidation::version`]. When the log was pruned past
	/// `version`, the feed starts with [`Invalidation::Everything`].
	pub async fn changes_since(&self, version: i64) -> Result<Feed> {
		Ok(Feed::new(Reader::start(self.clone(), Some(version)).await?))
	}

	/// Every commit to a live table after `version`, or from now on when
	/// `None`, with what each wrote: for a server's own use. See
	/// [`ChangeLog`].
	///
	/// # Errors
	///
	/// [`DbError::Forbidden`] unless this handle is a [`Db::system`] one,
	/// since the log holds rows whoever may read them.
	pub async fn change_log(&self, version: Option<i64>) -> Result<ChangeLog> {
		if self.context().is_some() {
			return Err(DbError::Forbidden {
				table: "live.changes".to_string(),
				message: "the change log holds rows whoever may read them, so only a `system` \
				          handle reads it"
					.to_string(),
			});
		}

		Ok(ChangeLog::new(Reader::start(self.clone(), version).await?))
	}

	/// The newest committed version of the change log, or 0 before the
	/// first, or without live tables.
	pub async fn live_version(&self) -> Result<i64> {
		super::log::latest_version(self).await
	}

	/// Deletes the change log's commits older than `retention`. A feed or
	/// client further behind than that hears
	/// [`Invalidation::Everything`] and refetches, which is always correct:
	/// resuming is only an optimisation. Call it now and then, from one
	/// server or several; Postgres lets one prune at a time. Returns how many
	/// commits it removed.
	pub async fn prune_changes(&self, retention: std::time::Duration) -> Result<u64> {
		super::log::prune(self, retention).await
	}

	/// Makes sure something hears commits: on Postgres, the connection that
	/// listens for them.
	#[cfg_attr(
		not(feature = "postgres"),
		expect(clippy::unused_async, reason = "only Postgres listens before reading")
	)]
	pub(crate) async fn listen(&self) -> Result<()> {
		#[cfg(feature = "postgres")]
		if let Some(url) = self.postgres_url() {
			self.live
				.listener
				.get_or_try_init(|| {
					crate::postgres::Listener::start(url, std::sync::Arc::downgrade(&self.live))
				})
				.await?;
		}

		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn keys_read_as_clients_write_them() {
		assert_eq!(
			key_text(&Value::Uuid(uuid::Uuid::from_u128(1))),
			"00000000-0000-0000-0000-000000000001"
		);
		assert_eq!(key_text(&Value::Int(42)), "42");
		assert_eq!(key_text(&Value::Text("ada".into())), "ada");
		assert_eq!(key_text(&Value::Bool(true)), "true");
	}
}
