//! Random tokens and codes, and the hashes the database keeps instead of
//! them.

use std::fmt;

use aes_gcm_siv::Aes256GcmSiv;
use aes_gcm_siv::KeyInit;
use aes_gcm_siv::Nonce;
use aes_gcm_siv::aead::Aead;
use aes_gcm_siv::aead::Payload;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::Hmac;
use hmac::Mac;
use sha2::Digest;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::AuthError;

/// The server's keys, newest first: the newest hashes new codes and
/// encrypts new secrets, and any of them checks or decrypts one, so rotating
/// a key never invalidates anything in flight.
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
	/// When there are no keys, an ID repeats or is longer than 64 bytes, or
	/// a key isn't 32 bytes.
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

			if id.is_empty() || id.len() > MAX_KEY_ID || keys.iter().any(|key| key.id == id) {
				return Err(AuthError::Config(format!(
					"key IDs must be unique, non-empty, and at most {MAX_KEY_ID} bytes; `{id}` \
					 isn't"
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

	/// 32 bytes derived from the key named `id` for one `purpose`, such as
	/// a signing key that another crate keeps only the public half of. Each
	/// purpose gets bytes unrelated to every other's, and to the hashes this
	/// crate keeps. `None` when no key has that ID, such as after it was
	/// retired.
	pub fn derive(&self, id: &str, purpose: &[u8]) -> Option<[u8; 32]> {
		let key = self.keys.iter().find(|key| key.id == id)?;
		let bytes = mac(&key.secret, &[b"leptoskit derived key", purpose]);
		<[u8; 32]>::try_from(bytes).ok()
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

	/// Encrypts `plaintext` under the newest key with AES-256-GCM-SIV, bound
	/// to `context`: decrypting needs the same context, so a value copied to
	/// another row or purpose doesn't open. The result names the key, so
	/// any listed key can open it after a rotation.
	///
	/// Layout: a format byte (`1`), the key ID's length and bytes, a 96-bit
	/// random nonce, then the ciphertext and its tag.
	pub(crate) fn seal(&self, context: &[&[u8]], plaintext: &[u8]) -> Result<Vec<u8>, AuthError> {
		let key = &self.keys[0];
		let nonce = random::<NONCE_LENGTH>()?;
		let aad = associated_data(context);
		let ciphertext = cipher(&key.secret)
			.encrypt(
				Nonce::from_slice(&nonce),
				Payload {
					msg: plaintext,
					aad: &aad,
				},
			)
			.map_err(|_| AuthError::Config("a secret couldn't be encrypted".into()))?;
		let id = key.id.as_bytes();
		let mut sealed = Vec::with_capacity(2 + id.len() + NONCE_LENGTH + ciphertext.len());
		sealed.push(SEALED_FORMAT);
		sealed.push(u8::try_from(id.len()).expect("key IDs are at most 64 bytes"));

		sealed.extend_from_slice(id);
		sealed.extend_from_slice(&nonce);
		sealed.extend_from_slice(&ciphertext);
		Ok(sealed)
	}

	/// The ID of the newest key: the one that encrypts new secrets, and that
	/// new [derived](Self::derive) keys come from.
	pub fn newest(&self) -> &str {
		&self.keys[0].id
	}

	/// The IDs of every key, newest first.
	pub fn ids(&self) -> Vec<&str> {
		self.keys.iter().map(|key| key.id.as_str()).collect()
	}

	/// The ID of the key that sealed `sealed`, from its header, if it has
	/// one.
	pub(crate) fn sealed_with(sealed: &[u8]) -> Option<&str> {
		let (&format, rest) = sealed.split_first()?;
		let (&length, rest) = rest.split_first()?;

		if format != SEALED_FORMAT || rest.len() < usize::from(length) {
			return None;
		}

		std::str::from_utf8(&rest[..usize::from(length)]).ok()
	}

	/// Decrypts what [`Keys::seal`] made with the same `context`.
	///
	/// # Errors
	///
	/// [`AuthError::Config`] when the key it names is no longer listed, or
	/// it doesn't decrypt: tampered with, truncated, or sealed for another
	/// context. The message never includes the value.
	pub(crate) fn open(&self, context: &[&[u8]], sealed: &[u8]) -> Result<Vec<u8>, AuthError> {
		let unreadable = || AuthError::Config("a stored secret couldn't be decrypted".into());
		let (&format, rest) = sealed.split_first().ok_or_else(unreadable)?;
		let (&id_length, rest) = rest.split_first().ok_or_else(unreadable)?;

		if format != SEALED_FORMAT || rest.len() < usize::from(id_length) + NONCE_LENGTH {
			return Err(unreadable());
		}

		let (id, rest) = rest.split_at(usize::from(id_length));
		let (nonce, ciphertext) = rest.split_at(NONCE_LENGTH);
		let key = self
			.keys
			.iter()
			.find(|key| key.id.as_bytes() == id)
			.ok_or_else(|| {
				AuthError::Config(format!(
					"a stored secret was encrypted with key `{}`, which the keys no longer list",
					String::from_utf8_lossy(id)
				))
			})?;
		let aad = associated_data(context);
		cipher(&key.secret)
			.decrypt(
				Nonce::from_slice(nonce),
				Payload {
					msg: ciphertext,
					aad: &aad,
				},
			)
			.map_err(|_| unreadable())
	}
}

/// The longest key ID, so a sealed value's header stays one byte.
const MAX_KEY_ID: usize = 64;

/// The layout [`Keys::seal`] writes.
const SEALED_FORMAT: u8 = 1;

/// AES-GCM-SIV's nonce: 96 bits.
const NONCE_LENGTH: usize = 12;

/// The cipher for one key. Encryption uses its own key, derived from the
/// configured one, so the keys that hash codes never encrypt.
fn cipher(secret: &[u8; 32]) -> Aes256GcmSiv {
	let key = mac(secret, &[b"leptoskit-auth encryption v1"]);
	Aes256GcmSiv::new_from_slice(&key).expect("an HMAC-SHA-256 is 32 bytes, AES-256's key size")
}

/// What a sealed value is bound to, as AES-GCM-SIV's associated data.
fn associated_data(context: &[&[u8]]) -> Vec<u8> {
	digest(context)
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
pub(crate) const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

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

/// SHA-256 of `bytes` alone, as PKCE's `S256` hashes its verifier.
#[cfg_attr(not(feature = "oauth"), allow(dead_code, reason = "PKCE needs it"))]
pub(crate) fn digest_plain(bytes: &[u8]) -> Vec<u8> {
	Sha256::digest(bytes).to_vec()
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
	fn derived_keys_are_per_key_and_per_purpose() {
		let keys = Keys::parse(&format!(
			"new:{},old:{}",
			URL_SAFE_NO_PAD.encode([2; 32]),
			URL_SAFE_NO_PAD.encode([1; 32])
		))
		.unwrap();
		let signing = keys.derive("new", b"signing").unwrap();

		assert_eq!(keys.newest(), "new");
		assert_eq!(keys.derive("new", b"signing"), Some(signing));
		assert_ne!(keys.derive("old", b"signing"), Some(signing));
		assert_ne!(keys.derive("new", b"other"), Some(signing));
		assert_ne!(keys.hash(&[b"signing"]), signing.to_vec());
		assert_eq!(keys.derive("gone", b"signing"), None);
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
	fn sealed_secrets_open_only_with_their_context_and_a_listed_key() {
		const ADA: [&[u8]; 2] = [b"totp", b"ada"];
		const GRACE: [&[u8]; 2] = [b"totp", b"grace"];
		let old = Keys::single("old", [1; 32]);
		let sealed = old.seal(&ADA, b"the secret").unwrap();

		assert_eq!(sealed[0], SEALED_FORMAT);
		assert!(!sealed.windows(10).any(|window| window == b"the secret"));
		assert_eq!(old.open(&ADA, &sealed).unwrap(), b"the secret");
		// Sealing twice gives different bytes: the nonce is random.
		assert_ne!(old.seal(&ADA, b"the secret").unwrap(), sealed);

		// Another account's row, or a tampered byte, doesn't open.
		assert!(old.open(&GRACE, &sealed).is_err());
		let mut tampered = sealed.clone();
		*tampered.last_mut().unwrap() ^= 1;
		assert!(old.open(&ADA, &tampered).is_err());
		assert!(old.open(&ADA, &sealed[..20]).is_err());
		assert!(old.open(&ADA, &[]).is_err());

		// After a rotation the old key still opens it; once dropped, the error
		// names the key, never the value.
		let rotated = Keys::parse(&format!(
			"new:{},old:{}",
			URL_SAFE_NO_PAD.encode([2; 32]),
			URL_SAFE_NO_PAD.encode([1; 32])
		))
		.unwrap();
		assert_eq!(rotated.open(&ADA, &sealed).unwrap(), b"the secret");
		assert_eq!(Keys::sealed_with(&sealed), Some("old"));
		assert_eq!(Keys::sealed_with(&[SEALED_FORMAT]), None);
		assert_eq!(Keys::sealed_with(&[9, 0]), None);
		assert_eq!(rotated.newest(), "new");
		assert_eq!(rotated.ids(), ["new", "old"]);
		let dropped = Keys::single("new", [2; 32]);

		let error = dropped.open(&ADA, &sealed).unwrap_err().to_string();
		assert!(error.contains("key `old`"), "{error}");
		assert!(!error.contains("secret`"), "{error}");
	}

	#[test]
	fn key_ids_fit_a_sealed_header() {
		let key = URL_SAFE_NO_PAD.encode([1; 32]);
		assert!(Keys::parse(&format!("{}:{key}", "a".repeat(64))).is_ok());
		let error = Keys::parse(&format!("{}:{key}", "a".repeat(65)))
			.unwrap_err()
			.to_string();
		assert!(error.contains("at most 64 bytes"), "{error}");
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
