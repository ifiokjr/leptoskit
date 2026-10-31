//! What the `/oauth` routes send and receive, and what `lk auth idp client`
//! asks the server to do. Clients and the CLI build these too, so they need
//! no server code.

use leptoskit_db::Timestamp;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::ClientAuth;
use crate::GrantType;
use crate::SigningAlgorithm;

/// The app asking for access, as a consent screen names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
	/// Its OAuth client ID.
	pub client_id: String,
	/// Its name, as its operator registered it.
	pub name: String,
}

/// One scope on a consent screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeDescription {
	/// The scope, such as `posts:read`.
	pub name: String,
	/// What it lets the app do, such as "Read your posts and drafts."
	pub description: String,
}

/// The answer to `GET /oauth/consent?request=…`: what the consent screen
/// shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentRequest {
	/// The app asking.
	pub app: AppInfo,
	/// What it asks to do, in the order it asked.
	pub scopes: Vec<ScopeDescription>,
	/// The host it will send the person back to, such as
	/// `partner.example`, so the screen can say where they're going.
	pub redirect_host: String,
}

/// The body of `POST /oauth/consent`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentDecision {
	/// The `request` the consent screen was opened with.
	pub request: String,
	/// Whether the person allowed it.
	pub approve: bool,
}

/// The answer to `POST /oauth/consent`: where to send the browser next,
/// back to the app with a code or with `access_denied`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentRedirect {
	/// The address to navigate to.
	pub redirect_to: String,
}

/// An app someone has let sign in with their account, as `GET
/// /oauth/grants` lists them for a "Connected apps" screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectedApp {
	/// The app.
	pub app: AppInfo,
	/// What they let it do.
	pub scopes: Vec<ScopeDescription>,
	/// When they first connected it.
	pub connected_at: Timestamp,
	/// When it last renewed its access.
	pub last_used_at: Option<Timestamp>,
}

/// The body of `POST /oauth/grants/revoke`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisconnectApp {
	/// The app to disconnect.
	pub client_id: String,
}

/// An OAuth error, as the token, revocation and introspection endpoints
/// send it (RFC 6749 §5.2), and as the other routes send theirs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthError {
	/// The error code, such as `invalid_grant`.
	pub error: String,
	/// What went wrong, for the app's developer.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error_description: Option<String>,
}

/// The answer to a successful `POST /oauth/token`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenResponse {
	/// A 10-minute JWT, sent to your API as `Authorization: Bearer`, or as
	/// `Authorization: DPoP` with a proof when it's bound to a key.
	pub access_token: String,
	/// `Bearer`, or `DPoP` for a token bound to the key of the DPoP proof
	/// the request came with.
	pub token_type: String,
	/// Seconds until the access token expires.
	pub expires_in: u64,
	/// Trades for new tokens at the token endpoint, once. An app acting as
	/// itself gets none.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub refresh_token: Option<String>,
	/// The scopes the access token carries, space-separated.
	pub scope: String,
	/// The OpenID Connect ID token, when `openid` was granted.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub id_token: Option<String>,
}

impl std::fmt::Debug for TokenResponse {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("TokenResponse")
			.field("token_type", &self.token_type)
			.field("expires_in", &self.expires_in)
			.field("scope", &self.scope)
			.finish_non_exhaustive()
	}
}

/// An app to register, as `lk auth idp client add` describes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewClientSpec {
	/// Its name, as the consent screen shows it.
	pub name: String,
	/// Where codes may be sent. Each is compared exactly. An app that only
	/// uses the device flow or acts as itself needs none.
	#[serde(default)]
	pub redirect_uris: Vec<String>,
	/// Where people may be sent after signing out.
	#[serde(default)]
	pub post_logout_redirect_uris: Vec<String>,
	/// The scopes it may ask for, such as `openid`, `profile` and
	/// `posts:read`.
	pub scopes: Vec<String>,
	/// How it proves who it is: a secret, a key, or nothing for a public
	/// app.
	pub auth_method: ClientAuth,
	/// How it gets tokens: the authorization code flow, unless it says
	/// otherwise.
	#[serde(default = "default_grant_types")]
	pub grant_types: Vec<GrantType>,
	/// The public keys it signs `private_key_jwt` assertions with, as a JWK
	/// set: `{"keys": [...]}`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub jwks: Option<Value>,
	/// How its ID tokens are signed: ES256, unless it needs RS256.
	#[serde(default)]
	pub id_token_alg: SigningAlgorithm,
	/// One of your own apps: it sees the account ID as `sub`, and needs no
	/// consent.
	#[serde(default)]
	pub first_party: bool,
	/// Whether it may introspect other apps' tokens, as a resource server
	/// does.
	#[serde(default)]
	pub introspection: bool,
	/// Whether every access token it gets must be bound to its key with
	/// DPoP (RFC 9449). It needs the provider's DPoP on.
	#[serde(default)]
	pub dpop_bound: bool,
}

/// The authorization code flow: what an app gets unless it asks otherwise.
pub fn default_grant_types() -> Vec<GrantType> {
	vec![GrantType::AuthorizationCode]
}

/// A registered app, as `lk auth idp client list` shows it. No secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(
	clippy::struct_excessive_bools,
	reason = "independent properties of an app, as its JSON carries them"
)]
pub struct ClientInfo {
	/// Its OAuth client ID.
	pub client_id: String,
	/// Its name.
	pub name: String,
	/// How it authenticates.
	pub auth_method: ClientAuth,
	/// How it gets tokens.
	pub grant_types: Vec<GrantType>,
	/// The public keys it signs assertions with, for `private_key_jwt`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub jwks: Option<Value>,
	/// How its ID tokens are signed.
	pub id_token_alg: SigningAlgorithm,
	/// Where codes may be sent.
	pub redirect_uris: Vec<String>,
	/// Where people may be sent after signing out.
	pub post_logout_redirect_uris: Vec<String>,
	/// The scopes it may ask for.
	pub scopes: Vec<String>,
	/// Whether it's one of your own apps.
	pub first_party: bool,
	/// Whether it may introspect other apps' tokens.
	pub introspection: bool,
	/// Whether its access tokens must be bound to its key with DPoP.
	#[serde(default)]
	pub dpop_bound: bool,
	/// Whether it registered itself, through dynamic registration.
	#[serde(default)]
	pub self_registered: bool,
	/// When it was registered.
	pub created_at: Timestamp,
}

/// What `lk auth idp client` asks the app's server to do. The CLI runs the
/// server binary with it in `LEPTOSKIT_IDP_COMMAND`, and the server's call
/// to `Idp::run_command_if_requested` carries it out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ClientCommand {
	/// Registers an app.
	Add(NewClientSpec),
	/// Lists every registered app.
	List,
	/// Removes an app, ending every grant and token it holds.
	Remove {
		/// Its client ID.
		client_id: String,
	},
	/// Replaces an app's secret, ending the tokens it got as itself.
	RotateSecret {
		/// Its client ID.
		client_id: String,
	},
}

/// What the server answers a [`ClientCommand`] with, in the file named by
/// `LEPTOSKIT_IDP_OUT`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ClientCommandOutput {
	/// The app was registered.
	Added {
		/// The app.
		client: ClientInfo,
		/// Its secret, shown this once; `None` for a public app.
		secret: Option<String>,
	},
	/// Every registered app.
	Listed {
		/// The apps, oldest first.
		clients: Vec<ClientInfo>,
	},
	/// Whether an app was removed.
	Removed {
		/// `false` when no app had that client ID.
		removed: bool,
	},
	/// The app's new secret, shown this once.
	Rotated {
		/// The secret.
		secret: String,
	},
	/// The command failed.
	Failed {
		/// Why.
		message: String,
	},
}

impl std::fmt::Debug for ClientCommandOutput {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Added { client, secret } => {
				f.debug_struct("Added")
					.field("client", client)
					.field("secret", &secret.as_ref().map(|_| "…"))
					.finish()
			}
			Self::Listed { clients } => f.debug_struct("Listed").field("clients", clients).finish(),
			Self::Removed { removed } => {
				f.debug_struct("Removed").field("removed", removed).finish()
			}
			Self::Rotated { .. } => f.debug_struct("Rotated").finish_non_exhaustive(),
			Self::Failed { message } => f.debug_struct("Failed").field("message", message).finish(),
		}
	}
}

/// The answer to `POST /oauth/device_authorization` (RFC 8628 §3.2): what a
/// CLI or a TV shows, and how it polls. Its `Debug` leaves out the device
/// code.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAuthorization {
	/// The secret the device polls the token endpoint with.
	pub device_code: String,
	/// The code to show, such as `WDJB-MJHT`.
	pub user_code: String,
	/// Where to enter it.
	pub verification_uri: String,
	/// The same page with the code filled in, for a QR code or a link.
	pub verification_uri_complete: String,
	/// Seconds until the codes expire.
	pub expires_in: u64,
	/// Seconds to wait between polls.
	pub interval: u64,
}

impl std::fmt::Debug for DeviceAuthorization {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("DeviceAuthorization")
			.field("user_code", &self.user_code)
			.field("verification_uri", &self.verification_uri)
			.field("expires_in", &self.expires_in)
			.field("interval", &self.interval)
			.finish_non_exhaustive()
	}
}

/// The answer to `GET /oauth/device?user_code=…`: what the device page
/// shows before someone approves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRequest {
	/// The app on the device.
	pub app: AppInfo,
	/// What it asks to do.
	pub scopes: Vec<ScopeDescription>,
	/// The code, as the device shows it, for the person to compare.
	pub user_code: String,
}

/// The body of `POST /oauth/device`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDecision {
	/// The code the device shows, as typed.
	pub user_code: String,
	/// Whether the person allowed it.
	pub approve: bool,
}

/// The answer to `POST /oauth/par` (RFC 9126 §2.2): the handle to send the
/// browser to `/oauth/authorize` with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushedAuthorization {
	/// `urn:ietf:params:oauth:request_uri:…`, which works once.
	pub request_uri: String,
	/// Seconds until it expires.
	pub expires_in: u64,
}

/// An app someone registers in the developer portal, with `POST
/// /oauth/apps`. The portal's apps are never first-party, and can't
/// introspect other apps' tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewApp {
	/// Its name, as consent screens show it.
	pub name: String,
	/// Where codes may be sent.
	#[serde(default)]
	pub redirect_uris: Vec<String>,
	/// Where people may be sent after signing out.
	#[serde(default)]
	pub post_logout_redirect_uris: Vec<String>,
	/// The scopes it may ask for.
	pub scopes: Vec<String>,
	/// How it proves who it is.
	pub auth_method: ClientAuth,
	/// How it gets tokens.
	#[serde(default = "default_grant_types")]
	pub grant_types: Vec<GrantType>,
	/// Its public keys, for `private_key_jwt`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub jwks: Option<Value>,
}

/// The body of `POST /oauth/apps/update`: what an app's owner may change.
/// How it authenticates stays as it was registered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppChanges {
	/// The app.
	pub client_id: String,
	/// Its name.
	pub name: String,
	/// Where codes may be sent.
	#[serde(default)]
	pub redirect_uris: Vec<String>,
	/// Where people may be sent after signing out.
	#[serde(default)]
	pub post_logout_redirect_uris: Vec<String>,
	/// The scopes it may ask for. Grants keep only these.
	pub scopes: Vec<String>,
	/// Its public keys, for `private_key_jwt`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub jwks: Option<Value>,
}

/// The body of `POST /oauth/apps/rotate-secret` and `POST
/// /oauth/apps/delete`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRef {
	/// The app.
	pub client_id: String,
}

/// The answer to registering an app in the developer portal, or replacing
/// its secret: the app, and its secret this once. Its `Debug` leaves out
/// the secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredApp {
	/// The app.
	pub app: ClientInfo,
	/// Its secret, shown this once; `None` for an app with no secret.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub secret: Option<String>,
}

impl std::fmt::Debug for RegisteredApp {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("RegisteredApp")
			.field("app", &self.app)
			.field("secret", &self.secret.as_ref().map(|_| "…"))
			.finish()
	}
}

/// The environment variable that carries a [`ClientCommand`], as JSON.
pub const COMMAND_ENV: &str = "LEPTOSKIT_IDP_COMMAND";

/// The environment variable naming the file a [`ClientCommandOutput`] is
/// written to.
pub const COMMAND_OUT_ENV: &str = "LEPTOSKIT_IDP_OUT";

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn secrets_stay_out_of_debug() {
		let tokens = TokenResponse {
			access_token: "access-secret".into(),
			token_type: "Bearer".into(),
			expires_in: 600,
			refresh_token: Some("refresh-secret".into()),
			scope: "openid".into(),
			id_token: Some("id-secret".into()),
		};
		let added = ClientCommandOutput::Added {
			client: ClientInfo {
				client_id: "app".into(),
				name: "App".into(),
				auth_method: ClientAuth::SecretBasic,
				grant_types: default_grant_types(),
				jwks: None,
				id_token_alg: SigningAlgorithm::Es256,
				redirect_uris: Vec::new(),
				post_logout_redirect_uris: Vec::new(),
				scopes: Vec::new(),
				first_party: false,
				introspection: false,
				dpop_bound: false,
				self_registered: false,
				created_at: Timestamp::default(),
			},
			secret: Some("client-secret".into()),
		};

		let rotated = ClientCommandOutput::Rotated {
			secret: "rotated-secret".into(),
		};
		let device = DeviceAuthorization {
			device_code: "device-secret".into(),
			user_code: "WDJB-MJHT".into(),
			verification_uri: "https://notes.example/device".into(),
			verification_uri_complete: "https://notes.example/device?user_code=WDJB-MJHT".into(),
			expires_in: 600,
			interval: 5,
		};
		let registered = RegisteredApp {
			app: match &added {
				ClientCommandOutput::Added { client, .. } => client.clone(),
				_ => unreachable!(),
			},
			secret: Some("portal-secret".into()),
		};

		for text in [
			format!("{tokens:?}"),
			format!("{added:?}"),
			format!("{rotated:?}"),
			format!("{device:?}"),
			format!("{registered:?}"),
		] {
			assert!(!text.contains("secret\""), "{text}");
			assert!(!text.contains("-secret"), "{text}");
		}
	}

	#[test]
	fn commands_travel_as_tagged_json() {
		let command = ClientCommand::Remove {
			client_id: "abc".into(),
		};
		let json = serde_json::to_string(&command).unwrap();
		assert_eq!(json, r#"{"command":"remove","client_id":"abc"}"#);
		assert_eq!(
			serde_json::from_str::<ClientCommand>(r#"{"command":"list"}"#).unwrap(),
			ClientCommand::List
		);
		let spec: NewClientSpec = serde_json::from_str(
			r#"{"name":"A","redirect_uris":[],"scopes":["openid"],"auth_method":"none"}"#,
		)
		.unwrap();
		assert_eq!(spec.auth_method, ClientAuth::None);
		assert_eq!(spec.grant_types, [GrantType::AuthorizationCode]);
		assert_eq!(spec.id_token_alg, SigningAlgorithm::Es256);
		assert!(!spec.first_party);
		let spec: NewClientSpec = serde_json::from_str(
			r#"{"name":"CLI","scopes":["notes:read"],"auth_method":"none","grant_types":["urn:ietf:params:oauth:grant-type:device_code"],"id_token_alg":"RS256"}"#,
		)
		.unwrap();
		assert_eq!(spec.grant_types, [GrantType::DeviceCode]);
		assert_eq!(spec.id_token_alg, SigningAlgorithm::Rs256);
		assert_eq!(spec.redirect_uris, [] as [String; 0]);
	}
}
