//! Sign in with your app, end to end, against the real routes and layers on
//! SQLite and Postgres: discovery and JWKS, the code flow with PKCE and
//! consent, ID tokens, `userinfo`, scoped API calls, refresh rotation with
//! reuse detection, revocation, introspection, connected apps, and signing
//! out. Without Postgres binaries the Postgres half is skipped, unless
//! `LEPTOSKIT_REQUIRE_POSTGRES` is set.

#![cfg(all(feature = "axum", feature = "postgres", feature = "sqlite"))]
// `#[server]` functions are async by contract, awaited or not.
#![allow(clippy::unused_async)]

mod support;

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_idp::ClientAuth;
use leptoskit_idp::ClientCommand;
use leptoskit_idp::ClientCommandOutput;
use leptoskit_idp::ConnectedApp;
use leptoskit_idp::ConsentRedirect;
use leptoskit_idp::ConsentRequest;
use leptoskit_idp::Grant;
use leptoskit_idp::Idp;
use leptoskit_idp::Introspection;
use leptoskit_idp::NewClientSpec;
use leptoskit_idp::TokenResponse;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use support::*;

#[tokio::test]
#[expect(
	clippy::too_many_lines,
	reason = "one partner's whole journey reads best top to bottom"
)]
async fn a_partner_app_signs_someone_in_and_calls_the_api() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(db).await;
		let (client_id, secret) = app
			.register(
				ClientAuth::SecretBasic,
				&["openid", "profile", "email", "notes:read"],
			)
			.await;
		let secret = secret.unwrap();

		// Discovery names every endpoint under the issuer.
		let discovery: Value = body(app.get("/.well-known/openid-configuration", None).await).await;
		assert_eq!(discovery["issuer"], ORIGIN);
		assert_eq!(discovery["token_endpoint"], format!("{ORIGIN}/oauth/token"));
		assert_eq!(
			discovery["code_challenge_methods_supported"],
			json!(["S256"])
		);
		assert!(
			discovery["scopes_supported"]
				.as_array()
				.unwrap()
				.contains(&json!("notes:read"))
		);

		// Signed out: off to the sign-in page, which comes back here.
		let url = App::authorize_url(&client_id, "openid profile email notes:read", &[]);
		let to = location(&app.get(&url, None).await);
		assert!(to.starts_with("/sign-in?"), "{to}");
		let return_to = param(&to, "return_to").unwrap();
		assert!(return_to.starts_with("/oauth/authorize?"), "{return_to}");
		assert_eq!(
			param(&return_to, "client_id").as_deref(),
			Some(client_id.as_str())
		);

		let cookie = app.sign_in("ada@example.com").await;

		// Signed in: off to the consent page, which reads the request.
		let to = location(&app.get(&return_to, Some(&cookie)).await);
		assert!(to.starts_with("/consent?request="), "{to}");
		let request = param(&to, "request").unwrap();
		let consent: ConsentRequest = body(
			app.get(&format!("/oauth/consent?request={request}"), Some(&cookie))
				.await,
		)
		.await;
		assert_eq!(consent.app.name, "Partner");
		assert_eq!(consent.redirect_host, "partner.example");
		assert_eq!(consent.scopes[3].name, "notes:read");
		assert_eq!(consent.scopes[3].description, "Read your notes.");

		// Someone else's session can't read or answer it.
		let other = app.sign_in("grace@example.com").await;
		let foreign = app
			.get(&format!("/oauth/consent?request={request}"), Some(&other))
			.await;
		assert_eq!(foreign.status(), StatusCode::NOT_FOUND);

		let decided = app
			.post_json(
				"/oauth/consent",
				json!({ "request": request, "approve": true }),
				Some(&cookie),
			)
			.await;
		let to = body::<ConsentRedirect>(decided).await.redirect_to;
		let code = param(&to, "code").unwrap();
		assert_eq!(param(&to, "iss").as_deref(), Some(ORIGIN));

		// An answer only counts once.
		let again = app
			.post_json(
				"/oauth/consent",
				json!({ "request": request, "approve": true }),
				Some(&cookie),
			)
			.await;
		assert_eq!(again.status(), StatusCode::NOT_FOUND);

		// The wrong verifier, the wrong secret, the wrong redirect: refused.
		let wrong = app
			.exchange(
				&client_id,
				Some(&secret),
				&code,
				"a-different-verifier-that-is-also-long-enough-000000",
			)
			.await;
		assert_eq!(
			App::error(wrong, StatusCode::BAD_REQUEST).await,
			"invalid_grant"
		);
		let wrong = app
			.exchange(&client_id, Some("not-the-secret"), &code, VERIFIER)
			.await;
		assert_eq!(
			wrong.headers()[header::WWW_AUTHENTICATE],
			"Basic realm=\"oauth\""
		);
		assert_eq!(
			App::error(wrong, StatusCode::UNAUTHORIZED).await,
			"invalid_client"
		);

		let exchanged = app
			.exchange(&client_id, Some(&secret), &code, VERIFIER)
			.await;
		assert_eq!(exchanged.status(), StatusCode::OK);
		assert_eq!(exchanged.headers()[header::CACHE_CONTROL], "no-store");
		assert_eq!(
			exchanged.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
			"*"
		);
		let tokens: TokenResponse = body(exchanged).await;
		assert_eq!(tokens.token_type, "Bearer");
		assert_eq!(tokens.expires_in, 600);
		assert_eq!(tokens.scope, "openid profile email notes:read");

		// The ID token is signed with a published key, for this app.
		let id_token = tokens.id_token.clone().unwrap();
		assert_eq!(verify_with_jwks(&app, &id_token).await, "ES256");
		let id = claims(&id_token);
		assert_eq!(id["iss"], ORIGIN);
		assert_eq!(id["aud"], client_id.as_str());
		assert_eq!(id["nonce"], "n-0S6_WzA2Mj");
		assert!(id["auth_time"].is_i64());
		let access = claims(&tokens.access_token);
		assert_eq!(access["sub"], id["sub"]);
		assert_eq!(access["client_id"], client_id.as_str());
		assert_eq!(access["aud"], ORIGIN);

		// The subject is pairwise, not the account ID.
		let me: String = text(app.get("/me", Some(&cookie)).await).await;
		let account = me.split(' ').next().unwrap().to_owned();
		assert_ne!(id["sub"], account.as_str());

		// userinfo releases what the scopes allow.
		let info = app
			.with_bearer("GET", "/oauth/userinfo", &tokens.access_token)
			.await;
		assert_eq!(info.status(), StatusCode::OK);
		let info: Value = body(info).await;
		assert_eq!(info["sub"], id["sub"]);
		assert_eq!(info["name"], "ada");
		assert_eq!(info["email"], "ada@example.com");
		assert_eq!(info["email_verified"], true);

		// The token calls the scoped function as Ada, from another site, and
		// her policy shows her only her own row.
		let called = app
			.with_bearer("POST", "/api/list_notes", &tokens.access_token)
			.await;
		assert_eq!(called.status(), StatusCode::OK);
		assert_eq!(text(called).await, format!("{account} sees 1"));

		// It can't call what needs another scope, or what names none.
		let refused = app
			.with_bearer("POST", "/api/write_note", &tokens.access_token)
			.await;
		assert_eq!(refused.status(), StatusCode::FORBIDDEN);
		assert_eq!(
			refused.headers()[header::WWW_AUTHENTICATE],
			"Bearer error=\"insufficient_scope\", scope=\"notes:read notes:write\""
		);
		for path in ["/api/delete_everything", "/auth/sessions", "/oauth/grants"] {
			let method = if path.starts_with("/api") {
				"POST"
			} else {
				"GET"
			};
			let refused = app.with_bearer(method, path, &tokens.access_token).await;
			assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{path}");
		}
		let garbage = app.with_bearer("POST", "/api/list_notes", "a.b.c").await;
		assert_eq!(garbage.status(), StatusCode::UNAUTHORIZED);

		// Introspection describes both tokens to the app that holds them.
		let described = app
			.introspect(&client_id, &secret, &tokens.access_token)
			.await;
		assert!(described.active);
		assert_eq!(described.token_type.as_deref(), Some("Bearer"));
		assert_eq!(described.sub.as_deref(), id["sub"].as_str());
		let described = app
			.introspect(
				&client_id,
				&secret,
				tokens.refresh_token.as_deref().unwrap(),
			)
			.await;
		assert!(described.active);
		assert_eq!(described.token_type.as_deref(), Some("refresh_token"));

		// Refreshing rotates the refresh token.
		let refreshed = app
			.refresh(
				&client_id,
				&secret,
				tokens.refresh_token.as_deref().unwrap(),
			)
			.await;
		assert_eq!(refreshed.status(), StatusCode::OK);
		let refreshed: TokenResponse = body(refreshed).await;
		assert_ne!(refreshed.refresh_token, tokens.refresh_token);
		assert!(refreshed.id_token.is_some());
		assert!(
			!app.introspect(
				&client_id,
				&secret,
				tokens.refresh_token.as_deref().unwrap()
			)
			.await
			.active
		);

		// Using the old one again ends the whole family.
		let second_use = app
			.refresh(
				&client_id,
				&secret,
				tokens.refresh_token.as_deref().unwrap(),
			)
			.await;
		assert_eq!(
			App::error(second_use, StatusCode::BAD_REQUEST).await,
			"invalid_grant"
		);
		let after = app
			.refresh(
				&client_id,
				&secret,
				refreshed.refresh_token.as_deref().unwrap(),
			)
			.await;
		assert_eq!(
			App::error(after, StatusCode::BAD_REQUEST).await,
			"invalid_grant"
		);
		let after = app
			.with_bearer("GET", "/oauth/userinfo", &refreshed.access_token)
			.await;
		assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
		assert_eq!(
			after.headers()[header::WWW_AUTHENTICATE],
			"Bearer error=\"invalid_token\""
		);

		// The code worked once; replaying it is refused.
		let replayed = app
			.exchange(&client_id, Some(&secret), &code, VERIFIER)
			.await;
		assert_eq!(
			App::error(replayed, StatusCode::BAD_REQUEST).await,
			"invalid_grant"
		);

		// Consent is remembered: the next sign-in skips the consent page.
		let code = app.code(&client_id, "openid notes:read", &cookie).await;
		let tokens: TokenResponse = body(
			app.exchange(&client_id, Some(&secret), &code, VERIFIER)
				.await,
		)
		.await;
		assert_eq!(tokens.scope, "openid notes:read");

		// Revoking the refresh token ends its access token too.
		let revoked = app
			.post_form(
				"/oauth/revoke",
				&[
					("token", tokens.refresh_token.as_deref().unwrap()),
					("token_type_hint", "refresh_token"),
				],
				Some(&basic(&client_id, &secret)),
			)
			.await;
		assert_eq!(revoked.status(), StatusCode::OK);
		assert!(
			!app.introspect(&client_id, &secret, &tokens.access_token)
				.await
				.active
		);
		let unknown = app
			.post_form(
				"/oauth/revoke",
				&[("token", "never-issued")],
				Some(&basic(&client_id, &secret)),
			)
			.await;
		assert_eq!(unknown.status(), StatusCode::OK);

		// Connected apps lists the partner, and disconnecting it ends its
		// tokens.
		let code = app.code(&client_id, "openid", &cookie).await;
		let tokens: TokenResponse = body(
			app.exchange(&client_id, Some(&secret), &code, VERIFIER)
				.await,
		)
		.await;
		let connected: Vec<ConnectedApp> =
			body(app.get("/oauth/grants", Some(&cookie)).await).await;
		assert_eq!(connected.len(), 1);
		assert_eq!(connected[0].app.client_id, client_id);
		assert!(
			connected[0]
				.scopes
				.iter()
				.any(|scope| scope.name == "notes:read")
		);
		let disconnected = app
			.post_json(
				"/oauth/grants/revoke",
				json!({ "client_id": client_id }),
				Some(&cookie),
			)
			.await;
		assert_eq!(disconnected.status(), StatusCode::NO_CONTENT);
		let after = app
			.refresh(
				&client_id,
				&secret,
				tokens.refresh_token.as_deref().unwrap(),
			)
			.await;
		assert_eq!(
			App::error(after, StatusCode::BAD_REQUEST).await,
			"invalid_grant"
		);
		let after = app
			.with_bearer("POST", "/api/list_notes", &tokens.access_token)
			.await;
		assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
		let connected: Vec<ConnectedApp> =
			body(app.get("/oauth/grants", Some(&cookie)).await).await;
		assert_eq!(connected, [] as [ConnectedApp; 0]);
		let again = app
			.post_json(
				"/oauth/grants/revoke",
				json!({ "client_id": client_id }),
				Some(&cookie),
			)
			.await;
		assert_eq!(again.status(), StatusCode::NOT_FOUND);

		// Disconnected, the next sign-in asks for consent again.
		let to = location(
			&app.get(
				&App::authorize_url(&client_id, "openid", &[]),
				Some(&cookie),
			)
			.await,
		);
		assert!(to.starts_with("/consent?"), "{to}");
	}
}

#[tokio::test]
async fn a_public_app_signs_in_with_pkce_and_signs_out() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(db).await;
		let (client_id, secret) = app.register(ClientAuth::None, &["openid"]).await;
		assert!(secret.is_none());
		let cookie = app.sign_in("ada@example.com").await;

		let code = app.code(&client_id, "openid", &cookie).await;
		let tokens = app.exchange(&client_id, None, &code, VERIFIER).await;
		assert_eq!(tokens.status(), StatusCode::OK);
		let tokens: TokenResponse = body(tokens).await;

		// A public app gets the same family rules: refresh with its ID alone.
		let refreshed = app
			.post_form(
				"/oauth/token",
				&[
					("grant_type", "refresh_token"),
					("refresh_token", tokens.refresh_token.as_deref().unwrap()),
					("client_id", client_id.as_str()),
				],
				None,
			)
			.await;
		assert_eq!(refreshed.status(), StatusCode::OK);
		let tokens: TokenResponse = body(refreshed).await;

		// A token without the scope can't call the scoped function.
		let refused = app
			.with_bearer("POST", "/api/list_notes", &tokens.access_token)
			.await;
		assert_eq!(refused.status(), StatusCode::FORBIDDEN);

		// A public app can't introspect.
		let refused = app
			.post_form(
				"/oauth/introspect",
				&[
					("token", tokens.access_token.as_str()),
					("client_id", client_id.as_str()),
				],
				None,
			)
			.await;
		assert_eq!(
			App::error(refused, StatusCode::UNAUTHORIZED).await,
			"invalid_client"
		);

		// Signing out from the app ends its tokens and the browser's session,
		// and returns to the registered address with its state.
		let id_token = tokens.id_token.unwrap();
		let query = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs([
				("id_token_hint", id_token.as_str()),
				("post_logout_redirect_uri", SIGNED_OUT),
				("state", "bye"),
			])
			.finish();
		let ended = app
			.get(&format!("/oauth/end_session?{query}"), Some(&cookie))
			.await;
		assert_eq!(location(&ended), format!("{SIGNED_OUT}?state=bye"));
		assert!(
			ended.headers()[header::SET_COOKIE]
				.to_str()
				.unwrap()
				.contains("Max-Age=0")
		);
		let session: Value = body(app.get("/auth/session", Some(&cookie)).await).await;
		assert_eq!(session["session"], Value::Null);
		let after = app
			.with_bearer("GET", "/oauth/userinfo", &tokens.access_token)
			.await;
		assert_eq!(after.status(), StatusCode::UNAUTHORIZED);

		// Signed out now, `prompt=none` hears `login_required`.
		let to = location(
			&app.get(
				&App::authorize_url(&client_id, "openid", &[("prompt", "none")]),
				Some(&cookie),
			)
			.await,
		);
		assert_eq!(param(&to, "error").as_deref(), Some("login_required"));

		// An unregistered post-logout address is refused, not followed.
		let query = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs([
				("id_token_hint", id_token.as_str()),
				("post_logout_redirect_uri", "https://evil.example/"),
			])
			.finish();
		let refused = app.get(&format!("/oauth/end_session?{query}"), None).await;
		assert_eq!(refused.status(), StatusCode::BAD_REQUEST);

		// Without a hint, nobody is signed out.
		let cookie = app.sign_in("ada@example.com").await;
		let ended = app.get("/oauth/end_session", Some(&cookie)).await;
		assert_eq!(location(&ended), "/");
		assert!(ended.headers().get(header::SET_COOKIE).is_none());
	}
}

#[tokio::test]
async fn bad_requests_are_refused_where_its_safe_to_say_so() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	let app = app(db).await;
	let (client_id, _) = app.register(ClientAuth::SecretBasic, &["openid"]).await;
	let cookie = app.sign_in("ada@example.com").await;
	let authorize = |extra: &[(&str, &str)], drop: &[&str]| {
		let url = App::authorize_url(&client_id, "openid", extra);
		let mut pairs = query(&url);
		pairs.retain(|(key, _)| !drop.contains(&key.as_str()));
		let mut deduped: Vec<(String, String)> = Vec::new();
		for (key, value) in pairs.into_iter().rev() {
			if !deduped.iter().any(|(seen, _)| *seen == key) {
				deduped.push((key, value));
			}
		}
		let query = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs(deduped)
			.finish();
		format!("/oauth/authorize?{query}")
	};

	// Nowhere safe to send the browser: a page, not a redirect.
	for url in [
		authorize(&[("client_id", "unknown")], &[]),
		authorize(&[("redirect_uri", "https://evil.example/callback")], &[]),
		authorize(&[], &["redirect_uri"]),
	] {
		let response = app.get(&url, Some(&cookie)).await;
		assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{url}");
		assert!(text(response).await.contains("doesn't work"));
	}

	// Everything else goes back to the app.
	for (extra, drop, error) in [
		(
			&[("response_type", "token")][..],
			&[][..],
			"unsupported_response_type",
		),
		(
			&[("code_challenge_method", "plain")],
			&[],
			"invalid_request",
		),
		(&[], &["code_challenge"], "invalid_request"),
		(&[("scope", "openid notes:read")], &[], "invalid_scope"),
		(&[("scope", "")], &[], "invalid_scope"),
		(&[("prompt", "none login")], &[], "invalid_request"),
		(&[("max_age", "soon")], &[], "invalid_request"),
		(&[("request", "eyJ")], &[], "request_not_supported"),
		(&[("response_mode", "fragment")], &[], "invalid_request"),
		(&[("prompt", "none")], &[], "consent_required"),
	] {
		let to = location(&app.get(&authorize(extra, drop), Some(&cookie)).await);
		assert!(to.starts_with(CALLBACK), "{to}");
		assert_eq!(
			param(&to, "error").as_deref(),
			Some(error),
			"{extra:?} {drop:?}"
		);
		assert_eq!(param(&to, "state").as_deref(), Some("af0ifjsldkj"));
	}

	// `prompt=login` asks to sign in again, then comes back satisfied.
	let to = location(
		&app.get(&authorize(&[("prompt", "login")], &[]), Some(&cookie))
			.await,
	);
	let return_to = param(&to, "return_to").unwrap();
	assert_eq!(param(&return_to, "prompt"), None);
	assert_eq!(param(&return_to, "max_age").as_deref(), Some("300"));

	// Denying consent sends `access_denied`.
	let to = location(&app.get(&authorize(&[], &[]), Some(&cookie)).await);
	let request = param(&to, "request").unwrap();
	let denied = app
		.post_json(
			"/oauth/consent",
			json!({ "request": request, "approve": false }),
			Some(&cookie),
		)
		.await;
	let to = body::<ConsentRedirect>(denied).await.redirect_to;
	assert_eq!(param(&to, "error").as_deref(), Some("access_denied"));

	// Signed out, the consent routes refuse.
	let refused = app.get("/oauth/consent?request=x", None).await;
	assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
	let refused = app.get("/oauth/grants", None).await;
	assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_token_endpoints_refuse_what_they_cannot_take() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	let app = app(db).await;

	// The token endpoint wants a form, a grant type it knows, and the code.
	let (secret_client, secret) = app.register(ClientAuth::SecretPost, &["openid"]).await;
	let secret = secret.unwrap();
	let post_auth = [
		("client_id", secret_client.as_str()),
		("client_secret", secret.as_str()),
	];
	for (pairs, error) in [
		(vec![("grant_type", "password")], "unsupported_grant_type"),
		(vec![], "invalid_request"),
		(
			vec![("grant_type", "authorization_code")],
			"invalid_request",
		),
		(
			vec![
				("grant_type", "authorization_code"),
				("code", "nope"),
				("redirect_uri", CALLBACK),
				("code_verifier", VERIFIER),
			],
			"invalid_grant",
		),
		(
			vec![("grant_type", "refresh_token"), ("refresh_token", "nope")],
			"invalid_grant",
		),
	] {
		let mut pairs = pairs;
		pairs.extend_from_slice(&post_auth);
		let response = app.post_form("/oauth/token", &pairs, None).await;
		assert_eq!(
			App::error(response, StatusCode::BAD_REQUEST).await,
			error,
			"{pairs:?}"
		);
	}
	// A secret_post app can't switch to Basic.
	let response = app
		.post_form(
			"/oauth/token",
			&[("grant_type", "refresh_token"), ("refresh_token", "x")],
			Some(&basic(&secret_client, &secret)),
		)
		.await;
	assert_eq!(
		App::error(response, StatusCode::UNAUTHORIZED).await,
		"invalid_client"
	);
	// Repeated parameters aren't a form this endpoint takes.
	let response = app
		.post_form(
			"/oauth/token",
			&[
				("grant_type", "refresh_token"),
				("grant_type", "refresh_token"),
			],
			None,
		)
		.await;
	assert_eq!(
		App::error(response, StatusCode::BAD_REQUEST).await,
		"invalid_request"
	);
	// Preflights are answered for browsers.
	let preflight = app
		.send(
			Request::builder()
				.method("OPTIONS")
				.uri("/oauth/userinfo")
				.body(Body::empty())
				.unwrap(),
		)
		.await;
	assert_eq!(preflight.status(), StatusCode::NO_CONTENT);
	assert_eq!(
		preflight.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS],
		"authorization, content-type"
	);
	let missing = app.get("/oauth/userinfo", None).await;
	assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn operators_manage_clients_and_removing_one_ends_its_grants() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	let app = app(db).await;
	let (client_id, secret) = app
		.register(ClientAuth::SecretBasic, &["openid", "notes:read"])
		.await;
	let cookie = app.sign_in("ada@example.com").await;
	let code = app.code(&client_id, "openid", &cookie).await;
	let tokens: TokenResponse = body(
		app.exchange(&client_id, secret.as_deref(), &code, VERIFIER)
			.await,
	)
	.await;

	let ClientCommandOutput::Listed { clients } = app.idp.run_command(&ClientCommand::List).await
	else {
		panic!("listing works");
	};
	assert_eq!(clients.len(), 1);
	assert_eq!(clients[0].scopes, ["openid", "notes:read"]);

	let refused = app
		.idp
		.run_command(&ClientCommand::Add(NewClientSpec {
			name: "Bad".into(),
			redirect_uris: vec!["http://partner.example/cb".into()],
			post_logout_redirect_uris: Vec::new(),
			scopes: vec!["openid".into()],
			auth_method: ClientAuth::None,
			grant_types: leptoskit_idp::default_grant_types(),
			jwks: None,
			id_token_alg: leptoskit_idp::SigningAlgorithm::Es256,
			first_party: false,
			introspection: false,
		}))
		.await;
	assert!(
		matches!(&refused, ClientCommandOutput::Failed { message } if message.contains("https")),
		"{refused:?}"
	);
	let refused = app
		.idp
		.run_command(&ClientCommand::Add(NewClientSpec {
			name: "Bad".into(),
			redirect_uris: vec![CALLBACK.into()],
			post_logout_redirect_uris: Vec::new(),
			scopes: vec!["admin".into()],
			auth_method: ClientAuth::SecretBasic,
			grant_types: leptoskit_idp::default_grant_types(),
			jwks: None,
			id_token_alg: leptoskit_idp::SigningAlgorithm::Es256,
			first_party: false,
			introspection: false,
		}))
		.await;
	assert!(
		matches!(&refused, ClientCommandOutput::Failed { message } if message.contains("admin")),
		"{refused:?}"
	);

	let removed = app
		.idp
		.run_command(&ClientCommand::Remove {
			client_id: client_id.clone(),
		})
		.await;
	assert_eq!(removed, ClientCommandOutput::Removed { removed: true });
	let grants = Grant::query()
		.count(&app.db.system("test: count grants"))
		.await
		.unwrap();
	assert_eq!(grants, 0);
	let after = app
		.with_bearer("GET", "/oauth/userinfo", &tokens.access_token)
		.await;
	assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
	let again = app
		.idp
		.run_command(&ClientCommand::Remove { client_id })
		.await;
	assert_eq!(again, ClientCommandOutput::Removed { removed: false });
}

#[tokio::test]
async fn first_party_apps_see_the_account_id_and_skip_consent() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	let app = app(db).await;
	let output = app
		.idp
		.run_command(&ClientCommand::Add(NewClientSpec {
			name: "Notes for iOS".into(),
			redirect_uris: vec![CALLBACK.into()],
			post_logout_redirect_uris: Vec::new(),
			scopes: vec!["openid".into()],
			auth_method: ClientAuth::None,
			grant_types: leptoskit_idp::default_grant_types(),
			jwks: None,
			id_token_alg: leptoskit_idp::SigningAlgorithm::Es256,
			first_party: true,
			introspection: false,
		}))
		.await;
	let ClientCommandOutput::Added { client, .. } = output else {
		panic!("{output:?}");
	};
	let cookie = app.sign_in("ada@example.com").await;
	let account = text(app.get("/me", Some(&cookie)).await).await;
	let account = account.split(' ').next().unwrap();

	let to = location(
		&app.get(
			&App::authorize_url(&client.client_id, "openid", &[]),
			Some(&cookie),
		)
		.await,
	);
	assert!(to.starts_with(CALLBACK), "no consent page: {to}");
	let code = param(&to, "code").unwrap();
	let tokens: TokenResponse =
		body(app.exchange(&client.client_id, None, &code, VERIFIER).await).await;
	assert_eq!(claims(&tokens.id_token.unwrap())["sub"], account);
}

#[tokio::test]
async fn edges_of_the_token_endpoints_are_refused_or_handled() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	let app = app(db).await;
	let (client_id, secret) = app
		.register(ClientAuth::SecretBasic, &["openid", "notes:read"])
		.await;
	let secret = secret.unwrap();
	let auth = basic(&client_id, &secret);
	let cookie = app.sign_in("ada@example.com").await;

	// A code must go back to the address it was sent to.
	let code = app.code(&client_id, "openid notes:read", &cookie).await;
	let wrong = app
		.post_form(
			"/oauth/token",
			&[
				("grant_type", "authorization_code"),
				("code", code.as_str()),
				("redirect_uri", "https://partner.example/other"),
				("code_verifier", VERIFIER),
			],
			Some(&auth),
		)
		.await;
	assert_eq!(
		App::error(wrong, StatusCode::BAD_REQUEST).await,
		"invalid_grant"
	);
	let tokens: TokenResponse = body(
		app.exchange(&client_id, Some(&secret), &code, VERIFIER)
			.await,
	)
	.await;

	// A refresh can narrow the scopes, never widen them.
	let narrowed = app
		.post_form(
			"/oauth/token",
			&[
				("grant_type", "refresh_token"),
				("refresh_token", tokens.refresh_token.as_deref().unwrap()),
				("scope", "notes:read"),
			],
			Some(&auth),
		)
		.await;
	let narrowed: TokenResponse = body(narrowed).await;
	assert_eq!(narrowed.scope, "notes:read");
	assert!(narrowed.id_token.is_none());
	let widened = app
		.post_form(
			"/oauth/token",
			&[
				("grant_type", "refresh_token"),
				("refresh_token", narrowed.refresh_token.as_deref().unwrap()),
				("scope", "openid profile"),
			],
			Some(&auth),
		)
		.await;
	assert_eq!(
		App::error(widened, StatusCode::BAD_REQUEST).await,
		"invalid_scope"
	);

	// Without `openid`, userinfo is refused.
	let refused = app
		.with_bearer("GET", "/oauth/userinfo", &narrowed.access_token)
		.await;
	assert_eq!(refused.status(), StatusCode::FORBIDDEN);

	// Revoking and introspecting need a form, a token, and usable credentials.
	for path in ["/oauth/revoke", "/oauth/introspect"] {
		let missing = app.post_form(path, &[], Some(&auth)).await;
		assert_eq!(
			App::error(missing, StatusCode::BAD_REQUEST).await,
			"invalid_request"
		);
		let bad = app
			.post_form(path, &[("token", "x")], Some("Basic !!!"))
			.await;
		assert_eq!(
			App::error(bad, StatusCode::UNAUTHORIZED).await,
			"invalid_client"
		);
		let wrong = app
			.post_form(path, &[("token", "x")], Some(&basic(&client_id, "nope")))
			.await;
		assert_eq!(
			App::error(wrong, StatusCode::UNAUTHORIZED).await,
			"invalid_client"
		);
		let repeated = app
			.post_form(path, &[("token", "x"), ("token", "y")], Some(&auth))
			.await;
		assert_eq!(
			App::error(repeated, StatusCode::BAD_REQUEST).await,
			"invalid_request"
		);
	}
	let bad = app
		.post_form("/oauth/token", &[("grant_type", "x")], Some("Basic !!!"))
		.await;
	assert_eq!(
		App::error(bad, StatusCode::UNAUTHORIZED).await,
		"invalid_client"
	);

	// A resource server that may introspect sees other apps' tokens.
	let output = app
		.idp
		.run_command(&ClientCommand::Add(NewClientSpec {
			name: "API".into(),
			redirect_uris: vec![CALLBACK.into()],
			post_logout_redirect_uris: Vec::new(),
			scopes: vec!["openid".into()],
			auth_method: ClientAuth::SecretPost,
			grant_types: leptoskit_idp::default_grant_types(),
			jwks: None,
			id_token_alg: leptoskit_idp::SigningAlgorithm::Es256,
			first_party: false,
			introspection: true,
		}))
		.await;
	let ClientCommandOutput::Added {
		client: resource,
		secret: Some(resource_secret),
	} = output
	else {
		panic!("{output:?}");
	};
	let seen = app
		.post_form(
			"/oauth/introspect",
			&[
				("token", narrowed.access_token.as_str()),
				("client_id", resource.client_id.as_str()),
				("client_secret", resource_secret.as_str()),
			],
			None,
		)
		.await;
	let seen: Introspection = body(seen).await;
	assert!(seen.active);
	assert_eq!(seen.client_id.as_deref(), Some(client_id.as_str()));
	let (other, other_secret) = app.register(ClientAuth::SecretBasic, &["openid"]).await;
	let unseen = app
		.introspect(&other, &other_secret.unwrap(), &narrowed.access_token)
		.await;
	assert!(!unseen.active);
}

#[tokio::test]
async fn edges_of_signing_in_and_out_are_refused_or_handled() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	let app = app(db).await;
	let (client_id, secret) = app
		.register(ClientAuth::SecretBasic, &["openid", "notes:read"])
		.await;
	let secret = secret.unwrap();
	let auth = basic(&client_id, &secret);
	let (other, _) = app.register(ClientAuth::SecretBasic, &["openid"]).await;
	let cookie = app.sign_in("ada@example.com").await;
	let code = app.code(&client_id, "openid notes:read", &cookie).await;
	let tokens: TokenResponse = body(
		app.exchange(&client_id, Some(&secret), &code, VERIFIER)
			.await,
	)
	.await;

	// A sign-in link with a repeated parameter is a page, not a redirect.
	let repeated = app
		.get(
			&format!("/oauth/authorize?client_id={client_id}&client_id={client_id}"),
			Some(&cookie),
		)
		.await;
	assert_eq!(repeated.status(), StatusCode::BAD_REQUEST);

	// Signing out: a forged hint, a hint for another app, and a registered
	// address without state.
	let id_token = tokens.id_token.unwrap();
	for query in [
		vec![("id_token_hint", "a.b.c")],
		vec![
			("id_token_hint", id_token.as_str()),
			("client_id", other.as_str()),
		],
	] {
		let query = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs(query)
			.finish();
		let refused = app.get(&format!("/oauth/end_session?{query}"), None).await;
		assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
	}
	let query = url::form_urlencoded::Serializer::new(String::new())
		.extend_pairs([
			("client_id", client_id.as_str()),
			("post_logout_redirect_uri", SIGNED_OUT),
		])
		.finish();
	let ended = app.get(&format!("/oauth/end_session?{query}"), None).await;
	assert_eq!(location(&ended), SIGNED_OUT);
	let malformed = app.get("/oauth/end_session?state=a&state=b", None).await;
	assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

	// Disconnecting an app nobody registered finds nothing.
	let unknown = app
		.post_json(
			"/oauth/grants/revoke",
			json!({ "client_id": "nobody" }),
			Some(&cookie),
		)
		.await;
	assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

	// A disabled account's tokens and codes stop working.
	let code = app.code(&client_id, "openid", &cookie).await;
	let account = Id::<Account>::parse(
		text(app.get("/me", Some(&cookie)).await)
			.await
			.split(' ')
			.next()
			.unwrap(),
	)
	.unwrap();
	let auth_service = Auth::builder(&app.db, Keys::single("test", [5; 32]))
		.mailer(Mailbox::new())
		.build()
		.unwrap();
	auth_service
		.disable(account, &leptoskit_auth::RequestInfo::default())
		.await
		.unwrap();
	let refused = app
		.exchange(&client_id, Some(&secret), &code, VERIFIER)
		.await;
	assert_eq!(
		App::error(refused, StatusCode::BAD_REQUEST).await,
		"invalid_grant"
	);
	let refused = app
		.with_bearer("POST", "/api/list_notes", &tokens.access_token)
		.await;
	assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
	let refused = app
		.post_form(
			"/oauth/token",
			&[
				("grant_type", "refresh_token"),
				("refresh_token", tokens.refresh_token.as_deref().unwrap()),
			],
			Some(&auth),
		)
		.await;
	assert_eq!(
		App::error(refused, StatusCode::BAD_REQUEST).await,
		"invalid_grant"
	);
}

#[tokio::test]
async fn the_provider_is_configured_carefully() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let without_origin = Auth::builder(&db, Keys::single("test", [5; 32]))
		.mailer(Mailbox::new())
		.build()
		.unwrap();
	let error = |builder: leptoskit_idp::IdpBuilder| builder.build().unwrap_err().to_string();

	assert!(error(Idp::builder(&db, &without_origin)).contains("set the issuer"));
	assert!(
		error(
			Idp::builder(&db, &without_origin)
				.issuer("https://notes.example")
				.signed_out_page("https://evil.example")
		)
		.contains("path on this site")
	);
	assert!(
		error(
			Idp::builder(&db, &without_origin)
				.issuer("https://notes.example")
				.scopes::<Scope>()
				.scopes::<Scope>()
		)
		.contains("declared twice")
	);
	assert!(
		error(
			Idp::builder(&db, &without_origin)
				.issuer("https://notes.example")
				.lifetimes(leptoskit_idp::Lifetimes {
					code: std::time::Duration::from_secs(3600),
					..leptoskit_idp::Lifetimes::default()
				})
		)
		.contains("lifetimes")
	);

	let builder = Idp::builder(&db, &without_origin)
		.issuer("https://notes.example/")
		.audience("https://api.notes.example")
		.scopes::<Scope>();
	assert!(format!("{builder:?}").contains("notes.example"));
	let idp = builder.build().unwrap();
	assert_eq!(idp.issuer(), "https://notes.example");
	assert_eq!(idp.scopes().len(), 5);
	assert!(format!("{idp:?}").contains("https://notes.example"));
	assert!(format!("{:?}", idp.layer()).contains("IdpLayer"));

	// Access tokens name the audience, and only it is accepted.
	let output = idp
		.run_command(&ClientCommand::Add(NewClientSpec {
			name: "Partner".into(),
			redirect_uris: vec![CALLBACK.into()],
			post_logout_redirect_uris: Vec::new(),
			scopes: vec!["openid".into()],
			auth_method: ClientAuth::None,
			grant_types: leptoskit_idp::default_grant_types(),
			jwks: None,
			id_token_alg: leptoskit_idp::SigningAlgorithm::Es256,
			first_party: true,
			introspection: false,
		}))
		.await;
	assert!(matches!(output, ClientCommandOutput::Added { .. }));
	let request = leptoskit_idp::TokenRequest {
		grant_type: Some("refresh_token".into()),
		refresh_token: Some("secret-refresh-token".into()),
		code_verifier: Some("secret-verifier".into()),
		..leptoskit_idp::TokenRequest::default()
	};
	let printed = format!("{request:?}");
	assert!(!printed.contains("secret"), "{printed}");
	let hint = leptoskit_idp::EndSessionRequest {
		id_token_hint: Some("secret-id-token".into()),
		..leptoskit_idp::EndSessionRequest::default()
	};
	assert!(!format!("{hint:?}").contains("secret"));
}
