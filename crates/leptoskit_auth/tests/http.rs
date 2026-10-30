//! Sign-in over HTTP: the session layer, the `/auth` routes, cookies and
//! bearer tokens, and the same-site check on everything that changes state.

#![cfg(all(feature = "axum", feature = "sqlite"))]

use std::net::SocketAddr;

use axum::Extension;
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use http_body_util::BodyExt;
use leptoskit_auth::Auth;
use leptoskit_auth::Device;
use leptoskit_auth::ErrorResponse;
use leptoskit_auth::Event;
use leptoskit_auth::Keys;
use leptoskit_auth::Limit;
use leptoskit_auth::Limits;
use leptoskit_auth::Mailbox;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::Session;
use leptoskit_auth::SessionResponse;
use leptoskit_auth::SignedIn;
use leptoskit_auth::SignedInResponse;
use leptoskit_auth::Tokens;
use leptoskit_db::Db;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt;

const ORIGIN: &str = "https://notes.example";

/// Adds headers to a request.
type Configure = fn(axum::http::request::Builder) -> axum::http::request::Builder;

struct App {
	router: Router,
	mailbox: Mailbox,
	db: Db,
}

async fn app(
	configure: impl FnOnce(leptoskit_auth::AuthBuilder) -> leptoskit_auth::AuthBuilder,
) -> App {
	let db = leptoskit_db::testing::sqlite().await;
	let mailbox = Mailbox::new();
	let auth = configure(
		Auth::builder(&db, Keys::single("test", [3; 32]))
			.mailer(mailbox.clone())
			.origin(ORIGIN),
	)
	.build()
	.unwrap();
	let router = Router::new()
		.route(
			"/me",
			get(|me: SignedIn| async move { me.account.to_string() }),
		)
		.route(
			"/visible-sessions",
			get(|Extension(db): Extension<Db>| {
				async move { Session::query().count(&db).await.unwrap().to_string() }
			}),
		)
		.route("/change", post(|| async { "changed" }))
		.route(
			"/where",
			get(|request: RequestInfo| async move { format!("{:?}", request.ip) }),
		)
		.merge(auth.router())
		.layer(auth.layer());

	App {
		router,
		mailbox,
		db,
	}
}

/// A request as a browser on this site sends it, with a JSON body or none.
fn browser(method: &str, path: &str, json: bool) -> axum::http::request::Builder {
	let builder = Request::builder()
		.method(method)
		.uri(path)
		.header(header::ORIGIN, ORIGIN);

	if json {
		builder.header(header::CONTENT_TYPE, "application/json")
	} else {
		builder
	}
}

fn with_body(builder: axum::http::request::Builder, body: Option<Value>) -> Request<Body> {
	builder
		.body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
		.unwrap()
}

impl App {
	async fn send(&self, request: Request<Body>) -> Response {
		self.router.clone().oneshot(request).await.unwrap()
	}

	async fn call(
		&self,
		method: &str,
		path: &str,
		body: Option<Value>,
		cookie: Option<&str>,
	) -> Response {
		let mut builder = browser(method, path, body.is_some());

		if let Some(cookie) = cookie {
			builder = builder.header(header::COOKIE, cookie);
		}

		self.send(with_body(builder, body)).await
	}

	/// Signs in with an emailed code, returning the cookie to send back.
	async fn sign_in(&self, address: &str) -> String {
		let sent = self
			.call(
				"POST",
				"/auth/email/send",
				Some(json!({ "email": address })),
				None,
			)
			.await;
		assert_eq!(sent.status(), StatusCode::NO_CONTENT);
		let verified = self
			.call(
				"POST",
				"/auth/email/verify",
				Some(json!({ "email": address, "code": self.code(address) })),
				None,
			)
			.await;
		assert_eq!(verified.status(), StatusCode::OK);
		let cookie = verified.headers()[header::SET_COOKIE]
			.to_str()
			.unwrap()
			.to_owned();
		cookie.split(';').next().unwrap().to_owned()
	}

	fn code(&self, address: &str) -> String {
		let mail = self.mailbox.latest_to(address).unwrap();
		mail.subject.rsplit(' ').next().unwrap().to_owned()
	}
}

async fn body<T: serde::de::DeserializeOwned>(response: Response) -> T {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	serde_json::from_slice(&bytes).unwrap()
}

async fn text(response: Response) -> String {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn a_browser_signs_in_with_a_host_only_cookie_and_signs_out() {
	let app = app(|builder| builder).await;

	app.call(
		"POST",
		"/auth/email/send",
		Some(json!({ "email": "ada@example.com" })),
		None,
	)
	.await;
	let verified = app
		.call(
			"POST",
			"/auth/email/verify",
			Some(json!({ "email": "ada@example.com", "code": app.code("ada@example.com") })),
			None,
		)
		.await;
	assert_eq!(verified.status(), StatusCode::OK);
	let set_cookie = verified.headers()[header::SET_COOKIE]
		.to_str()
		.unwrap()
		.to_owned();
	assert!(set_cookie.starts_with("__Host-session="));
	assert!(set_cookie.ends_with("; Path=/; HttpOnly; SameSite=Lax; Max-Age=7776000; Secure"));
	let signed_in: SignedInResponse = body(verified).await;
	assert!(signed_in.signed_up);
	assert_eq!(signed_in.tokens, None);

	let cookie = set_cookie.split(';').next().unwrap();
	let current: SessionResponse =
		body(app.call("GET", "/auth/session", None, Some(cookie)).await).await;
	assert_eq!(current.session.unwrap().id, signed_in.session.id);
	let me = app.call("GET", "/me", None, Some(cookie)).await;
	assert_eq!(text(me).await, signed_in.session.account_id.to_string());

	let signed_out = app.call("POST", "/auth/sign-out", None, Some(cookie)).await;
	assert_eq!(signed_out.status(), StatusCode::NO_CONTENT);
	assert!(
		signed_out.headers()[header::SET_COOKIE]
			.to_str()
			.unwrap()
			.starts_with("__Host-session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
	);
	let current: SessionResponse =
		body(app.call("GET", "/auth/session", None, Some(cookie)).await).await;
	assert_eq!(current.session, None);

	let refused = app.call("GET", "/me", None, Some(cookie)).await;
	assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
	let error: ErrorResponse = body(refused).await;
	assert_eq!(error.error, "signed_out");
}

/// A request as a native app sends it: no `Origin`, no cookie, and the
/// access token, if it has one, as a bearer token.
fn native(method: &str, path: &str, access: Option<&str>, body: Option<Value>) -> Request<Body> {
	let mut builder = Request::builder()
		.method(method)
		.uri(path)
		.header(header::USER_AGENT, "Notes/1.0 (iPhone; iOS 18.0)");

	if let Some(access) = access {
		builder = builder.header(header::AUTHORIZATION, format!("Bearer {access}"));
	}

	if body.is_some() {
		builder = builder.header(header::CONTENT_TYPE, "application/json");
	}

	with_body(builder, body)
}

impl App {
	/// Signs a native app in, returning its tokens.
	async fn sign_in_native(&self, address: &str) -> Tokens {
		let sent = self
			.send(native(
				"POST",
				"/auth/email/send",
				None,
				Some(json!({ "email": address })),
			))
			.await;
		assert_eq!(sent.status(), StatusCode::NO_CONTENT);
		let verified = self
			.send(native(
				"POST",
				"/auth/email/verify",
				None,
				Some(json!({
					"email": address,
					"code": self.code(address),
					"transport": "bearer",
					"device_name": "Ada's iPhone",
				})),
			))
			.await;
		assert_eq!(verified.status(), StatusCode::OK);
		assert!(verified.headers().get(header::SET_COOKIE).is_none());
		let signed_in: SignedInResponse = body(verified).await;
		signed_in.tokens.unwrap()
	}

	async fn refresh(&self, refresh_token: &str) -> Response {
		self.send(native(
			"POST",
			"/auth/refresh",
			None,
			Some(json!({ "refresh_token": refresh_token })),
		))
		.await
	}

	async fn me(&self, access: &str) -> StatusCode {
		self.send(native("GET", "/me", Some(access), None))
			.await
			.status()
	}
}

#[tokio::test]
async fn a_native_app_signs_in_with_tokens_and_refreshes_them() {
	let app = app(|builder| builder).await;
	let first = app.sign_in_native("ada@example.com").await;
	assert!((890..=900).contains(&first.expires_in));
	assert_ne!(first.access_token, first.refresh_token);
	assert!(!format!("{first:?}").contains(&first.refresh_token));
	assert_eq!(app.me(&first.access_token).await, StatusCode::OK);
	assert_eq!(app.me("made-up").await, StatusCode::UNAUTHORIZED);

	let devices: Vec<Device> = body(
		app.send(native(
			"GET",
			"/auth/sessions",
			Some(&first.access_token),
			None,
		))
		.await,
	)
	.await;
	assert_eq!(devices.len(), 1);
	assert!(devices[0].current);
	assert_eq!(devices[0].session.device, "Ada's iPhone");

	// Trading the refresh token retires the old access token.
	let second: Tokens = body(app.refresh(&first.refresh_token).await).await;
	assert_eq!(app.me(&second.access_token).await, StatusCode::OK);
	assert_eq!(app.me(&first.access_token).await, StatusCode::UNAUTHORIZED);

	// The response was lost, so the app tries its old refresh token again.
	// Within the grace it still works, and the pair it never got stops.
	let retried: Tokens = body(app.refresh(&first.refresh_token).await).await;
	assert_eq!(app.me(&second.access_token).await, StatusCode::UNAUTHORIZED);
	let unused = app.refresh(&second.refresh_token).await;
	assert_eq!(unused.status(), StatusCode::UNAUTHORIZED);
	let third: Tokens = body(app.refresh(&retried.refresh_token).await).await;
	assert_eq!(app.me(&third.access_token).await, StatusCode::OK);

	// Signing out with the access token ends the session and its refresh
	// token.
	let signed_out = app
		.send(native(
			"POST",
			"/auth/sign-out",
			Some(&third.access_token),
			None,
		))
		.await;
	assert_eq!(signed_out.status(), StatusCode::NO_CONTENT);
	assert_eq!(app.me(&third.access_token).await, StatusCode::UNAUTHORIZED);
	let error: ErrorResponse = body(app.refresh(&third.refresh_token).await).await;
	assert_eq!(error.error, "signed_out");
}

#[tokio::test]
async fn a_browser_and_a_native_app_see_each_other_as_devices() {
	let app = app(|builder| builder).await;
	let laptop = app.sign_in("ada@example.com").await;
	let phone = app.sign_in_native("ada@example.com").await;

	let devices: Vec<Device> =
		body(app.call("GET", "/auth/sessions", None, Some(&laptop)).await).await;
	let mut seen: Vec<_> = devices
		.iter()
		.map(|device| (device.session.device.as_str(), device.current))
		.collect();
	seen.sort_unstable();
	assert_eq!(seen, [("Ada's iPhone", false), ("Unknown device", true)]);
	let phone_id = devices
		.iter()
		.find(|device| !device.current)
		.unwrap()
		.session
		.id;

	// The laptop ends the phone's session; its refresh token dies with it.
	let revoked = app
		.call(
			"POST",
			"/auth/sessions/revoke",
			Some(json!({ "id": phone_id })),
			Some(&laptop),
		)
		.await;
	assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
	assert_eq!(app.me(&phone.access_token).await, StatusCode::UNAUTHORIZED);
	let refused = app.refresh(&phone.refresh_token).await;
	assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
}
#[tokio::test]
async fn changes_from_other_sites_are_refused() {
	let app = app(|builder| builder).await;
	let cookie = app.sign_in("ada@example.com").await;
	let post = |configure: Configure| {
		with_body(
			configure(Request::builder().method("POST").uri("/change")),
			None,
		)
	};

	let cases: [(Configure, StatusCode); 6] = [
		(
			|request| request.header(header::ORIGIN, ORIGIN),
			StatusCode::OK,
		),
		(
			|request| request.header(header::ORIGIN, "https://evil.example"),
			StatusCode::FORBIDDEN,
		),
		(
			|request| request.header(header::ORIGIN, "null"),
			StatusCode::FORBIDDEN,
		),
		(
			|request| request.header("sec-fetch-site", "same-origin"),
			StatusCode::OK,
		),
		(
			|request| request.header("sec-fetch-site", "cross-site"),
			StatusCode::FORBIDDEN,
		),
		(|request| request, StatusCode::OK),
	];

	for (configure, status) in cases {
		assert_eq!(app.send(post(configure)).await.status(), status);
	}

	// A cookie with neither header could be a forged form post.
	let forged = Request::builder()
		.method("POST")
		.uri("/change")
		.header(header::COOKIE, &cookie)
		.body(Body::empty())
		.unwrap();
	let refused = app.send(forged).await;
	assert_eq!(refused.status(), StatusCode::FORBIDDEN);
	let error: ErrorResponse = body(refused).await;
	assert_eq!(error.error, "forbidden");

	// Reads are never refused.
	let read = Request::builder()
		.uri("/me")
		.header(header::ORIGIN, "https://evil.example")
		.header(header::COOKIE, &cookie)
		.body(Body::empty())
		.unwrap();
	assert_eq!(app.send(read).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn without_an_origin_requests_must_come_from_the_host() {
	let db = leptoskit_db::testing::sqlite().await;
	let auth = Auth::builder(&db, Keys::single("test", [3; 32]))
		.mailer(Mailbox::new())
		.build()
		.unwrap();
	let router = Router::new()
		.route("/change", post(|| async { "changed" }))
		.layer(auth.layer());
	let post = |origin: &str| {
		Request::builder()
			.method("POST")
			.uri("/change")
			.header(header::HOST, "notes.example")
			.header(header::ORIGIN, origin)
			.body(Body::empty())
			.unwrap()
	};

	let same = router
		.clone()
		.oneshot(post("https://notes.example"))
		.await
		.unwrap();
	assert_eq!(same.status(), StatusCode::OK);
	let other = router.oneshot(post("https://evil.example")).await.unwrap();
	assert_eq!(other.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn failures_say_what_went_wrong_without_saying_too_much() {
	let app = app(|builder| {
		builder.limits(Limits {
			code_per_address: Limit::new(1, std::time::Duration::from_secs(60)),
			..Limits::default()
		})
	})
	.await;

	let invalid = app
		.call(
			"POST",
			"/auth/email/send",
			Some(json!({ "email": "nope" })),
			None,
		)
		.await;
	assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
	let error: ErrorResponse = body(invalid).await;
	assert_eq!(error.error, "invalid_email");

	app.call(
		"POST",
		"/auth/email/send",
		Some(json!({ "email": "ada@example.com" })),
		None,
	)
	.await;
	let limited = app
		.call(
			"POST",
			"/auth/email/send",
			Some(json!({ "email": "ada@example.com" })),
			None,
		)
		.await;
	assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
	let retry_after: u64 = limited.headers()[header::RETRY_AFTER]
		.to_str()
		.unwrap()
		.parse()
		.unwrap();
	let error: ErrorResponse = body(limited).await;
	assert_eq!(error.error, "rate_limited");
	assert_eq!(error.retry_after, Some(retry_after));
	assert!((1..=60).contains(&retry_after));

	let wrong = app
		.call(
			"POST",
			"/auth/email/verify",
			Some(json!({ "email": "ada@example.com", "code": "AAAA-AAAA" })),
			None,
		)
		.await;
	assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
	let error: ErrorResponse = body(wrong).await;
	assert_eq!(error.error, "invalid_code");
}

#[tokio::test]
async fn people_list_and_end_their_own_sessions() {
	let app = app(|builder| builder).await;
	let laptop = app.sign_in("ada@example.com").await;
	let phone = app.sign_in("ada@example.com").await;
	app.sign_in("bob@example.com").await;

	let listed: Vec<Device> =
		body(app.call("GET", "/auth/sessions", None, Some(&laptop)).await).await;
	assert_eq!(listed.len(), 2);
	// The laptop asked, so it's the current one.
	assert_eq!(listed.iter().filter(|device| device.current).count(), 1);
	let visible = app
		.call("GET", "/visible-sessions", None, Some(&laptop))
		.await;
	assert_eq!(text(visible).await, "2");
	let anonymous = app.call("GET", "/visible-sessions", None, None).await;
	assert_eq!(text(anonymous).await, "0");

	let current: SessionResponse =
		body(app.call("GET", "/auth/session", None, Some(&phone)).await).await;
	let phone_id = current.session.unwrap().id;
	let revoked = app
		.call(
			"POST",
			"/auth/sessions/revoke",
			Some(json!({ "id": phone_id })),
			Some(&laptop),
		)
		.await;
	assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
	let again = app
		.call(
			"POST",
			"/auth/sessions/revoke",
			Some(json!({ "id": phone_id })),
			Some(&laptop),
		)
		.await;
	assert_eq!(again.status(), StatusCode::NOT_FOUND);
	assert_eq!(
		app.call("GET", "/me", None, Some(&phone)).await.status(),
		StatusCode::UNAUTHORIZED
	);

	let tablet = app.sign_in("ada@example.com").await;
	let others = app
		.call("POST", "/auth/sessions/revoke-others", None, Some(&laptop))
		.await;
	assert_eq!(others.status(), StatusCode::NO_CONTENT);
	assert_eq!(
		app.call("GET", "/me", None, Some(&tablet)).await.status(),
		StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		app.call("GET", "/me", None, Some(&laptop)).await.status(),
		StatusCode::OK
	);

	let signed_out = app.call("GET", "/auth/sessions", None, None).await;
	assert_eq!(signed_out.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn signing_out_everywhere_ends_every_session() {
	let app = app(|builder| builder).await;
	let laptop = app.sign_in("ada@example.com").await;
	let phone = app.sign_in("ada@example.com").await;
	let bob = app.sign_in("bob@example.com").await;

	let everywhere = app
		.call("POST", "/auth/sessions/revoke-all", None, Some(&laptop))
		.await;
	assert_eq!(everywhere.status(), StatusCode::NO_CONTENT);
	assert!(
		everywhere.headers()[header::SET_COOKIE]
			.to_str()
			.unwrap()
			.contains("Max-Age=0")
	);

	for (cookie, status) in [
		(&laptop, StatusCode::UNAUTHORIZED),
		(&phone, StatusCode::UNAUTHORIZED),
		(&bob, StatusCode::OK),
	] {
		assert_eq!(
			app.call("GET", "/me", None, Some(cookie)).await.status(),
			status
		);
	}

	let again = app
		.call("POST", "/auth/sessions/revoke-all", None, Some(&laptop))
		.await;
	assert_eq!(again.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn development_has_a_mailbox_and_signs_in_without_codes() {
	let app = app(|builder| builder.dev_mailbox().dev_sign_in()).await;

	let empty = text(app.call("GET", "/_leptoskit/mail", None, None).await).await;
	assert!(empty.contains("No email yet"));

	let signed_in = app
		.call(
			"POST",
			"/auth/dev/sign-in",
			Some(json!({ "email": "ada@example.com" })),
			None,
		)
		.await;
	assert_eq!(signed_in.status(), StatusCode::OK);
	assert!(signed_in.headers().contains_key(header::SET_COOKIE));
	let signed_in: SignedInResponse = body(signed_in).await;
	assert!(signed_in.signed_up);

	app.call(
		"POST",
		"/auth/email/send",
		Some(json!({ "email": "<b>@example.com" })),
		None,
	)
	.await;
	app.call(
		"POST",
		"/auth/email/send",
		Some(json!({ "email": "bob@example.com" })),
		None,
	)
	.await;
	let page = text(app.call("GET", "/_leptoskit/mail", None, None).await).await;
	assert!(page.contains("bob@example.com"));
	assert!(page.contains("Your code to sign in"));
	assert!(page.find("bob@example.com") < page.find("&lt;b&gt;@example.com"));
	assert!(!page.contains("<b>"));
	// The test's own mailbox isn't the one the server sends to.
	assert!(app.mailbox.messages().is_empty());

	let production = self::app(|builder| builder).await;
	assert_eq!(
		production
			.call("GET", "/_leptoskit/mail", None, None)
			.await
			.status(),
		StatusCode::NOT_FOUND
	);
	assert_eq!(
		production
			.call(
				"POST",
				"/auth/dev/sign-in",
				Some(json!({ "email": "ada@example.com" })),
				None
			)
			.await
			.status(),
		StatusCode::NOT_FOUND
	);
}

#[tokio::test]
async fn plain_http_development_uses_a_plain_cookie() {
	let db = leptoskit_db::testing::sqlite().await;
	let auth = Auth::builder(&db, Keys::single("test", [3; 32]))
		.dev_mailbox()
		.dev_sign_in()
		.origin("http://localhost:3000/")
		.build()
		.unwrap();
	let router = auth.router::<()>().layer(auth.layer());
	let response = router
		.oneshot(
			Request::builder()
				.method("POST")
				.uri("/auth/dev/sign-in")
				.header(header::ORIGIN, "http://localhost:3000")
				.header(header::CONTENT_TYPE, "application/json")
				.body(Body::from(
					json!({ "email": "ada@example.com" }).to_string(),
				))
				.unwrap(),
		)
		.await
		.unwrap();
	let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
	assert!(cookie.starts_with("session="));
	assert!(!cookie.contains("Secure"));
}

#[tokio::test]
async fn the_client_address_comes_from_the_peer_or_a_trusted_header() {
	let app = app(|builder| builder).await;
	let peer: SocketAddr = "198.51.100.7:4000".parse().unwrap();
	let mut request = Request::builder()
		.uri("/where")
		.body(Body::empty())
		.unwrap();
	request.extensions_mut().insert(ConnectInfo(peer));
	assert_eq!(text(app.send(request).await).await, "Some(198.51.100.7)");

	let proxied = self::app(|builder| builder.client_ip_header("Fly-Client-IP")).await;
	let request = Request::builder()
		.method("POST")
		.uri("/auth/email/send")
		.header(header::ORIGIN, ORIGIN)
		.header("fly-client-ip", "203.0.113.5, 10.0.0.1")
		.header(header::USER_AGENT, "Firefox/131.0")
		.header(header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			json!({ "email": "ada@example.com" }).to_string(),
		))
		.unwrap();
	assert_eq!(proxied.send(request).await.status(), StatusCode::NO_CONTENT);
	let events = Event::query()
		.all(&proxied.db.system("test"))
		.await
		.unwrap();
	assert_eq!(events[0].ip.as_deref(), Some("203.0.113.5"));
	assert_eq!(events[0].user_agent.as_deref(), Some("Firefox/131.0"));
}
