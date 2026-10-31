//! Compact JWS with ES256: just what ID and access tokens need, and nothing
//! a token could use to choose its own verification.
//!
//! Verification accepts only `ES256`, only a key ID this server published,
//! and only the `typ` the caller expects, so an ID token can never pass as
//! an access token or the other way round.

use p256::ecdsa::Signature;
use p256::ecdsa::SigningKey;
use p256::ecdsa::VerifyingKey;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::signature::Verifier;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::secrets::base64url;
use crate::secrets::from_base64url;

/// The `typ` of an access token (RFC 9068 §2.1).
pub(crate) const ACCESS_TOKEN_TYPE: &str = "at+jwt";

/// The `typ` of an ID token.
pub(crate) const ID_TOKEN_TYPE: &str = "JWT";

/// The only algorithm this server signs or accepts.
pub(crate) const ALGORITHM: &str = "ES256";

#[derive(Serialize, Deserialize)]
struct Header {
	alg: String,
	#[serde(default)]
	typ: Option<String>,
	#[serde(default)]
	kid: Option<String>,
}

/// Signs `claims` as a compact JWS.
pub(crate) fn sign(kid: &str, key: &SigningKey, typ: &str, claims: &Value) -> String {
	let header = Header {
		alg: ALGORITHM.into(),
		typ: Some(typ.into()),
		kid: Some(kid.into()),
	};
	let header = serde_json::to_vec(&header).expect("a header always serializes");
	let claims = serde_json::to_vec(claims).expect("claims always serialize");
	let input = format!("{}.{}", base64url(&header), base64url(&claims));
	let signature: Signature = key.sign(input.as_bytes());

	format!("{input}.{}", base64url(&signature.to_bytes()))
}

/// A token whose signature hasn't been checked yet.
pub(crate) struct Unverified<'a> {
	/// The key ID the token names.
	pub kid: String,
	typ: Option<String>,
	input: &'a str,
	signature: Signature,
	claims: Value,
}

/// Splits a compact JWS, refusing anything but an `ES256` token with a key
/// ID and a JSON object for claims.
pub(crate) fn parse(token: &str) -> Option<Unverified<'_>> {
	let (input, signature) = token.rsplit_once('.')?;
	let (header, claims) = input.split_once('.')?;
	let header: Header = serde_json::from_slice(&from_base64url(header)?).ok()?;
	let claims: Value = serde_json::from_slice(&from_base64url(claims)?).ok()?;
	let signature = Signature::from_slice(&from_base64url(signature)?).ok()?;

	if header.alg != ALGORITHM || !claims.is_object() {
		return None;
	}

	Some(Unverified {
		kid: header.kid?,
		typ: header.typ,
		input,
		signature,
		claims,
	})
}

impl Unverified<'_> {
	/// The claims, if `key` signed them and the token is of type `typ`.
	pub(crate) fn verify(self, key: &VerifyingKey, typ: &str) -> Option<Value> {
		let typ_matches = self
			.typ
			.as_deref()
			.is_some_and(|found| found.eq_ignore_ascii_case(typ));

		(typ_matches && key.verify(self.input.as_bytes(), &self.signature).is_ok())
			.then_some(self.claims)
	}
}

/// A signing key derived from 32 secret bytes, if they're a valid P-256
/// scalar, which all but about one in 2³² are.
pub(crate) fn signing_key(secret: &[u8; 32]) -> Option<SigningKey> {
	SigningKey::from_slice(secret).ok()
}

/// A public key's coordinates, in base64url, as a JWK carries them.
pub(crate) fn coordinates(key: &VerifyingKey) -> (String, String) {
	let point = key.to_sec1_point(false);
	let bytes = point.as_bytes();
	(base64url(&bytes[1..33]), base64url(&bytes[33..65]))
}

/// The public key with these coordinates.
pub(crate) fn verifying_key(x: &str, y: &str) -> Option<VerifyingKey> {
	let x = from_base64url(x)?;
	let y = from_base64url(y)?;

	if x.len() != 32 || y.len() != 32 {
		return None;
	}

	let mut point = Vec::with_capacity(65);
	point.push(0x04);
	point.extend_from_slice(&x);
	point.extend_from_slice(&y);
	VerifyingKey::from_sec1_bytes(&point).ok()
}

/// The JWK for a public key.
pub(crate) fn jwk(kid: &str, x: &str, y: &str) -> Value {
	serde_json::json!({
		"kty": "EC",
		"crv": "P-256",
		"alg": ALGORITHM,
		"use": "sig",
		"kid": kid,
		"x": x,
		"y": y,
	})
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	fn key(byte: u8) -> SigningKey {
		signing_key(&[byte; 32]).unwrap()
	}

	#[test]
	fn tokens_verify_only_with_their_key_and_type() {
		let signer = key(7);
		let (x, y) = coordinates(signer.verifying_key());
		let public = verifying_key(&x, &y).unwrap();
		let token = sign("k1", &signer, ACCESS_TOKEN_TYPE, &json!({ "sub": "a" }));

		let parsed = parse(&token).unwrap();
		assert_eq!(parsed.kid, "k1");
		assert_eq!(
			parsed.verify(&public, ACCESS_TOKEN_TYPE),
			Some(json!({ "sub": "a" }))
		);
		assert!(
			parse(&token)
				.unwrap()
				.verify(&public, ID_TOKEN_TYPE)
				.is_none()
		);

		let other = *key(8).verifying_key();
		assert!(
			parse(&token)
				.unwrap()
				.verify(&other, ACCESS_TOKEN_TYPE)
				.is_none()
		);

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
		assert!(parse(&format!("{no_kid}.{claims}.{signature}")).is_none());
	}

	#[test]
	fn jwks_carry_the_public_coordinates() {
		let (x, y) = coordinates(key(9).verifying_key());
		let jwk = jwk("k", &x, &y);
		assert_eq!(jwk["kty"], "EC");
		assert_eq!(jwk["crv"], "P-256");
		assert_eq!(jwk["x"].as_str().unwrap().len(), 43);
		assert!(verifying_key("AA", &y).is_none());
		assert!(signing_key(&[0; 32]).is_none());
	}
}
