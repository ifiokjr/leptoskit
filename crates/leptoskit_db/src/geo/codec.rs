//! Shapes as bytes: ISO WKB for the wire, EWKB (WKB with an SRID) for
//! PostGIS, and GeoPackage's binary encoding (a header with the SRID and the
//! bounding box, then WKB) for SQLite.
//!
//! The database layer's own form of a geospatial value is the GeoPackage
//! encoding, whichever database holds it; the Postgres driver converts to
//! and from EWKB at the edge. Decoding accepts either.

use geo_traits::GeometryTrait;
use geo_traits::to_geo::ToGeoGeometry;

use super::Geometry;
use super::visit_coords;

/// The flag EWKB sets in a geometry's type when an SRID follows it.
const EWKB_SRID: u32 = 0x2000_0000;

/// GeoPackage's magic bytes, `GP`.
const GPKG_MAGIC: [u8; 2] = *b"GP";

/// The GeoPackage header's flags bit saying the geometry is empty.
const GPKG_EMPTY: u8 = 0b0001_0000;

/// The GeoPackage header's flags bit saying an extension geometry type
/// follows, which no database here writes.
const GPKG_EXTENDED: u8 = 0b0010_0000;

/// A shape's bounding box: `[min_x, max_x, min_y, max_y]`, the order
/// GeoPackage's header uses, or `None` for an empty shape.
pub(crate) fn envelope(geometry: &Geometry) -> Option<[f64; 4]> {
	let mut envelope: Option<[f64; 4]> = None;
	visit_coords(geometry, &mut |coord| {
		let bounds = envelope.get_or_insert([coord.x, coord.x, coord.y, coord.y]);
		bounds[0] = bounds[0].min(coord.x);
		bounds[1] = bounds[1].max(coord.x);
		bounds[2] = bounds[2].min(coord.y);
		bounds[3] = bounds[3].max(coord.y);
	});
	envelope
}

/// `geometry` as ISO WKB, little-endian, with two dimensions.
pub(crate) fn encode_wkb(geometry: &Geometry) -> Result<Vec<u8>, String> {
	let mut bytes = Vec::new();
	wkb::writer::write_geometry(
		&mut bytes,
		geometry,
		&wkb::writer::WriteOptions {
			endianness: wkb::Endianness::LittleEndian,
		},
	)
	.map_err(|error| error.to_string())?;
	Ok(bytes)
}

/// The shape ISO WKB or EWKB holds, in two dimensions. An empty point,
/// which `geo-types` can't hold, and shapes with Z or M values are refused.
pub(crate) fn decode_wkb(bytes: &[u8]) -> Result<Geometry, String> {
	let wkb = wkb::reader::read_wkb(bytes).map_err(|error| format!("invalid WKB: {error}"))?;

	if wkb.dim() != geo_traits::Dimensions::Xy {
		return Err("only two-dimensional shapes are supported, without Z or M values".into());
	}

	wkb.try_to_geometry()
		.ok_or_else(|| "an empty point can't be read; store no shape (NULL) instead".to_string())
}

/// `geometry` as EWKB with `srid`, which PostGIS reads and writes.
#[doc(hidden)]
pub fn encode_ewkb(geometry: &Geometry, srid: u32) -> Result<Vec<u8>, String> {
	let mut bytes = encode_wkb(geometry)?;
	let kind = u32::from_le_bytes(bytes[1..5].try_into().expect("WKB has a type")) | EWKB_SRID;
	bytes[1..5].copy_from_slice(&kind.to_le_bytes());
	bytes.splice(5..5, srid.to_le_bytes());
	Ok(bytes)
}

/// The SRID EWKB carries, if any.
fn ewkb_srid(bytes: &[u8]) -> Result<Option<u32>, String> {
	let too_short = || "invalid EWKB: too short".to_string();
	let little = match bytes.first() {
		Some(1) => true,
		Some(0) => false,
		_ => return Err("invalid WKB: the first byte isn't a byte order".into()),
	};
	let word = |range: std::ops::Range<usize>| -> Result<u32, String> {
		let word: [u8; 4] = bytes
			.get(range)
			.and_then(|slice| slice.try_into().ok())
			.ok_or_else(too_short)?;
		Ok(if little {
			u32::from_le_bytes(word)
		} else {
			u32::from_be_bytes(word)
		})
	};

	if word(1..5)? & EWKB_SRID == 0 {
		return Ok(None);
	}

	word(5..9).map(Some)
}

/// `geometry` in GeoPackage's binary encoding with `srid`: a header with
/// the bounding box, then little-endian WKB.
#[doc(hidden)]
pub fn encode_gpkg(geometry: &Geometry, srid: u32) -> Result<Vec<u8>, String> {
	let srid = i32::try_from(srid).map_err(|_| format!("SRID {srid} is too large"))?;
	let envelope = envelope(geometry);
	let mut bytes = Vec::with_capacity(48);
	bytes.extend_from_slice(&GPKG_MAGIC);
	bytes.push(0);

	match envelope {
		// Little-endian, with an XY envelope.
		Some(_) => bytes.push(0b0000_0011),
		// Little-endian, empty, with no envelope.
		None => bytes.push(GPKG_EMPTY | 1),
	}

	bytes.extend_from_slice(&srid.to_le_bytes());

	for bound in envelope.into_iter().flatten() {
		bytes.extend_from_slice(&bound.to_le_bytes());
	}

	bytes.extend_from_slice(&encode_wkb(geometry)?);
	Ok(bytes)
}

/// A GeoPackage geometry, read.
#[derive(Debug, PartialEq)]
pub(crate) struct Gpkg {
	pub(crate) srid: i32,
	/// The header's bounding box, when it has one.
	pub(crate) envelope: Option<[f64; 4]>,
	/// Whether the header says the shape is empty.
	pub(crate) empty: bool,
	/// Where the WKB starts.
	pub(crate) wkb: usize,
}

impl Gpkg {
	/// Reads a GeoPackage header.
	pub(crate) fn header(bytes: &[u8]) -> Result<Self, String> {
		let too_short = || "invalid GeoPackage geometry: too short".to_string();

		if bytes.get(..2) != Some(GPKG_MAGIC.as_slice()) {
			return Err("not a GeoPackage geometry: it doesn't start with `GP`".into());
		}

		let version = *bytes.get(2).ok_or_else(too_short)?;
		let flags = *bytes.get(3).ok_or_else(too_short)?;

		if version != 0 {
			return Err(format!(
				"GeoPackage geometry version {version} isn't supported"
			));
		}

		if flags & GPKG_EXTENDED != 0 {
			return Err("extended GeoPackage geometry types aren't supported".into());
		}

		let little = flags & 1 == 1;
		let doubles = match (flags >> 1) & 0b111 {
			0 => 0,
			1 => 4,
			2 | 3 => 6,
			4 => 8,
			other => return Err(format!("GeoPackage envelope kind {other} isn't defined")),
		};
		let end = 8 + doubles * 8;

		if bytes.len() < end {
			return Err(too_short());
		}

		let srid: [u8; 4] = bytes[4..8].try_into().expect("checked the length");
		let srid = if little {
			i32::from_le_bytes(srid)
		} else {
			i32::from_be_bytes(srid)
		};
		let double = |index: usize| {
			let bytes: [u8; 8] = bytes[8 + index * 8..16 + index * 8]
				.try_into()
				.expect("checked the length");
			if little {
				f64::from_le_bytes(bytes)
			} else {
				f64::from_be_bytes(bytes)
			}
		};
		let envelope = (doubles > 0).then(|| [double(0), double(1), double(2), double(3)]);

		Ok(Self {
			srid,
			envelope,
			empty: flags & GPKG_EMPTY != 0,
			wkb: end,
		})
	}
}

/// The SRID and shape of a GeoPackage geometry or of EWKB, which is how
/// PostGIS returns them. EWKB without an SRID reads as SRID 0.
#[doc(hidden)]
pub fn decode_any(bytes: &[u8]) -> Result<(i64, Geometry), String> {
	if bytes.starts_with(&GPKG_MAGIC) {
		let header = Gpkg::header(bytes)?;
		let geometry = decode_wkb(&bytes[header.wkb..])?;
		return Ok((i64::from(header.srid), geometry));
	}

	let srid = ewkb_srid(bytes)?.unwrap_or(0);
	Ok((i64::from(srid), decode_wkb(bytes)?))
}

/// Converts EWKB, as PostGIS returns it, to the database layer's GeoPackage
/// encoding.
#[cfg(feature = "postgres")]
pub(crate) fn ewkb_to_gpkg(bytes: &[u8]) -> Result<Vec<u8>, String> {
	let (srid, geometry) = decode_any(bytes)?;
	encode_gpkg(
		&geometry,
		u32::try_from(srid).map_err(|_| format!("SRID {srid} is negative"))?,
	)
}

/// Converts a GeoPackage geometry, or EWKB, to EWKB for PostGIS.
#[cfg(feature = "postgres")]
pub(crate) fn to_ewkb(bytes: &[u8]) -> Result<Vec<u8>, String> {
	let (srid, geometry) = decode_any(bytes)?;
	encode_ewkb(
		&geometry,
		u32::try_from(srid).map_err(|_| format!("SRID {srid} is negative"))?,
	)
}
