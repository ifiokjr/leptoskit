//! Queries and mutations behind the session layer run as the signed-in
//! person, and each session's idempotency keys are its own.

#![cfg(all(feature = "rpc", feature = "axum", feature = "sqlite"))]

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use http_body_util::BodyExt;
use leptoskit_auth::Auth;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::SignedIn;
use leptoskit_rpc as rpc;
use leptoskit_rpc::Identify;
use leptoskit_rpc::server::Server;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt;

const ORIGIN: &str = "https://notes.example";

/// Who is calling: their account, or nobody.
#[rpc::query]
async fn whoami(cx: &rpc::QueryCx) -> rpc::Result<Option<String>> {
	Ok(cx.get::<SignedIn>().map(|me| me.account.to_string()))
}

/// Counts calls that really ran.
static RUNS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Runs once per idempotency key, for the signed-in person only.
#[rpc::mutation]
async fn tick(cx: &rpc::MutationCx) -> rpc::Result<u32> {
	cx.require::<SignedIn>()?;
	Ok(RUNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst))
}

struct App {
	router: Router,
	mailbox: Mailbox,
	auth: Auth,
}

async fn app() -> App {
	let db = leptoskit_db::testing::sqlite().await;
	let mailbox = Mailbox::new();
	let auth = Auth::builder(&db, Keys::single("test", [3; 32]))
		.mailer(mailbox.clone())
		.origin(ORIGIN)
		.build()
		.unwrap();
	let router = Router::new()
		.merge(Server::new(db).identify(auth.clone()).router())
		.merge(auth.router())
		.layer(auth.layer());
	App {
		router,
		mailbox,
		auth,
	}
}

impl App {
	async fn post(
		&self,
		path: &str,
		body: Value,
		headers: &[(&str, &str)],
	) -> (StatusCode, String) {
		let mut request = Request::post(path)
			.header(header::ORIGIN, ORIGIN)
			.header(header::CONTENT_TYPE, "application/json");

		for (name, value) in headers {
			request = request.header(*name, *value);
		}

		let response = self
			.router
			.clone()
			.oneshot(request.body(Body::from(body.to_string())).unwrap())
			.await
			.unwrap();
		let status = response.status();
		let cookie = response.headers().get(header::SET_COOKIE).map(|cookie| {
			cookie
				.to_str()
				.unwrap()
				.split(';')
				.next()
				.unwrap()
				.to_owned()
		});
		let bytes = response.into_body().collect().await.unwrap().to_bytes();
		let text = String::from_utf8(bytes.to_vec()).unwrap();

		(status, cookie.unwrap_or(text))
	}

	async fn sign_in(&self, email: &str) -> String {
		let (status, _) = self
			.post("/auth/email/send", json!({ "email": email }), &[])
			.await;
		assert_eq!(status, StatusCode::NO_CONTENT);
		let code = self.mailbox.latest_to(email).unwrap();
		let code = code.subject.rsplit(' ').next().unwrap().to_owned();
		let (status, cookie) = self
			.post(
				"/auth/email/verify",
				json!({ "email": email, "code": code }),
				&[],
			)
			.await;
		assert_eq!(status, StatusCode::OK);
		cookie
	}
}

#[tokio::test]
async fn operations_run_as_the_signed_in_person() {
	let app = app().await;
	let (status, body) = app.post("/api/whoami", json!({}), &[]).await;
	assert_eq!((status, body.as_str()), (StatusCode::OK, "null"));
	let (status, body) = app.post("/api/tick", json!({}), &[]).await;
	assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

	let ada = app.sign_in("ada@notes.example").await;
	let (status, body) = app
		.post("/api/whoami", json!({}), &[("cookie", &ada)])
		.await;
	assert_eq!(status, StatusCode::OK);
	assert!(body.starts_with('"'), "an account ID: {body}");

	// The same key from the same session runs once; another session's is
	// its own.
	let first = app
		.post(
			"/api/tick",
			json!({}),
			&[("cookie", &ada), ("idempotency-key", "k")],
		)
		.await;
	let again = app
		.post(
			"/api/tick",
			json!({}),
			&[("cookie", &ada), ("idempotency-key", "k")],
		)
		.await;
	assert_eq!(first.0, StatusCode::OK);
	assert_eq!(first, again);
	let grace = app.sign_in("grace@notes.example").await;
	let other = app
		.post(
			"/api/tick",
			json!({}),
			&[("cookie", &grace), ("idempotency-key", "k")],
		)
		.await;
	assert_ne!(first.1, other.1);

	// Without the layer, the request's own cookie still identifies it.
	let (parts, ()) = Request::post("/api/whoami")
		.header("cookie", &ada)
		.body(())
		.unwrap()
		.into_parts();
	let identity = app.auth.identify(&parts).await.unwrap();
	assert!(identity.caller.is_some());
	assert!(identity.context.get::<SignedIn>().is_some());
	let (parts, ()) = Request::post("/api/whoami").body(()).unwrap().into_parts();
	let nobody = app.auth.identify(&parts).await.unwrap();
	assert!(nobody.caller.is_none());
}
