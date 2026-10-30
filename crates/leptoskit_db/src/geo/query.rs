//! Spatial filters and measurements on geospatial columns.
//!
//! Each becomes one SQL condition: a PostGIS function on Postgres, and on
//! SQLite one of the functions the database layer registers, narrowed first
//! by the column's R*Tree when it has `#[db(geo(index))]`.

use std::marker::PhantomData;

use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::GeoIndex;
use leptoskit_db_schema::SEARCH_ROW;
use leptoskit_db_schema::TableName;
use leptoskit_db_schema::quote_identifier;

use super::Areal;
use super::CoordinateSystem;
use super::GeoValue;
use super::Geometry;
use super::Linear;
use super::Planar;
use super::Point;
use super::Shape;
use super::Space;
use super::Spheroid;
use super::codec;
use super::column::GeoColumn;
use crate::Col;
use crate::DbError;
use crate::Expr;
use crate::Table;
use crate::Value;
use crate::expr::Comparison;
use crate::sql::Sql;

/// The spaces in which distances between shapes `A` and `B` can be
/// measured alike on both databases: any planar shapes, and points on the
/// spheroid, which both measure along the geodesic (Karney's algorithm).
#[diagnostic::on_unimplemented(
	message = "distances between a `{A}` and a `{B}` aren't measured in `{Self}`",
	label = "on the globe, `Geography` measures distances between points; use `Geo<T, Srid<N>>` \
	         in a projected system for other shapes"
)]
pub trait MeasuresDistance<A, B> {}

impl<A: Shape, B: Shape, S: CoordinateSystem> MeasuresDistance<A, B> for Planar<S> {}
impl MeasuresDistance<Point, Point> for Spheroid {}

/// What a spatial condition reads: a column, or a shape PostGIS computes
/// from one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Source {
	Column {
		name: &'static str,
		/// The column's spatial index on SQLite, when it has one.
		indexed: Option<Indexed>,
	},
	Buffer(Box<Source>, f64),
	Simplify(Box<Source>, f64),
	Transform(Box<Source>, u32),
	Union(Box<Source>, Argument),
	/// A shape that couldn't be made; the query fails when it runs.
	Invalid(String),
}

/// A spatially indexed column: its table, the table's key, and the index.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Indexed {
	table: TableName,
	key: &'static [&'static str],
	index: GeoIndex,
}

impl Indexed {
	/// The R*Tree holding the boxes.
	pub(crate) fn rtree_table(&self) -> String {
		self.index.rtree_table(&self.table)
	}
}

/// A shape given to a condition, in its coordinate system.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Argument {
	geometry: Geometry,
	srid: u32,
	geography: bool,
}

impl Argument {
	pub(crate) fn of<G: GeoValue>(value: &G) -> Result<Self, String> {
		value.check()?;
		Ok(Self {
			geometry: value.shape().to_geometry(),
			srid: G::Space::SRID,
			geography: G::Space::GEOGRAPHY,
		})
	}

	pub(crate) fn envelope(&self) -> Option<[f64; 4]> {
		super::envelope(&self.geometry)
	}

	/// The shape as a bound value: `ST_GeomFromEWKB($n)` on Postgres, cast
	/// to geography for geography, and GeoPackage bytes on SQLite.
	pub(crate) fn write(&self, sql: &mut Sql) -> Result<(), DbError> {
		match sql.dialect() {
			Dialect::Postgres => {
				let bytes = codec::encode_ewkb(&self.geometry, self.srid).map_err(invalid)?;
				sql.push("ST_GeomFromEWKB(")
					.value(Value::Bytes(bytes))
					.push(")");

				if self.geography {
					sql.push("::geography");
				}
			}
			Dialect::Sqlite => {
				let bytes = codec::encode_gpkg(&self.geometry, self.srid).map_err(invalid)?;
				sql.value(Value::Bytes(bytes));
			}
		}

		Ok(())
	}
}

fn invalid(message: impl std::fmt::Display) -> DbError {
	crate::expr::invalid(&message.to_string())
}

impl Source {
	/// The column, when the source is a column itself.
	pub(crate) fn column(&self) -> Option<(&'static str, Option<&Indexed>)> {
		match self {
			Self::Column { name, indexed } => Some((name, indexed.as_ref())),
			_ => None,
		}
	}

	pub(crate) fn write(&self, sql: &mut Sql) -> Result<(), DbError> {
		match (self, sql.dialect()) {
			(Self::Column { name, .. }, _) => {
				sql.identifier(name);
			}
			(Self::Invalid(problem), _) => return Err(invalid(problem)),
			(Self::Buffer(inner, distance), Dialect::Postgres) => {
				sql.push("ST_Buffer(");
				inner.write(sql)?;
				sql.push(", ").value(Value::Float(*distance)).push(")");
			}
			(Self::Simplify(inner, tolerance), Dialect::Postgres) => {
				sql.push("ST_SimplifyPreserveTopology(");
				inner.write(sql)?;
				sql.push(", ").value(Value::Float(*tolerance)).push(")");
			}
			(Self::Transform(inner, srid), Dialect::Postgres) => {
				sql.push("ST_Transform(");
				inner.write(sql)?;
				sql.push(&format!("::geometry, {srid})"));
			}
			(Self::Union(inner, other), Dialect::Postgres) => {
				sql.push("ST_Union(");
				inner.write(sql)?;
				sql.push(", ");
				other.write(sql)?;
				sql.push(")");
			}
			(_, Dialect::Sqlite) => {
				return Err(invalid(
					"buffer, simplify, transform, and union are PostGIS operations, which \
					 SQLite's geospatial layer doesn't have; use Postgres, or compute the shape \
					 in Rust with `leptoskit::db::geo::algorithms`",
				));
			}
		}

		Ok(())
	}
}

/// A spatial condition.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct SpatialExpr {
	source: Source,
	geography: bool,
	kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Predicate {
	Intersects,
	Contains,
	Within,
	Touches,
	Disjoint,
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
	Predicate(Predicate, Argument),
	WithinDistance(Argument, f64),
	Measure(Measured, Comparison, f64),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Measured {
	Distance(Argument),
	Area,
	Length,
}

impl SpatialExpr {
	pub(crate) fn write(&self, sql: &mut Sql) -> Result<(), DbError> {
		match &self.kind {
			Kind::Predicate(predicate, other) => self.write_predicate(sql, *predicate, other),
			Kind::WithinDistance(other, distance) => {
				let narrowed = self.narrow(sql, other.envelope(), *distance);
				let function = match (sql.dialect(), self.geography) {
					(Dialect::Postgres, _) => "ST_DWithin(",
					(Dialect::Sqlite, false) => "lk_st_dwithin(",
					(Dialect::Sqlite, true) => "lk_st_geodesic_dwithin(",
				};
				sql.push(function);
				self.source.write(sql)?;
				sql.push(", ");
				other.write(sql)?;
				sql.push(", ").value(Value::Float(*distance)).push(")");
				close(sql, narrowed);
				Ok(())
			}
			Kind::Measure(measured, comparison, bound) => {
				write_measure(sql, &self.source, self.geography, measured)?;
				sql.push(comparison.operator()).value(Value::Float(*bound));
				Ok(())
			}
		}
	}

	fn write_predicate(
		&self,
		sql: &mut Sql,
		predicate: Predicate,
		other: &Argument,
	) -> Result<(), DbError> {
		let narrowed = if predicate == Predicate::Disjoint {
			false
		} else {
			self.narrow(sql, other.envelope(), 0.0)
		};
		let (function, negated, as_geometry) = match (sql.dialect(), self.geography, predicate) {
			(Dialect::Postgres, _, Predicate::Intersects | Predicate::Disjoint) => {
				("ST_Intersects", predicate == Predicate::Disjoint, false)
			}
			(Dialect::Postgres, false, Predicate::Contains) => ("ST_Contains", false, false),
			(Dialect::Postgres, false, Predicate::Within) => ("ST_Within", false, false),
			// PostGIS's geography has `ST_Covers` and `ST_CoveredBy`, which
			// count the boundary as inside, and no `ST_Touches`.
			(Dialect::Postgres, true, Predicate::Contains) => ("ST_Covers", false, false),
			(Dialect::Postgres, true, Predicate::Within) => ("ST_CoveredBy", false, false),
			(Dialect::Postgres, geography, Predicate::Touches) => ("ST_Touches", false, geography),
			(Dialect::Sqlite, _, Predicate::Intersects | Predicate::Disjoint) => {
				("lk_st_intersects", predicate == Predicate::Disjoint, false)
			}
			(Dialect::Sqlite, false, Predicate::Contains) => ("lk_st_contains", false, false),
			(Dialect::Sqlite, false, Predicate::Within) => ("lk_st_within", false, false),
			(Dialect::Sqlite, true, Predicate::Contains) => ("lk_st_covers", false, false),
			(Dialect::Sqlite, true, Predicate::Within) => ("lk_st_coveredby", false, false),
			(Dialect::Sqlite, _, Predicate::Touches) => ("lk_st_touches", false, false),
		};
		let cast = if as_geometry { "::geometry" } else { "" };

		if negated {
			sql.push("NOT ");
		}

		sql.push(function).push("(");
		self.source.write(sql)?;
		sql.push(cast).push(", ");
		other.write(sql)?;
		sql.push(cast).push(")");
		close(sql, narrowed);
		Ok(())
	}

	/// On SQLite, for an indexed column, opens `(key) IN (rows whose box
	/// meets the envelope grown by distance) AND (`, so the exact test only
	/// runs on candidates. Returns whether it did, so [`close`] ends it.
	fn narrow(&self, sql: &mut Sql, envelope: Option<[f64; 4]>, distance: f64) -> bool {
		let Some((_, Some(indexed))) = self.source.column() else {
			return false;
		};

		if sql.dialect() != Dialect::Sqlite {
			return false;
		}

		let Some(envelope) = envelope else {
			return false;
		};
		let search = if self.geography {
			geodesic_box(envelope, distance)
		} else {
			[
				envelope[0] - distance,
				envelope[1] + distance,
				envelope[2] - distance,
				envelope[3] + distance,
			]
		};

		sql.push("(");
		write_candidates(sql, indexed, search);
		sql.push(" AND ");
		true
	}
}

/// `(key) IN (SELECT key FROM rows JOIN rtree … WHERE the box meets
/// search)`, the keys of the rows whose bounding box meets `search`
/// (`[min_x, max_x, min_y, max_y]`).
pub(crate) fn write_candidates(sql: &mut Sql, indexed: &Indexed, search: [f64; 4]) {
	let rows = quote_identifier(&indexed.index.rows_table(&indexed.table));
	let rtree = quote_identifier(&indexed.index.rtree_table(&indexed.table));
	let keys: Vec<String> = indexed
		.key
		.iter()
		.map(|key| quote_identifier(key))
		.collect();
	let selected: Vec<String> = keys
		.iter()
		.map(|key| format!("\"lk_geo_rows\".{key}"))
		.collect();

	if let [key] = keys.as_slice() {
		sql.push(key);
	} else {
		sql.push(&format!("({})", keys.join(", ")));
	}

	sql.push(&format!(
		" IN (SELECT {} FROM {rows} AS \"lk_geo_rows\" JOIN {rtree} AS \"lk_geo_box\" ON \
		 \"lk_geo_box\".\"id\" = \"lk_geo_rows\".{} WHERE \"lk_geo_box\".\"maxx\" >= ",
		selected.join(", "),
		quote_identifier(SEARCH_ROW)
	))
	.value(Value::Float(search[0]))
	.push(" AND \"lk_geo_box\".\"minx\" <= ")
	.value(Value::Float(search[1]))
	.push(" AND \"lk_geo_box\".\"maxy\" >= ")
	.value(Value::Float(search[2]))
	.push(" AND \"lk_geo_box\".\"miny\" <= ")
	.value(Value::Float(search[3]))
	.push(")");
}

fn close(sql: &mut Sql, narrowed: bool) {
	if narrowed {
		sql.push(")");
	}
}

/// The smallest radius of curvature of the WGS 84 spheroid, the meridian's
/// at the equator: converting a distance with it overestimates the angle.
const MIN_RADIUS: f64 = 6_335_439.0;

/// A longitude and latitude box holding every point within `meters` of
/// `envelope` along the spheroid, generously: the full range of longitudes
/// when the box would reach a pole or cross the antimeridian.
pub(crate) fn geodesic_box(envelope: [f64; 4], meters: f64) -> [f64; 4] {
	let angle = (meters.max(0.0) * 1.01 + 1.0) / MIN_RADIUS;
	let degrees = angle.to_degrees();
	let min_y = envelope[2] - degrees;
	let max_y = envelope[3] + degrees;

	if min_y <= -90.0 || max_y >= 90.0 || angle >= std::f64::consts::FRAC_PI_2 {
		return [-180.0, 180.0, min_y.max(-90.0), max_y.min(90.0)];
	}

	let latitude = envelope[2].abs().max(envelope[3].abs()).to_radians();
	let ratio = angle.sin() / latitude.cos();

	if ratio >= 1.0 {
		return [-180.0, 180.0, min_y, max_y];
	}

	let spread = ratio.asin().to_degrees();
	let (min_x, max_x) = (envelope[0] - spread, envelope[1] + spread);

	if min_x < -180.0 || max_x > 180.0 {
		return [-180.0, 180.0, min_y, max_y];
	}

	[min_x, max_x, min_y, max_y]
}

/// A measurement of the shape `source` reads.
pub(crate) fn write_measure(
	sql: &mut Sql,
	source: &Source,
	geography: bool,
	measured: &Measured,
) -> Result<(), DbError> {
	let function = match (sql.dialect(), geography, measured) {
		(Dialect::Postgres, _, Measured::Distance(_)) => "ST_Distance(",
		(Dialect::Postgres, _, Measured::Area) => "ST_Area(",
		(Dialect::Postgres, _, Measured::Length) => "ST_Length(",
		(Dialect::Sqlite, false, Measured::Distance(_)) => "lk_st_distance(",
		(Dialect::Sqlite, false, Measured::Area) => "lk_st_area(",
		(Dialect::Sqlite, false, Measured::Length) => "lk_st_length(",
		(Dialect::Sqlite, true, Measured::Distance(_)) => "lk_st_geodesic_distance(",
		(Dialect::Sqlite, true, Measured::Area) => "lk_st_geodesic_area(",
		(Dialect::Sqlite, true, Measured::Length) => "lk_st_geodesic_length(",
	};
	sql.push(function);
	source.write(sql)?;

	if let Measured::Distance(other) = measured {
		sql.push(", ");
		other.write(sql)?;
	}

	sql.push(")");
	Ok(())
}

/// A shape a spatial condition reads from table `T`'s rows, in the space
/// `P`, of kind `S`: a geospatial column ([`Col::term`]), or a shape
/// PostGIS computes from one with [`GeoTerm::buffer`],
/// [`GeoTerm::simplify`], [`GeoTerm::transform`], or [`GeoTerm::union`].
/// It takes the same filters and measurements as a column.
pub struct GeoTerm<T, P, S> {
	source: Source,
	types: PhantomData<Types<T, P, S>>,
}

/// The table, space, and shape a [`GeoTerm`] is typed by, without owning
/// any of them.
type Types<T, P, S> = fn() -> (T, P, S);

impl<T, P, S> Clone for GeoTerm<T, P, S> {
	fn clone(&self) -> Self {
		Self {
			source: self.source.clone(),
			types: PhantomData,
		}
	}
}

impl<T, P, S> GeoTerm<T, P, S> {
	fn derived<Q, R>(&self, wrap: impl FnOnce(Box<Source>) -> Source) -> GeoTerm<T, Q, R> {
		GeoTerm {
			source: wrap(Box::new(self.source.clone())),
			types: PhantomData,
		}
	}
}

/// A measurement of each row's shape, to compare with a number: a distance,
/// an area (square metres on the globe), or a length.
///
/// ```ignore
/// Parcel::query().filter(|p| p.boundary.area().gt(10_000.0))
/// ```
#[must_use = "a measurement does nothing until it's compared"]
pub struct Measure {
	source: Source,
	geography: bool,
	measured: Result<Measured, String>,
}

impl Measure {
	fn compare(self, comparison: Comparison, bound: impl Into<f64>) -> Expr {
		match self.measured {
			Ok(measured) => {
				Expr::Spatial(Box::new(SpatialExpr {
					source: self.source,
					geography: self.geography,
					kind: Kind::Measure(measured, comparison, bound.into()),
				}))
			}
			Err(problem) => Expr::Invalid(problem),
		}
	}

	/// Less than `bound`.
	pub fn lt(self, bound: impl Into<f64>) -> Expr {
		self.compare(Comparison::Less, bound)
	}

	/// At most `bound`.
	pub fn lte(self, bound: impl Into<f64>) -> Expr {
		self.compare(Comparison::LessOrEqual, bound)
	}

	/// More than `bound`.
	pub fn gt(self, bound: impl Into<f64>) -> Expr {
		self.compare(Comparison::Greater, bound)
	}

	/// At least `bound`.
	pub fn gte(self, bound: impl Into<f64>) -> Expr {
		self.compare(Comparison::GreaterOrEqual, bound)
	}
}

fn predicate<G: GeoValue, P: Space>(source: Source, predicate: Predicate, other: &G) -> Expr {
	match Argument::of(other) {
		Ok(other) => {
			Expr::Spatial(Box::new(SpatialExpr {
				source,
				geography: P::GEOGRAPHY,
				kind: Kind::Predicate(predicate, other),
			}))
		}
		Err(problem) => Expr::Invalid(problem),
	}
}

fn within_distance<G: GeoValue, P: Space>(source: Source, other: &G, distance: f64) -> Expr {
	match Argument::of(other) {
		Ok(other) => {
			Expr::Spatial(Box::new(SpatialExpr {
				source,
				geography: P::GEOGRAPHY,
				kind: Kind::WithinDistance(other, distance),
			}))
		}
		Err(problem) => Expr::Invalid(problem),
	}
}

fn measure<P: Space>(source: Source, measured: Result<Measured, String>) -> Measure {
	Measure {
		source,
		geography: P::GEOGRAPHY,
		measured,
	}
}

/// Spatial filters and measurements, written once for columns and computed
/// shapes. The other shape must be in the same coordinate system: a
/// `Geography` column takes `Geography` shapes, and a `Geo<_, Srid<N>>`
/// column takes shapes in `Srid<N>`, or the code doesn't compile.
macro_rules! spatial_methods {
	($space:ty, $shape:ty) => {
		/// The shapes share at least one point.
		pub fn intersects<G: GeoValue<Space = $space>>(&self, other: &G) -> Expr {
			predicate::<G, $space>(self.source(), Predicate::Intersects, other)
		}

		/// Every point of the shape is in `other`: `contains` the other way
		/// round.
		pub fn within<G: GeoValue<Space = $space>>(&self, other: &G) -> Expr {
			predicate::<G, $space>(self.source(), Predicate::Within, other)
		}

		/// The shapes share no point.
		pub fn disjoint<G: GeoValue<Space = $space>>(&self, other: &G) -> Expr {
			predicate::<G, $space>(self.source(), Predicate::Disjoint, other)
		}

		/// The shapes meet only at their boundaries.
		pub fn touches<G: GeoValue<Space = $space>>(&self, other: &G) -> Expr {
			predicate::<G, $space>(self.source(), Predicate::Touches, other)
		}

		/// The shapes are at most `distance` apart: metres on the globe, the
		/// coordinate system's units otherwise. On the globe both shapes are
		/// points.
		pub fn within_distance<G>(&self, other: &G, distance: <$space as Space>::Distance) -> Expr
		where
			G: GeoValue<Space = $space>,
			$space: MeasuresDistance<$shape, G::Shape>,
		{
			within_distance::<G, $space>(self.source(), other, distance.into())
		}

		/// The distance from each row's shape to `other`, to compare.
		pub fn distance_to<G>(&self, other: &G) -> Measure
		where
			G: GeoValue<Space = $space>,
			$space: MeasuresDistance<$shape, G::Shape>,
		{
			measure::<$space>(self.source(), Argument::of(other).map(Measured::Distance))
		}

		/// Each row's area, to compare: square metres on the globe.
		pub fn area(&self) -> Measure
		where
			$shape: Areal,
		{
			measure::<$space>(self.source(), Ok(Measured::Area))
		}

		/// Each row's length, to compare: metres on the globe.
		pub fn length(&self) -> Measure
		where
			$shape: Linear,
		{
			measure::<$space>(self.source(), Ok(Measured::Length))
		}
	};
}

/// A shape [`Col::contains`] tests a geospatial column against: the
/// column's shapes hold every point of it. On the globe the boundary counts
/// as inside (PostGIS's `ST_Covers`); planar shapes follow `ST_Contains`,
/// where a point on the boundary isn't contained.
impl<T: Table, V: GeoColumn, G> crate::expr::Contained<T, V> for &G
where
	G: GeoValue<Space = <V::Value as GeoValue>::Space>,
{
	fn condition(self, column: &Col<T, V>) -> Expr {
		predicate::<G, <V::Value as GeoValue>::Space>(column.source(), Predicate::Contains, self)
	}
}

/// Spatial filters and measurements on a geospatial column, such as
/// `p.location.within_distance(&here, Meters(500.0))`, and the PostGIS-only
/// operations that compute a shape from it. Those exist on SQLite builds
/// too, when both drivers are compiled in, but a query using one fails
/// there.
impl<T: Table, V: GeoColumn> Col<T, V> {
	spatial_methods!(<V::Value as GeoValue>::Space, <V::Value as GeoValue>::Shape);

	pub(crate) fn source(&self) -> Source {
		let name = self.name();
		let indexed = T::GEO_INDEXED.contains(&name).then(|| {
			Indexed {
				table: T::table_name(),
				key: T::KEY,
				index: GeoIndex::new(name),
			}
		});
		Source::Column { name, indexed }
	}

	/// The column as a term, for the operations [`GeoTerm`] has.
	pub fn term(&self) -> GeoTerm<T, <V::Value as GeoValue>::Space, <V::Value as GeoValue>::Shape> {
		GeoTerm {
			source: self.source(),
			types: PhantomData,
		}
	}

	/// The area within `distance` of the shape: PostGIS's `ST_Buffer`.
	pub fn buffer(
		&self,
		distance: <<V::Value as GeoValue>::Space as Space>::Distance,
	) -> GeoTerm<T, <V::Value as GeoValue>::Space, Geometry> {
		self.term().buffer(distance)
	}

	/// The shape in coordinate system `C`, through PostGIS's PROJ:
	/// `ST_Transform`.
	pub fn transform<C: CoordinateSystem>(
		&self,
	) -> GeoTerm<T, Planar<C>, <V::Value as GeoValue>::Shape> {
		self.term().transform::<C>()
	}
}

impl<T, P: Space, S: Shape> GeoTerm<T, P, S> {
	spatial_methods!(P, S);

	/// The shape holds every point of `other`. On the globe the boundary
	/// counts as inside (PostGIS's `ST_Covers`); planar shapes follow
	/// `ST_Contains`, where a point on the boundary isn't contained.
	pub fn contains<G: GeoValue<Space = P>>(&self, other: &G) -> Expr {
		predicate::<G, P>(self.source(), Predicate::Contains, other)
	}

	fn source(&self) -> Source {
		self.source.clone()
	}

	/// The area within `distance` of the shape: PostGIS's `ST_Buffer`.
	pub fn buffer(&self, distance: P::Distance) -> GeoTerm<T, P, Geometry> {
		let distance = distance.into();
		self.derived(|source| Source::Buffer(source, distance))
	}

	/// The shape in coordinate system `C`, through PostGIS's PROJ:
	/// `ST_Transform`.
	pub fn transform<C: CoordinateSystem>(&self) -> GeoTerm<T, Planar<C>, S> {
		self.derived(|source| Source::Transform(source, C::SRID))
	}
}

impl<T, C: CoordinateSystem, S: Shape> GeoTerm<T, Planar<C>, S> {
	/// The shape with fewer points, within `tolerance`, keeping its
	/// topology: PostGIS's `ST_SimplifyPreserveTopology`.
	#[must_use]
	pub fn simplify(&self, tolerance: f64) -> Self {
		self.derived(|source| Source::Simplify(source, tolerance))
	}

	/// The shape merged with `other`: PostGIS's `ST_Union`.
	pub fn union<G: GeoValue<Space = Planar<C>>>(
		&self,
		other: &G,
	) -> GeoTerm<T, Planar<C>, Geometry> {
		match Argument::of(other) {
			Ok(other) => self.derived(|source| Source::Union(source, other)),
			Err(problem) => self.derived(|_| Source::Invalid(problem)),
		}
	}
}
