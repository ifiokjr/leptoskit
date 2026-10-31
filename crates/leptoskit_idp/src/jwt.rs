//! Compact JWS with ES256 and RS256: just what ID tokens, access tokens and
//! client assertions need, and nothing a token could use to choose its own
//! verification.
//!
//! Verification accepts only the algorithm of the key it's checked with,
//! so a token can't pick a weaker one (or `none`), and only the `typ` the
//! caller expects, so an ID token can never pass as an access token or the
//! other way round.

use std::sync::Arc;

use p256::ecdsa::Signature;
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::signature::Verifier as _;
use ring::signature::RsaKeyPair;
use ring::signature::RsaPublicKeyComponents;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::SigningAlgorithm;
use crate::secrets::base64url;
use crate::secrets::from_base64url;

/// The `typ` of an access token (RFC 9068 §2.1).
pub(crate) const ACCESS_TOKEN_TYPE: &str = "at+jwt";

/// The `typ` of an ID token.
pub(crate) const ID_TOKEN_TYPE: &str = "JWT";

#[derive(Serialize, Deserialize)]
struct Header {
	alg: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	typ: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	kid: Option<String>,
}

/// A private key this server signs with.
#[derive(Clone)]
pub(crate) enum PrivateKey {
	/// ES256: derived from the server's keys.
	Ec(Box<p256::ecdsa::SigningKey>),
	/// RS256: the RSA key the server was given.
	Rsa(Arc<RsaKeyPair>),
}

/// A key that signs tokens, with the ID they name it by.
#[derive(Clone)]
pub(crate) struct Signer {
	pub kid: String,
	pub key: PrivateKey,
}

/// A public key a token is checked against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PublicKey {
	Ec(p256::ecdsa::VerifyingKey),
	Rsa {
		/// The modulus, big-endian.
		n: Vec<u8>,
		/// The exponent, big-endian.
		e: Vec<u8>,
	},
}

impl PublicKey {
	pub(crate) fn algorithm(&self) -> SigningAlgorithm {
		match self {
			Self::Ec(_) => SigningAlgorithm::Es256,
			Self::Rsa { .. } => SigningAlgorithm::Rs256,
		}
	}

	fn verifies(&self, input: &[u8], signature: &[u8]) -> bool {
		match self {
			Self::Ec(key) => {
				Signature::from_slice(signature)
					.is_ok_and(|signature| key.verify(input, &signature).is_ok())
			}
			Self::Rsa { n, e } => {
				RsaPublicKeyComponents { n, e }
					.verify(
						&ring::signature::RSA_PKCS1_2048_8192_SHA256,
						input,
						signature,
					)
					.is_ok()
			}
		}
	}

	/// The key from a JWK, if it's an ES256 or RS256 public key this server
	/// can check signatures with. A JWK naming another `alg`, or meant for
	/// encryption, is refused.
	pub(crate) fn from_jwk(jwk: &Value) -> Option<Self> {
		let text = |name: &str| jwk.get(name).and_then(Value::as_str);

		if text("use").is_some_and(|usage| usage != "sig") {
			return None;
		}

		match (text("kty")?, text("alg")) {
			("EC", None | Some("ES256")) if text("crv") == Some("P-256") => {
				verifying_key(text("x")?, text("y")?).map(Self::Ec)
			}
			("RSA", None | Some("RS256")) => {
				let n = from_base64url(text("n")?)?;
				let e = from_base64url(text("e")?)?;
				// ring checks the sizes again when it verifies.
				(n.len() >= 256 && !e.is_empty()).then_some(Self::Rsa { n, e })
			}
			_ => None,
		}
	}
}

/// Signs `claims` as a compact JWS with `typ`.
///
/// # Errors
///
/// When the RSA key can't sign, which only a broken key does.
pub(crate) fn sign(
	signer: &Signer,
	typ: &str,
	claims: &Value,
) -> Result<String, ring::error::Unspecified> {
	let algorithm = match &signer.key {
		PrivateKey::Ec(_) => SigningAlgorithm::Es256,
		PrivateKey::Rsa(_) => SigningAlgorithm::Rs256,
	};
	let header = Header {
		alg: algorithm.as_str().into(),
		typ: Some(typ.into()),
		kid: Some(signer.kid.clone()),
	};
	let header = serde_json::to_vec(&header).expect("a header always serializes");
	let claims = serde_json::to_vec(claims).expect("claims always serialize");
	let input = format!("{}.{}", base64url(&header), base64url(&claims));
	let signature = match &signer.key {
		PrivateKey::Ec(key) => {
			let signature: Signature = key.sign(input.as_bytes());
			signature.to_bytes().to_vec()
		}
		PrivateKey::Rsa(key) => {
			let mut signature = vec![0; key.public().modulus_len()];
			key.sign(
				&ring::signature::RSA_PKCS1_SHA256,
				&ring::rand::SystemRandom::new(),
				input.as_bytes(),
				&mut signature,
			)?;
			signature
		}
	};

	Ok(format!("{input}.{}", base64url(&signature)))
}

/// A token whose signature hasn't been checked yet.
pub(crate) struct Unverified<'a> {
	/// The key ID the token names, if any.
	pub kid: Option<String>,
	pub algorithm: SigningAlgorithm,
	typ: Option<String>,
	input: &'a str,
	signature: Vec<u8>,
	claims: Value,
}

/// Splits a compact JWS, refusing anything but an `ES256` or `RS256` token
/// with a JSON object for claims.
pub(crate) fn parse(token: &str) -> Option<Unverified<'_>> {
	let (input, signature) = token.rsplit_once('.')?;
	let (header, claims) = input.split_once('.')?;
	let header: Header = serde_json::from_slice(&from_base64url(header)?).ok()?;
	let claims: Value = serde_json::from_slice(&from_base64url(claims)?).ok()?;
	let signature = from_base64url(signature)?;
	let algorithm = match header.alg.as_str() {
		"ES256" => SigningAlgorithm::Es256,
		"RS256" => SigningAlgorithm::Rs256,
		_ => return None,
	};

	if !claims.is_object() || signature.is_empty() {
		return None;
	}

	Some(Unverified {
		kid: header.kid,
		algorithm,
		typ: header.typ,
		input,
		signature,
		claims,
	})
}

impl Unverified<'_> {
	/// The claims, if `key` signed them with its own algorithm and the token
	/// is of type `typ`.
	pub(crate) fn verify(&self, key: &PublicKey, typ: &str) -> Option<Value> {
		let typ_matches = self
			.typ
			.as_deref()
			.is_some_and(|found| found.eq_ignore_ascii_case(typ));

		(typ_matches && self.signed_by(key)).then(|| self.claims.clone())
	}

	/// The claims of an app's own assertion, if `key` signed them. Its `typ`
	/// may be absent, `JWT`, or RFC 7523's `client-authentication+jwt`, since
	/// apps' libraries differ.
	pub(crate) fn verify_assertion(&self, key: &PublicKey) -> Option<Value> {
		let typ_allowed = self.typ.as_deref().is_none_or(|typ| {
			typ.eq_ignore_ascii_case("JWT") || typ.eq_ignore_ascii_case("client-authentication+jwt")
		});

		(typ_allowed && self.signed_by(key)).then(|| self.claims.clone())
	}

	fn signed_by(&self, key: &PublicKey) -> bool {
		key.algorithm() == self.algorithm && key.verifies(self.input.as_bytes(), &self.signature)
	}
}

/// A signing key derived from 32 secret bytes, if they're a valid P-256
/// scalar, which all but about one in 2³² are.
pub(crate) fn signing_key(secret: &[u8; 32]) -> Option<p256::ecdsa::SigningKey> {
	p256::ecdsa::SigningKey::from_slice(secret).ok()
}

/// A public key's coordinates, in base64url, as a JWK carries them.
pub(crate) fn coordinates(key: &p256::ecdsa::VerifyingKey) -> (String, String) {
	let point = key.to_sec1_point(false);
	let bytes = point.as_bytes();
	(base64url(&bytes[1..33]), base64url(&bytes[33..65]))
}

/// The public key with these coordinates.
pub(crate) fn verifying_key(x: &str, y: &str) -> Option<p256::ecdsa::VerifyingKey> {
	let x = from_base64url(x)?;
	let y = from_base64url(y)?;

	if x.len() != 32 || y.len() != 32 {
		return None;
	}

	let mut point = Vec::with_capacity(65);
	point.push(0x04);
	point.extend_from_slice(&x);
	point.extend_from_slice(&y);
	p256::ecdsa::VerifyingKey::from_sec1_bytes(&point).ok()
}

/// An RSA private key, from PEM (`PRIVATE KEY`, PKCS#8, or `RSA PRIVATE
/// KEY`, PKCS#1) or the DER either holds.
pub(crate) fn rsa_key(text: &[u8]) -> Option<RsaKeyPair> {
	let der = pem_body(text).unwrap_or_else(|| text.to_vec());
	RsaKeyPair::from_pkcs8(&der)
		.or_else(|_| RsaKeyPair::from_der(&der))
		.ok()
		// Under 2048 bits is too weak to verify, so refuse to sign with it.
		.filter(|key| key.public().modulus_len() >= 256)
}

/// The base64 between a PEM block's header and footer, decoded.
fn pem_body(text: &[u8]) -> Option<Vec<u8>> {
	use base64::Engine;

	let text = std::str::from_utf8(text).ok()?;
	let body: String = text
		.lines()
		.map(str::trim)
		.skip_while(|line| !line.starts_with("-----BEGIN "))
		.skip(1)
		.take_while(|line| !line.starts_with("-----END "))
		.collect();

	if body.is_empty() {
		return None;
	}

	base64::engine::general_purpose::STANDARD.decode(body).ok()
}

/// The JWK for a stored public key.
pub(crate) fn jwk(kid: &str, key: &PublicKey) -> Value {
	match key {
		PublicKey::Ec(key) => {
			let (x, y) = coordinates(key);
			serde_json::json!({
				"kty": "EC",
				"crv": "P-256",
				"alg": "ES256",
				"use": "sig",
				"kid": kid,
				"x": x,
				"y": y,
			})
		}
		PublicKey::Rsa { n, e } => {
			serde_json::json!({
				"kty": "RSA",
				"alg": "RS256",
				"use": "sig",
				"kid": kid,
				"n": base64url(n),
				"e": base64url(e),
			})
		}
	}
}

/// An RSA public key's RFC 7638 thumbprint, which names it as `kid`.
pub(crate) fn rsa_thumbprint(n: &[u8], e: &[u8]) -> String {
	use sha2::Digest;

	let canonical = format!(
		r#"{{"e":"{}","kty":"RSA","n":"{}"}}"#,
		base64url(e),
		base64url(n)
	);
	base64url(&sha2::Sha256::digest(canonical.as_bytes()))
}

/// An RSA key pair's public half.
pub(crate) fn rsa_public(key: &RsaKeyPair) -> PublicKey {
	let components = RsaPublicKeyComponents::<Vec<u8>>::from(key.public());
	PublicKey::Rsa {
		n: components.n,
		e: components.e,
	}
}

#[cfg(test)]
pub(crate) mod tests {
	use serde_json::json;

	use super::*;

	/// A 2048-bit RSA key for tests only, made with `openssl genpkey`.
	pub(crate) const TEST_RSA_KEY: &str = include_str!("../tests/fixtures/test-only-rsa-key.pem");

	fn key(byte: u8) -> Signer {
		Signer {
			kid: "k1".into(),
			key: PrivateKey::Ec(Box::new(signing_key(&[byte; 32]).unwrap())),
		}
	}

	fn public(signer: &Signer) -> PublicKey {
		match &signer.key {
			PrivateKey::Ec(key) => PublicKey::Ec(*key.verifying_key()),
			PrivateKey::Rsa(key) => rsa_public(key),
		}
	}

	fn rsa() -> Signer {
		Signer {
			kid: "r1".into(),
			key: PrivateKey::Rsa(Arc::new(rsa_key(TEST_RSA_KEY.as_bytes()).unwrap())),
		}
	}

	#[test]
	fn tokens_verify_only_with_their_key_and_type() {
		for signer in [key(7), rsa()] {
			let public = public(&signer);
			let token = sign(&signer, ACCESS_TOKEN_TYPE, &json!({ "sub": "a" })).unwrap();

			let parsed = parse(&token).unwrap();
			assert_eq!(parsed.kid.as_deref(), Some(signer.kid.as_str()));
			assert_eq!(parsed.algorithm, public.algorithm());
			assert_eq!(
				parsed.verify(&public, ACCESS_TOKEN_TYPE),
				Some(json!({ "sub": "a" }))
			);
			assert!(parsed.verify(&public, ID_TOKEN_TYPE).is_none());
			assert!(
				parsed.verify_assertion(&public).is_none(),
				"at+jwt isn't an assertion"
			);

			let other = public_of(8);
			assert!(parsed.verify(&other, ACCESS_TOKEN_TYPE).is_none());

			// A changed payload breaks the signature.
			let mut parts: Vec<&str> = token.split('.').collect();
			let forged = base64url(br#"{"sub":"b"}"#);
			parts[1] = &forged;
			let forged = parts.join(".");
			assert!(
				parse(&forged)
					.unwrap()
					.verify(&public, ACCESS_TOKEN_TYPE)
					.is_none()
			);
		}

		// An RS256 header can't be checked against an EC key, or the other
		// way round.
		let token = sign(&rsa(), ID_TOKEN_TYPE, &json!({})).unwrap();
		assert!(
			parse(&token)
				.unwrap()
				.verify(&public_of(7), ID_TOKEN_TYPE)
				.is_none()
		);
	}

	fn public_of(byte: u8) -> PublicKey {
		public(&key(byte))
	}

	#[test]
	fn other_algorithms_and_malformed_tokens_are_refused() {
		let header = |alg: &str| base64url(format!(r#"{{"alg":"{alg}","kid":"k"}}"#).as_bytes());
		let claims = base64url(b"{}");
		let signature = base64url(&[0; 64]);

		assert!(parse(&format!("{}.{claims}.{signature}", header("none"))).is_none());
		assert!(parse(&format!("{}.{claims}.{signature}", header("HS256"))).is_none());
		assert!(parse(&format!("{}.{claims}.", header("ES256"))).is_none());
		assert!(parse("not a token").is_none());
		assert!(
			parse(&format!(
				"{}.{}.{signature}",
				header("ES256"),
				base64url(b"[]")
			))
			.is_none()
		);
		let no_kid = base64url(br#"{"alg":"ES256"}"#);
		let token = format!("{no_kid}.{claims}.{signature}");
		let parsed = parse(&token).unwrap();
		assert_eq!(parsed.kid, None);
		assert!(parsed.verify_assertion(&public_of(1)).is_none());
	}

	#[test]
	fn assertions_take_their_own_types() {
		let signer = key(3);
		let public = public(&signer);
		for typ in ["JWT", "client-authentication+jwt"] {
			let token = sign(&signer, typ, &json!({ "iss": "app" })).unwrap();
			assert!(
				parse(&token).unwrap().verify_assertion(&public).is_some(),
				"{typ}"
			);
		}
		let header = base64url(br#"{"alg":"ES256"}"#);
		let input = format!("{header}.{}", base64url(br#"{"iss":"app"}"#));
		let PrivateKey::Ec(private) = &signer.key else {
			unreachable!()
		};
		let signature: Signature = private.sign(input.as_bytes());
		let untyped = format!("{input}.{}", base64url(&signature.to_bytes()));
		assert!(parse(&untyped).unwrap().verify_assertion(&public).is_some());
	}

	#[test]
	fn jwks_carry_the_public_halves_and_read_back() {
		for signer in [key(9), rsa()] {
			let public = public(&signer);
			let jwk = jwk("k", &public);
			assert_eq!(PublicKey::from_jwk(&jwk), Some(public));
		}

		let (x, y) = coordinates(&match public_of(9) {
			PublicKey::Ec(key) => key,
			PublicKey::Rsa { .. } => unreachable!(),
		});
		assert_eq!(x.len(), 43);
		assert!(verifying_key("AA", &y).is_none());
		assert!(signing_key(&[0; 32]).is_none());

		for refused in [
			json!({ "kty": "EC", "crv": "P-384", "x": x, "y": y }),
			json!({ "kty": "EC", "crv": "P-256", "alg": "ES384", "x": x, "y": y }),
			json!({ "kty": "EC", "crv": "P-256", "use": "enc", "x": x, "y": y }),
			json!({ "kty": "RSA", "n": "AQAB", "e": "AQAB" }),
			json!({ "kty": "oct", "k": "c2VjcmV0" }),
		] {
			assert_eq!(PublicKey::from_jwk(&refused), None, "{refused}");
		}
	}

	#[test]
	fn rsa_keys_read_from_pem_and_name_themselves() {
		let key = rsa_key(TEST_RSA_KEY.as_bytes()).unwrap();
		let PublicKey::Rsa { n, e } = rsa_public(&key) else {
			unreachable!()
		};
		assert_eq!(n.len(), 256);
		assert_eq!(rsa_thumbprint(&n, &e).len(), 43);
		assert_eq!(rsa_thumbprint(&n, &e), rsa_thumbprint(&n, &e));
		assert!(rsa_key(b"not a key").is_none());
		assert!(rsa_key(b"-----BEGIN PRIVATE KEY-----\n!!\n-----END PRIVATE KEY-----").is_none());
		assert!(pem_body(b"-----BEGIN X-----\n-----END X-----").is_none());
	}
}
