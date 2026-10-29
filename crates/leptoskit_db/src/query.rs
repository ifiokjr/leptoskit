//! Reading rows, and changing many at once.

use std::marker::PhantomData;

use crate::Col;
use crate::Column;
use crate::Connection;
use crate::DbError;
use crate::Expr;
use crate::Operand;
use crate::Order;
use crate::Result;
use crate::Table;
use crate::Value;
use crate::sql::Sql;

/// A query on table `T`. Start one with `User::query()`.
///
/// Rows soft-deleted through a `#[db(deleted_at)]` column are left out unless
/// the query asks for them with [`Query::with_deleted`].
#[must_use = "a query does nothing until it runs"]
pub struct Query<T> {
	filter: Option<Expr>,
	order: Vec<Order>,
	limit: Option<u64>,
	offset: Option<u64>,
	with_deleted: bool,
	table: PhantomData<fn() -> T>,
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

	/// Every matching row.
	pub async fn all(self, connection: &impl Connection) -> Result<Vec<T>> {
		let mut sql = Sql::new(connection.dialect());
		sql.push("SELECT ")
			.columns(T::COLUMNS)
			.push(" FROM ")
			.table::<T>();
		self.write_filter(&mut sql)?;

		for (index, order) in self.order.iter().enumerate() {
			sql.push(if index == 0 { " ORDER BY " } else { ", " });
			order.write(&mut sql);
		}

		if let Some(limit) = self.limit {
			sql.push(&format!(" LIMIT {limit}"));
		}

		if let Some(offset) = self.offset {
			// SQLite only accepts OFFSET after a LIMIT; -1 means no limit.
			if self.limit.is_none() && connection.dialect() == leptoskit_db_schema::Dialect::Sqlite
			{
				sql.push(" LIMIT -1");
			}

			sql.push(&format!(" OFFSET {offset}"));
		}

		let rows = connection.fetch(sql.finish()).await?;
		rows.into_iter()
			.map(|values| T::from_values(values).map_err(DbError::from))
			.collect()
	}

	/// The first matching row, if any.
	pub async fn first(self, connection: &impl Connection) -> Result<Option<T>> {
		Ok(self.limit(1).all(connection).await?.into_iter().next())
	}

	/// The first matching row, or [`DbError::NotFound`].
	pub async fn one(self, connection: &impl Connection) -> Result<T> {
		self.first(connection).await?.ok_or_else(|| {
			DbError::NotFound {
				table: T::flat_name(),
			}
		})
	}

	/// How many rows match.
	pub async fn count(self, connection: &impl Connection) -> Result<u64> {
		let mut sql = Sql::new(connection.dialect());
		sql.push("SELECT count(*) FROM ").table::<T>();
		self.write_filter(&mut sql)?;

		match connection
			.fetch(sql.finish())
			.await?
			.first()
			.and_then(|row| row.first())
		{
			Some(Value::Int(count)) => Ok(u64::try_from(*count).unwrap_or_default()),
			_ => {
				Err(DbError::Decode(crate::DecodeError::custom(
					"count(*) returned no integer",
				)))
			}
		}
	}

	/// Whether any row matches.
	pub async fn exists(self, connection: &impl Connection) -> Result<bool> {
		Ok(self
			.limit(1)
			.all(connection)
			.await?
			.into_iter()
			.next()
			.is_some())
	}

	/// Deletes every matching row: soft-deletes them when the table has a
	/// `#[db(deleted_at)]` column, along with the rows their
	/// `on_soft_delete = cascade` links reach, in one transaction. Returns how
	/// many rows of this table changed.
	pub async fn delete(self, connection: &impl Connection) -> Result<u64> {
		let Some(column) = T::DELETED_AT else {
			return self.destroy(connection).await;
		};

		let now = crate::now();
		let mut update = Update::<T>::new();
		update.filter = self.filter;
		update.sets.push((column, Ok(Value::Timestamp(now))));

		#[cfg(any(feature = "postgres", feature = "sqlite"))]
		if let [key] = T::KEY
			&& crate::cascade::has_cascades::<T>()
		{
			return match connection.begin_unless_in_transaction().await? {
				Some(tx) => {
					let result = soft_delete_cascading(&tx, update, key, now).await;
					crate::connection::finish(tx, result).await
				}
				None => soft_delete_cascading(connection, update, key, now).await,
			};
		}

		#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
		let _ = now;

		update.run(connection).await
	}

	/// Removes every matching row for good, soft-deleted or not.
	pub async fn destroy(mut self, connection: &impl Connection) -> Result<u64> {
		self.with_deleted = true;
		let mut sql = Sql::new(connection.dialect());
		sql.push("DELETE FROM ").table::<T>();
		self.write_filter(&mut sql)?;
		connection.execute(sql.finish()).await
	}

	fn write_filter(&self, sql: &mut Sql) -> Result<()> {
		write_where::<T>(sql, self.filter.as_ref(), self.with_deleted)
	}
}

/// Changes columns on every matching row. Start one with `User::update()`.
///
/// `updated_at` is set too. Soft-deleted rows are left alone unless
/// [`Update::with_deleted`] asks for them.
#[must_use = "an update does nothing until it runs"]
pub struct Update<T> {
	filter: Option<Expr>,
	sets: Vec<(&'static str, Result<Value, crate::EncodeError>)>,
	with_deleted: bool,
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
			.push((column(T::columns()).name(), value.into_value()));
		self
	}

	/// Includes soft-deleted rows.
	pub fn with_deleted(mut self) -> Self {
		self.with_deleted = true;
		self
	}

	/// Runs the update and returns how many rows changed.
	pub async fn run(self, connection: &impl Connection) -> Result<u64> {
		let sql = self.statement(connection.dialect())?;
		connection.execute(sql.finish()).await
	}

	fn statement(self, dialect: crate::Dialect) -> Result<Sql> {
		let mut sql = Sql::new(dialect);
		sql.push("UPDATE ").table::<T>().push(" SET ");
		let touch = T::UPDATED_AT.filter(|column| !self.sets.iter().any(|(set, _)| set == column));
		let sets = self
			.sets
			.into_iter()
			.map(|(column, value)| Ok((column, value?)));
		let touched = touch.map(|column| Ok((column, Value::Timestamp(crate::now()))));

		let mut first = true;

		for set in sets.chain(touched) {
			let (column, value): (&str, Value) = set.map_err(DbError::Encode)?;

			if !first {
				sql.push(", ");
			}

			first = false;
			sql.identifier(column).push(" = ").value(value);
		}

		// A bulk write changes versioned rows, so a save based on an earlier
		// read must see a new version.
		if let Some(version) = T::VERSION {
			if !first {
				sql.push(", ");
			}

			sql.identifier(version)
				.push(" = ")
				.identifier(version)
				.push(" + 1");
		}

		write_where::<T>(&mut sql, self.filter.as_ref(), self.with_deleted)?;
		Ok(sql)
	}
}

/// Soft-deletes the rows an update matches and the rows their cascading
/// links reach, returning how many rows of `T` it deleted.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
async fn soft_delete_cascading<T: Table>(
	connection: &impl Connection,
	update: Update<T>,
	key: &str,
	now: crate::Timestamp,
) -> Result<u64> {
	let mut sql = update.statement(connection.dialect())?;
	sql.push(" RETURNING ").identifier(key);
	let keys: Vec<Value> = connection
		.fetch(sql.finish())
		.await?
		.into_iter()
		.filter_map(|row| row.into_iter().next())
		.collect();
	let deleted = keys.len() as u64;
	crate::cascade::soft_delete_linked(connection, T::table_name(), keys, now).await?;
	Ok(deleted)
}

fn write_where<T: Table>(sql: &mut Sql, filter: Option<&Expr>, with_deleted: bool) -> Result<()> {
	let soft_delete = T::DELETED_AT.filter(|_| !with_deleted);

	if filter.is_none() && soft_delete.is_none() {
		return Ok(());
	}

	sql.push(" WHERE ");

	if let Some(filter) = filter {
		sql.push("(");
		filter.write(sql)?;
		sql.push(")");
	}

	if let Some(column) = soft_delete {
		if filter.is_some() {
			sql.push(" AND ");
		}

		sql.identifier(column).push(" IS NULL");
	}

	Ok(())
}
