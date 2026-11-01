//! Random tokens and secrets, the hashes the database keeps instead of
//! them, PKCE, and pairwise subjects.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use leptoskit_db::Id;
use sha2::Digest;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::IdpError;
use crate::Result;

/// A client secret, shown once when the app is registered. Its `Debug` is
/// redacted so it never reaches a log.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientSecret(String);

impl ClientSecret {
	pub(crate) fn generate() -> Result<Self> {
		Ok(Self(token()?))
	}

	/// The secret, to hand to the app's operator once.
	pub fn expose(&self) -> &str {
		&self.0
	}
}

impl fmt::Debug for ClientSecret {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("ClientSecret(…)")
	}
}

/// Bytes from the operating system's random number generator.
pub(crate) fn random<const N: usize>() -> Result<[u8; N]> {
	let mut bytes = [0; N];
	getrandom::fill(&mut bytes)
		.map_err(|error| IdpError::Config(format!("no randomness available: {error}")))?;
	Ok(bytes)
}

/// 256 random bits in base64url: an authorization code, a refresh token, a
/// consent handle, or a client secret.
pub(crate) fn token() -> Result<String> {
	Ok(URL_SAFE_NO_PAD.encode(random::<32>()?))
}

/// A short random identifier, such as a client ID or a key ID.
pub(crate) fn identifier() -> Result<String> {
	Ok(URL_SAFE_NO_PAD.encode(random::<16>()?))
}

/// What the database keeps instead of a token. A plain hash is enough: with
/// 256 random bits there's nothing to guess.
pub(crate) fn hash(token: &str) -> Vec<u8> {
	Sha256::digest(token.as_bytes()).to_vec()
}

/// A SHA-256 over several parts, each prefixed with its length so no two
/// lists of parts hash alike.
pub(crate) fn digest(parts: &[&[u8]]) -> Vec<u8> {
	let mut hasher = Sha256::new();

	for part in parts {
		hasher.update((part.len() as u64).to_be_bytes());
		hasher.update(part);
	}

	hasher.finalize().to_vec()
}

/// The letters a device code is made of: consonants only, so no code spells
/// a word, and none that look alike (RFC 8628 §6.1).
const USER_CODE_ALPHABET: &[u8; 20] = b"BCDFGHJKLMNPQRSTVWXZ";

/// How many letters a user code has: 20⁸, about 2³⁴, of which guesses are
/// rate limited.
const USER_CODE_LENGTH: usize = 8;

/// A code for someone to type on another device, such as `WDJB-MJHT`.
pub(crate) fn user_code() -> Result<String> {
	let mut letters = Vec::with_capacity(USER_CODE_LENGTH);

	while letters.len() < USER_CODE_LENGTH {
		for byte in random::<16>()? {
			// 240 is the largest multiple of 20 a byte holds: past it, the
			// letters wouldn't be equally likely.
			if byte < 240 && letters.len() < USER_CODE_LENGTH {
				letters.push(USER_CODE_ALPHABET[usize::from(byte % 20)]);
			}
		}
	}

	let (first, second) = letters.split_at(USER_CODE_LENGTH / 2);
	Ok(format!(
		"{}-{}",
		String::from_utf8_lossy(first),
		String::from_utf8_lossy(second)
	))
}

/// A typed user code as it's stored: upper case, with spaces and dashes
/// dropped. `None` when it can't be one.
pub(crate) fn normalize_user_code(typed: &str) -> Option<String> {
	let code: String = typed
		.chars()
		.filter(|c| !matches!(c, '-' | ' ' | '\t'))
		.map(|c| c.to_ascii_uppercase())
		.collect();

	(code.len() == USER_CODE_LENGTH && code.bytes().all(|byte| USER_CODE_ALPHABET.contains(&byte)))
		.then_some(code)
}

/// Whether `token` hashes to `expected`, compared in constant time.
pub(crate) fn matches(token: &str, expected: &[u8]) -> bool {
	bool::from(hash(token).ct_eq(expected))
}

/// Whether `value` can be a PKCE code verifier or `S256` challenge: 43 to
/// 128 unreserved characters (RFC 7636 §4.1).
pub(crate) fn is_pkce_value(value: &str) -> bool {
	(43..=128).contains(&value.len())
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
}

/// Whether `verifier` answers the `S256` `challenge`, compared in constant
/// time.
pub(crate) fn pkce_matches(verifier: &str, challenge: &str) -> bool {
	if !is_pkce_value(verifier) {
		return false;
	}

	let computed = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
	bool::from(computed.as_bytes().ct_eq(challenge.as_bytes()))
}

/// The `sub` one app sees for one account: stable for as long as the app is
/// registered, and unrelated to what any other app sees.
pub(crate) fn pairwise_subject<T>(salt: &[u8], account: Id<T>) -> String {
	let mut hasher = Sha256::new();
	hasher.update(b"leptoskit_idp pairwise subject");
	hasher.update((salt.len() as u64).to_be_bytes());
	hasher.update(salt);
	hasher.update(account.uuid().as_bytes());
	URL_SAFE_NO_PAD.encode(hasher.finalize())
}

/// Base64url without padding, as JOSE and PKCE use it.
pub(crate) fn base64url(bytes: &[u8]) -> String {
	URL_SAFE_NO_PAD.encode(bytes)
}

/// Decodes base64url, with or without padding.
pub(crate) fn from_base64url(text: &str) -> Option<Vec<u8>> {
	URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).ok()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn tokens_are_random_and_hashed() {
		let first = token().unwrap();
		assert_ne!(first, token().unwrap());
		assert_eq!(first.len(), 43);
		assert!(matches(&first, &hash(&first)));
		assert!(!matches(&first, &hash("other")));
		assert_eq!(identifier().unwrap().len(), 22);

		let secret = ClientSecret::generate().unwrap();
		assert_eq!(format!("{secret:?}"), "ClientSecret(…)");
		assert_eq!(secret.expose().len(), 43);
	}

	#[test]
	fn user_codes_are_easy_to_type_and_compare() {
		let code = user_code().unwrap();
		assert_eq!(code.len(), 9);
		assert_eq!(&code[4..5], "-");
		assert_ne!(code, user_code().unwrap());
		assert_eq!(
			normalize_user_code(&code.to_lowercase()).unwrap(),
			code.replace('-', "")
		);
		assert_eq!(
			normalize_user_code("wdjb mjht").as_deref(),
			Some("WDJBMJHT")
		);
		assert_eq!(normalize_user_code("WDJB-MJH"), None);
		assert_eq!(normalize_user_code("AEIO-UAEI"), None);

		assert_ne!(digest(&[b"ab", b"c"]), digest(&[b"a", b"bc"]));
	}

	#[test]
	fn pkce_follows_rfc_7636() {
		// The example from RFC 7636, appendix B.
		let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
		let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

		assert!(is_pkce_value(verifier));
		assert!(pkce_matches(verifier, challenge));
		assert!(!pkce_matches(challenge, challenge));
		assert!(!pkce_matches("short", challenge));
		assert!(!is_pkce_value(&"a".repeat(129)));
		assert!(!is_pkce_value(&format!("{}+", "a".repeat(43))));
	}

	#[test]
	fn pairwise_subjects_differ_per_app_and_stay_put() {
		let account: Id<()> = Id::new();
		let one = pairwise_subject(b"app one", account);

		assert_eq!(one, pairwise_subject(b"app one", account));
		assert_ne!(one, pairwise_subject(b"app two", account));
		assert_ne!(one, pairwise_subject(b"app one", Id::<()>::new()));
		assert_eq!(from_base64url(&base64url(b"xyz")).unwrap(), b"xyz");
		assert_eq!(from_base64url("eHl6=").unwrap(), b"xyz");
	}
}
