//! Vector columns: embeddings and other fixed-length vectors, stored by
//! pgvector on Postgres and by sqlite-vec, linked into the build, on
//! SQLite, and searched by distance.
//!
//! ```ignore
//! use leptoskit::db::vector::Vector;
//!
//! #[db::table(name = "articles", public)]
//! pub struct Article {
//! 	pub id: Id<Article>,
//! 	pub title: String,
//! 	#[db(vector(distance = cosine, index = hnsw, source(title)))]
//! 	pub embedding: Option<Vector<1536>>,
//! }
//!
//! let similar = Article::query()
//! 	.nearest(|a| a.embedding, &query_embedding)
//! 	.limit(10)
//! 	.all(&db)
//! 	.await?;
//! ```
//!
//! The number of dimensions is part of the type, so storing a 768-dimension
//! embedding in a 1536-dimension column fails to compile:
//!
//! - [`Vector`] holds 32-bit floats: pgvector's `vector`, sqlite-vec's `float`.
//! - [`HalfVector`] holds 16-bit floats, half the size: pgvector's `halfvec`;
//!   SQLite keeps them as 32-bit floats.
//! - [`BitVector`] holds bits, for binary quantisation: `bit` on both.
//! - [`SparseVector`] holds mostly-zero floats as their nonzero entries:
//!   pgvector's `sparsevec`, on Postgres only.
//!
//! Every one travels as JSON: floats as an array of numbers, bits as a
//! string of `0` and `1`, sparse vectors as their indices and values.

#[cfg(feature = "server")]
mod column;
#[cfg(feature = "server")]
mod hybrid;
#[cfg(feature = "server")]
mod nearest;
#[cfg(feature = "sqlite")]
pub(crate) mod sqlite;
#[cfg(feature = "server")]
mod stale;

use std::borrow::Cow;
use std::fmt;

#[cfg(feature = "server")]
pub use column::VectorColumn;
#[cfg(feature = "server")]
pub use column::VectorValue;
#[cfg(feature = "postgres")]
pub(crate) use column::from_postgres;
#[cfg(feature = "postgres")]
pub(crate) use column::to_postgres;
/// The 16-bit float [`HalfVector`] holds.
pub use half::f16;
#[cfg(feature = "server")]
pub use hybrid::Hybrid;
#[cfg(feature = "server")]
pub use hybrid::HybridHit;
#[cfg(feature = "server")]
pub use nearest::Nearest;
#[cfg(feature = "server")]
pub use nearest::NearestWithDistance;
use schemars::JsonSchema;
use schemars::SchemaGenerator;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
#[cfg(feature = "server")]
pub use stale::Embedder;
#[cfg(feature = "server")]
#[doc(hidden)]
pub use stale::Stale;

/// A vector of `N` 32-bit floats, such as an embedding from a language
/// model. Every value must be finite.
#[derive(Clone, PartialEq)]
pub struct Vector<const N: usize>(Box<[f32; N]>);

impl<const N: usize> Vector<N> {
	/// The vector holding `values`.
	pub fn new(values: [f32; N]) -> Self {
		Self(Box::new(values))
	}

	/// The values.
	pub fn as_slice(&self) -> &[f32] {
		self.0.as_slice()
	}

	/// The values, as an array.
	pub fn as_array(&self) -> &[f32; N] {
		&self.0
	}

	/// The same vector scaled to length 1, or `None` when every value is
	/// zero. Cosine distance and inner product agree on normalised vectors.
	#[must_use]
	pub fn normalized(&self) -> Option<Self> {
		let length = self
			.0
			.iter()
			.map(|value| f64::from(*value).powi(2))
			.sum::<f64>()
			.sqrt();

		if length == 0.0 {
			return None;
		}

		Some(Self::new(
			self.0.map(|value| (f64::from(value) / length) as f32),
		))
	}
}

impl<const N: usize> TryFrom<Vec<f32>> for Vector<N> {
	type Error = Vec<f32>;

	/// The vector holding `values`, or `values` back when there aren't
	/// exactly `N`.
	fn try_from(values: Vec<f32>) -> Result<Self, Vec<f32>> {
		values
			.into_boxed_slice()
			.try_into()
			.map(Self)
			.map_err(Vec::from)
	}
}

impl<const N: usize> From<[f32; N]> for Vector<N> {
	fn from(values: [f32; N]) -> Self {
		Self::new(values)
	}
}

impl<const N: usize> fmt::Debug for Vector<N> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Vector<{N}>")?;
		f.debug_list().entries(self.0.iter()).finish()
	}
}

/// A vector of `N` 16-bit floats: half a [`Vector`]'s size, with about
/// three significant digits, which is enough for most embeddings. Every
/// value must be finite.
#[derive(Clone, PartialEq)]
pub struct HalfVector<const N: usize>(Box<[f16; N]>);

impl<const N: usize> HalfVector<N> {
	/// The vector holding `values`.
	pub fn new(values: [f16; N]) -> Self {
		Self(Box::new(values))
	}

	/// The values.
	pub fn as_slice(&self) -> &[f16] {
		self.0.as_slice()
	}

	/// The values, widened to 32-bit floats, which hold each one exactly.
	pub fn to_vector(&self) -> Vector<N> {
		Vector(Box::new(self.0.map(f16::to_f32)))
	}
}

impl<const N: usize> From<&Vector<N>> for HalfVector<N> {
	/// Each value rounded to the nearest 16-bit float. Values beyond
	/// ±65504 become infinite, which can't be stored.
	fn from(vector: &Vector<N>) -> Self {
		Self(Box::new(vector.0.map(f16::from_f32)))
	}
}

impl<const N: usize> TryFrom<Vec<f16>> for HalfVector<N> {
	type Error = Vec<f16>;

	/// The vector holding `values`, or `values` back when there aren't
	/// exactly `N`.
	fn try_from(values: Vec<f16>) -> Result<Self, Vec<f16>> {
		values
			.into_boxed_slice()
			.try_into()
			.map(Self)
			.map_err(Vec::from)
	}
}

impl<const N: usize> fmt::Debug for HalfVector<N> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "HalfVector<{N}>")?;
		f.debug_list().entries(self.0.iter()).finish()
	}
}

/// A vector of `N` bits, such as an embedding quantised to one bit a
/// dimension, compared by how many bits differ. `N` is a multiple of 8;
/// another fails to compile where the vector is made.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct BitVector<const N: usize>(Box<[u8]>);

impl<const N: usize> BitVector<N> {
	/// Refuses dimensions that aren't a positive multiple of 8, which
	/// sqlite-vec needs, when the vector is made.
	const WHOLE_BYTES: () = assert!(
		N > 0 && N % 8 == 0,
		"a BitVector's dimensions are a positive multiple of 8"
	);

	/// The vector with these bits.
	pub fn from_bits(bits: [bool; N]) -> Self {
		let () = Self::WHOLE_BYTES;
		let mut bytes = vec![0_u8; N / 8].into_boxed_slice();

		for (index, bit) in bits.iter().enumerate() {
			if *bit {
				bytes[index / 8] |= 1 << (index % 8);
			}
		}

		Self(bytes)
	}

	/// The vector packed in `bytes`, eight bits a byte, the first bit in
	/// each byte's lowest bit, as sqlite-vec packs them; or `None` when
	/// there aren't `N / 8` bytes.
	pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
		let () = Self::WHOLE_BYTES;
		(bytes.len() == N / 8).then(|| Self(bytes.into()))
	}

	/// Each dimension of `vector` as a bit, set when it's above zero: the
	/// binary quantisation pgvector's `binary_quantize` and sqlite-vec's
	/// `vec_quantize_binary` make.
	pub fn quantize(vector: &Vector<N>) -> Self {
		Self::from_bits(vector.0.map(|value| value > 0.0))
	}

	/// The bits, packed as [`BitVector::from_bytes`] reads them.
	pub fn as_bytes(&self) -> &[u8] {
		&self.0
	}

	/// The bit at `index`, or `None` past the end.
	pub fn get(&self, index: usize) -> Option<bool> {
		(index < N).then(|| self.0[index / 8] & (1 << (index % 8)) != 0)
	}

	/// How many bits are set.
	pub fn count_ones(&self) -> u32 {
		self.0.iter().map(|byte| byte.count_ones()).sum()
	}

	/// The bits as `0` and `1`, first to last: the vector's JSON form.
	fn to_text(&self) -> String {
		(0..N)
			.map(|index| {
				if self.get(index) == Some(true) {
					'1'
				} else {
					'0'
				}
			})
			.collect()
	}

	/// The vector [`BitVector::to_text`] wrote.
	fn from_text(text: &str) -> Result<Self, String> {
		let () = Self::WHOLE_BYTES;

		if text.len() != N {
			return Err(format!("a BitVector<{N}> has {N} bits, not {}", text.len()));
		}

		let mut bytes = vec![0_u8; N / 8].into_boxed_slice();

		for (index, bit) in text.bytes().enumerate() {
			match bit {
				b'1' => bytes[index / 8] |= 1 << (index % 8),
				b'0' => {}
				_ => return Err("a BitVector's bits are written as `0` and `1`".into()),
			}
		}

		Ok(Self(bytes))
	}
}

impl<const N: usize> fmt::Debug for BitVector<N> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "BitVector<{N}>({})", self.to_text())
	}
}

/// A vector of `N` 32-bit floats that are mostly zero, kept as the index
/// and value of each one that isn't, such as a bag of words over a large
/// vocabulary. Postgres only: SQLite has no sparse vectors.
#[derive(Clone, Debug, PartialEq)]
pub struct SparseVector<const N: usize> {
	indices: Vec<u32>,
	values: Vec<f32>,
}

impl<const N: usize> SparseVector<N> {
	/// The vector with each `(index, value)` entry; zeros are left out.
	/// Fails when an index is `N` or more, appears twice, or a value isn't
	/// finite.
	pub fn new(entries: impl IntoIterator<Item = (u32, f32)>) -> Result<Self, String> {
		let mut entries: Vec<(u32, f32)> = entries
			.into_iter()
			.filter(|(_, value)| *value != 0.0)
			.collect();
		entries.sort_by_key(|(index, _)| *index);

		for pair in entries.windows(2) {
			if pair[0].0 == pair[1].0 {
				return Err(format!("index {} appears twice", pair[0].0));
			}
		}

		if let Some((index, _)) = entries.iter().find(|(index, _)| *index as usize >= N) {
			return Err(format!(
				"index {index} is past a SparseVector<{N}>'s last, {}",
				N.saturating_sub(1)
			));
		}

		if let Some((index, value)) = entries.iter().find(|(_, value)| !value.is_finite()) {
			return Err(format!("the value at {index}, {value}, isn't finite"));
		}

		let (indices, values) = entries.into_iter().unzip();
		Ok(Self { indices, values })
	}

	/// The nonzero values of `values`.
	pub fn from_dense(values: &[f32; N]) -> Result<Self, String> {
		Self::new(
			values
				.iter()
				.enumerate()
				.map(|(index, value)| (index as u32, *value)),
		)
	}

	/// The nonzero entries, by index, in order.
	pub fn entries(&self) -> impl Iterator<Item = (u32, f32)> + '_ {
		self.indices
			.iter()
			.copied()
			.zip(self.values.iter().copied())
	}

	/// The value at `index`: zero when it isn't kept.
	pub fn get(&self, index: u32) -> f32 {
		self.indices
			.binary_search(&index)
			.map_or(0.0, |position| self.values[position])
	}

	/// How many values aren't zero.
	pub fn nonzero(&self) -> usize {
		self.indices.len()
	}
}

/// A float vector's values, each finite.
fn finite(values: impl IntoIterator<Item = f32>) -> Result<(), String> {
	match values
		.into_iter()
		.enumerate()
		.find(|(_, value)| !value.is_finite())
	{
		Some((index, value)) => Err(format!("the value at {index}, {value}, isn't finite")),
		None => Ok(()),
	}
}

impl<const N: usize> Serialize for Vector<N> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.collect_seq(self.0.iter())
	}
}

impl<'de, const N: usize> Deserialize<'de> for Vector<N> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let values = Vec::<f32>::deserialize(deserializer)?;
		finite(values.iter().copied()).map_err(serde::de::Error::custom)?;
		let found = values.len();
		Self::try_from(values).map_err(|_| {
			serde::de::Error::custom(format!("a Vector<{N}> has {N} values, not {found}"))
		})
	}
}

impl<const N: usize> Serialize for HalfVector<N> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.collect_seq(self.0.iter().map(|value| value.to_f32()))
	}
}

impl<'de, const N: usize> Deserialize<'de> for HalfVector<N> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let values: Vec<f16> = Vec::<f32>::deserialize(deserializer)?
			.into_iter()
			.map(f16::from_f32)
			.collect();
		finite(values.iter().map(|value| value.to_f32())).map_err(serde::de::Error::custom)?;
		let found = values.len();
		Self::try_from(values).map_err(|_| {
			serde::de::Error::custom(format!("a HalfVector<{N}> has {N} values, not {found}"))
		})
	}
}

impl<const N: usize> Serialize for BitVector<N> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(&self.to_text())
	}
}

impl<'de, const N: usize> Deserialize<'de> for BitVector<N> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let text = Cow::<str>::deserialize(deserializer)?;
		Self::from_text(&text).map_err(serde::de::Error::custom)
	}
}

/// A sparse vector's JSON form.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sparse {
	indices: Vec<u32>,
	values: Vec<f32>,
}

impl<const N: usize> Serialize for SparseVector<N> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		Sparse {
			indices: self.indices.clone(),
			values: self.values.clone(),
		}
		.serialize(serializer)
	}
}

impl<'de, const N: usize> Deserialize<'de> for SparseVector<N> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let sparse = Sparse::deserialize(deserializer)?;

		if sparse.indices.len() != sparse.values.len() {
			return Err(serde::de::Error::custom(
				"a sparse vector has as many indices as values",
			));
		}

		Self::new(sparse.indices.into_iter().zip(sparse.values)).map_err(serde::de::Error::custom)
	}
}

/// The JSON Schema of `N` numbers.
fn floats_schema(name: &str, n: usize) -> schemars::Schema {
	schemars::json_schema!({
		"type": "array",
		"items": { "type": "number" },
		"minItems": n,
		"maxItems": n,
		"title": format!("{name}<{n}>"),
		"description": format!("A vector of {n} numbers"),
	})
}

impl<const N: usize> JsonSchema for Vector<N> {
	fn schema_name() -> Cow<'static, str> {
		format!("Vector{N}").into()
	}

	fn json_schema(_: &mut SchemaGenerator) -> schemars::Schema {
		floats_schema("Vector", N)
	}
}

impl<const N: usize> JsonSchema for HalfVector<N> {
	fn schema_name() -> Cow<'static, str> {
		format!("HalfVector{N}").into()
	}

	fn json_schema(_: &mut SchemaGenerator) -> schemars::Schema {
		floats_schema("HalfVector", N)
	}
}

impl<const N: usize> JsonSchema for BitVector<N> {
	fn schema_name() -> Cow<'static, str> {
		format!("BitVector{N}").into()
	}

	fn json_schema(_: &mut SchemaGenerator) -> schemars::Schema {
		schemars::json_schema!({
			"type": "string",
			"pattern": format!("^[01]{{{N}}}$"),
			"title": format!("BitVector<{N}>"),
			"description": format!("A vector of {N} bits, written as `0` and `1`"),
		})
	}
}

impl<const N: usize> JsonSchema for SparseVector<N> {
	fn schema_name() -> Cow<'static, str> {
		format!("SparseVector{N}").into()
	}

	fn json_schema(_: &mut SchemaGenerator) -> schemars::Schema {
		schemars::json_schema!({
			"type": "object",
			"properties": {
				"indices": {
					"type": "array",
					"items": { "type": "integer", "minimum": 0, "maximum": N.saturating_sub(1) },
				},
				"values": { "type": "array", "items": { "type": "number" } },
			},
			"required": ["indices", "values"],
			"additionalProperties": false,
			"title": format!("SparseVector<{N}>"),
			"description": format!(
				"A vector of {N} numbers, mostly zero: the index and value of each that isn't"
			),
		})
	}
}

#[cfg(test)]
mod tests;
