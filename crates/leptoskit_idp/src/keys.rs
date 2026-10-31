//! Signing keys: created when needed, rotated on a schedule, and published
//! until every token they signed has expired.
//!
//! The database keeps only a key's public half. An ES256 key's private half
//! is derived from one of the server's `AUTH_ENCRYPTION_KEYS` and the key's
//! ID whenever it's needed; an RS256 key is the RSA key the server was
//! given. Either way, a copy of the database can't sign anything.

use std::sync::Arc;

use leptoskit_db::DbError;
use p256::ecdsa::SigningKey as EcKey;
use serde_json::Value;

use crate::Idp;
use crate::IdpError;
use crate::NewSigningKey;
use crate::Result;
use crate::SigningAlgorithm;
use crate::SigningKey;
use crate::jwt;
use crate::jwt::PrivateKey;
use crate::jwt::PublicKey;
use crate::jwt::Signer;
use crate::secrets::base64url;
use crate::secrets::from_base64url;
use crate::secrets::identifier;

/// The margin a retired key stays published past the last token it could
/// have signed, for clocks that disagree.
const PUBLISHED_MARGIN: chrono::TimeDelta = chrono::TimeDelta::hours(1);

impl Idp {
	/// The key that signs new tokens with `algorithm`, and its ID.
	///
	/// For ES256, the newest key still in service whose server key is still
	/// configured, or a new one. For RS256, the configured RSA key, published
	/// on first use.
	pub(crate) async fn signer(&self, algorithm: SigningAlgorithm) -> Result<Signer> {
		match algorithm {
			SigningAlgorithm::Es256 => self.ec_signer().await,
			SigningAlgorithm::Rs256 => self.rsa_signer().await,
		}
	}

	async fn ec_signer(&self) -> Result<Signer> {
		let now = leptoskit_db::now();
		let candidates = SigningKey::query()
			.filter(|key| key.retires_at.gt(now) & key.algorithm.eq(SigningAlgorithm::Es256))
			.order_by(|key| key.created_at.desc())
			.all(&self.inner.db)
			.await?;

		for key in candidates {
			let Some(derived_from) = &key.derived_from else {
				continue;
			};
			if let Some(private) = self.private_key(derived_from, &key.kid) {
				// A server key replaced under the same ID would derive another
				// key; skip it rather than sign what JWKS can't verify.
				let (x, y) = jwt::coordinates(private.verifying_key());
				if key.x.as_deref() == Some(x.as_str()) && key.y.as_deref() == Some(y.as_str()) {
					return Ok(Signer {
						kid: key.kid,
						key: PrivateKey::Ec(Box::new(private)),
					});
				}
			}
		}

		self.create_signing_key().await
	}

	async fn create_signing_key(&self) -> Result<Signer> {
		let keys = self.inner.auth.keys();
		let derived_from = keys.newest().to_owned();
		let (retires_at, expires_at) = self.key_schedule()?;

		// All but one in 2³² IDs derive a valid key; another ID fixes the rest.
		for _ in 0..4 {
			let kid = identifier()?;
			let Some(private) = self.private_key(&derived_from, &kid) else {
				continue;
			};
			let (x, y) = jwt::coordinates(private.verifying_key());
			let inserted = NewSigningKey {
				kid: kid.clone(),
				algorithm: SigningAlgorithm::Es256,
				derived_from: Some(derived_from.clone()),
				x: Some(x),
				y: Some(y),
				n: None,
				e: None,
				retires_at,
				expires_at,
			}
			.insert(&self.inner.db)
			.await;

			match inserted {
				Ok(_) => {
					return Ok(Signer {
						kid,
						key: PrivateKey::Ec(Box::new(private)),
					});
				}
				// Another server took the same ID: try another.
				Err(DbError::Conflict { .. }) => {}
				Err(error) => return Err(error.into()),
			}
		}

		Err(IdpError::Config("couldn't create a signing key".into()))
	}

	/// When a key that starts signing now retires, and when it leaves the
	/// published key set.
	fn key_schedule(&self) -> Result<(leptoskit_db::Timestamp, leptoskit_db::Timestamp)> {
		let lifetimes = &self.inner.lifetimes;
		let retires_at = leptoskit_db::now() + Self::lifetime(lifetimes.signing_key)?;
		let longest = lifetimes.access.max(lifetimes.id_token);
		let expires_at = retires_at + Self::lifetime(longest)? + PUBLISHED_MARGIN;
		Ok((retires_at, expires_at))
	}

	/// The RSA key, published under its thumbprint. Its row is renewed while
	/// it signs, so it stays in the key set; once the server is given another
	/// key, the old one stops being renewed and leaves the set after the
	/// last token it signed has expired.
	async fn rsa_signer(&self) -> Result<Signer> {
		let Some(key) = &self.inner.rsa_key else {
			return Err(IdpError::Config(
				"an app asked for RS256 ID tokens, but the provider has no RSA key".into(),
			));
		};
		let PublicKey::Rsa { n, e } = jwt::rsa_public(key) else {
			unreachable!("an RSA key pair has an RSA public key");
		};
		let kid = jwt::rsa_thumbprint(&n, &e);
		let (retires_at, expires_at) = self.key_schedule()?;
		let renew_after = retires_at - Self::lifetime(self.inner.lifetimes.signing_key / 2)?;
		let stored = SigningKey::query()
			.filter(|row| row.kid.eq(kid.clone()))
			.first(&self.inner.db)
			.await?;

		match stored {
			Some(row) if row.retires_at < renew_after => {
				SigningKey::update()
					.filter(|candidate| candidate.id.eq(row.id))
					.set(|candidate| candidate.retires_at, retires_at)
					.set(|candidate| candidate.expires_at, expires_at)
					.run(&self.inner.db)
					.await?;
				self.forget_cached_key(&kid);
			}
			Some(_) => {}
			None => {
				let inserted = NewSigningKey {
					kid: kid.clone(),
					algorithm: SigningAlgorithm::Rs256,
					derived_from: None,
					x: None,
					y: None,
					n: Some(base64url(&n)),
					e: Some(base64url(&e)),
					retires_at,
					expires_at,
				}
				.insert(&self.inner.db)
				.await;

				// Another server published it first.
				if let Err(error) = inserted
					&& !matches!(error, DbError::Conflict { .. })
				{
					return Err(error.into());
				}
			}
		}

		Ok(Signer {
			kid,
			key: PrivateKey::Rsa(Arc::clone(key)),
		})
	}

	/// The private key for `kid`, derived from the server key `derived_from`,
	/// if that key is still configured.
	fn private_key(&self, derived_from: &str, kid: &str) -> Option<EcKey> {
		let purpose = [b"leptoskit_idp signing key ".as_slice(), kid.as_bytes()].concat();
		let secret = self.inner.auth.keys().derive(derived_from, &purpose)?;
		jwt::signing_key(&secret)
	}

	fn forget_cached_key(&self, kid: &str) {
		self.inner
			.verifying_keys
			.lock()
			.expect("the key cache is never poisoned")
			.remove(kid);
	}

	/// The published public key `kid`, if it's still in the key set.
	pub(crate) async fn verifying_key(&self, kid: &str) -> Result<Option<PublicKey>> {
		let now = leptoskit_db::now();
		let cached = self
			.inner
			.verifying_keys
			.lock()
			.expect("the key cache is never poisoned")
			.get(kid)
			.cloned();

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
		let Some(key) = public_key(&stored) else {
			return Ok(None);
		};

		self.inner
			.verifying_keys
			.lock()
			.expect("the key cache is never poisoned")
			.insert(stored.kid, (key.clone(), stored.expires_at));
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
			.filter_map(|key| public_key(key).map(|public| jwt::jwk(&key.kid, &public)))
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
		let Some(kid) = &unverified.kid else {
			return Ok(None);
		};
		let Some(key) = self.verifying_key(kid).await? else {
			return Ok(None);
		};
		Ok(unverified.verify(&key, typ))
	}
}

/// A stored key's public half, if its columns hold one.
fn public_key(key: &SigningKey) -> Option<PublicKey> {
	match key.algorithm {
		SigningAlgorithm::Es256 => {
			jwt::verifying_key(key.x.as_deref()?, key.y.as_deref()?).map(PublicKey::Ec)
		}
		SigningAlgorithm::Rs256 => {
			Some(PublicKey::Rsa {
				n: from_base64url(key.n.as_deref()?)?,
				e: from_base64url(key.e.as_deref()?)?,
			})
		}
	}
}
