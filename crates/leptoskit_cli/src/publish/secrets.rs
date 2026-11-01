//! Credentials, and the signed tokens the stores' APIs ask for.
//!
//! A credential comes from the environment, or from a TOML file of the same
//! names outside the project. It lives in a [`Secret`], which never prints
//! its value, so no log line, error, or `Debug` output can show it.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::SystemRandom;
use ring::signature;

/// A credential's value. It prints as `[redacted]`.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
	/// Holds `value` as a secret.
	#[must_use]
	pub fn new(value: String) -> Self {
		Self(value)
	}

	/// The value, for the one request that needs it.
	#[must_use]
	pub fn expose(&self) -> &str {
		&self.0
	}
}

impl std::fmt::Debug for Secret {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str("[redacted]")
	}
}

impl std::fmt::Display for Secret {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str("[redacted]")
	}
}

/// Every credential a run can see: the environment, over a file.
#[derive(Debug, Default)]
pub struct Credentials {
	file: BTreeMap<String, Secret>,
}

impl Credentials {
	/// Reads `file`, a TOML table of the same names as the environment
	/// variables. A file inside `root` must be one git ignores, so it can't
	/// be committed by accident.
	///
	/// # Errors
	///
	/// When the file can't be read or parsed, or is inside the project and
	/// not ignored.
	pub fn load(file: Option<&Path>, root: &Path) -> Result<Self> {
		let Some(file) = file else {
			return Ok(Self::default());
		};
		let absolute = std::fs::canonicalize(file)
			.with_context(|| format!("could not read {}", file.display()))?;
		let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
		if absolute.starts_with(&root) {
			let ignored = crate::git::command(&root)
				.args(["check-ignore", "-q"])
				.arg(&absolute)
				.stderr(std::process::Stdio::null())
				.status()
				.is_ok_and(|status| status.success());
			anyhow::ensure!(
				ignored,
				"{} is inside the project and git doesn't ignore it; move it out, or ignore it, \
				 so your credentials can't be committed",
				file.display()
			);
		}
		let text = std::fs::read_to_string(&absolute)
			.with_context(|| format!("could not read {}", file.display()))?;
		let table: BTreeMap<String, toml::Value> =
			toml::from_str(&text).with_context(|| format!("could not parse {}", file.display()))?;
		let file = table
			.into_iter()
			.map(|(name, value)| {
				let value = match value {
					toml::Value::String(value) => value,
					other => other.to_string(),
				};
				(name, Secret(value))
			})
			.collect();
		Ok(Self { file })
	}

	/// The credential `name`, if it is set.
	#[must_use]
	pub fn get(&self, name: &str) -> Option<Secret> {
		std::env::var(name)
			.ok()
			.filter(|value| !value.is_empty())
			.map(Secret)
			.or_else(|| self.file.get(name).cloned())
	}

	/// The credential `name`, or an error that names it and says where to
	/// get it.
	///
	/// # Errors
	///
	/// When it isn't set.
	pub fn require(&self, name: &str, how: &str) -> Result<Secret> {
		self.get(name).with_context(|| {
			format!("{name} isn't set: {how}. `leptoskit publish credentials` lists every one")
		})
	}
}

fn now() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |elapsed| elapsed.as_secs())
}

fn encode(value: &serde_json::Value) -> String {
	URL_SAFE_NO_PAD.encode(value.to_string())
}

/// The DER inside a PEM block, such as a service account's
/// `-----BEGIN PRIVATE KEY-----`.
///
/// # Errors
///
/// When the text isn't PEM.
pub fn pem_der(pem: &str) -> Result<Vec<u8>> {
	let body: String = pem
		.lines()
		.filter(|line| !line.starts_with("-----"))
		.map(str::trim)
		.collect();
	anyhow::ensure!(!body.is_empty(), "the key isn't a PEM block");
	base64::engine::general_purpose::STANDARD
		.decode(body)
		.context("the key's PEM block isn't base64")
}

/// A JWT signed with RS256, as Google's OAuth token endpoint takes from a
/// service account.
///
/// # Errors
///
/// When the key isn't an RSA PKCS#8 key.
pub fn rs256(claims: &serde_json::Value, private_key_pem: &str) -> Result<String> {
	let key = signature::RsaKeyPair::from_pkcs8(&pem_der(private_key_pem)?)
		.map_err(|error| anyhow::anyhow!("the service account's key isn't usable: {error}"))?;
	let message = format!(
		"{}.{}",
		encode(&serde_json::json!({ "alg": "RS256", "typ": "JWT" })),
		encode(claims)
	);
	let mut signed = vec![0; key.public().modulus_len()];
	key.sign(
		&signature::RSA_PKCS1_SHA256,
		&SystemRandom::new(),
		message.as_bytes(),
		&mut signed,
	)
	.map_err(|_| anyhow::anyhow!("could not sign with the service account's key"))?;
	Ok(format!("{message}.{}", URL_SAFE_NO_PAD.encode(signed)))
}

/// A JWT signed with ES256 under the key `key_id`, as App Store Connect
/// takes: `private_key_pem` is the `.p8` key it gave out.
///
/// # Errors
///
/// When the key isn't a P-256 PKCS#8 key.
pub fn es256(claims: &serde_json::Value, key_id: &str, private_key_pem: &str) -> Result<String> {
	let random = SystemRandom::new();
	let key = signature::EcdsaKeyPair::from_pkcs8(
		&signature::ECDSA_P256_SHA256_FIXED_SIGNING,
		&pem_der(private_key_pem)?,
		&random,
	)
	.map_err(|error| anyhow::anyhow!("the App Store Connect key isn't usable: {error}"))?;
	let message = format!(
		"{}.{}",
		encode(&serde_json::json!({ "alg": "ES256", "kid": key_id, "typ": "JWT" })),
		encode(claims)
	);
	let signed = key
		.sign(&random, message.as_bytes())
		.map_err(|_| anyhow::anyhow!("could not sign with the App Store Connect key"))?;
	Ok(format!(
		"{message}.{}",
		URL_SAFE_NO_PAD.encode(signed.as_ref())
	))
}

/// A JWT signed with HS256, as addons.mozilla.org takes: issued now, for
/// one minute, with a unique ID.
///
/// # Errors
///
/// When randomness is unavailable.
pub fn hs256(issuer: &str, secret: &Secret) -> Result<String> {
	use ring::rand::SecureRandom;

	let mut nonce = [0_u8; 16];
	SystemRandom::new()
		.fill(&mut nonce)
		.map_err(|_| anyhow::anyhow!("no secure randomness"))?;
	let seconds = now();
	let claims = serde_json::json!({
		"iss": issuer,
		"jti": URL_SAFE_NO_PAD.encode(nonce),
		"iat": seconds,
		"exp": seconds + 60,
	});
	let message = format!(
		"{}.{}",
		encode(&serde_json::json!({ "alg": "HS256", "typ": "JWT" })),
		encode(&claims)
	);
	let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.expose().as_bytes());
	let tag = ring::hmac::sign(&key, message.as_bytes());
	Ok(format!(
		"{message}.{}",
		URL_SAFE_NO_PAD.encode(tag.as_ref())
	))
}

/// The seconds since the epoch, for token claims.
#[must_use]
pub fn issued_at() -> u64 {
	now()
}

#[cfg(test)]
pub(crate) mod tests {
	use super::*;

	#[test]
	fn secrets_never_print() {
		let secret = Secret("hunter2".into());
		assert_eq!(format!("{secret} {secret:?}"), "[redacted] [redacted]");
		assert_eq!(secret.expose(), "hunter2");
	}

	#[test]
	fn hs256_tokens_verify_with_the_secret() {
		let secret = Secret("shh".into());
		let token = hs256("user:1", &secret).unwrap();
		let (message, tag) = token.rsplit_once('.').unwrap();
		let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"shh");
		ring::hmac::verify(
			&key,
			message.as_bytes(),
			&URL_SAFE_NO_PAD.decode(tag).unwrap(),
		)
		.unwrap();
		let claims = message.split('.').nth(1).unwrap();
		let claims: serde_json::Value =
			serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims).unwrap()).unwrap();
		assert_eq!(claims["iss"], "user:1");
		assert_eq!(
			claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap(),
			60
		);
	}

	#[test]
	fn es256_tokens_verify_with_the_public_key() {
		use ring::signature::KeyPair as _;

		let Some(pem) = ec_key() else { return };
		let token = es256(&serde_json::json!({ "iss": "issuer" }), "KEY123", &pem).unwrap();
		let (message, signed) = token.rsplit_once('.').unwrap();
		let header = message.split('.').next().unwrap();
		let header: serde_json::Value =
			serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header).unwrap()).unwrap();
		assert_eq!(header["alg"], "ES256");
		assert_eq!(header["kid"], "KEY123");
		let key = signature::EcdsaKeyPair::from_pkcs8(
			&signature::ECDSA_P256_SHA256_FIXED_SIGNING,
			&pem_der(&pem).unwrap(),
			&SystemRandom::new(),
		)
		.unwrap();
		signature::UnparsedPublicKey::new(
			&signature::ECDSA_P256_SHA256_FIXED,
			key.public_key().as_ref(),
		)
		.verify(message.as_bytes(), &URL_SAFE_NO_PAD.decode(signed).unwrap())
		.unwrap();
	}

	/// A fresh P-256 key in PKCS#8 PEM, the form of App Store Connect's
	/// `.p8` files, so no private key is ever committed.
	pub(crate) fn ec_key() -> Option<String> {
		let output = std::process::Command::new("openssl")
			.args([
				"genpkey",
				"-algorithm",
				"EC",
				"-pkeyopt",
				"ec_paramgen_curve:P-256",
			])
			.output()
			.ok()?;
		output
			.status
			.success()
			.then(|| String::from_utf8(output.stdout).unwrap())
	}

	#[test]
	fn credentials_come_from_a_file_outside_the_project() {
		let outside = tempfile::tempdir().unwrap();
		let project = tempfile::tempdir().unwrap();
		let file = outside.path().join("credentials.toml");
		std::fs::write(&file, "LEPTOSKIT_PUBLISH_TEST_ONLY = \"value\"\n").unwrap();
		let credentials = Credentials::load(Some(&file), project.path()).unwrap();
		assert_eq!(
			credentials
				.get("LEPTOSKIT_PUBLISH_TEST_ONLY")
				.unwrap()
				.expose(),
			"value"
		);
		let error = credentials
			.require("LEPTOSKIT_PUBLISH_TEST_MISSING", "make one")
			.unwrap_err();
		assert!(
			error
				.to_string()
				.contains("LEPTOSKIT_PUBLISH_TEST_MISSING isn't set")
		);

		let inside = project.path().join("credentials.toml");
		std::fs::write(&inside, "A = \"b\"\n").unwrap();
		assert!(Credentials::load(Some(&inside), project.path()).is_err());
	}
}
