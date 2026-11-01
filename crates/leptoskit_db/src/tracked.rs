//! Read sets: what a read-only connection read, so that a live query can
//! tell which writes could change its result.
//!
//! A [`Tracked`] connection wraps another and records one [`ReadEntry`] per
//! read: the table, every condition that decides which of its rows the read
//! saw, and the keys of the rows it returned. A write to a row that meets no
//! entry's condition, before or after the write, can't change what the reads
//! returned. Entries may over-approximate, so that a doubtful row re-runs a
//! query, but must never miss a read.

use std::future::Future;
use std::sync::Mutex;
use std::sync::PoisonError;

use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::TableName;

use crate::DbError;
use crate::Expr;
use crate::Order;
use crate::Read;
use crate::Result;
use crate::Table;
use crate::Value;
use crate::policy::Access;
use crate::sql::Statement;

/// What kind of read a [`ReadEntry`] records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReadKind {
	/// Rows, in order and maybe limited: [`Query::all`](crate::Query::all),
	/// `first` and `one`, shapes and their nested rows, link accessors that
	/// list rows, searches, the rows nearest a place, and the rows a link's
	/// visibility rule or a variant's supertype looks up.
	Rows,
	/// How many rows match: [`Query::count`](crate::Query::count), a
	/// search's `count`, a shape's `count(…)`, and a policy's `cx.count(…)` or
	/// `cx.exists(…)`.
	Count,
	/// Whether any row matches: [`Query::exists`](crate::Query::exists).
	Exists,
	/// One row by its key: `get`, `find`, and the accessor of a link column.
	Point,
	/// Any row of the table: raw SQL, and the tables a view declares it
	/// reads.
	Table,
}

/// One read a [`Tracked`] connection made.
///
/// A write can change what the read returned only if the row it wrote met
/// `condition` before or after the write; [`Expr::eval`] tells, without the
/// database, when a row certainly doesn't.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ReadEntry {
	/// The table read.
	pub table: TableName,
	/// What the read asked for.
	pub kind: ReadKind,
	/// Which rows the read saw: the query's own filter, the `select` policy
	/// of whoever read, and the soft-delete default, combined. Conditions on
	/// the rows around a nested read, such as a shape's link to its parent
	/// row, and a search's match, are unknown to [`Expr::eval`].
	pub condition: Expr,
	/// The read's ordering, by its columns. Empty when no column orders
	/// it, or when something only the database computes orders it first,
	/// such as a search's rank or the distance to a place: a limited read
	/// without an order can't tell which rows enter its window.
	pub order: Vec<Order>,
	/// The most rows it returned.
	pub limit: Option<u64>,
	/// How many rows it skipped.
	pub offset: Option<u64>,
	/// The keys of the rows it returned, each in the table's key order,
	/// when it returned whole rows of a keyed table; `None` when it doesn't
	/// know them, as for counts and shapes.
	pub keys: Option<Vec<Vec<Value>>>,
}

impl ReadEntry {
	/// Whether a write to one row could change what this read returned.
	///
	/// It says `false` only when the row certainly wasn't in the result,
	/// before or after the write: it belongs to another table, or neither
	/// image meets the read's condition. A condition
	/// [`Expr::eval`](crate::Expr::eval) can't decide — a dialect
	/// difference, a subquery, a setting — counts as a change, so the worst
	/// case is a re-run. A row that matches may still fall outside a
	/// limited window; that re-runs too.
	#[must_use]
	pub fn could_change(&self, change: &crate::live::Change, dialect: Dialect) -> bool {
		if self.table != change.table {
			return false;
		}
		if self.kind == ReadKind::Table {
			return true;
		}
		let Some(entry) = crate::table::entries().get(&change.table).copied() else {
			return true;
		};
		let definition = (entry.definition)();
		let decode = |image: &serde_json::Map<String, serde_json::Value>| {
			let names: Vec<&str> = image.keys().map(String::as_str).collect();
			let values = crate::live::logged_values(&definition, &names, image, dialect).ok()?;
			Some(
				names
					.into_iter()
					.zip(values)
					.map(|(name, value)| (name.to_owned(), value))
					.collect::<std::collections::HashMap<_, _>>(),
			)
		};
		let old = change.old.as_ref().and_then(decode);
		let new = change.new.as_ref().and_then(decode);
		let may =
			|row: &std::collections::HashMap<String, Value>| self.condition.eval(row).may_match();

		old.as_ref().is_some_and(may) || new.as_ref().is_some_and(may)
	}

	/// An entry for rows of `table` meeting `condition`, in no order.
	pub(crate) fn new(table: TableName, kind: ReadKind, condition: Expr) -> Self {
		Self {
			table,
			kind,
			condition,
			order: Vec::new(),
			limit: None,
			offset: None,
			keys: None,
		}
	}

	/// An entry for any row of `table`.
	fn whole(table: TableName) -> Self {
		Self::new(table, ReadKind::Table, Expr::always())
	}
}

/// What one statement the database layer runs reads: its own entry first,
/// then the entries of the subqueries and nested reads inside it.
#[doc(hidden)]
#[derive(Clone, Debug, Default)]
pub struct Reads {
	pub(crate) entries: Vec<ReadEntry>,
	/// The key of a returned row, for the first entry, when the rows are
	/// whole rows of a keyed table.
	pub(crate) keys: Option<KeyOf>,
}

/// The key of a row returned in column order, decoded through its table's
/// types: SQLite returns a UUID as bytes.
pub(crate) type KeyOf = fn(Vec<Value>) -> Result<Vec<Value>>;

/// The key of a row of `T`, from its values in [`Table::COLUMNS`] order,
/// and whatever the statement selects after them, such as a search's rank.
fn key_of<T: Table>(mut values: Vec<Value>) -> Result<Vec<Value>> {
	values.truncate(T::COLUMNS.len());
	let values = T::from_values(values)?.to_values()?;
	Ok(crate::policy::key_of::<T>(&values))
}

impl Reads {
	/// The read of `T`'s rows meeting `filter`, as `access` sees them, with
	/// the entries of every subquery inside.
	pub(crate) fn of<T: Table>(
		access: &Access,
		kind: ReadKind,
		filter: Option<&Expr>,
		with_deleted: bool,
	) -> Result<Self> {
		let policy = crate::policy::condition::<T>(access, crate::Action::Select);
		let condition = conjunction([
			filter.cloned(),
			policy,
			not_deleted(T::DELETED_AT, with_deleted),
		]);
		let mut entries = Vec::new();
		subqueries(&condition, &mut entries);
		entries.insert(0, ReadEntry::new(T::table_name(), kind, condition));

		// A view's rows come from the tables its query reads.
		if T::READ_ONLY {
			let Some(tables) = T::reads_through() else {
				return Err(DbError::UndeclaredRead {
					table: Some(T::flat_name()),
				});
			};
			entries.extend(tables.into_iter().map(ReadEntry::whole));
		}

		let keys = match kind {
			ReadKind::Rows | ReadKind::Exists | ReadKind::Point if !T::KEY.is_empty() => {
				Some(key_of::<T> as KeyOf)
			}
			_ => None,
		};

		Ok(Self { entries, keys })
	}

	/// The same reads, with `condition` too on the first entry's rows: what
	/// narrows a read past its filter, such as a search's match.
	pub(crate) fn narrowed(mut self, condition: Expr) -> Self {
		if let Some(entry) = self.entries.first_mut() {
			entry.condition = entry.condition.clone() & condition;
		}

		self
	}

	/// The same reads, ordered, limited and offset as the statement is.
	pub(crate) fn tail(mut self, order: &[Order], limit: Option<u64>, offset: Option<u64>) -> Self {
		if let Some(entry) = self.entries.first_mut() {
			entry.order = order.to_vec();
			entry.limit = limit;
			entry.offset = offset;
		}

		self
	}
}

/// Every condition in `conditions`, combined with `&`, or every row when
/// there are none.
pub(crate) fn conjunction(conditions: impl IntoIterator<Item = Option<Expr>>) -> Expr {
	conditions
		.into_iter()
		.flatten()
		.reduce(|left, right| left & right)
		.unwrap_or_else(Expr::always)
}

/// `column IS NULL`, for a table that soft-deletes into `column`, unless
/// the read keeps deleted rows.
pub(crate) fn not_deleted(column: Option<&'static str>, with_deleted: bool) -> Option<Expr> {
	column.filter(|_| !with_deleted).map(|column| {
		Expr::Null {
			column,
			negated: false,
		}
	})
}

/// Appends an entry for every subquery `condition` reads: the counts of a
/// policy's `cx.count(…)`, and the rows a link's visibility looks up, along
/// with the subqueries inside those.
pub(crate) fn subqueries(condition: &Expr, entries: &mut Vec<ReadEntry>) {
	match condition {
		Expr::And(left, right) | Expr::Or(left, right) => {
			subqueries(left, entries);
			subqueries(right, entries);
		}
		Expr::Not(inner) => subqueries(inner, entries),
		Expr::Count { count, .. } => count.rows().record(ReadKind::Count, entries),
		Expr::Within { rows, .. } => rows.record(ReadKind::Rows, entries),
		_ => {}
	}
}

/// `condition`, with every comparison to another row's column made a
/// [`Expr::Correlated`] one: a subquery's or nested read's filter can name
/// the row around it, which an entry about the subquery's own rows can't
/// see.
pub(crate) fn decorrelate(condition: &Expr) -> Expr {
	match condition {
		Expr::CompareColumn {
			column,
			comparison,
			table,
			other,
		} => {
			Expr::Correlated {
				column,
				comparison: *comparison,
				table: table.clone(),
				other,
			}
		}
		Expr::And(left, right) => decorrelate(left) & decorrelate(right),
		Expr::Or(left, right) => decorrelate(left) | decorrelate(right),
		Expr::Not(inner) => !decorrelate(inner),
		other => other.clone(),
	}
}

/// A connection that records what it reads, for a live query to know
/// which writes could change its result. It only reads: every write needs a
/// [`Connection`](crate::Connection), which it isn't, so a write through it
/// fails to compile.
///
/// ```ignore
/// let tracked = Tracked::new(db.with_context(cx));
/// let messages = Message::query().filter(|m| m.channel_id.eq(channel)).all(&tracked).await?;
/// for entry in tracked.reads() {
/// 	// `entry.table`, `entry.condition`, `entry.keys`, …
/// }
/// ```
///
/// Every read records a [`ReadEntry`]: queries, counts, `get` and `find`,
/// link accessors, each level of a shape, searches, and the subqueries a
/// policy's `cx.count(…)` reads, so a revoked membership is a change the
/// read saw.
/// Raw SQL must say which tables it reads with
/// [`Statement::reads`](crate::Statement::reads), and a view with
/// `#[db::view(reads(…))]`; otherwise the read fails with
/// [`DbError::UndeclaredRead`] rather than go unrecorded.
pub struct Tracked<C> {
	connection: C,
	reads: Mutex<Vec<ReadEntry>>,
}

impl<C: Read> Tracked<C> {
	/// Records every read made through it on `connection`, which can be a
	/// `Db`, a `&Tx`, or anything else that reads.
	pub fn new(connection: C) -> Self {
		Self {
			connection,
			reads: Mutex::new(Vec::new()),
		}
	}

	/// The connection being tracked.
	pub fn connection(&self) -> &C {
		&self.connection
	}

	/// Everything read so far, in the order the reads finished.
	pub fn reads(&self) -> Vec<ReadEntry> {
		self.reads
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.clone()
	}

	/// Everything read, ending the tracking.
	pub fn into_reads(self) -> Vec<ReadEntry> {
		self.reads
			.into_inner()
			.unwrap_or_else(PoisonError::into_inner)
	}
}

impl<C: Read> Read for Tracked<C> {
	fn dialect(&self) -> Dialect {
		self.connection.dialect()
	}

	fn access(&self) -> &Access {
		self.connection.access()
	}

	fn row_security(&self) -> bool {
		self.connection.row_security()
	}

	fn fetch(&self, _statement: Statement) -> impl Future<Output = Result<Vec<Vec<Value>>>> + Send {
		std::future::ready(Err(DbError::UndeclaredRead { table: None }))
	}

	async fn read(
		&self,
		statement: Statement,
		describe: impl FnOnce() -> Result<Reads> + Send,
	) -> Result<Vec<Vec<Value>>> {
		let Reads { mut entries, keys } = describe()?;
		let inner = Reads {
			entries: entries.clone(),
			keys,
		};
		let rows = self.connection.read(statement, move || Ok(inner)).await;

		if let (Ok(rows), Some(key_of), Some(entry)) = (&rows, keys, entries.first_mut()) {
			entry.keys = Some(rows.iter().cloned().map(key_of).collect::<Result<_>>()?);
		}

		self.reads
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.extend(entries);
		rows
	}
}

/// SQL you wrote, run as a read that says which tables it reads, so that a
/// [`Tracked`] connection can record them. [`Statement::reads`] starts one:
///
/// ```ignore
/// let rows = Statement::new("SELECT count(*) FROM messages WHERE body LIKE $1", vec![pattern.into()])
/// 	.reads::<Message>()
/// 	.fetch(&tracked)
/// 	.await?;
/// ```
///
/// Policies don't apply to raw SQL, so it records every row of each table
/// as read.
#[must_use = "a read does nothing until it's fetched"]
pub struct RawRead {
	statement: Statement,
	tables: Vec<TableName>,
}

impl Statement {
	/// A statement with these placeholder values: `$1`, `$2`, … on Postgres,
	/// and `?1`, `?2`, … on SQLite.
	pub fn new(sql: impl Into<String>, args: Vec<Value>) -> Self {
		Self {
			sql: sql.into(),
			args,
		}
	}

	/// Runs the statement as a read of `T`, and of any other table named
	/// with [`RawRead::reads`].
	pub fn reads<T: Table>(self) -> RawRead {
		RawRead {
			statement: self,
			tables: vec![T::table_name()],
		}
	}
}

impl RawRead {
	/// It reads `T` too.
	pub fn reads<T: Table>(mut self) -> Self {
		self.tables.push(T::table_name());
		self
	}

	/// Runs the statement and returns its rows, each as values in column
	/// order.
	pub async fn fetch(self, connection: &impl Read) -> Result<Vec<Vec<Value>>> {
		let tables = self.tables;
		connection
			.read(self.statement, move || {
				Ok(Reads {
					entries: tables.into_iter().map(ReadEntry::whole).collect(),
					keys: None,
				})
			})
			.await
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Context;
	use crate::expr::Comparison;
	use crate::sql::Sql;

	/// A room, to count in conditions.
	#[crate::table(name = "rooms")]
	pub struct Room {
		pub id: crate::Id<Self>,
		pub floor: i64,
	}

	fn rooms() -> TableName {
		Room::table_name()
	}

	#[test]
	fn other_rows_columns_become_correlated_wherever_they_are() {
		let other = || {
			Expr::CompareColumn {
				column: "floor",
				comparison: Comparison::Less,
				table: rooms(),
				other: "floor",
			}
		};
		let correlated = Expr::Correlated {
			column: "floor",
			comparison: Comparison::Less,
			table: rooms(),
			other: "floor",
		};
		let mixed = !(other() | Expr::always()) & other();

		assert_eq!(
			decorrelate(&mixed),
			!(correlated.clone() | Expr::always()) & correlated.clone()
		);

		// Only a read set holds one; it doesn't run.
		let mut sql = Sql::new(Dialect::Sqlite);
		let refused = correlated.write(&mut sql).unwrap_err();
		assert!(matches!(refused, DbError::Encode(_)));
	}

	#[test]
	fn subqueries_are_found_under_every_connective() {
		let cx = Context::new();
		let count = || cx.exists(Room::query().filter(|room| room.floor.gt(1)));
		let mut entries = Vec::new();
		subqueries(&(!count() | (Expr::always() & count())), &mut entries);

		assert_eq!(entries.len(), 2);
		assert!(entries.iter().all(|entry| {
			entry.table == rooms() && entry.kind == ReadKind::Count && entry.keys.is_none()
		}));
	}
	#[test]
	fn a_write_changes_a_read_only_when_its_row_could_match() {
		use crate::live::Change;
		use crate::live::ChangeOp;

		let entry = ReadEntry {
			table: rooms(),
			kind: ReadKind::Rows,
			condition: Room::columns().floor.gt(1),
			order: Vec::new(),
			limit: None,
			offset: None,
			keys: None,
		};
		let image = |floor: i64| {
			serde_json::Map::from_iter([("floor".to_owned(), serde_json::json!(floor))])
		};
		let change = |op, old: Option<i64>, new: Option<i64>| {
			Change {
				table: rooms(),
				op,
				key: vec!["1".to_owned()],
				old: old.map(image),
				new: new.map(image),
				changed: Vec::new(),
			}
		};

		assert!(entry.could_change(&change(ChangeOp::Insert, None, Some(3)), Dialect::Sqlite));
		assert!(!entry.could_change(&change(ChangeOp::Insert, None, Some(0)), Dialect::Sqlite));
		assert!(entry.could_change(&change(ChangeOp::Update, Some(0), Some(2)), Dialect::Sqlite));
		assert!(!entry.could_change(&change(ChangeOp::Delete, Some(1), None), Dialect::Sqlite));

		// An image without the column can't be judged, so the read re-runs.
		let missing = Change {
			table: rooms(),
			op: ChangeOp::Insert,
			key: vec!["1".to_owned()],
			old: None,
			new: Some(serde_json::Map::new()),
			changed: Vec::new(),
		};
		assert!(entry.could_change(&missing, Dialect::Sqlite));
	}
}
