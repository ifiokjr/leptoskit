//! Dynamic client registration (RFC 7591 and 7592), end to end on SQLite
//! and Postgres: an app registers itself, signs someone in, reads, changes
//! and deletes its registration, and everything it isn't allowed to ask
//! for is refused. Without Postgres binaries the Postgres half is skipped,
//! unless `LEPTOSKIT_REQUIRE_POSTGRES` is set.

#![cfg(all(feature = "axum", feature = "postgres", feature = "sqlite"))]

mod support;

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use leptoskit_idp::ClientMetadata;
use leptoskit_idp::ClientRegistration;
use leptoskit_idp::Registration;
use leptoskit_idp::TokenResponse;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use support::*;

/// An initial access token, as an operator would put in the environment.
const INITIAL: &str = "an-initial-access-token-of-at-least-32-characters";

impl App {
	/// A JSON request to a registration route, with `Authorization: Bearer
	/// <token>` when there is one.
	async fn registration(
		&self,
		method: &str,
		path: &str,
		token: Option<&str>,
		body: Option<Value>,
	) -> Response {
		let mut builder = Request::builder()
			.method(method)
			.uri(path)
			.header(header::ORIGIN, "https://partner.example")
			.header(header::CONTENT_TYPE, "application/json");

		if let Some(token) = token {
			builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
		}

		let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
		self.send(builder.body(body).unwrap()).await
	}

	async fn register_json(&self, token: Option<&str>, metadata: Value) -> Response {
		self.registration("POST", "/oauth/register", token, Some(metadata))
			.await
	}
}

fn partner() -> Value {
	json!({
		"client_name": "Partner",
		"redirect_uris": [CALLBACK],
		"post_logout_redirect_uris": [SIGNED_OUT],
		"scope": "openid notes:read",
		"logo_uri": "https://partner.example/logo.png",
		"contacts": ["dev@partner.example"],
	})
}

fn path_of(registration: &ClientRegistration) -> String {
	registration
		.registration_client_uri
		.strip_prefix(ORIGIN)
		.unwrap()
		.to_owned()
}

#[tokio::test]
#[expect(
	clippy::too_many_lines,
	reason = "an app's whole registration, from signing up to deleting it, reads best top to \
	          bottom"
)]
async fn an_app_registers_itself_signs_someone_in_and_manages_its_registration() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app_with(db, |idp| idp.dynamic_registration(Registration::default())).await;
		assert_eq!(
			app.idp.discovery()["registration_endpoint"],
			"https://notes.example/oauth/register"
		);

		let response = app.register_json(None, partner()).await;
		assert_eq!(response.status(), StatusCode::CREATED);
		assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
		let registered: ClientRegistration = body(response).await;
		let secret = registered.client_secret.clone().unwrap();
		let manager = registered.registration_access_token.clone().unwrap();
		assert_eq!(registered.client_name, "Partner");
		assert_eq!(registered.client_secret_expires_at, Some(0));

		assert_eq!(
			registered.registration_client_uri,
			format!("{ORIGIN}/oauth/register/{}", registered.client_id)
		);
		assert_eq!(
			registered.grant_types,
			["authorization_code", "refresh_token"]
		);
		assert_eq!(registered.response_types, ["code"]);
		assert_eq!(registered.token_endpoint_auth_method, "client_secret_basic");
		assert_eq!(registered.id_token_signed_response_alg, "ES256");
		assert_eq!(registered.subject_type, "pairwise");
		assert_eq!(registered.scope, "openid notes:read");
		assert!(!format!("{registered:?}").contains(&secret));

		// It works like any other app.
		let client_id = registered.client_id.clone();
		let cookie = app.sign_in("ada@example.com").await;
		let code = app.code(&client_id, "openid notes:read", &cookie).await;
		let response = app
			.exchange(&client_id, Some(&secret), &code, VERIFIER)
			.await;
		assert_eq!(response.status(), StatusCode::OK);
		let tokens: TokenResponse = body(response).await;
		let listed = app.idp.clients().await.unwrap();
		let info = listed
			.iter()
			.find(|client| client.client_id == client_id)
			.unwrap();

		assert!(info.self_registered && !info.first_party && !info.introspection);

		// It reads its registration back, without the secret.
		let path = path_of(&registered);
		let response = app.registration("GET", &path, Some(&manager), None).await;
		assert_eq!(response.status(), StatusCode::OK);
		let read: ClientRegistration = body(response).await;
		assert_eq!(read.client_secret, None);
		assert_eq!(read.registration_access_token, None);
		assert_eq!(read.redirect_uris, [CALLBACK]);

		// Nobody else can.
		for token in [None, Some("not-the-token"), Some(secret.as_str())] {
			let response = app.registration("GET", &path, token, None).await;
			assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{token:?}");
			assert_eq!(
				response.headers()[header::WWW_AUTHENTICATE],
				r#"Bearer error="invalid_token""#
			);
		}

		let other: ClientRegistration = body(app.register_json(None, partner()).await).await;
		let response = app
			.registration(
				"GET",
				&path,
				other.registration_access_token.as_deref(),
				None,
			)
			.await;
		assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
		let unknown = app
			.registration("GET", "/oauth/register/nobody", Some(&manager), None)
			.await;

		assert_eq!(unknown.status(), StatusCode::UNAUTHORIZED);

		// A change must name the app, can't set the server's fields, and
		// must send the current secret if it sends one.
		let mut change = partner();
		for (field, value, error) in [
			(
				"client_id",
				json!("someone-else"),
				"invalid_client_metadata",
			),
			("client_secret", json!("wrong"), "invalid_client_metadata"),
			("client_id_issued_at", json!(1), "invalid_client_metadata"),
		] {
			let mut sent = change.clone();
			sent["client_id"] = json!(client_id);
			sent[field] = value;
			let response = app
				.registration("PUT", &path, Some(&manager), Some(sent))
				.await;
			assert_eq!(App::error(response, StatusCode::BAD_REQUEST).await, error);
		}

		// This server has no RSA key, so a change can't ask for RS256 either.
		let mut rsa = change.clone();
		rsa["client_id"] = json!(client_id);
		rsa["id_token_signed_response_alg"] = json!("RS256");
		let response = app
			.registration("PUT", &path, Some(&manager), Some(rsa))
			.await;
		assert_eq!(
			App::error(response, StatusCode::BAD_REQUEST).await,
			"invalid_client_metadata"
		);

		// Renamed, with `notes:read` taken away: everyone who connected it
		// loses that scope, and the sign-ins that carried it end.
		change["client_id"] = json!(client_id);
		change["client_secret"] = json!(secret);
		change["client_name"] = json!("Partner Two");
		change["scope"] = json!("openid");
		let response = app
			.registration("PUT", &path, Some(&manager), Some(change.clone()))
			.await;
		assert_eq!(response.status(), StatusCode::OK);
		let changed: ClientRegistration = body(response).await;
		assert_eq!(changed.client_name, "Partner Two");

		assert_eq!(changed.scope, "openid");
		assert_eq!(changed.client_secret, None);
		let refreshed = app
			.refresh(&client_id, &secret, tokens.refresh_token.as_ref().unwrap())
			.await;
		assert_eq!(
			App::error(refreshed, StatusCode::BAD_REQUEST).await,
			"invalid_grant"
		);

		// Becoming a public app drops the secret; becoming confidential
		// again makes a new one, shown in the answer.
		change["token_endpoint_auth_method"] = json!("none");
		change.as_object_mut().unwrap().remove("client_secret");
		let response = app
			.registration("PUT", &path, Some(&manager), Some(change.clone()))
			.await;
		let public: ClientRegistration = body(response).await;
		assert_eq!(public.token_endpoint_auth_method, "none");
		let refused = app.refresh(&client_id, &secret, "anything").await;
		assert_eq!(
			App::error(refused, StatusCode::UNAUTHORIZED).await,
			"invalid_client"
		);
		change["token_endpoint_auth_method"] = json!("client_secret_post");

		let response = app
			.registration("PUT", &path, Some(&manager), Some(change.clone()))
			.await;
		let confidential: ClientRegistration = body(response).await;
		assert!(confidential.client_secret.is_some());
		assert_eq!(confidential.client_secret_expires_at, Some(0));

		// Deleting it ends everything it holds, and the token with it.
		let response = app
			.registration("DELETE", &path, Some(&manager), None)
			.await;
		assert_eq!(response.status(), StatusCode::NO_CONTENT);
		let gone = app.registration("GET", &path, Some(&manager), None).await;
		assert_eq!(gone.status(), StatusCode::UNAUTHORIZED);
		let gone = app
			.registration("DELETE", &path, Some(&manager), None)
			.await;
		assert_eq!(gone.status(), StatusCode::UNAUTHORIZED);
		assert!(
			!app.idp
				.clients()
				.await
				.unwrap()
				.iter()
				.any(|client| client.client_id == client_id)
		);
	}
}

#[tokio::test]
#[expect(
	clippy::too_many_lines,
	reason = "every refusal and every default, in one table and one list"
)]
async fn registrations_ask_only_for_what_the_provider_offers() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app_with(db, |idp| {
			// Every attempt counts against open registration's limit, so
			// this one needs the initial access token.
			idp.dynamic_registration(Registration {
				initial_access_token: Some(INITIAL.into()),
			})
			.rsa_key(TEST_RSA_KEY)
		})
		.await;
		let with = |field: &str, value: Value| {
			let mut metadata = partner();
			metadata[field] = value;
			metadata
		};

		for (metadata, error) in [
			(
				with("redirect_uris", json!(["http://partner.example/cb"])),
				"invalid_redirect_uri",
			),
			(
				with("redirect_uris", json!(["https://partner.example/cb#x"])),
				"invalid_redirect_uri",
			),
			(
				with("jwks_uri", json!("https://partner.example/jwks")),
				"invalid_client_metadata",
			),
			(
				with("grant_types", json!(["implicit"])),
				"invalid_client_metadata",
			),
			(
				with("grant_types", json!(["password"])),
				"invalid_client_metadata",
			),
			(
				with("response_types", json!(["token"])),
				"invalid_client_metadata",
			),
			(
				with("grant_types", json!(["client_credentials"])),
				"invalid_client_metadata",
			),
			(
				with("scope", json!("openid admin")),
				"invalid_client_metadata",
			),
			(
				with("token_endpoint_auth_method", json!("tls_client_auth")),
				"invalid_client_metadata",
			),
			(
				with("id_token_signed_response_alg", json!("HS256")),
				"invalid_client_metadata",
			),
			(
				with("registration_access_token", json!("mine")),
				"invalid_client_metadata",
			),
			(
				with("dpop_bound_access_tokens", json!(true)),
				"invalid_client_metadata",
			),
			(
				with("redirect_uris", json!("not a list")),
				"invalid_client_metadata",
			),
			(
				with("token_endpoint_auth_method", json!("private_key_jwt")),
				"invalid_client_metadata",
			),
		] {
			let response = app.register_json(Some(INITIAL), metadata.clone()).await;
			assert_eq!(
				App::error(response, StatusCode::BAD_REQUEST).await,
				error,
				"{metadata}"
			);
		}

		// What it may ask for, and the defaults it gets.
		let keyed: ClientRegistration = body(
			app.register_json(
				Some(INITIAL),
				json!({
					"redirect_uris": [CALLBACK],
					"token_endpoint_auth_method": "private_key_jwt",
					"jwks": { "keys": [rsa_jwk()] },
					"id_token_signed_response_alg": "ES256",
				}),
			)
			.await,
		)
		.await;
		assert_eq!(keyed.client_name, "partner.example");
		assert_eq!(keyed.client_secret, None);
		assert_eq!(keyed.scope, "openid profile email");
		assert_eq!(keyed.id_token_signed_response_alg, "ES256");
		assert!(keyed.jwks.is_some());

		let machine: ClientRegistration = body(
			app.register_json(
				Some(INITIAL),
				json!({
					"grant_types": ["client_credentials"],
					"response_types": [],
					"scope": "notes:read",
				}),
			)
			.await,
		)
		.await;
		assert_eq!(machine.client_name, "Unnamed app");
		assert_eq!(machine.grant_types, ["client_credentials"]);
		assert_eq!(machine.response_types, [] as [String; 0]);
		// With an RSA key, OpenID Connect's default applies.
		assert_eq!(machine.id_token_signed_response_alg, "RS256");

		let device: ClientRegistration = body(
			app.register_json(
				Some(INITIAL),
				json!({
					"client_name": "Notes CLI",
					"token_endpoint_auth_method": "none",
					"grant_types": [
						"urn:ietf:params:oauth:grant-type:device_code",
						"refresh_token",
					],
					"response_types": [],
				}),
			)
			.await,
		)
		.await;
		assert_eq!(device.token_endpoint_auth_method, "none");
		assert_eq!(
			device.grant_types,
			[
				"urn:ietf:params:oauth:grant-type:device_code",
				"refresh_token"
			]
		);

		let response = app
			.send(
				Request::post("/oauth/register")
					.header(header::CONTENT_TYPE, "text/plain")
					.body(Body::from("name=me"))
					.unwrap(),
			)
			.await;
		assert_eq!(
			App::error(response, StatusCode::BAD_REQUEST).await,
			"invalid_client_metadata"
		);
		let response = app
			.registration(
				"PUT",
				&path_of(&device),
				device.registration_access_token.as_deref(),
				Some(json!("not an object")),
			)
			.await;
		assert_eq!(
			App::error(response, StatusCode::BAD_REQUEST).await,
			"invalid_client_metadata"
		);
		let response = app
			.registration(
				"PUT",
				&path_of(&device),
				device.registration_access_token.as_deref(),
				Some(json!({ "client_id": device.client_id, "scope": "openid admin" })),
			)
			.await;
		assert_eq!(
			App::error(response, StatusCode::BAD_REQUEST).await,
			"invalid_client_metadata"
		);
	}
}

#[tokio::test]
async fn an_initial_access_token_closes_registration() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app_with(db, |idp| {
			idp.dynamic_registration(Registration {
				initial_access_token: Some(INITIAL.into()),
			})
			.dpop(true)
		})
		.await;

		for token in [None, Some("a-guess")] {
			let response = app.register_json(token, partner()).await;
			assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{token:?}");
			assert_eq!(
				response.headers()[header::WWW_AUTHENTICATE],
				r#"Bearer error="invalid_token""#
			);
		}

		let mut bound = partner();
		bound["dpop_bound_access_tokens"] = json!(true);
		let response = app.register_json(Some(INITIAL), bound).await;
		assert_eq!(response.status(), StatusCode::CREATED);
		let registered: ClientRegistration = body(response).await;
		assert!(registered.dpop_bound_access_tokens);

		// Turning DPoP binding off is a change like any other.
		let mut change = partner();
		change["client_id"] = json!(registered.client_id);
		let response = app
			.registration(
				"PUT",
				&path_of(&registered),
				registered.registration_access_token.as_deref(),
				Some(change),
			)
			.await;
		assert_eq!(response.status(), StatusCode::OK);
		assert!(
			!body::<ClientRegistration>(response)
				.await
				.dpop_bound_access_tokens
		);
	}
}

#[tokio::test]
async fn registration_is_off_by_default() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let closed = app(db).await;
		assert!(
			closed
				.idp
				.discovery()
				.get("registration_endpoint")
				.is_none()
		);
		let response = closed.register_json(None, partner()).await;
		assert_ne!(response.status(), StatusCode::CREATED);
		assert!(matches!(
			closed
				.idp
				.register_dynamically(
					None,
					ClientMetadata::default(),
					&leptoskit_auth::RequestInfo::default()
				)
				.await,
			Err(leptoskit_idp::IdpError::NotFound)
		));
		assert!(matches!(
			closed.idp.read_registration("anyone", Some("token")).await,
			Err(leptoskit_idp::IdpError::NotFound)
		));
	}
}

#[tokio::test]
async fn open_registration_is_limited() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let open = app_with(db, |idp| idp.dynamic_registration(Registration::default())).await;

		for _ in 0..10 {
			let response = open.register_json(None, partner()).await;
			assert_eq!(response.status(), StatusCode::CREATED);
		}

		let response = open.register_json(None, partner()).await;
		assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
		assert!(response.headers().contains_key(header::RETRY_AFTER));
	}
}
