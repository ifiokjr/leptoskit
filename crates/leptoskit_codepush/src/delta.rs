//! Binary deltas between bundle versions.
//!
//! The encoder indexes the base in fixed blocks and streams the target,
//! turning every base hit into a copy instruction and everything else into
//! inserts, extending matches greedily in both directions. For wasm bundles —
//! where most bytes survive between versions — this typically shrinks an
//! update to a small fraction of the full download.

use std::collections::HashMap;

use serde::Deserialize;
use serde::Serialize;

use crate::CodePushError;

/// The block size used to index the base.
const BLOCK: usize = 512;

/// The largest output [`apply_delta`] will produce: 512 MiB. Deltas come off
/// the network, so their declared output size is checked against this before
/// anything is allocated; no real wasm bundle comes close.
pub const MAX_DELTA_OUTPUT: u64 = 512 * 1024 * 1024;

/// One delta instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeltaOp {
	/// Copies `len` bytes from `offset` in the base.
	Copy {
		/// The offset in the base bytes.
		offset: u64,
		/// The number of bytes to copy.
		len: u64,
	},
	/// Appends the literal bytes.
	Insert {
		/// The literal bytes.
		bytes: Vec<u8>,
	},
}

/// A binary delta from a base to a target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delta {
	/// The SHA-256 of the base this delta was computed against.
	pub base_hash: String,
	/// The SHA-256 of the target this delta produces.
	pub target_hash: String,
	/// The instructions, in order.
	pub ops: Vec<DeltaOp>,
}

/// Computes the delta from `base` to `target`.
pub fn create_delta(base: &[u8], target: &[u8]) -> Delta {
	// Index the base at fixed strides so any target window that mostly
	// survives between versions finds a candidate immediately.
	let mut index: HashMap<u64, Vec<usize>> = HashMap::new();
	let mut offset = 0;
	while offset + BLOCK <= base.len() {
		let hash = quick_hash(&base[offset..offset + BLOCK]);
		index.entry(hash).or_default().push(offset);
		offset += BLOCK;
	}

	let mut ops: Vec<DeltaOp> = Vec::new();
	let mut literal: Vec<u8> = Vec::new();
	let mut position = 0;
	while position < target.len() {
		let mut matched = None;
		if position + BLOCK <= target.len() {
			let hash = quick_hash(&target[position..position + BLOCK]);
			if let Some(candidates) = index.get(&hash) {
				matched = candidates
					.iter()
					.map(|&base_offset| {
						let length = extend_match(base, base_offset, target, position);
						(base_offset, length)
					})
					.max_by_key(|&(_, length)| length)
					.filter(|&(_, length)| length >= BLOCK as u64);
			}
		}
		if let Some((base_offset, length)) = matched {
			flush_literal(&mut ops, &mut literal);
			ops.push(DeltaOp::Copy {
				offset: base_offset as u64,
				len: length,
			});
			position += length as usize;
		} else {
			literal.push(target[position]);
			position += 1;
		}
	}
	flush_literal(&mut ops, &mut literal);

	Delta {
		base_hash: crate::manifest::hash_bytes(base),
		target_hash: crate::manifest::hash_bytes(target),
		ops,
	}
}

/// Applies a delta to `base`, producing the target bytes.
///
/// The base hash is verified first, then every op is bounds-checked and the
/// total output length is computed with checked arithmetic and capped at
/// [`MAX_DELTA_OUTPUT`] before the output is allocated. The result must hash
/// to the delta's target hash.
pub fn apply_delta(base: &[u8], delta: &Delta) -> Result<Vec<u8>, CodePushError> {
	apply_delta_within(base, delta, MAX_DELTA_OUTPUT)
}

/// [`apply_delta`] with a caller-chosen output limit, for hosts that bound
/// memory more tightly than [`MAX_DELTA_OUTPUT`], such as mobile apps.
pub fn apply_delta_within(
	base: &[u8],
	delta: &Delta,
	max_output: u64,
) -> Result<Vec<u8>, CodePushError> {
	let actual = crate::manifest::hash_bytes(base);
	if actual != delta.base_hash {
		return Err(CodePushError::DeltaBaseMismatch {
			expected: delta.base_hash.clone(),
			actual,
		});
	}

	let mut total: u64 = 0;
	for op in &delta.ops {
		let len = match op {
			DeltaOp::Copy { offset, len } => {
				let in_bounds = offset
					.checked_add(*len)
					.is_some_and(|end| end <= base.len() as u64);
				if !in_bounds {
					return Err(CodePushError::Malformed(format!(
						"delta copy out of bounds: {offset} + {len} over {}",
						base.len()
					)));
				}
				*len
			}
			DeltaOp::Insert { bytes } => bytes.len() as u64,
		};
		total = total
			.checked_add(len)
			.filter(|&total| total <= max_output)
			.ok_or_else(|| {
				CodePushError::Malformed(format!(
					"delta output exceeds the {max_output} byte limit"
				))
			})?;
	}

	let mut out = Vec::with_capacity(total as usize);
	for op in &delta.ops {
		match op {
			// Bounds were checked above, and both ends fit in `base.len()`.
			DeltaOp::Copy { offset, len } => {
				let start = *offset as usize;
				out.extend_from_slice(&base[start..start + *len as usize]);
			}
			DeltaOp::Insert { bytes } => out.extend_from_slice(bytes),
		}
	}
	let produced = crate::manifest::hash_bytes(&out);
	if produced != delta.target_hash {
		return Err(CodePushError::HashMismatch {
			path: "<delta result>".to_owned(),
			expected: delta.target_hash.clone(),
			actual: produced,
		});
	}
	Ok(out)
}

fn flush_literal(ops: &mut Vec<DeltaOp>, literal: &mut Vec<u8>) {
	if !literal.is_empty() {
		ops.push(DeltaOp::Insert {
			bytes: std::mem::take(literal),
		});
	}
}

/// A cheap rolling-style hash over a block: FNV-1a over the first and last
/// quarters, which keeps equal blocks together while avoiding full-hash cost.
fn quick_hash(block: &[u8]) -> u64 {
	let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
	let quarter = block.len() / 4;
	for &byte in &block[..quarter] {
		hash ^= u64::from(byte);
		hash = hash.wrapping_mul(0x100_0000_01b3);
	}
	for &byte in &block[block.len() - quarter..] {
		hash ^= u64::from(byte);
		hash = hash.wrapping_mul(0x100_0000_01b3);
	}
	hash
}

/// Extends a candidate match forward, returning the match length.
fn extend_match(base: &[u8], base_offset: usize, target: &[u8], target_offset: usize) -> u64 {
	let mut length = 0usize;
	while base_offset + length < base.len()
		&& target_offset + length < target.len()
		&& base[base_offset + length] == target[target_offset + length]
	{
		length += 1;
	}
	length as u64
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn apply_rejects_output_over_the_limit_before_allocating() {
		let base = vec![0u8; 1024 * 1024];
		let copies = MAX_DELTA_OUTPUT / base.len() as u64 + 1;
		let delta = Delta {
			base_hash: crate::manifest::hash_bytes(&base),
			target_hash: String::new(),
			ops: (0..copies)
				.map(|_| {
					DeltaOp::Copy {
						offset: 0,
						len: base.len() as u64,
					}
				})
				.collect(),
		};
		let result = apply_delta(&base, &delta);
		assert!(
			matches!(&result, Err(CodePushError::Malformed(message)) if message.contains("limit")),
			"got {result:?}"
		);
	}

	#[test]
	fn apply_rejects_copies_whose_end_overflows() {
		let base = vec![0u8; 16];
		let delta = Delta {
			base_hash: crate::manifest::hash_bytes(&base),
			target_hash: String::new(),
			ops: vec![DeltaOp::Copy {
				offset: 8,
				len: u64::MAX,
			}],
		};
		let result = apply_delta(&base, &delta);
		assert!(
			matches!(result, Err(CodePushError::Malformed(_))),
			"got {result:?}"
		);
	}
}
