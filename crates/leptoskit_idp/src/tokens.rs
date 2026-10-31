//! Tokens: trading codes, device codes, refresh tokens and an app's own
//! credentials, checking access tokens, and what `userinfo`, revocation and
//! introspection say about them.

use std::fmt;

use leptoskit_auth::Account;
use leptoskit_auth::EmailAddress;
use leptoskit_auth::EventKind;
use leptoskit_auth::RequestInfo;
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
use crate::GrantType;
use crate::Idp;
use crate::IdpError;
use crate::NewFamily;
use crate::NewRefreshToken;
use crate::RefreshToken;
use crate::Result;
use crate::SigningAlgorithm;
use crate::TokenResponse;
use crate::idp::ClaimSet;
use crate::idp::GrantedScopes;
use crate::jwt;
use crate::jwt::ACCESS_TOKEN_TYPE;
use crate::jwt::ID_TOKEN_TYPE;
use crate::jwt::Signer;
use crate::secrets;

/// The scopes about a person, which an app acting as itself can't have.
pub(crate) const PERSON_SCOPES: [&str; 4] = ["openid", "profile", "email", "offline_access"];

/// The form of `POST /oauth/token`. Its `Debug` leaves out the code, the
/// verifier, the device code, the refresh token, the secret and the
/// assertion.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
pub struct TokenRequest {
	/// `authorization_code`, `refresh_token`,
	/// `urn:ietf:params:oauth:grant-type:device_code`, or
	/// `client_credentials`.
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
	/// The device code a CLI or a TV polls with.
	#[serde(default)]
	pub device_code: Option<String>,
	/// The refresh token to trade.
	#[serde(default)]
	pub refresh_token: Option<String>,
	/// Fewer scopes than the refresh token's family carries, or the scopes
	/// an app acting as itself asks for.
	#[serde(default)]
	pub scope: Option<String>,
	/// The app, when it authenticates in the body or is public.
	#[serde(default)]
	pub client_id: Option<String>,
	/// The app's secret, when it authenticates in the body.
	#[serde(default)]
	pub client_secret: Option<String>,
	/// `urn:ietf:params:oauth:client-assertion-type:jwt-bearer`, with
	/// `private_key_jwt`.
	#[serde(default)]
	pub client_assertion_type: Option<String>,
	/// The app's signed assertion, with `private_key_jwt`.
	#[serde(default)]
	pub client_assertion: Option<String>,
	/// The RFC 7638 thumbprint of the key of a DPoP proof the request came
	/// with, already checked: the new access token is bound to it. The
	/// token endpoint sets it from the `DPoP` header; a form can't.
	#[serde(skip)]
	pub dpop_jkt: Option<String>,
}

impl fmt::Debug for TokenRequest {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("TokenRequest")
			.field("grant_type", &self.grant_type)
			.field("client_id", &self.client_id)
			.field("scope", &self.scope)
			.field("dpop_jkt", &self.dpop_jkt)
			.finish_non_exhaustive()
	}
}

/// A checked access token: who it acts for, which app holds it, and what it
/// may do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessToken {
	/// The person who granted access, or `None` for an app acting as itself
	/// (`client_credentials`).
	pub account: Option<Id<Account>>,
	/// The app holding the token.
	pub client_id: String,
	/// The `sub` that app sees: the person's, or for an app acting as itself
	/// its own client ID.
	pub subject: String,
	/// What the token may do.
	pub scopes: Vec<String>,
	/// When the person last proved who they are.
	pub auth_time: Timestamp,
	/// When it was issued.
	pub issued_at: Timestamp,
	/// When it expires.
	pub expires_at: Timestamp,
	/// The family it belongs to.
	pub family: Id<Family>,
	/// The RFC 7638 thumbprint of the DPoP key it's bound to, if it is:
	/// then it works only with a proof signed by that key.
	pub jkt: Option<String>,
}

impl AccessToken {
	/// The person, as policies see them during a call this token makes, or
	/// `None` for an app acting as itself. It carries no session of its own.
	pub fn signed_in(&self) -> Option<SignedIn> {
		Some(SignedIn {
			account: self.account?,
			session: Id::from_uuid(uuid::Uuid::nil()),
			aal: leptoskit_auth::Aal::One,
			authenticated_at: self.auth_time,
			mfa_at: None,
		})
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
	/// The DPoP key a bound access token needs, as `{"jkt": …}` (RFC 9449
	/// §6.2).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cnf: Option<Value>,
}

/// The keys a token response is signed with: access tokens are always
/// ES256, and ID tokens as the app registered.
pub(crate) struct Signers {
	access: Signer,
	id_token: Signer,
}

/// What goes into new tokens besides the family's own details.
#[derive(Default)]
pub(crate) struct Extras<'a> {
	/// Echoed in the ID token.
	pub nonce: Option<&'a str>,
	/// Claims the app asked for in its ID token.
	pub id_token_claims: Map<String, Value>,
	/// Whether to issue a refresh token.
	pub refresh: bool,
	/// The thumbprint of the DPoP key to bind the access token to.
	pub dpop: Option<&'a str>,
}

impl Idp {
	/// Answers `POST /oauth/token`: trades an authorization code, a device
	/// code, or a refresh token for new tokens, or gives an app acting as
	/// itself an access token. `info` says where the request came from, for
	/// the audit trail.
	///
	/// # Errors
	///
	/// [`IdpError::OAuth`] with `invalid_client` when the app doesn't
	/// authenticate as it registered; `unauthorized_client` for a grant the
	/// app didn't register for; `invalid_grant` for a code or refresh token
	/// that's unknown, someone else's, expired, or already used (which also
	/// ends every token it led to); the device flow's
	/// `authorization_pending`, `slow_down`, `access_denied` and
	/// `expired_token`; `invalid_request`, `invalid_scope` or
	/// `unsupported_grant_type` for a malformed request; or when the
	/// database fails.
	pub async fn token(
		&self,
		request: &TokenRequest,
		credentials: &ClientCredentials,
		info: &RequestInfo,
	) -> Result<TokenResponse> {
		let client = self.authenticate(credentials).await?;

		if client.dpop_bound && request.dpop_jkt.is_none() {
			return Err(crate::dpop::invalid(
				"this app's tokens must be bound to its key: send a proof in the `DPoP` header",
			));
		}

		let allowed = |grant: GrantType| {
			if client.grant_types.contains(&grant) {
				Ok(())
			} else {
				Err(IdpError::oauth(
					"unauthorized_client",
					format!("this app isn't registered for {}", grant.as_str()),
				))
			}
		};

		match request.grant_type.as_deref() {
			Some("authorization_code") => {
				allowed(GrantType::AuthorizationCode)?;
				self.exchange_code(request, &client, info).await
			}
			Some("refresh_token") => self.exchange_refresh(request, &client, info).await,
			Some(grant) if grant == GrantType::DeviceCode.as_str() => {
				allowed(GrantType::DeviceCode)?;
				self.exchange_device_code(request, &client, info).await
			}
			Some("client_credentials") => {
				allowed(GrantType::ClientCredentials)?;
				self.exchange_client_credentials(request, &client).await
			}
			Some(_) => {
				Err(IdpError::oauth(
					"unsupported_grant_type",
					"supported: authorization_code, refresh_token, client_credentials, and the \
					 device flow",
				))
			}
			None => Err(IdpError::invalid_request("grant_type is missing")),
		}
	}

	async fn exchange_code(
		&self,
		request: &TokenRequest,
		client: &Client,
		info: &RequestInfo,
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
				self.end_reused(
					&self.inner.db,
					family,
					Some(authorization.account_id),
					client,
					info,
				)
				.await?;
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

		// Signing may create a key, and claims read the app's tables, each on
		// a connection of its own, so they happen before the transaction holds
		// one: SQLite in memory has only one.
		let signers = self.signers(client).await?;
		let id_token_claims = self
			.id_token_claims(
				authorization.account_id,
				&authorization.scopes,
				&authorization.id_token_claims,
			)
			.await?;
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

			let family = self
				.start_family(
					&tx,
					client,
					authorization.account_id,
					authorization.scopes.0.clone(),
					authorization.id_token_claims.0.clone(),
					authorization.auth_time,
					request.dpop_jkt.as_deref(),
				)
				.await?;
			Authorization::update()
				.filter(|row| row.id.eq(authorization.id))
				.set(|row| row.family_id, Some(family.id))
				.run(&tx)
				.await?;

			self.issue(
				&tx,
				&signers,
				client,
				&family,
				&family.scopes,
				Extras {
					nonce: authorization.nonce.as_deref(),
					id_token_claims,
					refresh: true,
					dpop: request.dpop_jkt.as_deref(),
				},
			)
			.await
		}
		.await;

		finish(tx, result).await
	}

	/// A new refresh-token family for `account`'s sign-in with `client`,
	/// under their grant. A public app that sent a DPoP proof, whose key is
	/// `dpop`, gets refresh tokens bound to that key (RFC 9449 §5): a
	/// confidential app's are bound by its own authentication instead.
	#[allow(clippy::too_many_arguments)]
	pub(crate) async fn start_family(
		&self,
		tx: &Tx,
		client: &Client,
		account: Id<Account>,
		scopes: Vec<String>,
		id_token_claims: Vec<String>,
		auth_time: Timestamp,
		dpop: Option<&str>,
	) -> Result<Family> {
		let grant = Grant::query()
			.filter(|grant| grant.account_id.eq(account) & grant.client_id.eq(client.id))
			.first(tx)
			.await?
			.ok_or_else(|| IdpError::invalid_grant("the person disconnected this app"))?;
		Ok(NewFamily {
			grant_id: Some(grant.id),
			account_id: Some(account),
			client_id: client.id,
			scopes: Json(scopes),
			id_token_claims: Json(id_token_claims),
			auth_time,
			expires_at: leptoskit_db::now() + Self::lifetime(self.inner.lifetimes.refresh)?,
			revoked_at: None,
			jkt: dpop
				.filter(|_| client.auth_method == ClientAuth::None)
				.map(str::to_owned),
		}
		.insert(tx)
		.await?)
	}

	async fn exchange_refresh(
		&self,
		request: &TokenRequest,
		client: &Client,
		info: &RequestInfo,
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
		let (Some(account), Some(grant)) = (family.account_id, family.grant_id) else {
			return Err(invalid());
		};

		// Checked before anything else: without the key, a stolen refresh
		// token can't even end the family it belongs to.
		if let Some(bound) = &family.jkt
			&& request.dpop_jkt.as_deref() != Some(bound.as_str())
		{
			return Err(crate::dpop::invalid(
				"this refresh token is bound to a key: send a proof signed by it",
			));
		}

		if token.used_at.is_some() {
			// Each refresh token works once. Seen again, someone else holds a
			// copy, and the whole family ends.
			self.end_reused(&self.inner.db, family.id, Some(account), client, info)
				.await?;
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
				let asked = split_scopes(asked);
				if asked.is_empty() || asked.iter().any(|scope| !family.scopes.contains(scope)) {
					return Err(IdpError::invalid_scope(
						"a refresh can only ask for scopes already granted",
					));
				}
				asked
			}
		};

		let signers = self.signers(client).await?;
		let id_token_claims = self
			.id_token_claims(account, &scopes, &family.id_token_claims)
			.await?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			let used = RefreshToken::update()
				.filter(|row| row.id.eq(token.id) & row.used_at.is_null())
				.set(|row| row.used_at, Some(now))
				.run(&tx)
				.await?;

			if used == 0 {
				// Another request traded it first: one of the two is a copy.
				self.end_reused(&tx, family.id, Some(account), client, info)
					.await?;
				return Ok(None);
			}
			if !usable(&tx, account).await? {
				return Err(IdpError::invalid_grant("that account can't sign in"));
			}

			Grant::update()
				.filter(|row| row.id.eq(grant))
				.set(|row| row.last_used_at, Some(now))
				.run(&tx)
				.await?;
			self.issue(
				&tx,
				&signers,
				client,
				&family,
				&Json(scopes.clone()),
				Extras {
					id_token_claims,
					refresh: true,
					dpop: request.dpop_jkt.as_deref(),
					..Extras::default()
				},
			)
			.await
			.map(Some)
		}
		.await;

		// A family ended by a race stays ended, so commit before refusing.
		finish(tx, result).await?.ok_or_else(|| {
			IdpError::invalid_grant("that refresh token was already used; sign in again")
		})
	}

	/// Ends a family whose code or refresh token was used twice, and says so
	/// in the person's audit trail.
	pub(crate) async fn end_reused(
		&self,
		connection: &impl Connection,
		family: Id<Family>,
		account: Option<Id<Account>>,
		client: &Client,
		info: &RequestInfo,
	) -> Result<()> {
		if revoke_family(connection, family).await? {
			self.inner
				.auth
				.record(
					connection,
					account,
					EventKind::AppTokenReused,
					Some(&client.name),
					info,
				)
				.await?;
		}
		Ok(())
	}

	/// An access token for an app acting as itself (RFC 6749 §4.4): no
	/// person, no refresh token, and only the app's own API scopes.
	async fn exchange_client_credentials(
		&self,
		request: &TokenRequest,
		client: &Client,
	) -> Result<TokenResponse> {
		if client.auth_method == ClientAuth::None {
			return Err(IdpError::invalid_client());
		}

		let machine_scopes = || {
			client
				.scopes
				.iter()
				.filter(|scope| !PERSON_SCOPES.contains(&scope.as_str()))
				.cloned()
				.collect::<Vec<_>>()
		};
		let scopes = match request.scope.as_deref() {
			None => machine_scopes(),
			Some(asked) => split_scopes(asked),
		};

		if scopes.is_empty() {
			return Err(IdpError::invalid_scope(
				"this app may not ask for any scope as itself",
			));
		}
		if let Some(scope) = scopes.iter().find(|scope| {
			PERSON_SCOPES.contains(&scope.as_str()) || !client.scopes.contains(*scope)
		}) {
			return Err(IdpError::invalid_scope(format!(
				"this app may not ask for `{scope}` as itself"
			)));
		}

		let signers = self.signers(client).await?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			let now = leptoskit_db::now();
			// Each token gets a family, so it can be revoked; the families of
			// tokens that expired a day ago go.
			Family::query()
				.filter(|family| {
					family.account_id.is_null()
						& family.expires_at.lt(now - chrono::TimeDelta::days(1))
				})
				.delete(&tx)
				.await?;
			let family = NewFamily {
				grant_id: None,
				account_id: None,
				client_id: client.id,
				scopes: Json(scopes.clone()),
				id_token_claims: Json(Vec::new()),
				auth_time: now,
				expires_at: now + Self::lifetime(self.inner.lifetimes.access)?,
				revoked_at: None,
				jkt: None,
			}
			.insert(&tx)
			.await?;

			self.issue(
				&tx,
				&signers,
				client,
				&family,
				&family.scopes,
				Extras {
					dpop: request.dpop_jkt.as_deref(),
					..Extras::default()
				},
			)
			.await
		}
		.await;

		finish(tx, result).await
	}

	/// The keys `client`'s tokens are signed with.
	pub(crate) async fn signers(&self, client: &Client) -> Result<Signers> {
		let access = self.signer(SigningAlgorithm::Es256).await?;
		let id_token = match client.id_token_alg {
			SigningAlgorithm::Es256 => access.clone(),
			SigningAlgorithm::Rs256 => self.signer(SigningAlgorithm::Rs256).await?,
		};
		Ok(Signers { access, id_token })
	}

	/// New tokens for `family`: an access token, a refresh token when
	/// `extras.refresh` says so, and with `openid` an ID token.
	pub(crate) async fn issue(
		&self,
		tx: &Tx,
		signers: &Signers,
		client: &Client,
		family: &Family,
		scopes: &Json<Vec<String>>,
		extras: Extras<'_>,
	) -> Result<TokenResponse> {
		let now = leptoskit_db::now();
		let refresh_token = if extras.refresh {
			let refresh = secrets::token()?;
			NewRefreshToken {
				family_id: family.id,
				token_hash: secrets::hash(&refresh),
				used_at: None,
			}
			.insert(tx)
			.await?;
			Some(refresh)
		} else {
			None
		};

		let subject = match family.account_id {
			Some(account) => subject(client, account),
			None => client.client_id.clone(),
		};
		let access_lifetime = Self::lifetime(self.inner.lifetimes.access)?;
		let scope = scopes.join(" ");
		let mut access_claims = json!({
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
		});
		if let Some(jkt) = extras.dpop {
			access_claims["cnf"] = json!({ "jkt": jkt });
		}
		let access_token = sign(&signers.access, ACCESS_TOKEN_TYPE, &access_claims)?;
		let id_token =
			if family.account_id.is_some() && scopes.iter().any(|scope| scope == "openid") {
				let mut claims = Value::Object(extras.id_token_claims);
				let standard = json!({
					"iss": self.inner.issuer,
					"sub": subject,
					"aud": client.client_id,
					"iat": now.timestamp(),
					"exp": (now + Self::lifetime(self.inner.lifetimes.id_token)?).timestamp(),
					"auth_time": family.auth_time.timestamp(),
					"sid": family.id.to_string(),
				});
				for (name, value) in standard.as_object().into_iter().flatten() {
					claims[name] = value.clone();
				}
				if let Some(nonce) = extras.nonce {
					claims["nonce"] = Value::from(nonce);
				}
				Some(sign(&signers.id_token, ID_TOKEN_TYPE, &claims)?)
			} else {
				None
			};

		Ok(TokenResponse {
			access_token,
			token_type: if extras.dpop.is_some() {
				"DPoP"
			} else {
				"Bearer"
			}
			.into(),
			expires_in: u64::try_from(access_lifetime.num_seconds()).unwrap_or_default(),
			refresh_token,
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

		if family.revoked_at.is_some() || text("client_id") != Some(client.client_id.as_str()) {
			return Ok(None);
		}
		if let Some(account) = family.account_id
			&& !usable(&self.inner.db, account).await?
		{
			return Ok(None);
		}

		Ok(Some(AccessToken {
			account: family.account_id,
			client_id: client.client_id,
			subject: text("sub").unwrap_or_default().to_owned(),
			scopes: split_scopes(text("scope").unwrap_or_default()),
			auth_time,
			issued_at: iat,
			expires_at: exp,
			family: family.id,
			jkt: claims
				.get("cnf")
				.and_then(|cnf| cnf.get("jkt"))
				.and_then(Value::as_str)
				.map(str::to_owned),
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
		let Some(account) = token
			.account
			.filter(|_| token.scopes.iter().any(|scope| scope == "openid"))
		else {
			return Err(IdpError::oauth(
				"insufficient_scope",
				"userinfo needs the openid scope",
			));
		};

		let mut out = self.released_claims(account, &token.scopes).await?;
		out.insert("sub".into(), Value::from(token.subject.clone()));
		Ok(out)
	}

	/// The claims `scopes` release about `account`: whatever the app's
	/// [`Claims`](crate::Claims) adds, and with `email` their verified
	/// primary address. Not `sub`, which depends on the app asking.
	async fn released_claims(
		&self,
		account: Id<Account>,
		scopes: &[String],
	) -> Result<Map<String, Value>> {
		let scopes = GrantedScopes::new(scopes.to_vec());
		let mut claims = ClaimSet::default();
		let signed_in = SignedIn {
			account,
			session: Id::from_uuid(uuid::Uuid::nil()),
			aal: leptoskit_auth::Aal::One,
			authenticated_at: leptoskit_db::now(),
			mfa_at: None,
		};
		let db = self
			.inner
			.db
			.with_context(leptoskit_auth::context(Some(signed_in)));
		(self.inner.claims)(&db, account, &scopes, &mut claims).await?;

		let mut out: Map<String, Value> = claims.into_inner().into_iter().collect();

		if scopes.has("email") {
			let email = EmailAddress::query()
				.filter(|email| {
					email.account_id.eq(account)
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

	/// The claims an app asked to find in its ID token, of those its scopes
	/// release.
	pub(crate) async fn id_token_claims(
		&self,
		account: Id<Account>,
		scopes: &[String],
		requested: &[String],
	) -> Result<Map<String, Value>> {
		if requested.is_empty() || !scopes.iter().any(|scope| scope == "openid") {
			return Ok(Map::new());
		}

		let mut released = self.released_claims(account, scopes).await?;
		let wanted = |name: &str| requested.iter().any(|claim| claim == name);
		// `email_verified` says what `email` is worth, so it comes along.
		released.retain(|name, _| wanted(name) || (name == "email_verified" && wanted("email")));
		Ok(released)
	}

	/// Answers `POST /oauth/revoke` (RFC 7009): ends the family `token`
	/// belongs to, if it's the calling app's. Unknown tokens are already
	/// revoked, so they succeed too.
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
				token_type: Some(
					if access.jkt.is_some() {
						"DPoP"
					} else {
						"Bearer"
					}
					.into(),
				),
				exp: Some(access.expires_at.timestamp()),
				iat: Some(access.issued_at.timestamp()),
				iss: Some(self.inner.issuer.clone()),
				aud: Some(self.inner.audience.clone()),
				cnf: access.jkt.map(|jkt| json!({ "jkt": jkt })),
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
		let Some(account) = family.account_id else {
			return Ok(Introspection::default());
		};

		if family.revoked_at.is_some()
			|| family.expires_at <= now
			|| !may_see(&client)
			|| !usable(&self.inner.db, account).await?
		{
			return Ok(Introspection::default());
		}

		Ok(Introspection {
			active: true,
			scope: Some(family.scopes.join(" ")),
			sub: Some(subject(&client, account)),
			client_id: Some(client.client_id),
			token_type: Some("refresh_token".into()),
			exp: Some(family.expires_at.timestamp()),
			iat: Some(refresh.created_at.timestamp()),
			iss: Some(self.inner.issuer.clone()),
			aud: None,
			cnf: None,
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

/// Signs a token, as the protocol's own error if the key can't.
fn sign(signer: &Signer, typ: &str, claims: &Value) -> Result<String> {
	jwt::sign(signer, typ, claims)
		.map_err(|_| IdpError::Config("the RSA key couldn't sign a token".into()))
}

/// Space-separated scopes, without empties.
pub(crate) fn split_scopes(scopes: &str) -> Vec<String> {
	let mut unique: Vec<String> = Vec::new();
	for scope in scopes.split(' ').filter(|scope| !scope.is_empty()) {
		if !unique.iter().any(|known| known == scope) {
			unique.push(scope.to_owned());
		}
	}
	unique
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

/// Ends a family: its refresh tokens stop working, and so do its access
/// tokens wherever they're checked against it. Returns whether it was live.
pub(crate) async fn revoke_family(
	connection: &impl Connection,
	family: Id<Family>,
) -> Result<bool> {
	let revoked = Family::update()
		.filter(|row| row.id.eq(family) & row.revoked_at.is_null())
		.set(|row| row.revoked_at, Some(leptoskit_db::now()))
		.run(connection)
		.await?;
	Ok(revoked > 0)
}

/// Ends every token an app got as itself, with `client_credentials`.
pub(crate) async fn revoke_machine_tokens(
	connection: &impl Connection,
	client: Id<Client>,
) -> Result<()> {
	Family::update()
		.filter(|row| {
			row.client_id.eq(client) & row.account_id.is_null() & row.revoked_at.is_null()
		})
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

pub(crate) fn required<'a>(value: Option<&'a String>, name: &str) -> Result<&'a str> {
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn scopes_split_without_empties_or_repeats() {
		assert_eq!(split_scopes(" a  b a "), ["a", "b"]);
		assert_eq!(split_scopes(""), [] as [String; 0]);
		let debug = format!(
			"{:?}",
			TokenRequest {
				device_code: Some("device-secret".into()),
				client_assertion: Some("assertion-secret".into()),
				..TokenRequest::default()
			}
		);
		assert!(!debug.contains("secret"), "{debug}");
	}
}
