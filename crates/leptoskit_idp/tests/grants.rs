//! The ways in beyond a browser's code flow, end to end on SQLite and
//! Postgres: the device flow for CLIs, apps acting as themselves, pushed
//! authorization requests, `private_key_jwt`, RS256 ID tokens, the `claims`
//! parameter, form posts, the audit trail, and the developer portal.
//! Without Postgres binaries the Postgres half is skipped, unless
//! `LEPTOSKIT_REQUIRE_POSTGRES` is set.

#![cfg(all(feature = "axum", feature = "postgres", feature = "sqlite"))]

mod support;

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use leptoskit_auth::EventKind;
use leptoskit_db::Db;
use leptoskit_idp::AppChanges;
use leptoskit_idp::ClientAuth;
use leptoskit_idp::ClientCommand;
use leptoskit_idp::ClientCommandOutput;
use leptoskit_idp::ClientInfo;
use leptoskit_idp::ConnectedApp;
use leptoskit_idp::DeviceAuthorization;
use leptoskit_idp::DeviceRequest;
use leptoskit_idp::GrantType;
use leptoskit_idp::Introspection;
use leptoskit_idp::NewApp;
use leptoskit_idp::NewClientSpec;
use leptoskit_idp::Portal;
use leptoskit_idp::PushedAuthorization;
use leptoskit_idp::RegisteredApp;
use leptoskit_idp::SigningAlgorithm;
use leptoskit_idp::TokenResponse;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use support::*;

const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

impl App {
	async fn start_device(&self, client_id: &str, scope: &str) -> DeviceAuthorization {
		let response = self
			.post_form(
				"/oauth/device_authorization",
				&[("client_id", client_id), ("scope", scope)],
				None,
			)
			.await;
		assert_eq!(response.status(), StatusCode::OK);
		body(response).await
	}

	async fn poll(&self, client_id: &str, device_code: &str) -> axum::response::Response {
		self.post_form(
			"/oauth/token",
			&[
				("grant_type", DEVICE_GRANT),
				("device_code", device_code),
				("client_id", client_id),
			],
			None,
		)
		.await
	}

	/// Makes the device's last poll long enough ago that it may poll again.
	async fn wait_out_the_interval(&self) {
		self.set_time(
			"device_codes",
			"polled_at",
			leptoskit_db::now() - chrono::TimeDelta::minutes(5),
			"polled_at IS NOT NULL",
		)
		.await;
	}

	async fn machine_token(
		&self,
		client_id: &str,
		secret: &str,
		scope: Option<&str>,
	) -> axum::response::Response {
		let mut pairs = vec![("grant_type", "client_credentials")];

		if let Some(scope) = scope {
			pairs.push(("scope", scope));
		}

		self.post_form("/oauth/token", &pairs, Some(&basic(client_id, secret)))
			.await
	}
}

fn count(events: &[(EventKind, Option<String>)], kind: EventKind) -> usize {
	events.iter().filter(|(found, _)| *found == kind).count()
}

#[tokio::test]
#[expect(
	clippy::too_many_lines,
	reason = "a CLI's whole sign-in reads best top to bottom"
)]
async fn a_cli_signs_in_with_a_code_entered_on_another_device() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(db).await;
		let (client_id, _) = app
			.register_spec(NewClientSpec {
				name: "Notes CLI".into(),
				redirect_uris: Vec::new(),
				grant_types: vec![GrantType::DeviceCode],
				..spec(ClientAuth::None, &["openid", "notes:read"])
			})
			.await;

		// The CLI asks for a code, and shows it.
		let device = app.start_device(&client_id, "openid notes:read").await;
		assert_eq!(device.verification_uri, format!("{ORIGIN}/device"));
		assert_eq!(
			param(&device.verification_uri_complete, "user_code").as_deref(),
			Some(device.user_code.as_str())
		);
		assert_eq!(device.interval, 5);
		assert_eq!(device.expires_in, 600);
		assert!(!format!("{device:?}").contains(&device.device_code));

		// Nobody has entered it yet, and polling too soon slows it down.
		let pending = app.poll(&client_id, &device.device_code).await;
		assert_eq!(
			App::error(pending, StatusCode::BAD_REQUEST).await,
			"authorization_pending"
		);
		let early = app.poll(&client_id, &device.device_code).await;
		assert_eq!(
			App::error(early, StatusCode::BAD_REQUEST).await,
			"slow_down"
		);

		// Signed out, the device page can't look the code up.
		let path = format!("/oauth/device?user_code={}", device.user_code);
		assert_eq!(
			app.get(&path, None).await.status(),
			StatusCode::UNAUTHORIZED
		);

		// Signed in on a phone, Ada types it in lower case, without the dash.
		let cookie = app.sign_in("ada@example.com").await;
		let typed = device.user_code.to_lowercase().replace('-', "");
		let request: DeviceRequest = body(
			app.get(&format!("/oauth/device?user_code={typed}"), Some(&cookie))
				.await,
		)
		.await;
		assert_eq!(request.app.name, "Notes CLI");
		assert_eq!(request.user_code, device.user_code);
		assert_eq!(request.scopes[1].name, "notes:read");
		assert_eq!(request.scopes[1].description, "Read your notes.");

		let approved = app
			.post_json(
				"/oauth/device",
				json!({ "user_code": typed, "approve": true }),
				Some(&cookie),
			)
			.await;
		assert_eq!(approved.status(), StatusCode::NO_CONTENT);
		// It's answered: the code no longer finds a waiting device.
		let again = app
			.post_json(
				"/oauth/device",
				json!({ "user_code": typed, "approve": false }),
				Some(&cookie),
			)
			.await;
		assert_eq!(again.status(), StatusCode::NOT_FOUND);

		// The CLI's next poll gets tokens for Ada.
		app.wait_out_the_interval().await;
		let tokens: TokenResponse = body(app.poll(&client_id, &device.device_code).await).await;
		assert!(tokens.id_token.is_some());
		assert_eq!(tokens.scope, "openid notes:read");
		let notes = app
			.with_bearer("POST", "/api/list_notes", &tokens.access_token)
			.await;
		assert_eq!(notes.status(), StatusCode::OK);
		assert!(text(notes).await.ends_with("sees 1"));

		// Ada sees the CLI among her connected apps, and the audit trail says
		// she connected it.
		let apps: Vec<ConnectedApp> = body(app.get("/oauth/grants", Some(&cookie)).await).await;
		assert_eq!(apps.len(), 1);
		assert_eq!(apps[0].app.name, "Notes CLI");
		assert!(
			events(&app)
				.await
				.contains(&(EventKind::AppConnected, Some("Notes CLI".into())))
		);

		// Its refresh token rotates.
		let refreshed = app
			.post_form(
				"/oauth/token",
				&[
					("grant_type", "refresh_token"),
					("refresh_token", tokens.refresh_token.as_deref().unwrap()),
					("client_id", &client_id),
				],
				None,
			)
			.await;
		assert_eq!(refreshed.status(), StatusCode::OK);

		// The device code works once: seen again, it was copied, and the
		// sign-in it started ends.
		app.wait_out_the_interval().await;
		let replayed = app.poll(&client_id, &device.device_code).await;
		assert_eq!(
			App::error(replayed, StatusCode::BAD_REQUEST).await,
			"invalid_grant"
		);
		let refused = app
			.with_bearer("POST", "/api/list_notes", &tokens.access_token)
			.await;
		assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
		assert!(
			events(&app)
				.await
				.contains(&(EventKind::AppTokenReused, Some("Notes CLI".into())))
		);

		// Someone who says no: the CLI hears `access_denied`.
		let denied = app.start_device(&client_id, "notes:read").await;
		let answered = app
			.post_json(
				"/oauth/device",
				json!({ "user_code": denied.user_code, "approve": false }),
				Some(&cookie),
			)
			.await;
		assert_eq!(answered.status(), StatusCode::NO_CONTENT);
		let polled = app.poll(&client_id, &denied.device_code).await;
		assert_eq!(
			App::error(polled, StatusCode::BAD_REQUEST).await,
			"access_denied"
		);

		// A code nobody enters in time expires.
		let late = app.start_device(&client_id, "notes:read").await;
		app.set_time(
			"device_codes",
			"expires_at",
			leptoskit_db::now() - chrono::TimeDelta::seconds(1),
			"used_at IS NULL AND approved IS NULL",
		)
		.await;
		let expired = app.poll(&client_id, &late.device_code).await;
		assert_eq!(
			App::error(expired, StatusCode::BAD_REQUEST).await,
			"expired_token"
		);
		let typed_late = app
			.get(
				&format!("/oauth/device?user_code={}", late.user_code),
				Some(&cookie),
			)
			.await;
		assert_eq!(typed_late.status(), StatusCode::NOT_FOUND);

		// Unknown and malformed codes find nothing.
		for code in ["BCDF-GHJK", "nope", ""] {
			let path = format!("/oauth/device?user_code={code}");
			assert_eq!(
				app.get(&path, Some(&cookie)).await.status(),
				StatusCode::NOT_FOUND,
				"{code}"
			);
		}

		assert_eq!(
			app.get("/oauth/device", Some(&cookie)).await.status(),
			StatusCode::BAD_REQUEST
		);
		let unknown_device = app.poll(&client_id, "not-a-device-code").await;
		assert_eq!(
			App::error(unknown_device, StatusCode::BAD_REQUEST).await,
			"invalid_grant"
		);
		let missing = app
			.post_form(
				"/oauth/token",
				&[("grant_type", DEVICE_GRANT), ("client_id", &client_id)],
				None,
			)
			.await;
		assert_eq!(
			App::error(missing, StatusCode::BAD_REQUEST).await,
			"invalid_request"
		);
	}
}

#[tokio::test]
async fn the_device_flow_is_only_for_apps_registered_for_it() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(db).await;
		let (browser_app, _) = app.register(ClientAuth::None, &["openid"]).await;
		let refused = app
			.post_form(
				"/oauth/device_authorization",
				&[("client_id", &browser_app), ("scope", "openid")],
				None,
			)
			.await;
		assert_eq!(
			App::error(refused, StatusCode::BAD_REQUEST).await,
			"unauthorized_client"
		);
		let polled = app.poll(&browser_app, "anything").await;
		assert_eq!(
			App::error(polled, StatusCode::BAD_REQUEST).await,
			"unauthorized_client"
		);

		let (cli, _) = app
			.register_spec(NewClientSpec {
				redirect_uris: Vec::new(),
				grant_types: vec![GrantType::DeviceCode],
				..spec(ClientAuth::None, &["notes:read"])
			})
			.await;

		for scope in ["", "notes:write", "nonsense"] {
			let response = app
				.post_form(
					"/oauth/device_authorization",
					&[("client_id", &cli), ("scope", scope)],
					None,
				)
				.await;
			assert_eq!(
				App::error(response, StatusCode::BAD_REQUEST).await,
				"invalid_scope",
				"{scope}"
			);
		}

		// A device app has no redirect URI, so it can't start the code flow.
		let url = App::authorize_url(&cli, "notes:read", &[]);
		assert_eq!(app.get(&url, None).await.status(), StatusCode::BAD_REQUEST);
		let malformed = app
			.send(
				Request::post("/oauth/device_authorization")
					.header(header::CONTENT_TYPE, "application/json")
					.body(Body::from("{}"))
					.unwrap(),
			)
			.await;
		assert_eq!(
			App::error(malformed, StatusCode::BAD_REQUEST).await,
			"invalid_request"
		);

		let unauthenticated = app
			.post_form("/oauth/device_authorization", &[("scope", "openid")], None)
			.await;
		assert_eq!(
			App::error(unauthenticated, StatusCode::UNAUTHORIZED).await,
			"invalid_client"
		);
	}
}

#[tokio::test]
async fn a_server_acts_as_itself_with_client_credentials() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(db).await;
		let (worker, secret) = app
			.register_spec(NewClientSpec {
				name: "Nightly export".into(),
				redirect_uris: Vec::new(),
				grant_types: vec![GrantType::ClientCredentials],
				..spec(ClientAuth::SecretBasic, &["openid", "notes:read"])
			})
			.await;
		let secret = secret.unwrap();

		// It gets an access token with its own API scopes, and nothing else.
		let tokens: TokenResponse = body(app.machine_token(&worker, &secret, None).await).await;
		assert_eq!(tokens.scope, "notes:read");
		assert!(tokens.refresh_token.is_none());
		assert!(tokens.id_token.is_none());
		assert_eq!(claims(&tokens.access_token)["sub"], worker.as_str());

		// It calls what takes no person, as itself...
		let counted = app
			.with_bearer("POST", "/api/count_notes", &tokens.access_token)
			.await;
		assert_eq!(counted.status(), StatusCode::OK);
		assert_eq!(text(counted).await, format!("{worker} as itself"));
		// ...but not what needs someone signed in, nor `userinfo`.
		let listed = app
			.with_bearer("POST", "/api/list_notes", &tokens.access_token)
			.await;
		assert_eq!(listed.status(), StatusCode::UNAUTHORIZED);
		let userinfo = app
			.with_bearer("GET", "/oauth/userinfo", &tokens.access_token)
			.await;
		assert_eq!(userinfo.status(), StatusCode::FORBIDDEN);

		let seen = app.introspect(&worker, &secret, &tokens.access_token).await;
		assert!(seen.active);
		assert_eq!(seen.sub.as_deref(), Some(worker.as_str()));

		// Scopes about people, and scopes it wasn't given, are refused.
		for scope in ["openid", "notes:write", " "] {
			let response = app.machine_token(&worker, &secret, Some(scope)).await;
			assert_eq!(
				App::error(response, StatusCode::BAD_REQUEST).await,
				"invalid_scope",
				"{scope}"
			);
		}

		let narrowed: TokenResponse = body(
			app.machine_token(&worker, &secret, Some("notes:read"))
				.await,
		)
		.await;

		// Revoking a token ends it at once.
		let revoked = app
			.post_form(
				"/oauth/revoke",
				&[("token", &tokens.access_token)],
				Some(&basic(&worker, &secret)),
			)
			.await;
		assert_eq!(revoked.status(), StatusCode::OK);
		let after = app
			.with_bearer("POST", "/api/count_notes", &tokens.access_token)
			.await;
		assert_eq!(after.status(), StatusCode::UNAUTHORIZED);

		// Replacing the secret ends the rest, and the old secret stops working.
		let rotated = app
			.idp
			.run_command(&ClientCommand::RotateSecret {
				client_id: worker.clone(),
			})
			.await;
		let ClientCommandOutput::Rotated { secret: new_secret } = rotated else {
			panic!("the secret should rotate: {rotated:?}");
		};
		let ended = app
			.with_bearer("POST", "/api/count_notes", &narrowed.access_token)
			.await;
		assert_eq!(ended.status(), StatusCode::UNAUTHORIZED);
		let old = app.machine_token(&worker, &secret, None).await;
		assert_eq!(
			App::error(old, StatusCode::UNAUTHORIZED).await,
			"invalid_client"
		);
		let fresh = app.machine_token(&worker, &new_secret, None).await;

		assert_eq!(fresh.status(), StatusCode::OK);

		// A public app can't act as itself, and an app not registered for it
		// isn't let.
		let output = app
			.idp
			.run_command(&ClientCommand::Add(NewClientSpec {
				redirect_uris: Vec::new(),
				grant_types: vec![GrantType::ClientCredentials],
				..spec(ClientAuth::None, &["notes:read"])
			}))
			.await;
		assert!(
			matches!(&output, ClientCommandOutput::Failed { message } if message.contains("acting as itself")),
			"{output:?}"
		);
		let (partner, partner_secret) =
			app.register(ClientAuth::SecretBasic, &["notes:read"]).await;
		let refused = app
			.machine_token(&partner, &partner_secret.unwrap(), None)
			.await;
		assert_eq!(
			App::error(refused, StatusCode::BAD_REQUEST).await,
			"unauthorized_client"
		);
		let (public, _) = app.register(ClientAuth::None, &["openid"]).await;
		let output = app
			.idp
			.run_command(&ClientCommand::RotateSecret { client_id: public })
			.await;

		assert!(matches!(output, ClientCommandOutput::Failed { .. }));
		let output = app
			.idp
			.run_command(&ClientCommand::RotateSecret {
				client_id: "nobody".into(),
			})
			.await;
		assert!(matches!(output, ClientCommandOutput::Failed { .. }));
		let unknown = app
			.post_form(
				"/oauth/token",
				&[("grant_type", "password")],
				Some(&basic(&worker, &new_secret)),
			)
			.await;
		assert_eq!(
			App::error(unknown, StatusCode::BAD_REQUEST).await,
			"unsupported_grant_type"
		);
	}
}

#[tokio::test]
#[expect(
	clippy::too_many_lines,
	reason = "pushing a request and each way it's refused read best together"
)]
async fn an_app_pushes_its_request_before_sending_the_browser() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(db).await;
		let (client_id, secret) = app
			.register(ClientAuth::SecretBasic, &["openid", "notes:read"])
			.await;
		let secret = secret.unwrap();
		let challenge = challenge(VERIFIER);
		let request = |scope: &'static str| {
			vec![
				("response_type", "code"),
				("redirect_uri", CALLBACK),
				("scope", scope),
				("state", "pushed-state"),
				("code_challenge", challenge.clone().leak()),
				("code_challenge_method", "S256"),
			]
		};

		// The partner's server pushes the request, authenticated.
		let pushed = app
			.post_form(
				"/oauth/par",
				&request("openid notes:read"),
				Some(&basic(&client_id, &secret)),
			)
			.await;
		assert_eq!(pushed.status(), StatusCode::CREATED);
		let pushed: PushedAuthorization = body(pushed).await;
		assert!(
			pushed
				.request_uri
				.starts_with("urn:ietf:params:oauth:request_uri:")
		);
		assert_eq!(pushed.expires_in, 600);

		// The browser carries only the handle. Signed out, it goes to sign in
		// and comes back with the handle alone.
		let query = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs([
				("client_id", client_id.as_str()),
				("request_uri", pushed.request_uri.as_str()),
			])
			.finish();
		let url = format!("/oauth/authorize?{query}");
		let to = location(&app.get(&url, None).await);
		let return_to = param(&to, "return_to").unwrap();
		assert_eq!(param(&return_to, "scope"), None);
		assert_eq!(
			param(&return_to, "request_uri").as_deref(),
			Some(pushed.request_uri.as_str())
		);

		// Signed in, it reaches consent, and the code it brings back trades.
		let cookie = app.sign_in("ada@example.com").await;
		let to = location(&app.get(&return_to, Some(&cookie)).await);
		assert!(to.starts_with("/consent?"), "{to}");
		let decided = app
			.post_json(
				"/oauth/consent",
				json!({ "request": param(&to, "request").unwrap(), "approve": true }),
				Some(&cookie),
			)
			.await;
		let to = body::<leptoskit_idp::ConsentRedirect>(decided)
			.await
			.redirect_to;
		assert_eq!(param(&to, "state").as_deref(), Some("pushed-state"));
		let code = param(&to, "code").unwrap();
		let exchanged = app
			.exchange(&client_id, Some(&secret), &code, VERIFIER)
			.await;

		assert_eq!(exchanged.status(), StatusCode::OK);

		// The handle worked once.
		let reused = app.get(&url, Some(&cookie)).await;
		assert_eq!(reused.status(), StatusCode::BAD_REQUEST);
		assert!(text(reused).await.contains("request_uri"));

		// A pushed `prompt=login` asks once, then comes back satisfied.
		let mut fresh = request("openid");
		fresh.push(("prompt", "login"));
		let pushed: PushedAuthorization = body(
			app.post_form("/oauth/par", &fresh, Some(&basic(&client_id, &secret)))
				.await,
		)
		.await;
		let query = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs([
				("client_id", client_id.as_str()),
				("request_uri", pushed.request_uri.as_str()),
			])
			.finish();
		let to = location(
			&app.get(&format!("/oauth/authorize?{query}"), Some(&cookie))
				.await,
		);
		let return_to = param(&to, "return_to").unwrap();
		let back = location(&app.get(&return_to, Some(&cookie)).await);
		assert!(back.starts_with(CALLBACK), "{back}");

		// What `/oauth/authorize` would refuse is refused at once.
		for (pairs, error) in [
			(request("notes:write"), "invalid_scope"),
			(
				vec![("response_type", "token"), ("redirect_uri", CALLBACK)],
				"unsupported_response_type",
			),
			(
				vec![("redirect_uri", "https://evil.example/cb")],
				"invalid_request",
			),
			(
				{
					let mut nested = request("openid");
					nested.push(("request_uri", "urn:ietf:params:oauth:request_uri:x"));
					nested
				},
				"invalid_request",
			),
		] {
			let response = app
				.post_form("/oauth/par", &pairs, Some(&basic(&client_id, &secret)))
				.await;
			assert_eq!(
				App::error(response, StatusCode::BAD_REQUEST).await,
				error,
				"{pairs:?}"
			);
		}
		let unauthenticated = app.post_form("/oauth/par", &request("openid"), None).await;
		assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
		// Naming another app than the one that authenticated is refused.
		let mut other = request("openid");
		other.push(("client_id", "someone-else"));
		let response = app
			.post_form("/oauth/par", &other, Some(&basic(&client_id, &secret)))
			.await;
		assert_eq!(
			App::error(response, StatusCode::UNAUTHORIZED).await,
			"invalid_client"
		);

		// A handle is the app's own: another app can't use it, and a made-up
		// one finds nothing.
		let (other, _) = app.register(ClientAuth::None, &["openid"]).await;
		let pushed: PushedAuthorization = body(
			app.post_form(
				"/oauth/par",
				&request("openid"),
				Some(&basic(&client_id, &secret)),
			)
			.await,
		)
		.await;
		for (client, uri) in [
			(other.as_str(), pushed.request_uri.as_str()),
			(
				client_id.as_str(),
				"urn:ietf:params:oauth:request_uri:made-up",
			),
		] {
			let query = url::form_urlencoded::Serializer::new(String::new())
				.extend_pairs([("client_id", client), ("request_uri", uri)])
				.finish();
			let response = app
				.get(&format!("/oauth/authorize?{query}"), Some(&cookie))
				.await;
			assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{client}");
		}

		// A public app pushes too, with PKCE as its protection.
		let pushed = app
			.post_form(
				"/oauth/par",
				&[
					("client_id", other.as_str()),
					("response_type", "code"),
					("redirect_uri", CALLBACK),
					("scope", "openid"),
					("code_challenge", challenge.as_str()),
					("code_challenge_method", "S256"),
				],
				None,
			)
			.await;
		assert_eq!(pushed.status(), StatusCode::CREATED);

		// A request_uri that isn't from `/oauth/par` is refused, back at the
		// app.
		let url = App::authorize_url(
			&client_id,
			"openid",
			&[("request_uri", "https://x.example/r")],
		);
		let to = location(&app.get(&url, Some(&cookie)).await);
		assert_eq!(
			param(&to, "error").as_deref(),
			Some("request_uri_not_supported")
		);
	}
}

#[tokio::test]
#[expect(
	clippy::too_many_lines,
	reason = "each way an assertion is refused reads best next to the one that works"
)]
async fn an_app_authenticates_with_its_own_key() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(db).await;
		let key = p256::ecdsa::SigningKey::from_slice(&[7; 32]).unwrap();
		let point = key.verifying_key().to_sec1_point(false);
		let coordinate = |range: std::ops::Range<usize>| {
			use base64::Engine;
			base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&point.as_bytes()[range])
		};
		let jwks = json!({ "keys": [
			{ "kty": "EC", "crv": "P-256", "kid": "partner-1", "x": coordinate(1..33), "y": coordinate(33..65) },
			rsa_jwk(),
		] });
		let (client_id, secret) = app
			.register_spec(NewClientSpec {
				jwks: Some(jwks.clone()),
				..spec(ClientAuth::PrivateKeyJwt, &["openid", "notes:read"])
			})
			.await;
		assert_eq!(secret, None, "a key-holding app has no secret");

		let now = leptoskit_db::now().timestamp();
		let assertion = |jti: &str, aud: Value, exp: i64| {
			json!({
				"iss": client_id,
				"sub": client_id,
				"aud": aud,
				"jti": jti,
				"exp": exp,
				"iat": now,
			})
		};
		let token_endpoint = json!(format!("{ORIGIN}/oauth/token"));
		let cookie = app.sign_in("ada@example.com").await;
		let exchange = |assertion: String, code: String| {
			let app = &app;
			async move {
				app.post_form(
					"/oauth/token",
					&[
						("grant_type", "authorization_code"),
						("code", &code),
						("redirect_uri", CALLBACK),
						("code_verifier", VERIFIER),
						(
							"client_assertion_type",
							"urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
						),
						("client_assertion", &assertion),
					],
					None,
				)
				.await
			}
		};

		// Signed with its key, for this provider, once: it gets tokens.
		let signed = es256_jwt(
			&key,
			Some("partner-1"),
			&assertion("one", token_endpoint.clone(), now + 60),
		);
		let code = app.code(&client_id, "openid notes:read", &cookie).await;
		let tokens = exchange(signed.clone(), code).await;
		assert_eq!(tokens.status(), StatusCode::OK);

		// The same assertion again is refused.
		let code = app.code(&client_id, "openid", &cookie).await;
		let replayed = exchange(signed, code.clone()).await;
		assert_eq!(
			App::error(replayed, StatusCode::UNAUTHORIZED).await,
			"invalid_client"
		);

		// So is one meant for someone else, expired, too long-lived, from a
		// key it didn't register, or about another app.
		let stranger = p256::ecdsa::SigningKey::from_slice(&[8; 32]).unwrap();
		for refused in [
			es256_jwt(
				&key,
				None,
				&assertion("two", json!("https://other.example"), now + 60),
			),
			es256_jwt(
				&key,
				None,
				&assertion("three", token_endpoint.clone(), now - 120),
			),
			es256_jwt(
				&key,
				None,
				&assertion("four", token_endpoint.clone(), now + 7200),
			),
			es256_jwt(
				&stranger,
				None,
				&assertion("five", token_endpoint.clone(), now + 60),
			),
			es256_jwt(
				&key,
				Some("other-kid"),
				&assertion("six", token_endpoint.clone(), now + 60),
			),
			es256_jwt(
				&key,
				None,
				&json!({ "iss": client_id, "sub": "someone", "aud": ORIGIN, "jti": "seven", "exp": now + 60 }),
			),
			es256_jwt(
				&key,
				None,
				&json!({ "iss": client_id, "sub": client_id, "aud": ORIGIN, "exp": now + 60 }),
			),
			"not.a.jwt".to_owned(),
		] {
			let response = exchange(refused, code.clone()).await;
			assert_eq!(
				App::error(response, StatusCode::UNAUTHORIZED).await,
				"invalid_client"
			);
		}

		// Its RSA key works too, with the issuer as audience and no key ID.
		let rsa = rs256_jwt(&assertion("eight", json!([ORIGIN]), now + 60));
		let exchanged = exchange(rsa, code).await;
		assert_eq!(exchanged.status(), StatusCode::OK);

		// An assertion and a secret together are refused, as is the wrong
		// assertion type.
		let both = app
			.post_form(
				"/oauth/token",
				&[
					("grant_type", "refresh_token"),
					("refresh_token", "x"),
					("client_secret", "x"),
					("client_id", &client_id),
					(
						"client_assertion_type",
						"urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
					),
					("client_assertion", "x.y.z"),
				],
				None,
			)
			.await;
		assert_eq!(
			App::error(both, StatusCode::BAD_REQUEST).await,
			"invalid_request"
		);
		let wrong_type = app
			.post_form(
				"/oauth/token",
				&[
					("grant_type", "refresh_token"),
					("client_assertion_type", "saml"),
					("client_assertion", "x.y.z"),
				],
				None,
			)
			.await;
		assert_eq!(wrong_type.status(), StatusCode::UNAUTHORIZED);

		// Introspection takes the assertion too.
		let tokens: TokenResponse = body(tokens).await;
		let seen = app
			.post_form(
				"/oauth/introspect",
				&[
					("token", &tokens.access_token),
					(
						"client_assertion_type",
						"urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
					),
					(
						"client_assertion",
						&es256_jwt(
							&key,
							None,
							&assertion(
								"nine",
								json!(format!("{ORIGIN}/oauth/introspect")),
								now + 60,
							),
						),
					),
				],
				None,
			)
			.await;
		assert!(body::<Introspection>(seen).await.active);

		// Registering keys is checked.
		for (jwks, method) in [
			(None, ClientAuth::PrivateKeyJwt),
			(Some(json!({ "keys": [] })), ClientAuth::PrivateKeyJwt),
			(Some(jwks.clone()), ClientAuth::SecretBasic),
		] {
			let output = app
				.idp
				.run_command(&ClientCommand::Add(NewClientSpec {
					jwks,
					..spec(method, &["openid"])
				}))
				.await;
			assert!(
				matches!(output, ClientCommandOutput::Failed { .. }),
				"{output:?}"
			);
		}
	}
}

#[tokio::test]
async fn apps_that_need_rs256_get_rsa_signed_id_tokens() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		// Without an RSA key, only ES256 is offered.
		let plain = app(Db::connect("sqlite::memory:").await.unwrap()).await;
		let discovery: Value =
			body(plain.get("/.well-known/openid-configuration", None).await).await;
		assert_eq!(
			discovery["id_token_signing_alg_values_supported"],
			json!(["ES256"])
		);
		let output = plain
			.idp
			.run_command(&ClientCommand::Add(NewClientSpec {
				id_token_alg: SigningAlgorithm::Rs256,
				..spec(ClientAuth::SecretBasic, &["openid"])
			}))
			.await;
		assert!(
			matches!(&output, ClientCommandOutput::Failed { message } if message.contains("RSA key")),
			"{output:?}"
		);

		let app = app_with(db, |idp| idp.rsa_key(TEST_RSA_KEY)).await;
		let discovery: Value = body(app.get("/.well-known/openid-configuration", None).await).await;
		assert_eq!(
			discovery["id_token_signing_alg_values_supported"],
			json!(["ES256", "RS256"])
		);
		let (client_id, secret) = app
			.register_spec(NewClientSpec {
				id_token_alg: SigningAlgorithm::Rs256,
				..spec(ClientAuth::SecretBasic, &["openid", "notes:read"])
			})
			.await;
		let secret = secret.unwrap();
		let cookie = app.sign_in("ada@example.com").await;
		let code = app.code(&client_id, "openid notes:read", &cookie).await;
		let tokens: TokenResponse = body(
			app.exchange(&client_id, Some(&secret), &code, VERIFIER)
				.await,
		)
		.await;

		// The ID token is RS256 and verifies against JWKS; the access token
		// stays ES256.
		let id_token = tokens.id_token.unwrap();
		assert_eq!(verify_with_jwks(&app, &id_token).await, "RS256");
		assert_eq!(verify_with_jwks(&app, &tokens.access_token).await, "ES256");

		// An RS256 ID token works as a sign-out hint.
		let query = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs([("id_token_hint", id_token.as_str())])
			.finish();
		let out = app
			.get(&format!("/oauth/end_session?{query}"), Some(&cookie))
			.await;
		assert_eq!(out.status(), StatusCode::SEE_OTHER);
		assert!(out.headers().contains_key(header::SET_COOKIE));

		// The RSA key stays published, its row renewed once half its term is
		// used.
		app.set_time(
			"signing_keys",
			"retires_at",
			leptoskit_db::now() + chrono::TimeDelta::days(1),
			"algorithm = 'RS256'",
		)
		.await;
		let cookie = app.sign_in("ada@example.com").await;
		let code = app.code(&client_id, "openid", &cookie).await;
		let tokens: TokenResponse = body(
			app.exchange(&client_id, Some(&secret), &code, VERIFIER)
				.await,
		)
		.await;
		assert_eq!(
			verify_with_jwks(&app, &tokens.id_token.unwrap()).await,
			"RS256"
		);
		let renewed = leptoskit_idp::SigningKey::query()
			.filter(|key| key.algorithm.eq(SigningAlgorithm::Rs256))
			.first(&app.db.system("test"))
			.await
			.unwrap()
			.unwrap();
		assert!(renewed.retires_at > leptoskit_db::now() + chrono::TimeDelta::days(20));
	}

	// A key that isn't one is refused when the provider is built.
	let db = leptoskit_db::testing::sqlite().await;
	let auth = leptoskit_auth::Auth::builder(&db, leptoskit_auth::Keys::single("k", [1; 32]))
		.origin(ORIGIN)
		.mailer(leptoskit_auth::Mailbox::new())
		.build()
		.unwrap();
	let built = leptoskit_idp::Idp::builder(&db, &auth)
		.rsa_key("-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----")
		.build();
	assert!(built.is_err());
}

#[tokio::test]
async fn the_claims_parameter_puts_released_claims_in_the_id_token() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(db).await;
		let (client_id, secret) = app
			.register(ClientAuth::SecretBasic, &["openid", "profile", "email"])
			.await;
		let secret = secret.unwrap();
		let cookie = app.sign_in("ada@example.com").await;
		let ask = |scope: &str, claims: &str| {
			App::authorize_url(&client_id, scope, &[("claims", claims)])
		};
		let code_for = |url: String| {
			let app = &app;
			let cookie = cookie.clone();
			async move {
				let mut to = location(&app.get(&url, Some(&cookie)).await);

				if to.starts_with("/consent?") {
					let decided = app
						.post_json(
							"/oauth/consent",
							json!({ "request": param(&to, "request").unwrap(), "approve": true }),
							Some(&cookie),
						)
						.await;

					to = body::<leptoskit_idp::ConsentRedirect>(decided)
						.await
						.redirect_to;
				}

				to
			}
		};

		let wanted =
			r#"{"id_token":{"email":{"essential":true},"name":null},"userinfo":{"picture":null}}"#;
		let to = code_for(ask("openid profile email", wanted)).await;
		let code = param(&to, "code").unwrap();
		let tokens: TokenResponse = body(
			app.exchange(&client_id, Some(&secret), &code, VERIFIER)
				.await,
		)
		.await;
		let id_token = claims(tokens.id_token.as_deref().unwrap());
		assert_eq!(id_token["email"], "ada@example.com");
		assert_eq!(id_token["email_verified"], true);
		assert_eq!(id_token["name"], "ada");

		// Refreshed ID tokens carry them too.
		let refreshed: TokenResponse = body(
			app.refresh(
				&client_id,
				&secret,
				tokens.refresh_token.as_deref().unwrap(),
			)
			.await,
		)
		.await;
		assert_eq!(
			claims(refreshed.id_token.as_deref().unwrap())["name"],
			"ada"
		);

		// Claims the scopes don't release stay out.
		let to = code_for(ask("openid", wanted)).await;
		let code = param(&to, "code").unwrap();
		let tokens: TokenResponse = body(
			app.exchange(&client_id, Some(&secret), &code, VERIFIER)
				.await,
		)
		.await;
		let id_token = claims(tokens.id_token.as_deref().unwrap());
		assert_eq!(id_token.get("email"), None);
		assert_eq!(id_token.get("name"), None);

		// Asking for someone else by `sub` finds them not signed in.
		let to = code_for(ask(
			"openid",
			r#"{"id_token":{"sub":{"value":"someone-else"}}}"#,
		))
		.await;
		assert_eq!(param(&to, "error").as_deref(), Some("login_required"));

		// A malformed parameter goes back to the app.
		let to = code_for(ask("openid", "[1, 2]")).await;
		assert_eq!(param(&to, "error").as_deref(), Some("invalid_request"));

		let discovery: Value = body(app.get("/.well-known/openid-configuration", None).await).await;
		assert_eq!(discovery["claims_parameter_supported"], true);
	}
}

#[tokio::test]
async fn apps_may_post_forms_to_authorize_and_sign_out() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(db).await;
		let (client_id, _) = app.register(ClientAuth::None, &["openid"]).await;
		let challenge = challenge(VERIFIER);

		// A form post from the partner's page is sent on as a `GET`, which
		// carries the session cookie.
		let posted = app
			.post_form(
				"/oauth/authorize",
				&[
					("response_type", "code"),
					("client_id", &client_id),
					("redirect_uri", CALLBACK),
					("scope", "openid"),
					("state", "posted"),
					("code_challenge", &challenge),
					("code_challenge_method", "S256"),
				],
				None,
			)
			.await;
		let to = location(&posted);
		assert!(to.starts_with("/oauth/authorize?"), "{to}");
		assert_eq!(param(&to, "state").as_deref(), Some("posted"));
		assert_eq!(param(&to, "client_id").as_deref(), Some(client_id.as_str()));
		let cookie = app.sign_in("ada@example.com").await;
		let next = location(&app.get(&to, Some(&cookie)).await);
		assert!(next.starts_with("/consent?"), "{next}");

		let posted = app
			.post_form(
				"/oauth/end_session",
				&[("client_id", &client_id), ("state", "bye")],
				None,
			)
			.await;
		let to = location(&posted);
		assert!(to.starts_with("/oauth/end_session?"), "{to}");
		assert_eq!(location(&app.get(&to, Some(&cookie)).await), "/");

		// A repeated parameter, or a body too big to read, is refused.
		let repeated = app
			.post_form(
				"/oauth/authorize",
				&[("client_id", "a"), ("client_id", "b")],
				None,
			)
			.await;
		assert_eq!(repeated.status(), StatusCode::BAD_REQUEST);
		let huge = app
			.send(
				Request::post("/oauth/authorize")
					.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
					.body(Body::from(format!("state={}", "a".repeat(70 * 1024))))
					.unwrap(),
			)
			.await;
		assert_eq!(huge.status(), StatusCode::BAD_REQUEST);
	}
}

#[tokio::test]
async fn the_audit_trail_records_connecting_disconnecting_and_reuse() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(db).await;
		let (client_id, secret) = app
			.register(ClientAuth::SecretBasic, &["openid", "notes:read"])
			.await;
		let secret = secret.unwrap();
		let cookie = app.sign_in("ada@example.com").await;

		let code = app.code(&client_id, "openid", &cookie).await;
		let tokens: TokenResponse = body(
			app.exchange(&client_id, Some(&secret), &code, VERIFIER)
				.await,
		)
		.await;
		// Consenting to the same again records nothing new; to more, it does.
		app.code(&client_id, "openid", &cookie).await;
		app.code(&client_id, "openid notes:read", &cookie).await;
		assert_eq!(count(&events(&app).await, EventKind::AppConnected), 2);

		// A refresh token used twice ends the sign-in, once.
		let refresh = tokens.refresh_token.unwrap();
		app.refresh(&client_id, &secret, &refresh).await;
		app.refresh(&client_id, &secret, &refresh).await;
		app.refresh(&client_id, &secret, &refresh).await;
		assert_eq!(count(&events(&app).await, EventKind::AppTokenReused), 1);

		// A code used twice ends what it started.
		let code = app.code(&client_id, "openid", &cookie).await;
		app.exchange(&client_id, Some(&secret), &code, VERIFIER)
			.await;
		app.exchange(&client_id, Some(&secret), &code, VERIFIER)
			.await;
		assert_eq!(count(&events(&app).await, EventKind::AppTokenReused), 2);

		let disconnected = app
			.post_json(
				"/oauth/grants/revoke",
				json!({ "client_id": client_id }),
				Some(&cookie),
			)
			.await;
		assert_eq!(disconnected.status(), StatusCode::NO_CONTENT);
		let trail = events(&app).await;
		assert!(trail.contains(&(EventKind::AppDisconnected, Some("Partner".into()))));

		// Removing an app tells everyone who had it connected.
		app.code(&client_id, "openid", &cookie).await;
		let removed = app
			.idp
			.run_command(&ClientCommand::Remove {
				client_id: client_id.clone(),
			})
			.await;
		assert_eq!(removed, ClientCommandOutput::Removed { removed: true });
		assert_eq!(count(&events(&app).await, EventKind::AppDisconnected), 2);
	}
}

#[tokio::test]
#[expect(
	clippy::too_many_lines,
	reason = "a developer's whole portal session reads best top to bottom"
)]
async fn developers_register_and_manage_their_own_apps() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		// Off by default: the routes aren't there.
		let closed = app(Db::connect("sqlite::memory:").await.unwrap()).await;
		let cookie = closed.sign_in("dev@example.com").await;
		assert_eq!(
			closed.get("/oauth/apps", Some(&cookie)).await.status(),
			StatusCode::NOT_FOUND
		);

		let app = app_with(db, |idp| {
			idp.developer_portal(Portal { apps_per_person: 2 })
		})
		.await;
		let developer = app.sign_in("dev@example.com").await;
		let other = app.sign_in("ada@example.com").await;
		assert_eq!(
			app.get("/oauth/apps", None).await.status(),
			StatusCode::UNAUTHORIZED
		);

		let created = app
			.post_json(
				"/oauth/apps",
				serde_json::to_value(NewApp {
					name: "  Dev's App ".into(),
					redirect_uris: vec![CALLBACK.into()],
					post_logout_redirect_uris: Vec::new(),
					scopes: vec!["openid".into(), "notes:read".into(), "notes:write".into()],
					auth_method: ClientAuth::SecretBasic,
					grant_types: leptoskit_idp::default_grant_types(),
					jwks: None,
				})
				.unwrap(),
				Some(&developer),
			)
			.await;
		assert_eq!(created.status(), StatusCode::CREATED);
		let created: RegisteredApp = body(created).await;
		assert_eq!(created.app.name, "Dev's App");
		assert!(!created.app.first_party && !created.app.introspection);
		let client_id = created.app.client_id.clone();
		let secret = created.secret.unwrap();

		// The developer sees it; nobody else does, and the operator's list
		// keeps to the operator's apps.
		let mine: Vec<ClientInfo> = body(app.get("/oauth/apps", Some(&developer)).await).await;
		assert_eq!(mine.len(), 1);
		let theirs: Vec<ClientInfo> = body(app.get("/oauth/apps", Some(&other)).await).await;
		assert_eq!(theirs, [] as [ClientInfo; 0]);
		assert_eq!(
			app.idp.run_command(&ClientCommand::List).await,
			ClientCommandOutput::Listed {
				clients: Vec::new()
			}
		);

		// Ada connects it with both scopes.
		let code = app
			.code(&client_id, "openid notes:read notes:write", &other)
			.await;
		let tokens: TokenResponse = body(
			app.exchange(&client_id, Some(&secret), &code, VERIFIER)
				.await,
		)
		.await;

		// Taking `notes:write` away narrows Ada's grant and ends the sign-in
		// that carried it.
		let updated = app
			.post_json(
				"/oauth/apps/update",
				serde_json::to_value(AppChanges {
					client_id: client_id.clone(),
					name: "Dev's Notes".into(),
					redirect_uris: vec![CALLBACK.into()],
					post_logout_redirect_uris: vec![SIGNED_OUT.into()],
					scopes: vec!["openid".into(), "notes:read".into()],
					jwks: None,
				})
				.unwrap(),
				Some(&developer),
			)
			.await;
		assert_eq!(updated.status(), StatusCode::OK);
		let updated: ClientInfo = body(updated).await;
		assert_eq!(updated.name, "Dev's Notes");
		let connected: Vec<ConnectedApp> = body(app.get("/oauth/grants", Some(&other)).await).await;
		let names: Vec<&str> = connected[0]
			.scopes
			.iter()
			.map(|scope| scope.name.as_str())
			.collect();
		assert_eq!(names, ["openid", "notes:read"]);
		let ended = app
			.with_bearer("POST", "/api/list_notes", &tokens.access_token)
			.await;

		assert_eq!(ended.status(), StatusCode::UNAUTHORIZED);

		// Someone else can't change, rotate or delete it.
		for path in ["/oauth/apps/rotate-secret", "/oauth/apps/delete"] {
			let response = app
				.post_json(path, json!({ "client_id": client_id }), Some(&other))
				.await;
			assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
		}

		let hijack = app
			.post_json(
				"/oauth/apps/update",
				json!({ "client_id": client_id, "name": "Mine", "scopes": ["openid"] }),
				Some(&other),
			)
			.await;
		assert_eq!(hijack.status(), StatusCode::NOT_FOUND);

		// A new secret replaces the old one.
		let rotated: RegisteredApp = body(
			app.post_json(
				"/oauth/apps/rotate-secret",
				json!({ "client_id": client_id }),
				Some(&developer),
			)
			.await,
		)
		.await;
		let new_secret = rotated.secret.unwrap();
		assert_ne!(new_secret, secret);
		let code = app.code(&client_id, "openid", &other).await;
		let old = app
			.exchange(&client_id, Some(&secret), &code, VERIFIER)
			.await;
		assert_eq!(old.status(), StatusCode::UNAUTHORIZED);

		// The portal's limit holds.
		let public = |name: &str| {
			json!({
				"name": name,
				"redirect_uris": [CALLBACK],
				"scopes": ["openid"],
				"auth_method": "none",
			})
		};
		let second = app
			.post_json("/oauth/apps", public("Second"), Some(&developer))
			.await;
		assert_eq!(second.status(), StatusCode::CREATED);
		let second: RegisteredApp = body(second).await;
		assert_eq!(second.secret, None);
		let third = app
			.post_json("/oauth/apps", public("Third"), Some(&developer))
			.await;
		assert_eq!(
			App::error(third, StatusCode::BAD_REQUEST).await,
			"invalid_client_metadata"
		);
		let no_secret = app
			.post_json(
				"/oauth/apps/rotate-secret",
				json!({ "client_id": second.app.client_id }),
				Some(&developer),
			)
			.await;

		assert_eq!(
			App::error(no_secret, StatusCode::BAD_REQUEST).await,
			"invalid_client_metadata"
		);
		let malformed = app
			.post_json("/oauth/apps", json!({ "name": 1 }), Some(&developer))
			.await;
		assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

		// Deleting it ends Ada's grant, and her audit trail says so.
		let deleted = app
			.post_json(
				"/oauth/apps/delete",
				json!({ "client_id": client_id }),
				Some(&developer),
			)
			.await;
		assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
		let connected: Vec<ConnectedApp> = body(app.get("/oauth/grants", Some(&other)).await).await;
		assert_eq!(connected, [] as [ConnectedApp; 0]);
		let trail = events(&app).await;
		for kind in [
			EventKind::AppRegistered,
			EventKind::AppChanged,
			EventKind::AppDeleted,
			EventKind::AppDisconnected,
		] {
			assert!(count(&trail, kind) > 0, "{kind:?}");
		}
		let again = app
			.post_json(
				"/oauth/apps/delete",
				json!({ "client_id": client_id }),
				Some(&developer),
			)
			.await;
		assert_eq!(again.status(), StatusCode::NOT_FOUND);
	}

	// A portal must allow at least one app.
	let db = leptoskit_db::testing::sqlite().await;
	let auth = leptoskit_auth::Auth::builder(&db, leptoskit_auth::Keys::single("k", [1; 32]))
		.origin(ORIGIN)
		.mailer(leptoskit_auth::Mailbox::new())
		.build()
		.unwrap();
	let built = leptoskit_idp::Idp::builder(&db, &auth)
		.developer_portal(Portal { apps_per_person: 0 })
		.build();
	assert!(built.is_err());
}
