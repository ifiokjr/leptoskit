//! Providers' own tokens, kept for providers configured with
//! `store_tokens`: sealed at rest, refreshed before they expire, deleted
//! when the grant ends, and deleted and revoked when the connected account
//! goes, by disconnecting, merging, or deleting the account.

#![cfg(all(
	feature = "postgres",
	feature = "sqlite",
	feature = "oauth",
	feature = "axum"
))]

#[path = "support/oidc.rs"]
mod oidc;
#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;

use std::collections::HashMap;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use http_body_util::BodyExt;
use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::Event;
use leptoskit_auth::EventKind;
use leptoskit_auth::IdentityInfo;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::Provider;
use leptoskit_auth::ProviderToken;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::SessionResponse;
use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::Statement;
use leptoskit_db::Value as DbValue;
use oidc::FakeIdp;
use oidc::SECRET;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt;

const ORIGIN: &str = "https://notes.example";

struct App {
	auth: Auth,
	router: Router,
	idp: FakeIdp,
}

fn app(db: &Db, keys: Keys) -> App {
	let idp = FakeIdp::new();
	let auth = Auth::builder(db, keys)
		.app_name("Notes")
		.origin(ORIGIN)
		.mailer(Mailbox::new())
		.provider(Provider::google("google-client", SECRET).store_tokens(true))
		.provider(Provider::discord("discord-client", SECRET).store_tokens(true))
		.provider(
			Provider::oidc(
				"company",
				"Company SSO",
				oidc::ISSUER,
				"company-client",
				Some(SECRET.into()),
			)
			.unwrap()
			.store_tokens(true),
		)
		.provider(Provider::github("github-client", SECRET))
		.provider_http(idp.clone())
		.build()
		.unwrap();
	App {
		router: Router::new().merge(auth.router()).layer(auth.layer()),
		auth,
		idp,
	}
}

async fn databases(cluster: Option<&PostgresCluster>) -> Vec<Db> {
	let mut databases = vec![Db::connect("sqlite::memory:").await.unwrap()];

	if let Some(cluster) = cluster {
		databases.push(Db::connect(&cluster.database()).await.unwrap());
	}

	for db in &databases {
		leptoskit_db::testing::create_tables(db).await.unwrap();
	}

	databases
}

#[derive(Default)]
struct Browser {
	jar: HashMap<String, String>,
}

impl Browser {
	async fn send(
		&mut self,
		app: &App,
		request: axum::http::request::Builder,
		body: Body,
	) -> Response {
		let cookies = self
			.jar
			.iter()
			.map(|(name, value)| format!("{name}={value}"))
			.collect::<Vec<_>>()
			.join("; ");
		let request = if cookies.is_empty() {
			request
		} else {
			request.header(header::COOKIE, cookies)
		};

		let response = app
			.router
			.clone()
			.oneshot(request.body(body).unwrap())
			.await
			.unwrap();

		for value in response.headers().get_all(header::SET_COOKIE) {
			let value = value.to_str().unwrap();
			let (pair, attributes) = value.split_once(';').unwrap_or((value, ""));
			let (name, cookie) = pair.split_once('=').unwrap();

			if cookie.is_empty() || attributes.contains("Max-Age=0") {
				self.jar.remove(name);
			} else {
				self.jar.insert(name.to_owned(), cookie.to_owned());
			}
		}

		response
	}

	async fn post(&mut self, app: &App, path: &str, body: Value) -> Response {
		let request = Request::post(path)
			.header(header::ORIGIN, ORIGIN)
			.header(header::CONTENT_TYPE, "application/json");
		self.send(app, request, Body::from(body.to_string())).await
	}

	/// Signs in (or connects, with `link`) with `provider` as `claims`, and
	/// returns the authorization URL and where the browser landed.
	async fn through(
		&mut self,
		app: &App,
		provider: &str,
		link: bool,
		claims: Value,
	) -> (String, String) {
		let path = if link {
			"/auth/oauth/link"
		} else {
			"/auth/oauth/start"
		};

		let started = self.post(app, path, json!({ "provider": provider })).await;
		assert_eq!(started.status(), StatusCode::OK);
		let url = json_body(started).await["url"].as_str().unwrap().to_owned();
		let back = app.idp.authorize(&url, claims);
		let query = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs(&back)
			.finish();
		let response = self
			.send(
				app,
				Request::get(format!("/auth/callback/{provider}?{query}")),
				Body::empty(),
			)
			.await;

		let location = response.headers()[header::LOCATION]
			.to_str()
			.unwrap()
			.to_owned();
		(url, location)
	}

	async fn account(&mut self, app: &App) -> Id<Account> {
		let response = self
			.send(app, Request::get("/auth/session"), Body::empty())
			.await;
		let session: SessionResponse = serde_json::from_value(json_body(response).await).unwrap();
		session.session.unwrap().account_id
	}
}

async fn json_body(response: Response) -> Value {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn person(sub: &str) -> Value {
	json!({ "sub": sub, "email": format!("{sub}@example.com"), "email_verified": true })
}

/// Moves the account's stored tokens' expiry to within the refresh margin.
async fn expire(db: &Db, account: Id<Account>) {
	let system = db.system("test: expire provider tokens");

	let (table, first, second) = match system.dialect() {
		Dialect::Postgres => ("auth.provider_tokens", "$1", "$2"),
		Dialect::Sqlite => ("auth_provider_tokens", "?1", "?2"),
	};

	system
		.execute(Statement {
			sql: format!("UPDATE {table} SET expires_at = {first} WHERE account_id = {second}"),
			args: vec![
				DbValue::Timestamp(leptoskit_db::now() + chrono::TimeDelta::seconds(30)),
				DbValue::Uuid(account.uuid()),
			],
		})
		.await
		.unwrap();
}

async fn stored(db: &Db, account: Id<Account>) -> Vec<ProviderToken> {
	ProviderToken::query()
		.filter(|token| token.account_id.eq(account))
		.all(&db.system("test"))
		.await
		.unwrap()
}

#[tokio::test]
async fn kept_tokens_are_sealed_and_refreshed_until_the_grant_ends() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db, Keys::single("test", [9; 32]));
		let mut browser = Browser::default();
		let (url, _) = browser.through(&app, "google", false, person("g-1")).await;
		let query: HashMap<String, String> = url::Url::parse(&url)
			.unwrap()
			.query_pairs()
			.into_owned()
			.collect();
		assert_eq!(
			query["access_type"], "offline",
			"Google needs asking for a refresh token"
		);
		let account = browser.account(&app).await;

		// Stored, but never as plain text.
		let rows = stored(&db, account).await;
		assert_eq!(rows.len(), 1);
		assert_eq!(rows[0].provider, "google");
		assert_eq!(rows[0].scope.as_deref(), Some("granted"));
		assert!(rows[0].refresh_token.is_some());
		assert!(
			!String::from_utf8_lossy(&rows[0].access_token).contains("access-"),
			"sealed"
		);

		let access = app
			.auth
			.provider_access_token(account, "google")
			.await
			.unwrap()
			.unwrap();
		assert!(access.access_token.starts_with("access-"));
		assert!(access.expires_at.is_some());
		assert!(!format!("{access:?}").contains("access-"), "Debug hides it");

		// Close to expiring, it's refreshed, and the new refresh token kept.
		expire(&db, account).await;
		let refreshed = app
			.auth
			.provider_access_token(account, "google")
			.await
			.unwrap()
			.unwrap();
		assert_ne!(refreshed.access_token, access.access_token);
		let row = &stored(&db, account).await[0];
		assert_ne!(row.refresh_token, rows[0].refresh_token, "rotated");
		let again = app
			.auth
			.provider_access_token(account, "google")
			.await
			.unwrap()
			.unwrap();
		assert_eq!(again.access_token, refreshed.access_token, "fresh, so kept");

		// The provider ends the grant: the tokens go, and the app asks the
		// person to connect again.
		app.idp.end_grants();
		expire(&db, account).await;
		assert!(
			app.auth
				.provider_access_token(account, "google")
				.await
				.unwrap()
				.is_none()
		);
		assert!(stored(&db, account).await.is_empty());

		// Signing in again keeps new ones.
		Browser::default()
			.through(&app, "google", false, person("g-1"))
			.await;
		assert_eq!(stored(&db, account).await.len(), 1);

		// A provider that doesn't keep tokens has none.
		let mut github = Browser::default();
		github
			.through(
				&app,
				"github",
				false,
				json!({ "user": { "id": 1, "login": "ada" }, "emails": [] }),
			)
			.await;
		let other = github.account(&app).await;
		assert!(stored(&db, other).await.is_empty());
		assert!(
			app.auth
				.provider_access_token(other, "github")
				.await
				.unwrap()
				.is_none()
		);
	}
}

#[tokio::test]
async fn disconnecting_merging_and_deleting_revoke_kept_tokens() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db, Keys::single("test", [9; 32]));

		// Disconnecting deletes the tokens and revokes them at Google.
		let mut ada = Browser::default();
		ada.through(
			&app,
			"discord",
			false,
			json!({ "user": { "id": "d-1", "username": "ada" } }),
		)
		.await;
		let account = ada.account(&app).await;
		ada.through(&app, "google", true, person("g-ada")).await;
		assert_eq!(stored(&db, account).await.len(), 2);
		let identities: Vec<IdentityInfo> = serde_json::from_value(
			json_body(
				ada.send(&app, Request::get("/auth/identities"), Body::empty())
					.await,
			)
			.await,
		)
		.unwrap();
		let google = identities
			.iter()
			.find(|identity| identity.provider == "google")
			.unwrap();
		let response = ada
			.post(&app, "/auth/identities/unlink", json!({ "id": google.id }))
			.await;

		assert_eq!(response.status(), StatusCode::NO_CONTENT);
		let rows = stored(&db, account).await;
		assert_eq!(rows.len(), 1);
		assert_eq!(rows[0].provider, "discord");
		assert!(
			app.idp
				.requests()
				.contains(&"POST https://oauth2.googleapis.com/revoke".to_owned())
		);
		assert_eq!(app.idp.revoked().len(), 1);
		assert!(
			app.idp.revoked()[0].starts_with("refresh-"),
			"the refresh token ends the grant"
		);

		// Merging another account in revokes the tokens that account kept.
		let mut old = Browser::default();
		old.through(&app, "company", false, person("c-old")).await;
		let absorbed = old.account(&app).await;
		assert_eq!(stored(&db, absorbed).await.len(), 1);
		ada.through(&app, "company", true, person("c-old")).await;
		let merged = ada
			.post(
				&app,
				"/auth/merge",
				json!({ "mode": "move_login", "confirm": true }),
			)
			.await;
		assert_eq!(merged.status(), StatusCode::NO_CONTENT);
		assert!(stored(&db, absorbed).await.is_empty());

		assert_eq!(
			stored(&db, account).await.len(),
			1,
			"only its own discord tokens"
		);
		assert!(
			app.idp
				.requests()
				.contains(&"POST https://sso.example/revoke".to_owned()),
			"found through discovery"
		);
		assert_eq!(app.idp.revoked().len(), 2);

		// Deleting the account revokes the rest, and leaves one entry naming
		// it.
		app.auth
			.delete_account(account, &RequestInfo::default())
			.await
			.unwrap();
		let system = db.system("test");
		assert!(Account::find(&system, account).await.unwrap().is_none());
		assert!(stored(&db, account).await.is_empty());
		assert!(
			app.idp
				.requests()
				.contains(&"POST https://discord.com/api/oauth2/token/revoke".to_owned())
		);
		let deleted = Event::query()
			.filter(|event| event.kind.eq(EventKind::AccountDeleted))
			.first(&system)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(deleted.account_id, None);
		assert_eq!(deleted.detail, Some(account.to_string()));
	}
}

#[tokio::test]
async fn rotating_keys_reseals_kept_tokens() {
	let db = leptoskit_db::testing::sqlite().await;
	let old = Keys::single("old", [1; 32]);
	let app = app(&db, old);
	let mut browser = Browser::default();
	browser.through(&app, "google", false, person("g-r")).await;
	let account = browser.account(&app).await;
	let before = app
		.auth
		.provider_access_token(account, "google")
		.await
		.unwrap()
		.unwrap();

	let both = Keys::parse(
		"new:AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI,old:\
		 AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE",
	)
	.unwrap();
	let rotated = Auth::builder(&db, both)
		.mailer(Mailbox::new())
		.provider(Provider::google("google-client", SECRET).store_tokens(true))
		.provider_http(app.idp.clone())
		.build()
		.unwrap();
	let report = rotated.rotate_keys().await.unwrap();
	assert_eq!(report.provider_tokens, 2, "the access and refresh tokens");
	assert_eq!(rotated.rotate_keys().await.unwrap().provider_tokens, 0);

	// Only the new key is needed now.
	let only_new = Auth::builder(&db, Keys::single("new", [2; 32]))
		.mailer(Mailbox::new())
		.provider(Provider::google("google-client", SECRET).store_tokens(true))
		.provider_http(app.idp.clone())
		.build()
		.unwrap();
	let after = only_new
		.provider_access_token(account, "google")
		.await
		.unwrap()
		.unwrap();
	assert_eq!(after.access_token, before.access_token);
}
