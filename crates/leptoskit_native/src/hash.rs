//! A fast hash for the renderer's own maps.

use std::hash::BuildHasherDefault;
use std::hash::Hasher;

/// Hashes maps keyed by the app's own class strings and node ids.
pub(crate) type FastHash = BuildHasherDefault<FastHasher>;

/// A word-at-a-time hash, after rustc's `FxHash`.
///
/// Keys come from app code, so resistance to crafted collisions buys nothing.
/// Inside the native guest's interpreter, the default `SipHash` cost two thirds
/// of a cached class parse.
#[derive(Default)]
pub(crate) struct FastHasher(u64);

impl FastHasher {
	const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

	fn add(&mut self, word: u64) {
		self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(Self::SEED);
	}
}

impl Hasher for FastHasher {
	fn finish(&self) -> u64 {
		self.0
	}

	fn write(&mut self, bytes: &[u8]) {
		let mut words = bytes.chunks_exact(8);

		for word in &mut words {
			self.add(u64::from_le_bytes(word.try_into().expect("eight bytes")));
		}

		let rest = words.remainder();

		if !rest.is_empty() {
			// The tail has at most seven bytes, so its length fits in the
			// last one, keeping `"a"` apart from `"a\0"`.
			let mut word = [0; 8];
			word[..rest.len()].copy_from_slice(rest);
			word[7] = rest.len() as u8;
			self.add(u64::from_le_bytes(word));
		}
	}

	fn write_u8(&mut self, byte: u8) {
		self.add(u64::from(byte));
	}

	fn write_u64(&mut self, word: u64) {
		self.add(word);
	}

	fn write_i64(&mut self, word: i64) {
		self.add(u64::from_ne_bytes(word.to_ne_bytes()));
	}
}

#[cfg(test)]
mod tests {
	use std::hash::BuildHasher;

	use super::FastHash;

	#[test]
	fn equal_keys_hash_equally_and_near_keys_apart() {
		let hash = |value: &str| FastHash::default().hash_one(value);

		assert_eq!(hash("flex-1 items-center"), hash("flex-1 items-center"));
		// Tails shorter than a word, and strings a zero byte apart.
		for (a, b) in [
			("p-4", "p-2"),
			("text-[10px]", "text-[11px]"),
			("a", "a\0"),
			("", " "),
		] {
			assert_ne!(hash(a), hash(b), "{a:?} and {b:?}");
		}
		let ids: std::collections::HashSet<u64> = (0..10_000_i64)
			.map(|id| FastHash::default().hash_one(id))
			.collect();
		assert_eq!(ids.len(), 10_000, "node ids never collide");
	}
}
