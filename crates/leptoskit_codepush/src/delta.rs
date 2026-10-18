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
pub fn apply_delta(base: &[u8], delta: &Delta) -> Result<Vec<u8>, CodePushError> {
	let actual = crate::manifest::hash_bytes(base);
	if actual != delta.base_hash {
		return Err(CodePushError::DeltaBaseMismatch {
			expected: delta.base_hash.clone(),
			actual,
		});
	}
	let mut out = Vec::new();
	for op in &delta.ops {
		match op {
			DeltaOp::Copy { offset, len } => {
				let start = *offset as usize;
				let end = start.saturating_add(*len as usize);
				let slice = base.get(start..end).ok_or_else(|| {
					CodePushError::Transport(format!(
						"delta copy out of bounds: {start}..{end} over {}",
						base.len()
					))
				})?;
				out.extend_from_slice(slice);
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
