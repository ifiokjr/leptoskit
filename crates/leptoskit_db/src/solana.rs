//! Solana's two everyday values, as scalars: a [`Pubkey`] (an address) and
//! a [`Signature`]. Both are text in base58, Solana's own alphabet, and
//! both parse strictly: the text must decode to exactly the right number
//! of bytes, so an address that exists is one a wallet could hold.

use crate::Scalar;

/// A Solana address: an ed25519 public key, 32 bytes, written in base58
/// as wallets and explorers show it, such as
/// `9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM`.
///
/// ```
/// # use leptoskit_db::Pubkey;
/// let address = Pubkey::parse(" 9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM ").unwrap();
/// assert_eq!(address.to_bytes().len(), 32);
/// assert!(Pubkey::parse("not an address").is_err());
/// ```
#[derive(Scalar, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[db(
	min_len = 32,
	max_len = 44,
	regex = r"^[1-9A-HJ-NP-Za-km-z]+$",
	normalize(trim),
	validate_with = pubkey
)]
pub struct Pubkey(String);

impl Pubkey {
	/// The address of these 32 bytes.
	pub fn from_bytes(bytes: [u8; 32]) -> Self {
		Self(base58::encode(&bytes))
	}

	/// The key's 32 bytes.
	pub fn to_bytes(&self) -> [u8; 32] {
		base58::decode_exact(&self.0).expect("a `Pubkey` only holds 32 bytes in base58")
	}
}

/// An ed25519 signature, 64 bytes, in base58, as Solana writes transaction
/// signatures.
#[derive(Scalar, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[db(
	min_len = 64,
	max_len = 88,
	regex = r"^[1-9A-HJ-NP-Za-km-z]+$",
	normalize(trim),
	validate_with = signature
)]
pub struct Signature(String);

impl Signature {
	/// The signature of these 64 bytes.
	pub fn from_bytes(bytes: [u8; 64]) -> Self {
		Self(base58::encode(&bytes))
	}

	/// The signature's 64 bytes.
	pub fn to_bytes(&self) -> [u8; 64] {
		base58::decode_exact(&self.0).expect("a `Signature` only holds 64 bytes in base58")
	}
}

fn pubkey(value: &str) -> Result<(), &'static str> {
	base58::decode_exact::<32>(value)
		.map(drop)
		.ok_or("a Solana address: 32 bytes in base58")
}

fn signature(value: &str) -> Result<(), &'static str> {
	base58::decode_exact::<64>(value)
		.map(drop)
		.ok_or("a signature: 64 bytes in base58")
}

/// Base58 in Bitcoin's alphabet, which Solana uses: no `0`, `O`, `I`, or
/// `l`, and each leading zero byte written as `1`. Every byte string has
/// exactly one spelling, so decoding is strict without extra checks.
pub mod base58 {
	const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

	/// The base58 spelling of `bytes`.
	pub fn encode(bytes: &[u8]) -> String {
		let zeros = bytes.iter().take_while(|byte| **byte == 0).count();
		// Base-58 digits, least significant first.
		let mut digits: Vec<u8> = Vec::with_capacity(bytes.len() * 138 / 100 + 1);

		for &byte in &bytes[zeros..] {
			let mut carry = u32::from(byte);

			for digit in &mut digits {
				carry += u32::from(*digit) << 8;
				*digit = (carry % 58) as u8;
				carry /= 58;
			}

			while carry > 0 {
				digits.push((carry % 58) as u8);
				carry /= 58;
			}
		}

		let mut text = String::with_capacity(zeros + digits.len());
		text.extend(std::iter::repeat_n('1', zeros));
		text.extend(
			digits
				.iter()
				.rev()
				.map(|digit| char::from(ALPHABET[usize::from(*digit)])),
		);
		text
	}

	/// The bytes `text` spells, or `None` when a character isn't in the
	/// alphabet. Long input is slow to decode (quadratic), so bound it
	/// first; [`decode_exact`] does.
	pub fn decode(text: &str) -> Option<Vec<u8>> {
		let zeros = text
			.bytes()
			.take_while(|character| *character == b'1')
			.count();
		// Bytes, least significant first.
		let mut bytes: Vec<u8> = Vec::with_capacity(text.len());

		for character in text.bytes().skip(zeros) {
			let mut carry = u32::from(value(character)?);

			for byte in &mut bytes {
				carry += u32::from(*byte) * 58;
				*byte = (carry & 0xff) as u8;
				carry >>= 8;
			}

			while carry > 0 {
				bytes.push((carry & 0xff) as u8);
				carry >>= 8;
			}
		}

		let mut decoded = vec![0; zeros];
		decoded.extend(bytes.iter().rev());
		Some(decoded)
	}

	/// The `N` bytes `text` spells, or `None` for anything else: a wrong
	/// character, or more or fewer bytes.
	pub fn decode_exact<const N: usize>(text: &str) -> Option<[u8; N]> {
		// `N` bytes take at most `N` characters of leading `1`s, or about
		// 1.37 characters a byte otherwise.
		if text.is_empty() || text.len() > 2 * N {
			return None;
		}

		decode(text)?.try_into().ok()
	}

	fn value(character: u8) -> Option<u8> {
		ALPHABET
			.iter()
			.position(|candidate| *candidate == character)
			.and_then(|position| u8::try_from(position).ok())
	}
}

// The scalars' checks need `regex`, which `server` turns on.
#[cfg(all(test, feature = "regex"))]
mod tests {
	use super::*;

	#[test]
	fn base58_matches_known_spellings() {
		// Vectors from the Bitcoin base58 test suite.
		for (bytes, text) in [
			(&b""[..], ""),
			(&[0x61][..], "2g"),
			(&[0x62, 0x62, 0x62][..], "a3gV"),
			(&[0x63, 0x63, 0x63][..], "aPEr"),
			(&[0x00, 0x00, 0x00, 0x28, 0x7f, 0xb4, 0xcd][..], "111233QC4"),
			(&b"simply a long string"[..], "2cFupjhnEsSn59qHXstmK2ffpLv2"),
			(&[0x00; 3][..], "111"),
		] {
			assert_eq!(base58::encode(bytes), text);
			assert_eq!(base58::decode(text).unwrap(), bytes);
		}

		assert_eq!(base58::decode("0OIl"), None);
		assert_eq!(base58::decode_exact::<2>(""), None);
		assert_eq!(base58::decode_exact::<1>("2g"), Some([0x61]));
		assert_eq!(base58::decode_exact::<2>("2g"), None);
	}

	#[test]
	fn addresses_are_exactly_32_bytes() {
		// The system program, and a well-known wallet address.
		let system = Pubkey::parse("11111111111111111111111111111111").unwrap();
		assert_eq!(system.to_bytes(), [0; 32]);
		assert_eq!(Pubkey::from_bytes([0; 32]), system);

		let address = Pubkey::parse("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM").unwrap();
		assert_eq!(Pubkey::from_bytes(address.to_bytes()), address);

		// 33 ones is 33 zero bytes; the rest aren't base58 or aren't 32 bytes.
		for invalid in [
			"",
			"111111111111111111111111111111111",
			"9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWW0",
			"9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWMM",
			"zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
		] {
			assert!(Pubkey::parse(invalid).is_err(), "{invalid:?} parsed");
		}

		let error = Pubkey::parse("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").unwrap_err();
		assert_eq!(
			error.to_string(),
			"`Pubkey` must be a Solana address: 32 bytes in base58"
		);
	}

	#[test]
	fn signatures_are_exactly_64_bytes() {
		let bytes: [u8; 64] = std::array::from_fn(|index| u8::try_from(index).unwrap());
		let signature = Signature::from_bytes(bytes);
		assert_eq!(signature.to_bytes(), bytes);
		assert_eq!(
			Signature::parse(signature.as_inner().clone()).unwrap(),
			signature
		);
		assert!(Signature::parse("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM").is_err());
		let json = serde_json::to_string(&signature).unwrap();
		assert_eq!(serde_json::from_str::<Signature>(&json).unwrap(), signature);
		assert!(serde_json::from_str::<Signature>("\"2g\"").is_err());
	}
}
