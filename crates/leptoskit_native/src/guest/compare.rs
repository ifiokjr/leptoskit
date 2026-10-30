//! Memory comparison eight bytes at a time for WebAssembly guests.
//!
//! WebAssembly has instructions to copy and fill memory but none to compare
//! it, so Rust's `compiler_builtins` compares byte by byte. Inside the
//! interpreter that costs about ten instructions per byte: checking a 64 KiB
//! plugin reply took half a millisecond. Its `memcmp` and `bcmp` are weak
//! symbols so a program can supply its own; these compare a word at a time
//! and look at single bytes only to find the first difference.
//!
//! The symbols are exported only for `wasm32-unknown-unknown`, which has no
//! libc of its own; WASI and native targets keep their C library's.

#![allow(unsafe_code)]

/// Compares `len` bytes, returning the difference of the first unequal pair
/// as `memcmp` does: negative, zero or positive.
///
/// # Safety
/// Both pointers must be valid for reads of `len` bytes.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, len: usize) -> i32 {
	// SAFETY: The caller guarantees both ranges are readable.
	unsafe { compare(left, right, len) }
}

/// Compares `len` bytes for equality: zero when equal, nonzero otherwise.
///
/// # Safety
/// Both pointers must be valid for reads of `len` bytes.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bcmp(left: *const u8, right: *const u8, len: usize) -> i32 {
	// SAFETY: The caller guarantees both ranges are readable.
	unsafe { compare(left, right, len) }
}

/// Skips equal eight-byte words, then compares the remaining bytes singly.
///
/// # Safety
/// Both pointers must be valid for reads of `len` bytes.
pub(crate) unsafe fn compare(left: *const u8, right: *const u8, len: usize) -> i32 {
	const WORD: usize = size_of::<u64>();
	let mut offset = 0;

	while len - offset >= WORD {
		// SAFETY: `offset + WORD <= len`, so both words are in range; reads
		// are unaligned because the slices may start anywhere.
		let (a, b) = unsafe {
			(
				left.add(offset).cast::<u64>().read_unaligned(),
				right.add(offset).cast::<u64>().read_unaligned(),
			)
		};

		if a != b {
			break;
		}

		offset += WORD;
	}

	while offset < len {
		// SAFETY: `offset < len`.
		let (a, b) = unsafe { (*left.add(offset), *right.add(offset)) };

		if a != b {
			return i32::from(a) - i32::from(b);
		}

		offset += 1;
	}

	0
}

#[cfg(test)]
mod tests {
	use super::compare;

	fn sign(value: i32) -> i32 {
		value.signum()
	}

	#[test]
	fn agrees_with_byte_comparison_at_every_length_and_offset() {
		let left: Vec<u8> = (0..64_u32).map(|i| (i * 37 % 251) as u8).collect();

		for len in 0..40 {
			for start in 0..8 {
				let a = &left[start..start + len];
				// SAFETY: Both slices hold `len` bytes.
				assert_eq!(unsafe { compare(a.as_ptr(), a.as_ptr(), len) }, 0);

				for changed in 0..len {
					for delta in [1_u8, 200] {
						let mut b = a.to_vec();
						b[changed] = b[changed].wrapping_add(delta);
						let expected = sign(i32::from(a[changed]) - i32::from(b[changed]));
						// SAFETY: Both slices hold `len` bytes.
						let found = sign(unsafe { compare(a.as_ptr(), b.as_ptr(), len) });
						assert_eq!(found, expected, "len {len} start {start} byte {changed}");
					}
				}
			}
		}
	}
}
