//! Dynamic client registration: apps register themselves at
//! `POST /oauth/register` (RFC 7591), then read, change, and delete their
//! registration with the token it returned (RFC 7592).
//!
//! An app registered this way is like one from the developer portal, with
//! no owner: never first-party, never able to introspect other apps'
//! tokens, and limited to the scopes the provider offers. Only the SHA-256
//! of its registration access token is kept.

use std::fmt;

use leptoskit_auth::Limit;
use leptoskit_auth::RequestInfo;
use leptoskit_db::Json;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::Client;
use crate::ClientAuth;
use crate::ClientInfo;
use crate::GrantType;
use crate::Idp;
use crate::IdpError;
use crate::NewClientSpec;
use crate::Result;
use crate::SigningAlgorithm;
use crate::clients::check_authentication;
use crate::clients::dedupe;
use crate::clients::metadata;
use crate::secrets;
use crate::tokens::finish;
use crate::tokens::split_scopes;

/// How many apps one address may register an hour without an initial
/// access token.
const OPEN_REGISTRATIONS: Limit = Limit::new(10, std::time::Duration::from_hours(1));

/// The scopes an app gets when it doesn't ask for any.
const DEFAULT_SCOPES: [&str; 3] = ["openid", "profile", "email"];

/// What an app says about itself when it registers (RFC 7591 §2, OpenID
/// Connect Dynamic Client Registration §2), as far as this provider uses
/// it. Anything else, such as `logo_uri` or `contacts`, is ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientMetadata {
	/// Where codes may be sent.
	#[serde(default)]
	pub redirect_uris: Vec<String>,
	/// Where people may be sent after signing out.
	#[serde(default)]
	pub post_logout_redirect_uris: Vec<String>,
	/// `client_secret_basic` (the default), `client_secret_post`,
	/// `private_key_jwt`, or `none` for a public app.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub token_endpoint_auth_method: Option<String>,
	/// `authorization_code` (the default), `refresh_token`, the device-code
	/// URN, and `client_credentials`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub grant_types: Option<Vec<String>>,
	/// `code`, the only one there is.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response_types: Option<Vec<String>>,
	/// The app's name, as the consent screen shows it.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub client_name: Option<String>,
	/// The scopes it may ask for, space-separated: `openid profile email`
	/// unless it says.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scope: Option<String>,
	/// Its public keys, for `private_key_jwt`, as a JWK set.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub jwks: Option<Value>,
	/// Refused: send `jwks` instead.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub jwks_uri: Option<String>,
	/// `ES256` or `RS256`; RS256 when the server has an RSA key, as OpenID
	/// Connect makes it the default, and ES256 otherwise.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub id_token_signed_response_alg: Option<String>,
	/// Whether every access token it gets must be bound to its key with
	/// DPoP (RFC 9449 §5.2).
	#[serde(default)]
	pub dpop_bound_access_tokens: bool,
	/// On a change (RFC 7592), the app's client ID, which must match.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub client_id: Option<String>,
	/// On a change, the app's current secret, if it sends it: it must match.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub client_secret: Option<String>,
	/// Never sent by an app: these are the server's to say.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub registration_access_token: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub registration_client_uri: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub client_id_issued_at: Option<i64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub client_secret_expires_at: Option<i64>,
}

/// What registration answers (RFC 7591 §3.2.1, RFC 7592 §3): the app as
/// registered, and on registering, its secret and the token that manages
/// it, each shown this once. Its `Debug` leaves both out.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientRegistration {
	pub client_id: String,
	/// Its secret, for an app that authenticates with one.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub client_secret: Option<String>,
	/// When it registered, in seconds since the epoch.
	pub client_id_issued_at: i64,
	/// `0`, for never, when it has a secret.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub client_secret_expires_at: Option<i64>,
	/// The token that reads, changes, and deletes the registration, sent as
	/// `Authorization: Bearer`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub registration_access_token: Option<String>,
	/// Where to read, change, and delete it.
	pub registration_client_uri: String,
	pub client_name: String,
	pub redirect_uris: Vec<String>,
	pub post_logout_redirect_uris: Vec<String>,
	pub token_endpoint_auth_method: String,
	pub grant_types: Vec<String>,
	pub response_types: Vec<String>,
	pub scope: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub jwks: Option<Value>,
	pub id_token_signed_response_alg: String,
	pub dpop_bound_access_tokens: bool,
	/// Always `pairwise`: each app sees its own `sub` for a person.
	pub subject_type: String,
}

impl fmt::Debug for ClientRegistration {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("ClientRegistration")
			.field("client_id", &self.client_id)
			.field("client_name", &self.client_name)
			.field("redirect_uris", &self.redirect_uris)
			.field(
				"token_endpoint_auth_method",
				&self.token_endpoint_auth_method,
			)
			.field("grant_types", &self.grant_types)
			.field("scope", &self.scope)
			.finish_non_exhaustive()
	}
}

impl Idp {
	/// Whether apps may register themselves.
	pub fn has_dynamic_registration(&self) -> bool {
		self.inner.registration.is_some()
	}

	/// Registers an app that asked to at `POST /oauth/register`. `token` is
	/// the bearer token the request carried: the initial access token, when
	/// the provider needs one. Without one, each address may register 10
	/// apps an hour.
	///
	/// # Errors
	///
	/// [`IdpError::NotFound`] when dynamic registration is off;
	/// `invalid_token` without the initial access token it needs;
	/// `invalid_redirect_uri` or `invalid_client_metadata` for an app that
	/// can't register as it asks; a rate limit; or when the database fails.
	pub async fn register_dynamically(
		&self,
		token: Option<&str>,
		metadata: ClientMetadata,
		info: &RequestInfo,
	) -> Result<ClientRegistration> {
		let registration = self.inner.registration.as_ref().ok_or(IdpError::NotFound)?;

		if let Some(expected) = &registration.initial_access_token {
			let presented = token.unwrap_or_default();
			if !secrets::matches(presented, &secrets::hash(expected)) {
				return Err(invalid_token("send the initial access token to register"));
			}
		} else {
			let source = info
				.ip
				.map_or_else(|| "unknown".to_owned(), |ip| ip.to_string());
			self.inner
				.auth
				.limit_attempts("idp: dynamic registration", &source, OPEN_REGISTRATIONS)
				.await?;
		}

		refuse_server_fields(&metadata)?;
		let spec = self.spec_from(metadata)?;
		let manager = secrets::token()?;
		let (app, secret) = self
			.insert_client(spec, None, Some(secrets::hash(&manager)), &self.inner.db)
			.await?;
		let mut answer = self.describe_registration(&app);
		answer.client_secret = secret.map(|secret| secret.expose().to_owned());
		answer.client_secret_expires_at = answer.client_secret.as_ref().map(|_| 0);
		answer.registration_access_token = Some(manager);
		Ok(answer)
	}

	/// The registration of `client_id`, for the app holding its
	/// registration access token (RFC 7592 §2.1). Its secret isn't shown
	/// again: only its hash is kept.
	///
	/// # Errors
	///
	/// [`IdpError::NotFound`] when dynamic registration is off;
	/// `invalid_token` when the token isn't that app's; or when the database
	/// fails.
	pub async fn read_registration(
		&self,
		client_id: &str,
		token: Option<&str>,
	) -> Result<ClientRegistration> {
		let client = self.managed(client_id, token).await?;
		Ok(self.describe_registration(&ClientInfo::from(client)))
	}

	/// Replaces what an app registered with `metadata` (RFC 7592 §2.2):
	/// anything it leaves out goes back to the default. Taking scopes away
	/// takes them from everyone who connected it; changing how it
	/// authenticates ends its sign-ins, and a new secret, if it now needs
	/// one, is in the answer.
	///
	/// # Errors
	///
	/// As [`Idp::read_registration`], and `invalid_client_metadata` for a
	/// change the app can't have, or a `client_id` or `client_secret` that
	/// isn't its own.
	pub async fn update_registration(
		&self,
		client_id: &str,
		token: Option<&str>,
		metadata: ClientMetadata,
	) -> Result<ClientRegistration> {
		let client = self.managed(client_id, token).await?;

		if metadata.client_id.as_deref() != Some(client_id) {
			return Err(metadata_error("send the app's own client_id"));
		}
		if let Some(secret) = &metadata.client_secret {
			let current = client
				.secret_hash
				.as_ref()
				.is_some_and(|hash| secrets::matches(secret, hash));
			if !current {
				return Err(metadata_error(
					"client_secret isn't the app's current secret",
				));
			}
		}
		refuse_server_fields(&metadata)?;

		let spec = self.spec_from(metadata)?;
		let name = self.check_metadata(
			&spec.name,
			&spec.redirect_uris,
			&spec.post_logout_redirect_uris,
			&spec.scopes,
			&spec.grant_types,
		)?;
		check_authentication(spec.auth_method, &spec.grant_types, spec.jwks.as_ref())?;
		if spec.dpop_bound && !self.inner.dpop {
			return Err(metadata_error(
				"DPoP-bound tokens need DPoP on the server: turn on `dpop` in [auth.idp]",
			));
		}
		if spec.id_token_alg == SigningAlgorithm::Rs256 && self.inner.rsa_key.is_none() {
			return Err(metadata_error(
				"RS256 ID tokens need an RSA key on the server: ask for ES256",
			));
		}

		let scopes = dedupe(spec.scopes);
		let removed: Vec<String> = client
			.scopes
			.iter()
			.filter(|scope| !scopes.contains(*scope))
			.cloned()
			.collect();
		let method_changed = spec.auth_method != client.auth_method;
		let needs_secret = matches!(
			spec.auth_method,
			ClientAuth::SecretBasic | ClientAuth::SecretPost
		);
		let new_secret = if needs_secret && client.secret_hash.is_none() {
			Some(crate::ClientSecret::generate()?)
		} else {
			None
		};
		let secret_hash = match &new_secret {
			Some(secret) => Some(secrets::hash(secret.expose())),
			None if needs_secret => client.secret_hash.clone(),
			None => None,
		};

		let tx = self.inner.db.begin().await?;
		let result = async {
			Client::update()
				.filter(|row| row.id.eq(client.id))
				.set(|row| row.name, name.clone())
				.set(|row| row.auth_method, spec.auth_method)
				.set(|row| row.secret_hash, secret_hash.clone())
				.set(|row| row.jwks, spec.jwks.clone().map(Json))
				.set(
					|row| row.grant_types,
					Json(dedupe(spec.grant_types.clone())),
				)
				.set(|row| row.id_token_alg, spec.id_token_alg)
				.set(
					|row| row.redirect_uris,
					Json(dedupe(spec.redirect_uris.clone())),
				)
				.set(
					|row| row.post_logout_redirect_uris,
					Json(dedupe(spec.post_logout_redirect_uris.clone())),
				)
				.set(|row| row.scopes, Json(scopes.clone()))
				.set(|row| row.dpop_bound, spec.dpop_bound)
				.run(&tx)
				.await?;

			if !removed.is_empty() {
				crate::portal::narrow_grants(&tx, &client, &scopes, &removed).await?;
			}
			if method_changed {
				// How it proves who it is changed, so nothing it got before
				// should outlive the change.
				let families = crate::Family::query()
					.filter(|family| family.client_id.eq(client.id) & family.revoked_at.is_null())
					.all(&tx)
					.await?;
				for family in families {
					crate::tokens::revoke_family(&tx, family.id).await?;
				}
			}

			Client::find(&tx, client.id)
				.await?
				.map(ClientInfo::from)
				.ok_or(IdpError::NotFound)
		}
		.await;

		let app = finish(tx, result).await?;
		let mut answer = self.describe_registration(&app);
		answer.client_secret = new_secret.map(|secret| secret.expose().to_owned());
		answer.client_secret_expires_at = answer.client_secret.as_ref().map(|_| 0);
		Ok(answer)
	}

	/// Deletes an app's registration (RFC 7592 §2.3), ending every grant and
	/// token it holds; everyone who had connected it finds that in their
	/// audit trail.
	///
	/// # Errors
	///
	/// As [`Idp::read_registration`].
	pub async fn delete_registration(
		&self,
		client_id: &str,
		token: Option<&str>,
		info: &RequestInfo,
	) -> Result<()> {
		let client = self.managed(client_id, token).await?;
		let tx = self.inner.db.begin().await?;
		let result = self.delete_client(&tx, &client, info).await;
		finish(tx, result).await
	}

	/// The self-registered app `client_id`, if `token` manages it.
	async fn managed(&self, client_id: &str, token: Option<&str>) -> Result<Client> {
		if self.inner.registration.is_none() {
			return Err(IdpError::NotFound);
		}

		let refused = || invalid_token("send the registration access token this app got");
		let token = token.ok_or_else(refused)?;
		let client = self.client(client_id).await?.ok_or_else(refused)?;
		let manages = client
			.registration_hash
			.as_ref()
			.is_some_and(|hash| secrets::matches(token, hash));

		if manages { Ok(client) } else { Err(refused()) }
	}

	/// What an app asks for, as the registration this provider makes.
	fn spec_from(&self, metadata: ClientMetadata) -> Result<NewClientSpec> {
		if metadata.jwks_uri.is_some() {
			return Err(metadata_error(
				"jwks_uri isn't supported: send the keys themselves, as jwks",
			));
		}

		let auth_method = match metadata.token_endpoint_auth_method.as_deref() {
			None | Some("client_secret_basic") => ClientAuth::SecretBasic,
			Some("client_secret_post") => ClientAuth::SecretPost,
			Some("private_key_jwt") => ClientAuth::PrivateKeyJwt,
			Some("none") => ClientAuth::None,
			Some(other) => {
				return Err(metadata_error(&format!(
					"token_endpoint_auth_method `{other}` isn't supported"
				)));
			}
		};

		let mut grant_types = Vec::new();
		for grant in metadata
			.grant_types
			.unwrap_or_else(|| vec![GrantType::AuthorizationCode.as_str().to_owned()])
		{
			match grant.as_str() {
				// Implied by the code and device grants.
				"refresh_token" => {}
				"authorization_code" => grant_types.push(GrantType::AuthorizationCode),
				"client_credentials" => grant_types.push(GrantType::ClientCredentials),
				other if other == GrantType::DeviceCode.as_str() => {
					grant_types.push(GrantType::DeviceCode);
				}
				other => {
					return Err(metadata_error(&format!(
						"the grant type `{other}` isn't supported"
					)));
				}
			}
		}

		let response_types = metadata
			.response_types
			.unwrap_or_else(|| vec!["code".to_owned()]);
		if response_types.iter().any(|kind| kind != "code") {
			return Err(metadata_error("the only response type is `code`"));
		}
		if !response_types.is_empty() && !grant_types.contains(&GrantType::AuthorizationCode) {
			return Err(metadata_error(
				"the `code` response type needs the authorization_code grant",
			));
		}

		let id_token_alg = match metadata.id_token_signed_response_alg.as_deref() {
			None if self.inner.rsa_key.is_some() => SigningAlgorithm::Rs256,
			None | Some("ES256") => SigningAlgorithm::Es256,
			Some("RS256") => SigningAlgorithm::Rs256,
			Some(other) => {
				return Err(metadata_error(&format!(
					"ID tokens can't be signed with `{other}`: use ES256 or RS256"
				)));
			}
		};

		let scopes = match metadata.scope.as_deref() {
			Some(scope) => split_scopes(scope),
			None => {
				DEFAULT_SCOPES
					.iter()
					.map(|scope| (*scope).to_owned())
					.collect()
			}
		};

		if let Some(uri) = metadata
			.redirect_uris
			.iter()
			.find(|uri| crate::clients::check_redirect_uri(uri).is_err())
		{
			return Err(IdpError::oauth(
				"invalid_redirect_uri",
				format!(
					"`{uri}` must be https, http to this device, or a reverse-domain app scheme, \
					 with no fragment"
				),
			));
		}

		let name = metadata
			.client_name
			.filter(|name| !name.trim().is_empty())
			.or_else(|| {
				metadata
					.redirect_uris
					.first()
					.and_then(|uri| url::Url::parse(uri).ok())
					.and_then(|url| url.host_str().map(str::to_owned))
			})
			.unwrap_or_else(|| "Unnamed app".to_owned());

		Ok(NewClientSpec {
			name,
			redirect_uris: metadata.redirect_uris,
			post_logout_redirect_uris: metadata.post_logout_redirect_uris,
			scopes,
			auth_method,
			grant_types,
			jwks: metadata.jwks,
			id_token_alg,
			first_party: false,
			introspection: false,
			dpop_bound: metadata.dpop_bound_access_tokens,
		})
	}

	/// The registration answer for `app`, without its secrets.
	fn describe_registration(&self, app: &ClientInfo) -> ClientRegistration {
		let refreshes = app
			.grant_types
			.iter()
			.any(|grant| matches!(grant, GrantType::AuthorizationCode | GrantType::DeviceCode));
		let mut grant_types: Vec<String> = app
			.grant_types
			.iter()
			.map(|grant| grant.as_str().to_owned())
			.collect();
		if refreshes {
			grant_types.push("refresh_token".to_owned());
		}

		ClientRegistration {
			client_id: app.client_id.clone(),
			client_secret: None,
			client_id_issued_at: app.created_at.timestamp(),
			client_secret_expires_at: None,
			registration_access_token: None,
			registration_client_uri: self.url_of(&format!("/oauth/register/{}", app.client_id)),
			client_name: app.name.clone(),
			redirect_uris: app.redirect_uris.clone(),
			post_logout_redirect_uris: app.post_logout_redirect_uris.clone(),
			token_endpoint_auth_method: app.auth_method.as_str().to_owned(),
			grant_types,
			response_types: if app.grant_types.contains(&GrantType::AuthorizationCode) {
				vec!["code".to_owned()]
			} else {
				Vec::new()
			},
			scope: app.scopes.join(" "),
			jwks: app.jwks.clone(),
			id_token_signed_response_alg: app.id_token_alg.as_str().to_owned(),
			dpop_bound_access_tokens: app.dpop_bound,
			subject_type: "pairwise".to_owned(),
		}
	}
}

/// The fields only the server sets, which an app sending them is confused
/// about (RFC 7592 §2.2).
fn refuse_server_fields(metadata: &ClientMetadata) -> Result<()> {
	if metadata.registration_access_token.is_some()
		|| metadata.registration_client_uri.is_some()
		|| metadata.client_id_issued_at.is_some()
		|| metadata.client_secret_expires_at.is_some()
	{
		return Err(metadata_error(
			"don't send registration_access_token, registration_client_uri, client_id_issued_at \
			 or client_secret_expires_at: the server sets them",
		));
	}
	Ok(())
}

fn metadata_error(description: &str) -> IdpError {
	metadata(description.to_owned())
}

/// `invalid_token`: the bearer token a registration request needs is
/// missing or wrong.
fn invalid_token(description: &str) -> IdpError {
	IdpError::oauth("invalid_token", description.to_owned())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn registrations_never_show_their_secrets_in_debug() {
		let registration = ClientRegistration {
			client_id: "app".into(),
			client_secret: Some("s3cret".into()),
			client_id_issued_at: 0,
			client_secret_expires_at: Some(0),
			registration_access_token: Some("t0ken".into()),
			registration_client_uri: "https://notes.example/oauth/register/app".into(),
			client_name: "App".into(),
			redirect_uris: Vec::new(),
			post_logout_redirect_uris: Vec::new(),
			token_endpoint_auth_method: "client_secret_basic".into(),
			grant_types: Vec::new(),
			response_types: Vec::new(),
			scope: "openid".into(),
			jwks: None,
			id_token_signed_response_alg: "ES256".into(),
			dpop_bound_access_tokens: false,
			subject_type: "pairwise".into(),
		};
		let shown = format!("{registration:?}");

		assert!(!shown.contains("s3cret"), "{shown}");
		assert!(!shown.contains("t0ken"), "{shown}");
		assert!(shown.contains("app"), "{shown}");
	}

	#[test]
	fn server_fields_are_refused() {
		assert!(refuse_server_fields(&ClientMetadata::default()).is_ok());
		let sent = ClientMetadata {
			client_id_issued_at: Some(1),
			..ClientMetadata::default()
		};
		assert_eq!(
			refuse_server_fields(&sent).unwrap_err().code(),
			"invalid_client_metadata"
		);
	}
}
