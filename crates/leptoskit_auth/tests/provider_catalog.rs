//! Every built-in provider beyond the first four, through the real routes
//! against the fake identity provider at the provider's own URLs: who signs
//! up, which address is trusted, and what each does differently, such as
//! HTTP Basic client authentication or a missing nonce.

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
use leptoskit_auth::Auth;
use leptoskit_auth::EmailAddress;
use leptoskit_auth::Identity;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::Provider;
use leptoskit_auth::ProviderOutcome;
use leptoskit_auth::SessionResponse;
use leptoskit_db::Db;
use oidc::FakeIdp;
use oidc::SECRET;
use oidc::Tamper;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt;

const ORIGIN: &str = "https://notes.example";

struct App {
	router: Router,
	idp: FakeIdp,
}

fn app(db: &Db) -> App {
	let idp = FakeIdp::new();
	let auth = Auth::builder(db, Keys::single("test", [5; 32]))
		.app_name("Notes")
		.origin(ORIGIN)
		.mailer(Mailbox::new())
		.provider(Provider::facebook(
			"facebook-client",
			SECRET,
			leptoskit_auth::FACEBOOK_GRAPH_VERSION,
		))
		.provider(Provider::x("x-client", SECRET))
		.provider(Provider::discord("discord-client", SECRET))
		.provider(Provider::linkedin("linkedin-client", SECRET))
		.provider(
			Provider::gitlab(
				"gitlab-client",
				Some(SECRET.into()),
				leptoskit_auth::GITLAB_COM,
			)
			.unwrap(),
		)
		.provider(Provider::slack("slack-client", SECRET))
		.provider_http(idp.clone())
		.build()
		.unwrap();
	App {
		router: Router::new().merge(auth.router()).layer(auth.layer()),
		idp,
	}
}

/// A browser's cookie jar.
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

	/// Signs in with `provider` as someone with `claims`, and returns the
	/// outcome and the authorization URL the app sent the browser to.
	async fn sign_in(
		&mut self,
		app: &App,
		provider: &str,
		claims: Value,
	) -> (ProviderOutcome, String) {
		let started = self
			.send(
				app,
				Request::post("/auth/oauth/start")
					.header(header::ORIGIN, ORIGIN)
					.header(header::CONTENT_TYPE, "application/json"),
				Body::from(json!({ "provider": provider }).to_string()),
			)
			.await;
		assert_eq!(started.status(), StatusCode::OK, "{provider}");
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
		assert_eq!(response.status(), StatusCode::SEE_OTHER);
		let location = response.headers()[header::LOCATION].to_str().unwrap();
		let query = location.split_once('?').unwrap().1;
		(ProviderOutcome::from_query(query).unwrap(), url)
	}

	async fn account(&mut self, app: &App) -> leptoskit_db::Id<leptoskit_auth::Account> {
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

fn query(url: &str) -> HashMap<String, String> {
	url::Url::parse(url)
		.unwrap()
		.query_pairs()
		.into_owned()
		.collect()
}

/// One provider's fixture: who signs in there, and what the account here
/// should end up with.
struct Fixture {
	provider: &'static str,
	claims: Value,
	/// The identity's subject.
	subject: &'static str,
	/// Its name, or username when it has no name.
	name: &'static str,
	/// The address the account gets: only one the provider vouched for.
	trusted: Option<&'static str>,
}

fn fixtures() -> Vec<Fixture> {
	vec![
		Fixture {
			provider: "facebook",
			claims: json!({ "user": {
				"id": "10150", "name": "Ada Lovelace", "email": "ada@facebook.example",
				"picture": { "data": { "url": "https://graph.example/ada.jpg" } },
			}}),
			subject: "10150",
			name: "Ada Lovelace",
			trusted: None,
		},
		Fixture {
			provider: "x",
			claims: json!({ "user": { "data": {
				"id": "2244994945", "name": "Grace", "username": "grace",
			}}}),
			subject: "2244994945",
			name: "Grace",
			trusted: None,
		},
		Fixture {
			provider: "discord",
			claims: json!({ "user": {
				"id": "80351110224678912", "username": "linus", "global_name": null,
				"email": "linus@discord.example", "verified": true,
			}}),
			subject: "80351110224678912",
			name: "linus",
			trusted: Some("linus@discord.example"),
		},
		Fixture {
			provider: "linkedin",
			claims: json!({
				"sub": "782bbtaQ", "name": "Hedy Lamarr",
				"email": "hedy@linkedin.example", "email_verified": true,
			}),
			subject: "782bbtaQ",
			name: "Hedy Lamarr",
			trusted: Some("hedy@linkedin.example"),
		},
		Fixture {
			provider: "gitlab",
			claims: json!({
				"sub": "1234", "preferred_username": "margaret",
				"email": "margaret@gitlab.example", "email_verified": false,
			}),
			subject: "1234",
			name: "margaret",
			trusted: None,
		},
		Fixture {
			provider: "slack",
			claims: json!({
				"sub": "U0R7JM", "https://slack.com/team_id": "T0R7GR", "name": "Katherine",
				"email": "katherine@slack.example", "email_verified": true,
			}),
			subject: "T0R7GR:U0R7JM",
			name: "Katherine",
			trusted: Some("katherine@slack.example"),
		},
	]
}

#[tokio::test]
async fn each_provider_signs_people_up_with_only_the_addresses_it_vouches_for() {
	let cluster = PostgresCluster::start();
	let mut databases = vec![Db::connect("sqlite::memory:").await.unwrap()];

	if let Some(cluster) = &cluster {
		databases.push(Db::connect(&cluster.database()).await.unwrap());
	}

	for db in databases {
		leptoskit_db::testing::create_tables(&db).await.unwrap();
		let app = app(&db);
		let system = db.system("test");

		for fixture in fixtures() {
			let provider = fixture.provider;
			let mut browser = Browser::default();
			let (outcome, _) = browser
				.sign_in(&app, provider, fixture.claims.clone())
				.await;
			assert_eq!(outcome, ProviderOutcome::SignedIn, "{provider}");
			let account = browser.account(&app).await;
			let identity = Identity::query()
				.filter(|identity| identity.account_id.eq(account))
				.first(&system)
				.await
				.unwrap()
				.unwrap();
			assert_eq!(identity.provider, provider);
			assert_eq!(identity.subject, fixture.subject, "{provider}");
			assert_eq!(identity.name.as_deref(), Some(fixture.name), "{provider}");
			assert_eq!(
				identity.email_verified,
				fixture.trusted.is_some(),
				"{provider}"
			);
			let emails: Vec<String> = EmailAddress::query()
				.filter(|email| email.account_id.eq(account))
				.all(&system)
				.await
				.unwrap()
				.into_iter()
				.map(|email| email.address.to_string())
				.collect();
			assert_eq!(
				emails,
				fixture
					.trusted
					.into_iter()
					.map(str::to_owned)
					.collect::<Vec<_>>(),
				"{provider}"
			);

			// The same person again signs in to the same account.
			let mut again = Browser::default();
			let (outcome, _) = again.sign_in(&app, provider, fixture.claims).await;
			assert_eq!(outcome, ProviderOutcome::SignedIn, "{provider}");
			assert_eq!(again.account(&app).await, account, "{provider}");
		}
	}
}

#[tokio::test]
async fn each_provider_is_asked_the_way_it_expects() {
	let db = leptoskit_db::testing::sqlite().await;
	let app = app(&db);
	let expectations = [
		(
			"facebook",
			"https://www.facebook.com/v26.0/dialog/oauth",
			"public_profile email",
		),
		(
			"x",
			"https://x.com/i/oauth2/authorize",
			"tweet.read users.read",
		),
		(
			"discord",
			"https://discord.com/oauth2/authorize",
			"identify email",
		),
		(
			"linkedin",
			"https://www.linkedin.com/oauth/v2/authorization",
			"openid profile email",
		),
		(
			"gitlab",
			"https://gitlab.com/oauth/authorize",
			"openid profile email",
		),
		(
			"slack",
			"https://slack.com/openid/connect/authorize",
			"openid email profile",
		),
	];

	for (fixture, (provider, authorize, scope)) in fixtures().into_iter().zip(expectations) {
		let (_, url) = Browser::default()
			.sign_in(&app, provider, fixture.claims)
			.await;
		assert!(url.starts_with(&format!("{authorize}?")), "{url}");
		let query = query(&url);
		assert_eq!(query["scope"], scope, "{provider}");
		assert_eq!(query["code_challenge_method"], "S256", "{provider}");
		assert!(!query.contains_key("prompt"), "{provider}");
	}

	// The fake refuses a client that authenticates any other way than its
	// provider wants, so these requests reached it as X and Facebook ask.
	let requests = app.idp.requests();
	assert!(requests.contains(&"POST https://api.x.com/2/oauth2/token".to_owned()));
	assert!(requests.contains(
		&"GET https://graph.facebook.com/v26.0/me?fields=id,name,email,picture".to_owned()
	));
}

#[tokio::test]
async fn linkedin_may_leave_out_the_nonce_but_never_send_a_wrong_one() {
	let db = leptoskit_db::testing::sqlite().await;
	let app = app(&db);
	let person = json!({ "sub": "abc", "name": "Ada" });

	app.idp.tamper(Tamper::Nonce);
	let (outcome, _) = Browser::default()
		.sign_in(&app, "linkedin", person.clone())
		.await;
	assert_eq!(outcome, ProviderOutcome::Failed("provider".into()));

	// Every other provider that issues ID tokens needs it.
	app.idp.tamper(Tamper::NoNonce);
	let (outcome, _) = Browser::default()
		.sign_in(
			&app,
			"slack",
			json!({ "sub": "U1", "https://slack.com/team_id": "T1" }),
		)
		.await;
	assert_eq!(outcome, ProviderOutcome::Failed("provider".into()));

	app.idp.tamper(Tamper::Nothing);
	let (outcome, _) = Browser::default().sign_in(&app, "linkedin", person).await;
	assert_eq!(outcome, ProviderOutcome::SignedIn);
}

#[tokio::test]
async fn a_slack_user_is_one_identity_per_workspace() {
	let db = leptoskit_db::testing::sqlite().await;
	let app = app(&db);

	let mut first = Browser::default();
	let (outcome, _) = first
		.sign_in(
			&app,
			"slack",
			json!({ "sub": "U1", "https://slack.com/team_id": "T1" }),
		)
		.await;
	assert_eq!(outcome, ProviderOutcome::SignedIn);
	let mut second = Browser::default();
	second
		.sign_in(
			&app,
			"slack",
			json!({ "sub": "U1", "https://slack.com/team_id": "T2" }),
		)
		.await;
	assert_ne!(first.account(&app).await, second.account(&app).await);

	// A token that doesn't name its workspace can't say who it is.
	let (outcome, _) = Browser::default()
		.sign_in(&app, "slack", json!({ "sub": "U1" }))
		.await;
	assert_eq!(outcome, ProviderOutcome::Failed("provider".into()));
}

#[tokio::test]
async fn a_profile_without_a_usable_id_signs_no_one_in() {
	let db = leptoskit_db::testing::sqlite().await;
	let app = app(&db);

	for user in [
		json!({ "username": "nobody" }),
		json!({ "id": "" }),
		json!({ "id": true }),
	] {
		let (outcome, _) = Browser::default()
			.sign_in(&app, "discord", json!({ "user": user }))
			.await;
		assert_eq!(
			outcome,
			ProviderOutcome::Failed("provider".into()),
			"{user}"
		);
	}

	// An unverified address is kept on the identity, never on the account.
	let mut browser = Browser::default();
	browser
		.sign_in(
			&app,
			"discord",
			json!({ "user": { "id": 7, "username": "eve", "email": "eve@example.com", "verified": false } }),
		)
		.await;
	let account = browser.account(&app).await;
	let system = db.system("test");
	let identity = Identity::query()
		.filter(|identity| identity.account_id.eq(account))
		.first(&system)
		.await
		.unwrap()
		.unwrap();
	assert_eq!(identity.subject, "7", "numeric IDs become text");
	assert_eq!(identity.email.as_deref(), Some("eve@example.com"));
	assert!(!identity.email_verified);
}
