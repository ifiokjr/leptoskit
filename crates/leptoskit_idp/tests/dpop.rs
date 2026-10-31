//! DPoP (RFC 9449), end to end on SQLite and Postgres: tokens bound to an
//! app's key at the token endpoint, and refused at the API, `userinfo`, and
//! the token endpoint without a fresh proof that key signed. Without
//! Postgres binaries the Postgres half is skipped, unless
//! `LEPTOSKIT_REQUIRE_POSTGRES` is set.

#![cfg(all(feature = "axum", feature = "postgres", feature = "sqlite"))]

mod support;

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use leptoskit_idp::ClientAuth;
use leptoskit_idp::ClientCommand;
use leptoskit_idp::ClientCommandOutput;
use leptoskit_idp::GrantType;
use leptoskit_idp::NewClientSpec;
use leptoskit_idp::TokenResponse;
use p256::ecdsa::Signature;
use p256::ecdsa::SigningKey;
use p256::ecdsa::signature::Signer;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use sha2::Digest;
use support::*;

/// Makes each proof's `jti` unique.
static PROOFS: AtomicU64 = AtomicU64::new(0);

/// An app's DPoP key.
fn key(byte: u8) -> SigningKey {
	SigningKey::from_slice(&[byte; 32]).unwrap()
}

/// The public half of `key`, as a proof's header carries it.
fn jwk(key: &SigningKey) -> Value {
	let point = key.verifying_key().to_sec1_point(false);
	let bytes = point.as_bytes();
	json!({
		"kty": "EC",
		"crv": "P-256",
		"x": URL_SAFE_NO_PAD.encode(&bytes[1..33]),
		"y": URL_SAFE_NO_PAD.encode(&bytes[33..65]),
	})
}

/// `key`'s RFC 7638 thumbprint, worked out independently of the provider.
fn thumbprint(key: &SigningKey) -> String {
	let jwk = jwk(key);
	let canonical = format!(
		r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#,
		jwk["x"].as_str().unwrap(),
		jwk["y"].as_str().unwrap()
	);
	URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(canonical.as_bytes()))
}

/// A JWT with this header and these claims, signed with `key`.
fn signed(key: &SigningKey, header: &Value, claims: &Value) -> String {
	let input = format!(
		"{}.{}",
		URL_SAFE_NO_PAD.encode(header.to_string()),
		URL_SAFE_NO_PAD.encode(claims.to_string())
	);
	let signature: Signature = key.sign(input.as_bytes());
	format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

/// The claims of a proof for `method` on `path`, made now.
fn proof_claims(method: &str, path: &str, token: Option<&str>) -> Value {
	let mut claims = json!({
		"jti": format!("proof-{}", PROOFS.fetch_add(1, Ordering::Relaxed)),
		"htm": method,
		"htu": format!("{ORIGIN}{path}"),
		"iat": leptoskit_db::now().timestamp(),
	});
	if let Some(token) = token {
		claims["ath"] = json!(URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(token.as_bytes())));
	}
	claims
}

/// A DPoP proof `key` signs for `method` on `path`, sent with `token`.
fn proof(key: &SigningKey, method: &str, path: &str, token: Option<&str>) -> String {
	signed(
		key,
		&json!({ "typ": "dpop+jwt", "alg": "ES256", "jwk": jwk(key) }),
		&proof_claims(method, path, token),
	)
}

/// What a token request sends.
struct TokenCall<'a> {
	pairs: &'a [(&'a str, &'a str)],
	authorization: Option<&'a str>,
	proofs: &'a [&'a str],
}

impl App {
	/// A form post to the token endpoint with these `DPoP` headers.
	async fn token_with(&self, call: TokenCall<'_>) -> Response {
		let form = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs(call.pairs)
			.finish();
		let mut builder = Request::post("/oauth/token")
			.header(header::ORIGIN, "https://partner.example")
			.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
		if let Some(authorization) = call.authorization {
			builder = builder.header(header::AUTHORIZATION, authorization);
		}
		for proof in call.proofs {
			builder = builder.header("dpop", *proof);
		}
		self.send(builder.body(Body::from(form)).unwrap()).await
	}

	/// A call to `path` with `Authorization: <scheme> <token>` and these
	/// `DPoP` headers.
	async fn call(
		&self,
		method: &str,
		path: &str,
		scheme: &str,
		token: &str,
		proofs: &[&str],
	) -> Response {
		let mut builder = Request::builder()
			.method(method)
			.uri(path)
			.header(header::ORIGIN, "https://partner.example")
			.header(header::AUTHORIZATION, format!("{scheme} {token}"));
		for proof in proofs {
			builder = builder.header("dpop", *proof);
		}
		self.send(builder.body(Body::empty()).unwrap()).await
	}

	/// Signs `address` in to the public app and trades the code with a proof
	/// `key` signs.
	async fn bound_tokens(&self, client_id: &str, key: &SigningKey) -> TokenResponse {
		let cookie = self.sign_in("ada@example.com").await;
		let code = self.code(client_id, "openid notes:read", &cookie).await;
		let proof = proof(key, "POST", "/oauth/token", None);
		let response = self
			.token_with(TokenCall {
				pairs: &[
					("grant_type", "authorization_code"),
					("code", &code),
					("redirect_uri", CALLBACK),
					("code_verifier", VERIFIER),
					("client_id", client_id),
				],
				authorization: None,
				proofs: &[&proof],
			})
			.await;
		assert_eq!(response.status(), StatusCode::OK);
		body(response).await
	}

	async fn refresh_with(&self, client_id: &str, refresh: &str, proofs: &[&str]) -> Response {
		self.token_with(TokenCall {
			pairs: &[
				("grant_type", "refresh_token"),
				("refresh_token", refresh),
				("client_id", client_id),
			],
			authorization: None,
			proofs,
		})
		.await
	}
}

/// The challenge a refused call carries.
fn challenge(response: &Response) -> String {
	response.headers()[header::WWW_AUTHENTICATE]
		.to_str()
		.unwrap()
		.to_owned()
}

#[tokio::test]
async fn a_public_app_binds_its_tokens_to_its_key() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app_with(db, |idp| idp.dpop(true)).await;
		assert_eq!(
			app.idp.discovery()["dpop_signing_alg_values_supported"],
			json!(["ES256", "RS256"])
		);
		let (client_id, _) = app
			.register(ClientAuth::None, &["openid", "notes:read"])
			.await;
		let mine = key(7);
		let tokens = app.bound_tokens(&client_id, &mine).await;
		let access = tokens.access_token.as_str();

		assert_eq!(tokens.token_type, "DPoP");
		assert_eq!(claims(access)["cnf"]["jkt"], json!(thumbprint(&mine)));

		// With a proof for this call, the API takes it.
		let fresh = proof(&mine, "POST", "/api/list_notes", Some(access));
		let response = app
			.call("POST", "/api/list_notes", "DPoP", access, &[&fresh])
			.await;
		assert_eq!(response.status(), StatusCode::OK);
		assert!(text(response).await.ends_with("sees 1"));

		// A stolen copy is useless: the same proof again, as a bearer token,
		// with no proof, with another key's, or for another call.
		let replayed = app
			.call("POST", "/api/list_notes", "DPoP", access, &[&fresh])
			.await;
		assert_eq!(replayed.status(), StatusCode::UNAUTHORIZED);
		assert_eq!(
			challenge(&replayed),
			r#"DPoP error="invalid_dpop_proof", algs="ES256 RS256""#
		);
		let as_bearer = app.with_bearer("POST", "/api/list_notes", access).await;
		assert_eq!(as_bearer.status(), StatusCode::UNAUTHORIZED);
		assert_eq!(challenge(&as_bearer), r#"Bearer error="invalid_token""#);
		let unproved = app
			.call("POST", "/api/list_notes", "DPoP", access, &[])
			.await;
		assert_eq!(unproved.status(), StatusCode::UNAUTHORIZED);
		let theirs = proof(&key(8), "POST", "/api/list_notes", Some(access));
		let wrong_key = app
			.call("POST", "/api/list_notes", "DPoP", access, &[&theirs])
			.await;
		assert_eq!(wrong_key.status(), StatusCode::UNAUTHORIZED);
		for (method, path, token) in [
			("GET", "/api/list_notes", Some(access)),
			("POST", "/api/write_note", Some(access)),
			("POST", "/api/list_notes", Some("another-token")),
			("POST", "/api/list_notes", None),
		] {
			let other = proof(&mine, method, path, token);
			let response = app
				.call("POST", "/api/list_notes", "DPoP", access, &[&other])
				.await;
			assert_eq!(
				response.status(),
				StatusCode::UNAUTHORIZED,
				"{method} {path}"
			);
		}

		// Beyond the proof, scopes still decide what it may call.
		let write = proof(&mine, "POST", "/api/write_note", Some(access));
		let response = app
			.call("POST", "/api/write_note", "DPoP", access, &[&write])
			.await;
		assert_eq!(response.status(), StatusCode::FORBIDDEN);
		assert!(challenge(&response).starts_with("DPoP error=\"insufficient_scope\""));
		let me = proof(&mine, "GET", "/me", Some(access));
		let response = app.call("GET", "/me", "DPoP", access, &[&me]).await;
		assert_eq!(response.status(), StatusCode::FORBIDDEN);

		// `userinfo` asks for a proof too.
		let info = proof(&mine, "GET", "/oauth/userinfo", Some(access));
		let response = app
			.call("GET", "/oauth/userinfo", "DPoP", access, &[&info])
			.await;
		assert_eq!(response.status(), StatusCode::OK);
		assert_eq!(body::<Value>(response).await["sub"], claims(access)["sub"]);
		let response = app.with_bearer("GET", "/oauth/userinfo", access).await;
		assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

		// The refresh token is bound to the key as well, and refusing a proof
		// doesn't spend it.
		let refresh = tokens.refresh_token.unwrap();
		let refused = app.refresh_with(&client_id, &refresh, &[]).await;
		assert_eq!(
			App::error(refused, StatusCode::BAD_REQUEST).await,
			"invalid_dpop_proof"
		);
		let stolen = proof(&key(9), "POST", "/oauth/token", None);
		let refused = app.refresh_with(&client_id, &refresh, &[&stolen]).await;
		assert_eq!(
			App::error(refused, StatusCode::BAD_REQUEST).await,
			"invalid_dpop_proof"
		);
		let renewal = proof(&mine, "POST", "/oauth/token", None);
		let renewed = app.refresh_with(&client_id, &refresh, &[&renewal]).await;
		assert_eq!(renewed.status(), StatusCode::OK);
		let renewed: TokenResponse = body(renewed).await;
		assert_eq!(renewed.token_type, "DPoP");
		assert_eq!(
			claims(&renewed.access_token)["cnf"]["jkt"],
			json!(thumbprint(&mine))
		);

		// A resource server introspecting it learns the key it needs.
		let (resource, secret) = app
			.register_spec(NewClientSpec {
				introspection: true,
				..spec(ClientAuth::SecretBasic, &["notes:read"])
			})
			.await;
		let described = app
			.introspect(&resource, &secret.unwrap(), &renewed.access_token)
			.await;
		assert!(described.active);
		assert_eq!(described.token_type.as_deref(), Some("DPoP"));
		assert_eq!(described.cnf, Some(json!({ "jkt": thumbprint(&mine) })));
	}
}

#[tokio::test]
async fn confidential_and_dpop_bound_apps() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app_with(db, |idp| idp.dpop(true)).await;
		let mine = key(3);

		// A confidential app's access tokens are bound when it sends a proof,
		// but its refresh tokens are bound by its secret, not its key.
		let (client_id, secret) = app
			.register(ClientAuth::SecretBasic, &["openid", "notes:read"])
			.await;
		let secret = secret.unwrap();
		let cookie = app.sign_in("ada@example.com").await;
		let code = app.code(&client_id, "openid", &cookie).await;
		let token_proof = proof(&mine, "POST", "/oauth/token", None);
		let authorization = basic(&client_id, &secret);
		let response = app
			.token_with(TokenCall {
				pairs: &[
					("grant_type", "authorization_code"),
					("code", &code),
					("redirect_uri", CALLBACK),
					("code_verifier", VERIFIER),
				],
				authorization: Some(&authorization),
				proofs: &[&token_proof],
			})
			.await;
		assert_eq!(response.status(), StatusCode::OK);
		let tokens: TokenResponse = body(response).await;
		assert_eq!(tokens.token_type, "DPoP");
		let renewed = app
			.refresh(&client_id, &secret, &tokens.refresh_token.unwrap())
			.await;
		assert_eq!(renewed.status(), StatusCode::OK);
		let renewed: TokenResponse = body(renewed).await;
		assert_eq!(renewed.token_type, "Bearer");
		assert!(claims(&renewed.access_token).get("cnf").is_none());

		// An app registered as DPoP-bound must always send one.
		let output = app
			.idp
			.run_command(&ClientCommand::Add(NewClientSpec {
				grant_types: vec![GrantType::ClientCredentials],
				redirect_uris: Vec::new(),
				dpop_bound: true,
				..spec(ClientAuth::SecretBasic, &["notes:read"])
			}))
			.await;
		let ClientCommandOutput::Added { client, secret } = output else {
			panic!("the app should register: {output:?}");
		};
		assert!(client.dpop_bound);
		let authorization = basic(&client.client_id, &secret.unwrap());
		let refused = app
			.token_with(TokenCall {
				pairs: &[("grant_type", "client_credentials")],
				authorization: Some(&authorization),
				proofs: &[],
			})
			.await;
		assert_eq!(
			App::error(refused, StatusCode::BAD_REQUEST).await,
			"invalid_dpop_proof"
		);
		let machine_proof = proof(&mine, "POST", "/oauth/token", None);
		let response = app
			.token_with(TokenCall {
				pairs: &[("grant_type", "client_credentials")],
				authorization: Some(&authorization),
				proofs: &[&machine_proof],
			})
			.await;
		assert_eq!(response.status(), StatusCode::OK);
		let machine: TokenResponse = body(response).await;
		assert_eq!(machine.token_type, "DPoP");
		let call = proof(
			&mine,
			"POST",
			"/api/count_notes",
			Some(&machine.access_token),
		);
		let response = app
			.call(
				"POST",
				"/api/count_notes",
				"dpop",
				&machine.access_token,
				&[&call],
			)
			.await;
		assert_eq!(response.status(), StatusCode::OK);
		assert_eq!(
			text(response).await,
			format!("{} as itself", client.client_id)
		);

		// A bearer token sent as DPoP is refused, whatever the proof.
		let bearer = renewed.access_token;
		let call = proof(&mine, "POST", "/api/list_notes", Some(&bearer));
		let response = app
			.call("POST", "/api/list_notes", "DPoP", &bearer, &[&call])
			.await;
		assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
		assert_eq!(
			challenge(&response),
			r#"DPoP error="invalid_token", algs="ES256 RS256""#
		);
	}
}

#[tokio::test]
async fn malformed_proofs_are_refused_at_the_token_endpoint() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app_with(db, |idp| idp.dpop(true)).await;
		let (client_id, secret) = app
			.register_spec(NewClientSpec {
				grant_types: vec![GrantType::ClientCredentials],
				redirect_uris: Vec::new(),
				..spec(ClientAuth::SecretBasic, &["notes:read"])
			})
			.await;
		let authorization = basic(&client_id, &secret.unwrap());
		let mine = key(5);
		let header = |typ: &str| json!({ "typ": typ, "alg": "ES256", "jwk": jwk(&mine) });
		let token_claims = || proof_claims("POST", "/oauth/token", None);
		let without = |name: &str| {
			let mut claims = token_claims();
			claims.as_object_mut().unwrap().remove(name);
			claims
		};
		let mut private = jwk(&mine);
		private["d"] = json!("secret");
		let mut old = token_claims();
		old["iat"] = json!(leptoskit_db::now().timestamp() - 600);
		let mut future = token_claims();
		future["iat"] = json!(leptoskit_db::now().timestamp() + 600);
		let mut elsewhere = token_claims();
		elsewhere["htu"] = json!("https://other.example/oauth/token");
		let mut long_jti = token_claims();
		long_jti["jti"] = json!("j".repeat(257));

		let spent = signed(&mine, &header("dpop+jwt"), &token_claims());
		let malformed = [
			"not a jwt".to_owned(),
			signed(
				&mine,
				&json!({ "typ": "dpop+jwt", "alg": "ES256" }),
				&token_claims(),
			),
			signed(
				&mine,
				&json!({ "typ": "dpop+jwt", "alg": "ES256", "jwk": private }),
				&token_claims(),
			),
			signed(
				&mine,
				&json!({ "typ": "dpop+jwt", "alg": "ES256", "jwk": { "kty": "oct" } }),
				&token_claims(),
			),
			signed(&mine, &header("JWT"), &token_claims()),
			signed(
				&key(6),
				&json!({ "typ": "dpop+jwt", "alg": "ES256", "jwk": jwk(&key(6)) }),
				&proof_claims("GET", "/oauth/token", None),
			),
			signed(&mine, &header("dpop+jwt"), &elsewhere),
			signed(&mine, &header("dpop+jwt"), &without("iat")),
			signed(&mine, &header("dpop+jwt"), &old),
			signed(&mine, &header("dpop+jwt"), &future),
			signed(&mine, &header("dpop+jwt"), &without("jti")),
			signed(&mine, &header("dpop+jwt"), &long_jti),
		];

		let first = app
			.token_with(TokenCall {
				pairs: &[("grant_type", "client_credentials")],
				authorization: Some(&authorization),
				proofs: &[&spent],
			})
			.await;
		assert_eq!(first.status(), StatusCode::OK);

		for (index, proof) in malformed.iter().chain(std::iter::once(&spent)).enumerate() {
			let refused = app
				.token_with(TokenCall {
					pairs: &[("grant_type", "client_credentials")],
					authorization: Some(&authorization),
					proofs: &[proof],
				})
				.await;
			assert_eq!(
				App::error(refused, StatusCode::BAD_REQUEST).await,
				"invalid_dpop_proof",
				"proof {index}"
			);
		}

		let one = proof(&mine, "POST", "/oauth/token", None);
		let two = proof(&mine, "POST", "/oauth/token", None);
		let refused = app
			.token_with(TokenCall {
				pairs: &[("grant_type", "client_credentials")],
				authorization: Some(&authorization),
				proofs: &[&one, &two],
			})
			.await;
		assert_eq!(
			App::error(refused, StatusCode::BAD_REQUEST).await,
			"invalid_dpop_proof"
		);
	}
}

#[tokio::test]
async fn without_dpop_on_proofs_are_ignored() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(db).await;
		assert!(
			app.idp
				.discovery()
				.get("dpop_signing_alg_values_supported")
				.is_none()
		);
		let (client_id, _) = app
			.register(ClientAuth::None, &["openid", "notes:read"])
			.await;
		let tokens = app.bound_tokens(&client_id, &key(4)).await;
		assert_eq!(tokens.token_type, "Bearer");
		assert!(claims(&tokens.access_token).get("cnf").is_none());

		// `DPoP` isn't a scheme this server knows, so the call isn't the
		// person's.
		let response = app
			.call("POST", "/api/list_notes", "DPoP", &tokens.access_token, &[])
			.await;
		assert!(response.status().is_client_error(), "{response:?}");

		let output = app
			.idp
			.run_command(&ClientCommand::Add(NewClientSpec {
				dpop_bound: true,
				..spec(ClientAuth::None, &["openid"])
			}))
			.await;
		let ClientCommandOutput::Failed { message: error } = output else {
			panic!("a DPoP-bound app needs DPoP on: {output:?}");
		};
		assert!(error.contains("dpop"), "{error}");
	}
}
