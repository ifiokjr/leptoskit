//! The rows whose shapes are nearest a given one.

use std::marker::PhantomData;

use leptoskit_db_schema::Dialect;

use super::GeoValue;
use super::Space;
use super::column::GeoColumn;
use super::query::Argument;
use super::query::Measured;
use super::query::MeasuresDistance;
use super::query::Source;
use super::query::geodesic_box;
use super::query::write_candidates;
use super::query::write_measure;
use crate::Action;
use crate::Col;
use crate::DbError;
use crate::Expr;
use crate::Order;
use crate::Query;
use crate::Read;
use crate::Result;
use crate::Table;
use crate::Value;
use crate::policy;
use crate::sql::Sql;
use crate::tracked::ReadKind;
use crate::tracked::Reads;

impl<T: Table> Query<T> {
	/// The matching rows nearest `target`, nearest first, by the shapes in
	/// `column`; rows without a shape are left out. Give it a
	/// [`limit`](Nearest::limit):
	///
	/// ```ignore
	/// let nearby: Vec<(Place, Meters)> = Place::query()
	/// 	.filter(|p| p.open.eq(true))
	/// 	.nearest(|p| p.location, &here)
	/// 	.with_distance()
	/// 	.limit(20)
	/// 	.all(&db)
	/// 	.await?;
	/// ```
	///
	/// Distances are measured as [`Col::distance_to`] measures them, and ties
	/// are broken by the query's orderings, then the key. Postgres orders
	/// planar shapes with its index (`<->`); on the globe it orders by the
	/// exact spheroid distance, which its index can't, so a `within_distance`
	/// filter beside it keeps large tables fast. SQLite grows a search box in
	/// the column's R*Tree until it holds enough rows.
	pub fn nearest<V, G>(
		self,
		column: impl FnOnce(&T::Columns) -> Col<T, V>,
		target: &G,
	) -> Nearest<T, <V::Value as GeoValue>::Space>
	where
		V: GeoColumn,
		G: GeoValue<Space = <V::Value as GeoValue>::Space>,
		<V::Value as GeoValue>::Space:
			MeasuresDistance<<V::Value as GeoValue>::Shape, <G as GeoValue>::Shape>,
	{
		let column = column(T::columns());

		Nearest {
			query: self,
			source: column.source(),
			target: Argument::of(target),
			space: PhantomData,
		}
	}
}

/// The rows nearest a shape, nearest first. Start one with
/// [`Query::nearest`].
#[must_use = "a search does nothing until it runs"]
pub struct Nearest<T, P> {
	query: Query<T>,
	source: Source,
	target: std::result::Result<Argument, String>,
	space: PhantomData<fn() -> P>,
}

/// [`Nearest`], returning each row's distance too.
#[must_use = "a search does nothing until it runs"]
pub struct NearestWithDistance<T, P>(Nearest<T, P>);

impl<T: Table, P: Space> Nearest<T, P> {
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

	/// Returns each row with its distance: [`Meters`](super::Meters) on the
	/// globe, the coordinate system's units otherwise.
	pub fn with_distance(self) -> NearestWithDistance<T, P> {
		NearestWithDistance(self)
	}

	/// The rows, nearest first.
	pub async fn all(self, connection: &impl Read) -> Result<Vec<T>> {
		Ok(self
			.rows(connection)
			.await?
			.into_iter()
			.map(|(row, _)| row)
			.collect())
	}

	/// The nearest row, if any.
	pub async fn first(self, connection: &impl Read) -> Result<Option<T>> {
		Ok(self.limit(1).all(connection).await?.into_iter().next())
	}

	async fn rows(self, connection: &impl Read) -> Result<Vec<(T, f64)>> {
		let target = self
			.target
			.clone()
			.map_err(|problem| crate::expr::invalid(&problem))?;
		let wanted = self
			.query
			.limit
			.map(|limit| limit.saturating_add(self.query.offset.unwrap_or(0)));
		let indexed = self.source.column().and_then(|(_, indexed)| indexed);

		let rows = match (connection.dialect(), wanted, indexed, target.envelope()) {
			(Dialect::Sqlite, Some(wanted), Some(_), Some(envelope)) if wanted > 0 => {
				self.grow(connection, &target, wanted, envelope).await?
			}
			_ => {
				connection
					.read(self.statement(connection, &target, None)?, || {
						self.reads(connection)
					})
					.await?
			}
		};

		rows.into_iter().map(decode::<T>).collect()
	}

	/// The search on SQLite: rows within `radius` of the target, in a box
	/// the R*Tree finds, doubling the radius until `wanted` rows are in it,
	/// or the box covers every row, when the whole table is ordered instead.
	/// Rows at most `radius` away include every row nearer than the furthest
	/// of them, so the first `wanted` are the nearest.
	async fn grow(
		&self,
		connection: &impl Read,
		target: &Argument,
		wanted: u64,
		envelope: [f64; 4],
	) -> Result<Vec<Vec<Value>>> {
		let Some(extent) = self.extent(connection).await? else {
			return Ok(Vec::new());
		};
		let (width, height) = (extent[1] - extent[0], extent[3] - extent[2]);
		// A first guess: the radius of a box holding `wanted` of the
		// indexed boxes, were they spread evenly.
		let mut radius = (width.max(height) / 2.0) * (wanted as f64 / extent[4]).sqrt();

		if P::GEOGRAPHY {
			radius *= 111_320.0;
		}

		radius = radius.max(f64::EPSILON);

		for _ in 0..64 {
			let search = Self::search_box(envelope, radius);
			let covers = search[0] <= extent[0]
				&& search[1] >= extent[1]
				&& search[2] <= extent[2]
				&& search[3] >= extent[3];

			if covers {
				break;
			}

			let rows = connection
				.read(
					self.statement(connection, target, Some((search, radius)))?,
					|| Ok(Reads::default()),
				)
				.await?;

			if rows.len() as u64 >= wanted {
				return Ok(rows);
			}

			radius *= 2.0;
		}

		connection
			.read(self.statement(connection, target, None)?, || {
				Ok(Reads::default())
			})
			.await
	}

	/// What the search reads, for a read set: the query's rows with a
	/// shape, nearest first, which only the database can order.
	fn reads(&self, connection: &impl Read) -> Result<Reads> {
		let (column, _) = self.source.column().expect("nearest reads a column");
		Ok(self
			.query
			.reads(connection, ReadKind::Rows)?
			.narrowed(
				Expr::Null {
					column,
					negated: true,
				} & Expr::Opaque("the distance to a place"),
			)
			.tail(&[], self.query.limit, self.query.offset))
	}

	/// The box around the target that holds every shape within `radius`.
	fn search_box(envelope: [f64; 4], radius: f64) -> [f64; 4] {
		if P::GEOGRAPHY {
			geodesic_box(envelope, radius)
		} else {
			[
				envelope[0] - radius,
				envelope[1] + radius,
				envelope[2] - radius,
				envelope[3] + radius,
			]
		}
	}

	/// The box around every indexed shape, and how many there are: `[min_x,
	/// max_x, min_y, max_y, count]`, or `None` with none.
	async fn extent(&self, connection: &impl Read) -> Result<Option<[f64; 5]>> {
		let Some((_, Some(indexed))) = self.source.column() else {
			return Ok(None);
		};
		// The search's first statement, so it records what the search reads;
		// it returns the index's extent, not rows, so no keys.
		let statement = crate::Statement {
			sql: format!(
				"SELECT min(minx), max(maxx), min(miny), max(maxy), count(*) FROM {}",
				leptoskit_db_schema::quote_identifier(&indexed.rtree_table())
			),
			args: Vec::new(),
		};
		let rows = connection
			.read(statement, || {
				let mut reads = self.reads(connection)?;
				reads.keys = None;
				Ok(reads)
			})
			.await?;
		let float = |value: &Value| {
			match value {
				Value::Float(value) => Some(*value),
				Value::Int(value) => Some(*value as f64),
				_ => None,
			}
		};
		let Some(row) = rows.first() else {
			return Ok(None);
		};
		let values: Option<Vec<f64>> = row.iter().map(float).collect();

		Ok(values
			.and_then(|values| values.try_into().ok())
			.filter(|extent: &[f64; 5]| extent[4] > 0.0))
	}

	/// `SELECT columns, distance FROM table WHERE shape IS NOT NULL AND …
	/// ORDER BY distance`, within `radius` of the target in `search` when
	/// given.
	fn statement(
		&self,
		connection: &impl Read,
		target: &Argument,
		within: Option<([f64; 4], f64)>,
	) -> Result<crate::Statement> {
		let mut sql = Sql::new(connection.dialect());
		let distance = Measured::Distance(target.clone());
		let (column, indexed) = self.source.column().expect("nearest reads a column");
		sql.push("SELECT ").columns(T::COLUMNS).push(", ");
		write_measure(&mut sql, &self.source, P::GEOGRAPHY, &distance)?;
		sql.push(" FROM ")
			.table::<T>()
			.push(" WHERE ")
			.identifier(column)
			.push(" IS NOT NULL");

		if let (Some((search, radius)), Some(indexed)) = (within, indexed) {
			sql.push(" AND ");
			write_candidates(&mut sql, indexed, search);
			sql.push(" AND ");
			write_measure(&mut sql, &self.source, P::GEOGRAPHY, &distance)?;
			sql.push(" <= ").value(Value::Float(radius));
		}

		let policy = policy::condition::<T>(connection.access(), Action::Select);
		crate::query::write_conditions::<T>(
			&mut sql,
			self.query.filter.as_ref(),
			policy.as_ref(),
			self.query.with_deleted,
			true,
		)?;

		// The distance follows the row's own columns.
		match (sql.dialect(), P::GEOGRAPHY) {
			(Dialect::Postgres, false) => {
				sql.push(" ORDER BY ").identifier(column).push(" <-> ");
				target.write(&mut sql)?;
			}
			_ => {
				sql.push(&format!(" ORDER BY {}", T::COLUMNS.len() + 1));
			}
		}

		for order in &self.query.order {
			sql.push(", ");
			order.write(&mut sql);
		}

		for key in T::KEY {
			sql.push(", ").identifier(key).push(" ASC");
		}

		crate::query::write_order(&mut sql, &[], self.query.limit, self.query.offset);
		Ok(sql.finish())
	}
}

impl<T: Table, P: Space> NearestWithDistance<T, P> {
	/// The rows, nearest first, each with its distance.
	pub async fn all(self, connection: &impl Read) -> Result<Vec<(T, P::Distance)>> {
		Ok(self
			.0
			.rows(connection)
			.await?
			.into_iter()
			.map(|(row, distance)| (row, P::Distance::from(distance)))
			.collect())
	}

	/// The nearest row, if any, with its distance.
	pub async fn first(self, connection: &impl Read) -> Result<Option<(T, P::Distance)>> {
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
}

fn decode<T: Table>(mut values: Vec<Value>) -> Result<(T, f64)> {
	let distance = match values.pop() {
		Some(Value::Float(distance)) => distance,
		Some(Value::Int(distance)) => distance as f64,
		_ => {
			return Err(DbError::Decode(crate::DecodeError::custom(
				"the nearest rows came back without their distance",
			)));
		}
	};

	Ok((T::from_values(values)?, distance))
}
