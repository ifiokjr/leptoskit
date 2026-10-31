//! What an authenticator app computes, written independently of the crate's
//! own TOTP code, so the tests check it against RFC 6238 rather than itself.

use hmac::Hmac;
use hmac::Mac;

/// What an authenticator app shows for `secret` (base32), `steps` from now.
pub fn totp(secret: &str, steps: i64) -> String {
	let key = base32_decode(secret);
	let step = leptoskit_db::now().timestamp().div_euclid(30) + steps;
	let mut mac = <Hmac<sha1::Sha1> as Mac>::new_from_slice(&key).unwrap();
	mac.update(&step.to_be_bytes());
	let hash = mac.finalize().into_bytes();
	let offset = usize::from(hash[19] & 0x0f);
	let binary = u32::from_be_bytes([
		hash[offset] & 0x7f,
		hash[offset + 1],
		hash[offset + 2],
		hash[offset + 3],
	]);
	format!("{:06}", binary % 1_000_000)
}

pub fn base32_decode(text: &str) -> Vec<u8> {
	let mut bytes = Vec::new();
	let mut buffer = 0_u64;
	let mut bits = 0;

	for character in text.bytes() {
		let value = match character {
			b'A'..=b'Z' => character - b'A',
			b'2'..=b'7' => character - b'2' + 26,
			_ => panic!("not base32"),
		};
		buffer = (buffer << 5) | u64::from(value);
		bits += 5;

		if bits >= 8 {
			bits -= 8;
			bytes.push((buffer >> bits) as u8);
		}
	}

	bytes
}
