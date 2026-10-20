//! Portable cryptographic digests for native, browser, and interpreter guests.
//!
//! Direct Rust calls borrow input and return fixed-size digest arrays. Native
//! hosts can also register [`Crypto`] on the binary bridge. No text encoding or
//! base64 is involved. This is hashing, not encryption or secure key storage.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use sha2::Digest;
use sha2::Sha256;
use sha2::Sha512;

/// SHA-256: raw input bytes -> exactly 32 digest bytes.
pub const SHA256: MethodId = MethodId(0);
/// SHA-512: raw input bytes -> exactly 64 digest bytes.
pub const SHA512: MethodId = MethodId(1);

/// Computes SHA-256 without allocating an output buffer.
pub fn sha256(input: &[u8]) -> [u8; 32] {
	Sha256::digest(input).into()
}

/// Computes SHA-512 without allocating an output buffer.
pub fn sha512(input: &[u8]) -> [u8; 64] {
	Sha512::digest(input).into()
}

/// Binary bridge adapter for the same portable implementation.
#[derive(Debug, Default)]
pub struct Crypto;

impl Plugin for Crypto {
	fn name(&self) -> &'static str {
		"crypto"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[SHA256, SHA512]
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		match method {
			SHA256 => Ok(Bytes::copy_from_slice(&sha256(&payload))),
			SHA512 => Ok(Bytes::copy_from_slice(&sha512(&payload))),
			_ => Err(Error::Unavailable),
		}
	}
}

#[cfg(test)]
mod tests {
	use std::fmt::Write as _;

	use super::*;

	#[test]
	fn known_sha256_vector_and_bridge_agree() {
		let expected = [
			0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
			0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
			0xf2, 0x00, 0x15, 0xad,
		];
		assert_eq!(sha256(b"abc"), expected);
		assert_eq!(
			Crypto
				.invoke(SHA256, Bytes::from_static(b"abc"))
				.unwrap()
				.as_ref(),
			expected
		);
		assert_eq!(
			Crypto.invoke(MethodId(100), Bytes::new()),
			Err(Error::Unavailable)
		);
	}

	#[test]
	fn sha512_empty_input_matches_published_vector() {
		let digest = sha512(b"");
		let mut hex = String::with_capacity(128);
		for byte in digest {
			write!(&mut hex, "{byte:02x}").unwrap();
		}
		assert_eq!(
			hex,
			concat!(
				"cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce",
				"47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
			)
		);
	}
}
