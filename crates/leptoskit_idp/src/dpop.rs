//! DPoP (RFC 9449): an app proves, with every request, that it holds the
//! private key its tokens were bound to. A stolen access or refresh token is
//! then useless on its own.
//!
//! A proof is a short JWT the app signs for one request: its header carries
//! the public key, and its claims name the method (`htm`), the URL without
//! its query (`htu`), when it was made (`iat`), a unique `jti`, and, sent with
//! an access token, the token's hash (`ath`). Each works once.
//!
//! Server-provided nonces (`DPoP-Nonce`) aren't used: freshness rests on
//! `iat` and on remembering every `jti` until its proof is too old anyway.

use leptoskit_db::DbError;
use serde_json::Value;
use subtle::ConstantTimeEq;

use crate::DpopProof;
use crate::Idp;
use crate::IdpError;
use crate::NewDpopProof;
use crate::Result;
use crate::jwt;
use crate::jwt::PublicKey;
use crate::secrets;

/// The algorithms proofs may be signed with, as discovery lists them.
pub(crate) const ALGORITHMS: [&str; 2] = ["ES256", "RS256"];

/// How far a proof's `iat` may be from this server's clock, either way.
const SKEW: i64 = 60;

/// The longest `jti` kept.
const MAX_JTI: usize = 256;

/// What a proof must match: the request it came with.
pub(crate) struct Expected<'a> {
	/// The request's method, such as `POST`.
	pub method: &'a str,
	/// The URL the app sent the request to, as this server's issuer names
	/// it.
	pub url: &'a str,
	/// The access token sent with it, which `ath` must hash.
	pub access_token: Option<&'a str>,
}

impl Idp {
	/// Whether the provider takes DPoP proofs and binds tokens with them.
	pub(crate) fn takes_dpop(&self) -> bool {
		self.inner.dpop
	}

	/// The URL of `path` on this provider, as apps send requests to it.
	pub(crate) fn url_of(&self, path: &str) -> String {
		format!("{}{path}", self.inner.issuer)
	}

	/// Checks a DPoP proof against the request it came with, spends it, and
	/// returns the RFC 7638 thumbprint of the key it was signed with.
	///
	/// # Errors
	///
	/// `invalid_dpop_proof` when the proof is malformed, signed with
	/// anything but the key it carries, for another request, too old or
	/// too new, or used before; or when the database fails.
	pub(crate) async fn check_proof(&self, proof: &str, expected: &Expected<'_>) -> Result<String> {
		let unverified = jwt::parse(proof).ok_or_else(|| invalid("send a signed JWT"))?;
		let jwk = unverified
			.jwk
			.as_ref()
			.ok_or_else(|| invalid("its header must carry the public key, as `jwk`"))?;

		if ["d", "p", "q", "dp", "dq", "qi", "k"]
			.iter()
			.any(|private| jwk.get(*private).is_some())
		{
			return Err(invalid("its `jwk` must be a public key"));
		}

		let key = PublicKey::from_jwk(jwk)
			.ok_or_else(|| invalid("its key must be an ES256 or RS256 public key for signing"))?;
		let claims = unverified
			.verify_proof(&key)
			.ok_or_else(|| invalid("it must be a `dpop+jwt` signed by the key it carries"))?;
		let text = |name: &str| claims.get(name).and_then(Value::as_str);

		if text("htm") != Some(expected.method) {
			return Err(invalid("its `htm` must be this request's method"));
		}
		if !text("htu").is_some_and(|url| same_url(url, expected.url)) {
			return Err(invalid("its `htu` must be this request's URL"));
		}

		let now = leptoskit_db::now().timestamp();
		#[allow(clippy::cast_possible_truncation)]
		let issued = claims
			.get("iat")
			.and_then(Value::as_f64)
			.filter(|iat| iat.is_finite())
			.map(|iat| iat.floor() as i64)
			.ok_or_else(|| invalid("it must say when it was made, as `iat`"))?;

		if (issued - now).abs() > SKEW {
			return Err(invalid(
				"it must be made just now: check the app's clock, and make one per request",
			));
		}

		if let Some(token) = expected.access_token {
			let hash = secrets::base64url(&sha2_digest(token.as_bytes()));
			let matches =
				text("ath").is_some_and(|ath| bool::from(ath.as_bytes().ct_eq(hash.as_bytes())));

			if !matches {
				return Err(invalid(
					"its `ath` must be the hash of the access token it comes with",
				));
			}
		}

		let jti = text("jti")
			.filter(|jti| !jti.is_empty() && jti.len() <= MAX_JTI)
			.ok_or_else(|| invalid("it must have a unique `jti`"))?;
		let thumbprint = jwt::thumbprint(&key);
		self.spend_proof(&thumbprint, jti, issued).await?;

		Ok(thumbprint)
	}

	/// Records a proof as used, refusing one seen before. Proofs too old to
	/// be accepted are forgotten.
	async fn spend_proof(&self, thumbprint: &str, jti: &str, issued: i64) -> Result<()> {
		let now = leptoskit_db::now();
		DpopProof::query()
			.filter(|row| row.expires_at.lt(now))
			.delete(&self.inner.db)
			.await?;

		let expires_at = chrono::DateTime::from_timestamp(issued + 2 * SKEW, 0)
			.ok_or_else(|| invalid("its `iat` is out of range"))?;
		let inserted = NewDpopProof {
			jti_hash: secrets::digest(&[b"dpop", thumbprint.as_bytes(), jti.as_bytes()]),
			expires_at,
		}
		.insert(&self.inner.db)
		.await;

		match inserted {
			Ok(_) => Ok(()),
			Err(DbError::Conflict { .. }) => Err(invalid("that proof was already used")),
			Err(error) => Err(error.into()),
		}
	}
}

/// `invalid_dpop_proof`, saying what's wrong with the proof.
pub(crate) fn invalid(description: &str) -> IdpError {
	IdpError::oauth(
		"invalid_dpop_proof",
		format!("the DPoP proof: {description}"),
	)
}

/// Whether two URLs are the same resource for DPoP: the same scheme, host,
/// port and path, once normalized, whatever their queries and fragments
/// (RFC 9449 §4.3).
fn same_url(presented: &str, expected: &str) -> bool {
	let (Ok(presented), Ok(expected)) = (url::Url::parse(presented), url::Url::parse(expected))
	else {
		return false;
	};

	presented.scheme() == expected.scheme()
		&& presented.host_str() == expected.host_str()
		&& presented.port_or_known_default() == expected.port_or_known_default()
		&& presented.path() == expected.path()
}

fn sha2_digest(bytes: &[u8]) -> [u8; 32] {
	use sha2::Digest;

	sha2::Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn urls_match_without_their_query_and_with_their_default_port() {
		let token = "https://notes.example/oauth/token";

		assert!(same_url(token, token));
		assert!(same_url(
			"https://NOTES.example:443/oauth/token?x=1#y",
			token
		));
		assert!(!same_url("https://notes.example/oauth/token/", token));
		assert!(!same_url("http://notes.example/oauth/token", token));
		assert!(!same_url("https://notes.example:8443/oauth/token", token));
		assert!(!same_url("https://other.example/oauth/token", token));
		assert!(!same_url("/oauth/token", token));
	}

	#[test]
	fn errors_name_the_proof() {
		let error = invalid("its `jti` was used");
		assert_eq!(error.code(), "invalid_dpop_proof");
		assert!(error.description().starts_with("the DPoP proof: "));
	}
}
