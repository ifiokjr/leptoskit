//! Publisher signatures: who released a bundle, not just whether it arrived
//! intact.
//!
//! A release is signed with an ed25519 key over its manifest hash, which
//! covers the app, channel, version, build, mandatory flag, and every file
//! hash. The signature is detached: it is stored next to the container and
//! travels in the update offer, so the container format is unchanged and a
//! device can reject an offer before downloading anything.
//!
//! Keys are written as 64 lowercase hex characters: the 32-byte public key,
//! or the 32-byte secret seed. App shells pin a [`TrustedKeys`] set; more
//! than one key lets a publisher rotate by shipping the new public key in a
//! store release before signing with it.

use std::fmt;

use ed25519_dalek::Signature;
use ed25519_dalek::Signer;
use ed25519_dalek::SigningKey;
use ed25519_dalek::VerifyingKey;
use serde::Deserialize;
use serde::Serialize;

use crate::CodePushError;
use crate::manifest::BundleManifest;

/// The domain separator prepended to every signed message, so a release
/// signature can never be replayed as a signature over anything else.
const SIGNING_CONTEXT: &[u8] = b"leptoskit-codepush-release-v1:";

/// The bytes a release signature covers: the context, then the manifest's
/// hex bundle hash.
fn signed_message(manifest: &BundleManifest) -> Vec<u8> {
	let hash = manifest.bundle_hash();
	let mut message = Vec::with_capacity(SIGNING_CONTEXT.len() + hash.len());
	message.extend_from_slice(SIGNING_CONTEXT);
	message.extend_from_slice(hash.as_bytes());
	message
}

/// Decodes exactly `N` bytes from a hex string, ignoring surrounding
/// whitespace.
fn decode_hex<const N: usize>(what: &str, encoded: &str) -> Result<[u8; N], CodePushError> {
	let mut bytes = [0u8; N];
	// The decoding error is not passed on: it quotes characters of the input,
	// which may be a secret key.
	hex::decode_to_slice(encoded.trim(), &mut bytes).map_err(|_| {
		CodePushError::InvalidKey(format!("{what} must be {} hex characters", N * 2))
	})?;
	Ok(bytes)
}

/// A publisher's secret signing key.
///
/// Its `Debug` output names only the public key, and the secret bytes are
/// zeroed when it drops.
pub struct ReleaseSigningKey(SigningKey);

impl ReleaseSigningKey {
	/// A signing key from its 32-byte secret seed.
	pub fn from_seed(seed: &[u8; 32]) -> Self {
		Self(SigningKey::from_bytes(seed))
	}

	/// Parses a signing key from the hex encoding of its secret seed, as
	/// written by `leptoskit codepush keygen`.
	pub fn from_hex(encoded: &str) -> Result<Self, CodePushError> {
		Ok(Self::from_seed(&decode_hex::<32>(
			"a signing key",
			encoded,
		)?))
	}

	/// The hex encoding of the secret seed, for writing the key to a file.
	pub fn to_hex(&self) -> String {
		hex::encode(self.0.as_bytes())
	}

	/// The public half, which app shells trust.
	pub fn public_key(&self) -> ReleasePublicKey {
		ReleasePublicKey(self.0.verifying_key())
	}

	/// Signs a release manifest.
	pub fn sign(&self, manifest: &BundleManifest) -> ReleaseSignature {
		let signature = self.0.sign(&signed_message(manifest));
		ReleaseSignature {
			public_key: self.public_key().to_hex(),
			signature: hex::encode(signature.to_bytes()),
		}
	}
}

impl fmt::Debug for ReleaseSigningKey {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("ReleaseSigningKey")
			.field("public_key", &self.public_key().to_hex())
			.finish_non_exhaustive()
	}
}

/// A publisher's public key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ReleasePublicKey(VerifyingKey);

impl ReleasePublicKey {
	/// Parses a public key from hex, rejecting points that are not on the
	/// curve and weak (small-order) keys that would verify forged signatures.
	pub fn from_hex(encoded: &str) -> Result<Self, CodePushError> {
		let bytes = decode_hex::<32>("a public key", encoded)?;
		let key = VerifyingKey::from_bytes(&bytes)
			.map_err(|error| CodePushError::InvalidKey(format!("invalid public key: {error}")))?;

		if key.is_weak() {
			return Err(CodePushError::InvalidKey(
				"the public key is a weak, small-order point".to_owned(),
			));
		}

		Ok(Self(key))
	}

	/// The hex encoding of the key.
	pub fn to_hex(&self) -> String {
		hex::encode(self.0.as_bytes())
	}
}

impl fmt::Debug for ReleasePublicKey {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_tuple("ReleasePublicKey")
			.field(&self.to_hex())
			.finish()
	}
}

/// A detached release signature: the signing public key and the signature
/// over the manifest, both hex encoded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseSignature {
	/// The public key that made the signature.
	pub public_key: String,
	/// The ed25519 signature over the manifest hash.
	pub signature: String,
}

/// The publisher keys an app shell accepts releases from. Never empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedKeys(Vec<ReleasePublicKey>);

impl TrustedKeys {
	/// A trust set from public keys.
	///
	/// Fails with [`CodePushError::InvalidKey`] when `keys` is empty: a client
	/// that trusts nobody could never install an update.
	pub fn new(keys: impl IntoIterator<Item = ReleasePublicKey>) -> Result<Self, CodePushError> {
		let keys: Vec<_> = keys.into_iter().collect();

		if keys.is_empty() {
			return Err(CodePushError::InvalidKey(
				"at least one trusted public key is required".to_owned(),
			));
		}

		Ok(Self(keys))
	}

	/// A trust set from hex public keys, such as `codepush.public_keys`.
	pub fn from_hex<S: AsRef<str>>(
		keys: impl IntoIterator<Item = S>,
	) -> Result<Self, CodePushError> {
		let keys = keys
			.into_iter()
			.map(|key| ReleasePublicKey::from_hex(key.as_ref()))
			.collect::<Result<Vec<_>, _>>()?;
		Self::new(keys)
	}

	/// Whether `key` is in the set.
	pub fn contains(&self, key: &ReleasePublicKey) -> bool {
		self.0.contains(key)
	}

	/// Checks that `signature` is a valid signature over `manifest` by one of
	/// the trusted keys.
	///
	/// Verification is strict: non-canonical and malleable signatures are
	/// rejected.
	pub fn verify(
		&self,
		manifest: &BundleManifest,
		signature: Option<&ReleaseSignature>,
	) -> Result<(), CodePushError> {
		let release = || {
			format!(
				"{}/{}/{}+{}",
				manifest.app, manifest.channel, manifest.version, manifest.build
			)
		};
		let signature = signature.ok_or_else(|| CodePushError::Unsigned(release()))?;
		let key = ReleasePublicKey::from_hex(&signature.public_key)
			.ok()
			.filter(|key| self.contains(key))
			.ok_or_else(|| CodePushError::UntrustedKey(signature.public_key.clone()))?;
		let bytes = decode_hex::<64>("a signature", &signature.signature)
			.map_err(|_| CodePushError::BadSignature(release()))?;
		key.0
			.verify_strict(&signed_message(manifest), &Signature::from_bytes(&bytes))
			.map_err(|_| CodePushError::BadSignature(release()))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::manifest::MANIFEST_FORMAT_VERSION;

	fn manifest() -> BundleManifest {
		BundleManifest {
			format_version: MANIFEST_FORMAT_VERSION,
			app: "app".to_owned(),
			channel: "production".to_owned(),
			version: semver::Version::new(1, 0, 0),
			build: 1,
			entry: "app.wasm".to_owned(),
			files: Vec::new(),
			created_unix: 0,
			mandatory: false,
		}
	}

	#[test]
	fn keys_round_trip_through_hex() {
		let key = ReleaseSigningKey::from_seed(&[7; 32]);
		let parsed = ReleaseSigningKey::from_hex(&format!("{}\n", key.to_hex())).expect("parse");
		assert_eq!(parsed.public_key(), key.public_key());
		assert_eq!(
			ReleasePublicKey::from_hex(&key.public_key().to_hex()).expect("parse"),
			key.public_key()
		);
	}

	#[test]
	fn debug_output_never_contains_the_secret() {
		let key = ReleaseSigningKey::from_seed(&[7; 32]);
		assert!(!format!("{key:?}").contains(&key.to_hex()));
	}

	#[test]
	fn rejects_malformed_and_weak_public_keys() {
		for encoded in ["", "zz", &"00".repeat(31), &"00".repeat(33)] {
			assert!(
				matches!(
					ReleasePublicKey::from_hex(encoded),
					Err(CodePushError::InvalidKey(_))
				),
				"{encoded:?} must be rejected"
			);
		}

		// The identity point has small order.
		let mut identity = [0u8; 32];
		identity[0] = 1;
		assert!(matches!(
			ReleasePublicKey::from_hex(&hex::encode(identity)),
			Err(CodePushError::InvalidKey(_))
		));
	}

	#[test]
	fn an_empty_trust_set_is_rejected() {
		assert!(matches!(
			TrustedKeys::new([]),
			Err(CodePushError::InvalidKey(_))
		));
	}

	#[test]
	fn verifies_against_any_trusted_key() {
		let old = ReleaseSigningKey::from_seed(&[1; 32]);
		let new = ReleaseSigningKey::from_seed(&[2; 32]);
		let trusted = TrustedKeys::new([old.public_key(), new.public_key()]).expect("keys");
		let manifest = manifest();
		trusted
			.verify(&manifest, Some(&old.sign(&manifest)))
			.expect("old key verifies");
		trusted
			.verify(&manifest, Some(&new.sign(&manifest)))
			.expect("new key verifies");
	}

	#[test]
	fn a_signature_does_not_cover_a_changed_manifest() {
		let key = ReleaseSigningKey::from_seed(&[1; 32]);
		let trusted = TrustedKeys::new([key.public_key()]).expect("keys");
		let signature = key.sign(&manifest());
		let mut changed = manifest();
		changed.mandatory = true;
		assert!(matches!(
			trusted.verify(&changed, Some(&signature)),
			Err(CodePushError::BadSignature(_))
		));
	}
}
