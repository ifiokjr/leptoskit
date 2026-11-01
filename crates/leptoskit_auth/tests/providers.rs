//! Third-party sign-in through the real routes, against a fake identity
//! provider served from memory: the generic OpenID Connect client, Google,
//! Apple, Microsoft, and GitHub; forged and replayed answers; linking,
//! unlinking, and merging accounts.

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
#[path = "support/totp.rs"]
mod totp;

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
use leptoskit_auth::EmailAddress;
use leptoskit_auth::Factor;
use leptoskit_auth::Identity;
use leptoskit_auth::IdentityInfo;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::MergeOffer;
use leptoskit_auth::Provider;
use leptoskit_auth::ProviderInfo;
use leptoskit_auth::ProviderOutcome;
use leptoskit_auth::SessionResponse;
use leptoskit_db::Db;
use leptoskit_db::Id;
use oidc::FakeIdp;
use oidc::SECRET;
use oidc::Tamper;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt;

const ORIGIN: &str = "https://notes.example";

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

struct App {
	auth: Auth,
	router: Router,
	idp: FakeIdp,
	mailbox: Mailbox,
}

fn app(db: &Db) -> App {
	let idp = FakeIdp::new();
	let (pem, public) = oidc::apple_key();
	idp.trust_apple_key(public);
	let mailbox = Mailbox::new();
	let auth = Auth::builder(db, Keys::single("test", [3; 32]))
		.app_name("Notes")
		.origin(ORIGIN)
		.mailer(mailbox.clone())
		.provider(
			Provider::oidc(
				"company",
				"Company SSO",
				oidc::ISSUER,
				"notes-client",
				Some(SECRET.into()),
			)
			.unwrap(),
		)
		.provider(Provider::google("google-client", SECRET))
		.provider(Provider::github("github-client", SECRET))
		.provider(Provider::microsoft("microsoft-client", SECRET, "common"))
		.provider(Provider::apple("com.notes.web", "TEAM1", "KEY1", &pem).unwrap())
		.provider_http(idp.clone())
		.build()
		.unwrap();
	let router = Router::new().merge(auth.router()).layer(auth.layer());
	App {
		auth,
		router,
		idp,
		mailbox,
	}
}

/// A browser: a cookie jar, and requests from the app's own pages.
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
		let mut request = request.header("user-agent", "Firefox/131.0");

		if !cookies.is_empty() {
			request = request.header(header::COOKIE, cookies);
		}

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

	async fn get(&mut self, app: &App, path: &str) -> Response {
		self.send(app, Request::get(path), Body::empty()).await
	}

	/// Starts signing in (or connecting, with `link`) with `provider`, signs
	/// in there as `claims`, and follows the callback back. Returns where
	/// the browser lands.
	async fn through(&mut self, app: &App, provider: &str, link: bool, claims: Value) -> String {
		let path = if link {
			"/auth/oauth/link"
		} else {
			"/auth/oauth/start"
		};

		let started = self
			.post(
				app,
				path,
				json!({ "provider": provider, "redirect_to": "/home" }),
			)
			.await;
		assert_eq!(started.status(), StatusCode::OK, "{}", text(started).await);
		let url = json_body(started).await["url"].as_str().unwrap().to_owned();
		let query = app.idp.authorize(&url, claims);
		self.callback(app, provider, &query).await
	}

	/// The provider sends the browser back with `query`, by a top-level
	/// navigation.
	async fn callback(&mut self, app: &App, provider: &str, query: &[(String, String)]) -> String {
		let query = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs(query)
			.finish();
		let response = self
			.get(app, &format!("/auth/callback/{provider}?{query}"))
			.await;
		assert_eq!(response.status(), StatusCode::SEE_OTHER);
		response.headers()[header::LOCATION]
			.to_str()
			.unwrap()
			.to_owned()
	}

	fn signed_in(&self) -> bool {
		self.jar.contains_key("__Host-session")
	}
}

async fn json_body(response: Response) -> Value {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn text(response: Response) -> String {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	String::from_utf8_lossy(&bytes).into_owned()
}

fn outcome(location: &str) -> ProviderOutcome {
	let query = location.split_once('?').map(|(_, query)| query).unwrap();
	ProviderOutcome::from_query(query).unwrap()
}

fn person(sub: &str, email: &str, verified: bool) -> Value {
	json!({ "sub": sub, "email": email, "email_verified": verified, "name": "Ada Lovelace" })
}

async fn account_of(browser: &mut Browser, app: &App) -> Id<Account> {
	let session: SessionResponse =
		serde_json::from_value(json_body(browser.get(app, "/auth/session").await).await).unwrap();
	session.session.unwrap().account_id
}

async fn email_sign_in(app: &App, browser: &mut Browser, address: &str) {
	browser
		.post(app, "/auth/email/send", json!({ "email": address }))
		.await;
	let code = app
		.mailbox
		.latest_to(address)
		.unwrap()
		.subject
		.rsplit(' ')
		.next()
		.unwrap()
		.to_owned();
	let response = browser
		.post(
			app,
			"/auth/email/verify",
			json!({ "email": address, "code": code }),
		)
		.await;
	assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_openid_connect_provider_signs_people_up_then_in() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db);
		let mut browser = Browser::default();

		let providers: Vec<ProviderInfo> =
			serde_json::from_value(json_body(browser.get(&app, "/auth/providers").await).await)
				.unwrap();
		assert_eq!(providers.len(), 5);
		assert!(providers.contains(&ProviderInfo {
			id: "company".into(),
			name: "Company SSO".into()
		}));

		// The authorization request carries PKCE, state, and a nonce.
		let started = browser
			.post(&app, "/auth/oauth/start", json!({ "provider": "company" }))
			.await;
		assert!(browser.jar.contains_key("__Host-provider"));
		let url = json_body(started).await["url"].as_str().unwrap().to_owned();
		assert!(url.starts_with("https://sso.example/authorize?"));
		let query: HashMap<String, String> = url::Url::parse(&url)
			.unwrap()
			.query_pairs()
			.into_owned()
			.collect();
		assert_eq!(
			query["redirect_uri"],
			"https://notes.example/auth/callback/company"
		);
		assert_eq!(query["scope"], "openid email profile");
		assert_eq!(query["state"].len(), 43);

		assert_eq!(query["code_challenge"].len(), 43);
		assert!(query.contains_key("nonce"));

		let back = app
			.idp
			.authorize(&url, person("u-1", "ada@example.com", true));
		let location = browser.callback(&app, "company", &back).await;
		assert_eq!(location, "/?auth=signed_in");
		assert!(browser.signed_in());
		assert!(
			!browser.jar.contains_key("__Host-provider"),
			"the binding goes"
		);
		let account = account_of(&mut browser, &app).await;

		// The vouched-for address is the account's, and the identity is
		// listed.
		let email = EmailAddress::query()
			.filter(|email| email.account_id.eq(account))
			.first(&db.system("test"))
			.await
			.unwrap()
			.unwrap();
		assert_eq!(email.address.as_str(), "ada@example.com");
		assert!(email.verified_at.is_some());
		let identities: Vec<IdentityInfo> =
			serde_json::from_value(json_body(browser.get(&app, "/auth/identities").await).await)
				.unwrap();
		assert_eq!(identities.len(), 1);
		assert_eq!(identities[0].provider_name, "Company SSO");
		assert_eq!(identities[0].name.as_deref(), Some("Ada Lovelace"));

		// The same subject signs in to the same account, whatever its email
		// now says; the key set was fetched once.
		let mut again = Browser::default();
		let location = again
			.through(
				&app,
				"company",
				false,
				person("u-1", "changed@example.com", false),
			)
			.await;
		assert_eq!(location, "/home?auth=signed_in");
		assert_eq!(account_of(&mut again, &app).await, account);
		let fetched = app
			.idp
			.requests()
			.iter()
			.filter(|request| request.ends_with("/jwks"))
			.count();
		assert_eq!(fetched, 1);
	}
}

#[tokio::test]
async fn forged_replayed_and_misdirected_answers_are_refused() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let app = app(&db);

	for (tamper, code) in [
		(Tamper::ForeignKey, "provider"),
		(Tamper::Audience, "provider"),
		(Tamper::Nonce, "provider"),
		(Tamper::Expired, "provider"),
		(Tamper::Issuer, "provider"),
		(Tamper::Unsigned, "provider"),
	] {
		app.idp.tamper(tamper);
		let mut browser = Browser::default();
		let location = browser
			.through(
				&app,
				"company",
				false,
				person("u-2", "eve@example.com", true),
			)
			.await;
		assert_eq!(
			outcome(&location),
			ProviderOutcome::Failed(code.into()),
			"{tamper:?}"
		);
		assert!(!browser.signed_in(), "{tamper:?}");
	}

	app.idp.tamper(Tamper::Nothing);

	// A callback works once, and only in the browser that started it.
	let mut browser = Browser::default();
	let started = browser
		.post(&app, "/auth/oauth/start", json!({ "provider": "company" }))
		.await;
	let url = json_body(started).await["url"].as_str().unwrap().to_owned();
	let back = app
		.idp
		.authorize(&url, person("u-3", "bob@example.com", true));
	let mut attacker = Browser::default();
	let location = attacker.callback(&app, "company", &back).await;
	assert_eq!(
		outcome(&location),
		ProviderOutcome::Failed("sign_in_expired".into())
	);
	let mut other = Browser::default();

	other
		.post(&app, "/auth/oauth/start", json!({ "provider": "company" }))
		.await;
	let location = other.callback(&app, "company", &back).await;
	assert_eq!(
		outcome(&location),
		ProviderOutcome::Failed("sign_in_expired".into())
	);

	let location = browser.callback(&app, "company", &back).await;
	assert_eq!(outcome(&location), ProviderOutcome::SignedIn);
	browser
		.post(&app, "/auth/oauth/start", json!({ "provider": "company" }))
		.await;
	let location = browser.callback(&app, "company", &back).await;
	assert_eq!(
		outcome(&location),
		ProviderOutcome::Failed("sign_in_expired".into()),
		"the state was used"
	);

	// The provider's own refusal, and a flow for another provider.
	browser
		.post(&app, "/auth/oauth/start", json!({ "provider": "company" }))
		.await;
	let cancelled = vec![("error".to_owned(), "access_denied".to_owned())];
	let location = browser.callback(&app, "company", &cancelled).await;
	assert_eq!(
		outcome(&location),
		ProviderOutcome::Failed("cancelled".into())
	);
	let started = browser
		.post(&app, "/auth/oauth/start", json!({ "provider": "company" }))
		.await;
	let url = json_body(started).await["url"].as_str().unwrap().to_owned();
	let back = app
		.idp
		.authorize(&url, person("u-4", "x@example.com", true));
	let location = browser.callback(&app, "google", &back).await;

	assert_eq!(
		outcome(&location),
		ProviderOutcome::Failed("sign_in_expired".into())
	);

	// Starting: unknown providers, other sites, and native apps.
	for (body, status, code) in [
		(
			json!({ "provider": "myspace" }),
			StatusCode::NOT_FOUND,
			"unknown_provider",
		),
		(
			json!({ "provider": "company", "redirect_to": "//evil.example" }),
			StatusCode::BAD_REQUEST,
			"invalid_redirect",
		),
	] {
		let response = browser.post(&app, "/auth/oauth/start", body).await;
		assert_eq!(response.status(), status);
		assert_eq!(json_body(response).await["error"], code);
	}

	let native = app
		.router
		.clone()
		.oneshot(
			Request::post("/auth/oauth/start")
				.header(header::AUTHORIZATION, "Bearer nothing")
				.header(header::CONTENT_TYPE, "application/json")
				.body(Body::from(json!({ "provider": "company" }).to_string()))
				.unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(json_body(native).await["error"], "unsupported");
}

#[tokio::test]
async fn google_github_microsoft_and_apple_each_work_their_own_way() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let app = app(&db);

	// Google: OpenID Connect at Google's own URLs.
	let mut google = Browser::default();
	let location = google
		.through(
			&app,
			"google",
			false,
			person("g-1", "grace@example.com", true),
		)
		.await;
	assert_eq!(outcome(&location), ProviderOutcome::SignedIn);

	// GitHub: OAuth 2, then the verified primary address from its API.
	let mut github = Browser::default();
	let claims = json!({
		"user": { "id": 42, "login": "linus", "name": null },
		"emails": [
			{ "email": "old@example.com", "primary": false, "verified": true },
			{ "email": "linus@example.com", "primary": true, "verified": true },
		],
	});
	let location = github.through(&app, "github", false, claims).await;
	assert_eq!(outcome(&location), ProviderOutcome::SignedIn);
	let account = account_of(&mut github, &app).await;
	let identity = Identity::query()
		.filter(|identity| identity.account_id.eq(account))
		.first(&db.system("test"))
		.await
		.unwrap()
		.unwrap();
	assert_eq!(identity.subject, "42");
	assert_eq!(identity.name.as_deref(), Some("linus"));

	assert!(identity.email_verified);

	// Microsoft: the email claim is never vouched for, so it isn't stored
	// as the account's, and can't collide with anyone's.
	let mut microsoft = Browser::default();
	let claims = json!({
		"sub": "m-1",
		"tid": "72f988bf-86f1-41af-91ab-2d7cd011db47",
		"email": "grace@example.com",
		"email_verified": true,
	});
	let location = microsoft.through(&app, "microsoft", false, claims).await;
	assert_eq!(outcome(&location), ProviderOutcome::SignedIn);
	let account = account_of(&mut microsoft, &app).await;
	let emails = EmailAddress::query()
		.filter(|email| email.account_id.eq(account))
		.count(&db.system("test"))
		.await
		.unwrap();
	assert_eq!(emails, 0);

	// Apple posts its answer from its own site. That post only gets a page
	// that posts it again, from this site, where the binding cookie goes.
	let mut apple = Browser::default();
	let started = apple
		.post(
			&app,
			"/auth/oauth/start",
			json!({ "provider": "apple", "redirect_to": "/home" }),
		)
		.await;
	let url = json_body(started).await["url"].as_str().unwrap().to_owned();
	let mut back = app.idp.authorize(
		&url,
		json!({ "sub": "a-1", "email": "relay@privaterelay.appleid.com", "email_verified": "true" }),
	);
	back.push((
		"user".into(),
		r#"{"name":{"firstName":"Ada","lastName":"Lovelace"}}"#.into(),
	));
	let form = url::form_urlencoded::Serializer::new(String::new())
		.extend_pairs(&back)
		.finish();
	let cross_site = apple
		.send(
			&app,
			Request::post("/auth/callback/apple")
				.header(header::ORIGIN, "https://appleid.apple.com")
				.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded"),
			Body::from(form.clone()),
		)
		.await;
	assert_eq!(cross_site.status(), StatusCode::OK);

	let policy = cross_site.headers()[header::CONTENT_SECURITY_POLICY]
		.to_str()
		.unwrap()
		.to_owned();
	assert!(
		policy.starts_with("default-src 'none'; script-src 'sha256-"),
		"{policy}"
	);
	let page = text(cross_site).await;
	assert!(page.contains("action=\"/auth/callback/apple\""));
	assert!(page.contains("&quot;firstName&quot;"), "values are escaped");
	assert!(!apple.signed_in());

	let same_site = apple
		.send(
			&app,
			Request::post("/auth/callback/apple")
				.header(header::ORIGIN, ORIGIN)
				.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded"),
			Body::from(form),
		)
		.await;
	assert_eq!(same_site.status(), StatusCode::SEE_OTHER);
	assert!(apple.signed_in());
	let account = account_of(&mut apple, &app).await;
	let identity = Identity::query()
		.filter(|identity| identity.account_id.eq(account))
		.first(&db.system("test"))
		.await
		.unwrap()
		.unwrap();
	assert_eq!(identity.name.as_deref(), Some("Ada Lovelace"));
	assert!(identity.email_verified, "Apple's \"true\" is a string");
}

#[tokio::test]
async fn an_address_another_account_has_never_signs_in_to_it() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let app = app(&db);
	let mut owner = Browser::default();
	email_sign_in(&app, &mut owner, "ada@example.com").await;
	let owners = account_of(&mut owner, &app).await;

	let mut stranger = Browser::default();
	let location = stranger
		.through(
			&app,
			"google",
			false,
			person("g-ada", "ada@example.com", true),
		)
		.await;
	assert_eq!(outcome(&location), ProviderOutcome::EmailInUse);
	assert!(!stranger.signed_in());
	assert_eq!(
		Account::query().count(&db.system("test")).await.unwrap(),
		1,
		"nothing is created until they choose"
	);

	// Choosing a separate account makes one, without the address.
	let response = stranger.post(&app, "/auth/oauth/continue", json!({})).await;
	assert_eq!(response.status(), StatusCode::OK);
	assert!(stranger.signed_in());
	let separate = account_of(&mut stranger, &app).await;
	assert_ne!(separate, owners);
	let emails = EmailAddress::query()
		.filter(|email| email.account_id.eq(separate))
		.count(&db.system("test"))
		.await
		.unwrap();
	assert_eq!(emails, 0);
	let again = stranger.post(&app, "/auth/oauth/continue", json!({})).await;

	assert_eq!(json_body(again).await["error"], "sign_in_expired");
}

#[tokio::test]
async fn connecting_disconnecting_and_merging_accounts() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db);

		// Someone signed up with GitHub months ago.
		let mut old = Browser::default();
		let github = json!({
			"user": { "id": 7, "login": "ada-g", "name": "Ada" },
			"emails": [{ "email": "ada@work.example", "primary": true, "verified": true }],
		});
		old.through(&app, "github", false, github.clone()).await;
		let absorbed = account_of(&mut old, &app).await;

		// Today they sign up with email, and connect Google, then GitHub.
		let mut browser = Browser::default();
		email_sign_in(&app, &mut browser, "ada@example.com").await;
		let survivor = account_of(&mut browser, &app).await;
		let location = browser
			.through(
				&app,
				"google",
				true,
				person("g-1", "ada@gmail.example", true),
			)
			.await;
		assert_eq!(location, "/home?auth=linked");
		assert!(
			app.mailbox
				.latest_to("ada@example.com")
				.unwrap()
				.subject
				.contains("Google was connected")
		);
		let location = browser
			.through(
				&app,
				"google",
				true,
				person("g-1", "ada@gmail.example", true),
			)
			.await;
		assert_eq!(outcome(&location), ProviderOutcome::AlreadyLinked);

		let location = browser.through(&app, "github", true, github.clone()).await;
		assert_eq!(outcome(&location), ProviderOutcome::Merge);

		let offer: Option<MergeOffer> =
			serde_json::from_value(json_body(browser.get(&app, "/auth/merge").await).await)
				.unwrap();
		let offer = offer.unwrap();
		assert_eq!(offer.provider, "github");
		assert_eq!(offer.emails, ["ada@work.example"]);
		assert_eq!(offer.identities, ["GitHub"]);
		assert_eq!(offer.second_factor, [] as [Factor; 0]);

		// Not without an explicit go-ahead.
		let refused = browser
			.post(
				&app,
				"/auth/merge",
				json!({ "mode": "move_login", "confirm": false }),
			)
			.await;
		assert_eq!(json_body(refused).await["error"], "unsupported");

		let merged = browser
			.post(
				&app,
				"/auth/merge",
				json!({ "mode": "move_login", "confirm": true }),
			)
			.await;
		assert_eq!(merged.status(), StatusCode::NO_CONTENT);
		let identities: Vec<IdentityInfo> =
			serde_json::from_value(json_body(browser.get(&app, "/auth/identities").await).await)
				.unwrap();
		assert_eq!(identities.len(), 2);
		let system = db.system("test");
		assert!(Account::find(&system, absorbed).await.unwrap().is_none());
		assert_eq!(app.auth.resolve_merged(absorbed).await.unwrap(), survivor);

		assert_eq!(app.auth.resolve_merged(survivor).await.unwrap(), survivor);
		let moved = EmailAddress::query()
			.filter(|email| email.account_id.eq(survivor))
			.count(&system)
			.await
			.unwrap();
		assert_eq!(moved, 3, "email, gmail, and work addresses");
		assert!(!old.signed_in() || account_of_or_none(&mut old, &app).await.is_none());

		// GitHub now signs in here; the offer is gone.
		let mut later = Browser::default();
		later.through(&app, "github", false, github).await;
		assert_eq!(account_of(&mut later, &app).await, survivor);
		let none = json_body(browser.get(&app, "/auth/merge").await).await;
		assert_eq!(none, Value::Null);

		// Disconnecting keeps a way to sign in.
		let google = identities
			.iter()
			.find(|identity| identity.provider == "google")
			.unwrap();
		let response = browser
			.post(&app, "/auth/identities/unlink", json!({ "id": google.id }))
			.await;
		assert_eq!(response.status(), StatusCode::NO_CONTENT);
		let response = browser
			.post(&app, "/auth/identities/unlink", json!({ "id": google.id }))
			.await;
		assert_eq!(json_body(response).await["error"], "not_found");
	}
}

async fn account_of_or_none(browser: &mut Browser, app: &App) -> Option<Id<Account>> {
	let session: SessionResponse =
		serde_json::from_value(json_body(browser.get(app, "/auth/session").await).await).unwrap();
	session.session.map(|session| session.account_id)
}

#[tokio::test]
async fn the_last_way_to_sign_in_stays_and_merges_respect_two_step() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let app = app(&db);

	// An account with only Microsoft, whose address isn't kept, can't drop
	// it.
	let mut only = Browser::default();
	only.through(
		&app,
		"microsoft",
		false,
		json!({ "sub": "m-9", "tid": "72f988bf-86f1-41af-91ab-2d7cd011db47" }),
	)
	.await;
	let identities: Vec<IdentityInfo> =
		serde_json::from_value(json_body(only.get(&app, "/auth/identities").await).await).unwrap();
	let response = only
		.post(
			&app,
			"/auth/identities/unlink",
			json!({ "id": identities[0].id }),
		)
		.await;
	assert_eq!(json_body(response).await["error"], "last_sign_in_method");

	// The other account turns on an authenticator; merging it then needs a
	// code from it.
	let mut other = Browser::default();
	email_sign_in(&app, &mut other, "grace@example.com").await;
	let setup = json_body(other.post(&app, "/auth/totp", json!({})).await).await;
	let secret = setup["secret"].as_str().unwrap().to_owned();
	let confirmed = other
		.post(
			&app,
			"/auth/totp/confirm",
			json!({ "code": totp::totp(&secret, 0) }),
		)
		.await;
	assert_eq!(confirmed.status(), StatusCode::OK);
	let linked = other
		.through(
			&app,
			"google",
			true,
			person("g-grace", "grace@gmail.example", true),
		)
		.await;
	assert_eq!(outcome(&linked), ProviderOutcome::Linked);

	// Signing in with Google now asks for the second factor.
	let mut grace = Browser::default();
	let location = grace
		.through(
			&app,
			"google",
			false,
			person("g-grace", "grace@gmail.example", true),
		)
		.await;
	assert_eq!(
		outcome(&location),
		ProviderOutcome::SecondFactor(vec![Factor::Totp, Factor::RecoveryCode])
	);
	assert!(grace.jar.contains_key("__Host-second-factor"));
	assert!(!grace.signed_in());

	let mut ada = Browser::default();
	email_sign_in(&app, &mut ada, "ada@example.com").await;
	let location = ada
		.through(
			&app,
			"google",
			true,
			person("g-grace", "grace@gmail.example", true),
		)
		.await;
	assert_eq!(outcome(&location), ProviderOutcome::Merge);
	let offer = json_body(ada.get(&app, "/auth/merge").await).await;
	assert_eq!(offer["second_factor"], json!(["totp", "recovery_code"]));

	let without = ada
		.post(
			&app,
			"/auth/merge",
			json!({ "mode": "move_login", "confirm": true }),
		)
		.await;
	assert_eq!(json_body(without).await["error"], "step_up_required");
	let wrong = ada
		.post(
			&app,
			"/auth/merge",
			json!({ "mode": "move_login", "confirm": true, "code": "000000" }),
		)
		.await;
	assert_eq!(json_body(wrong).await["error"], "invalid_code");
	let right = ada
		.post(
			&app,
			"/auth/merge",
			json!({ "mode": "move_login", "confirm": true, "code": totp::totp(&secret, 1) }),
		)
		.await;
	assert_eq!(
		right.status(),
		StatusCode::NO_CONTENT,
		"{}",
		text(right).await
	);

	// Cancelling an offer that's gone is fine; an absorbed account has no
	// offer to make.
	let response = ada.post(&app, "/auth/merge/cancel", json!({})).await;
	assert_eq!(response.status(), StatusCode::NO_CONTENT);
}
