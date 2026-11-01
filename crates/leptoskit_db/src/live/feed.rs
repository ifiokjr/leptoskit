//! Where committed live changes go, and each subscriber's view of them.

use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use super::ChangedRows;
use super::Invalidation;
use super::RowChange;
use crate::Db;
use crate::DbError;
use crate::Read;
use crate::Result;
use crate::Value;
use crate::policy::Access;

/// How many change sets a subscriber may fall behind before it's told to
/// refetch everything instead.
const CAPACITY: usize = 1024;

/// Something every subscriber hears.
#[derive(Clone, Debug)]
pub(crate) enum Event {
	/// One transaction's changes.
	Changes(Arc<[RowChange]>),
	/// Changes may have been missed.
	#[cfg_attr(
		not(feature = "postgres"),
		expect(dead_code, reason = "only Postgres's listener can miss changes")
	)]
	Lost,
}

/// The changes a database's handles have committed, for its subscribers.
/// On Postgres they arrive through `LISTEN`, from every server sharing the
/// database; on SQLite, from this process's commits.
pub(crate) struct Hub {
	sender: broadcast::Sender<Event>,
	#[cfg(feature = "postgres")]
	pub(crate) listener: tokio::sync::OnceCell<crate::postgres::Listener>,
}

impl Hub {
	pub(crate) fn new() -> Self {
		Self {
			sender: broadcast::channel(CAPACITY).0,
			#[cfg(feature = "postgres")]
			listener: tokio::sync::OnceCell::new(),
		}
	}

	/// Tells every subscriber.
	pub(crate) fn send(&self, event: Event) {
		// Nobody listening is fine: there's no one to tell.
		let _ = self.sender.send(event);
	}

	pub(crate) fn subscribe(&self) -> broadcast::Receiver<Event> {
		self.sender.subscribe()
	}
}

/// The live changes one handle may know about, from when it subscribed:
/// [`Db::changes`] starts one.
///
/// Each change is checked against the handle's `select` policy, as the
/// rows were before and after it, so a subscriber learns of a change only
/// when it could read the row on one side of it.
pub struct Feed {
	db: Db,
	events: broadcast::Receiver<Event>,
}

impl Feed {
	pub(crate) fn new(db: Db, events: broadcast::Receiver<Event>) -> Self {
		Self { db, events }
	}

	/// Waits for the next change this handle may know about.
	///
	/// [`Invalidation::Everything`] says changes may have been missed: this
	/// subscriber fell behind, the server lost its connection to the
	/// database for a while, or a change came from a table this build
	/// doesn't know, as during a rolling deploy.
	///
	/// # Errors
	///
	/// When checking a change against the policy fails, such as when the
	/// database is unreachable. The feed stays usable; the change is lost, so
	/// a client should refetch everything.
	pub async fn next(&mut self) -> Result<Invalidation> {
		loop {
			let changes = match self.events.recv().await {
				Ok(Event::Changes(changes)) => changes,
				Ok(Event::Lost) | Err(RecvError::Lagged(_)) => return Ok(Invalidation::Everything),
				Err(RecvError::Closed) => return Err(DbError::connection(Closed)),
			};

			if let Some(invalidation) = self.visible(&changes).await? {
				return Ok(invalidation);
			}
		}
	}

	/// The changes in one set this handle may know about, by table, or
	/// `None` when it may know of none.
	async fn visible(&self, changes: &[RowChange]) -> Result<Option<Invalidation>> {
		let mut tables: Vec<(&leptoskit_db_schema::TableName, Vec<&RowChange>)> = Vec::new();

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
			let visible = self.readable(entry, &rows).await?;
			let mut seen = HashSet::new();
			let keys: Vec<Vec<String>> = rows
				.iter()
				.zip(visible)
				.filter(|(_, visible)| *visible)
				.map(|(change, _)| change.key.iter().map(key_text).collect::<Vec<_>>())
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
		rows: &[&RowChange],
	) -> Result<Vec<bool>> {
		// Each change's images, numbered in order, and which change each is.
		let mut images: Vec<&[Value]> = Vec::with_capacity(rows.len() * 2);
		let mut owners = Vec::with_capacity(rows.len() * 2);

		for (index, change) in rows.iter().enumerate() {
			for image in [&change.before, &change.after].into_iter().flatten() {
				images.push(image);
				owners.push(index);
			}
		}

		let access: &Access = self.db.access();
		let Some(statements) = entry.visible_images(access, self.db.dialect(), &images)? else {
			return Ok(vec![true; rows.len()]);
		};
		let mut visible = vec![false; rows.len()];

		for statement in statements {
			for row in self.db.fetch(statement).await? {
				let owner = match row.first() {
					Some(Value::Int(number)) => {
						usize::try_from(*number)
							.ok()
							.and_then(|number| owners.get(number))
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
		f.debug_struct("Feed").finish_non_exhaustive()
	}
}

/// Every handle to the database is gone, so no more changes will come.
#[derive(Debug, thiserror::Error)]
#[error("the database's live changes have stopped: every handle to it is closed")]
struct Closed;

/// A key column's value as a client names it: an `Id` as its hyphenated
/// UUID, numbers in decimal, text as it is.
fn key_text(value: &Value) -> String {
	match crate::audit::json(value) {
		serde_json::Value::String(text) => text,
		other => other.to_string(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Connection;

	#[cfg(feature = "sqlite")]
	#[test]
	fn a_handle_outside_a_transaction_publishes_at_once() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		runtime.block_on(async {
			let db = Db::connect("sqlite::memory:").await.unwrap();
			let mut events = db.live.subscribe();
			let change = RowChange {
				table: leptoskit_db_schema::TableName::new("nowhere"),
				op: crate::AuditOp::Insert,
				key: vec![Value::Int(1)],
				before: None,
				after: Some(vec![Value::Int(1)]),
			};
			db.publish(vec![change.clone()]).await.unwrap();
			let Ok(Event::Changes(changes)) = events.recv().await else {
				panic!("the change is published");
			};
			assert_eq!(&*changes, [change]);

			// A table this build doesn't know can't be checked: subscribers
			// are told to refetch everything.
			let mut feed = db.changes().await.unwrap();
			db.live.send(Event::Changes(changes));
			assert_eq!(feed.next().await.unwrap(), Invalidation::Everything);
		});
	}

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
