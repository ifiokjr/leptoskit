//! Third-party sign-in: OpenID Connect and OAuth 2 providers, as a client.
//!
//! The flow is the authorization code grant with PKCE (`S256`), `state`, and
//! for OpenID Connect a `nonce`. What the flow needs to finish (the PKCE
//! verifier, the nonce, where to return, and whether it signs in or connects
//! an account) is sealed in `auth.challenges`, bound to a random token in an
//! `HttpOnly` cookie, so a callback only finishes in the browser that
//! started it. Nothing about the flow is kept in script-readable storage.
//!
//! ID tokens are checked here with `ring`: the signature against the
//! provider's published keys (fetched again when an unknown key ID appears),
//! then `iss`, `aud`, `azp`, `exp`, `iat`, `nbf`, and the nonce. Only
//! asymmetric algorithms are accepted; `none` and HMAC never are.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;
use std::time::Instant;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use subtle::ConstantTimeEq;

use crate::Auth;
use crate::AuthError;
use crate::ProviderInfo;
use crate::Purpose;
use crate::RequestInfo;
use crate::Result;
use crate::SessionToken;
use crate::SignedIn;
use crate::Transport;
use crate::providers::CallbackParams;
use crate::providers::Flow;
use crate::providers::Intent;
use crate::providers::ProviderProfile;
use crate::secrets;

mod catalog;
mod profiles;

pub use catalog::FACEBOOK_GRAPH_VERSION;
pub use catalog::GITLAB_COM;
pub(crate) use catalog::built_in;
use profiles::ApiProfile;

/// How long a provider has to send the person back: long enough to sign
/// in there, with its own second factor.
const FLOW_LIFETIME: chrono::TimeDelta = chrono::TimeDelta::minutes(10);

/// How long a fetched key set or discovery document is trusted before it's
/// fetched again.
const CACHE_LIFETIME: Duration = Duration::from_hours(24);

/// How soon an unknown key ID may fetch the key set again, so forged
/// tokens can't make the server hammer the provider.
const REFETCH_AFTER: Duration = Duration::from_secs(60);

/// How far clocks may disagree when checking `exp`, `iat`, and `nbf`.
const SKEW: i64 = 60;

/// The most a provider's answer may hold.
const MAX_BODY: usize = 1024 * 1024;

/// A third-party sign-in provider: what it is, and the app's credentials
/// there.
///
/// ```ignore
/// let auth = Auth::builder(&db, keys)
/// 	.origin("https://myapp.com")
/// 	.provider(Provider::google(env("GOOGLE_CLIENT_ID")?, env("GOOGLE_CLIENT_SECRET")?))
/// 	.provider(Provider::oidc("company", "Company SSO", "https://sso.example.com", client_id, Some(secret))?)
/// 	.build()?;
/// ```
///
/// Its `Debug` leaves the secrets out.
pub struct Provider {
	pub(crate) id: String,
	pub(crate) name: String,
	pub(crate) flavor: Flavor,
	pub(crate) client_id: String,
	secret: Secret,
	pub(crate) scopes: Vec<String>,
	endpoints: Option<Endpoints>,
	/// Whether its tokens are kept, for the app to call its API.
	pub(crate) store_tokens: bool,
	quirks: Quirks,
}

impl fmt::Debug for Provider {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("Provider")
			.field("id", &self.id)
			.field("name", &self.name)
			.field("client_id", &self.client_id)
			.finish_non_exhaustive()
	}
}

/// What a provider needs beyond plain OpenID Connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Flavor {
	/// OpenID Connect, with the endpoints from its discovery document.
	Oidc { issuer: String },
	/// Google: OpenID Connect, with `email_verified`.
	Google,
	/// Sign in with Apple: an ES256 client secret minted per request, the
	/// response posted back as a form, and the name only on the first
	/// sign-in.
	Apple,
	/// Microsoft: the issuer names the person's tenant, and the email claim
	/// is never trusted.
	Microsoft { tenant: String },
	/// Fixed endpoints, whose issuer (if it issues ID tokens) is fixed too.
	Fixed,
}

/// How a provider differs from plain OpenID Connect with a client secret in
/// the form: data, so each provider is a definition rather than code.
#[derive(Clone, Debug)]
pub(crate) struct Quirks {
	/// How it says who signed in.
	identify: Identify,
	/// How the app proves who it is at the token endpoint.
	client_auth: ClientAuth,
	/// Whether connecting an account asks it to show its account picker,
	/// with `prompt=select_account`. Only providers known to accept it.
	select_account: bool,
	/// More parameters for its authorization page, such as Twitch's
	/// `claims`.
	authorize: Vec<(&'static str, String)>,
	/// Whether its requests must name the app in `User-Agent`, as Reddit
	/// asks: `web:<app name>:leptoskit-auth`, the app's name only, never
	/// anything about the person.
	user_agent: bool,
	/// What it calls the client ID: `client_id`, or TikTok's `client_key`.
	client_id_param: &'static str,
	/// What separates scopes: a space, or TikTok's comma.
	scope_separator: &'static str,
	/// Whether it takes PKCE. Every provider does except Notion's and
	/// TikTok's web sign-in, which don't accept it; there the code is bound
	/// to the browser by `state` and the binding cookie alone, and only the
	/// server can trade it, with the client secret.
	pkce: bool,
	/// How the token endpoint takes its fields.
	token_body: TokenBody,
	/// What a sign-in asks for to get a refresh token, when the app keeps
	/// tokens.
	offline: Offline,
}

impl Default for Quirks {
	fn default() -> Self {
		Self {
			identify: Identify::IdToken(IdTokenRules::default()),
			client_auth: ClientAuth::Form,
			select_account: false,
			authorize: Vec::new(),
			user_agent: false,
			client_id_param: "client_id",
			scope_separator: " ",
			pkce: true,
			token_body: TokenBody::Form,
			offline: Offline::Given,
		}
	}
}

/// How a provider says who signed in.
#[derive(Clone, Debug)]
pub(crate) enum Identify {
	/// An OpenID Connect ID token from the token endpoint.
	IdToken(IdTokenRules),
	/// Its own API, read with the access token.
	Api(Box<ApiProfile>),
}

/// What an ID token needs beyond the standard checks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct IdTokenRules {
	/// Whether its `email_verified` is believed. Microsoft's never is.
	trust_email: bool,
	/// Whether it may leave out the nonce, which LinkedIn's always does. Its
	/// token still comes straight from the token endpoint, for a code bound
	/// to this browser by `state`; a nonce it does send must match.
	nonce_optional: bool,
	/// A claim that scopes the subject, such as Slack's workspace: the
	/// identity is `{claim}:{sub}`, and a token without the claim is
	/// refused.
	scope_claim: Option<&'static str>,
}

impl Default for IdTokenRules {
	fn default() -> Self {
		Self {
			trust_email: true,
			nonce_optional: false,
			scope_claim: None,
		}
	}
}

/// What a sign-in asks for to get a refresh token, for a provider whose
/// tokens the app keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Offline {
	/// Nothing: it gives one when it gives any.
	Given,
	/// A parameter on the authorization page, such as Google's
	/// `access_type=offline`. It replaces a quirk's own parameter of the
	/// same name, such as Reddit's `duration`.
	Param(&'static str, &'static str),
	/// A scope, such as OpenID Connect's `offline_access`.
	Scope(&'static str),
}

/// How a token endpoint takes its fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TokenBody {
	/// A form, as OAuth 2 says.
	Form,
	/// JSON, as Notion's takes.
	Json,
}

/// How the app authenticates at the token endpoint, with a shared secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClientAuth {
	/// `client_secret` in the form (`client_secret_post`).
	Form,
	/// HTTP Basic (`client_secret_basic`), which X, Spotify, Reddit,
	/// Bitbucket, Notion, and Zoom require.
	Basic,
}

/// How the app proves who it is to the provider.
enum Secret {
	/// A public client: PKCE alone.
	None,
	/// A client secret.
	Shared(String),
	/// Sign in with Apple's `.p8` key, which signs a short-lived client
	/// secret for each request.
	Apple {
		team_id: String,
		key_id: String,
		key: Box<ring::signature::EcdsaKeyPair>,
	},
}

/// A provider's endpoints.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct Endpoints {
	#[serde(default)]
	issuer: Option<String>,
	authorization_endpoint: String,
	token_endpoint: String,
	#[serde(default)]
	jwks_uri: Option<String>,
	#[serde(default)]
	userinfo_endpoint: Option<String>,
	/// Where tokens are revoked (RFC 7009), when the provider has such a
	/// place.
	#[serde(default)]
	revocation_endpoint: Option<String>,
}

impl Endpoints {
	fn fixed(authorization: &str, token: &str, jwks: Option<&str>, issuer: Option<&str>) -> Self {
		Self {
			issuer: issuer.map(str::to_owned),
			authorization_endpoint: authorization.to_owned(),
			token_endpoint: token.to_owned(),
			jwks_uri: jwks.map(str::to_owned),
			userinfo_endpoint: None,
			revocation_endpoint: None,
		}
	}

	/// Revokes tokens at `url`.
	fn revoked_at(mut self, url: &str) -> Self {
		self.revocation_endpoint = Some(url.to_owned());
		self
	}
}

/// When each built-in definition was last checked against the provider's
/// documentation. `lk auth doctor` warns when one is a year old.
pub const VERIFIED: &[(&str, &str)] = &[
	("google", "2026-10-30"),
	("apple", "2026-10-30"),
	("microsoft", "2026-10-30"),
	("github", "2026-10-30"),
	("facebook", "2026-10-31"),
	("x", "2026-10-31"),
	("discord", "2026-10-31"),
	("linkedin", "2026-10-31"),
	("gitlab", "2026-10-31"),
	("slack", "2026-10-31"),
	("twitch", "2026-10-31"),
	("spotify", "2026-10-31"),
	("reddit", "2026-10-31"),
	("amazon", "2026-10-31"),
	("dropbox", "2026-10-31"),
	("bitbucket", "2026-10-31"),
	("notion", "2026-10-31"),
	("zoom", "2026-10-31"),
	("tiktok", "2026-10-31"),
	("yahoo", "2026-10-31"),
];

/// Microsoft's tenant for personal accounts.
const MICROSOFT_CONSUMERS: &str = "9188040d-6c67-4c5b-b112-36a304b66dad";

impl Provider {
	/// Google, with an OAuth client of type "Web application".
	pub fn google(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		Self {
			id: "google".into(),
			name: "Google".into(),
			flavor: Flavor::Google,
			client_id: client_id.into(),
			secret: Secret::Shared(client_secret.into()),
			scopes: vec!["openid".into(), "email".into(), "profile".into()],
			endpoints: Some(
				Endpoints::fixed(
					"https://accounts.google.com/o/oauth2/v2/auth",
					"https://oauth2.googleapis.com/token",
					Some("https://www.googleapis.com/oauth2/v3/certs"),
					Some("https://accounts.google.com"),
				)
				.revoked_at("https://oauth2.googleapis.com/revoke"),
			),
			store_tokens: false,
			quirks: Quirks {
				select_account: true,
				offline: Offline::Param("access_type", "offline"),
				..Quirks::default()
			},
		}
	}

	/// Sign in with Apple, with a Services ID, the team that owns it, and a
	/// Sign in with Apple key: its ID and its `.p8` file's contents.
	///
	/// # Errors
	///
	/// [`AuthError::Config`] when the key isn't a PKCS#8 P-256 private key.
	pub fn apple(
		services_id: impl Into<String>,
		team_id: impl Into<String>,
		key_id: impl Into<String>,
		private_key_pem: &str,
	) -> Result<Self> {
		let der = pem_body(private_key_pem)
			.ok_or_else(|| AuthError::Config("the Apple key isn't a .p8 file".into()))?;
		let key = ring::signature::EcdsaKeyPair::from_pkcs8(
			&ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
			&der,
			&ring::rand::SystemRandom::new(),
		)
		.map_err(|_| AuthError::Config("the Apple key isn't a P-256 private key".into()))?;
		Ok(Self {
			id: "apple".into(),
			name: "Apple".into(),
			flavor: Flavor::Apple,
			client_id: services_id.into(),
			secret: Secret::Apple {
				team_id: team_id.into(),
				key_id: key_id.into(),
				key: Box::new(key),
			},
			scopes: vec!["name".into(), "email".into()],
			endpoints: Some(
				Endpoints::fixed(
					"https://appleid.apple.com/auth/authorize",
					"https://appleid.apple.com/auth/token",
					Some("https://appleid.apple.com/auth/keys"),
					Some("https://appleid.apple.com"),
				)
				.revoked_at("https://appleid.apple.com/auth/revoke"),
			),
			store_tokens: false,
			quirks: Quirks::default(),
		})
	}

	/// Microsoft accounts. `tenant` is `common` (work, school, and personal
	/// accounts), `organizations`, `consumers`, or one tenant's ID.
	pub fn microsoft(
		client_id: impl Into<String>,
		client_secret: impl Into<String>,
		tenant: impl Into<String>,
	) -> Self {
		let tenant = tenant.into();
		let base = format!("https://login.microsoftonline.com/{tenant}");
		Self {
			id: "microsoft".into(),
			name: "Microsoft".into(),
			flavor: Flavor::Microsoft { tenant },
			client_id: client_id.into(),
			secret: Secret::Shared(client_secret.into()),
			scopes: vec!["openid".into(), "email".into(), "profile".into()],
			endpoints: Some(Endpoints::fixed(
				&format!("{base}/oauth2/v2.0/authorize"),
				&format!("{base}/oauth2/v2.0/token"),
				Some(&format!("{base}/discovery/v2.0/keys")),
				None,
			)),
			store_tokens: false,
			quirks: Quirks {
				identify: Identify::IdToken(IdTokenRules {
					trust_email: false,
					..IdTokenRules::default()
				}),
				select_account: true,
				offline: Offline::Scope("offline_access"),
				..Quirks::default()
			},
		}
	}

	/// GitHub, with an OAuth app.
	pub fn github(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
		Self {
			id: "github".into(),
			name: "GitHub".into(),
			flavor: Flavor::Fixed,
			client_id: client_id.into(),
			secret: Secret::Shared(client_secret.into()),
			scopes: vec!["read:user".into(), "user:email".into()],
			endpoints: Some(Endpoints::fixed(
				"https://github.com/login/oauth/authorize",
				"https://github.com/login/oauth/access_token",
				None,
				None,
			)),
			store_tokens: false,
			quirks: Quirks {
				identify: Identify::Api(Box::new(ApiProfile::github())),
				select_account: true,
				..Quirks::default()
			},
		}
	}

	/// Any OpenID Connect provider, such as Okta, Auth0, Keycloak, or a
	/// company's Entra tenant, found from its issuer's discovery document.
	/// `id` names it in routes and `[auth.providers]`; `name` is for people.
	/// Without a secret, it's a public client, and PKCE alone protects it.
	///
	/// # Errors
	///
	/// [`AuthError::Config`] for an ID that isn't lowercase letters, digits,
	/// and dashes, or an issuer that isn't `https` (or `http` on
	/// localhost).
	pub fn oidc(
		id: impl Into<String>,
		name: impl Into<String>,
		issuer: impl Into<String>,
		client_id: impl Into<String>,
		client_secret: Option<String>,
	) -> Result<Self> {
		let id = id.into();
		let issuer = issuer.into().trim_end_matches('/').to_owned();

		if !valid_id(&id) {
			return Err(AuthError::Config(format!(
				"provider ID `{id}` must be lowercase letters, digits, and dashes"
			)));
		}

		if !secure_url(&issuer) {
			return Err(AuthError::Config(format!(
				"the issuer of `{id}` must be https, or http on localhost"
			)));
		}

		Ok(Self {
			id,
			name: name.into(),
			flavor: Flavor::Oidc { issuer },
			client_id: client_id.into(),
			secret: client_secret.map_or(Secret::None, Secret::Shared),
			scopes: vec!["openid".into(), "email".into(), "profile".into()],
			endpoints: None,
			store_tokens: false,
			quirks: Quirks {
				select_account: true,
				offline: Offline::Scope("offline_access"),
				..Quirks::default()
			},
		})
	}

	/// Asks for these scopes instead of the defaults. OpenID Connect
	/// providers always get `openid`.
	#[must_use]
	pub fn scopes<S: Into<String>>(mut self, scopes: impl IntoIterator<Item = S>) -> Self {
		self.scopes = scopes.into_iter().map(Into::into).collect();

		if self.asks_for_openid() && !self.scopes.iter().any(|scope| scope == "openid") {
			self.scopes.insert(0, "openid".into());
		}

		self
	}

	/// Keeps its access and refresh tokens, sealed, so the app can call its
	/// API on the person's behalf with
	/// [`Auth::provider_access_token`](crate::Auth::provider_access_token).
	/// Off by default: an app that only signs people in has no use for
	/// them. Where the provider needs asking for a refresh token, such as
	/// Google's `access_type=offline`, sign-in asks.
	#[must_use]
	pub fn store_tokens(mut self, store: bool) -> Self {
		self.store_tokens = store;
		self
	}

	/// Shows it to people as `name`, such as "Company SSO".
	#[must_use]
	pub fn name(mut self, name: impl Into<String>) -> Self {
		self.name = name.into();
		self
	}

	/// Its ID, such as `google`.
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Names it `id` in routes and `[auth.providers]`, for a built-in
	/// provider configured under another name.
	pub(crate) fn with_id(mut self, id: impl Into<String>) -> Self {
		self.id = id.into();
		self
	}

	/// Whether it signs in with OpenID Connect's `openid` scope: every
	/// provider that issues ID tokens, except Apple, which has its own.
	fn asks_for_openid(&self) -> bool {
		matches!(self.quirks.identify, Identify::IdToken(_))
			&& !matches!(self.flavor, Flavor::Apple)
	}

	/// The rules its ID tokens follow, if it issues them.
	fn id_token_rules(&self) -> Option<IdTokenRules> {
		match &self.quirks.identify {
			Identify::IdToken(rules) => Some(*rules),
			Identify::Api(_) => None,
		}
	}

	/// The issuer its ID tokens must name, for the token's `tid` when it's
	/// Microsoft.
	fn issuer(&self, endpoints: &Endpoints, tid: Option<&str>) -> Option<String> {
		match &self.flavor {
			Flavor::Oidc { issuer } => Some(issuer.clone()),
			Flavor::Microsoft { tenant } => {
				let tid = tid?;
				let allowed = match tenant.as_str() {
					"common" => true,
					"consumers" => tid == MICROSOFT_CONSUMERS,
					"organizations" => tid != MICROSOFT_CONSUMERS,
					tenant => tid.eq_ignore_ascii_case(tenant),
				};

				allowed.then(|| format!("https://login.microsoftonline.com/{tid}/v2.0"))
			}
			_ => endpoints.issuer.clone(),
		}
	}
}

/// Whether `id` can name a provider in a route.
pub(crate) fn valid_id(id: &str) -> bool {
	!id.is_empty()
		&& id.len() <= 64
		&& id
			.bytes()
			.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// Whether `url` is `https`, or `http` on localhost.
fn secure_url(url: &str) -> bool {
	let Ok(url) = url::Url::parse(url) else {
		return false;
	};
	let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
	url.scheme() == "https" || (url.scheme() == "http" && local)
}

/// The base64 inside a PEM file.
fn pem_body(pem: &str) -> Option<Vec<u8>> {
	let body: String = pem
		.lines()
		.map(str::trim)
		.filter(|line| !line.starts_with("-----"))
		.collect();
	STANDARD.decode(body).ok().filter(|der| !der.is_empty())
}

/// A request to a provider.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderRequest {
	/// `GET` or `POST`.
	pub method: &'static str,
	/// The whole URL.
	pub url: String,
	/// The headers, such as `accept` and `authorization`.
	pub headers: Vec<(String, String)>,
	/// A form body, already encoded.
	pub body: Option<String>,
}

impl fmt::Debug for ProviderRequest {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("ProviderRequest")
			.field("method", &self.method)
			.field("url", &self.url)
			.finish_non_exhaustive()
	}
}

/// A provider's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderResponse {
	/// The status code.
	pub status: u16,
	/// The body.
	pub body: Vec<u8>,
}

/// A future a [`ProviderHttp`] returns.
pub type ProviderFuture<'a> =
	Pin<Box<dyn Future<Output = std::result::Result<ProviderResponse, String>> + Send + 'a>>;

/// Sends requests to providers. The default uses `reqwest`, with a ten
/// second timeout and no redirects; tests answer from memory.
pub trait ProviderHttp: Send + Sync + 'static {
	/// Sends `request`. An `Err` is a network failure, described for the
	/// server's logs.
	fn send(&self, request: ProviderRequest) -> ProviderFuture<'_>;
}

/// The network, through `reqwest`.
struct Network(reqwest::Client);

impl ProviderHttp for Network {
	fn send(&self, request: ProviderRequest) -> ProviderFuture<'_> {
		Box::pin(async move {
			let method = if request.method == "POST" {
				reqwest::Method::POST
			} else {
				reqwest::Method::GET
			};

			let mut builder = self.0.request(method, &request.url);

			for (name, value) in &request.headers {
				builder = builder.header(name, value);
			}

			if let Some(body) = request.body {
				builder = builder.body(body);
			}

			let mut response = builder.send().await.map_err(|error| error.to_string())?;
			let status = response.status().as_u16();
			let mut body = Vec::new();

			while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
				body.extend_from_slice(&chunk);

				if body.len() > MAX_BODY {
					return Err("the provider's answer was too large".into());
				}
			}

			Ok(ProviderResponse { status, body })
		})
	}
}

/// The configured providers, and what's been fetched from them.
pub(crate) struct Providers {
	list: Vec<Provider>,
	http: Arc<dyn ProviderHttp>,
	discovered: Mutex<HashMap<String, (Instant, Endpoints)>>,
	keys: Mutex<HashMap<String, (Instant, Jwks)>>,
}

impl Providers {
	pub(crate) fn new(list: Vec<Provider>, http: Option<Arc<dyn ProviderHttp>>) -> Self {
		let http = http.unwrap_or_else(|| {
			let client = reqwest::Client::builder()
				.timeout(Duration::from_secs(10))
				.redirect(reqwest::redirect::Policy::none())
				.user_agent(concat!("leptoskit-auth/", env!("CARGO_PKG_VERSION")))
				.build()
				.unwrap_or_default();
			Arc::new(Network(client))
		});
		Self {
			list,
			http,
			discovered: Mutex::new(HashMap::new()),
			keys: Mutex::new(HashMap::new()),
		}
	}

	pub(crate) fn get(&self, id: &str) -> Option<&Provider> {
		self.list.iter().find(|provider| provider.id == id)
	}

	pub(crate) fn info(&self) -> Vec<ProviderInfo> {
		self.list
			.iter()
			.map(|provider| {
				ProviderInfo {
					id: provider.id.clone(),
					name: provider.name.clone(),
				}
			})
			.collect()
	}

	async fn send(&self, request: ProviderRequest) -> Result<ProviderResponse> {
		let url = request.url.clone();
		self.http
			.send(request)
			.await
			.map_err(|error| AuthError::Provider(format!("{url}: {error}")))
	}

	async fn json<T: DeserializeOwned>(&self, request: ProviderRequest) -> Result<T> {
		let url = request.url.clone();
		let response = self.send(request).await?;

		if !(200..300).contains(&response.status) {
			return Err(AuthError::Provider(format!(
				"{url} answered {}",
				response.status
			)));
		}

		serde_json::from_slice(&response.body).map_err(|error| {
			AuthError::Provider(format!("{url} answered unreadable JSON: {error}"))
		})
	}

	/// The provider's endpoints: fixed, or from its discovery document,
	/// whose issuer must be exactly the one configured.
	async fn endpoints(&self, provider: &Provider) -> Result<Endpoints> {
		if let Some(endpoints) = &provider.endpoints {
			return Ok(endpoints.clone());
		}

		let Flavor::Oidc { issuer } = &provider.flavor else {
			return Err(AuthError::Config(format!(
				"`{}` has no endpoints",
				provider.id
			)));
		};

		if let Some((at, endpoints)) = lock(&self.discovered).get(issuer)
			&& at.elapsed() < CACHE_LIFETIME
		{
			return Ok(endpoints.clone());
		}

		let endpoints: Endpoints = self
			.json(get(&format!("{issuer}/.well-known/openid-configuration")))
			.await?;

		if endpoints.issuer.as_deref() != Some(issuer.as_str()) {
			return Err(AuthError::Provider(format!(
				"{issuer}'s discovery document names another issuer"
			)));
		}

		let urls = [
			Some(&endpoints.authorization_endpoint),
			Some(&endpoints.token_endpoint),
			endpoints.jwks_uri.as_ref(),
			endpoints.revocation_endpoint.as_ref(),
		];

		if urls.into_iter().flatten().any(|url| !secure_url(url)) || endpoints.jwks_uri.is_none() {
			return Err(AuthError::Provider(format!(
				"{issuer}'s discovery document has an endpoint that isn't https, or no keys"
			)));
		}

		lock(&self.discovered).insert(issuer.clone(), (Instant::now(), endpoints.clone()));
		Ok(endpoints)
	}

	/// The key `kid` from the key set at `uri`, fetching the set again when
	/// it's stale, or when it lacks `kid` and wasn't fetched in the last
	/// minute.
	async fn key(&self, uri: &str, kid: Option<&str>) -> Result<Jwk> {
		let cached = lock(&self.keys).get(uri).cloned();

		if let Some((at, keys)) = &cached
			&& at.elapsed() < CACHE_LIFETIME
		{
			if let Some(key) = keys.find(kid) {
				return Ok(key.clone());
			}

			if at.elapsed() < REFETCH_AFTER {
				return Err(AuthError::Provider(
					"the ID token's key isn't published".into(),
				));
			}
		}

		let keys: Jwks = self.json(get(uri)).await?;
		let key = keys.find(kid).cloned();
		lock(&self.keys).insert(uri.to_owned(), (Instant::now(), keys));
		key.ok_or_else(|| AuthError::Provider("the ID token's key isn't published".into()))
	}
}

impl Providers {
	/// Trades `refresh` for new tokens at `provider`. `None` when the
	/// provider ended the grant (`invalid_grant`), so asking again won't
	/// help.
	pub(crate) async fn refresh(
		&self,
		provider: &Provider,
		refresh: &str,
		user_agent: Option<(String, String)>,
	) -> Result<Option<crate::providers::ProviderGrant>> {
		let endpoints = self.endpoints(provider).await?;
		let fields = vec![
			("grant_type", "refresh_token".to_owned()),
			("refresh_token", refresh.to_owned()),
		];
		let mut request = token_request(provider, &endpoints.token_endpoint, fields)?;
		request.headers.extend(user_agent);
		let url = request.url.clone();
		let response = self.send(request).await?;
		let answer: serde_json::Value = serde_json::from_slice(&response.body).unwrap_or_default();
		let error = answer.get("error").filter(|error| !error.is_null());

		if error.and_then(serde_json::Value::as_str) == Some("invalid_grant") {
			return Ok(None);
		}

		if let Some(error) = error {
			return Err(AuthError::Provider(format!(
				"{url} refused to refresh: {}",
				describe(error)
			)));
		}

		if !(200..300).contains(&response.status) {
			return Err(AuthError::Provider(format!(
				"{url} answered {}",
				response.status
			)));
		}

		grant(&answer)
			.map(Some)
			.ok_or_else(|| AuthError::Provider(format!("{url} answered without an access token")))
	}

	/// Revokes `token` (RFC 7009) at `provider`, when it has somewhere to
	/// do that.
	pub(crate) async fn revoke(
		&self,
		provider: &Provider,
		token: &str,
		refresh: bool,
	) -> Result<()> {
		let endpoints = self.endpoints(provider).await?;
		let Some(url) = &endpoints.revocation_endpoint else {
			return Ok(());
		};
		let hint = if refresh {
			"refresh_token"
		} else {
			"access_token"
		};

		let fields = vec![
			("token", token.to_owned()),
			("token_type_hint", hint.to_owned()),
		];
		let response = self.send(token_request(provider, url, fields)?).await?;

		if (200..300).contains(&response.status) {
			Ok(())
		} else {
			Err(AuthError::Provider(format!(
				"{url} answered {}",
				response.status
			)))
		}
	}
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
	mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn get(url: &str) -> ProviderRequest {
	ProviderRequest {
		method: "GET",
		url: url.to_owned(),
		headers: vec![("accept".into(), "application/json".into())],
		body: None,
	}
}

/// A JSON Web Key Set.
#[derive(Clone, Debug, Deserialize)]
struct Jwks {
	keys: Vec<Jwk>,
}

impl Jwks {
	/// The signing key named `kid`, or the only one when the token names
	/// none.
	fn find(&self, kid: Option<&str>) -> Option<&Jwk> {
		let signing = self
			.keys
			.iter()
			.filter(|key| key.usage.as_deref().is_none_or(|usage| usage == "sig"));

		if let Some(kid) = kid {
			return signing
				.into_iter()
				.find(|key| key.kid.as_deref() == Some(kid));
		}

		let keys: Vec<&Jwk> = signing.collect();
		(keys.len() == 1).then(|| keys[0])
	}
}

/// A public key, as JSON.
#[derive(Clone, Debug, Deserialize)]
struct Jwk {
	kty: String,
	#[serde(default)]
	kid: Option<String>,
	#[serde(default, rename = "use")]
	usage: Option<String>,
	#[serde(default)]
	alg: Option<String>,
	#[serde(default)]
	crv: Option<String>,
	#[serde(default)]
	n: Option<String>,
	#[serde(default)]
	e: Option<String>,
	#[serde(default)]
	x: Option<String>,
	#[serde(default)]
	y: Option<String>,
}

#[derive(Deserialize)]
struct JwsHeader {
	alg: String,
	#[serde(default)]
	kid: Option<String>,
	#[serde(default)]
	crit: Option<serde_json::Value>,
}

/// The JWS compact serialization's three parts, decoded: header, payload,
/// and signature, with the signed input.
struct Jws {
	header: JwsHeader,
	payload: Vec<u8>,
	signature: Vec<u8>,
	signed: String,
}

fn parse_jws(token: &str) -> Result<Jws> {
	let bad = || AuthError::Provider("the ID token isn't a signed JWT".into());
	let mut parts = token.split('.');
	let (Some(header), Some(payload), Some(signature), None) =
		(parts.next(), parts.next(), parts.next(), parts.next())
	else {
		return Err(bad());
	};
	let decoded_header = URL_SAFE_NO_PAD.decode(header).map_err(|_| bad())?;
	let header_json: JwsHeader = serde_json::from_slice(&decoded_header).map_err(|_| bad())?;

	// An extension the token says must be understood, and this doesn't.
	if header_json.crit.is_some() {
		return Err(bad());
	}

	Ok(Jws {
		header: header_json,
		payload: URL_SAFE_NO_PAD.decode(payload).map_err(|_| bad())?,
		signature: URL_SAFE_NO_PAD.decode(signature).map_err(|_| bad())?,
		signed: format!("{header}.{payload}"),
	})
}

/// Checks `jws`'s signature with `key`, for the algorithm its header names,
/// which must also be the key's own.
fn verify_signature(jws: &Jws, key: &Jwk) -> Result<()> {
	use ring::signature;

	let refused = || AuthError::Provider("the ID token's signature didn't check out".into());
	let alg = jws.header.alg.as_str();

	if key.alg.as_deref().is_some_and(|key_alg| key_alg != alg) {
		return Err(refused());
	}

	let decode = |field: &Option<String>| {
		field
			.as_deref()
			.and_then(|value| URL_SAFE_NO_PAD.decode(value).ok())
			.ok_or_else(refused)
	};
	let message = jws.signed.as_bytes();
	let signature = jws.signature.as_slice();

	let rsa = |parameters: &'static signature::RsaParameters| {
		if key.kty != "RSA" {
			return Err(refused());
		}

		let n = decode(&key.n)?;
		let e = decode(&key.e)?;
		signature::RsaPublicKeyComponents { n: &n, e: &e }
			.verify(parameters, message, signature)
			.map_err(|_| refused())
	};
	let ec =
		|algorithm: &'static signature::EcdsaVerificationAlgorithm, curve: &str, size: usize| {
			if key.kty != "EC" || key.crv.as_deref() != Some(curve) {
				return Err(refused());
			}

			let x = decode(&key.x)?;
			let y = decode(&key.y)?;

			if x.len() != size || y.len() != size {
				return Err(refused());
			}

			let mut point = Vec::with_capacity(1 + 2 * size);
			point.push(4);
			point.extend_from_slice(&x);
			point.extend_from_slice(&y);
			signature::UnparsedPublicKey::new(algorithm, point)
				.verify(message, signature)
				.map_err(|_| refused())
		};

	match alg {
		"RS256" => rsa(&signature::RSA_PKCS1_2048_8192_SHA256),
		"RS384" => rsa(&signature::RSA_PKCS1_2048_8192_SHA384),
		"RS512" => rsa(&signature::RSA_PKCS1_2048_8192_SHA512),
		"PS256" => rsa(&signature::RSA_PSS_2048_8192_SHA256),
		"PS384" => rsa(&signature::RSA_PSS_2048_8192_SHA384),
		"PS512" => rsa(&signature::RSA_PSS_2048_8192_SHA512),
		"ES256" => ec(&signature::ECDSA_P256_SHA256_FIXED, "P-256", 32),
		"ES384" => ec(&signature::ECDSA_P384_SHA384_FIXED, "P-384", 48),
		"EdDSA" => {
			if key.kty != "OKP" || key.crv.as_deref() != Some("Ed25519") {
				return Err(refused());
			}

			signature::UnparsedPublicKey::new(&signature::ED25519, decode(&key.x)?)
				.verify(message, signature)
				.map_err(|_| refused())
		}
		// `none`, HMAC, and anything unknown.
		_ => Err(refused()),
	}
}

/// `aud`: one audience, or several.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Audience {
	One(String),
	Many(Vec<String>),
}

impl Audience {
	fn contains(&self, client_id: &str) -> bool {
		match self {
			Self::One(audience) => audience == client_id,
			Self::Many(audiences) => audiences.iter().any(|audience| audience == client_id),
		}
	}

	fn several(&self) -> bool {
		matches!(self, Self::Many(audiences) if audiences.len() > 1)
	}
}

/// A boolean claim that some providers, such as Apple, send as a string.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Flag {
	Bool(bool),
	Text(String),
}

impl Flag {
	fn is_true(&self) -> bool {
		match self {
			Self::Bool(value) => *value,
			Self::Text(value) => value == "true",
		}
	}
}

/// The claims an ID token carries that sign-in reads. Times are integers:
/// a token with `"exp": "never"` doesn't parse, so it can't skip the check.
#[derive(Debug, Deserialize)]
struct IdClaims {
	iss: String,
	sub: String,
	aud: Audience,
	exp: i64,
	iat: i64,
	#[serde(default)]
	nbf: Option<i64>,
	#[serde(default)]
	nonce: Option<String>,
	#[serde(default)]
	azp: Option<String>,
	#[serde(default)]
	tid: Option<String>,
	#[serde(default)]
	email: Option<String>,
	#[serde(default)]
	email_verified: Option<Flag>,
	#[serde(default)]
	name: Option<String>,
	#[serde(default)]
	preferred_username: Option<String>,
	#[serde(default)]
	picture: Option<String>,
	/// Everything else, for a provider's own claims, such as Slack's
	/// workspace.
	#[serde(flatten)]
	other: serde_json::Map<String, serde_json::Value>,
}

/// Checks an ID token from `provider`: its signature, then its claims.
async fn verify_id_token(
	providers: &Providers,
	provider: &Provider,
	endpoints: &Endpoints,
	token: &str,
	expected: Expected<'_>,
) -> Result<IdClaims> {
	let jws = parse_jws(token)?;
	let uri = endpoints
		.jwks_uri
		.as_deref()
		.ok_or_else(|| AuthError::Config(format!("`{}` publishes no keys", provider.id)))?;
	let key = providers.key(uri, jws.header.kid.as_deref()).await?;
	verify_signature(&jws, &key)?;
	let claims: IdClaims = serde_json::from_slice(&jws.payload)
		.map_err(|_| AuthError::Provider("the ID token's claims are malformed".into()))?;
	check_claims(provider, endpoints, &claims, expected)?;
	Ok(claims)
}

/// What an ID token must say besides its issuer: who it's for, the flow's
/// nonce, and the time now, in seconds since the epoch.
#[derive(Clone, Copy, Debug)]
struct Expected<'a> {
	/// The client it was issued to: the provider's client ID for the web,
	/// or the app's bundle ID for Sign in with Apple on the device.
	audience: &'a str,
	nonce: &'a str,
	/// Whether a token without a nonce passes. One with a nonce must match
	/// either way.
	nonce_optional: bool,
	now: i64,
}

fn check_claims(
	provider: &Provider,
	endpoints: &Endpoints,
	claims: &IdClaims,
	expected: Expected<'_>,
) -> Result<()> {
	let Expected {
		audience,
		nonce,
		nonce_optional,
		now,
	} = expected;
	check_issuer(provider, endpoints, claims)?;
	check_audience(claims, audience)?;
	check_times(claims, now)?;
	check_nonce(claims, nonce, nonce_optional)?;
	check_subject(claims)
}

/// The provider's issuer, or Google's short form of its own.
fn check_issuer(provider: &Provider, endpoints: &Endpoints, claims: &IdClaims) -> Result<()> {
	let issuer = provider.issuer(endpoints, claims.tid.as_deref());
	let google_short =
		matches!(provider.flavor, Flavor::Google) && claims.iss == "accounts.google.com";

	if issuer.as_deref() != Some(claims.iss.as_str()) && !google_short {
		return Err(wrong_claim("issuer"));
	}

	Ok(())
}

/// The app the token was issued to, and the party it was authorized for
/// when more than one is named.
fn check_audience(claims: &IdClaims, audience: &str) -> Result<()> {
	if !claims.aud.contains(audience) {
		return Err(wrong_claim("audience"));
	}

	if (claims.aud.several() || claims.azp.is_some()) && claims.azp.as_deref() != Some(audience) {
		return Err(wrong_claim("authorized party"));
	}

	Ok(())
}

/// When it was issued, and when it expires, within a small clock skew.
fn check_times(claims: &IdClaims, now: i64) -> Result<()> {
	if claims.exp <= now - SKEW {
		return Err(wrong_claim("expiry"));
	}

	if claims.iat > now + SKEW || claims.nbf.is_some_and(|nbf| nbf > now + SKEW) {
		return Err(wrong_claim("issue time"));
	}

	Ok(())
}

/// The nonce the flow bound the token to, in constant time.
fn check_nonce(claims: &IdClaims, nonce: &str, nonce_optional: bool) -> Result<()> {
	let matches = match claims.nonce.as_deref() {
		Some(claimed) => bool::from(claimed.as_bytes().ct_eq(nonce.as_bytes())),
		None => nonce_optional,
	};

	if !matches {
		return Err(wrong_claim("nonce"));
	}

	Ok(())
}

/// The person the token names.
fn check_subject(claims: &IdClaims) -> Result<()> {
	if claims.sub.is_empty() || claims.sub.len() > 255 {
		return Err(wrong_claim("subject"));
	}

	Ok(())
}

fn wrong_claim(what: &str) -> AuthError {
	AuthError::Provider(format!("the ID token's {what} is wrong"))
}

/// A flow that has started: where to send the browser, and the token that
/// binds the flow to it, for an `HttpOnly` cookie.
#[derive(Debug)]
pub struct ProviderStart {
	/// The provider's authorization page.
	pub url: String,
	/// Binds the callback to this browser, or to the native app that
	/// started it.
	pub binding: SessionToken,
	/// For Sign in with Apple on the device: the nonce its ID token must
	/// carry.
	pub nonce: Option<String>,
}

/// The provider's authorization page, with the flow and PKCE bound to
/// `state`.
fn authorization_url(
	provider: &Provider,
	authorization_endpoint: &str,
	redirect_uri: &str,
	state: &str,
	challenge: &str,
	nonce: &str,
	linking: bool,
) -> Result<url::Url> {
	let mut url = url::Url::parse(authorization_endpoint)
		.map_err(|_| AuthError::Config(format!("`{}` has a bad authorization URL", provider.id)))?;
	let mut query = url.query_pairs_mut();

	for (name, value) in
		authorization_params(provider, redirect_uri, state, challenge, nonce, linking)
	{
		query.append_pair(name, &value);
	}

	drop(query);
	Ok(url)
}

/// The authorization page's query: the standard parameters, then the
/// provider's quirks.
fn authorization_params(
	provider: &Provider,
	redirect_uri: &str,
	state: &str,
	challenge: &str,
	nonce: &str,
	linking: bool,
) -> Vec<(&'static str, String)> {
	let quirks = &provider.quirks;
	let offline = provider.store_tokens.then_some(quirks.offline);
	let mut params = vec![
		("response_type", "code".to_string()),
		(quirks.client_id_param, provider.client_id.clone()),
		("redirect_uri", redirect_uri.to_string()),
		("state", state.to_string()),
	];
	let scopes = scopes_with_offline(provider, offline);

	if !scopes.is_empty() {
		params.push(("scope", scopes.join(quirks.scope_separator)));
	}

	if quirks.pkce {
		params.push(("code_challenge", challenge.to_string()));
		params.push(("code_challenge_method", "S256".to_string()));
	}

	if provider.id_token_rules().is_some() {
		params.push(("nonce", nonce.to_string()));
	}

	if matches!(provider.flavor, Flavor::Apple) {
		params.push(("response_mode", "form_post".to_string()));
	}

	params.extend(extra_params(quirks, offline));

	if linking && quirks.select_account {
		// Connecting should pick the account, not reuse whichever one the
		// browser is signed in to there.
		params.push(("prompt", "select_account".to_string()));
	}

	params
}

/// The app's scopes, with the one that asks for a refresh token when the
/// provider wants it that way.
fn scopes_with_offline(provider: &Provider, offline: Option<Offline>) -> Vec<String> {
	let mut scopes = provider.scopes.clone();

	if let Some(Offline::Scope(scope)) = offline
		&& !scopes.iter().any(|existing| existing == scope)
	{
		scopes.push(scope.to_owned());
	}

	scopes
}

/// The parameters a provider's quirks add, with the one that asks for a
/// refresh token last, replacing its own of the same name.
fn extra_params(quirks: &Quirks, offline: Option<Offline>) -> Vec<(&'static str, String)> {
	let offline_param = match offline {
		Some(Offline::Param(name, value)) => Some((name, value)),
		_ => None,
	};
	let mut params = Vec::new();

	for (name, value) in &quirks.authorize {
		if offline_param.is_none_or(|(offline, _)| offline != *name) {
			params.push((*name, value.clone()));
		}
	}

	if let Some((name, value)) = offline_param {
		params.push((name, value.to_owned()));
	}

	params
}

impl Auth {
	pub(crate) fn provider(&self, id: &str) -> Result<&Provider> {
		self.inner
			.providers
			.get(id)
			.ok_or(AuthError::UnknownProvider)
	}

	/// The providers people can sign in with.
	pub fn providers(&self) -> Vec<ProviderInfo> {
		self.inner.providers.info()
	}

	/// The `User-Agent` `provider` wants, if it wants one.
	pub(crate) fn provider_user_agent(&self, provider: &Provider) -> Option<(String, String)> {
		provider
			.quirks
			.user_agent
			.then(|| app_user_agent(&self.inner.app_name))
	}

	/// The name of the provider `id`, for lists of connected accounts.
	pub(crate) fn provider_name(&self, id: &str) -> String {
		self.inner
			.providers
			.get(id)
			.map_or_else(|| id.to_owned(), |provider| provider.name.clone())
	}

	/// Where the provider sends people back to.
	fn redirect_uri(&self, provider: &Provider) -> Result<String> {
		let origin =
			self.inner.origin.as_deref().ok_or_else(|| {
				AuthError::Config("third-party sign-in needs `[auth] origin`".into())
			})?;
		Ok(format!("{origin}/auth/callback/{}", provider.id))
	}

	/// Starts signing in with `provider`, or with `me`, connecting it to
	/// their account. Send the browser to the [`ProviderStart`]'s URL, and
	/// keep its binding in an `HttpOnly` cookie until the provider sends it
	/// back. `redirect_to` is the path on this site to return to.
	///
	/// # Errors
	///
	/// [`AuthError::UnknownProvider`], [`AuthError::InvalidRedirect`],
	/// [`AuthError::RateLimited`], or for connecting, as
	/// [`Auth::begin_totp`] when the session isn't fresh.
	pub async fn start_provider(
		&self,
		provider: &str,
		me: Option<&SignedIn>,
		redirect_to: Option<&str>,
		request: &RequestInfo,
	) -> Result<ProviderStart> {
		if request.transport == Transport::Bearer {
			return Err(AuthError::Unsupported(
				"a native app signs in with a provider through the system browser: start it with \
				 `Auth::start_native_provider`",
			));
		}

		self.begin_provider(provider, me, redirect_to, None, request)
			.await
	}

	/// [`Auth::start_provider`] for a native app, which keeps the binding
	/// in memory and sends it to [`Auth::finish_native_provider`]. In the
	/// system browser, the provider comes back to this site, which sends the
	/// browser on to `return_to` without finishing anything: the code only
	/// works with the binding, which never leaves the app. With
	/// [`NativeStart::AppleId`](crate::NativeStart::AppleId), the device
	/// asks Apple itself, with the [`ProviderStart`]'s nonce.
	///
	/// # Errors
	///
	/// As [`Auth::start_provider`], and [`AuthError::InvalidRedirect`] for a
	/// `return_to` that isn't one of the app's (see
	/// [`AuthBuilder::native_apps`](crate::AuthBuilder::native_apps)), or
	/// [`AuthError::Unsupported`] for Sign in with Apple on the device with
	/// any other provider, or without a bundle ID.
	pub async fn start_native_provider(
		&self,
		provider: &str,
		me: Option<&SignedIn>,
		redirect_to: Option<&str>,
		native: &crate::NativeStart,
		request: &RequestInfo,
	) -> Result<ProviderStart> {
		match native {
			crate::NativeStart::Browser { return_to } => {
				self.native_return(return_to)?;
			}
			crate::NativeStart::AppleId => {
				let apple = matches!(self.provider(provider)?.flavor, Flavor::Apple);

				if !apple || self.inner.native.package_id.is_empty() {
					return Err(AuthError::Unsupported(
						"Sign in with Apple on the device needs the `apple` provider and the \
						 app's bundle ID, `[native] package_id`",
					));
				}
			}
		}

		let request = RequestInfo {
			transport: Transport::Bearer,
			..request.clone()
		};
		self.begin_provider(provider, me, redirect_to, Some(native), &request)
			.await
	}

	/// Checks that `return_to` is one of this app's return addresses: this
	/// site's own `/auth/native/return`, a loopback address with that path
	/// (any port, as RFC 8252 asks for desktop apps), or the private-use
	/// scheme named after the app's package ID.
	pub(crate) fn native_return(&self, return_to: &str) -> Result<()> {
		let path = leptoskit_config::NATIVE_AUTH_RETURN_PATH;
		let url = url::Url::parse(return_to).map_err(|_| AuthError::InvalidRedirect)?;
		let clean = url.query().is_none()
			&& url.fragment().is_none()
			&& url.username().is_empty()
			&& url.password().is_none()
			&& url.path() == path;

		if !clean {
			return Err(AuthError::InvalidRedirect);
		}

		let own_site = self.inner.origin.as_deref().is_some_and(|origin| {
			url::Url::parse(origin).is_ok_and(|origin| origin.origin() == url.origin())
		});
		let loopback = url.scheme() == "http"
			&& url.port().is_some()
			&& match url.host() {
				Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
				Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
				_ => false,
			};

		let package = &self.inner.native.package_id;
		let scheme = !package.is_empty()
			&& url.scheme() == package.to_ascii_lowercase()
			&& url.host().is_none()
			&& return_to == format!("{}:{path}", url.scheme());

		if own_site || loopback || scheme {
			Ok(())
		} else {
			Err(AuthError::InvalidRedirect)
		}
	}

	async fn begin_provider(
		&self,
		provider: &str,
		me: Option<&SignedIn>,
		redirect_to: Option<&str>,
		native: Option<&crate::NativeStart>,
		request: &RequestInfo,
	) -> Result<ProviderStart> {
		let provider = self.provider(provider)?;
		let redirect_to = crate::providers::return_path(redirect_to)?;

		if let Some(ip) = request.ip_text() {
			self.limit(
				&[b"ceremony-ip", ip.as_bytes()],
				self.inner.limits.ceremony_per_ip,
			)
			.await?;
		}

		let intent = match me {
			Some(me) => {
				self.require_fresh(&self.inner.db, me).await?;
				Intent::Link {
					account: me.account,
					session: me.session,
				}
			}
			None => Intent::SignIn,
		};

		let endpoints = self.inner.providers.endpoints(provider).await?;
		let redirect_uri = self.redirect_uri(provider)?;
		let verifier = URL_SAFE_NO_PAD.encode(secrets::random::<32>()?);
		let nonce = URL_SAFE_NO_PAD.encode(secrets::random::<32>()?);
		let challenge = URL_SAFE_NO_PAD.encode(secrets::digest_plain(verifier.as_bytes()));
		let binding = SessionToken::generate()?;
		let flow = Flow {
			provider: provider.id.clone(),
			verifier,
			nonce: nonce.clone(),
			redirect_to,
			intent,
			native: native.cloned(),
		};
		let state = self
			.store_ceremony_for(
				&self.inner.db,
				Purpose::Provider,
				&crate::providers::binding_subject(&binding),
				&flow.seal()?,
				FLOW_LIFETIME,
			)
			.await?;
		let url = authorization_url(
			provider,
			&endpoints.authorization_endpoint,
			&redirect_uri,
			state.expose(),
			&challenge,
			&nonce,
			me.is_some(),
		)?;

		Ok(ProviderStart {
			url: url.into(),
			binding,
			nonce: matches!(native, Some(crate::NativeStart::AppleId)).then_some(nonce),
		})
	}

	/// Where a native app's system browser goes once the provider sends it
	/// back here, with the provider's answer for the app to finish with.
	/// `None` when `params` isn't a native app's flow, which the browser
	/// finishes as usual. Nothing is used up: without the binding, which
	/// only the app holds, the answer is worthless.
	pub(crate) async fn native_relay(
		&self,
		provider: &str,
		params: &CallbackParams,
	) -> Result<Option<String>> {
		let Some(state) = params.state.as_deref() else {
			return Ok(None);
		};
		let hash = SessionToken::new(state).hash();
		let now = leptoskit_db::now();
		let row = crate::Challenge::query()
			.filter(|row| {
				row.purpose.eq(Purpose::Provider)
					& row.secret_hash.eq(hash.clone())
					& row.used_at.is_null()
					& row.expires_at.gt(now)
			})
			.first(&self.inner.db)
			.await?;
		let Some(row) = row else {
			return Ok(None);
		};
		let Some(sealed) = &row.state else {
			return Ok(None);
		};
		let context =
			crate::passkeys::ceremony_context(Purpose::Provider, &row.subject, &row.secret_hash);
		let flow = Flow::open(&self.inner.keys.open(&context, sealed)?)?;
		let Some(crate::NativeStart::Browser { return_to }) = flow.native else {
			return Ok(None);
		};

		if flow.provider != provider {
			return Ok(None);
		}

		let mut location = url::Url::parse(&return_to).map_err(|_| AuthError::InvalidRedirect)?;
		{
			let mut query = location.query_pairs_mut();

			for (name, value) in [
				("state", &params.state),
				("code", &params.code),
				("error", &params.error),
				("user", &params.user),
			] {
				if let Some(value) = value {
					query.append_pair(name, value);
				}
			}
		}

		Ok(Some(location.into()))
	}

	/// Finishes a native app's provider flow with what the system browser
	/// came back with, or with Sign in with Apple's ID token from the
	/// device. The flow is found by the binding only the app holds.
	///
	/// # Errors
	///
	/// [`AuthError::SignInExpired`] when there's no such flow, or as the
	/// browser's callback fails: [`AuthError::Cancelled`] when the person
	/// said no, and [`AuthError::Provider`] when the provider's answer
	/// doesn't check out.
	pub async fn finish_native_provider(
		&self,
		finish: &crate::FinishProvider,
		request: &RequestInfo,
	) -> Result<crate::ProviderResult> {
		let binding = SessionToken::new(finish.token.as_str());
		let request = RequestInfo {
			transport: Transport::Bearer,
			device_name: finish.device_name.clone(),
			..request.clone()
		};
		let params = CallbackParams {
			code: finish.code.clone(),
			state: finish.state.clone(),
			error: finish.error.clone(),
			user: finish.user.clone(),
		};

		let (flow, profile) = match &finish.id_token {
			Some(id_token) => {
				self.apple_id_profile(&finish.provider, &binding, id_token, &params)
					.await?
			}
			None => {
				self.provider_profile(&finish.provider, &binding, &params, Mode::NativeBrowser)
					.await?
			}
		};

		self.finish_flow(&flow, &profile, &binding, &request).await
	}

	/// Who Sign in with Apple on the device says signed in: its ID token,
	/// issued to the app's bundle ID with the flow's nonce.
	async fn apple_id_profile(
		&self,
		provider_id: &str,
		binding: &SessionToken,
		id_token: &str,
		params: &CallbackParams,
	) -> Result<(Flow, ProviderProfile)> {
		let provider = self.provider(provider_id)?;

		if let Some(error) = &params.error {
			return Err(refusal(error));
		}

		let sealed = self
			.take_ceremony(
				Purpose::Provider,
				&crate::providers::binding_subject(binding),
				None,
			)
			.await?
			.ok_or(AuthError::SignInExpired)?;
		let flow = Flow::open(&sealed)?;

		if flow.provider != provider.id
			|| flow.native != Some(crate::NativeStart::AppleId)
			|| !matches!(provider.flavor, Flavor::Apple)
		{
			return Err(AuthError::SignInExpired);
		}

		let providers = &self.inner.providers;
		let endpoints = providers.endpoints(provider).await?;
		let expected = Expected {
			audience: &self.inner.native.package_id,
			nonce: &flow.nonce,
			nonce_optional: false,
			now: leptoskit_db::now().timestamp(),
		};
		let claims = verify_id_token(providers, provider, &endpoints, id_token, expected).await?;
		let profile = ProviderProfile {
			subject: claims.sub,
			email: claims.email,
			email_verified: claims.email_verified.is_some_and(|flag| flag.is_true()),
			name: claims
				.name
				.or_else(|| params.user.as_deref().and_then(apple_name)),
			preferred_username: claims.preferred_username,
			picture: claims.picture,
			grant: None,
		};
		Ok((flow, profile))
	}

	/// Exchanges the code a provider sent back for who the person is there,
	/// after checking the flow started in this browser.
	pub(crate) async fn provider_profile(
		&self,
		provider_id: &str,
		binding: &SessionToken,
		params: &CallbackParams,
		mode: Mode,
	) -> Result<(Flow, ProviderProfile)> {
		let provider = self.provider(provider_id)?;

		if let Some(error) = &params.error {
			return Err(refusal(error));
		}

		let state = params.state.as_deref().ok_or(AuthError::SignInExpired)?;
		let sealed = self
			.take_ceremony(
				Purpose::Provider,
				&crate::providers::binding_subject(binding),
				Some(&SessionToken::new(state)),
			)
			.await?
			.ok_or(AuthError::SignInExpired)?;
		let flow = Flow::open(&sealed)?;
		let native = matches!(flow.native, Some(crate::NativeStart::Browser { .. }));

		// A browser's flow finishes only in the browser, and a native app's
		// only in the app.
		if flow.provider != provider.id || native != (mode == Mode::NativeBrowser) {
			return Err(AuthError::SignInExpired);
		}

		let code = params
			.code
			.as_deref()
			.ok_or_else(|| AuthError::Provider("no authorization code".into()))?;
		let providers = &self.inner.providers;
		let endpoints = providers.endpoints(provider).await?;
		let redirect_uri = self.redirect_uri(provider)?;
		let user_agent = self.provider_user_agent(provider);
		let mut fields = vec![
			("grant_type", "authorization_code".to_owned()),
			("code", code.to_owned()),
			("redirect_uri", redirect_uri),
		];

		if provider.quirks.pkce {
			fields.push(("code_verifier", flow.verifier.clone()));
		}

		let mut request = token_request(provider, &endpoints.token_endpoint, fields)?;
		request.headers.extend(user_agent.clone());
		let tokens: serde_json::Value = providers.json(request).await?;

		if let Some(error) = tokens.get("error").filter(|error| !error.is_null()) {
			return Err(AuthError::Provider(format!(
				"the token endpoint refused: {}",
				describe(error)
			)));
		}

		let text = |name: &str| {
			tokens
				.get(name)
				.and_then(serde_json::Value::as_str)
				.map(str::to_owned)
		};

		let mut profile = match &provider.quirks.identify {
			Identify::Api(api) => {
				let access = text("access_token")
					.ok_or_else(|| AuthError::Provider("no access token".into()))?;
				api.read(providers, &access, &tokens, user_agent).await?
			}
			Identify::IdToken(rules) => {
				let id_token =
					text("id_token").ok_or_else(|| AuthError::Provider("no ID token".into()))?;
				let expected = Expected {
					audience: &provider.client_id,
					nonce: &flow.nonce,
					nonce_optional: rules.nonce_optional,
					now: leptoskit_db::now().timestamp(),
				};
				let claims =
					verify_id_token(providers, provider, &endpoints, &id_token, expected).await?;
				let mut profile = ProviderProfile {
					subject: scoped_subject(rules, &claims)?,
					email: claims.email,
					email_verified: rules.trust_email
						&& claims.email_verified.is_some_and(|flag| flag.is_true()),
					name: claims.name.or_else(|| given_and_family_name(&claims.other)),
					preferred_username: claims.preferred_username,
					picture: claims.picture,
					grant: None,
				};

				if matches!(provider.flavor, Flavor::Apple) && profile.name.is_none() {
					profile.name = params.user.as_deref().and_then(apple_name);
				}

				profile
			}
		};

		profile.grant = provider.store_tokens.then(|| grant(&tokens)).flatten();

		Ok((flow, profile))
	}
}

/// Where a provider flow is being finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
	/// At the callback, in the browser that started it.
	Browser,
	/// At `POST /auth/oauth/finish`, by the native app that started it.
	NativeBrowser,
}

/// The error for a provider's `error` answer.
fn refusal(error: &str) -> AuthError {
	if error == "access_denied" || error == "user_cancelled_authorize" {
		AuthError::Cancelled
	} else {
		AuthError::Provider(format!("the provider refused: {error}"))
	}
}

/// A request to the token endpoint at `url`, or another endpoint that
/// authenticates the app the same way, such as revocation: `fields`, with
/// the app's credentials as the provider asks, as a form or JSON.
fn token_request(
	provider: &Provider,
	url: &str,
	mut fields: Vec<(&'static str, String)>,
) -> Result<ProviderRequest> {
	let client_secret = match &provider.secret {
		Secret::None => None,
		Secret::Shared(secret) => Some(secret.clone()),
		Secret::Apple {
			team_id,
			key_id,
			key,
		} => {
			Some(apple_client_secret(
				team_id,
				key_id,
				&provider.client_id,
				key,
			)?)
		}
	};

	let quirks = &provider.quirks;
	let mut headers = vec![("accept".to_owned(), "application/json".to_owned())];

	match client_secret {
		Some(secret) if quirks.client_auth == ClientAuth::Basic => {
			headers.push((
				"authorization".to_owned(),
				basic_credentials(&provider.client_id, &secret),
			));
		}

		secret => {
			fields.push((quirks.client_id_param, provider.client_id.clone()));
			fields.extend(secret.map(|secret| ("client_secret", secret)));
		}
	}

	let (content_type, body) = if quirks.token_body == TokenBody::Json {
		let body: serde_json::Map<String, serde_json::Value> = fields
			.into_iter()
			.map(|(name, value)| (name.to_owned(), value.into()))
			.collect();
		(
			"application/json",
			serde_json::Value::Object(body).to_string(),
		)
	} else {
		let mut form = url::form_urlencoded::Serializer::new(String::new());
		form.extend_pairs(fields);
		("application/x-www-form-urlencoded", form.finish())
	};

	headers.push(("content-type".to_owned(), content_type.to_owned()));

	Ok(ProviderRequest {
		method: "POST",
		url: url.to_owned(),
		headers,
		body: Some(body),
	})
}

/// The tokens a token endpoint's answer holds, to keep.
fn grant(tokens: &serde_json::Value) -> Option<crate::providers::ProviderGrant> {
	let text = |name: &str| {
		tokens
			.get(name)
			.and_then(serde_json::Value::as_str)
			.filter(|text| !text.is_empty())
			.map(str::to_owned)
	};
	Some(crate::providers::ProviderGrant {
		access_token: text("access_token")?,
		refresh_token: text("refresh_token"),
		expires_in: tokens
			.get("expires_in")
			.and_then(|seconds| seconds.as_i64().or_else(|| seconds.as_str()?.parse().ok())),
		scope: text("scope"),
	})
}

/// The `User-Agent` header naming the app, for providers that ask every
/// app to say who it is: only characters that are safe in a header, and
/// nothing about the person signing in.
fn app_user_agent(app_name: &str) -> (String, String) {
	let name: String = app_name
		.chars()
		.filter(|character| character.is_ascii_alphanumeric() || " -_.".contains(*character))
		.take(64)
		.collect();
	let name = name.trim().replace(' ', "-");
	let name = if name.is_empty() {
		"app"
	} else {
		name.as_str()
	};

	(
		"user-agent".to_owned(),
		format!("web:{name}:leptoskit-auth/{}", env!("CARGO_PKG_VERSION")),
	)
}

/// A name from `given_name` and `family_name`, for providers whose ID
/// tokens carry no `name`, such as Dropbox's.
fn given_and_family_name(claims: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
	let full = ["given_name", "family_name"]
		.into_iter()
		.filter_map(|claim| claims.get(claim)?.as_str())
		.map(str::trim)
		.filter(|part| !part.is_empty())
		.collect::<Vec<_>>()
		.join(" ");
	(!full.is_empty()).then_some(full)
}

/// HTTP Basic credentials, each part form-encoded first, as RFC 6749
/// §2.3.1 asks.
fn basic_credentials(client_id: &str, secret: &str) -> String {
	let encode =
		|part: &str| url::form_urlencoded::byte_serialize(part.as_bytes()).collect::<String>();
	format!(
		"Basic {}",
		STANDARD.encode(format!("{}:{}", encode(client_id), encode(secret)))
	)
}

/// A provider's `error`, for the server's logs: its code, or its message.
fn describe(error: &serde_json::Value) -> String {
	error
		.as_str()
		.or_else(|| error.get("message").and_then(serde_json::Value::as_str))
		.or_else(|| error.get("code").and_then(serde_json::Value::as_str))
		.unwrap_or("an error")
		.chars()
		.take(200)
		.collect()
}

/// The identity an ID token names: its subject, within the claim that
/// scopes it, when the provider's subjects are only unique within one, such
/// as Slack's workspaces.
fn scoped_subject(rules: &IdTokenRules, claims: &IdClaims) -> Result<String> {
	let Some(claim) = rules.scope_claim else {
		return Ok(claims.sub.clone());
	};
	let scope = claims
		.other
		.get(claim)
		.and_then(serde_json::Value::as_str)
		.filter(|scope| !scope.is_empty() && !scope.contains(':'))
		.ok_or_else(|| AuthError::Provider(format!("the ID token has no `{claim}`")))?;
	let subject = format!("{scope}:{}", claims.sub);

	if subject.len() > 255 {
		return Err(AuthError::Provider(
			"the ID token's subject is too long".into(),
		));
	}

	Ok(subject)
}

/// Sign in with Apple's client secret: an ES256 JWT, signed with the
/// app's key, valid for five minutes.
fn apple_client_secret(
	team_id: &str,
	key_id: &str,
	client_id: &str,
	key: &ring::signature::EcdsaKeyPair,
) -> Result<String> {
	let now = leptoskit_db::now().timestamp();
	let header = serde_json::json!({ "alg": "ES256", "kid": key_id });
	let claims = serde_json::json!({
		"iss": team_id,
		"iat": now,
		"exp": now + 300,
		"aud": "https://appleid.apple.com",
		"sub": client_id,
	});
	let signed = format!(
		"{}.{}",
		URL_SAFE_NO_PAD.encode(header.to_string()),
		URL_SAFE_NO_PAD.encode(claims.to_string())
	);
	let signature = key
		.sign(&ring::rand::SystemRandom::new(), signed.as_bytes())
		.map_err(|_| AuthError::Config("the Apple key couldn't sign".into()))?;
	Ok(format!(
		"{signed}.{}",
		URL_SAFE_NO_PAD.encode(signature.as_ref())
	))
}

/// The name in Sign in with Apple's first-time `user` parameter.
fn apple_name(user: &str) -> Option<String> {
	#[derive(Deserialize)]
	struct User {
		name: Option<Name>,
	}
	#[derive(Deserialize)]
	#[serde(rename_all = "camelCase")]
	struct Name {
		first_name: Option<String>,
		last_name: Option<String>,
	}

	let user: User = serde_json::from_str(user).ok()?;
	let name = user.name?;
	let full = [name.first_name, name.last_name]
		.into_iter()
		.flatten()
		.map(|part| part.trim().to_owned())
		.filter(|part| !part.is_empty())
		.collect::<Vec<_>>()
		.join(" ");
	(!full.is_empty()).then(|| full.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
	use super::*;

	/// RFC 7515 appendix A.2: an RS256 JWS and the public half of its key.
	/// Both are published in the RFC; no private key is here.
	const RFC7515_RS256: &str = "eyJhbGciOiJSUzI1NiJ9.eyJpc3MiOiJqb2UiLA0KICJleHAiOjEzMDA4MTkzODAsDQogImh0dHA6Ly9leGFtcGxlLmNvbS9pc19yb290Ijp0cnVlfQ.cC4hiUPoj9Eetdgtv3hF80EGrhuB__dzERat0XF9g2VtQgr9PJbu3XOiZj5RZmh7AAuHIm4Bh-0Qc_lF5YKt_O8W2Fp5jujGbds9uJdbF9CUAr7t1dnZcAcQjbKBYNX4BAynRFdiuB--f_nZLgrnbyTyWzO75vRK5h6xBArLIARNPvkSjtQBMHlb1L07Qe7K0GarZRmB_eSN9383LcOLn6_dO--xi12jzDwusC-eOkHWEsqtFZESc6BfI7noOPqvhJ1phCnvWh6IeYI2w9QOYEUipUTI8np6LbgGY9Fs98rqVt5AXLIhWkWywlVmtVrBp0igcN_IoypGlUPQGe77Rw"; // gitleaks:allow (a published test vector)
	const RFC7515_N: &str = "ofgWCuLjybRlzo0tZWJjNiuSfb4p4fAkd_wWJcyQoTbji9k0l8W26mPddxHmfHQp-Vaw-4qPCJrcS2mJPMEzP1Pt0Bm4d4QlL-yRT-SFd2lZS-pCgNMsD1W_YpRPEwOWvG6b32690r2jZ47soMZo9wGzjb_7OMg0LOL-bSf63kpaSHSXndS5z5rexMdbBYUsLA9e-KXBdQOS-UTo7WTBEMa2R2CapHg665xsmtdVMTBQY4uDZlxvb3qCo5ZwKh9kG4LT6_I5IhlJH7aGhyxXFvUK-DWNmoudF8NAco9_h9iaGNj8q2ethFkMLs91kzk2PAcDTW9gb54h4FRWyuXpoQ";

	fn rfc_key() -> Jwk {
		Jwk {
			kty: "RSA".into(),
			kid: None,
			usage: None,
			alg: None,
			crv: None,
			n: Some(RFC7515_N.into()),
			e: Some("AQAB".into()),
			x: None,
			y: None,
		}
	}

	#[test]
	fn rs256_signatures_check_against_the_rfc_vector() {
		let jws = parse_jws(RFC7515_RS256).unwrap();
		assert_eq!(jws.header.alg, "RS256");
		verify_signature(&jws, &rfc_key()).unwrap();

		// One changed byte of the payload breaks it.
		let mut tampered = parse_jws(RFC7515_RS256).unwrap();
		tampered.signed.push('x');
		assert!(verify_signature(&tampered, &rfc_key()).is_err());

		// The key must be for the token's algorithm.
		let mut wrong = rfc_key();
		wrong.alg = Some("RS512".into());
		assert!(verify_signature(&jws, &wrong).is_err());
		let mut ec = rfc_key();
		ec.kty = "EC".into();
		assert!(verify_signature(&jws, &ec).is_err());
	}

	#[test]
	fn unsigned_and_symmetric_tokens_are_refused() {
		for alg in ["none", "HS256", "XS999"] {
			let header = URL_SAFE_NO_PAD.encode(format!(r#"{{"alg":"{alg}"}}"#));
			let token = format!("{header}.e30.AA");
			let jws = parse_jws(&token).unwrap();
			assert!(verify_signature(&jws, &rfc_key()).is_err(), "{alg}");
		}

		let critical = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","crit":["exp"]}"#);
		assert!(parse_jws(&format!("{critical}.e30.AA")).is_err());
		assert!(parse_jws("only.two").is_err());
		assert!(parse_jws("a.b.c.d").is_err());
		assert!(parse_jws("!.e30.AA").is_err());
	}

	#[test]
	fn es256_and_eddsa_keys_check_their_curves() {
		use ring::signature::Ed25519KeyPair;
		use ring::signature::KeyPair;

		let rng = ring::rand::SystemRandom::new();
		let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
		let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
		let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"EdDSA"}"#);
		let payload = URL_SAFE_NO_PAD.encode("{}");
		let signed = format!("{header}.{payload}");
		let token = format!(
			"{signed}.{}",
			URL_SAFE_NO_PAD.encode(pair.sign(signed.as_bytes()).as_ref())
		);
		let key = Jwk {
			kty: "OKP".into(),
			crv: Some("Ed25519".into()),
			x: Some(URL_SAFE_NO_PAD.encode(pair.public_key().as_ref())),
			..rfc_key()
		};

		let jws = parse_jws(&token).unwrap();
		verify_signature(&jws, &key).unwrap();
		let wrong_curve = Jwk {
			crv: Some("X25519".into()),
			..key.clone()
		};
		assert!(verify_signature(&jws, &wrong_curve).is_err());
		let es_header = parse_jws(&format!(
			"{}.{payload}.AA",
			URL_SAFE_NO_PAD.encode(r#"{"alg":"ES256"}"#)
		))
		.unwrap();
		let short = Jwk {
			kty: "EC".into(),
			crv: Some("P-256".into()),
			x: Some("AA".into()),
			y: Some("AA".into()),
			..rfc_key()
		};
		assert!(verify_signature(&es_header, &short).is_err());
	}

	fn claims(json: serde_json::Value) -> IdClaims {
		serde_json::from_value(json).unwrap()
	}

	#[test]
	fn claims_are_checked_and_times_must_be_numbers() {
		let provider = Provider::google("client", "secret");
		let endpoints = provider.endpoints.clone().unwrap();
		let now = 1_800_000_000;
		let good = serde_json::json!({
			"iss": "https://accounts.google.com",
			"sub": "1234",
			"aud": "client",
			"exp": now + 300,
			"iat": now,
			"nonce": "n-0S6_WzA2Mj",
		});
		let check = |json: serde_json::Value| {
			let expected = Expected {
				audience: "client",
				nonce: "n-0S6_WzA2Mj",
				nonce_optional: false,
				now,
			};
			check_claims(&provider, &endpoints, &claims(json), expected)
		};
		check(good.clone()).unwrap();

		let with = |key: &str, value: serde_json::Value| {
			let mut json = good.clone();
			json[key] = value;
			json
		};
		check(with("iss", "accounts.google.com".into())).unwrap();
		assert!(check(with("iss", "https://evil.example".into())).is_err());
		assert!(check(with("aud", "other".into())).is_err());
		assert!(check(with("aud", serde_json::json!(["client", "other"]))).is_err());
		check(with("aud", serde_json::json!(["client", "other"])))
			.err()
			.unwrap();
		let mut both = with("aud", serde_json::json!(["client", "other"]));
		both["azp"] = "client".into();

		check(both).unwrap();
		assert!(check(with("azp", "other".into())).is_err());
		assert!(check(with("exp", (now - 61).into())).is_err());
		assert!(check(with("iat", (now + 61).into())).is_err());
		assert!(check(with("nbf", (now + 61).into())).is_err());
		assert!(check(with("nonce", "replayed".into())).is_err());
		assert!(check(with("sub", "".into())).is_err());
		let mut no_nonce = good.clone();
		no_nonce.as_object_mut().unwrap().remove("nonce");
		assert!(check(no_nonce).is_err());

		// A time that isn't a number doesn't parse, so it can't skip a check.
		assert!(serde_json::from_value::<IdClaims>(with("exp", "never".into())).is_err());
	}

	#[test]
	fn microsofts_issuer_follows_the_tenant() {
		let endpoints = Endpoints::fixed("a", "t", None, None);
		let issuer = |tenant: &str, tid: Option<&str>| {
			Provider::microsoft("client", "secret", tenant).issuer(&endpoints, tid)
		};
		let work = "72f988bf-86f1-41af-91ab-2d7cd011db47";

		assert_eq!(
			issuer("common", Some(work)),
			Some(format!("https://login.microsoftonline.com/{work}/v2.0"))
		);
		assert_eq!(issuer("common", None), None);
		assert!(issuer("consumers", Some(MICROSOFT_CONSUMERS)).is_some());
		assert_eq!(issuer("consumers", Some(work)), None);
		assert_eq!(issuer("organizations", Some(MICROSOFT_CONSUMERS)), None);
		assert!(issuer("organizations", Some(work)).is_some());
		assert!(issuer(work, Some(work)).is_some());
		assert_eq!(issuer(work, Some(MICROSOFT_CONSUMERS)), None);
	}

	#[test]
	fn apple_names_and_strings_that_mean_true() {
		assert_eq!(
			apple_name(r#"{"name":{"firstName":"Ada","lastName":"Lovelace"},"email":"a@b"}"#),
			Some("Ada Lovelace".into())
		);
		assert_eq!(apple_name(r#"{"name":{"firstName":" "}}"#), None);
		assert_eq!(apple_name("not json"), None);
		assert!(Flag::Text("true".into()).is_true());
		assert!(!Flag::Text("false".into()).is_true());
		assert!(Flag::Bool(true).is_true());
	}

	#[test]
	fn provider_definitions_are_checked() {
		assert!(Provider::oidc("Bad ID", "x", "https://sso.example", "c", None).is_err());
		assert!(Provider::oidc("sso", "x", "http://sso.example", "c", None).is_err());
		assert!(Provider::oidc("sso", "x", "http://localhost:8080", "c", None).is_ok());
		assert!(
			Provider::apple(
				"s",
				"t",
				"k",
				"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----"
			)
			.is_err()
		);
		assert!(Provider::apple("s", "t", "k", "").is_err());

		let github = Provider::github("c", "s").scopes(["user:email"]);
		assert_eq!(github.scopes, ["user:email"]);
		let google = Provider::google("c", "s")
			.scopes(["email"])
			.name("Google Workspace");
		assert_eq!(google.scopes, ["openid", "email"]);
		assert_eq!(google.name, "Google Workspace");
		assert_eq!(google.id(), "google");
		let debug = format!("{:?}", Provider::google("c", "very-secret"));
		assert!(!debug.contains("very-secret"), "{debug}");

		assert!(valid_id("company-sso"));
		assert!(!valid_id(""));
		assert!(!valid_id(&"a".repeat(65)));
	}

	#[tokio::test]
	async fn the_network_transport_sends_headers_and_bodies_and_caps_answers() {
		use axum::routing::post;

		let app = axum::Router::new()
			.route(
				"/token",
				post(|headers: axum::http::HeaderMap, body: String| {
					async move {
						let accept = headers["accept"].to_str().unwrap().to_owned();
						axum::Json(serde_json::json!({ "accept": accept, "body": body }))
					}
				}),
			)
			.route(
				"/big",
				axum::routing::get(|| async { "x".repeat(MAX_BODY + 1) }),
			)
			.route(
				"/broken",
				axum::routing::get(|| async { (axum::http::StatusCode::BAD_GATEWAY, "{}") }),
			);
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let address = listener.local_addr().unwrap();
		tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
		let providers = Providers::new(Vec::new(), None);
		let base = format!("http://{address}");

		let answer: serde_json::Value = providers
			.json(ProviderRequest {
				method: "POST",
				url: format!("{base}/token"),
				headers: vec![("accept".into(), "application/json".into())],
				body: Some("code=1".into()),
			})
			.await
			.unwrap();
		assert_eq!(answer["accept"], "application/json");
		assert_eq!(answer["body"], "code=1");

		let too_large = providers
			.send(get(&format!("{base}/big")))
			.await
			.unwrap_err();
		assert!(too_large.to_string().contains("too large"), "{too_large}");
		let refused = providers
			.json::<serde_json::Value>(get(&format!("{base}/broken")))
			.await
			.unwrap_err();
		assert!(refused.to_string().contains("answered 502"), "{refused}");
		let unreachable = providers
			.send(get("http://127.0.0.1:9/nothing"))
			.await
			.unwrap_err();
		assert!(matches!(unreachable, AuthError::Provider(_)));
		assert!(!format!("{:?}", get("https://x.example/a")).contains("accept"));
	}

	#[test]
	fn the_app_names_itself_and_nothing_else() {
		let (name, value) = app_user_agent("My App: \"notes\"\r\nX-Evil: 1");
		assert_eq!(name, "user-agent");
		assert!(
			value.starts_with("web:My-App-notesX-Evil-1:leptoskit-auth/"),
			"{value}"
		);
		assert!(app_user_agent("  ").1.starts_with("web:app:"));
		let claims = serde_json::json!({ "given_name": "Ada", "family_name": " Lovelace " });
		assert_eq!(
			given_and_family_name(claims.as_object().unwrap()).as_deref(),
			Some("Ada Lovelace")
		);
		assert_eq!(given_and_family_name(&serde_json::Map::new()), None);
	}

	#[test]
	fn key_sets_pick_signing_keys() {
		let keys: Jwks = serde_json::from_value(serde_json::json!({
			"keys": [
				{ "kty": "RSA", "kid": "a", "use": "sig", "n": "AQ", "e": "AQAB" },
				{ "kty": "RSA", "kid": "b", "use": "enc", "n": "AQ", "e": "AQAB" },
			]
		}))
		.unwrap();
		assert!(keys.find(Some("a")).is_some());
		assert!(
			keys.find(Some("b")).is_none(),
			"an encryption key never verifies"
		);
		assert!(keys.find(None).is_some(), "the only signing key");
		assert!(keys.find(Some("c")).is_none());
	}
}
