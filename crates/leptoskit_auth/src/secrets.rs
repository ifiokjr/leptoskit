//! Random tokens and codes, and the hashes the database keeps instead of
//! them.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::Hmac;
use hmac::Mac;
use sha2::Digest;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::AuthError;

/// The server's keys, newest first: the newest hashes new codes, and any of
/// them checks one, so rotating a key never invalidates codes in flight.
///
/// They come from an environment variable, never from `leptoskit.toml`:
///
/// ```text
/// AUTH_ENCRYPTION_KEYS=2026-10:<32 bytes, base64url>,2026-04:<…>
/// ```
#[derive(Clone)]
pub struct Keys {
	keys: Vec<Key>,
}

#[derive(Clone)]
struct Key {
	id: String,
	secret: [u8; 32],
}

impl Keys {
	/// Reads keys from the environment variable `name`.
	///
	/// # Errors
	///
	/// When the variable is missing, or [`Keys::parse`] refuses it.
	pub fn from_env(name: &str) -> Result<Self, AuthError> {
		let value = std::env::var(name)
			.map_err(|_| AuthError::Config(format!("set {name} to the server's auth keys")))?;
		Self::parse(&value)
			.map_err(|error| AuthError::Config(format!("{name} is invalid: {error}")))
	}

	/// Parses `id:key` pairs, comma-separated, newest first. Each key is 32
	/// random bytes in unpadded base64url, which
	/// `openssl rand 32 | basenc --base64url | tr -d =` prints.
	///
	/// # Errors
	///
	/// When there are no keys, an ID repeats, or a key isn't 32 bytes.
	pub fn parse(text: &str) -> Result<Self, AuthError> {
		let mut keys: Vec<Key> = Vec::new();

		for pair in text
			.split(',')
			.map(str::trim)
			.filter(|pair| !pair.is_empty())
		{
			let Some((id, encoded)) = pair.split_once(':') else {
				return Err(AuthError::Config(
					"each key is `id:key`, such as `2026-10:…`".into(),
				));
			};
			let id = id.trim();
			let secret = URL_SAFE_NO_PAD
				.decode(encoded.trim().trim_end_matches('='))
				.ok()
				.and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
				.ok_or_else(|| {
					AuthError::Config(format!("key `{id}` isn't 32 bytes of base64url"))
				})?;

			if id.is_empty() || keys.iter().any(|key| key.id == id) {
				return Err(AuthError::Config(format!(
					"key IDs must be unique and non-empty; `{id}` isn't"
				)));
			}

			keys.push(Key {
				id: id.to_owned(),
				secret,
			});
		}

		if keys.is_empty() {
			return Err(AuthError::Config("there are no keys".into()));
		}

		Ok(Self { keys })
	}

	/// A single key, for tests and examples.
	pub fn single(id: &str, secret: [u8; 32]) -> Self {
		Self {
			keys: vec![Key {
				id: id.to_owned(),
				secret,
			}],
		}
	}

	/// The keyed hash of `parts` under the newest key. Each part is length
	/// prefixed, so ("ab", "c") and ("a", "bc") hash apart.
	pub(crate) fn hash(&self, parts: &[&[u8]]) -> Vec<u8> {
		mac(&self.keys[0].secret, parts)
	}

	/// Whether `hash` is the keyed hash of `parts` under any key, compared in
	/// constant time.
	pub(crate) fn verify(&self, hash: &[u8], parts: &[&[u8]]) -> bool {
		self.keys.iter().fold(false, |found, key| {
			found | bool::from(mac(&key.secret, parts).ct_eq(hash))
		})
	}
}

impl fmt::Debug for Keys {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let ids: Vec<&str> = self.keys.iter().map(|key| key.id.as_str()).collect();
		f.debug_struct("Keys")
			.field("ids", &ids)
			.finish_non_exhaustive()
	}
}

fn mac(secret: &[u8; 32], parts: &[&[u8]]) -> Vec<u8> {
	let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret).expect("HMAC takes any key length");

	for part in parts {
		mac.update(&(part.len() as u64).to_be_bytes());
		mac.update(part);
	}

	mac.finalize().into_bytes().to_vec()
}

/// A session token: 256 random bits, which only the device keeps. Its
/// `Debug` is redacted so it never reaches a log.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionToken(String);

impl SessionToken {
	pub(crate) fn generate() -> Result<Self, AuthError> {
		Ok(Self(URL_SAFE_NO_PAD.encode(random::<32>()?)))
	}

	/// A token the device sent back, to resolve.
	pub fn new(token: impl Into<String>) -> Self {
		Self(token.into())
	}

	/// The token, to give to the device: in a cookie, or for its secure
	/// store.
	pub fn expose(&self) -> &str {
		&self.0
	}

	/// What the database keeps instead of the token. A plain hash is enough:
	/// with 256 random bits there is nothing to guess.
	pub(crate) fn hash(&self) -> Vec<u8> {
		Sha256::digest(self.0.as_bytes()).to_vec()
	}
}

impl fmt::Debug for SessionToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("SessionToken(…)")
	}
}

/// Letters and digits that can't be mistaken for each other: no 0, O, 1,
/// or I. 32 of them, so each random byte picks one without bias.
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// The length of an emailed code: 40 bits, which five attempts in ten
/// minutes can't guess.
pub(crate) const CODE_LENGTH: usize = 8;

/// A new code, such as `K7QM-R2XD`.
pub(crate) fn generate_code() -> Result<String, AuthError> {
	let bytes = random::<CODE_LENGTH>()?;
	let mut code = String::with_capacity(CODE_LENGTH + 1);

	for (index, byte) in bytes.into_iter().enumerate() {
		if index == CODE_LENGTH / 2 {
			code.push('-');
		}
		code.push(char::from(CODE_ALPHABET[usize::from(byte % 32)]));
	}

	Ok(code)
}

/// A code as typed, in the form it was hashed in: uppercase, without
/// spaces or the dash. `None` when it can't be a code at all.
pub(crate) fn normalize_code(input: &str) -> Option<String> {
	let code: String = input
		.chars()
		.filter(|character| !character.is_whitespace() && *character != '-')
		.map(|character| character.to_ascii_uppercase())
		.collect();
	let valid = code.len() == CODE_LENGTH && code.bytes().all(|byte| CODE_ALPHABET.contains(&byte));

	valid.then_some(code)
}

/// Bytes from the operating system's random number generator.
pub(crate) fn random<const N: usize>() -> Result<[u8; N], AuthError> {
	let mut bytes = [0; N];
	getrandom::fill(&mut bytes)
		.map_err(|error| AuthError::Config(format!("no randomness available: {error}")))?;
	Ok(bytes)
}

/// A plain SHA-256 hash, for rate-limit keys.
pub(crate) fn digest(parts: &[&[u8]]) -> Vec<u8> {
	let mut hasher = Sha256::new();

	for part in parts {
		hasher.update((part.len() as u64).to_be_bytes());
		hasher.update(part);
	}

	hasher.finalize().to_vec()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn keys_parse_newest_first_and_check_with_any() {
		let old = Keys::single("old", [1; 32]);
		let keys = Keys::parse(&format!(
			"new:{}, old:{}=",
			URL_SAFE_NO_PAD.encode([2; 32]),
			URL_SAFE_NO_PAD.encode([1; 32])
		))
		.unwrap();
		let hashed_before_rotation = old.hash(&[b"code"]);

		assert!(keys.verify(&hashed_before_rotation, &[b"code"]));
		assert!(keys.verify(&keys.hash(&[b"code"]), &[b"code"]));
		assert_ne!(keys.hash(&[b"code"]), hashed_before_rotation);
		assert!(!keys.verify(&keys.hash(&[b"code"]), &[b"other"]));
		assert!(!keys.verify(&keys.hash(&[b"ab", b"c"]), &[b"a", b"bc"]));
		assert_eq!(format!("{keys:?}"), r#"Keys { ids: ["new", "old"], .. }"#);
	}

	#[test]
	fn keys_refuse_what_they_cannot_use() {
		let key = URL_SAFE_NO_PAD.encode([1; 32]);

		for (text, message) in [
			("", "there are no keys"),
			("nocolon", "each key is `id:key`"),
			("a:short", "key `a` isn't 32 bytes"),
			(&format!("a:{key},a:{key}") as &str, "`a` isn't"),
			(&format!(":{key}"), "non-empty"),
		] {
			let error = Keys::parse(text).unwrap_err().to_string();
			assert!(error.contains(message), "{text:?}: {error}");
		}

		let missing = Keys::from_env("LEPTOSKIT_AUTH_TEST_KEYS_THAT_ARE_NOT_SET").unwrap_err();
		assert!(missing.to_string().contains("set LEPTOSKIT_AUTH_TEST_KEYS"));
	}

	#[test]
	fn codes_are_readable_and_normalize_as_typed() {
		let code = generate_code().unwrap();
		assert_eq!(code.len(), CODE_LENGTH + 1);
		assert_eq!(code.as_bytes()[4], b'-');

		let normalized = normalize_code(&code).unwrap();
		assert_eq!(
			normalize_code(&normalized.to_lowercase()),
			Some(normalized.clone())
		);
		assert_eq!(normalize_code(" k7qm r2xd "), Some("K7QMR2XD".into()));
		assert_eq!(normalize_code("K7QM-R2X"), None);
		assert_eq!(normalize_code("K7QM-R2X0"), None);
	}

	#[test]
	fn tokens_are_random_and_redacted() {
		let first = SessionToken::generate().unwrap();
		let second = SessionToken::generate().unwrap();

		assert_ne!(first, second);
		assert_eq!(first.expose().len(), 43);
		assert_eq!(first.hash().len(), 32);
		assert_eq!(SessionToken::new(first.expose()).hash(), first.hash());
		assert_eq!(format!("{first:?}"), "SessionToken(…)");
	}
}
