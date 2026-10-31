//! Tokens: trading codes and refresh tokens, checking access tokens, and
//! what `userinfo`, revocation and introspection say about them.

use std::fmt;

use leptoskit_auth::Account;
use leptoskit_auth::EmailAddress;
use leptoskit_auth::SignedIn;
use leptoskit_db::Connection;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Timestamp;
use leptoskit_db::Tx;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;

use crate::Authorization;
use crate::Client;
use crate::ClientAuth;
use crate::ClientCredentials;
use crate::Family;
use crate::Grant;
use crate::Idp;
use crate::IdpError;
use crate::NewFamily;
use crate::NewRefreshToken;
use crate::RefreshToken;
use crate::Result;
use crate::TokenResponse;
use crate::idp::ClaimSet;
use crate::idp::GrantedScopes;
use crate::jwt;
use crate::jwt::ACCESS_TOKEN_TYPE;
use crate::jwt::ID_TOKEN_TYPE;
use crate::secrets;

/// The form of `POST /oauth/token`. Its `Debug` leaves out the code, the
/// verifier, the refresh token and the secret.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
pub struct TokenRequest {
	/// `authorization_code` or `refresh_token`.
	#[serde(default)]
	pub grant_type: Option<String>,
	/// The authorization code.
	#[serde(default)]
	pub code: Option<String>,
	/// The redirect URI the code was sent to.
	#[serde(default)]
	pub redirect_uri: Option<String>,
	/// The PKCE verifier.
	#[serde(default)]
	pub code_verifier: Option<String>,
	/// The refresh token to trade.
	#[serde(default)]
	pub refresh_token: Option<String>,
	/// Fewer scopes than the refresh token's family carries.
	#[serde(default)]
	pub scope: Option<String>,
	/// The app, when it authenticates in the body or is public.
	#[serde(default)]
	pub client_id: Option<String>,
	/// The app's secret, when it authenticates in the body.
	#[serde(default)]
	pub client_secret: Option<String>,
}

impl fmt::Debug for TokenRequest {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("TokenRequest")
			.field("grant_type", &self.grant_type)
			.field("client_id", &self.client_id)
			.field("scope", &self.scope)
			.finish_non_exhaustive()
	}
}

/// A checked access token: who it acts for, which app holds it, and what it
/// may do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessToken {
	/// The person who granted access.
	pub account: Id<Account>,
	/// The app holding the token.
	pub client_id: String,
	/// The `sub` that app sees.
	pub subject: String,
	/// What the token may do.
	pub scopes: Vec<String>,
	/// When the person last proved who they are.
	pub auth_time: Timestamp,
	/// When it was issued.
	pub issued_at: Timestamp,
	/// When it expires.
	pub expires_at: Timestamp,
	/// The refresh-token family it belongs to.
	pub family: Id<Family>,
}

impl AccessToken {
	/// The person, as policies see them during a call this token makes. It
	/// carries no session of its own.
	pub fn signed_in(&self) -> SignedIn {
		SignedIn {
			account: self.account,
			session: Id::from_uuid(uuid::Uuid::nil()),
			aal: leptoskit_auth::Aal::One,
			authenticated_at: self.auth_time,
			mfa_at: None,
		}
	}
}

/// What `POST /oauth/introspect` says about a token (RFC 7662).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Introspection {
	/// Whether the token works right now. Nothing else is said when it
	/// doesn't.
	pub active: bool,
	/// Its scopes, space-separated.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scope: Option<String>,
	/// The app holding it.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub client_id: Option<String>,
	/// The `sub` that app sees.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub sub: Option<String>,
	/// `Bearer` for an access token, `refresh_token` for a refresh token.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub token_type: Option<String>,
	/// When it expires, in seconds since the epoch.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub exp: Option<i64>,
	/// When it was issued.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub iat: Option<i64>,
	/// The issuer.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub iss: Option<String>,
	/// The audience of an access token.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub aud: Option<String>,
}

impl Idp {
	/// Answers `POST /oauth/token`: trades an authorization code, or a
	/// refresh token, for new tokens.
	///
	/// # Errors
	///
	/// [`IdpError::OAuth`] with `invalid_client` when the app doesn't
	/// authenticate as it registered; `invalid_grant` for a code or refresh
	/// token that's unknown, someone else's, expired, or already used (which
	/// also ends every token it led to); `invalid_request`,
	/// `invalid_scope` or `unsupported_grant_type` for a malformed request;
	/// or when the database fails.
	pub async fn token(
		&self,
		request: &TokenRequest,
		credentials: &ClientCredentials,
	) -> Result<TokenResponse> {
		let client = self.authenticate(credentials).await?;

		match request.grant_type.as_deref() {
			Some("authorization_code") => self.exchange_code(request, &client).await,
			Some("refresh_token") => self.exchange_refresh(request, &client).await,
			Some(_) => {
				Err(IdpError::oauth(
					"unsupported_grant_type",
					"only authorization_code and refresh_token are supported",
				))
			}
			None => Err(IdpError::invalid_request("grant_type is missing")),
		}
	}

	async fn exchange_code(
		&self,
		request: &TokenRequest,
		client: &Client,
	) -> Result<TokenResponse> {
		let code = required(request.code.as_ref(), "code")?;
		let verifier = required(request.code_verifier.as_ref(), "code_verifier")?;
		let redirect_uri = required(request.redirect_uri.as_ref(), "redirect_uri")?;
		let authorization = Authorization::query()
			.filter(|row| row.code_hash.eq(secrets::hash(code)))
			.first(&self.inner.db)
			.await?
			.filter(|row| row.client_id == client.id)
			.ok_or_else(|| IdpError::invalid_grant("that code isn't valid"))?;

		if authorization.used_at.is_some() {
			// A code works once. Whoever has it now may have stolen it, so the
			// tokens it led to end too (RFC 6749 §4.1.2).
			if let Some(family) = authorization.family_id {
				revoke_family(&self.inner.db, family).await?;
			}
			return Err(IdpError::invalid_grant("that code was already used"));
		}
		if authorization.expires_at <= leptoskit_db::now() {
			return Err(IdpError::invalid_grant("that code has expired"));
		}
		if authorization.redirect_uri != redirect_uri {
			return Err(IdpError::invalid_grant(
				"redirect_uri must be the one the code was sent to",
			));
		}
		if !secrets::pkce_matches(verifier, &authorization.code_challenge) {
			return Err(IdpError::invalid_grant(
				"code_verifier doesn't match the challenge",
			));
		}

		// Signing may create a key, on a connection of its own, so it happens
		// before the transaction holds one: SQLite in memory has only one.
		let signer = self.signer().await?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			let now = leptoskit_db::now();
			let used = Authorization::update()
				.filter(|row| row.id.eq(authorization.id) & row.used_at.is_null())
				.set(|row| row.used_at, Some(now))
				.run(&tx)
				.await?;

			if used == 0 {
				return Err(IdpError::invalid_grant("that code was already used"));
			}
			if !usable(&tx, authorization.account_id).await? {
				return Err(IdpError::invalid_grant("that account can't sign in"));
			}

			let grant = Grant::query()
				.filter(|grant| {
					grant.account_id.eq(authorization.account_id) & grant.client_id.eq(client.id)
				})
				.first(&tx)
				.await?
				.ok_or_else(|| IdpError::invalid_grant("the person disconnected this app"))?;
			let family = NewFamily {
				grant_id: grant.id,
				account_id: authorization.account_id,
				client_id: client.id,
				scopes: authorization.scopes.clone(),
				auth_time: authorization.auth_time,
				expires_at: now + Self::lifetime(self.inner.lifetimes.refresh)?,
				revoked_at: None,
			}
			.insert(&tx)
			.await?;
			Authorization::update()
				.filter(|row| row.id.eq(authorization.id))
				.set(|row| row.family_id, Some(family.id))
				.run(&tx)
				.await?;

			self.issue(
				&tx,
				&signer,
				client,
				&family,
				&family.scopes,
				authorization.nonce.as_deref(),
			)
			.await
		}
		.await;

		finish(tx, result).await
	}

	async fn exchange_refresh(
		&self,
		request: &TokenRequest,
		client: &Client,
	) -> Result<TokenResponse> {
		let presented = required(request.refresh_token.as_ref(), "refresh_token")?;
		let invalid = || IdpError::invalid_grant("that refresh token isn't valid");
		let token = RefreshToken::query()
			.filter(|row| row.token_hash.eq(secrets::hash(presented)))
			.first(&self.inner.db)
			.await?
			.ok_or_else(invalid)?;
		let family = Family::find(&self.inner.db, token.family_id)
			.await?
			.filter(|family| family.client_id == client.id)
			.ok_or_else(invalid)?;

		if token.used_at.is_some() {
			// Each refresh token works once. Seen again, someone else holds a
			// copy, and the whole family ends.
			revoke_family(&self.inner.db, family.id).await?;
			return Err(IdpError::invalid_grant(
				"that refresh token was already used; sign in again",
			));
		}

		let now = leptoskit_db::now();
		if family.revoked_at.is_some() || family.expires_at <= now {
			return Err(invalid());
		}

		let scopes = match request.scope.as_deref() {
			None => family.scopes.0.clone(),
			Some(asked) => {
				let asked: Vec<String> = asked
					.split(' ')
					.filter(|scope| !scope.is_empty())
					.map(str::to_owned)
					.collect();
				if asked.is_empty() || asked.iter().any(|scope| !family.scopes.contains(scope)) {
					return Err(IdpError::invalid_scope(
						"a refresh can only ask for scopes already granted",
					));
				}
				asked
			}
		};

		let signer = self.signer().await?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			let used = RefreshToken::update()
				.filter(|row| row.id.eq(token.id) & row.used_at.is_null())
				.set(|row| row.used_at, Some(now))
				.run(&tx)
				.await?;

			if used == 0 {
				// Another request traded it first: one of the two is a copy.
				revoke_family(&tx, family.id).await?;
				return Ok(None);
			}
			if !usable(&tx, family.account_id).await? {
				return Err(IdpError::invalid_grant("that account can't sign in"));
			}

			Grant::update()
				.filter(|row| row.id.eq(family.grant_id))
				.set(|row| row.last_used_at, Some(now))
				.run(&tx)
				.await?;
			self.issue(&tx, &signer, client, &family, &Json(scopes.clone()), None)
				.await
				.map(Some)
		}
		.await;

		// A family ended by a race stays ended, so commit before refusing.
		finish(tx, result).await?.ok_or_else(|| {
			IdpError::invalid_grant("that refresh token was already used; sign in again")
		})
	}

	/// New tokens for `family`: an access token, a refresh token, and with
	/// `openid` an ID token.
	async fn issue(
		&self,
		tx: &Tx,
		(kid, key): &(String, p256::ecdsa::SigningKey),
		client: &Client,
		family: &Family,
		scopes: &Json<Vec<String>>,
		nonce: Option<&str>,
	) -> Result<TokenResponse> {
		let now = leptoskit_db::now();
		let refresh = secrets::token()?;
		NewRefreshToken {
			family_id: family.id,
			token_hash: secrets::hash(&refresh),
			used_at: None,
		}
		.insert(tx)
		.await?;

		let subject = subject(client, family.account_id);
		let access_lifetime = Self::lifetime(self.inner.lifetimes.access)?;
		let scope = scopes.join(" ");
		let access_token = jwt::sign(
			kid,
			key,
			ACCESS_TOKEN_TYPE,
			&json!({
				"iss": self.inner.issuer,
				"sub": subject,
				"aud": self.inner.audience,
				"client_id": client.client_id,
				"iat": now.timestamp(),
				"exp": (now + access_lifetime).timestamp(),
				"jti": secrets::identifier()?,
				"scope": scope,
				"auth_time": family.auth_time.timestamp(),
				"sid": family.id.to_string(),
			}),
		);
		let id_token = if scopes.iter().any(|scope| scope == "openid") {
			let mut claims = json!({
				"iss": self.inner.issuer,
				"sub": subject,
				"aud": client.client_id,
				"iat": now.timestamp(),
				"exp": (now + Self::lifetime(self.inner.lifetimes.id_token)?).timestamp(),
				"auth_time": family.auth_time.timestamp(),
				"sid": family.id.to_string(),
			});
			if let Some(nonce) = nonce {
				claims["nonce"] = Value::from(nonce);
			}
			Some(jwt::sign(kid, key, ID_TOKEN_TYPE, &claims))
		} else {
			None
		};

		Ok(TokenResponse {
			access_token,
			token_type: "Bearer".into(),
			expires_in: u64::try_from(access_lifetime.num_seconds()).unwrap_or_default(),
			refresh_token: refresh,
			scope,
			id_token,
		})
	}

	/// Checks an access token this provider issued: its signature, type,
	/// issuer, audience and expiry, and that its family is live and its
	/// person can still sign in. `None` for any token that fails.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn verify_access_token(&self, token: &str) -> Result<Option<AccessToken>> {
		let Some(claims) = self.verified_claims(token, ACCESS_TOKEN_TYPE).await? else {
			return Ok(None);
		};
		let now = leptoskit_db::now().timestamp();
		let text = |name: &str| claims.get(name).and_then(Value::as_str);
		let time = |name: &str| {
			claims
				.get(name)
				.and_then(Value::as_i64)
				.and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
		};
		let (Some(exp), Some(iat), Some(auth_time)) = (time("exp"), time("iat"), time("auth_time"))
		else {
			return Ok(None);
		};

		if text("iss") != Some(self.inner.issuer.as_str())
			|| text("aud") != Some(self.inner.audience.as_str())
			|| exp.timestamp() <= now
		{
			return Ok(None);
		}

		let Some(family) = text("sid").and_then(|sid| Id::<Family>::parse(sid).ok()) else {
			return Ok(None);
		};
		let Some(family) = Family::find(&self.inner.db, family).await? else {
			return Ok(None);
		};
		let Some(client) = Client::find(&self.inner.db, family.client_id).await? else {
			return Ok(None);
		};

		if family.revoked_at.is_some()
			|| text("client_id") != Some(client.client_id.as_str())
			|| !usable(&self.inner.db, family.account_id).await?
		{
			return Ok(None);
		}

		Ok(Some(AccessToken {
			account: family.account_id,
			client_id: client.client_id,
			subject: text("sub").unwrap_or_default().to_owned(),
			scopes: text("scope")
				.unwrap_or_default()
				.split(' ')
				.filter(|scope| !scope.is_empty())
				.map(str::to_owned)
				.collect(),
			auth_time,
			issued_at: iat,
			expires_at: exp,
			family: family.id,
		}))
	}

	/// What `userinfo` says about the person behind `token`: their `sub`,
	/// their verified primary email with the `email` scope, and whatever
	/// the app's [`Claims`](crate::Claims) adds.
	///
	/// # Errors
	///
	/// [`IdpError::OAuth`] (`insufficient_scope`) without the `openid`
	/// scope, or when the database or the app's claims fail.
	pub async fn userinfo(&self, token: &AccessToken) -> Result<Map<String, Value>> {
		if !token.scopes.iter().any(|scope| scope == "openid") {
			return Err(IdpError::oauth(
				"insufficient_scope",
				"userinfo needs the openid scope",
			));
		}

		let scopes = GrantedScopes::new(token.scopes.clone());
		let mut claims = ClaimSet::default();
		let db = self
			.inner
			.db
			.with_context(leptoskit_auth::context(Some(token.signed_in())));
		(self.inner.claims)(&db, token.account, &scopes, &mut claims).await?;

		let mut out: Map<String, Value> = claims.into_inner().into_iter().collect();
		out.insert("sub".into(), Value::from(token.subject.clone()));

		if scopes.has("email") {
			let email = EmailAddress::query()
				.filter(|email| {
					email.account_id.eq(token.account)
						& email.is_primary.eq(true)
						& email.verified_at.is_not_null()
				})
				.first(&self.inner.db)
				.await?;
			if let Some(email) = email {
				out.insert("email".into(), Value::from(email.address.to_string()));
				out.insert("email_verified".into(), Value::from(true));
			}
		}

		Ok(out)
	}

	/// Answers `POST /oauth/revoke` (RFC 7009): ends the refresh-token
	/// family `token` belongs to, if it's the calling app's. Unknown tokens
	/// are already revoked, so they succeed too.
	///
	/// # Errors
	///
	/// `invalid_client` when the app doesn't authenticate, or when the
	/// database fails.
	pub async fn revoke_token(&self, credentials: &ClientCredentials, token: &str) -> Result<()> {
		let client = self.authenticate(credentials).await?;

		if let Some(family) = self.family_of(token, true).await?
			&& family.client_id == client.id
		{
			revoke_family(&self.inner.db, family.id).await?;
		}

		Ok(())
	}

	/// Answers `POST /oauth/introspect` (RFC 7662) for a confidential app:
	/// what `token` is, if it's live and the app's own, or the app may
	/// introspect any token.
	///
	/// # Errors
	///
	/// `invalid_client` when the app doesn't authenticate or is public, or
	/// when the database fails.
	pub async fn introspect(
		&self,
		credentials: &ClientCredentials,
		token: &str,
	) -> Result<Introspection> {
		let caller = self.authenticate(credentials).await?;

		if caller.auth_method == ClientAuth::None {
			return Err(IdpError::invalid_client());
		}

		let may_see = |client: &Client| caller.introspection || client.id == caller.id;

		if let Some(access) = self.verify_access_token(token).await? {
			let Some(client) = self.client(&access.client_id).await? else {
				return Ok(Introspection::default());
			};
			if !may_see(&client) {
				return Ok(Introspection::default());
			}
			return Ok(Introspection {
				active: true,
				scope: Some(access.scopes.join(" ")),
				client_id: Some(access.client_id),
				sub: Some(access.subject),
				token_type: Some("Bearer".into()),
				exp: Some(access.expires_at.timestamp()),
				iat: Some(access.issued_at.timestamp()),
				iss: Some(self.inner.issuer.clone()),
				aud: Some(self.inner.audience.clone()),
			});
		}

		let now = leptoskit_db::now();
		let refresh = RefreshToken::query()
			.filter(|row| row.token_hash.eq(secrets::hash(token)) & row.used_at.is_null())
			.first(&self.inner.db)
			.await?;
		let Some(refresh) = refresh else {
			return Ok(Introspection::default());
		};
		let Some(family) = Family::find(&self.inner.db, refresh.family_id).await? else {
			return Ok(Introspection::default());
		};
		let Some(client) = Client::find(&self.inner.db, family.client_id).await? else {
			return Ok(Introspection::default());
		};

		if family.revoked_at.is_some()
			|| family.expires_at <= now
			|| !may_see(&client)
			|| !usable(&self.inner.db, family.account_id).await?
		{
			return Ok(Introspection::default());
		}

		Ok(Introspection {
			active: true,
			scope: Some(family.scopes.join(" ")),
			sub: Some(subject(&client, family.account_id)),
			client_id: Some(client.client_id),
			token_type: Some("refresh_token".into()),
			exp: Some(family.expires_at.timestamp()),
			iat: Some(refresh.created_at.timestamp()),
			iss: Some(self.inner.issuer.clone()),
			aud: None,
		})
	}

	/// The family a refresh token or access token belongs to. With
	/// `expired`, an access token that has run out still names its family,
	/// so revoking it ends its refresh tokens too.
	pub(crate) async fn family_of(&self, token: &str, expired: bool) -> Result<Option<Family>> {
		let refresh = RefreshToken::query()
			.filter(|row| row.token_hash.eq(secrets::hash(token)))
			.first(&self.inner.db)
			.await?;

		if let Some(refresh) = refresh {
			return Ok(Family::find(&self.inner.db, refresh.family_id).await?);
		}

		let Some(claims) = self.verified_claims(token, ACCESS_TOKEN_TYPE).await? else {
			return Ok(None);
		};
		let live = claims
			.get("exp")
			.and_then(Value::as_i64)
			.is_some_and(|exp| exp > leptoskit_db::now().timestamp());
		let family = claims
			.get("sid")
			.and_then(Value::as_str)
			.and_then(|sid| Id::<Family>::parse(sid).ok());

		match family {
			Some(family) if live || expired => Ok(Family::find(&self.inner.db, family).await?),
			_ => Ok(None),
		}
	}
}

/// The `sub` `client` sees for `account`: the account ID for one of your own
/// apps, and a pairwise one for everyone else.
pub(crate) fn subject(client: &Client, account: Id<Account>) -> String {
	if client.first_party {
		account.to_string()
	} else {
		secrets::pairwise_subject(&client.subject_salt, account)
	}
}

/// Ends a refresh-token family: its refresh tokens stop working, and so do
/// its access tokens wherever they're checked against it.
pub(crate) async fn revoke_family(connection: &impl Connection, family: Id<Family>) -> Result<()> {
	Family::update()
		.filter(|row| row.id.eq(family) & row.revoked_at.is_null())
		.set(|row| row.revoked_at, Some(leptoskit_db::now()))
		.run(connection)
		.await?;
	Ok(())
}

/// Whether `account` still exists and isn't disabled.
pub(crate) async fn usable(connection: &impl Connection, account: Id<Account>) -> Result<bool> {
	let account = Account::find(connection, account).await?;
	Ok(account.is_some_and(|account| account.disabled_at.is_none()))
}

fn required<'a>(value: Option<&'a String>, name: &str) -> Result<&'a str> {
	value
		.map(String::as_str)
		.filter(|value| !value.is_empty())
		.ok_or_else(|| IdpError::invalid_request(format!("{name} is missing")))
}

/// Ends a transaction: commits it when `result` is `Ok`, and rolls it back
/// when it isn't.
pub(crate) async fn finish<T>(tx: Tx, result: Result<T>) -> Result<T> {
	match result {
		Ok(value) => {
			tx.commit().await?;
			Ok(value)
		}
		Err(error) => {
			tx.rollback().await?;
			Err(error)
		}
	}
}
