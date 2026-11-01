//! Time-based one-time passwords (RFC 6238), the codes authenticator apps
//! show: HMAC-SHA-1 over 30-second steps, six digits, the parameters every
//! app supports.

use std::fmt;
use std::fmt::Write as _;

use hmac::Hmac;
use hmac::Mac;
use sha1::Sha1;
use subtle::ConstantTimeEq;

/// How long each code lasts.
pub(crate) const STEP_SECONDS: i64 = 30;

/// How many digits a code has.
const DIGITS: u32 = 6;

/// How many steps either side of now a code may come from, for a phone
/// whose clock is a little off.
const SKEW: i64 = 1;

/// The length of a secret, in bytes: 160 bits, as RFC 4226 recommends for
/// HMAC-SHA-1.
pub(crate) const SECRET_LENGTH: usize = 20;

/// A TOTP secret. Its `Debug` is redacted so it never reaches a log.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Secret(pub(crate) Vec<u8>);

impl fmt::Debug for Secret {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("Secret(…)")
	}
}

impl Secret {
	/// The secret as authenticator apps take it: unpadded base32.
	pub(crate) fn base32(&self) -> String {
		base32(&self.0)
	}

	/// The step that contains `unix_seconds`.
	pub(crate) fn step(unix_seconds: i64) -> i64 {
		unix_seconds.div_euclid(STEP_SECONDS)
	}

	/// The step whose code `input` is, among the steps around `now`, if it
	/// is one of them and comes after `after`, which stops a code being
	/// used twice. Every candidate is computed and compared in constant
	/// time, so the answer takes as long whichever step matched, if any.
	pub(crate) fn verify(&self, input: &str, now: i64, after: Option<i64>) -> Option<i64> {
		let code = normalize(input)?;
		let mut matched = None;

		for step in now - SKEW..=now + SKEW {
			let candidate = format!(
				"{:0width$}",
				hotp(&self.0, step, DIGITS),
				width = DIGITS as usize
			);
			let equal = bool::from(candidate.as_bytes().ct_eq(code.as_bytes()));

			if equal && after.is_none_or(|after| step > after) {
				matched = Some(step);
			}
		}

		matched
	}

	/// The `otpauth://` link an authenticator app scans as a QR code, or
	/// opens when tapped: the app's name as the issuer, and the account's
	/// label.
	pub(crate) fn uri(&self, issuer: &str, account: &str) -> String {
		let issuer = percent_encode(issuer);
		format!(
			"otpauth://totp/{issuer}:{}?secret={}&issuer={issuer}&algorithm=SHA1&digits={DIGITS}&\
			 period={STEP_SECONDS}",
			percent_encode(account),
			self.base32(),
		)
	}
}

/// A code as typed: digits only, without spaces or a dash. `None` when it
/// can't be a code at all.
pub(crate) fn normalize(input: &str) -> Option<String> {
	let code: String = input
		.chars()
		.filter(|character| !character.is_whitespace() && *character != '-')
		.collect();
	let valid = code.len() == DIGITS as usize && code.bytes().all(|byte| byte.is_ascii_digit());

	valid.then_some(code)
}

/// RFC 4226's HOTP: the HMAC-SHA-1 of the counter, dynamically truncated
/// to `digits` decimal digits.
pub(crate) fn hotp(secret: &[u8], counter: i64, digits: u32) -> u32 {
	let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(secret).expect("HMAC takes any key length");
	mac.update(&counter.to_be_bytes());
	let hash = mac.finalize().into_bytes();
	let offset = usize::from(hash[hash.len() - 1] & 0x0f);
	let binary = u32::from_be_bytes([
		hash[offset] & 0x7f,
		hash[offset + 1],
		hash[offset + 2],
		hash[offset + 3],
	]);

	binary % 10_u32.pow(digits)
}

/// RFC 4648 base32, upper case, without padding: what authenticator apps
/// expect a secret in.
pub(crate) fn base32(bytes: &[u8]) -> String {
	const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
	let mut encoded = String::with_capacity(bytes.len().div_ceil(5) * 8);
	let mut buffer: u64 = 0;
	let mut bits = 0;

	for byte in bytes {
		buffer = (buffer << 8) | u64::from(*byte);
		bits += 8;

		while bits >= 5 {
			bits -= 5;
			encoded.push(char::from(ALPHABET[((buffer >> bits) & 0x1f) as usize]));
		}
	}

	if bits > 0 {
		encoded.push(char::from(
			ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize],
		));
	}

	encoded
}

/// Percent-encodes everything but unreserved characters, for the label and
/// issuer in an `otpauth://` link.
fn percent_encode(text: &str) -> String {
	let mut encoded = String::with_capacity(text.len());

	for byte in text.bytes() {
		if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
			encoded.push(char::from(byte));
		} else {
			let _ = write!(encoded, "%{byte:02X}");
		}
	}

	encoded
}

#[cfg(test)]
mod tests {
	use super::*;

	/// RFC 4226 and RFC 6238's SHA-1 test secret.
	const RFC_SECRET: &[u8] = b"12345678901234567890";

	#[test]
	fn hotp_matches_rfc_4226() {
		// Appendix D.
		let expected = [
			755_224, 287_082, 359_152, 969_429, 338_314, 254_676, 287_922, 162_583, 399_871,
			520_489,
		];

		for (counter, code) in expected.into_iter().enumerate() {
			assert_eq!(
				hotp(RFC_SECRET, counter as i64, 6),
				code,
				"counter {counter}"
			);
		}
	}

	#[test]
	fn totp_matches_rfc_6238() {
		// Appendix B, SHA-1, eight digits.
		for (time, code) in [
			(59, 94_287_082),
			(1_111_111_109, 7_081_804),
			(1_111_111_111, 14_050_471),
			(1_234_567_890, 89_005_924),
			(2_000_000_000, 69_279_037),
			(20_000_000_000, 65_353_130),
		] {
			assert_eq!(hotp(RFC_SECRET, Secret::step(time), 8), code, "time {time}");
		}
	}

	#[test]
	fn a_code_matches_its_step_and_its_neighbours_once() {
		let secret = Secret(RFC_SECRET.to_vec());
		let now = Secret::step(59);
		// Six digits of the eight in RFC 6238's first vector.
		assert_eq!(secret.verify("287082", now, None), Some(now));
		assert_eq!(secret.verify("287 082", now, None), Some(now));
		// Used once, it's refused at the same step.
		assert_eq!(secret.verify("287082", now, Some(now)), None);
		// One step later it's still accepted, for a slow phone.
		assert_eq!(secret.verify("287082", now + 1, None), Some(now));
		assert_eq!(secret.verify("287082", now + 2, None), None);
		assert_eq!(secret.verify("287083", now, None), None);

		assert_eq!(secret.verify("28708", now, None), None);
		assert_eq!(secret.verify("abcdef", now, None), None);
	}

	#[test]
	fn secrets_encode_as_rfc_4648_base32() {
		// RFC 4648's test vectors, without padding.
		for (input, output) in [
			("", ""),
			("f", "MY"),
			("fo", "MZXQ"),
			("foo", "MZXW6"),
			("foob", "MZXW6YQ"),
			("fooba", "MZXW6YTB"),
			("foobar", "MZXW6YTBOI"),
		] {
			assert_eq!(base32(input.as_bytes()), output);
		}

		assert_eq!(
			Secret(RFC_SECRET.to_vec()).base32(),
			"GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
		);
	}

	#[test]
	fn links_name_the_app_and_the_account() {
		let secret = Secret(RFC_SECRET.to_vec());
		assert_eq!(
			secret.uri("My Notes", "ada@example.com"),
			"otpauth://totp/My%20Notes:ada%40example.com?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&\
			 issuer=My%20Notes&algorithm=SHA1&digits=6&period=30"
		);
		assert_eq!(format!("{secret:?}"), "Secret(…)");
	}
}
