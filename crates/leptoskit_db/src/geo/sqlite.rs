//! The SQL functions SQLite's geospatial layer runs on, registered on every
//! connection through `rusqlite`'s safe API.
//!
//! GeoPackage's names (`ST_MinX`, `ST_MaxX`, `ST_MinY`, `ST_MaxY`,
//! `ST_IsEmpty`) feed the R*Tree triggers, as GeoPackage's own spatial
//! index extension expects. The `lk_` functions test and measure shapes with
//! the `geo` crate: predicates through its DE-9IM `Relate`, planar
//! measurements in the coordinate system's units, and measurements on the
//! spheroid with Karney's geodesic algorithms, as PostGIS's geography does.

use geo::Area;
use geo::Distance;
use geo::Euclidean;
use geo::Geodesic;
use geo::GeodesicArea;
use geo::Length;
use geo::Relate;
use rusqlite::Connection;
use rusqlite::functions::Context;
use rusqlite::functions::FunctionFlags;
use rusqlite::types::ValueRef;

use super::Geometry;
use super::check_coordinates;
use super::codec;
use super::codec::Gpkg;

/// A DE-9IM test of how two shapes relate.
type Test = fn(&geo::relate::IntersectionMatrix) -> bool;

/// Registers every function on `connection`.
pub(crate) fn register(connection: &Connection) -> rusqlite::Result<()> {
	let flags = FunctionFlags::SQLITE_UTF8
		| FunctionFlags::SQLITE_DETERMINISTIC
		| FunctionFlags::SQLITE_INNOCUOUS;

	for (name, index) in [
		("ST_MinX", 0),
		("ST_MaxX", 1),
		("ST_MinY", 2),
		("ST_MaxY", 3),
	] {
		connection.create_scalar_function(name, 1, flags, move |context| {
			Ok(bounds(&context.get_raw(0))?.map(|bounds| bounds[index]))
		})?;
	}

	connection.create_scalar_function("ST_IsEmpty", 1, flags, |context| {
		blob(&context.get_raw(0))?
			.map(|bytes| bounds_of(bytes).map(|bounds| i64::from(bounds.is_none())))
			.transpose()
	})?;

	connection.create_scalar_function("lk_geo_check", 4, flags, |context| {
		let bytes = match context.get_raw(0) {
			// `NULL` passes, as every check does; `NOT NULL` is the column's.
			ValueRef::Null => return Ok(None),
			ValueRef::Blob(bytes) => bytes,
			_ => return Ok(Some(0)),
		};

		let shape: String = context.get(1)?;
		let srid: i64 = context.get(2)?;
		let geography: bool = context.get(3)?;
		Ok(Some(i64::from(conforms(bytes, &shape, srid, geography))))
	})?;

	connection.create_scalar_function("lk_st_isvalid", 1, flags, |context| {
		one(context, |geometry| {
			Ok(i64::from(super::is_valid(&geometry)))
		})
	})?;

	let predicates: [(&str, Test); 6] = [
		("lk_st_intersects", |matrix| matrix.is_intersects()),
		("lk_st_contains", |matrix| matrix.is_contains()),
		("lk_st_within", |matrix| matrix.is_within()),
		("lk_st_covers", |matrix| matrix.is_covers()),
		("lk_st_coveredby", |matrix| matrix.is_coveredby()),
		("lk_st_touches", |matrix| matrix.is_touches()),
	];

	for (name, test) in predicates {
		connection.create_scalar_function(name, 2, flags, move |context| {
			two(context, |left, right| {
				Ok(i64::from(test(&left.relate(&right))))
			})
		})?;
	}

	connection.create_scalar_function("lk_st_distance", 2, flags, |context| {
		two(context, |left, right| Ok(Euclidean.distance(&left, &right)))
	})?;
	connection.create_scalar_function("lk_st_geodesic_distance", 2, flags, |context| {
		two(context, geodesic_distance)
	})?;
	connection.create_scalar_function("lk_st_dwithin", 3, flags, |context| {
		let distance: Option<f64> = context.get(2)?;
		two(context, |left, right| {
			Ok(distance.map(|distance| i64::from(Euclidean.distance(&left, &right) <= distance)))
		})
		.map(Option::flatten)
	})?;
	connection.create_scalar_function("lk_st_geodesic_dwithin", 3, flags, |context| {
		let distance: Option<f64> = context.get(2)?;
		two(context, |left, right| {
			let between = geodesic_distance(left, right)?;
			Ok(distance.map(|distance| i64::from(between <= distance)))
		})
		.map(Option::flatten)
	})?;
	connection.create_scalar_function("lk_st_area", 1, flags, |context| {
		one(context, |geometry| Ok(geometry.unsigned_area()))
	})?;
	connection.create_scalar_function("lk_st_geodesic_area", 1, flags, |context| {
		one(context, |geometry| Ok(geometry.geodesic_area_unsigned()))
	})?;
	connection.create_scalar_function("lk_st_length", 1, flags, |context| {
		one(context, |geometry| Ok(length(&geometry, &Euclidean)))
	})?;
	connection.create_scalar_function("lk_st_geodesic_length", 1, flags, |context| {
		one(context, |geometry| Ok(length(&geometry, &Geodesic)))
	})?;

	Ok(())
}

fn failure(message: impl Into<String>) -> rusqlite::Error {
	rusqlite::Error::UserFunctionError(message.into().into())
}

/// The bytes of a blob argument, `None` for `NULL`, and an error otherwise.
fn blob<'a>(value: &ValueRef<'a>) -> rusqlite::Result<Option<&'a [u8]>> {
	match value {
		ValueRef::Null => Ok(None),
		ValueRef::Blob(bytes) => Ok(Some(bytes)),
		_ => Err(failure("a shape is a GeoPackage geometry blob")),
	}
}

/// The bounding box of a shape argument, from its header when it has one.
fn bounds(value: &ValueRef<'_>) -> rusqlite::Result<Option<[f64; 4]>> {
	blob(value)?.map_or(Ok(None), bounds_of)
}

fn bounds_of(bytes: &[u8]) -> rusqlite::Result<Option<[f64; 4]>> {
	let header = Gpkg::header(bytes).map_err(failure)?;

	if header.empty {
		return Ok(None);
	}

	if let Some(envelope) = header.envelope {
		return Ok(Some(envelope));
	}

	let geometry = codec::decode_wkb(&bytes[header.wkb..]).map_err(failure)?;
	Ok(super::envelope(&geometry))
}

/// A shape argument and its SRID.
fn shape(value: &ValueRef<'_>) -> rusqlite::Result<Option<(i64, Geometry)>> {
	blob(value)?
		.map(|bytes| codec::decode_any(bytes).map_err(failure))
		.transpose()
}

/// Calls `measure` with one shape argument, or returns `NULL` for `NULL`.
fn one<R>(
	context: &Context<'_>,
	measure: impl FnOnce(Geometry) -> rusqlite::Result<R>,
) -> rusqlite::Result<Option<R>> {
	shape(&context.get_raw(0))?
		.map(|(_, geometry)| measure(geometry))
		.transpose()
}

/// Calls `compare` with two shape arguments in the same coordinate system,
/// or returns `NULL` when either is `NULL`.
fn two<R>(
	context: &Context<'_>,
	compare: impl FnOnce(Geometry, Geometry) -> rusqlite::Result<R>,
) -> rusqlite::Result<Option<R>> {
	let (Some((left_srid, left)), Some((right_srid, right))) =
		(shape(&context.get_raw(0))?, shape(&context.get_raw(1))?)
	else {
		return Ok(None);
	};

	if left_srid != right_srid {
		return Err(failure(format!(
			"the shapes are in different coordinate systems, SRID {left_srid} and {right_srid}"
		)));
	}

	compare(left, right).map(Some)
}

/// The geodesic distance between two points, in metres: Karney's
/// algorithm, as PostGIS measures geography.
fn geodesic_distance(left: Geometry, right: Geometry) -> rusqlite::Result<f64> {
	match (left, right) {
		(Geometry::Point(left), Geometry::Point(right)) => Ok(Geodesic.distance(left, right)),
		_ => {
			Err(failure(
				"distances on the globe are measured between points",
			))
		}
	}
}

/// The total length of a shape's lines, as PostGIS's `ST_Length` adds them
/// up: polygons and points have none.
fn length(geometry: &Geometry, metric: &impl Length<f64>) -> f64 {
	match geometry {
		Geometry::Line(line) => metric.length(line),
		Geometry::LineString(line) => metric.length(line),
		Geometry::MultiLineString(lines) => metric.length(lines),
		Geometry::GeometryCollection(collection) => {
			collection
				.iter()
				.map(|geometry| length(geometry, metric))
				.sum()
		}
		_ => 0.0,
	}
}

/// Whether `bytes` is a GeoPackage geometry a column of `shape` in `srid`
/// may hold: well formed, of that shape, in range for geography, with a
/// bounding box that matches its coordinates (the spatial index reads it).
fn conforms(bytes: &[u8], shape: &str, srid: i64, geography: bool) -> bool {
	let Ok(header) = Gpkg::header(bytes) else {
		return false;
	};
	let Ok(geometry) = codec::decode_wkb(&bytes[header.wkb..]) else {
		return false;
	};

	let kind = match &geometry {
		Geometry::Point(_) => "Point",
		Geometry::LineString(_) => "LineString",
		Geometry::Polygon(_) => "Polygon",
		Geometry::MultiPoint(_) => "MultiPoint",
		Geometry::MultiLineString(_) => "MultiLineString",
		Geometry::MultiPolygon(_) => "MultiPolygon",
		Geometry::GeometryCollection(_) => "GeometryCollection",
		_ => return false,
	};

	let envelope = super::envelope(&geometry);

	let header_fits = match header.envelope {
		Some(found) => envelope == Some(found),
		None => true,
	};

	i64::from(header.srid) == srid
		&& (shape == "Geometry" || shape == kind)
		&& header.empty == envelope.is_none()
		&& header_fits
		&& check_coordinates(&geometry, geography).is_ok()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::geo::line_string;
	use crate::geo::point;
	use crate::geo::polygon;

	fn connection() -> Connection {
		let connection = Connection::open_in_memory().unwrap();
		register(&connection).unwrap();
		connection
	}

	fn gpkg(geometry: &Geometry, srid: u32) -> Vec<u8> {
		codec::encode_gpkg(geometry, srid).unwrap()
	}

	#[test]
	fn null_shapes_give_null() {
		let connection = connection();
		let (box_, empty, distance, valid): (Option<f64>, Option<i64>, Option<f64>, Option<i64>) =
			connection
				.query_row(
					"SELECT ST_MinX(NULL), ST_IsEmpty(NULL), lk_st_distance(NULL, NULL), \
					 lk_st_isvalid(NULL)",
					[],
					|row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
				)
				.unwrap();
		assert_eq!((box_, empty, distance, valid), (None, None, None, None));
	}

	#[test]
	fn boxes_come_from_the_header() {
		let connection = connection();
		let square = gpkg(
			&polygon![(x: 1.0, y: 2.0), (x: 4.0, y: 2.0), (x: 4.0, y: 6.0), (x: 1.0, y: 2.0)]
				.into(),
			27700,
		);
		let bounds: (f64, f64, f64, f64, i64) = connection
			.query_row(
				"SELECT ST_MinX(?1), ST_MaxX(?1), ST_MinY(?1), ST_MaxY(?1), ST_IsEmpty(?1)",
				[&square],
				|row| {
					Ok((
						row.get(0)?,
						row.get(1)?,
						row.get(2)?,
						row.get(3)?,
						row.get(4)?,
					))
				},
			)
			.unwrap();
		assert_eq!(bounds, (1.0, 4.0, 2.0, 6.0, 0));

		let empty = gpkg(
			&Geometry::MultiPolygon(geo::MultiPolygon(Vec::new())),
			27700,
		);
		let empty: (Option<f64>, i64) = connection
			.query_row("SELECT ST_MinX(?1), ST_IsEmpty(?1)", [&empty], |row| {
				Ok((row.get(0)?, row.get(1)?))
			})
			.unwrap();
		assert_eq!(empty, (None, 1));
	}

	#[test]
	fn mismatched_and_malformed_shapes_fail() {
		let connection = connection();
		let here = gpkg(&point!(x: 0.0, y: 0.0).into(), 4326);
		let there = gpkg(&point!(x: 0.0, y: 0.0).into(), 3857);
		let mixed =
			connection.query_row("SELECT lk_st_intersects(?1, ?2)", [&here, &there], |row| {
				row.get::<_, i64>(0)
			});
		assert!(
			mixed
				.unwrap_err()
				.to_string()
				.contains("different coordinate systems")
		);

		let text = connection.query_row("SELECT ST_MinX('nope')", [], |row| {
			row.get::<_, Option<f64>>(0)
		});
		assert!(text.is_err());

		let garbage = connection.query_row("SELECT ST_MinX(x'00')", [], |row| {
			row.get::<_, Option<f64>>(0)
		});
		assert!(garbage.is_err());

		let line = gpkg(
			&line_string![(x: 0.0, y: 0.0), (x: 1.0, y: 1.0)].into(),
			4326,
		);
		let geodesic = connection.query_row(
			"SELECT lk_st_geodesic_distance(?1, ?2)",
			[&line, &here],
			|row| row.get::<_, f64>(0),
		);
		assert!(geodesic.unwrap_err().to_string().contains("between points"));
	}

	#[test]
	fn checks_refuse_what_the_column_doesnt_hold() {
		let connection = connection();
		let check = |bytes: &[u8], shape: &str, srid: i64, geography: bool| -> i64 {
			connection
				.query_row(
					"SELECT lk_geo_check(?1, ?2, ?3, ?4)",
					rusqlite::params![bytes, shape, srid, geography],
					|row| row.get(0),
				)
				.unwrap()
		};
		let here = gpkg(&point!(x: 10.0, y: 20.0).into(), 4326);
		assert_eq!(check(&here, "Point", 4326, true), 1);
		assert_eq!(check(&here, "Geometry", 4326, true), 1);
		assert_eq!(check(&here, "Polygon", 4326, true), 0);
		assert_eq!(check(&here, "Point", 27700, false), 0);
		assert_eq!(check(b"GP", "Point", 4326, true), 0);

		let far = gpkg(&point!(x: 200.0, y: 20.0).into(), 4326);
		assert_eq!(check(&far, "Point", 4326, false), 1);
		assert_eq!(check(&far, "Point", 4326, true), 0);

		// A header whose box lies about the shape would mislead the index.
		let mut lying = here.clone();
		lying[8..16].copy_from_slice(&(-5.0_f64).to_le_bytes());
		assert_eq!(check(&lying, "Point", 4326, true), 0);

		let null: Option<i64> = connection
			.query_row("SELECT lk_geo_check(NULL, 'Point', 4326, 1)", [], |row| {
				row.get(0)
			})
			.unwrap();
		assert_eq!(null, None);
		assert_eq!(
			connection
				.query_row("SELECT lk_geo_check('text', 'Point', 4326, 1)", [], |row| {
					row.get::<_, i64>(0)
				})
				.unwrap(),
			0
		);
	}
}
