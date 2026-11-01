//! Vectors as columns, and their stored forms.
//!
//! A vector is held as sqlite-vec's layout whichever the database: four
//! little-endian bytes a float (16-bit floats widened to 32), eight bits a
//! byte with the first in its lowest bit. Sparse vectors, which only
//! Postgres stores, are held as pgvector's binary form. The Postgres driver
//! converts the others to and from pgvector's.

use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::VectorElement;

use super::BitVector;
use super::HalfVector;
use super::SparseVector;
use super::Vector;
use super::f16;
use super::finite;
use crate::Column;
use crate::DecodeError;
use crate::EncodeError;
use crate::Value;

mod sealed {
	pub trait Sealed {}
}

/// A vector type: [`Vector`], [`HalfVector`], [`BitVector`], or
/// [`SparseVector`].
pub trait VectorValue: sealed::Sealed + Column + Clone + Send + Sync + 'static {
	/// What each dimension holds.
	#[doc(hidden)]
	const ELEMENT: VectorElement;
	/// How many dimensions it has.
	const DIMENSIONS: usize;

	/// The value as the database layer stores it, checked.
	#[doc(hidden)]
	fn to_bytes(&self) -> Result<Vec<u8>, String>;

	/// The value [`VectorValue::to_bytes`] stored.
	#[doc(hidden)]
	fn from_bytes(bytes: &[u8]) -> Result<Self, String>;
}

/// A column holding vectors: [`Vector`], [`HalfVector`], [`BitVector`],
/// [`SparseVector`], and `Option` of any of them.
/// [`Query::nearest`](crate::Query::nearest) exists for these columns.
#[diagnostic::on_unimplemented(
	message = "`{Self}` isn't a vector column",
	label = "`#[db(vector(…))]` needs `Vector<N>`, `HalfVector<N>`, `BitVector<N>`, or \
	         `SparseVector<N>`"
)]
pub trait VectorColumn: Column {
	/// The value the column holds, without `Option`.
	type Value: VectorValue;
}

impl<V: VectorColumn> VectorColumn for Option<V> {
	type Value = V::Value;
}

/// The floats in sqlite-vec's layout.
fn floats_to_bytes(values: impl ExactSizeIterator<Item = f32> + Clone) -> Result<Vec<u8>, String> {
	finite(values.clone())?;
	let mut bytes = Vec::with_capacity(values.len() * 4);

	for value in values {
		bytes.extend_from_slice(&value.to_le_bytes());
	}

	Ok(bytes)
}

/// The floats [`floats_to_bytes`] laid out, `n` of them.
fn floats_from_bytes(bytes: &[u8], n: usize, name: &str) -> Result<Vec<f32>, String> {
	if bytes.len() != n * 4 {
		return Err(format!(
			"a {name}<{n}> is {} bytes, and the stored value is {}",
			n * 4,
			bytes.len()
		));
	}

	Ok(bytes
		.as_chunks::<4>()
		.0
		.iter()
		.map(|chunk| f32::from_le_bytes(*chunk))
		.collect())
}

impl<const N: usize> sealed::Sealed for Vector<N> {}

impl<const N: usize> VectorValue for Vector<N> {
	const DIMENSIONS: usize = N;
	const ELEMENT: VectorElement = VectorElement::Float32;

	fn to_bytes(&self) -> Result<Vec<u8>, String> {
		floats_to_bytes(self.0.iter().copied())
	}

	fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
		let values = floats_from_bytes(bytes, N, "Vector")?;
		Self::try_from(values).map_err(|_| "the stored vector has the wrong length".to_string())
	}
}

impl<const N: usize> sealed::Sealed for HalfVector<N> {}

impl<const N: usize> VectorValue for HalfVector<N> {
	const DIMENSIONS: usize = N;
	const ELEMENT: VectorElement = VectorElement::Float16;

	fn to_bytes(&self) -> Result<Vec<u8>, String> {
		floats_to_bytes(self.0.iter().map(|value| value.to_f32()))
	}

	fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
		let values: Vec<f16> = floats_from_bytes(bytes, N, "HalfVector")?
			.into_iter()
			.map(f16::from_f32)
			.collect();
		Self::try_from(values).map_err(|_| "the stored vector has the wrong length".to_string())
	}
}

impl<const N: usize> sealed::Sealed for BitVector<N> {}

impl<const N: usize> VectorValue for BitVector<N> {
	const DIMENSIONS: usize = N;
	const ELEMENT: VectorElement = VectorElement::Bit;

	fn to_bytes(&self) -> Result<Vec<u8>, String> {
		Ok(self.0.to_vec())
	}

	fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
		Self::from_bytes(bytes).ok_or_else(|| {
			format!(
				"a BitVector<{N}> is {} bytes, and the stored value is {}",
				N / 8,
				bytes.len()
			)
		})
	}
}

impl<const N: usize> sealed::Sealed for SparseVector<N> {}

/// pgvector's binary form of a sparse vector: its dimensions, how many
/// entries it has, a reserved zero, then each entry's index and each
/// entry's value, all big-endian.
impl<const N: usize> VectorValue for SparseVector<N> {
	const DIMENSIONS: usize = N;
	const ELEMENT: VectorElement = VectorElement::Sparse;

	fn to_bytes(&self) -> Result<Vec<u8>, String> {
		let dimensions = i32::try_from(N)
			.map_err(|_| format!("a sparse vector of {N} dimensions is too long"))?;
		let count = i32::try_from(self.indices.len())
			.map_err(|_| "the sparse vector has too many entries".to_string())?;
		let mut bytes = Vec::with_capacity(12 + self.indices.len() * 8);
		bytes.extend_from_slice(&dimensions.to_be_bytes());
		bytes.extend_from_slice(&count.to_be_bytes());
		bytes.extend_from_slice(&0_i32.to_be_bytes());

		for index in &self.indices {
			bytes.extend_from_slice(&index.to_be_bytes());
		}

		for value in &self.values {
			bytes.extend_from_slice(&value.to_be_bytes());
		}

		Ok(bytes)
	}

	fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
		let word = |at: usize| {
			bytes
				.get(at..at + 4)
				.map(|word| [word[0], word[1], word[2], word[3]])
				.ok_or_else(|| "the stored sparse vector is cut short".to_string())
		};
		let dimensions = u32::from_be_bytes(word(0)?) as usize;
		let count = u32::from_be_bytes(word(4)?) as usize;

		if dimensions != N {
			return Err(format!(
				"the stored sparse vector has {dimensions} dimensions, and the column's type \
				 expects {N}"
			));
		}

		if bytes.len() != 12 + count * 8 {
			return Err("the stored sparse vector is the wrong length".into());
		}

		let entries = (0..count)
			.map(|entry| {
				Ok((
					u32::from_be_bytes(word(12 + entry * 4)?),
					f32::from_be_bytes(word(12 + (count + entry) * 4)?),
				))
			})
			.collect::<Result<Vec<_>, String>>()?;
		Self::new(entries)
	}
}

macro_rules! vector_columns {
	($($ty:ident),*) => {$(
		impl<const N: usize> VectorColumn for $ty<N> {
			type Value = Self;
		}

		impl<const N: usize> Column for $ty<N> {
			fn column_type() -> ColumnType {
				ColumnType::Vector {
					element: <Self as VectorValue>::ELEMENT,
					dimensions: N as u32,
				}
			}

			fn to_value(&self) -> Result<Value, EncodeError> {
				self.to_bytes().map(Value::Bytes).map_err(EncodeError::new)
			}

			fn from_value(value: Value) -> Result<Self, DecodeError> {
				match value {
					Value::Bytes(bytes) => {
						<Self as VectorValue>::from_bytes(&bytes).map_err(DecodeError::custom)
					}
					other => Err(DecodeError::unexpected("a vector", &other)),
				}
			}
		}
	)*};
}

vector_columns!(Vector, HalfVector, BitVector, SparseVector);

/// A stored vector, in sqlite-vec's layout, as pgvector's binary form for
/// a parameter of Postgres type `ty`: its dimensions and a reserved zero,
/// then each float big-endian; or a bit string's length in bits, then its
/// bytes, the first bit in each byte's highest. Sparse vectors are stored
/// in pgvector's form already. `None` for other types.
#[cfg(feature = "postgres")]
pub(crate) fn to_postgres(ty: &str, bytes: &[u8]) -> Option<Result<Vec<u8>, String>> {
	let floats = |half: bool| {
		let count = bytes.len() / 4;
		let dimensions = u16::try_from(count)
			.map_err(|_| format!("a vector of {count} dimensions is too long for pgvector"))?;
		let mut out = Vec::with_capacity(4 + bytes.len());
		out.extend_from_slice(&dimensions.to_be_bytes());
		out.extend_from_slice(&0_u16.to_be_bytes());

		for chunk in bytes.as_chunks::<4>().0 {
			let value = f32::from_le_bytes(*chunk);

			if half {
				out.extend_from_slice(&f16::from_f32(value).to_be_bytes());
			} else {
				out.extend_from_slice(&value.to_be_bytes());
			}
		}

		Ok(out)
	};

	match ty {
		"vector" => Some(floats(false)),
		"halfvec" => Some(floats(true)),
		"bit" | "varbit" => {
			let bits = u32::try_from(bytes.len() * 8)
				.map_err(|_| "the bit vector is too long".to_string());
			Some(bits.map(|bits| {
				let mut out = bits.to_be_bytes().to_vec();
				out.extend(bytes.iter().map(|byte| byte.reverse_bits()));
				out
			}))
		}
		"sparsevec" => Some(Ok(bytes.to_vec())),
		_ => None,
	}
}

/// A value of Postgres type `ty` in pgvector's binary form, stored as
/// [`to_postgres`] reads it. `None` for other types.
#[cfg(feature = "postgres")]
pub(crate) fn from_postgres(ty: &str, raw: &[u8]) -> Option<Result<Vec<u8>, String>> {
	let floats = |half: bool| {
		let width = if half { 2 } else { 4 };
		let Some(header) = raw.get(..4) else {
			return Err("pgvector sent a vector without its dimensions".to_string());
		};
		let dimensions = usize::from(u16::from_be_bytes([header[0], header[1]]));

		if raw.len() != 4 + dimensions * width {
			return Err("pgvector sent a vector of the wrong length".to_string());
		}

		let mut out = Vec::with_capacity(dimensions * 4);

		for chunk in raw[4..].chunks_exact(width) {
			let value = if half {
				f16::from_be_bytes([chunk[0], chunk[1]]).to_f32()
			} else {
				f32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
			};
			out.extend_from_slice(&value.to_le_bytes());
		}

		Ok(out)
	};

	match ty {
		"vector" => Some(floats(false)),
		"halfvec" => Some(floats(true)),
		"bit" | "varbit" => {
			Some(
				raw.get(4..)
					.map(|bytes| bytes.iter().map(|byte| byte.reverse_bits()).collect())
					.ok_or_else(|| "Postgres sent a bit string without its length".to_string()),
			)
		}
		"sparsevec" => Some(Ok(raw.to_vec())),
		_ => None,
	}
}
