//! Reading rows, and changing many at once.

use std::borrow::Cow;
use std::marker::PhantomData;

use crate::Action;
use crate::Col;
use crate::Column;
use crate::Connection;
use crate::DbError;
use crate::Expr;
use crate::Operand;
use crate::Order;
use crate::Read;
use crate::Result;
use crate::Table;
use crate::Value;
use crate::audit;
use crate::audit::AuditOp;
use crate::audit::Change;
use crate::connection::atomically;
use crate::policy;
use crate::sql::Sql;
use crate::tracked::ReadKind;
use crate::tracked::Reads;

/// A query on table `T`. Start one with `User::query()`.
///
/// Rows soft-deleted through a `#[db(deleted_at)]` column are left out unless
/// the query asks for them with [`Query::with_deleted`].
#[must_use = "a query does nothing until it runs"]
pub struct Query<T> {
	pub(crate) filter: Option<Expr>,
	pub(crate) order: Vec<Order>,
	pub(crate) limit: Option<u64>,
	pub(crate) offset: Option<u64>,
	pub(crate) with_deleted: bool,
	skip_hooks: bool,
	/// Whether it reads one row by its key, for a read set.
	point: bool,
	table: PhantomData<fn() -> T>,
}

impl<T> Clone for Query<T> {
	fn clone(&self) -> Self {
		Self {
			filter: self.filter.clone(),
			order: self.order.clone(),
			limit: self.limit,
			offset: self.offset,
			with_deleted: self.with_deleted,
			skip_hooks: self.skip_hooks,
			point: self.point,
			table: PhantomData,
		}
	}
}

impl<T: Table> Default for Query<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T: Table> Query<T> {
	/// Every row.
	pub fn new() -> Self {
		Self {
			filter: None,
			order: Vec::new(),
			limit: None,
			offset: None,
			with_deleted: false,
			skip_hooks: false,
			point: false,
			table: PhantomData,
		}
	}

	/// Keeps only rows matching `condition`. Several filters must all match.
	pub fn filter(mut self, condition: impl FnOnce(&T::Columns) -> Expr) -> Self {
		let condition = condition(T::columns());
		self.filter = Some(match self.filter.take() {
			Some(filter) => filter & condition,
			None => condition,
		});
		self
	}

	/// Marks the query as a read of one row by its key, which `get`, `find`
	/// and link accessors make, so a read set records it as one.
	#[doc(hidden)]
	pub fn point(mut self) -> Self {
		self.point = true;
		self
	}

	/// Lets [`Query::delete`] and [`Query::destroy`] run on a table with an
	/// `after_delete` hook, which they skip: they never load the rows they
	/// delete. Without it, they fail with
	/// [`DbError::HooksSkipped`](crate::DbError::HooksSkipped).
	pub fn allow_skipping_hooks(mut self) -> Self {
		self.skip_hooks = true;
		self
	}

	/// Orders the rows. Later orderings break ties in earlier ones.
	pub fn order_by(mut self, order: impl FnOnce(&T::Columns) -> Order) -> Self {
		self.order.push(order(T::columns()));
		self
	}

	/// At most this many rows.
	pub fn limit(mut self, limit: u64) -> Self {
		self.limit = Some(limit);
		self
	}

	/// Skips this many rows.
	pub fn offset(mut self, offset: u64) -> Self {
		self.offset = Some(offset);
		self
	}

	/// Includes soft-deleted rows.
	pub fn with_deleted(mut self) -> Self {
		self.with_deleted = true;
		self
	}

	/// The same query, ordered by `order` when nothing orders it yet.
	pub(crate) fn ordered_or(mut self, order: impl FnOnce() -> Order) -> Self {
		if self.order.is_empty() {
			self.order.push(order());
		}

		self
	}

	/// Whether the query has a limit or an offset.
	pub(crate) fn is_paged(&self) -> bool {
		self.limit.is_some() || self.offset.is_some()
	}

	/// The query's own condition and whether it keeps soft-deleted rows, for
	/// a policy that counts it.
	pub(crate) fn into_condition(self) -> (Option<Expr>, bool) {
		(self.filter, self.with_deleted)
	}

	/// Every matching row.
	pub async fn all(self, connection: &impl Read) -> Result<Vec<T>> {
		let kind = if self.point {
			ReadKind::Point
		} else {
			ReadKind::Rows
		};
		self.rows(connection, kind).await
	}

	/// The first matching row, if any.
	pub async fn first(self, connection: &impl Read) -> Result<Option<T>> {
		Ok(self.limit(1).all(connection).await?.into_iter().next())
	}

	/// The first matching row, or [`DbError::NotFound`].
	pub async fn one(self, connection: &impl Read) -> Result<T> {
		self.first(connection).await?.ok_or_else(|| {
			DbError::NotFound {
				table: T::flat_name(),
			}
		})
	}

	/// How many rows match.
	pub async fn count(self, connection: &impl Read) -> Result<u64> {
		let mut sql = Sql::new(connection.dialect());
		sql.push("SELECT count(*) FROM ").table::<T>();
		self.write_filter(&mut sql, connection, Action::Select)?;

		let rows = connection
			.read(sql.finish(), || self.reads(connection, ReadKind::Count))
			.await?;
		crate::sql::count_of(&rows)
	}

	/// Whether any row matches.
	pub async fn exists(self, connection: &impl Read) -> Result<bool> {
		Ok(!self
			.limit(1)
			.rows(connection, ReadKind::Exists)
			.await?
			.is_empty())
	}

	/// The matching rows, read as `kind`.
	async fn rows(self, connection: &impl Read, kind: ReadKind) -> Result<Vec<T>> {
		let mut sql = Sql::new(connection.dialect());
		sql.push("SELECT ")
			.columns(T::COLUMNS)
			.push(" FROM ")
			.table::<T>();
		self.write_filter(&mut sql, connection, Action::Select)?;
		write_order(&mut sql, &self.order, self.limit, self.offset);

		let rows = connection
			.read(sql.finish(), || {
				Ok(self
					.reads(connection, kind)?
					.tail(&self.order, self.limit, self.offset))
			})
			.await?;
		rows.into_iter()
			.map(|values| T::from_values(values).map_err(DbError::from))
			.collect()
	}

	/// What the query reads on `connection`, for a read set.
	pub(crate) fn reads(&self, connection: &impl Read, kind: ReadKind) -> Result<Reads> {
		Reads::of::<T>(
			connection.access(),
			kind,
			self.filter.as_ref(),
			self.with_deleted,
		)
	}

	/// Deletes every matching row: soft-deletes them when the table has a
	/// `#[db(deleted_at)]` column, along with the rows their
	/// `on_soft_delete = cascade` links reach, in one transaction. Returns how
	/// many rows of this table changed.
	///
	/// On an audited table, every deleted row is recorded. An `after_delete`
	/// hook doesn't run; see [`Query::allow_skipping_hooks`].
	pub async fn delete(self, connection: &impl Connection) -> Result<u64> {
		let Some(column) = T::DELETED_AT else {
			return self.destroy(connection).await;
		};

		refuse_bulk::<T>(T::HOOKS.after_delete, self.skip_hooks)?;
		let now = crate::now();
		let mut update = Update::<T>::new();
		update.filter = self.filter;
		update.action = Action::Delete;
		update.sets.push((column.into(), Ok(Value::Timestamp(now))));
		let policy = policy::condition::<T>(connection.access(), Action::Delete);
		let changes = update.changes(connection.access())?;

		#[cfg(any(feature = "postgres", feature = "sqlite"))]
		if audit::records::<T>() || crate::cascade::has_cascades::<T>() {
			return atomically!(connection, |tx| {
				soft_delete_cascading::<T>(tx, &changes, policy.as_ref(), now).await
			});
		}

		#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
		let _ = now;

		let sql = changes.statement(connection.dialect(), None, policy.as_ref())?;
		connection.execute(sql.finish()).await
	}

	/// Removes every matching row for good, soft-deleted or not.
	///
	/// On an audited table, every removed row is recorded. An `after_delete`
	/// hook doesn't run; see [`Query::allow_skipping_hooks`].
	pub async fn destroy(mut self, connection: &impl Connection) -> Result<u64> {
		refuse_bulk::<T>(T::HOOKS.after_delete, self.skip_hooks)?;
		self.with_deleted = true;

		// A variant of a polymorphic table removes its supertype rows after
		// it, so it needs to know which rows it removed.
		if crate::fallout::reaches::<T>() || T::SUPERTYPE.is_some() {
			let mut condition = self.filter.take().unwrap_or_else(Expr::always);

			if let Some(policy) = policy::condition::<T>(connection.access(), Action::Delete) {
				condition = condition & policy;
			}

			return atomically!(connection, |tx| {
				destroy_with_fallout::<T>(tx, &condition).await
			});
		}

		let mut sql = Sql::new(connection.dialect());
		sql.push("DELETE FROM ").table::<T>();
		self.write_filter(&mut sql, connection, Action::Delete)?;

		if !audit::records::<T>() {
			return connection.execute(sql.finish()).await;
		}

		sql.push(" RETURNING ").columns(T::COLUMNS);
		atomically!(connection, |tx| audited_destroy::<T>(tx, sql).await)
	}

	/// Fetches a shape of each matching row, in one statement, rather than
	/// the rows themselves:
	///
	/// ```ignore
	/// let cards: Vec<ProfileCard> = User::query().filter(|u| u.handle.eq(handle)).shape().all(&db).await?;
	/// ```
	pub fn shape<S: crate::Shape<From = T>>(self) -> crate::Shaped<S> {
		crate::Shaped::new(self)
	}

	/// The query's ordering, limit, and offset, for a read set.
	pub(crate) fn tail(&self) -> (&[Order], Option<u64>, Option<u64>) {
		(&self.order, self.limit, self.offset)
	}

	/// The query's ordering, limit, and offset.
	pub(crate) fn write_tail(&self, sql: &mut Sql) {
		write_order(sql, &self.order, self.limit, self.offset);
	}

	/// The query's conditions, plus what the connection's policies allow for
	/// `action`.
	pub(crate) fn write_filter(
		&self,
		sql: &mut Sql,
		connection: &impl Read,
		action: Action,
	) -> Result<()> {
		let policy = policy::condition::<T>(connection.access(), action);
		write_where::<T>(
			sql,
			self.filter.as_ref(),
			policy.as_ref(),
			self.with_deleted,
		)
	}
}

/// Changes columns on every matching row. Start one with `User::update()`.
///
/// `updated_at` is set too. Soft-deleted rows are left alone unless
/// [`Update::with_deleted`] asks for them.
#[must_use = "an update does nothing until it runs"]
pub struct Update<T> {
	filter: Option<Expr>,
	sets: Vec<(Cow<'static, str>, Result<Value, crate::EncodeError>)>,
	with_deleted: bool,
	/// The policy action it's checked as: `Update`, or `Delete` for a soft
	/// delete.
	action: Action,
	skip_hooks: bool,
	table: PhantomData<fn() -> T>,
}

impl<T: Table> Default for Update<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T: Table> Update<T> {
	/// An update of every row.
	pub fn new() -> Self {
		Self {
			filter: None,
			sets: Vec::new(),
			with_deleted: false,
			action: Action::Update,
			skip_hooks: false,
			table: PhantomData,
		}
	}

	/// Changes only rows matching `condition`.
	pub fn filter(mut self, condition: impl FnOnce(&T::Columns) -> Expr) -> Self {
		let condition = condition(T::columns());
		self.filter = Some(match self.filter.take() {
			Some(filter) => filter & condition,
			None => condition,
		});
		self
	}

	/// Sets a column.
	pub fn set<V: Column>(
		mut self,
		column: impl FnOnce(&T::Columns) -> Col<T, V>,
		value: impl Operand<V>,
	) -> Self {
		self.sets
			.push((column(T::columns()).name().into(), value.into_value()));
		self
	}

	/// Sets a column known only by name, such as the fingerprint of a
	/// vector's sources.
	#[cfg(feature = "vector")]
	pub(crate) fn set_named(mut self, column: impl Into<Cow<'static, str>>, value: Value) -> Self {
		self.sets.push((column.into(), Ok(value)));
		self
	}

	/// Includes soft-deleted rows.
	pub fn with_deleted(mut self) -> Self {
		self.with_deleted = true;
		self
	}

	/// Lets the update run on a table with a `before_write` or
	/// `after_update` hook, which it skips: it never loads the rows it
	/// changes. Without it, it fails with
	/// [`DbError::HooksSkipped`](crate::DbError::HooksSkipped).
	pub fn allow_skipping_hooks(mut self) -> Self {
		self.skip_hooks = true;
		self
	}

	/// Runs the update and returns how many rows changed.
	///
	/// With access policies, only rows the request may change are changed,
	/// and what they become must be allowed too, or nothing changes and the
	/// update fails with [`DbError::Forbidden`]. On an audited table, every
	/// changed row is recorded.
	pub async fn run(self, connection: &impl Connection) -> Result<u64> {
		refuse_bulk::<T>(T::HOOKS.on_update(), self.skip_hooks)?;
		let policy = policy::condition::<T>(connection.access(), self.action);
		let changes = self.changes(connection.access())?;

		if audit::records::<T>() || policy.is_some() {
			return atomically!(connection, |tx| {
				changes.apply(tx, policy.as_ref()).await
			})
			.map(|keys| keys.len() as u64);
		}

		let sql = changes.statement(connection.dialect(), None, None)?;
		connection.execute(sql.finish()).await
	}

	/// The update's columns and values, checked, with the ones the database
	/// layer sets itself: `updated_at`, and `updated_by` from who is acting.
	fn changes(self, access: &policy::Access) -> Result<Changes<T>> {
		let mut sets = Vec::with_capacity(self.sets.len() + 2);

		for (column, value) in self.sets {
			let value = value.map_err(DbError::Encode)?;

			if T::GENERATED.contains(&&*column) {
				return Err(crate::expr::invalid(&format!(
					"`{column}` is generated by the database, so it can't be set"
				)));
			}

			if Some(&*column) == T::CREATED_BY || Some(&*column) == T::UPDATED_BY {
				return Err(DbError::ReadOnly {
					table: T::flat_name(),
					message: format!(
						"`{column}` records who wrote the `{}` row, so only the database layer \
						 sets it",
						T::flat_name()
					),
				});
			}

			sets.push((column, value));
		}

		if let Some(column) =
			T::UPDATED_AT.filter(|column| !sets.iter().any(|(set, _)| set == column))
		{
			sets.push((column.into(), Value::Timestamp(crate::now())));
		}

		if let Some(column) = T::UPDATED_BY {
			sets.push((column.into(), actor_value::<T>(access, column)?));
		}

		Ok(Changes {
			filter: self.filter,
			sets,
			with_deleted: self.with_deleted,
			action: self.action,
			table: PhantomData,
		})
	}
}

/// Fails a bulk write on a view, or one that would skip the table's hooks
/// without being told it may.
fn refuse_bulk<T: Table>(skips_hooks: bool, allowed: bool) -> Result<()> {
	if T::READ_ONLY {
		return Err(DbError::ReadOnly {
			table: T::flat_name(),
			message: format!("`{}` is a view, so it can't be written", T::flat_name()),
		});
	}

	if skips_hooks && !allowed {
		return Err(DbError::HooksSkipped {
			table: T::flat_name(),
		});
	}

	Ok(())
}

/// The value recording who is acting in `column`: their UUID, or `NULL`
/// when nobody is and the column allows it.
pub(crate) fn actor_value<T: Table>(access: &policy::Access, column: &str) -> Result<Value> {
	if let Some(actor) = access.actor() {
		return Ok(Value::Uuid(actor.uuid()));
	}

	if T::definition()
		.column(column)
		.is_some_and(|column| column.nullable)
	{
		Ok(Value::Null)
	} else {
		Err(DbError::NoActor {
			table: T::flat_name(),
			column: column.to_string(),
		})
	}
}

/// A bulk write, ready to run: its filter, and every column it sets.
struct Changes<T> {
	filter: Option<Expr>,
	sets: Vec<(Cow<'static, str>, Value)>,
	with_deleted: bool,
	action: Action,
	table: PhantomData<fn() -> T>,
}

/// How many rows one audited statement changes at most, keeping the keys
/// it names well inside both databases' limits on bound values.
const BATCH: usize = 500;

impl<T: Table> Changes<T> {
	/// `UPDATE … SET … WHERE …`, narrowed to `rows` when given.
	fn statement(
		&self,
		dialect: crate::Dialect,
		rows: Option<&Expr>,
		policy: Option<&Expr>,
	) -> Result<Sql> {
		let mut sql = Sql::new(dialect);
		sql.push("UPDATE ").table::<T>().push(" SET ");

		for (index, (column, value)) in self.sets.iter().enumerate() {
			if index > 0 {
				sql.push(", ");
			}

			sql.identifier(column).push(" = ").value(value.clone());
		}

		// A bulk write changes versioned rows, so a save based on an earlier
		// read must see a new version.
		if let Some(version) = T::VERSION {
			if !self.sets.is_empty() {
				sql.push(", ");
			}

			sql.identifier(version)
				.push(" = ")
				.identifier(version)
				.push(" + 1");
		}

		let filter = match (&self.filter, rows) {
			(Some(filter), Some(rows)) => Some(filter.clone() & rows.clone()),
			(filter, rows) => filter.clone().or_else(|| rows.cloned()),
		};
		write_where::<T>(&mut sql, filter.as_ref(), policy, self.with_deleted)?;
		Ok(sql)
	}

	/// Runs the update and returns the keys of the rows it changed, after
	/// checking what they became against `policy`, for an update, and
	/// recording them in the audit log, for an audited table.
	async fn apply(
		&self,
		connection: &impl Connection,
		policy: Option<&Expr>,
	) -> Result<Vec<Vec<Value>>> {
		let keys = if audit::records::<T>() {
			self.audited(connection, policy).await?
		} else {
			let mut sql = self.statement(connection.dialect(), None, policy)?;
			sql.push(" RETURNING ").columns(T::KEY);
			connection.fetch(sql.finish()).await?
		};

		if self.action == Action::Update
			&& let Some(check) = policy
		{
			policy::verify::<T>(connection, &keys, Action::Update, check).await?;
		}

		Ok(keys)
	}

	/// Reads and locks the rows the update would change, then changes
	/// exactly those, and records each one before and after.
	async fn audited(
		&self,
		connection: &impl Connection,
		policy: Option<&Expr>,
	) -> Result<Vec<Vec<Value>>> {
		let mut matching = [self.filter.clone(), policy.cloned()]
			.into_iter()
			.flatten()
			.reduce(|left, right| left & right)
			.unwrap_or_else(Expr::always);

		if let Some(column) = T::DELETED_AT.filter(|_| !self.with_deleted) {
			matching = matching
				& Expr::Null {
					column,
					negated: false,
				};
		}

		let old: Vec<Vec<Value>> = crate::write::select_for_update::<T>(connection, &matching)
			.await?
			.iter()
			.map(Table::to_values)
			.collect::<Result<_, _>>()?;
		let op = if self.action == Action::Delete {
			AuditOp::Delete
		} else {
			AuditOp::Update
		};
		let mut keys = Vec::with_capacity(old.len());
		let mut changes = Vec::with_capacity(old.len());

		for batch in old.chunks(BATCH) {
			let batch_keys: Vec<Vec<Value>> =
				batch.iter().map(|row| policy::key_of::<T>(row)).collect();
			let rows = policy::keys_condition::<T>(&batch_keys);
			let mut sql = self.statement(connection.dialect(), Some(&rows), policy)?;
			sql.push(" RETURNING ").columns(T::COLUMNS);
			// Keyed by their debug text: `Value` floats aren't `Hash`.
			let mut before: std::collections::HashMap<String, &Vec<Value>> = batch
				.iter()
				.map(|row| (format!("{:?}", policy::key_of::<T>(row)), row))
				.collect();

			for after in connection.fetch(sql.finish()).await? {
				let after = T::from_values(after)?.to_values()?;
				let key = policy::key_of::<T>(&after);
				let old = before.remove(&format!("{key:?}")).cloned();
				keys.push(key);
				changes.push(Change {
					op,
					before: old,
					after: Some(after),
				});
			}
		}

		audit::record_changes(connection, audit::Subject::of::<T>(), changes).await?;
		Ok(keys)
	}
}

/// Removes rows with `sql`, which returns them, and records each one.
async fn audited_destroy<T: Table>(connection: &impl Connection, sql: Sql) -> Result<u64> {
	let removed = connection.fetch(sql.finish()).await?;
	let count = removed.len() as u64;
	let changes = removed
		.into_iter()
		.map(|before| {
			Change {
				op: AuditOp::Delete,
				before: Some(before),
				after: None,
			}
		})
		.collect();
	audit::record_changes(connection, audit::Subject::of::<T>(), changes).await?;
	Ok(count)
}

/// Removes the rows matching `condition` for good, and records them and
/// the rows the database's own cascades change with them: reads and locks
/// them first, then deletes exactly those, then their supertype rows.
async fn destroy_with_fallout<T: Table>(
	connection: &impl Connection,
	condition: &Expr,
) -> Result<u64> {
	let rows: Vec<Vec<Value>> = crate::write::select_for_update::<T>(connection, condition)
		.await?
		.iter()
		.map(Table::to_values)
		.collect::<Result<_, _>>()?;
	let fallout =
		crate::fallout::before_delete(connection, T::table_name(), T::COLUMNS, rows.clone())
			.await?;
	let mut removed = 0;

	for batch in rows.chunks(BATCH) {
		let keys: Vec<Vec<Value>> = batch.iter().map(|row| policy::key_of::<T>(row)).collect();
		let mut sql = Sql::new(connection.dialect());
		sql.push("DELETE FROM ").table::<T>().push(" WHERE ");
		policy::keys_condition::<T>(&keys).write(&mut sql)?;

		if audit::records::<T>() {
			sql.push(" RETURNING ").columns(T::COLUMNS);
			removed += audited_destroy::<T>(connection, sql).await?;
		} else {
			removed += connection.execute(sql.finish()).await?;
		}
	}

	fallout.record(connection).await?;

	if T::SUPERTYPE.is_some() {
		let keys = rows
			.iter()
			.flat_map(|row| policy::key_of::<T>(row))
			.collect();
		T::remove_supertypes(keys, connection).await?;
	}

	Ok(removed)
}

/// Soft-deletes the rows an update matches and the rows their cascading
/// links reach, returning how many rows of `T` it deleted.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
async fn soft_delete_cascading<T: Table>(
	connection: &impl Connection,
	changes: &Changes<T>,
	policy: Option<&Expr>,
	now: crate::Timestamp,
) -> Result<u64> {
	let keys = changes.apply(connection, policy).await?;
	let deleted = keys.len() as u64;

	if let [_] = T::KEY {
		let keys = keys
			.into_iter()
			.filter_map(|key| key.into_iter().next())
			.collect();
		crate::cascade::soft_delete_from::<T>(connection, keys, now).await?;
	}

	Ok(deleted)
}

/// `ORDER BY`, `LIMIT`, and `OFFSET`.
pub(crate) fn write_order(sql: &mut Sql, order: &[Order], limit: Option<u64>, offset: Option<u64>) {
	for (index, order) in order.iter().enumerate() {
		sql.push(if index == 0 { " ORDER BY " } else { ", " });
		order.write(sql);
	}

	if let Some(limit) = limit {
		sql.push(&format!(" LIMIT {limit}"));
	}

	if let Some(offset) = offset {
		// SQLite only accepts OFFSET after a LIMIT; -1 means no limit.
		if limit.is_none() && sql.dialect() == leptoskit_db_schema::Dialect::Sqlite {
			sql.push(" LIMIT -1");
		}

		sql.push(&format!(" OFFSET {offset}"));
	}
}

/// `WHERE` the query's filter, the access policy's condition, and, unless
/// asked for, not soft-deleted.
fn write_where<T: Table>(
	sql: &mut Sql,
	filter: Option<&Expr>,
	policy: Option<&Expr>,
	with_deleted: bool,
) -> Result<()> {
	write_conditions::<T>(sql, filter, policy, with_deleted, false)
}

/// The conditions [`write_where`] writes, after `WHERE` and conditions of
/// the caller's own when `started`.
pub(crate) fn write_conditions<T: Table>(
	sql: &mut Sql,
	filter: Option<&Expr>,
	policy: Option<&Expr>,
	with_deleted: bool,
	started: bool,
) -> Result<()> {
	let mut first = !started;

	for condition in [filter, policy].into_iter().flatten() {
		sql.push(if first { " WHERE (" } else { " AND (" });
		condition.write(sql)?;
		sql.push(")");
		first = false;
	}

	if let Some(column) = T::DELETED_AT.filter(|_| !with_deleted) {
		sql.push(if first { " WHERE " } else { " AND " })
			.identifier(column)
			.push(" IS NULL");
	}

	Ok(())
}
