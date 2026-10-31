//! Geospatial values as columns.

use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::GeoShape;

use super::CoordinateSystem;
use super::Geo;
use super::GeoValue;
use super::Geography;
use super::Shape;
use super::Space;
use super::Wkb;
use super::codec;
use crate::Column;
use crate::DecodeError;
use crate::EncodeError;
use crate::Value;

/// A column holding shapes: [`Geography`], [`Geo`], [`Wkb`] of either, and
/// `Option` of any of them. Spatial filters, measurements, and
/// [`Query::nearest`](crate::Query::nearest) exist for these columns.
#[diagnostic::on_unimplemented(
	message = "`{Self}` isn't a geospatial column",
	label = "`#[db(geo(…))]` and spatial filters need `Geography<T>` or `Geo<T, Srid<N>>`"
)]
pub trait GeoColumn: Column {
	/// The value the column holds, without `Option`.
	type Value: GeoValue;
}

impl<T: Shape> GeoColumn for Geography<T> {
	type Value = Self;
}

impl<T: Shape, S: CoordinateSystem> GeoColumn for Geo<T, S> {
	type Value = Self;
}

impl<G: GeoValue> GeoColumn for Wkb<G>
where
	Self: Column,
{
	type Value = Self;
}

impl<C: GeoColumn> GeoColumn for Option<C> {
	type Value = C::Value;
}

/// The schema's name for a shape type.
pub(crate) fn shape_of<T: Shape>() -> GeoShape {
	GeoShape::from_name(T::NAME).expect("every shape is named after a GeoPackage shape")
}

/// A value as the database layer stores it: GeoPackage's encoding, which
/// the Postgres driver converts to EWKB.
pub(crate) fn encode<G: GeoValue>(value: &G) -> Result<Value, EncodeError> {
	value.check().map_err(EncodeError::new)?;
	codec::encode_gpkg(&value.shape().to_geometry(), G::Space::SRID)
		.map(Value::Bytes)
		.map_err(EncodeError::new)
}

/// A value from GeoPackage's encoding or EWKB, in the coordinate system
/// its type says.
pub(crate) fn decode<G: GeoValue>(value: Value) -> Result<G, DecodeError> {
	let Value::Bytes(bytes) = value else {
		return Err(DecodeError::unexpected("a shape", &value));
	};
	let (srid, geometry) = codec::decode_any(&bytes).map_err(DecodeError::custom)?;

	if srid != i64::from(G::Space::SRID) {
		return Err(DecodeError::custom(format!(
			"the shape is in SRID {srid}, and the column's type expects {}",
			G::Space::SRID
		)));
	}

	G::from_geometry(geometry).map_err(DecodeError::custom)
}

impl<T: Shape> Column for Geography<T> {
	fn column_type() -> ColumnType {
		ColumnType::Geography {
			shape: shape_of::<T>(),
		}
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		encode(self)
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		decode(value)
	}
}

impl<T: Shape, S: CoordinateSystem> Column for Geo<T, S> {
	fn column_type() -> ColumnType {
		ColumnType::Geometry {
			shape: shape_of::<T>(),
			srid: S::SRID,
		}
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		encode(self)
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		decode(value)
	}
}

impl<G: GeoValue + Column> Column for Wkb<G> {
	fn column_type() -> ColumnType {
		G::column_type()
	}

	fn to_value(&self) -> Result<Value, EncodeError> {
		self.0.to_value()
	}

	fn from_value(value: Value) -> Result<Self, DecodeError> {
		G::from_value(value).map(Self)
	}
}
