//! A fake identity provider, answering from memory: OpenID Connect
//! discovery, a key set, an authorization step, and a token endpoint that
//! checks PKCE and the client's credentials, then signs ID tokens with a
//! P-256 key made at test time. It answers for a generic issuer
//! (`https://sso.example`) and at Google's, Apple's, Microsoft's, and
//! GitHub's own URLs, so their definitions run as they would against the
//! real thing, without a network.

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
	/// Issues a token that expired an hour ago.
	Expired,
	/// Names another issuer.
	Issuer,
	/// Signs with `alg: none`.
	Unsigned,
}

/// An authorization the person granted, waiting for its code.
struct Grant {
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
		assert_eq!(query["code_challenge_method"], "S256");
		let host = url.host_str().unwrap();
		let issuer = match host {
			"accounts.google.com" => "https://accounts.google.com".to_owned(),
			"appleid.apple.com" => "https://appleid.apple.com".to_owned(),
			"login.microsoftonline.com" => {
				format!(
					"https://login.microsoftonline.com/{}/v2.0",
					claims["tid"].as_str().unwrap()
				)
			}
			"github.com" => "https://github.com".to_owned(),
			_ => ISSUER.to_owned(),
		};

		if host == "appleid.apple.com" {
			assert_eq!(query["response_mode"], "form_post");
		}

		let code = URL_SAFE_NO_PAD.encode(digest(&SHA256, url.as_str().as_bytes()));
		self.state.lock().unwrap().grants.insert(
			code.clone(),
			Grant {
				client_id: query["client_id"].clone(),
				redirect_uri: query["redirect_uri"].clone(),
				challenge: query["code_challenge"].clone(),
				nonce: query.get("nonce").cloned(),
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
			"nonce": grant.nonce,
		});

		for (name, value) in grant.claims.as_object().unwrap() {
			claims[name] = value.clone();
		}

		match tamper {
			Tamper::Audience => claims["aud"] = "someone-else".into(),
			Tamper::Nonce => claims["nonce"] = "a-nonce-of-its-own".into(),
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

	/// The token endpoint: checks the code, the redirect, the client, and
	/// PKCE, then answers with tokens.
	fn token(&self, body: &str) -> (u16, Value) {
		let form: HashMap<String, String> = url::form_urlencoded::parse(body.as_bytes())
			.into_owned()
			.collect();
		let Some(grant) = self.state.lock().unwrap().grants.remove(&form["code"]) else {
			return (400, json!({ "error": "invalid_grant" }));
		};
		let verifier = URL_SAFE_NO_PAD.encode(digest(&SHA256, form["code_verifier"].as_bytes()));
		let client_ok = match grant.issuer.as_str() {
			"https://appleid.apple.com" => self.apple_secret_ok(&form["client_secret"], &grant),
			_ => {
				form.get("client_secret")
					.is_none_or(|secret| secret == SECRET)
			}
		};

		if form["grant_type"] != "authorization_code"
			|| form["client_id"] != grant.client_id
			|| form["redirect_uri"] != grant.redirect_uri
			|| verifier != grant.challenge
			|| !client_ok
		{
			return (400, json!({ "error": "invalid_grant" }));
		}

		if grant.issuer == "https://github.com" {
			let access = URL_SAFE_NO_PAD.encode(digest(&SHA256, form["code"].as_bytes()));
			self.state
				.lock()
				.unwrap()
				.tokens
				.insert(access.clone(), grant.claims);
			return (
				200,
				json!({ "access_token": access, "token_type": "bearer" }),
			);
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

	fn github(&self, path: &str, authorization: Option<&str>) -> (u16, Value) {
		let token = authorization
			.and_then(|value| value.strip_prefix("Bearer "))
			.unwrap_or_default();
		let Some(claims) = self.state.lock().unwrap().tokens.get(token).cloned() else {
			return (401, json!({ "message": "Bad credentials" }));
		};

		match path {
			"/user" => (200, claims["user"].clone()),
			"/user/emails" => (200, claims["emails"].clone()),
			_ => (404, json!({})),
		}
	}

	fn answer(&self, request: &ProviderRequest) -> (u16, Value) {
		let url = request.url.as_str();
		let header = |name: &str| {
			request
				.headers
				.iter()
				.find(|(key, _)| key.eq_ignore_ascii_case(name))
				.map(|(_, value)| value.as_str())
		};

		match (request.method, url) {
			("GET", "https://sso.example/.well-known/openid-configuration") => {
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
			(
				"GET",
				"https://sso.example/jwks"
				| "https://www.googleapis.com/oauth2/v3/certs"
				| "https://appleid.apple.com/auth/keys",
			) => (200, self.jwks()),
			("GET", url) if url.starts_with("https://login.microsoftonline.com/") => {
				(200, self.jwks())
			}
			(
				"POST",
				"https://sso.example/token"
				| "https://oauth2.googleapis.com/token"
				| "https://appleid.apple.com/auth/token"
				| "https://github.com/login/oauth/access_token",
			) => self.token(request.body.as_deref().unwrap_or_default()),
			("POST", url) if url.starts_with("https://login.microsoftonline.com/") => {
				self.token(request.body.as_deref().unwrap_or_default())
			}
			("GET", url) if url.starts_with("https://api.github.com") => {
				self.github(
					url.trim_start_matches("https://api.github.com"),
					header("authorization"),
				)
			}
			_ => (404, json!({ "error": "not_found" })),
		}
	}
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
