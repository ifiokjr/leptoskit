//! Signing keys: created when needed, rotated on a schedule, and published
//! until every token they signed has expired.
//!
//! The database keeps only a key's public half. The private key is derived
//! from one of the server's `AUTH_ENCRYPTION_KEYS` and the key's ID whenever
//! it's needed, so a copy of the database can't sign anything.

use leptoskit_db::DbError;
use p256::ecdsa::SigningKey as PrivateKey;
use p256::ecdsa::VerifyingKey;
use serde_json::Value;

use crate::Idp;
use crate::IdpError;
use crate::NewSigningKey;
use crate::Result;
use crate::SigningKey;
use crate::jwt;
use crate::secrets::identifier;

/// The margin a retired key stays published past the last token it could
/// have signed, for clocks that disagree.
const PUBLISHED_MARGIN: chrono::TimeDelta = chrono::TimeDelta::hours(1);

impl Idp {
	/// The key that signs new tokens, and its ID: the newest one still in
	/// service whose server key is still configured, or a new one.
	pub(crate) async fn signer(&self) -> Result<(String, PrivateKey)> {
		let now = leptoskit_db::now();
		let candidates = SigningKey::query()
			.filter(|key| key.retires_at.gt(now))
			.order_by(|key| key.created_at.desc())
			.all(&self.inner.db)
			.await?;

		for key in candidates {
			if let Some(private) = self.private_key(&key.derived_from, &key.kid) {
				// A server key replaced under the same ID would derive another
				// key; skip it rather than sign what JWKS can't verify.
				if jwt::coordinates(private.verifying_key()) == (key.x.clone(), key.y.clone()) {
					return Ok((key.kid, private));
				}
			}
		}

		self.create_signing_key().await
	}

	async fn create_signing_key(&self) -> Result<(String, PrivateKey)> {
		let keys = self.inner.auth.keys();
		let derived_from = keys.newest().to_owned();
		let lifetimes = &self.inner.lifetimes;
		let now = leptoskit_db::now();
		let retires_at = now + Self::lifetime(lifetimes.signing_key)?;
		let longest = lifetimes.access.max(lifetimes.id_token);
		let expires_at = retires_at + Self::lifetime(longest)? + PUBLISHED_MARGIN;

		// All but one in 2³² IDs derive a valid key; another ID fixes the rest.
		for _ in 0..4 {
			let kid = identifier()?;
			let Some(private) = self.private_key(&derived_from, &kid) else {
				continue;
			};
			let (x, y) = jwt::coordinates(private.verifying_key());
			let inserted = NewSigningKey {
				kid: kid.clone(),
				derived_from: derived_from.clone(),
				x,
				y,
				retires_at,
				expires_at,
			}
			.insert(&self.inner.db)
			.await;

			match inserted {
				Ok(_) => return Ok((kid, private)),
				// Another server took the same ID: try another.
				Err(DbError::Conflict { .. }) => {}
				Err(error) => return Err(error.into()),
			}
		}

		Err(IdpError::Config("couldn't create a signing key".into()))
	}

	/// The private key for `kid`, derived from the server key `derived_from`,
	/// if that key is still configured.
	fn private_key(&self, derived_from: &str, kid: &str) -> Option<PrivateKey> {
		let purpose = [b"leptoskit_idp signing key ".as_slice(), kid.as_bytes()].concat();
		let secret = self.inner.auth.keys().derive(derived_from, &purpose)?;
		jwt::signing_key(&secret)
	}

	/// The published public key `kid`, if it's still in the key set.
	pub(crate) async fn verifying_key(&self, kid: &str) -> Result<Option<VerifyingKey>> {
		let now = leptoskit_db::now();
		let cached = self
			.inner
			.verifying_keys
			.lock()
			.expect("the key cache is never poisoned")
			.get(kid)
			.copied();

		if let Some((key, expires_at)) = cached {
			return Ok((expires_at > now).then_some(key));
		}

		let stored = SigningKey::query()
			.filter(|key| key.kid.eq(kid.to_owned()) & key.expires_at.gt(now))
			.first(&self.inner.db)
			.await?;
		let Some(stored) = stored else {
			return Ok(None);
		};
		let Some(key) = jwt::verifying_key(&stored.x, &stored.y) else {
			return Ok(None);
		};

		self.inner
			.verifying_keys
			.lock()
			.expect("the key cache is never poisoned")
			.insert(stored.kid, (key, stored.expires_at));
		Ok(Some(key))
	}

	/// The published key set, which `/oauth/jwks` serves: every key that
	/// signs, or signed a token that hasn't expired yet.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn jwks(&self) -> Result<Value> {
		let now = leptoskit_db::now();
		let keys = SigningKey::query()
			.filter(|key| key.expires_at.gt(now))
			.order_by(|key| key.created_at.desc())
			.all(&self.inner.db)
			.await?;
		let keys: Vec<Value> = keys
			.iter()
			.map(|key| jwt::jwk(&key.kid, &key.x, &key.y))
			.collect();

		Ok(serde_json::json!({ "keys": keys }))
	}

	/// The claims of `token`, if this provider signed it with a published
	/// key and it's of type `typ`. Expiry and the other claims are the
	/// caller's to check.
	pub(crate) async fn verified_claims(&self, token: &str, typ: &str) -> Result<Option<Value>> {
		let Some(unverified) = jwt::parse(token) else {
			return Ok(None);
		};
		let Some(key) = self.verifying_key(&unverified.kid).await? else {
			return Ok(None);
		};
		Ok(unverified.verify(&key, typ))
	}
}
