//! The rows whose vectors are nearest a given one.

use std::marker::PhantomData;

use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::SEARCH_ROW;
use leptoskit_db_schema::VectorDistance;
use leptoskit_db_schema::VectorElement;
use leptoskit_db_schema::VectorIndexKind;
use leptoskit_db_schema::VectorOptions;
use leptoskit_db_schema::quote_identifier;

use super::BitVector;
use super::HalfVector;
use super::SparseVector;
use super::Vector;
use super::column::VectorValue;
use crate::Action;
use crate::Col;
use crate::Connection;
use crate::DbError;
use crate::Expr;
use crate::NearestTo;
use crate::Order;
use crate::Query;
use crate::Read;
use crate::Result;
use crate::Statement;
use crate::Table;
use crate::Value;
use crate::connection::atomically;
use crate::policy;
use crate::sql::Sql;

macro_rules! nearest_vectors {
	($($ty:ident),*) => {$(
		impl<const N: usize> NearestTo<$ty<N>> for $ty<N> {
			type Search<T: Table> = Nearest<T, Self>;

			fn nearest<T: Table>(query: Query<T>, column: Col<T, Self>, target: &$ty<N>) -> Nearest<T, Self> {
				Nearest::new(query, column.name(), target)
			}
		}

		impl<const N: usize> NearestTo<$ty<N>> for Option<$ty<N>> {
			type Search<T: Table> = Nearest<T, $ty<N>>;

			fn nearest<T: Table>(query: Query<T>, column: Col<T, Self>, target: &$ty<N>) -> Nearest<T, $ty<N>> {
				Nearest::new(query, column.name(), target)
			}
		}
	)*};
}

nearest_vectors!(Vector, HalfVector, BitVector, SparseVector);

/// The rows whose vectors are nearest one, nearest first. Start one with
/// [`Query::nearest`] on a vector column:
///
/// ```ignore
/// let similar: Vec<(Article, f32)> = Article::query()
/// 	.filter(|a| a.published_at.is_not_null())
/// 	.nearest(|a| a.embedding, &query_embedding)
/// 	.with_distance()
/// 	.limit(10)
/// 	.all(&db)
/// 	.await?;
/// ```
///
/// Distances are measured as the column's `#[db(vector(distance = …))]`
/// says, smaller nearer, and ties are broken by the query's orderings, then
/// the key. The query's filters, `select` policy, and soft-delete filter
/// apply as they do to [`Query::all`].
///
/// Postgres orders by pgvector's operator, so an `hnsw` or `ivfflat` index
/// finds the rows approximately; with filters or policies it scans the
/// index until it has enough rows that match. SQLite measures every vector
/// exactly: with a limit, in the column's sqlite-vec `vec0` table, among
/// the rows the filters and policies allow; otherwise, and for inner
/// products and Jaccard distances, which sqlite-vec doesn't rank by, by
/// measuring each row.
#[must_use = "a search does nothing until it runs"]
pub struct Nearest<T, V> {
	query: Query<T>,
	column: &'static str,
	target: std::result::Result<Vec<u8>, String>,
	exact: bool,
	value: PhantomData<fn() -> V>,
}

/// [`Nearest`], returning each row's distance too.
#[must_use = "a search does nothing until it runs"]
pub struct NearestWithDistance<T, V>(Nearest<T, V>);

/// The alias of the measured distance.
const DISTANCE: &str = "lk_distance";

impl<T: Table, V: VectorValue> Nearest<T, V> {
	fn new(query: Query<T>, column: &'static str, target: &V) -> Self {
		Self {
			query,
			column,
			target: target.to_bytes(),
			exact: false,
			value: PhantomData,
		}
	}

	/// Keeps only rows matching `condition` as well.
	pub fn filter(mut self, condition: impl FnOnce(&T::Columns) -> Expr) -> Self {
		self.query = self.query.filter(condition);
		self
	}

	/// Breaks ties between rows at the same distance. The key breaks the
	/// rest.
	pub fn order_by(mut self, order: impl FnOnce(&T::Columns) -> Order) -> Self {
		self.query = self.query.order_by(order);
		self
	}

	/// At most this many rows.
	pub fn limit(mut self, limit: u64) -> Self {
		self.query = self.query.limit(limit);
		self
	}

	/// Skips this many rows.
	pub fn offset(mut self, offset: u64) -> Self {
		self.query = self.query.offset(offset);
		self
	}

	/// Includes soft-deleted rows.
	pub fn with_deleted(mut self) -> Self {
		self.query = self.query.with_deleted();
		self
	}

	/// Measures every row rather than asking an approximate index, so the
	/// rows are exactly the nearest, more slowly. On Postgres it turns index
	/// scans off for the search; SQLite always measures exactly.
	pub fn exact(mut self) -> Self {
		self.exact = true;
		self
	}

	/// Returns each row with its distance.
	pub fn with_distance(self) -> NearestWithDistance<T, V> {
		NearestWithDistance(self)
	}

	/// The rows, nearest first.
	pub async fn all(self, connection: &impl Connection) -> Result<Vec<T>> {
		Ok(self
			.rows(connection)
			.await?
			.into_iter()
			.map(|(row, _)| row)
			.collect())
	}

	/// The nearest row, if any.
	pub async fn first(self, connection: &impl Connection) -> Result<Option<T>> {
		Ok(self.limit(1).all(connection).await?.into_iter().next())
	}

	pub(crate) async fn rows(self, connection: &impl Connection) -> Result<Vec<(T, f32)>> {
		let target = self
			.target
			.clone()
			.map_err(|problem| crate::expr::invalid(&problem))?;
		let options = self.options();
		let distance = options.distance.ok_or_else(|| {
			crate::expr::invalid(&format!(
				"`{}.{}` has no distance to search by; declare one with `#[db(vector(distance = \
				 …))]`",
				T::flat_name(),
				self.column
			))
		})?;
		let policy = policy::condition::<T>(connection.access(), Action::Select);
		let search = Search {
			query: &self.query,
			column: self.column,
			options: &options,
			distance,
			target,
			exact: self.exact,
			policy: policy.as_ref(),
		};

		let rows = match connection.dialect() {
			Dialect::Postgres => search.postgres(connection).await?,
			Dialect::Sqlite => {
				connection
					.fetch(search.sqlite(connection.dialect())?)
					.await?
			}
		};

		rows.into_iter().map(decode::<T>).collect()
	}

	/// The column's vector options.
	fn options(&self) -> VectorOptions {
		T::definition()
			.vectors
			.into_iter()
			.find(|options| options.column == self.column)
			.unwrap_or_else(|| VectorOptions::new(self.column, None))
	}
}

impl<T: Table, V: VectorValue> NearestWithDistance<T, V> {
	/// The rows, nearest first, each with its distance.
	pub async fn all(self, connection: &impl Connection) -> Result<Vec<(T, f32)>> {
		self.0.rows(connection).await
	}

	/// The nearest row, if any, with its distance.
	pub async fn first(self, connection: &impl Connection) -> Result<Option<(T, f32)>> {
		Ok(Self(self.0.limit(1))
			.all(connection)
			.await?
			.into_iter()
			.next())
	}

	/// At most this many rows.
	pub fn limit(self, limit: u64) -> Self {
		Self(self.0.limit(limit))
	}

	/// Skips this many rows.
	pub fn offset(self, offset: u64) -> Self {
		Self(self.0.offset(offset))
	}

	/// Measures every row: see [`Nearest::exact`].
	pub fn exact(self) -> Self {
		Self(self.0.exact())
	}
}

/// One search, ready to write.
struct Search<'a, T> {
	query: &'a Query<T>,
	column: &'static str,
	options: &'a VectorOptions,
	distance: VectorDistance,
	target: Vec<u8>,
	exact: bool,
	policy: Option<&'a Expr>,
}

impl<T: Table> Search<'_, T> {
	/// Whether anything but the distance chooses the rows.
	fn is_filtered(&self) -> bool {
		self.query.filter.is_some()
			|| self.policy.is_some()
			|| (T::DELETED_AT.is_some() && !self.query.with_deleted)
	}

	/// The query's filter, policy, and soft-delete filter, after `WHERE` or
	/// after conditions of the caller's own when `started`.
	fn write_conditions(&self, sql: &mut Sql, started: bool) -> Result<()> {
		crate::query::write_conditions::<T>(
			sql,
			self.query.filter.as_ref(),
			self.policy,
			self.query.with_deleted,
			started,
		)
	}

	/// `, order…, key… LIMIT … OFFSET …`, after the distance.
	fn write_tail(&self, sql: &mut Sql, limit: Option<u64>, offset: Option<u64>) {
		for order in &self.query.order {
			sql.push(", ");
			order.write(sql);
		}

		for key in T::KEY {
			sql.push(", ").identifier(key).push(" ASC");
		}

		crate::query::write_order(sql, &[], limit, offset);
	}

	/// The search on Postgres. With an index, the index is scanned until
	/// enough rows match, rather than stopping at its first candidates
	/// (HNSW's `ef_search`, IVFFlat's `probes`) whatever the filters keep:
	/// in order for HNSW; IVFFlat's scan only roughly keeps order, so its
	/// rows are sorted again.
	async fn postgres(&self, connection: &impl Connection) -> Result<Vec<Vec<Value>>> {
		let Some(index) = self.options.index else {
			return connection.fetch(self.postgres_statement(None)?).await;
		};
		let (setting, order) = match index {
			_ if self.exact => ("enable_indexscan", "off"),
			VectorIndexKind::Hnsw => ("hnsw.iterative_scan", "strict_order"),
			VectorIndexKind::IvfFlat => ("ivfflat.iterative_scan", "relaxed_order"),
		};
		let set = |value: &str| {
			Statement {
				sql: format!("SET LOCAL {setting} = {value}"),
				args: Vec::new(),
			}
		};
		let statement = self.postgres_statement((!self.exact).then_some(index))?;

		atomically!(connection, |tx| {
			tx.execute(set(order)).await?;
			let rows = tx.fetch(statement).await?;
			tx.execute(set("DEFAULT")).await?;
			Ok(rows)
		})
	}

	/// `SELECT columns, distance FROM table WHERE vector IS NOT NULL AND …
	/// ORDER BY vector <op> target`, sorted again when an IVFFlat scan
	/// could have left it out of order.
	fn postgres_statement(&self, index: Option<VectorIndexKind>) -> Result<Statement> {
		let mut sql = Sql::new(Dialect::Postgres);
		let resorted = index == Some(VectorIndexKind::IvfFlat);
		let operator = format!(" {} ", self.distance.operator());

		if resorted {
			sql.push("SELECT ")
				.columns(T::COLUMNS)
				.push(", ")
				.identifier(DISTANCE)
				.push(" FROM (");
		}

		sql.push("SELECT ")
			.columns(T::COLUMNS)
			.push(", ")
			.identifier(self.column)
			.push(&operator)
			.value(Value::Bytes(self.target.clone()))
			.push(" AS ")
			.identifier(DISTANCE)
			.push(" FROM ")
			.table::<T>()
			.push(" WHERE ")
			.identifier(self.column)
			.push(" IS NOT NULL");
		self.write_conditions(&mut sql, true)?;
		sql.push(" ORDER BY ")
			.identifier(self.column)
			.push(&operator)
			.value(Value::Bytes(self.target.clone()));

		if resorted {
			let wanted = self
				.query
				.limit
				.map(|limit| limit.saturating_add(self.query.offset.unwrap_or(0)));
			self.write_tail(&mut sql, wanted, None);
			sql.push(") AS ")
				.identifier("lk_near")
				.push(" ORDER BY ")
				.identifier(DISTANCE);
		}

		self.write_tail(&mut sql, self.query.limit, self.query.offset);
		Ok(sql.finish())
	}

	/// The search on SQLite: in the column's `vec0` table when it has one
	/// and the search a limit, by measuring each row otherwise.
	fn sqlite(&self, dialect: Dialect) -> Result<Statement> {
		let element = T::definition()
			.column(self.column)
			.and_then(|column| column.ty.vector())
			.map(|(element, _)| element);

		if element == Some(VectorElement::Sparse) {
			return Err(crate::expr::invalid(
				"sparse vectors need Postgres: sqlite-vec has no sparse type",
			));
		}

		let bits = element == Some(VectorElement::Bit);

		match self.query.limit {
			Some(limit) if self.options.is_indexed(Dialect::Sqlite) => {
				self.sqlite_knn(dialect, bits, limit)
			}
			_ => self.sqlite_exact(dialect, bits),
		}
	}

	/// The `k` nearest vectors in the `vec0` table among the rows the
	/// filters and policies allow, joined to their rows through the table
	/// numbering them by key:
	///
	/// ```sql
	/// SELECT columns, lk_distance FROM table JOIN (
	/// 	SELECT keys, distance AS lk_distance FROM vec0 JOIN rows ON lk_row = rowid
	/// 	WHERE lk_vector MATCH ?1 AND k = ?2
	/// 		AND rowid IN (SELECT lk_row FROM rows WHERE keys IN (SELECT keys FROM table WHERE …))
	/// ) USING (keys) ORDER BY lk_distance, …
	/// ```
	fn sqlite_knn(&self, dialect: Dialect, bits: bool, limit: u64) -> Result<Statement> {
		let table = T::table_name();
		let vectors = quote_identifier(&self.options.vec_table(&table));
		let rows = quote_identifier(&self.options.rows_table(&table));
		let row = quote_identifier(SEARCH_ROW);
		let wanted = limit.saturating_add(self.query.offset.unwrap_or(0));
		let keys: Vec<String> = T::KEY.iter().map(|key| quote_identifier(key)).collect();
		let keys = keys.join(", ");
		let mut sql = Sql::new(dialect);

		sql.push("SELECT ")
			.columns(T::COLUMNS)
			.push(", ")
			.identifier(DISTANCE)
			.push(" FROM ")
			.table::<T>()
			.push(&format!(
				" JOIN (SELECT \"lk_rows\".{}, \"lk_knn\".\"distance\" AS {} FROM {vectors} AS \
				 \"lk_knn\" JOIN {rows} AS \"lk_rows\" ON \"lk_rows\".{row} = \
				 \"lk_knn\".\"rowid\" WHERE \"lk_knn\".\"lk_vector\" MATCH ",
				T::KEY
					.iter()
					.map(|key| quote_identifier(key))
					.collect::<Vec<_>>()
					.join(", \"lk_rows\"."),
				quote_identifier(DISTANCE)
			));
		write_target(&mut sql, bits, &self.target);
		sql.push(" AND \"lk_knn\".\"k\" = ")
			.value(Value::Int(i64::try_from(wanted).unwrap_or(i64::MAX)));

		if self.is_filtered() {
			sql.push(&format!(
				" AND \"lk_knn\".\"rowid\" IN (SELECT {row} FROM {rows} WHERE ({keys}) IN (SELECT \
				 {keys} FROM "
			))
			.table::<T>();
			self.write_conditions(&mut sql, false)?;
			sql.push("))");
		}

		sql.push(&format!(") AS \"lk_near\" USING ({keys}) ORDER BY "))
			.identifier(DISTANCE);
		self.write_tail(&mut sql, self.query.limit, self.query.offset);
		Ok(sql.finish())
	}

	/// Every row with a vector, measured: `SELECT columns, distance FROM
	/// table WHERE vector IS NOT NULL AND … ORDER BY distance`.
	fn sqlite_exact(&self, dialect: Dialect, bits: bool) -> Result<Statement> {
		let function = match self.distance {
			VectorDistance::L2 => "vec_distance_l2",
			VectorDistance::Cosine => "vec_distance_cosine",
			VectorDistance::L1 => "vec_distance_l1",
			VectorDistance::Hamming => "vec_distance_hamming",
			VectorDistance::InnerProduct => "lk_vec_distance_ip",
			VectorDistance::Jaccard => "lk_vec_distance_jaccard",
		};
		// sqlite-vec reads a bare BLOB as floats; its Hamming distance needs
		// both marked as bits.
		let marked = bits && self.distance == VectorDistance::Hamming;
		let mut sql = Sql::new(dialect);
		sql.push("SELECT ")
			.columns(T::COLUMNS)
			.push(&format!(", {function}("));

		if marked {
			sql.push("vec_bit(").identifier(self.column).push(")");
		} else {
			sql.identifier(self.column);
		}

		sql.push(", ");
		write_target(&mut sql, marked, &self.target);
		sql.push(") AS ")
			.identifier(DISTANCE)
			.push(" FROM ")
			.table::<T>()
			.push(" WHERE ")
			.identifier(self.column)
			.push(" IS NOT NULL");
		self.write_conditions(&mut sql, true)?;
		sql.push(" ORDER BY ").identifier(DISTANCE);
		self.write_tail(&mut sql, self.query.limit, self.query.offset);
		Ok(sql.finish())
	}
}

/// The target vector, marked as bits for sqlite-vec when it is them.
fn write_target(sql: &mut Sql, bits: bool, target: &[u8]) {
	if bits {
		sql.push("vec_bit(")
			.value(Value::Bytes(target.to_vec()))
			.push(")");
	} else {
		sql.value(Value::Bytes(target.to_vec()));
	}
}

fn decode<T: Table>(mut values: Vec<Value>) -> Result<(T, f32)> {
	let distance = match values.pop() {
		Some(Value::Float(distance)) => distance as f32,
		Some(Value::Int(distance)) => distance as f32,
		_ => {
			return Err(DbError::Decode(crate::DecodeError::custom(
				"the nearest rows came back without their distance",
			)));
		}
	};

	Ok((T::from_values(values)?, distance))
}
