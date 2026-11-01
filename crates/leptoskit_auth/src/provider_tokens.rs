//! Providers' own tokens, for apps that call a provider's API on the
//! person's behalf, such as listing their GitHub repositories. Off unless a
//! provider is configured with `store_tokens`.
//!
//! The tokens are sealed with the server's keys and bound to the connected
//! account they came with, so a copy of the database calls no one's API.
//! Nobody reads the table directly; [`Auth::provider_access_token`] opens
//! them, and refreshes the access token when it's about to expire.
//! Disconnecting the account, merging it into another, or deleting the
//! account deletes them, and revokes them at providers that allow it.

use std::fmt;

use leptoskit_db::Connection;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;

use crate::Account;
use crate::Auth;
use crate::EventKind;
use crate::Identity;
use crate::NewProviderToken;
use crate::ProviderToken;
use crate::RequestInfo;
use crate::Result;
use crate::auth::finish;
use crate::providers::ProviderProfile;

/// A provider's access token for one of the person's connected accounts.
/// Its `Debug` leaves the token out.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderAccess {
	/// The connected account it's for.
	pub identity: Id<Identity>,
	/// The provider's ID, such as `github`.
	pub provider: String,
	/// The token, for the provider's API as `Authorization: Bearer`.
	pub access_token: String,
	/// When it stops working, if the provider said.
	pub expires_at: Option<Timestamp>,
	/// The scopes the provider granted, as it listed them.
	pub scope: Option<String>,
}

impl fmt::Debug for ProviderAccess {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("ProviderAccess")
			.field("identity", &self.identity)
			.field("provider", &self.provider)
			.field("expires_at", &self.expires_at)
			.field("scope", &self.scope)
			.finish_non_exhaustive()
	}
}

/// Tokens taken out of the table, to revoke at their provider once the
/// change that removed them has committed.
pub(crate) struct Ended {
	pub provider: String,
	/// The refresh token when there was one, which revokes the whole grant
	/// at most providers, and the access token otherwise.
	pub token: String,
	pub refresh: bool,
}

/// What a token is sealed under: its identity and which token it is, so
/// one can't stand in for another.
pub(crate) fn token_context<'a>(identity: &'a [u8], which: &'a [u8]) -> [&'a [u8]; 3] {
	[b"provider-token", identity, which]
}

impl Auth {
	/// Keeps the tokens `profile` came with for `identity`, replacing any
	/// it had. Nothing changes when there are none: the provider isn't one
	/// whose tokens the app keeps, or the sign-in gave none, such as Sign in
	/// with Apple on the device.
	pub(crate) async fn keep_grant(
		&self,
		connection: &impl Connection,
		identity: &Identity,
		profile: &ProviderProfile,
	) -> Result<()> {
		let Some(grant) = &profile.grant else {
			return Ok(());
		};
		let keys = &self.inner.keys;
		let id = identity.id.uuid();
		let access = keys.seal(
			&token_context(id.as_bytes(), b"access"),
			grant.access_token.as_bytes(),
		)?;
		let refresh = grant
			.refresh_token
			.as_deref()
			.map(|refresh| {
				keys.seal(
					&token_context(id.as_bytes(), b"refresh"),
					refresh.as_bytes(),
				)
			})
			.transpose()?;
		let now = leptoskit_db::now();
		let expires_at = grant
			.expires_in
			.filter(|seconds| *seconds > 0)
			.map(|seconds| now + chrono::TimeDelta::seconds(seconds.min(i64::from(i32::MAX))));
		let scope = grant
			.scope
			.as_deref()
			.map(|scope| scope.chars().take(2048).collect::<String>());
		let changed = ProviderToken::update()
			.filter(|row| row.identity_id.eq(identity.id))
			.set(|row| row.account_id, identity.account_id)
			.set(|row| row.access_token, access.clone())
			.set(|row| row.refresh_token, refresh.clone())
			.set(|row| row.expires_at, expires_at)
			.set(|row| row.scope, scope.clone())
			.set(|row| row.updated_at, now)
			.run(connection)
			.await?;

		if changed == 0 {
			NewProviderToken {
				identity_id: identity.id,
				account_id: identity.account_id,
				provider: identity.provider.clone(),
				access_token: access,
				refresh_token: refresh,
				expires_at,
				scope,
				updated_at: now,
			}
			.insert(connection)
			.await?;
		}

		Ok(())
	}

	/// Deletes the stored tokens of `account`, or only of `identity`, and
	/// returns them opened, to revoke once the change commits. A token
	/// sealed under a key that's no longer listed is deleted without being
	/// revoked.
	pub(crate) async fn take_provider_tokens(
		&self,
		connection: &impl Connection,
		account: Id<Account>,
		identity: Option<Id<Identity>>,
	) -> Result<Vec<Ended>> {
		let rows = ProviderToken::query()
			.filter(|row| {
				let mine = row.account_id.eq(account);

				match identity {
					Some(identity) => mine & row.identity_id.eq(identity),
					None => mine,
				}
			})
			.all(connection)
			.await?;
		let mut ended = Vec::new();

		for row in rows {
			ProviderToken::query()
				.filter(|token| token.id.eq(row.id))
				.delete(connection)
				.await?;
			let id = row.identity_id.uuid();

			let (sealed, which, refresh) = match &row.refresh_token {
				Some(refresh) => (refresh, b"refresh".as_slice(), true),
				None => (&row.access_token, b"access".as_slice(), false),
			};

			let opened = self
				.inner
				.keys
				.open(&token_context(id.as_bytes(), which), sealed)
				.ok()
				.and_then(|token| String::from_utf8(token).ok());

			if let Some(token) = opened {
				ended.push(Ended {
					provider: row.provider,
					token,
					refresh,
				});
			}
		}

		Ok(ended)
	}

	/// Deletes `account` for good: its sign-in methods, sessions, and
	/// history, and the app's own rows that cascade from it, after revoking
	/// the provider tokens it kept where providers allow that (Apple asks
	/// apps to, when someone deletes their account). For an account's owner
	/// deleting it themselves, or an administrator; check that the caller
	/// may, such as with a recent step-up.
	///
	/// # Errors
	///
	/// When the database fails, or the app's rows refuse to go, such as a
	/// foreign key without `on_delete = cascade`; then nothing is deleted.
	pub async fn delete_account(&self, account: Id<Account>, request: &RequestInfo) -> Result<()> {
		let tx = self.inner.db.begin().await?;
		let result = async {
			let ended = self.take_provider_tokens(&tx, account, None).await?;
			Account::query()
				.filter(|row| row.id.eq(account))
				.delete(&tx)
				.await?;
			// The account's own history goes with it; this one entry, which
			// names only its ID, stays.
			let id = account.to_string();
			self.record(&tx, None, EventKind::AccountDeleted, Some(&id), request)
				.await?;
			Ok(ended)
		}
		.await;
		let ended = finish(tx, result).await?;
		self.revoke_provider_tokens(ended).await;
		Ok(())
	}

	/// Revokes `ended` at their providers, best effort: the tokens are
	/// already gone from here, and a provider that's down or doesn't allow
	/// revoking leaves them to expire.
	#[cfg(feature = "oauth")]
	pub(crate) async fn revoke_provider_tokens(&self, ended: Vec<Ended>) {
		for token in ended {
			let Ok(provider) = self.provider(&token.provider) else {
				continue;
			};
			let _ = self
				.inner
				.providers
				.revoke(provider, &token.token, token.refresh)
				.await;
		}
	}

	#[cfg(not(feature = "oauth"))]
	#[allow(
		clippy::unused_async,
		clippy::unused_self,
		clippy::needless_pass_by_value,
		reason = "without providers, nothing was ever stored"
	)]
	pub(crate) async fn revoke_provider_tokens(&self, _ended: Vec<Ended>) {}
}

#[cfg(feature = "oauth")]
impl Auth {
	/// An access token for `provider`'s API on `account`'s behalf: from the
	/// connected account that signed in most recently, refreshed first when
	/// it expires within a minute. `None` when there's nothing to use: the
	/// provider doesn't keep tokens (`store_tokens`), the person hasn't
	/// signed in with it since, the token expired without a refresh token,
	/// or the provider ended the grant, in which case the stored tokens are
	/// deleted. Ask the person to connect the provider again with
	/// `link_provider()`.
	///
	/// ```ignore
	/// let Some(github) = auth.provider_access_token(me.account, "github").await? else {
	/// 	return Err(MyError::ConnectGitHub);
	/// };
	/// client.get("https://api.github.com/user/repos").bearer_auth(&github.access_token)
	/// ```
	///
	/// # Errors
	///
	/// When the database fails, or the provider fails to refresh the token
	/// for any reason but ending the grant (the stored tokens stay, to try
	/// again).
	pub async fn provider_access_token(
		&self,
		account: Id<Account>,
		provider: &str,
	) -> Result<Option<ProviderAccess>> {
		let db = &self.inner.db;
		let Some(row) = latest(db, account, provider).await? else {
			return Ok(None);
		};
		let now = leptoskit_db::now();
		let fresh = row
			.expires_at
			.is_none_or(|at| at - now > chrono::TimeDelta::seconds(60));

		if fresh {
			return self.open_access(&row).map(Some);
		}

		let Some(sealed) = &row.refresh_token else {
			return Ok(None);
		};
		let Ok(definition) = self.provider(&row.provider) else {
			return Ok(None);
		};
		let id = row.identity_id.uuid();
		let refresh = self
			.inner
			.keys
			.open(&token_context(id.as_bytes(), b"refresh"), sealed)?;
		let refresh = String::from_utf8(refresh)
			.map_err(|_| crate::AuthError::Provider("a stored refresh token isn't text".into()))?;
		let user_agent = self.provider_user_agent(definition);
		let Some(grant) = self
			.inner
			.providers
			.refresh(definition, &refresh, user_agent)
			.await?
		else {
			// The provider ended the grant, unless another request already
			// traded this refresh token for a new one.
			let gone = ProviderToken::query()
				.filter(|token| token.id.eq(row.id) & token.updated_at.eq(row.updated_at))
				.delete(db)
				.await?;

			if gone == 1 {
				return Ok(None);
			}

			let row = latest(db, account, provider).await?;

			return row.map(|row| self.open_access(&row)).transpose();
		};
		let identity = Identity::get(db, row.identity_id).await?;
		let grant = crate::providers::ProviderGrant {
			refresh_token: grant.refresh_token.or(Some(refresh)),
			..grant
		};
		let profile = ProviderProfile {
			grant: Some(grant),
			..ProviderProfile::default()
		};
		let tx = db.begin().await?;
		let result = async {
			// Only if nobody refreshed meanwhile; otherwise theirs stands.
			let current = ProviderToken::query()
				.filter(|token| token.id.eq(row.id) & token.updated_at.eq(row.updated_at))
				.count(&tx)
				.await?;

			if current == 1 {
				self.keep_grant(&tx, &identity, &profile).await?;
			}

			Ok(())
		}
		.await;
		finish(tx, result).await?;
		let row = latest(db, account, provider).await?;
		row.map(|row| self.open_access(&row)).transpose()
	}

	fn open_access(&self, row: &ProviderToken) -> Result<ProviderAccess> {
		let id = row.identity_id.uuid();
		let token = self
			.inner
			.keys
			.open(&token_context(id.as_bytes(), b"access"), &row.access_token)?;
		Ok(ProviderAccess {
			identity: row.identity_id,
			provider: row.provider.clone(),
			access_token: String::from_utf8(token).map_err(|_| {
				crate::AuthError::Provider("a stored access token isn't text".into())
			})?,
			expires_at: row.expires_at,
			scope: row.scope.clone(),
		})
	}
}

/// The stored tokens of `account`'s most recently signed-in connected
/// account at `provider`.
#[cfg(feature = "oauth")]
async fn latest(
	connection: &impl Connection,
	account: Id<Account>,
	provider: &str,
) -> Result<Option<ProviderToken>> {
	Ok(ProviderToken::query()
		.filter(|row| row.account_id.eq(account) & row.provider.eq(provider.to_owned()))
		.order_by(|row| row.updated_at.desc())
		.first(connection)
		.await?)
}
