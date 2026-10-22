//! Digests, secure randomness and UUID v4 for native, browser and guest apps.
//!
//! Direct Rust calls borrow input and return fixed-size digest arrays. Native
//! hosts can also register [`Crypto`] on the binary bridge. No text encoding or
//! base64 is involved. Native randomness uses the operating system CSPRNG;
//! browser builds enable `web` to use `Crypto.getRandomValues`. Interpreter
//! guests enable `guest` and request randomness from the native host.
//! Encryption and secure key storage are outside this crate's API.

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
/// Secure random bytes: byte count u32 LE -> exactly that many raw bytes.
pub const RANDOM_BYTES: MethodId = MethodId(2);
/// UUID v4: empty input -> 16 bytes in RFC 9562 network order.
pub const UUID_V4: MethodId = MethodId(3);
/// Maximum random bytes per call, matching the browser Web Crypto quota.
pub const MAX_RANDOM_BYTES: usize = 65_536;

/// A validated UUID with version 4 and RFC 9562 variant bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UuidV4([u8; 16]);

impl UuidV4 {
	/// Validates the version and variant of a 16-byte UUID.
	pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, Error> {
		if bytes[6] >> 4 != 4 || bytes[8] >> 6 != 2 {
			return Err(Error::InvalidArgument);
		}
		Ok(Self(bytes))
	}

	/// Borrows the canonical 16-byte representation, without allocating.
	pub fn as_bytes(&self) -> &[u8; 16] {
		&self.0
	}

	/// Returns the canonical 16-byte representation.
	pub fn into_bytes(self) -> [u8; 16] {
		self.0
	}

	/// Decodes a bridge reply, rejecting wrong lengths, versions and variants.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		Self::from_bytes(bytes.try_into().map_err(|_| Error::InvalidArgument)?)
	}
}

impl std::fmt::Display for UuidV4 {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		for (index, byte) in self.0.iter().enumerate() {
			if matches!(index, 4 | 6 | 8 | 10) {
				formatter.write_str("-")?;
			}
			write!(formatter, "{byte:02x}")?;
		}
		Ok(())
	}
}

/// Fills an existing buffer from the platform CSPRNG. Requests above
/// [`MAX_RANDOM_BYTES`] fail before calling the platform. An error means the
/// buffer must not be used as random data; it may have been partially filled.
/// No insecure fallback is used if platform entropy is unavailable.
pub fn fill_random(bytes: &mut [u8]) -> Result<(), Error> {
	validate_random_length(bytes.len())?;
	#[cfg(any(
		not(target_arch = "wasm32"),
		all(target_arch = "wasm32", target_os = "unknown", feature = "web")
	))]
	{
		getrandom::fill(bytes).map_err(|_| Error::Native)
	}
	#[cfg(not(any(
		not(target_arch = "wasm32"),
		all(target_arch = "wasm32", target_os = "unknown", feature = "web")
	)))]
	{
		Err(Error::Unavailable)
	}
}

/// Allocates a bounded buffer filled from the platform CSPRNG.
pub fn random_bytes(length: usize) -> Result<Bytes, Error> {
	random_bytes_with(length, fill_random)
}

fn random_bytes_with(
	length: usize,
	fill: impl FnOnce(&mut [u8]) -> Result<(), Error>,
) -> Result<Bytes, Error> {
	validate_random_length(length)?;
	let mut bytes = vec![0; length];
	fill(&mut bytes)?;
	Ok(bytes.into())
}

/// Generates a UUID v4 from 122 random bits and the required version/variant.
pub fn uuid_v4() -> Result<UuidV4, Error> {
	let mut bytes = [0; 16];
	fill_random(&mut bytes)?;
	bytes[6] = (bytes[6] & 0x0f) | 0x40;
	bytes[8] = (bytes[8] & 0x3f) | 0x80;
	Ok(UuidV4(bytes))
}

fn validate_random_length(length: usize) -> Result<(), Error> {
	if length > MAX_RANDOM_BYTES {
		Err(Error::TooLarge)
	} else {
		Ok(())
	}
}

#[cfg(any(test, all(target_arch = "wasm32", feature = "guest")))]
fn decode_random_reply(length: usize, reply: Bytes) -> Result<Bytes, Error> {
	validate_random_length(length)?;
	if reply.len() != length {
		return Err(Error::InvalidArgument);
	}
	Ok(reply)
}

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
		&[SHA256, SHA512, RANDOM_BYTES, UUID_V4]
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		match method {
			SHA256 => Ok(Bytes::copy_from_slice(&sha256(&payload))),
			SHA512 => Ok(Bytes::copy_from_slice(&sha512(&payload))),
			RANDOM_BYTES => {
				let length = u32::from_le_bytes(
					payload
						.as_ref()
						.try_into()
						.map_err(|_| Error::InvalidArgument)?,
				);
				random_bytes(length as usize)
			}
			UUID_V4 => {
				if !payload.is_empty() {
					return Err(Error::InvalidArgument);
				}
				Ok(Bytes::copy_from_slice(uuid_v4()?.as_bytes()))
			}
			_ => Err(Error::Unavailable),
		}
	}
}

/// Typed cached client for native interpreter guests. Hashing can alternatively
/// use the direct Rust functions when avoiding a bridge round trip is
/// preferable.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use leptoskit_bridge::guest::Client;

	use super::*;

	/// Native crypto operations through the binary plugin bridge.
	#[derive(Debug, Clone, Copy)]
	pub struct Crypto(Client);

	impl Crypto {
		/// Resolves the installed crypto plugin once.
		pub fn new() -> Result<Self, Error> {
			Client::new("crypto").map(Self)
		}

		/// Computes SHA-256 natively and validates the fixed-width response.
		pub async fn sha256(self, input: &[u8]) -> Result<[u8; 32], Error> {
			self.0
				.call(SHA256, input)?
				.await?
				.as_ref()
				.try_into()
				.map_err(|_| Error::InvalidArgument)
		}

		/// Computes SHA-512 natively and validates the fixed-width response.
		pub async fn sha512(self, input: &[u8]) -> Result<[u8; 64], Error> {
			self.0
				.call(SHA512, input)?
				.await?
				.as_ref()
				.try_into()
				.map_err(|_| Error::InvalidArgument)
		}

		/// Requests bounded random bytes; truncated or oversized replies fail.
		pub async fn random_bytes(self, length: usize) -> Result<Bytes, Error> {
			validate_random_length(length)?;
			let count = u32::try_from(length).map_err(|_| Error::TooLarge)?;
			decode_random_reply(
				length,
				self.0.call(RANDOM_BYTES, &count.to_le_bytes())?.await?,
			)
		}

		/// Generates a UUID natively and validates its version and variant.
		pub async fn uuid_v4(self) -> Result<UuidV4, Error> {
			UuidV4::decode(&self.0.call(UUID_V4, &[])?.await?)
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod random_tests {
	use super::*;

	#[test]
	fn secure_random_requests_preserve_length_and_validate_wire() {
		for length in [0, 1, 32, MAX_RANDOM_BYTES] {
			let response = Crypto
				.invoke(
					RANDOM_BYTES,
					Bytes::copy_from_slice(&(length as u32).to_le_bytes()),
				)
				.unwrap();
			assert_eq!(response.len(), length);
			assert_eq!(
				decode_random_reply(length, response.clone()).unwrap(),
				response
			);
		}
		assert_eq!(random_bytes(MAX_RANDOM_BYTES + 1), Err(Error::TooLarge));
		assert_eq!(
			Crypto.invoke(
				RANDOM_BYTES,
				Bytes::copy_from_slice(&u32::MAX.to_le_bytes())
			),
			Err(Error::TooLarge)
		);
		for length in [0, 1, 3, 5, 8] {
			assert_eq!(
				Crypto.invoke(RANDOM_BYTES, vec![0; length].into()),
				Err(Error::InvalidArgument)
			);
		}
		assert_eq!(
			decode_random_reply(3, Bytes::from_static(&[1, 2])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			decode_random_reply(1, Bytes::from_static(&[1, 2])),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn uuid_has_rfc_version_and_variant_bits_and_canonical_format() {
		let expected = [
			0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x46, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
			0xee, 0xff,
		];
		let uuid = UuidV4::from_bytes(expected).unwrap();
		assert_eq!(uuid.to_string(), "00112233-4455-4677-8899-aabbccddeeff");
		assert_eq!(uuid.into_bytes(), expected);
		for _ in 0..32 {
			let uuid = uuid_v4().unwrap();
			assert_eq!(uuid.as_bytes()[6] & 0xf0, 0x40);
			assert_eq!(uuid.as_bytes()[8] & 0xc0, 0x80);
			assert_eq!(UuidV4::decode(uuid.as_bytes()).unwrap(), uuid);
			assert_eq!(uuid.to_string().len(), 36);
		}
		let wire = Crypto.invoke(UUID_V4, Bytes::new()).unwrap();
		UuidV4::decode(&wire).unwrap();
		assert_eq!(
			Crypto.invoke(UUID_V4, Bytes::from_static(&[0])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(UuidV4::decode(&expected[..15]), Err(Error::InvalidArgument));
		let mut invalid = expected;
		invalid[6] = 0x10;
		assert_eq!(UuidV4::from_bytes(invalid), Err(Error::InvalidArgument));
		invalid = expected;
		invalid[8] = 0xc0;
		assert_eq!(UuidV4::from_bytes(invalid), Err(Error::InvalidArgument));
	}

	#[test]
	fn failed_entropy_source_never_returns_partial_output_or_falls_back() {
		assert_eq!(
			random_bytes_with(16, |bytes| {
				bytes.fill(42);
				Err(Error::Native)
			}),
			Err(Error::Native)
		);
		assert_eq!(
			random_bytes_with(MAX_RANDOM_BYTES + 1, |_| {
				panic!("oversized request must not reach entropy source")
			}),
			Err(Error::TooLarge)
		);
		let mut too_large = vec![17; MAX_RANDOM_BYTES + 1];
		assert_eq!(fill_random(&mut too_large), Err(Error::TooLarge));
		assert!(too_large.iter().all(|byte| *byte == 17));
	}
}
