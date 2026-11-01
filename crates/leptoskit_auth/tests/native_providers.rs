//! Third-party sign-in from a native app, through the real routes and a
//! fake identity provider: the system browser comes back to this site,
//! which sends it on to the app with the provider's answer; the app
//! finishes with the binding only it holds. Also Sign in with Apple on the
//! device, which answers with an ID token for the app's bundle ID.

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
use leptoskit_auth::Auth;
use leptoskit_auth::Factor;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::NativeApps;
use leptoskit_auth::Provider;
use leptoskit_auth::ProviderFinished;
use leptoskit_auth::ProviderOutcome;
use leptoskit_db::Db;
use oidc::FakeIdp;
use oidc::SECRET;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt;

const ORIGIN: &str = "https://notes.example";
const BUNDLE: &str = "com.notes.App";
const LOOPBACK: &str = "http://127.0.0.1:49152/auth/native/return";

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
		.native_apps(NativeApps {
			package_id: BUNDLE.into(),
			apple_team_id: Some("ABCDE12345".into()),
			android_certificates: vec![[7; 32]],
		})
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
		.provider(Provider::apple("com.notes.web", "TEAM1", "KEY1", &pem).unwrap())
		.provider_http(idp.clone())
		.build()
		.unwrap();
	let router = Router::new().merge(auth.router()).layer(auth.layer());
	App {
		router,
		idp,
		mailbox,
	}
}

async fn send(app: &App, request: Request<Body>) -> Response {
	app.router.clone().oneshot(request).await.unwrap()
}

async fn json_body(response: Response) -> Value {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// A native app: no cookies, no `Origin`, and an access token once it has
/// one.
#[derive(Default)]
struct Native {
	access: Option<String>,
}

impl Native {
	async fn post(&self, app: &App, path: &str, body: Value) -> Response {
		let mut request = Request::post(path).header(header::CONTENT_TYPE, "application/json");

		if let Some(access) = &self.access {
			request = request.header(header::AUTHORIZATION, format!("Bearer {access}"));
		}

		send(app, request.body(Body::from(body.to_string())).unwrap()).await
	}

	async fn get(&self, app: &App, path: &str) -> Response {
		let mut request = Request::get(path);

		if let Some(access) = &self.access {
			request = request.header(header::AUTHORIZATION, format!("Bearer {access}"));
		}

		send(app, request.body(Body::empty()).unwrap()).await
	}

	/// Starts a flow in the system browser that returns to `return_to`:
	/// the authorization URL and the binding.
	async fn start(
		&self,
		app: &App,
		path: &str,
		provider: &str,
		return_to: &str,
	) -> (String, String) {
		let started = self
			.post(
				app,
				path,
				json!({
					"provider": provider,
					"native": { "browser": { "return_to": return_to } },
				}),
			)
			.await;
		assert_eq!(started.status(), StatusCode::OK);
		assert!(
			started.headers().get(header::SET_COOKIE).is_none(),
			"a native app's binding is in the body"
		);
		let body = json_body(started).await;
		assert!(body.get("nonce").is_none());
		(
			body["url"].as_str().unwrap().to_owned(),
			body["token"].as_str().unwrap().to_owned(),
		)
	}

	/// Finishes with what the browser came back to the app with.
	async fn finish(&mut self, app: &App, provider: &str, token: &str, returned: &str) -> Response {
		let query: HashMap<String, String> = url::Url::parse(returned)
			.unwrap()
			.query_pairs()
			.into_owned()
			.collect();
		let mut body =
			json!({ "provider": provider, "token": token, "device_name": "Ada's phone" });

		for name in ["state", "code", "error", "user"] {
			if let Some(value) = query.get(name) {
				body[name] = value.clone().into();
			}
		}

		self.post(app, "/auth/oauth/finish", body).await
	}

	async fn signed_in(&mut self, response: Response) -> ProviderFinished {
		let status = response.status();
		let body = json_body(response).await;
		assert_eq!(status, StatusCode::OK, "{body}");
		let finished: ProviderFinished = serde_json::from_value(body).unwrap();

		if let Some(signed_in) = &finished.signed_in {
			self.access = Some(signed_in.tokens.as_ref().unwrap().access_token.clone());
		}

		finished
	}
}

/// The person signs in at the provider in the system browser, which has
/// no cookies of this site's, and the provider sends it back. Returns where
/// this site sends it next.
async fn browse(app: &App, provider: &str, url: &str, claims: Value) -> String {
	let back = app.idp.authorize(url, claims);
	let query = url::form_urlencoded::Serializer::new(String::new())
		.extend_pairs(&back)
		.finish();
	let response = send(
		app,
		Request::get(format!("/auth/callback/{provider}?{query}"))
			.body(Body::empty())
			.unwrap(),
	)
	.await;
	assert_eq!(response.status(), StatusCode::SEE_OTHER);
	assert!(
		response.headers().get(header::SET_COOKIE).is_none(),
		"the system browser never gets a session"
	);
	assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
	response.headers()[header::LOCATION]
		.to_str()
		.unwrap()
		.to_owned()
}

fn person(sub: &str, email: &str) -> Value {
	json!({ "sub": sub, "email": email, "email_verified": true, "name": "Ada Lovelace" })
}

#[tokio::test]
async fn a_native_app_signs_in_through_the_system_browser() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db);
		let mut native = Native::default();
		let (url, token) = native
			.start(&app, "/auth/oauth/start", "company", LOOPBACK)
			.await;
		let query: HashMap<String, String> = url::Url::parse(&url)
			.unwrap()
			.query_pairs()
			.into_owned()
			.collect();
		// The provider sends people back to this site, as for the web, so
		// nothing changes in its setup.
		assert_eq!(
			query["redirect_uri"],
			"https://notes.example/auth/callback/company"
		);
		assert_eq!(query["code_challenge_method"], "S256");
		assert!(query.contains_key("nonce"));

		let returned = browse(&app, "company", &url, person("u-1", "ada@example.com")).await;
		assert!(
			returned.starts_with(&format!("{LOOPBACK}?state=")),
			"{returned}"
		);
		assert!(returned.contains("&code="));

		let finished = native.finish(&app, "company", &token, &returned).await;
		let finished = native.signed_in(finished).await;
		assert_eq!(finished.outcome, ProviderOutcome::SignedIn);
		assert!(finished.signed_in.unwrap().signed_up);

		let devices = json_body(native.get(&app, "/auth/sessions").await).await;
		assert_eq!(devices[0]["device"], "Ada's phone", "{devices}");

		// The answer works once.
		let again = Native::default()
			.finish(&app, "company", &token, &returned)
			.await;
		assert_eq!(json_body(again).await["error"], "sign_in_expired");

		// The same person signs in again on another device, to the same
		// account.
		let mut other = Native::default();
		let (url, token) = other
			.start(&app, "/auth/oauth/start", "company", LOOPBACK)
			.await;
		let returned = browse(&app, "company", &url, person("u-1", "ada@example.com")).await;
		let finished = other.finish(&app, "company", &token, &returned).await;
		let finished = other.signed_in(finished).await;
		assert!(!finished.signed_in.unwrap().signed_up);
	}
}

#[tokio::test]
async fn only_the_app_that_started_a_flow_finishes_it() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let app = app(&db);
	let native = Native::default();

	// Someone sends a person the authorization URL of a flow they started.
	// The person's browser goes on to the return address on the person's
	// own device, and the code is no use without the binding.
	let (url, token) = native
		.start(&app, "/auth/oauth/start", "company", LOOPBACK)
		.await;
	let returned = browse(&app, "company", &url, person("u-2", "eve@example.com")).await;
	let forged = Native::default()
		.finish(&app, "company", "a-guessed-binding", &returned)
		.await;
	assert_eq!(json_body(forged).await["error"], "sign_in_expired");

	// A browser's flow doesn't finish in an app, and an app's doesn't
	// finish in a browser.
	let started = send(
		&app,
		Request::post("/auth/oauth/start")
			.header(header::ORIGIN, ORIGIN)
			.header(header::CONTENT_TYPE, "application/json")
			.body(Body::from(json!({ "provider": "company" }).to_string()))
			.unwrap(),
	)
	.await;
	let cookie = started.headers()[header::SET_COOKIE]
		.to_str()
		.unwrap()
		.split(';')
		.next()
		.unwrap()
		.to_owned();
	let binding = cookie.split_once('=').unwrap().1.to_owned();
	let web_url = json_body(started).await["url"].as_str().unwrap().to_owned();
	let back = app
		.idp
		.authorize(&web_url, person("u-3", "bob@example.com"));
	let web_returned = format!(
		"{LOOPBACK}?{}",
		url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs(&back)
			.finish()
	);
	let crossed = Native::default()
		.finish(&app, "company", &binding, &web_returned)
		.await;
	assert_eq!(json_body(crossed).await["error"], "sign_in_expired");

	// Another provider's answer for this flow is refused too.
	let (url, token_two) = native
		.start(&app, "/auth/oauth/start", "company", LOOPBACK)
		.await;
	let returned_two = browse(&app, "company", &url, person("u-4", "cy@example.com")).await;
	let misdirected = Native::default()
		.finish(&app, "google", &token_two, &returned_two)
		.await;
	assert_eq!(json_body(misdirected).await["error"], "sign_in_expired");

	// The real app still finishes its own first flow.
	let mut real = Native::default();
	let finished = real.finish(&app, "company", &token, &returned).await;
	assert_eq!(
		real.signed_in(finished).await.outcome,
		ProviderOutcome::SignedIn
	);
}

#[tokio::test]
async fn return_addresses_are_the_apps_own() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let app = app(&db);
	let native = Native::default();

	for allowed in [
		LOOPBACK,
		"http://[::1]:8000/auth/native/return",
		"https://notes.example/auth/native/return",
		"com.notes.app:/auth/native/return",
	] {
		native
			.start(&app, "/auth/oauth/start", "company", allowed)
			.await;
	}

	for refused in [
		"https://evil.example/auth/native/return",
		"https://notes.example/auth/native/elsewhere",
		"https://notes.example/auth/native/return?next=/",
		"https://notes.example/auth/native/return#x",
		"https://user@notes.example/auth/native/return",
		"http://localhost:49152/auth/native/return",
		"http://127.0.0.1/auth/native/return",
		"http://192.168.1.2:80/auth/native/return",
		"com.other.app:/auth/native/return",
		"com.notes.app://auth/native/return",
		"javascript:/auth/native/return",
		"not a url",
	] {
		let started = native
			.post(
				&app,
				"/auth/oauth/start",
				json!({ "provider": "company", "native": { "browser": { "return_to": refused } } }),
			)
			.await;
		assert_eq!(started.status(), StatusCode::BAD_REQUEST, "{refused}");
		assert_eq!(
			json_body(started).await["error"],
			"invalid_redirect",
			"{refused}"
		);
	}
}

#[tokio::test]
async fn refusals_second_factors_and_taken_addresses_reach_the_app() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db);

		// The person says no at the provider.
		let mut native = Native::default();
		let (url, token) = native
			.start(&app, "/auth/oauth/start", "company", LOOPBACK)
			.await;
		let state = url::Url::parse(&url)
			.unwrap()
			.query_pairs()
			.find(|(name, _)| name == "state")
			.unwrap()
			.1
			.into_owned();
		let response = send(
			&app,
			Request::get(format!(
				"/auth/callback/company?state={state}&error=access_denied"
			))
			.body(Body::empty())
			.unwrap(),
		)
		.await;
		let returned = response.headers()[header::LOCATION]
			.to_str()
			.unwrap()
			.to_owned();
		assert!(returned.ends_with("&error=access_denied"), "{returned}");
		let refused = native.finish(&app, "company", &token, &returned).await;
		assert_eq!(json_body(refused).await["error"], "cancelled");

		// An account with an authenticator app asks for it next, and the
		// app finishes with its token.
		let mut first = Native::default();
		let (url, token) = first
			.start(&app, "/auth/oauth/start", "company", LOOPBACK)
			.await;
		let returned = browse(&app, "company", &url, person("u-5", "dee@example.com")).await;
		let finished = first.finish(&app, "company", &token, &returned).await;
		first.signed_in(finished).await;
		let setup = json_body(first.post(&app, "/auth/totp", json!({})).await).await;
		let secret = setup["secret"].as_str().unwrap().to_owned();
		let confirmed = first
			.post(
				&app,
				"/auth/totp/confirm",
				json!({ "code": totp::totp(&secret, 0) }),
			)
			.await;

		assert_eq!(confirmed.status(), StatusCode::OK);

		let mut second = Native::default();
		let (url, token) = second
			.start(&app, "/auth/oauth/start", "company", LOOPBACK)
			.await;
		let returned = browse(&app, "company", &url, person("u-5", "dee@example.com")).await;
		let finished = second.finish(&app, "company", &token, &returned).await;
		let finished = second.signed_in(finished).await;
		assert_eq!(
			finished.outcome,
			ProviderOutcome::SecondFactor(vec![Factor::Totp, Factor::RecoveryCode])
		);
		let step = finished.second_factor.unwrap();
		assert!(finished.signed_in.is_none());

		let completed = second
			.post(
				&app,
				"/auth/second-factor",
				json!({
					"code": totp::totp(&secret, 1),
					"token": step.token,
					"transport": "bearer",
				}),
			)
			.await;
		assert_eq!(completed.status(), StatusCode::OK);
		assert!(json_body(completed).await["tokens"]["access_token"].is_string());

		// A provider that vouches for an address another account has
		// creates nothing, until the person chooses a separate account.
		let mut rival = Native::default();
		let (url, token) = rival
			.start(&app, "/auth/oauth/start", "google", LOOPBACK)
			.await;
		let returned = browse(&app, "google", &url, person("g-5", "dee@example.com")).await;
		let finished = rival.finish(&app, "google", &token, &returned).await;
		let finished = rival.signed_in(finished).await;
		assert_eq!(finished.outcome, ProviderOutcome::EmailInUse);
		let wrong = rival
			.post(&app, "/auth/oauth/continue", json!({ "token": "nope" }))
			.await;
		assert_eq!(json_body(wrong).await["error"], "sign_in_expired");

		let continued = rival
			.post(
				&app,
				"/auth/oauth/continue",
				json!({ "token": token, "device_name": "Dee's tablet" }),
			)
			.await;
		assert_eq!(continued.status(), StatusCode::OK);
		let body = json_body(continued).await;
		assert!(body["signed_up"].as_bool().unwrap());
		assert!(body["tokens"]["refresh_token"].is_string());
	}
}

#[tokio::test]
async fn a_signed_in_app_connects_a_provider() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let app = app(&db);

	let mut native = Native::default();
	let (url, token) = native
		.start(&app, "/auth/oauth/start", "company", LOOPBACK)
		.await;
	let returned = browse(&app, "company", &url, person("u-6", "fay@example.com")).await;
	let finished = native.finish(&app, "company", &token, &returned).await;
	native.signed_in(finished).await;

	// Connecting needs the session.
	let anonymous = Native::default()
		.post(
			&app,
			"/auth/oauth/link",
			json!({ "provider": "google", "native": { "browser": { "return_to": LOOPBACK } } }),
		)
		.await;
	assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

	let (url, token) = native
		.start(&app, "/auth/oauth/link", "google", LOOPBACK)
		.await;
	let returned = browse(&app, "google", &url, person("g-6", "fay@gmail.example")).await;
	let finished = native.finish(&app, "google", &token, &returned).await;
	let finished: ProviderFinished = serde_json::from_value(json_body(finished).await).unwrap();
	assert_eq!(finished.outcome, ProviderOutcome::Linked);
	let identities = json_body(native.get(&app, "/auth/identities").await).await;
	assert_eq!(identities.as_array().unwrap().len(), 2);
	assert!(
		app.mailbox
			.latest_to("fay@example.com")
			.unwrap()
			.subject
			.contains("was connected")
	);
}

#[tokio::test]
async fn apples_posted_answer_goes_straight_on_to_the_app() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let app = app(&db);
	let mut native = Native::default();
	let (url, token) = native
		.start(
			&app,
			"/auth/oauth/start",
			"apple",
			"com.notes.app:/auth/native/return",
		)
		.await;
	let mut back = app
		.idp
		.authorize(&url, person("a-1", "gus@privaterelay.example"));
	back.push((
		"user".into(),
		json!({ "name": { "firstName": "Gus", "lastName": "Grissom" } }).to_string(),
	));
	let form = url::form_urlencoded::Serializer::new(String::new())
		.extend_pairs(&back)
		.finish();

	// Apple posts from its own site, with no cookie of this site's.
	let response = send(
		&app,
		Request::post("/auth/callback/apple")
			.header(header::ORIGIN, "https://appleid.apple.com")
			.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
			.body(Body::from(form))
			.unwrap(),
	)
	.await;
	assert_eq!(response.status(), StatusCode::SEE_OTHER);
	let returned = response.headers()[header::LOCATION]
		.to_str()
		.unwrap()
		.to_owned();
	assert!(returned.starts_with("com.notes.app:/auth/native/return?state="));
	let finished = native.finish(&app, "apple", &token, &returned).await;
	let finished = native.signed_in(finished).await;
	assert_eq!(finished.outcome, ProviderOutcome::SignedIn);
	let identities = json_body(native.get(&app, "/auth/identities").await).await;

	assert_eq!(identities[0]["name"], "Ada Lovelace");
}

#[tokio::test]
async fn sign_in_with_apple_on_the_device_checks_its_id_token() {
	let cluster = PostgresCluster::start();

	for db in databases(cluster.as_ref()).await {
		let app = app(&db);
		let start = async |provider: &str| {
			let started = Native::default()
				.post(
					&app,
					"/auth/oauth/start",
					json!({ "provider": provider, "native": "apple_id" }),
				)
				.await;
			json_body(started).await
		};
		let finish = async |token: &str, id_token: String, user: Option<&str>| {
			let mut body = json!({
				"provider": "apple",
				"token": token,
				"id_token": id_token,
				"device_name": "Gus's iPhone",
			});

			if let Some(user) = user {
				body["user"] = user.into();
			}

			Native::default()
				.post(&app, "/auth/oauth/finish", body)
				.await
		};

		// Only Apple signs in on the device.
		let other = start("google").await;
		assert_eq!(other["error"], "unsupported");

		// An ID token for the website's Services ID, or with another nonce,
		// is refused, and the flow is used up.
		for (audience, nonce) in [("com.notes.web", None), (BUNDLE, Some("someone-elses"))] {
			let started = start("apple").await;
			let token = started["token"].as_str().unwrap();
			let id_token = app.idp.apple_id_token(
				audience,
				nonce.unwrap_or(started["nonce"].as_str().unwrap()),
				person("a-2", "hal@example.com"),
			);
			let refused = finish(token, id_token.clone(), None).await;
			assert_eq!(json_body(refused).await["error"], "provider");
			let replay = finish(token, id_token, None).await;
			assert_eq!(json_body(replay).await["error"], "sign_in_expired");
		}

		let started = start("apple").await;
		let token = started["token"].as_str().unwrap();
		let nonce = started["nonce"].as_str().unwrap();
		assert_eq!(nonce.len(), 43);
		let id_token = app.idp.apple_id_token(
			BUNDLE,
			nonce,
			json!({ "sub": "a-2", "email": "hal@example.com", "email_verified": "true" }),
		);
		let user = json!({ "name": { "firstName": "Hal", "lastName": "Laning" } }).to_string();
		let mut native = Native::default();
		let finished = finish(token, id_token, Some(&user)).await;

		let finished = native.signed_in(finished).await;
		assert_eq!(finished.outcome, ProviderOutcome::SignedIn);
		let identities = json_body(native.get(&app, "/auth/identities").await).await;
		assert_eq!(identities[0]["provider"], "apple");
		assert_eq!(identities[0]["name"], "Hal Laning");
		assert_eq!(identities[0]["email"], "hal@example.com");
	}
}

// The loopback is the desktop plugin's, which only builds there.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[tokio::test]
async fn a_desktop_app_signs_in_through_its_loopback_address() {
	use std::io::Read;
	use std::io::Write;

	let db = Db::connect("sqlite::memory:").await.unwrap();
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let app = app(&db);
	let loopback = leptoskit_plugin_web_auth::desktop::Loopback::bind().unwrap();
	let return_to = loopback.return_to().to_owned();
	let mut native = Native::default();
	let (url, token) = native
		.start(&app, "/auth/oauth/start", "company", &return_to)
		.await;

	// The browser signs in at the provider, comes back to this site, and
	// follows its redirect to the app's loopback address.
	let location = browse(&app, "company", &url, person("u-9", "ida@example.com")).await;
	let target = url::Url::parse(&location).unwrap();
	let port = target.port().unwrap();
	let path = format!("{}?{}", target.path(), target.query().unwrap());
	let browser = std::thread::spawn(move || {
		let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
		write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
		let mut page = String::new();
		stream.read_to_string(&mut page).unwrap();
		page
	});
	let returned =
		tokio::task::spawn_blocking(move || loopback.wait(std::time::Duration::from_secs(10)))
			.await
			.unwrap()
			.unwrap();
	assert!(browser.join().unwrap().contains("go back to the app"));

	assert_eq!(returned, location);

	let finished = native.finish(&app, "company", &token, &returned).await;
	let finished = native.signed_in(finished).await;
	assert_eq!(finished.outcome, ProviderOutcome::SignedIn);
	let session = json_body(native.get(&app, "/auth/session").await).await;
	assert!(session["session"]["id"].is_string());
}

#[test]
fn the_native_wire_types_print_no_secrets() {
	let redirect = leptoskit_auth::ProviderRedirect {
		url: "https://sso.example/authorize".into(),
		token: Some("binding-secret".into()),
		nonce: Some("nonce-secret".into()),
	};
	let finish = leptoskit_auth::FinishProvider {
		provider: "company".into(),
		token: "binding-secret".into(),
		code: Some("code-secret".into()),
		id_token: Some("token-secret".into()),
		..leptoskit_auth::FinishProvider::default()
	};
	let continued = leptoskit_auth::ContinueProvider {
		token: Some("binding-secret".into()),
		device_name: Some("Ada's phone".into()),
	};

	for printed in [
		format!("{redirect:?}"),
		format!("{finish:?}"),
		format!("{continued:?}"),
	] {
		assert!(!printed.contains("secret"), "{printed}");
	}

	assert!(format!("{redirect:?}").contains("sso.example"));
	assert!(format!("{finish:?}").contains("company"));
}
