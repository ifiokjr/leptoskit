//! The built-in providers beyond the first four: each is data (endpoints,
//! scopes, how it says who signed in, and what it does differently), not
//! code. `VERIFIED` dates each one.

use super::ApiProfile;
use super::ClientAuth;
use super::Endpoints;
use super::Flavor;
use super::IdTokenRules;
use super::Identify;
use super::Provider;
use super::Quirks;
use super::Secret;
use super::TokenBody;
use super::profiles::EmailList;
use super::secure_url;
use crate::AuthError;
use crate::Result;

/// The Graph API version Facebook sign-in uses unless `[auth.providers.
/// facebook] api_version` says otherwise. Facebook retires each version
/// about two years after the next one ships.
pub const FACEBOOK_GRAPH_VERSION: &str = "v26.0";

/// GitLab's own site, for [`Provider::gitlab`]; a self-managed GitLab
/// passes its own address instead.
pub const GITLAB_COM: &str = "https://gitlab.com";

/// The built-in provider `kind` that needs only a client ID and secret, as
/// `Auth::from_config` builds it: `None` for any other kind.
pub(crate) fn built_in(kind: &str, client_id: String, client_secret: String) -> Option<Provider> {
	Some(match kind {
		"google" => Provider::google(client_id, client_secret),
		"github" => Provider::github(client_id, client_secret),
		"x" => Provider::x(client_id, client_secret),
		"discord" => Provider::discord(client_id, client_secret),
		"linkedin" => Provider::linkedin(client_id, client_secret),
		"slack" => Provider::slack(client_id, client_secret),
		"twitch" => Provider::twitch(client_id, client_secret),
		"spotify" => Provider::spotify(client_id, client_secret),
		"reddit" => Provider::reddit(client_id, client_secret),
		"amazon" => Provider::amazon(client_id, client_secret),
		"dropbox" => Provider::dropbox(client_id, client_secret),
		"bitbucket" => Provider::bitbucket(client_id, client_secret),
		"notion" => Provider::notion(client_id, client_secret),
		"zoom" => Provider::zoom(client_id, client_secret),
		"tiktok" => Provider::tiktok(client_id, client_secret),
		"yahoo" => Provider::yahoo(client_id, client_secret),
		_ => return None,
	})
}

/// A provider with fixed endpoints and a client secret.
fn fixed(
	id: &str,
	name: &str,
	client_id: String,
	client_secret: String,
	scopes: &[&str],
	endpoints: Endpoints,
	quirks: Quirks,
) -> Provider {
	Provider {
		id: id.into(),
		name: name.into(),
		flavor: Flavor::Fixed,
		client_id,
		secret: Secret::Shared(client_secret),
		scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
		endpoints: Some(endpoints),
		quirks,
	}
}

/// A provider whose API says who signed in.
fn api(profile: ApiProfile) -> Quirks {
	Quirks {
		identify: Identify::Api(Box::new(profile)),
		..Quirks::default()
	}
}

impl Provider {
	/// Facebook Login, through the Graph API `version` (such as
	/// [`FACEBOOK_GRAPH_VERSION`]), with an app's ID and secret. Facebook
	/// never says whether it checked an address, so none is trusted.
	pub fn facebook(
		client_id: impl Into<String>,
		client_secret: impl Into<String>,
		version: &str,
	) -> Self {
		let graph = format!("https://graph.facebook.com/{version}");
		fixed(
			"facebook",
			"Facebook",
			client_id.into(),
			client_secret.into(),
			&["public_profile", "email"],
			Endpoints::fixed(
				&format!("https://www.facebook.com/{version}/dialog/oauth"),
				&format!("{graph}/oauth/access_token"),
				None,
				None,
			),
			api(ApiProfile {
				name: Some("/name"),
				picture: Some("/picture/data/url"),
				email: Some("/email"),
				..ApiProfile::at(format!("{graph}/me?fields=id,name,email,picture"), "/id")
			}),
		)
	}

	/// X (formerly Twitter), with an OAuth 2.0 client of type "Web App".
	/// X never shares a checked address, so accounts made with it have none.
	pub fn x(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		fixed(
			"x",
			"X",
			client_id.into(),
			client_secret.into(),
			&["tweet.read", "users.read"],
			Endpoints::fixed(
				"https://x.com/i/oauth2/authorize",
				"https://api.x.com/2/oauth2/token",
				None,
				None,
			),
			Quirks {
				client_auth: ClientAuth::Basic,
				..api(ApiProfile {
					name: Some("/data/name"),
					username: Some("/data/username"),
					picture: Some("/data/profile_image_url"),
					..ApiProfile::at(
						"https://api.x.com/2/users/me?user.fields=profile_image_url",
						"/data/id",
					)
				})
			},
		)
	}

	/// Discord, with an application's OAuth client ID and secret. Its
	/// `verified` flag says whether it checked the address.
	pub fn discord(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		fixed(
			"discord",
			"Discord",
			client_id.into(),
			client_secret.into(),
			&["identify", "email"],
			Endpoints::fixed(
				"https://discord.com/oauth2/authorize",
				"https://discord.com/api/oauth2/token",
				None,
				None,
			),
			api(ApiProfile {
				name: Some("/global_name"),
				username: Some("/username"),
				email: Some("/email"),
				verified: Some("/verified"),
				..ApiProfile::at("https://discord.com/api/users/@me", "/id")
			}),
		)
	}

	/// LinkedIn, through "Sign In with LinkedIn using OpenID Connect", not
	/// the retired profile API. LinkedIn's ID tokens leave out the nonce.
	pub fn linkedin(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		fixed(
			"linkedin",
			"LinkedIn",
			client_id.into(),
			client_secret.into(),
			&["openid", "profile", "email"],
			Endpoints::fixed(
				"https://www.linkedin.com/oauth/v2/authorization",
				"https://www.linkedin.com/oauth/v2/accessToken",
				Some("https://www.linkedin.com/oauth/openid/jwks"),
				Some("https://www.linkedin.com/oauth"),
			),
			Quirks {
				identify: Identify::IdToken(IdTokenRules {
					nonce_optional: true,
					..IdTokenRules::default()
				}),
				..Quirks::default()
			},
		)
	}

	/// GitLab, at `base`: [`GITLAB_COM`], or a self-managed GitLab's
	/// address, which is also its issuer. Without a secret, it's a public
	/// client, and PKCE alone protects the exchange.
	///
	/// # Errors
	///
	/// [`AuthError::Config`] when `base` isn't `https` (or `http` on
	/// localhost).
	pub fn gitlab(
		client_id: impl Into<String>,
		client_secret: Option<String>,
		base: &str,
	) -> Result<Self> {
		let base = base.trim_end_matches('/');

		if !secure_url(base) {
			return Err(AuthError::Config(
				"GitLab's address must be https, or http on localhost".into(),
			));
		}

		let mut provider = fixed(
			"gitlab",
			"GitLab",
			client_id.into(),
			String::new(),
			&["openid", "profile", "email"],
			Endpoints::fixed(
				&format!("{base}/oauth/authorize"),
				&format!("{base}/oauth/token"),
				Some(&format!("{base}/oauth/discovery/keys")),
				Some(base),
			),
			Quirks::default(),
		);
		provider.secret = client_secret.map_or(Secret::None, Secret::Shared);
		Ok(provider)
	}

	/// Sign in with Slack (OpenID Connect). A Slack user belongs to one
	/// workspace, so the identity is the workspace and the user together:
	/// the same person in two workspaces is two connected accounts.
	pub fn slack(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		fixed(
			"slack",
			"Slack",
			client_id.into(),
			client_secret.into(),
			&["openid", "email", "profile"],
			Endpoints::fixed(
				"https://slack.com/openid/connect/authorize",
				"https://slack.com/api/openid.connect.token",
				Some("https://slack.com/openid/connect/keys"),
				Some("https://slack.com"),
			),
			Quirks {
				identify: Identify::IdToken(IdTokenRules {
					scope_claim: Some("https://slack.com/team_id"),
					..IdTokenRules::default()
				}),
				..Quirks::default()
			},
		)
	}

	/// Twitch, through its OpenID Connect sign-in. Twitch puts the address
	/// and username in the ID token only when asked for each by name, with
	/// the `claims` parameter.
	pub fn twitch(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		let claims = r#"{"id_token":{"email":null,"email_verified":null,"picture":null,"preferred_username":null}}"#;
		fixed(
			"twitch",
			"Twitch",
			client_id.into(),
			client_secret.into(),
			&["openid", "user:read:email"],
			Endpoints::fixed(
				"https://id.twitch.tv/oauth2/authorize",
				"https://id.twitch.tv/oauth2/token",
				Some("https://id.twitch.tv/oauth2/keys"),
				Some("https://id.twitch.tv/oauth2"),
			),
			Quirks {
				authorize: vec![("claims", claims.to_owned())],
				..Quirks::default()
			},
		)
	}

	/// Spotify, with an app from its developer dashboard. Spotify doesn't
	/// check addresses, so none is trusted.
	pub fn spotify(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		fixed(
			"spotify",
			"Spotify",
			client_id.into(),
			client_secret.into(),
			&["user-read-email"],
			Endpoints::fixed(
				"https://accounts.spotify.com/authorize",
				"https://accounts.spotify.com/api/token",
				None,
				None,
			),
			Quirks {
				client_auth: ClientAuth::Basic,
				..api(ApiProfile {
					name: Some("/display_name"),
					picture: Some("/images/0/url"),
					email: Some("/email"),
					..ApiProfile::at("https://api.spotify.com/v1/me", "/id")
				})
			},
		)
	}

	/// Reddit, with a "web app". Reddit shares no address, and asks every
	/// app to name itself in `User-Agent`, which carries the app's name and
	/// nothing about the person.
	pub fn reddit(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		fixed(
			"reddit",
			"Reddit",
			client_id.into(),
			client_secret.into(),
			&["identity"],
			Endpoints::fixed(
				"https://www.reddit.com/api/v1/authorize",
				"https://www.reddit.com/api/v1/access_token",
				None,
				None,
			),
			Quirks {
				client_auth: ClientAuth::Basic,
				authorize: vec![("duration", "temporary".to_owned())],
				user_agent: true,
				..api(ApiProfile {
					username: Some("/name"),
					picture: Some("/icon_img"),
					..ApiProfile::at("https://oauth.reddit.com/api/v1/me", "/id")
				})
			},
		)
	}

	/// Login with Amazon, with a security profile's web client. Amazon
	/// doesn't say whether it checked an address, so none is trusted.
	pub fn amazon(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		fixed(
			"amazon",
			"Amazon",
			client_id.into(),
			client_secret.into(),
			&["profile"],
			Endpoints::fixed(
				"https://www.amazon.com/ap/oa",
				"https://api.amazon.com/auth/o2/token",
				None,
				None,
			),
			api(ApiProfile {
				name: Some("/name"),
				email: Some("/email"),
				..ApiProfile::at("https://api.amazon.com/user/profile", "/user_id")
			}),
		)
	}

	/// Dropbox, through its OpenID Connect sign-in, with `email_verified`.
	pub fn dropbox(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		fixed(
			"dropbox",
			"Dropbox",
			client_id.into(),
			client_secret.into(),
			&["openid", "profile", "email"],
			Endpoints::fixed(
				"https://www.dropbox.com/oauth2/authorize",
				"https://api.dropboxapi.com/oauth2/token",
				Some("https://www.dropbox.com/.well-known/jwks"),
				Some("https://www.dropbox.com"),
			),
			Quirks::default(),
		)
	}

	/// Bitbucket Cloud, with an OAuth consumer. Its addresses come from
	/// `/user/emails`, whose primary entry counts when Bitbucket confirmed
	/// it.
	pub fn bitbucket(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		fixed(
			"bitbucket",
			"Bitbucket",
			client_id.into(),
			client_secret.into(),
			&["account", "email"],
			Endpoints::fixed(
				"https://bitbucket.org/site/oauth2/authorize",
				"https://bitbucket.org/site/oauth2/access_token",
				None,
				None,
			),
			Quirks {
				client_auth: ClientAuth::Basic,
				..api(ApiProfile {
					name: Some("/display_name"),
					username: Some("/nickname"),
					picture: Some("/links/avatar/href"),
					emails: Some(EmailList {
						url: "https://api.bitbucket.org/2.0/user/emails",
						list: "/values",
						email: "/email",
						primary: "/is_primary",
						verified: "/is_confirmed",
					}),
					..ApiProfile::at("https://api.bitbucket.org/2.0/user", "/uuid")
				})
			},
		)
	}

	/// Notion, with a public integration. A grant is for one workspace, and
	/// its owner is the person who signed in: the token endpoint's answer
	/// names them, and a grant owned by anything but a person signs no one
	/// in. Notion doesn't say whether it checked their address, so none is
	/// trusted, and its web sign-in takes no PKCE.
	pub fn notion(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		fixed(
			"notion",
			"Notion",
			client_id.into(),
			client_secret.into(),
			&[],
			Endpoints::fixed(
				"https://api.notion.com/v1/oauth/authorize",
				"https://api.notion.com/v1/oauth/token",
				None,
				None,
			),
			Quirks {
				client_auth: ClientAuth::Basic,
				authorize: vec![("owner", "user".to_owned())],
				pkce: false,
				token_body: TokenBody::Json,
				..api(ApiProfile {
					url: None,
					name: Some("/owner/user/name"),
					picture: Some("/owner/user/avatar_url"),
					email: Some("/owner/user/person/email"),
					person: Some(("/owner/user/type", "person")),
					..ApiProfile::at("", "/owner/user/id")
				})
			},
		)
	}

	/// Zoom, with a general app's user-managed OAuth. Its `verified` flag
	/// (`1`) says whether it checked the address.
	pub fn zoom(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		fixed(
			"zoom",
			"Zoom",
			client_id.into(),
			client_secret.into(),
			&["user:read:user"],
			Endpoints::fixed(
				"https://zoom.us/oauth/authorize",
				"https://zoom.us/oauth/token",
				None,
				None,
			),
			Quirks {
				client_auth: ClientAuth::Basic,
				..api(ApiProfile {
					name: Some("/display_name"),
					picture: Some("/pic_url"),
					email: Some("/email"),
					verified: Some("/verified"),
					..ApiProfile::at("https://api.zoom.us/v2/users/me", "/id")
				})
			},
		)
	}

	/// TikTok, through Login Kit for the web, with an app's client key and
	/// secret. TikTok calls the client ID `client_key`, separates scopes
	/// with commas, takes no PKCE on the web, and shares no address.
	pub fn tiktok(client_key: impl Into<String>, client_secret: impl Into<String>) -> Self {
		fixed(
			"tiktok",
			"TikTok",
			client_key.into(),
			client_secret.into(),
			&["user.info.basic"],
			Endpoints::fixed(
				"https://www.tiktok.com/v2/auth/authorize/",
				"https://open.tiktokapis.com/v2/oauth/token/",
				None,
				None,
			),
			Quirks {
				client_id_param: "client_key",
				scope_separator: ",",
				pkce: false,
				..api(ApiProfile {
					name: Some("/data/user/display_name"),
					picture: Some("/data/user/avatar_url"),
					..ApiProfile::at(
						"https://open.tiktokapis.com/v2/user/info/?fields=open_id,union_id,avatar_url,display_name",
						"/data/user/open_id",
					)
				})
			},
		)
	}

	/// Yahoo, through its OpenID Connect sign-in, with `email_verified`.
	pub fn yahoo(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		fixed(
			"yahoo",
			"Yahoo",
			client_id.into(),
			client_secret.into(),
			&["openid", "email", "profile"],
			Endpoints::fixed(
				"https://api.login.yahoo.com/oauth2/request_auth",
				"https://api.login.yahoo.com/oauth2/get_token",
				Some("https://api.login.yahoo.com/openid/v1/certs"),
				Some("https://api.login.yahoo.com"),
			),
			Quirks::default(),
		)
	}
}
