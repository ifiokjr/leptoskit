//! What the `/oauth` routes send and receive, and what `lk auth idp client`
//! asks the server to do. Clients and the CLI build these too, so they need
//! no server code.

use leptoskit_db::Timestamp;
use serde::Deserialize;
use serde::Serialize;

use crate::ClientAuth;

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
	/// A 10-minute JWT, sent to your API as `Authorization: Bearer`.
	pub access_token: String,
	/// Always `Bearer`.
	pub token_type: String,
	/// Seconds until the access token expires.
	pub expires_in: u64,
	/// Trades for new tokens at the token endpoint, once.
	pub refresh_token: String,
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
	/// Where codes may be sent. Each is compared exactly.
	pub redirect_uris: Vec<String>,
	/// Where people may be sent after signing out.
	#[serde(default)]
	pub post_logout_redirect_uris: Vec<String>,
	/// The scopes it may ask for, such as `openid`, `profile` and
	/// `posts:read`.
	pub scopes: Vec<String>,
	/// How it proves who it is: a secret, or nothing for a public app.
	pub auth_method: ClientAuth,
	/// One of your own apps: it sees the account ID as `sub`, and needs no
	/// consent.
	#[serde(default)]
	pub first_party: bool,
	/// Whether it may introspect other apps' tokens, as a resource server
	/// does.
	#[serde(default)]
	pub introspection: bool,
}

/// A registered app, as `lk auth idp client list` shows it. No secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
	/// Its OAuth client ID.
	pub client_id: String,
	/// Its name.
	pub name: String,
	/// How it authenticates.
	pub auth_method: ClientAuth,
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
			Self::Failed { message } => f.debug_struct("Failed").field("message", message).finish(),
		}
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
			refresh_token: "refresh-secret".into(),
			scope: "openid".into(),
			id_token: Some("id-secret".into()),
		};
		let added = ClientCommandOutput::Added {
			client: ClientInfo {
				client_id: "app".into(),
				name: "App".into(),
				auth_method: ClientAuth::SecretBasic,
				redirect_uris: Vec::new(),
				post_logout_redirect_uris: Vec::new(),
				scopes: Vec::new(),
				first_party: false,
				introspection: false,
				created_at: Timestamp::default(),
			},
			secret: Some("client-secret".into()),
		};

		for text in [format!("{tokens:?}"), format!("{added:?}")] {
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
		assert!(!spec.first_party);
	}
}
