//! Geospatial columns: places, routes, and areas, stored by PostGIS on
//! Postgres and by a built-in layer of the same operations on SQLite.
//!
//! ```ignore
//! use leptoskit::db::geo::{Geography, Point, point};
//!
//! #[db::table(name = "places", public)]
//! pub struct Place {
//! 	pub id: Id<Place>,
//! 	#[db(geo(index))]
//! 	pub location: Geography<Point>,
//! }
//!
//! let here = Geography(point!(x: -0.1276, y: 51.5072));
//! let nearby = Place::query()
//! 	.filter(|p| p.location.within_distance(&here, Meters(500.0)))
//! 	.all(&db)
//! 	.await?;
//! ```
//!
//! Two wrappers say what the coordinates mean:
//!
//! - [`Geography`] holds longitude and latitude on the WGS 84 spheroid, and
//!   measures in metres. It's the right choice for places on the real globe.
//! - [`Geo`] holds planar coordinates in a coordinate system named by its EPSG
//!   code, [`Srid`], and measures in that system's units: national grids, CAD
//!   drawings, game maps.
//!
//! The shape inside is any [`geo-types`](geo_types) shape: [`Point`],
//! [`LineString`], [`Polygon`], their `Multi` forms, a
//! [`GeometryCollection`], a [`Rect`] (stored as a polygon), or [`Geometry`]
//! for any shape. Both wrappers travel as GeoJSON (RFC 7946); [`Wkb`] sends
//! one as base64 WKB instead.

pub(crate) mod codec;
#[cfg(feature = "server")]
mod column;
#[cfg(feature = "server")]
mod nearest;
#[cfg(feature = "server")]
pub(crate) mod query;
#[cfg(feature = "sqlite")]
pub(crate) mod sqlite;

use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;

#[cfg(feature = "server")]
pub use column::GeoColumn;
/// The `geo` crate's algorithms, for measuring and testing shapes in Rust,
/// on the server or the client, without a round trip.
pub use geo as algorithms;
pub use geo_types::Coord;
pub use geo_types::Geometry;
pub use geo_types::GeometryCollection;
pub use geo_types::LineString;
pub use geo_types::MultiLineString;
pub use geo_types::MultiPoint;
pub use geo_types::MultiPolygon;
pub use geo_types::Point;
pub use geo_types::Polygon;
pub use geo_types::Rect;
pub use geo_types::coord;
pub use geo_types::line_string;
pub use geo_types::point;
pub use geo_types::polygon;
#[cfg(feature = "server")]
pub use nearest::Nearest;
#[cfg(feature = "server")]
pub use nearest::NearestWithDistance;
#[cfg(feature = "server")]
pub use query::GeoTerm;
#[cfg(feature = "server")]
pub use query::Measure;
#[cfg(feature = "server")]
pub use query::MeasuresDistance;
use schemars::JsonSchema;
use schemars::SchemaGenerator;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

#[doc(hidden)]
pub use self::codec::decode_any;
#[doc(hidden)]
pub use self::codec::encode_ewkb;
#[doc(hidden)]
pub use self::codec::encode_gpkg;
pub(crate) use self::codec::envelope;

/// A coordinate system, named by its EPSG code: [`Srid`].
pub trait CoordinateSystem: Send + Sync + 'static {
	/// The EPSG code, such as 4326 for WGS 84 longitude and latitude.
	const SRID: u32;
}

/// The coordinate system with EPSG code `CODE`, such as `Srid<27700>` for
/// the British National Grid. Mixing values of different systems fails to
/// compile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Srid<const CODE: u32>;

impl<const CODE: u32> CoordinateSystem for Srid<CODE> {
	const SRID: u32 = CODE;
}

/// WGS 84 longitude and latitude, in degrees: EPSG 4326.
pub type Wgs84 = Srid<4326>;

/// Web Mercator, in metres, as web map tiles use: EPSG 3857.
pub type WebMercator = Srid<3857>;

/// A shape in longitude and latitude on the WGS 84 spheroid. Distances,
/// areas, and lengths are in metres and follow the curve of the earth.
///
/// Longitudes run from −180 to 180 and latitudes from −90 to 90: values
/// outside that range fail to deserialize, fail to be written, and fail the
/// column's database check on SQLite (PostGIS coerces them into range).
#[derive(Clone, Debug, PartialEq)]
pub struct Geography<T>(pub T);

/// A planar shape in the coordinate system `S`, measured in that system's
/// units. `S` defaults to WGS 84, whose units are degrees; prefer
/// [`Geography`] for distances on the globe.
pub struct Geo<T, S: CoordinateSystem = Wgs84>(pub T, pub PhantomData<S>);

impl<T, S: CoordinateSystem> Geo<T, S> {
	/// The shape, in `S`.
	pub const fn new(shape: T) -> Self {
		Self(shape, PhantomData)
	}

	/// The shape.
	pub fn into_inner(self) -> T {
		self.0
	}
}

impl<T: Clone, S: CoordinateSystem> Clone for Geo<T, S> {
	fn clone(&self) -> Self {
		Self::new(self.0.clone())
	}
}

impl<T: fmt::Debug, S: CoordinateSystem> fmt::Debug for Geo<T, S> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_tuple("Geo").field(&self.0).field(&S::SRID).finish()
	}
}

impl<T: PartialEq, S: CoordinateSystem> PartialEq for Geo<T, S> {
	fn eq(&self, other: &Self) -> bool {
		self.0 == other.0
	}
}

impl<T> Geography<T> {
	/// The shape.
	pub fn into_inner(self) -> T {
		self.0
	}
}

/// A distance, area, or length on the globe, in metres (or square metres).
#[derive(
	Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Meters(pub f64);

impl From<Meters> for f64 {
	fn from(meters: Meters) -> Self {
		meters.0
	}
}

impl From<f64> for Meters {
	fn from(value: f64) -> Self {
		Self(value)
	}
}

mod sealed {
	pub trait Sealed {}
}

/// A `geo-types` shape a geospatial column can hold. Implemented for
/// [`Point`], [`LineString`], [`Polygon`], [`MultiPoint`],
/// [`MultiLineString`], [`MultiPolygon`], [`GeometryCollection`], [`Rect`],
/// and [`Geometry`].
pub trait Shape: sealed::Sealed + Clone + Send + Sync + Sized + 'static {
	/// How PostGIS and GeoJSON name the kind of shape a column of it holds:
	/// `Point`, `Polygon`, or `Geometry` for any.
	const NAME: &'static str;

	/// The shape as the `geo-types` enum.
	fn to_geometry(&self) -> Geometry;

	/// The shape, when `geometry` is one.
	fn from_geometry(geometry: Geometry) -> Result<Self, String>;
}

/// A shape with an area: polygons, and shapes that may hold them.
pub trait Areal: Shape {}

/// A shape with a length: lines, and shapes that may hold them.
pub trait Linear: Shape {}

macro_rules! shapes {
	($($ty:ident),* $(,)?) => {
		$(
			impl sealed::Sealed for $ty {}

			impl Shape for $ty {
				const NAME: &'static str = stringify!($ty);

				fn to_geometry(&self) -> Geometry {
					Geometry::$ty(self.clone())
				}

				fn from_geometry(geometry: Geometry) -> Result<Self, String> {
					match geometry {
						Geometry::$ty(shape) => Ok(shape),
						other => Err(format!(
							"expected a {}, found a {}",
							stringify!($ty),
							kind_name(&other)
						)),
					}
				}
			}
		)*
	};
}

shapes!(
	Point,
	LineString,
	Polygon,
	MultiPoint,
	MultiLineString,
	MultiPolygon,
	GeometryCollection,
);

impl sealed::Sealed for Geometry {}

impl Shape for Geometry {
	const NAME: &'static str = "Geometry";

	fn to_geometry(&self) -> Geometry {
		self.clone()
	}

	fn from_geometry(geometry: Geometry) -> Result<Self, String> {
		match geometry {
			Geometry::Line(line) => Ok(Geometry::LineString(line.into())),
			Geometry::Rect(rect) => Ok(Geometry::Polygon(rect.to_polygon())),
			Geometry::Triangle(triangle) => Ok(Geometry::Polygon(triangle.to_polygon())),
			geometry => Ok(geometry),
		}
	}
}

impl sealed::Sealed for Rect {}

/// A rectangle with sides along the axes, such as a map's viewport. It's
/// stored and sent as a polygon, and only an axis-aligned polygon of four
/// corners reads back as one.
impl Shape for Rect {
	const NAME: &'static str = "Polygon";

	fn to_geometry(&self) -> Geometry {
		Geometry::Polygon(self.to_polygon())
	}

	fn from_geometry(geometry: Geometry) -> Result<Self, String> {
		let polygon = Polygon::from_geometry(geometry)?;
		rect_of(&polygon).ok_or_else(|| "the polygon isn't an axis-aligned rectangle".to_string())
	}
}

impl Areal for Polygon {}
impl Areal for MultiPolygon {}
impl Areal for Rect {}
impl Areal for GeometryCollection {}
impl Areal for Geometry {}
impl Linear for LineString {}
impl Linear for MultiLineString {}
impl Linear for GeometryCollection {}
impl Linear for Geometry {}

/// The rectangle a polygon is, when it's one: no holes, and its ring the
/// four corners of its bounding box, closed.
fn rect_of(polygon: &Polygon) -> Option<Rect> {
	if !polygon.interiors().is_empty() {
		return None;
	}

	let ring = &polygon.exterior().0;
	let [first, .., last] = ring.as_slice() else {
		return None;
	};

	if ring.len() != 5 || first != last {
		return None;
	}

	let [min_x, max_x, min_y, max_y] = envelope(&Geometry::Polygon(polygon.clone()))?;
	let corners = [
		Coord { x: min_x, y: min_y },
		Coord { x: min_x, y: max_y },
		Coord { x: max_x, y: max_y },
		Coord { x: max_x, y: min_y },
	];
	let distinct = ring[..4].iter().all(|coord| corners.contains(coord))
		&& corners.iter().all(|corner| ring[..4].contains(corner));

	distinct.then(|| Rect::new(corners[0], corners[2]))
}

fn kind_name(geometry: &Geometry) -> &'static str {
	match geometry {
		Geometry::Point(_) => "Point",
		Geometry::Line(_) => "Line",
		Geometry::LineString(_) => "LineString",
		Geometry::Polygon(_) => "Polygon",
		Geometry::MultiPoint(_) => "MultiPoint",
		Geometry::MultiLineString(_) => "MultiLineString",
		Geometry::MultiPolygon(_) => "MultiPolygon",
		Geometry::GeometryCollection(_) => "GeometryCollection",
		Geometry::Rect(_) => "Rect",
		Geometry::Triangle(_) => "Triangle",
	}
}

/// What a geospatial value's coordinates mean: [`Spheroid`] for
/// [`Geography`], [`Planar`] for [`Geo`].
pub trait Space: Send + Sync + 'static {
	/// The coordinate system's EPSG code.
	const SRID: u32;
	/// Whether coordinates are longitude and latitude on the spheroid,
	/// measured in metres.
	const GEOGRAPHY: bool;
	/// What distances are measured in: [`Meters`], or `f64` in the system's
	/// units.
	type Distance: From<f64> + Into<f64> + Copy + fmt::Debug + PartialEq + Send + Sync;
}

/// Longitude and latitude on the WGS 84 spheroid.
#[derive(Clone, Copy, Debug)]
pub struct Spheroid;

impl Space for Spheroid {
	type Distance = Meters;

	const GEOGRAPHY: bool = true;
	const SRID: u32 = 4326;
}

/// Planar coordinates in the system `S`.
pub struct Planar<S>(PhantomData<S>);

impl<S: CoordinateSystem> Space for Planar<S> {
	type Distance = f64;

	const GEOGRAPHY: bool = false;
	const SRID: u32 = S::SRID;
}

/// A geospatial value: [`Geography`], [`Geo`], or either in [`Wkb`].
#[diagnostic::on_unimplemented(
	message = "`{Self}` isn't a geospatial value",
	label = "expected `Geography<T>` or `Geo<T, Srid<N>>`, holding a `geo-types` shape"
)]
pub trait GeoValue: Sized {
	/// The shape inside.
	type Shape: Shape;
	/// What its coordinates mean.
	type Space: Space;

	/// The shape.
	fn shape(&self) -> &Self::Shape;

	/// The value holding `shape`.
	fn from_shape(shape: Self::Shape) -> Self;

	/// Checks the coordinates: every one finite, and, on the spheroid, in
	/// range.
	fn check(&self) -> Result<(), String> {
		check_coordinates(&self.shape().to_geometry(), Self::Space::GEOGRAPHY)
	}

	/// The value of a geometry decoded from storage or the wire, checked.
	fn from_geometry(geometry: Geometry) -> Result<Self, String> {
		let value = Self::from_shape(Self::Shape::from_geometry(geometry)?);
		value.check()?;
		Ok(value)
	}
}

impl<T: Shape> GeoValue for Geography<T> {
	type Shape = T;
	type Space = Spheroid;

	fn shape(&self) -> &T {
		&self.0
	}

	fn from_shape(shape: T) -> Self {
		Self(shape)
	}
}

impl<T: Shape, S: CoordinateSystem> GeoValue for Geo<T, S> {
	type Shape = T;
	type Space = Planar<S>;

	fn shape(&self) -> &T {
		&self.0
	}

	fn from_shape(shape: T) -> Self {
		Self::new(shape)
	}
}

/// Checks that every coordinate is finite, and, for geography, a longitude
/// from −180 to 180 and a latitude from −90 to 90.
pub(crate) fn check_coordinates(geometry: &Geometry, geography: bool) -> Result<(), String> {
	let mut problem = None;
	visit_coords(geometry, &mut |coord| {
		if problem.is_some() {
			return;
		}

		if !coord.x.is_finite() || !coord.y.is_finite() {
			problem = Some(format!(
				"({}, {}) isn't a finite position",
				coord.x, coord.y
			));
		} else if geography && !(-180.0..=180.0).contains(&coord.x) {
			problem = Some(format!("longitude {} is outside −180 to 180", coord.x));
		} else if geography && !(-90.0..=90.0).contains(&coord.y) {
			problem = Some(format!("latitude {} is outside −90 to 90", coord.y));
		}
	});
	problem.map_or(Ok(()), Err)
}

/// Calls `visit` with every coordinate of `geometry`.
pub(crate) fn visit_coords(geometry: &Geometry, visit: &mut impl FnMut(Coord)) {
	let mut ring = |line: &LineString| line.0.iter().copied().for_each(&mut *visit);

	match geometry {
		Geometry::Point(point) => ring(&LineString(vec![point.0])),
		Geometry::Line(line) => ring(&LineString(vec![line.start, line.end])),
		Geometry::LineString(line) => ring(line),
		Geometry::Polygon(polygon) => {
			ring(polygon.exterior());
			polygon.interiors().iter().for_each(ring);
		}
		Geometry::MultiPoint(points) => {
			ring(&LineString(points.iter().map(|point| point.0).collect()));
		}
		Geometry::MultiLineString(lines) => lines.iter().for_each(ring),
		Geometry::MultiPolygon(polygons) => {
			for polygon in polygons {
				ring(polygon.exterior());
				polygon.interiors().iter().for_each(&mut ring);
			}
		}
		Geometry::GeometryCollection(collection) => {
			for geometry in collection {
				visit_coords(geometry, visit);
			}
		}
		Geometry::Rect(rect) => ring(rect.to_polygon().exterior()),
		Geometry::Triangle(triangle) => ring(triangle.to_polygon().exterior()),
	}
}

impl<T: Shape> Geography<T> {
	/// The smallest rectangle around the shape, in longitude and latitude,
	/// or `None` when it's empty.
	pub fn bounding_box(&self) -> Option<Rect> {
		bounding_box(&self.0.to_geometry())
	}
}

impl<T: Shape, S: CoordinateSystem> Geo<T, S> {
	/// The smallest rectangle around the shape, or `None` when it's empty.
	pub fn bounding_box(&self) -> Option<Rect> {
		bounding_box(&self.0.to_geometry())
	}
}

fn bounding_box(geometry: &Geometry) -> Option<Rect> {
	let [min_x, max_x, min_y, max_y] = envelope(geometry)?;
	Some(Rect::new(
		Coord { x: min_x, y: min_y },
		Coord { x: max_x, y: max_y },
	))
}

fn serialize_geojson<S: Serializer>(geometry: &Geometry, serializer: S) -> Result<S::Ok, S::Error> {
	geojson::Geometry::new(geojson::GeometryValue::from(geometry)).serialize(serializer)
}

fn deserialize_geojson<'de, V: GeoValue, D: Deserializer<'de>>(
	deserializer: D,
) -> Result<V, D::Error> {
	let geojson = geojson::Geometry::deserialize(deserializer)?;
	let geometry = Geometry::try_from(&geojson.value).map_err(serde::de::Error::custom)?;
	V::from_geometry(geometry).map_err(serde::de::Error::custom)
}

impl<T: Shape> Serialize for Geography<T> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serialize_geojson(&self.0.to_geometry(), serializer)
	}
}

impl<'de, T: Shape> Deserialize<'de> for Geography<T> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		deserialize_geojson(deserializer)
	}
}

impl<T: Shape, S: CoordinateSystem> Serialize for Geo<T, S> {
	fn serialize<Z: Serializer>(&self, serializer: Z) -> Result<Z::Ok, Z::Error> {
		serialize_geojson(&self.0.to_geometry(), serializer)
	}
}

impl<'de, T: Shape, S: CoordinateSystem> Deserialize<'de> for Geo<T, S> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		deserialize_geojson(deserializer)
	}
}

/// The JSON Schema of one GeoJSON position: two numbers, in range for
/// geography.
fn position_schema(geography: bool) -> serde_json::Value {
	let number = |min: f64, max: f64| {
		if geography {
			serde_json::json!({ "type": "number", "minimum": min, "maximum": max })
		} else {
			serde_json::json!({ "type": "number" })
		}
	};

	serde_json::json!({
		"type": "array",
		"prefixItems": [number(-180.0, 180.0), number(-90.0, 90.0)],
		"minItems": 2,
		"maxItems": 2,
	})
}

/// The JSON Schema of a GeoJSON geometry named `name` (`Geometry` for any).
fn geometry_schema(name: &str, geography: bool) -> serde_json::Value {
	let nested = |depth: usize| {
		(0..depth).fold(
			position_schema(geography),
			|items, _| serde_json::json!({ "type": "array", "items": items }),
		)
	};
	let object = |kind: &str, field: &str, value: serde_json::Value| {
		serde_json::json!({
			"type": "object",
			"properties": { "type": { "const": kind }, field: value },
			"required": ["type", field],
		})
	};

	match name {
		"Point" => object(name, "coordinates", nested(0)),
		"LineString" | "MultiPoint" => object(name, "coordinates", nested(1)),
		"Polygon" | "MultiLineString" => object(name, "coordinates", nested(2)),
		"MultiPolygon" => object(name, "coordinates", nested(3)),
		"GeometryCollection" => {
			object(
				name,
				"geometries",
				serde_json::json!({ "type": "array", "items": geometry_schema("Geometry", geography) }),
			)
		}
		_ => {
			let simple: Vec<serde_json::Value> = [
				"Point",
				"LineString",
				"Polygon",
				"MultiPoint",
				"MultiLineString",
				"MultiPolygon",
			]
			.into_iter()
			.map(|kind| geometry_schema(kind, geography))
			.chain([serde_json::json!({
				"type": "object",
				"properties": {
					"type": { "const": "GeometryCollection" },
					"geometries": { "type": "array", "items": { "type": "object" } },
				},
				"required": ["type", "geometries"],
			})])
			.collect();
			serde_json::json!({ "oneOf": simple })
		}
	}
}

fn schema_of(value: serde_json::Value, title: String) -> schemars::Schema {
	let mut schema = schemars::Schema::try_from(value).expect("a GeoJSON schema is an object");
	schema.insert("title".into(), title.into());
	schema
}

impl<T: Shape> JsonSchema for Geography<T> {
	fn schema_name() -> Cow<'static, str> {
		format!("GeoJson{}Geography", T::NAME).into()
	}

	fn json_schema(_: &mut SchemaGenerator) -> schemars::Schema {
		schema_of(
			geometry_schema(T::NAME, true),
			format!("GeoJSON {} in WGS 84 longitude and latitude", T::NAME),
		)
	}
}

impl<T: Shape, S: CoordinateSystem> JsonSchema for Geo<T, S> {
	fn schema_name() -> Cow<'static, str> {
		format!("GeoJson{}Srid{}", T::NAME, S::SRID).into()
	}

	fn json_schema(_: &mut SchemaGenerator) -> schemars::Schema {
		schema_of(
			geometry_schema(T::NAME, false),
			format!("GeoJSON {} in EPSG:{}", T::NAME, S::SRID),
		)
	}
}

/// A geospatial value that travels as base64 WKB rather than GeoJSON:
/// smaller for big polygons, for clients that decode WKB themselves. It's
/// stored like the value inside.
///
/// ```ignore
/// #[db(private)]
/// pub boundary: Wkb<Geography<MultiPolygon>>,
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Wkb<G>(pub G);

impl<G: GeoValue> GeoValue for Wkb<G> {
	type Shape = G::Shape;
	type Space = G::Space;

	fn shape(&self) -> &Self::Shape {
		self.0.shape()
	}

	fn from_shape(shape: Self::Shape) -> Self {
		Self(G::from_shape(shape))
	}
}

impl<G: GeoValue> Serialize for Wkb<G> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		use base64::Engine;

		let wkb =
			codec::encode_wkb(&self.0.shape().to_geometry()).map_err(serde::ser::Error::custom)?;
		serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(wkb))
	}
}

impl<'de, G: GeoValue> Deserialize<'de> for Wkb<G> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		use base64::Engine;

		let text = Cow::<str>::deserialize(deserializer)?;
		let bytes = base64::engine::general_purpose::STANDARD
			.decode(text.as_bytes())
			.map_err(serde::de::Error::custom)?;
		let geometry = codec::decode_wkb(&bytes).map_err(serde::de::Error::custom)?;
		G::from_geometry(geometry)
			.map(Self)
			.map_err(serde::de::Error::custom)
	}
}

impl<G: GeoValue> JsonSchema for Wkb<G> {
	fn schema_name() -> Cow<'static, str> {
		"Wkb".into()
	}

	fn json_schema(_: &mut SchemaGenerator) -> schemars::Schema {
		schemars::json_schema!({
			"type": "string",
			"contentEncoding": "base64",
			"description": "A shape as well-known binary (WKB), base64-encoded",
		})
	}
}

/// The validation `#[db(geo(…))]` adds to a column's `validate()`: the
/// coordinates, and, with `valid`, the shape's validity.
#[doc(hidden)]
pub mod rules {
	use super::GeoValue;
	use super::Shape;
	use crate::ValidationErrors;
	use crate::validate::Rule;

	/// A geospatial column's value, or `None` to skip the check.
	#[diagnostic::on_unimplemented(
		message = "`{Self}` isn't a geospatial column",
		label = "`#[db(geo(…))]` needs `Geography<T>` or `Geo<T, Srid<N>>`"
	)]
	pub trait Checked {
		/// The coordinates' problem, if any.
		fn coordinates(&self) -> Option<Result<(), String>>;

		/// Whether the shape is valid, or `None` to skip the check.
		fn valid(&self) -> Option<bool>;
	}

	impl<G: GeoValue> Checked for G {
		fn coordinates(&self) -> Option<Result<(), String>> {
			Some(self.check())
		}

		fn valid(&self) -> Option<bool> {
			Some(super::is_valid(&self.shape().to_geometry()))
		}
	}

	impl<G: Checked> Checked for Option<G> {
		fn coordinates(&self) -> Option<Result<(), String>> {
			self.as_ref().and_then(Checked::coordinates)
		}

		fn valid(&self) -> Option<bool> {
			self.as_ref().and_then(Checked::valid)
		}
	}

	pub fn geo<C: Checked>(errors: &mut ValidationErrors, field: &str, value: &C, valid: bool) {
		if let Some(Err(problem)) = value.coordinates() {
			errors.push(field, Rule::Coordinates(problem));
		}

		if valid && value.valid() == Some(false) {
			errors.push(field, Rule::ValidGeometry);
		}
	}
}

/// Whether `geometry` is valid as OGC defines it.
pub(crate) fn is_valid(geometry: &Geometry) -> bool {
	use geo::Validation;

	geometry.is_valid()
}

#[cfg(test)]
mod tests;
