//! A fake identity provider, answering from memory: OpenID Connect
//! discovery, a key set, an authorization step, and a token endpoint that
//! checks PKCE and the client's credentials the way each provider wants
//! them, then signs ID tokens with a P-256 key made at test time, or hands
//! out an access token for its profile API. It answers for a generic issuer
//! (`https://sso.example`) and at each built-in provider's own URLs (see
//! [`SPECS`]), so their definitions run as they would against the real
//! thing, without a network.

#![allow(dead_code, reason = "each test file uses part of this")]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use leptoskit_auth::ProviderFuture;
use leptoskit_auth::ProviderHttp;
use leptoskit_auth::ProviderRequest;
use leptoskit_auth::ProviderResponse;
use ring::digest::SHA256;
use ring::digest::digest;
use ring::rand::SystemRandom;
use ring::signature::ECDSA_P256_SHA256_FIXED;
use ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING;
use ring::signature::EcdsaKeyPair;
use ring::signature::KeyPair;
use ring::signature::UnparsedPublicKey;
use serde_json::Value;
use serde_json::json;

/// The generic issuer.
pub const ISSUER: &str = "https://sso.example";
/// Every client's secret.
pub const SECRET: &str = "client-secret";

/// What the provider does wrong, to check the client notices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tamper {
	#[default]
	Nothing,
	/// Signs with a key it never published, under the published key's ID.
	ForeignKey,
	/// Names another client as the audience.
	Audience,
	/// Echoes a nonce of its own.
	Nonce,
	/// Leaves the nonce out.
	NoNonce,
	/// Issues a token that expired an hour ago.
	Expired,
	/// Names another issuer.
	Issuer,
	/// Signs with `alg: none`.
	Unsigned,
}

/// What the fake checks and answers for one provider.
#[derive(Clone, Copy, Debug)]
#[allow(
	clippy::struct_excessive_bools,
	reason = "each flag is one thing a provider does differently"
)]
pub struct Spec {
	/// Its authorization endpoint's host.
	pub authorize_host: &'static str,
	/// Its token endpoint's host.
	pub token_host: &'static str,
	/// Its ID tokens' issuer; `None` for a provider whose API says who
	/// signed in.
	pub issuer: Option<&'static str>,
	/// Whether the client must authenticate with HTTP Basic, and never in
	/// the form.
	pub basic: bool,
	/// Whether its ID tokens carry the nonce.
	pub echoes_nonce: bool,
	/// Whether it takes PKCE; without, it refuses a challenge or verifier.
	pub pkce: bool,
	/// What it calls the client ID.
	pub client_param: &'static str,
	/// Whether its token endpoint takes JSON rather than a form.
	pub json: bool,
}

const fn id_tokens(
	authorize_host: &'static str,
	token_host: &'static str,
	issuer: &'static str,
) -> Spec {
	Spec {
		authorize_host,
		token_host,
		issuer: Some(issuer),
		basic: false,
		echoes_nonce: true,
		pkce: true,
		client_param: "client_id",
		json: false,
	}
}

const fn api(authorize_host: &'static str, token_host: &'static str, basic: bool) -> Spec {
	Spec {
		authorize_host,
		token_host,
		issuer: None,
		basic,
		echoes_nonce: false,
		pkce: true,
		client_param: "client_id",
		json: false,
	}
}

/// Every provider the fake stands in for. Microsoft's issuer names the
/// token's tenant, so it's filled in from the `tid` claim.
pub const SPECS: &[Spec] = &[
	id_tokens("sso.example", "sso.example", ISSUER),
	id_tokens(
		"accounts.google.com",
		"oauth2.googleapis.com",
		"https://accounts.google.com",
	),
	id_tokens(
		"appleid.apple.com",
		"appleid.apple.com",
		"https://appleid.apple.com",
	),
	id_tokens(
		"login.microsoftonline.com",
		"login.microsoftonline.com",
		"microsoft",
	),
	api("github.com", "github.com", false),
	api("www.facebook.com", "graph.facebook.com", false),
	api("x.com", "api.x.com", true),
	api("discord.com", "discord.com", false),
	Spec {
		echoes_nonce: false,
		..id_tokens(
			"www.linkedin.com",
			"www.linkedin.com",
			"https://www.linkedin.com/oauth",
		)
	},
	id_tokens("gitlab.com", "gitlab.com", "https://gitlab.com"),
	id_tokens("gitlab.example", "gitlab.example", "https://gitlab.example"),
	id_tokens("slack.com", "slack.com", "https://slack.com"),
	id_tokens(
		"id.twitch.tv",
		"id.twitch.tv",
		"https://id.twitch.tv/oauth2",
	),
	api("accounts.spotify.com", "accounts.spotify.com", true),
	api("www.reddit.com", "www.reddit.com", true),
	api("www.amazon.com", "api.amazon.com", false),
	id_tokens(
		"www.dropbox.com",
		"api.dropboxapi.com",
		"https://www.dropbox.com",
	),
	api("bitbucket.org", "bitbucket.org", true),
	Spec {
		pkce: false,
		json: true,
		..api("api.notion.com", "api.notion.com", true)
	},
	api("zoom.us", "zoom.us", true),
	Spec {
		pkce: false,
		client_param: "client_key",
		..api("www.tiktok.com", "open.tiktokapis.com", false)
	},
	id_tokens(
		"api.login.yahoo.com",
		"api.login.yahoo.com",
		"https://api.login.yahoo.com",
	),
];

/// The key sets the fake publishes, at each provider's own address.
const KEY_SETS: &[&str] = &[
	"https://sso.example/jwks",
	"https://www.googleapis.com/oauth2/v3/certs",
	"https://appleid.apple.com/auth/keys",
	"https://www.linkedin.com/oauth/openid/jwks",
	"https://gitlab.com/oauth/discovery/keys",
	"https://gitlab.example/oauth/discovery/keys",
	"https://slack.com/openid/connect/keys",
	"https://id.twitch.tv/oauth2/keys",
	"https://www.dropbox.com/.well-known/jwks",
	"https://api.login.yahoo.com/openid/v1/certs",
];

/// The hosts whose `GET`s, with an access token the fake issued, answer
/// from the person's `user` (or `emails`) claims.
const API_HOSTS: &[&str] = &[
	"api.github.com",
	"graph.facebook.com",
	"api.x.com",
	"discord.com",
	"api.spotify.com",
	"oauth.reddit.com",
	"api.amazon.com",
	"api.bitbucket.org",
	"api.zoom.us",
	"open.tiktokapis.com",
];

/// The hosts that refuse requests that don't name the app in `User-Agent`,
/// as Reddit does, with `429`.
const NAMED_APPS_ONLY: &[&str] = &["www.reddit.com", "oauth.reddit.com"];

/// An authorization the person granted, waiting for its code.
struct Grant {
	spec: Spec,
	client_id: String,
	redirect_uri: String,
	challenge: String,
	nonce: Option<String>,
	issuer: String,
	claims: Value,
}

struct State {
	grants: HashMap<String, Grant>,
	tokens: HashMap<String, Value>,
	tamper: Tamper,
	apple_key: Option<Vec<u8>>,
	requests: Vec<String>,
}

/// The fake provider. Clones share it.
#[derive(Clone)]
pub struct FakeIdp {
	state: Arc<Mutex<State>>,
	key: Arc<EcdsaKeyPair>,
	foreign: Arc<EcdsaKeyPair>,
}

fn key() -> EcdsaKeyPair {
	let rng = SystemRandom::new();
	let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
	EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap()
}

/// A Sign in with Apple key, as the `.p8` file holds it, and its public
/// half.
pub fn apple_key() -> (String, Vec<u8>) {
	let rng = SystemRandom::new();
	let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
	let pair =
		EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
	let pem = format!(
		"-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
		STANDARD.encode(pkcs8.as_ref())
	);
	(pem, pair.public_key().as_ref().to_vec())
}

impl Default for FakeIdp {
	fn default() -> Self {
		Self::new()
	}
}

impl FakeIdp {
	pub fn new() -> Self {
		Self {
			state: Arc::new(Mutex::new(State {
				grants: HashMap::new(),
				tokens: HashMap::new(),
				tamper: Tamper::Nothing,
				apple_key: None,
				requests: Vec::new(),
			})),
			key: Arc::new(key()),
			foreign: Arc::new(key()),
		}
	}

	/// Makes the next tokens wrong in this way.
	pub fn tamper(&self, tamper: Tamper) {
		self.state.lock().unwrap().tamper = tamper;
	}

	/// Checks Apple client secrets against this public key.
	pub fn trust_apple_key(&self, public: Vec<u8>) {
		self.state.lock().unwrap().apple_key = Some(public);
	}

	/// The URLs requested so far.
	pub fn requests(&self) -> Vec<String> {
		self.state.lock().unwrap().requests.clone()
	}

	/// The person signs in at the authorization `url` the app sent them to,
	/// as someone with `claims` (`sub`, `email`, …). Returns the query the
	/// provider sends back to the app: `code` and `state`.
	pub fn authorize(&self, url: &str, claims: Value) -> Vec<(String, String)> {
		let url = url::Url::parse(url).unwrap();
		let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
		assert_eq!(query["response_type"], "code");
		let host = url.host_str().unwrap();
		let spec = *SPECS
			.iter()
			.find(|spec| spec.authorize_host == host)
			.unwrap_or_else(|| panic!("the fake doesn't stand in for {host}"));
		let issuer = match spec.issuer {
			Some("microsoft") => {
				format!(
					"https://login.microsoftonline.com/{}/v2.0",
					claims["tid"].as_str().unwrap()
				)
			}
			Some(issuer) => issuer.to_owned(),
			None => String::new(),
		};

		if host == "appleid.apple.com" {
			assert_eq!(query["response_mode"], "form_post");
		}

		// Twitch's ID token carries only the claims asked for by name.
		if host == "id.twitch.tv" {
			let asked: Value = serde_json::from_str(&query["claims"]).unwrap();

			for claim in ["email", "email_verified", "preferred_username"] {
				assert!(asked["id_token"].get(claim).is_some(), "{claim}");
			}
		}

		// An OpenID Connect sign-in carries a nonce; an API's has no use for
		// one. A provider without PKCE would refuse a challenge.
		assert_eq!(query.contains_key("nonce"), spec.issuer.is_some(), "{host}");
		assert_eq!(query.contains_key("code_challenge"), spec.pkce, "{host}");

		if spec.pkce {
			assert_eq!(query["code_challenge_method"], "S256");
		}

		let code = URL_SAFE_NO_PAD.encode(digest(&SHA256, url.as_str().as_bytes()));
		self.state.lock().unwrap().grants.insert(
			code.clone(),
			Grant {
				spec,
				client_id: query[spec.client_param].clone(),
				redirect_uri: query["redirect_uri"].clone(),
				challenge: query.get("code_challenge").cloned().unwrap_or_default(),
				nonce: query.get("nonce").cloned().filter(|_| spec.echoes_nonce),
				issuer,
				claims,
			},
		);
		vec![
			("code".to_owned(), code),
			("state".to_owned(), query["state"].clone()),
		]
	}

	/// Sign in with Apple on a device: an ID token issued to the app's
	/// `bundle_id`, carrying `nonce`, for someone with `claims`.
	pub fn apple_id_token(&self, bundle_id: &str, nonce: &str, claims: Value) -> String {
		self.id_token(&Grant {
			spec: SPECS[2],
			client_id: bundle_id.to_owned(),
			redirect_uri: String::new(),
			challenge: String::new(),
			nonce: Some(nonce.to_owned()),
			issuer: "https://appleid.apple.com".to_owned(),
			claims,
		})
	}

	fn id_token(&self, grant: &Grant) -> String {
		let tamper = self.state.lock().unwrap().tamper;
		let now = chrono::Utc::now().timestamp();
		let mut claims = json!({
			"iss": grant.issuer,
			"aud": grant.client_id,
			"exp": now + 300,
			"iat": now,
		});

		if let Some(nonce) = &grant.nonce {
			claims["nonce"] = nonce.as_str().into();
		}

		for (name, value) in grant.claims.as_object().unwrap() {
			claims[name] = value.clone();
		}

		match tamper {
			Tamper::Audience => claims["aud"] = "someone-else".into(),
			Tamper::Nonce => claims["nonce"] = "a-nonce-of-its-own".into(),
			Tamper::NoNonce => {
				claims.as_object_mut().unwrap().remove("nonce");
			}
			Tamper::Expired => {
				claims["exp"] = (now - 3600).into();
				claims["iat"] = (now - 3900).into();
			}
			Tamper::Issuer => claims["iss"] = "https://evil.example".into(),
			_ => {}
		}

		let header = if tamper == Tamper::Unsigned {
			json!({ "alg": "none" })
		} else {
			json!({ "alg": "ES256", "kid": "k1" })
		};
		let signed = format!(
			"{}.{}",
			URL_SAFE_NO_PAD.encode(header.to_string()),
			URL_SAFE_NO_PAD.encode(claims.to_string())
		);
		let pair = if tamper == Tamper::ForeignKey {
			&self.foreign
		} else {
			&self.key
		};
		let signature = pair.sign(&SystemRandom::new(), signed.as_bytes()).unwrap();
		format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
	}

	fn jwks(&self) -> Value {
		let point = self.key.public_key().as_ref();
		json!({
			"keys": [{
				"kty": "EC",
				"crv": "P-256",
				"kid": "k1",
				"use": "sig",
				"alg": "ES256",
				"x": URL_SAFE_NO_PAD.encode(&point[1..33]),
				"y": URL_SAFE_NO_PAD.encode(&point[33..]),
			}]
		})
	}

	/// The token endpoint: checks the code, the redirect, the client (in
	/// the form, or with HTTP Basic where the provider wants that), and
	/// PKCE, then answers with tokens.
	fn token(&self, request: &ProviderRequest) -> (u16, Value) {
		let body = request.body.as_deref().unwrap_or_default();
		let json_body = header(request, "content-type") == Some("application/json");
		let form: HashMap<String, String> = if json_body {
			serde_json::from_str(body).unwrap()
		} else {
			url::form_urlencoded::parse(body.as_bytes())
				.into_owned()
				.collect()
		};
		let Some(code) = form.get("code") else {
			return (400, json!({ "error": "invalid_request" }));
		};
		let Some(grant) = self.state.lock().unwrap().grants.remove(code) else {
			return (400, json!({ "error": "invalid_grant" }));
		};
		let host = url::Url::parse(&request.url)
			.unwrap()
			.host_str()
			.unwrap()
			.to_owned();
		let basic = header(request, "authorization")
			.and_then(|value| value.strip_prefix("Basic "))
			.map(|credentials| {
				let decoded = String::from_utf8(STANDARD.decode(credentials).unwrap()).unwrap();
				let (id, secret) = decoded.split_once(':').unwrap();
				(decode(id), decode(secret))
			});
		let (client_id, secret) = match &basic {
			Some((id, secret)) => (Some(id.clone()), Some(secret.clone())),
			None => {
				(
					form.get(grant.spec.client_param).cloned(),
					form.get("client_secret").cloned(),
				)
			}
		};
		let authenticated_as_asked = if grant.spec.basic {
			basic.is_some() && !form.contains_key("client_secret")
		} else {
			basic.is_none()
		};
		let pkce_ok = match form.get("code_verifier") {
			Some(verifier) => {
				grant.spec.pkce
					&& URL_SAFE_NO_PAD.encode(digest(&SHA256, verifier.as_bytes()))
						== grant.challenge
			}
			None => !grant.spec.pkce,
		};
		let client_ok = match grant.issuer.as_str() {
			"https://appleid.apple.com" => {
				self.apple_secret_ok(secret.as_deref().unwrap_or_default(), &grant)
			}
			_ => secret.is_none_or(|secret| secret == SECRET),
		};

		if host != grant.spec.token_host
			|| form["grant_type"] != "authorization_code"
			|| client_id.as_deref() != Some(grant.client_id.as_str())
			|| form["redirect_uri"] != grant.redirect_uri
			|| !pkce_ok
			|| json_body != grant.spec.json
			|| !authenticated_as_asked
			|| !client_ok
		{
			return (400, json!({ "error": "invalid_grant" }));
		}

		if grant.spec.issuer.is_none() {
			let access = URL_SAFE_NO_PAD.encode(digest(&SHA256, code.as_bytes()));
			// Some providers, such as Notion, say who signed in right here.
			let mut answer = grant
				.claims
				.get("token")
				.cloned()
				.unwrap_or_else(|| json!({}));
			answer["access_token"] = access.as_str().into();
			answer["token_type"] = "bearer".into();
			self.state
				.lock()
				.unwrap()
				.tokens
				.insert(access, grant.claims);
			return (200, answer);
		}

		(
			200,
			json!({ "access_token": "opaque", "id_token": self.id_token(&grant) }),
		)
	}

	/// Whether an Apple client secret is an ES256 JWT for the grant's
	/// client, signed by the trusted key.
	fn apple_secret_ok(&self, secret: &str, grant: &Grant) -> bool {
		let Some(public) = self.state.lock().unwrap().apple_key.clone() else {
			return false;
		};
		let parts: Vec<&str> = secret.split('.').collect();
		let [header, claims, signature] = parts.as_slice() else {
			return false;
		};
		let signed = format!("{header}.{claims}");
		let signature = URL_SAFE_NO_PAD.decode(signature).unwrap();
		let claims: Value =
			serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims).unwrap()).unwrap();
		UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, public)
			.verify(signed.as_bytes(), &signature)
			.is_ok()
			&& claims["sub"] == grant.client_id
			&& claims["aud"] == "https://appleid.apple.com"
	}

	/// A profile API: the person's `emails` for a list of addresses, and
	/// their `user` for anything else.
	fn profile(&self, url: &url::Url, authorization: Option<&str>) -> (u16, Value) {
		let token = authorization
			.and_then(|value| value.strip_prefix("Bearer "))
			.unwrap_or_default();
		let Some(claims) = self.state.lock().unwrap().tokens.get(token).cloned() else {
			return (401, json!({ "message": "Bad credentials" }));
		};

		if url.path().ends_with("/emails") {
			(200, claims["emails"].clone())
		} else {
			(200, claims["user"].clone())
		}
	}

	fn answer(&self, request: &ProviderRequest) -> (u16, Value) {
		let url = url::Url::parse(&request.url).unwrap();
		let host = url.host_str().unwrap_or_default();
		let named = header(request, "user-agent").is_some_and(|agent| agent.starts_with("web:"));

		if NAMED_APPS_ONLY.contains(&host) && !named {
			return (429, json!({ "message": "Too Many Requests" }));
		}

		match request.method {
			"GET" if request.url == "https://sso.example/.well-known/openid-configuration" => {
				(
					200,
					json!({
						"issuer": ISSUER,
						"authorization_endpoint": "https://sso.example/authorize",
						"token_endpoint": "https://sso.example/token",
						"jwks_uri": "https://sso.example/jwks",
					}),
				)
			}
			"GET" if KEY_SETS.contains(&request.url.as_str()) => (200, self.jwks()),
			"GET" if host == "login.microsoftonline.com" => (200, self.jwks()),
			"GET" if API_HOSTS.contains(&host) => {
				self.profile(&url, header(request, "authorization"))
			}
			"POST" if SPECS.iter().any(|spec| spec.token_host == host) => self.token(request),
			_ => (404, json!({ "error": "not_found" })),
		}
	}
}

fn header<'a>(request: &'a ProviderRequest, name: &str) -> Option<&'a str> {
	request
		.headers
		.iter()
		.find(|(key, _)| key.eq_ignore_ascii_case(name))
		.map(|(_, value)| value.as_str())
}

fn decode(part: &str) -> String {
	url::form_urlencoded::parse(format!("x={part}").as_bytes())
		.next()
		.unwrap()
		.1
		.into_owned()
}

impl ProviderHttp for FakeIdp {
	fn send(&self, request: ProviderRequest) -> ProviderFuture<'_> {
		self.state
			.lock()
			.unwrap()
			.requests
			.push(format!("{} {}", request.method, request.url));
		let (status, body) = self.answer(&request);
		Box::pin(std::future::ready(Ok(ProviderResponse {
			status,
			body: body.to_string().into_bytes(),
		})))
	}
}
