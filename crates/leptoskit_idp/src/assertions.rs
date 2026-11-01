//! `private_key_jwt` (OpenID Connect Core §9, RFC 7523): an app proves who
//! it is with a short-lived JWT it signs with its own private key. The
//! server keeps only the public half, so there's no shared secret to leak.

use leptoskit_db::DbError;
use serde_json::Value;

use crate::Assertion;
use crate::Client;
use crate::Idp;
use crate::IdpError;
use crate::NewAssertion;
use crate::Result;
use crate::jwt;
use crate::jwt::PublicKey;

/// The only `client_assertion_type` there is.
pub(crate) const JWT_BEARER: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

/// The longest an assertion may live: they're meant to be made per request.
const MAX_LIFETIME: chrono::TimeDelta = chrono::TimeDelta::minutes(60);

/// How far apart this server's clock and an app's may be.
const SKEW: i64 = 60;

/// The longest `jti` kept.
const MAX_JTI: usize = 256;

impl Idp {
	/// The client an assertion authenticates, if it's valid: signed by a key
	/// the client registered, issued by and about the client, meant for this
	/// provider, live, and never used before.
	pub(crate) async fn authenticate_assertion(
		&self,
		assertion: &str,
		claimed: Option<&str>,
	) -> Result<Client> {
		let unverified = jwt::parse(assertion).ok_or_else(IdpError::invalid_client)?;
		let peek: Value = unverified_claims(assertion).ok_or_else(IdpError::invalid_client)?;
		let issuer = peek
			.get("iss")
			.and_then(Value::as_str)
			.ok_or_else(IdpError::invalid_client)?;

		if claimed.is_some_and(|claimed| claimed != issuer) {
			return Err(IdpError::invalid_client());
		}

		let client = self
			.client(issuer)
			.await?
			.filter(|client| client.auth_method == crate::ClientAuth::PrivateKeyJwt)
			.ok_or_else(IdpError::invalid_client)?;
		let keys = registered_keys(&client);
		let claims = keys
			.iter()
			.filter(|(kid, key)| {
				key.algorithm() == unverified.algorithm
					&& unverified
						.kid
						.as_ref()
						.is_none_or(|wanted| kid.as_ref() == Some(wanted))
			})
			.find_map(|(_, key)| unverified.verify_assertion(key))
			.ok_or_else(IdpError::invalid_client)?;

		let text = |name: &str| claims.get(name).and_then(Value::as_str);
		let number = |name: &str| claims.get(name).and_then(Value::as_i64);
		let now = leptoskit_db::now().timestamp();

		if text("iss") != Some(client.client_id.as_str())
			|| text("sub") != Some(client.client_id.as_str())
			|| !self.is_audience(claims.get("aud"))
		{
			return Err(IdpError::invalid_client());
		}

		let exp = number("exp").ok_or_else(IdpError::invalid_client)?;

		if exp + SKEW <= now
			|| exp > now + MAX_LIFETIME.num_seconds() + SKEW
			|| number("nbf").is_some_and(|nbf| nbf > now + SKEW)
		{
			return Err(IdpError::invalid_client());
		}

		let jti = text("jti")
			.filter(|jti| !jti.is_empty() && jti.len() <= MAX_JTI)
			.ok_or_else(IdpError::invalid_client)?;
		self.spend_assertion(&client, jti, exp).await?;

		Ok(client)
	}

	/// Whether an assertion's `aud` names this provider: its issuer, or one
	/// of the endpoints apps authenticate to.
	fn is_audience(&self, audience: Option<&Value>) -> bool {
		let issuer = &self.inner.issuer;
		let accepted = |value: &str| {
			value == issuer
				|| [
					"/oauth/token",
					"/oauth/par",
					"/oauth/revoke",
					"/oauth/introspect",
					"/oauth/device_authorization",
				]
				.iter()
				.any(|path| value.strip_prefix(issuer.as_str()) == Some(path))
		};

		match audience {
			Some(Value::String(value)) => accepted(value),
			Some(Value::Array(values)) => values.iter().filter_map(Value::as_str).any(accepted),
			_ => false,
		}
	}

	/// Records an assertion as used, refusing one seen before. Assertions
	/// that have expired are forgotten: they'd be refused anyway.
	async fn spend_assertion(&self, client: &Client, jti: &str, exp: i64) -> Result<()> {
		let now = leptoskit_db::now();
		Assertion::query()
			.filter(|row| row.expires_at.lt(now))
			.delete(&self.inner.db)
			.await?;

		let expires_at =
			chrono::DateTime::from_timestamp(exp + SKEW, 0).ok_or_else(IdpError::invalid_client)?;
		let inserted = NewAssertion {
			client_id: client.id,
			jti_hash: crate::secrets::digest(&[client.client_id.as_bytes(), jti.as_bytes()]),
			expires_at,
		}
		.insert(&self.inner.db)
		.await;

		match inserted {
			Ok(_) => Ok(()),
			Err(DbError::Conflict { .. }) => Err(IdpError::invalid_client()),
			Err(error) => Err(error.into()),
		}
	}
}

/// The keys a client registered, with their IDs.
fn registered_keys(client: &Client) -> Vec<(Option<String>, PublicKey)> {
	let Some(jwks) = &client.jwks else {
		return Vec::new();
	};
	jwks.0
		.get("keys")
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(|jwk| {
			let kid = jwk.get("kid").and_then(Value::as_str).map(str::to_owned);
			PublicKey::from_jwk(jwk).map(|key| (kid, key))
		})
		.collect()
}

/// Checks a JWK set an app registers: at least one key this server can
/// verify with, and nothing else, so a typo can't hide.
pub(crate) fn check_jwks(jwks: &Value) -> std::result::Result<(), String> {
	let keys = jwks
		.get("keys")
		.and_then(Value::as_array)
		.filter(|keys| !keys.is_empty())
		.ok_or("give the app a JWK set: {\"keys\": [...]}, with at least one key")?;

	if keys.len() > 10 {
		return Err("give the app at most 10 keys".into());
	}

	for key in keys {
		if PublicKey::from_jwk(key).is_none() {
			return Err(
				"each key must be an ES256 (P-256) or RS256 (at least 2048 bits) public key for \
				 signing"
					.into(),
			);
		}

		if ["d", "p", "q", "dp", "dq", "qi", "k"]
			.iter()
			.any(|private| key.get(*private).is_some())
		{
			return Err("register only public keys: that one includes its private half".into());
		}
	}

	Ok(())
}

/// A JWT's claims, before its signature is checked: only to learn which
/// client to check it against.
fn unverified_claims(token: &str) -> Option<Value> {
	let payload = token.split('.').nth(1)?;
	serde_json::from_slice(&crate::secrets::from_base64url(payload)?).ok()
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn jwk_sets_hold_public_signing_keys() {
		let key = p256::ecdsa::SigningKey::from_slice(&[4; 32]).unwrap();
		let (x, y) = jwt::coordinates(key.verifying_key());
		let public = json!({ "kty": "EC", "crv": "P-256", "x": x, "y": y });

		assert!(check_jwks(&json!({ "keys": [public] })).is_ok());
		assert!(check_jwks(&json!({ "keys": [] })).is_err());
		assert!(check_jwks(&json!([public])).is_err());
		assert!(check_jwks(&json!({ "keys": [{ "kty": "oct", "k": "abc" }] })).is_err());
		let mut private = public.clone();
		private["d"] = json!("secret");
		assert!(
			check_jwks(&json!({ "keys": [private] }))
				.unwrap_err()
				.contains("private half")
		);
		assert!(check_jwks(&json!({ "keys": vec![public; 11] })).is_err());

		assert_eq!(unverified_claims("a.e30.c"), Some(json!({})));
		assert_eq!(unverified_claims("nonsense"), None);
	}
}
