//! The headless client's server functions, served the way an app serves
//! them: through `leptos_axum`, inside the session layer.

#![cfg(all(feature = "ssr", feature = "sqlite"))]

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use axum::routing::post;
use http_body_util::BodyExt;
use leptos::server_fn::error::FromServerFnError;
use leptos::server_fn::error::ServerFnErrorErr;
use leptoskit_auth::Auth;
use leptoskit_auth::ErrorResponse;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::PublicSession;
use tower::ServiceExt;

const ORIGIN: &str = "https://notes.example";

async fn app() -> (Router, Mailbox) {
	let db = leptoskit_db::testing::sqlite().await;
	let mailbox = Mailbox::new();
	let auth = Auth::builder(&db, Keys::single("test", [5; 32]))
		.mailer(mailbox.clone())
		.origin(ORIGIN)
		.build()
		.unwrap();
	let router = Router::new()
		.route("/auth/rpc/{*name}", post(leptos_axum::handle_server_fns))
		.merge(auth.router())
		.layer(auth.layer());
	(router, mailbox)
}

/// Calls a server function the way the browser does: a form-encoded POST.
async fn call(router: &Router, name: &str, form: &str, cookie: Option<&str>) -> Response {
	let mut request = Request::builder()
		.method("POST")
		.uri(format!("/auth/rpc/{name}"))
		.header(header::ORIGIN, ORIGIN)
		.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");

	if let Some(cookie) = cookie {
		request = request.header(header::COOKIE, cookie);
	}

	router
		.clone()
		.oneshot(request.body(Body::from(form.to_owned())).unwrap())
		.await
		.unwrap()
}

async fn json<T: serde::de::DeserializeOwned>(response: Response) -> T {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	serde_json::from_slice(&bytes).unwrap_or_else(|error| {
		panic!("{error}: {}", String::from_utf8_lossy(&bytes));
	})
}

#[tokio::test]
async fn the_browser_signs_in_and_out_through_server_functions() {
	let (router, mailbox) = app().await;

	let signed_out = call(&router, "session", "", None).await;
	assert_eq!(signed_out.status(), StatusCode::OK);
	assert_eq!(json::<Option<PublicSession>>(signed_out).await, None);

	let sent = call(&router, "send_code", "email=ada%40example.com", None).await;
	assert_eq!(sent.status(), StatusCode::OK);
	let mail = mailbox.latest_to("ada@example.com").unwrap();
	let code = mail.subject.rsplit(' ').next().unwrap();

	let wrong = call(
		&router,
		"verify_code",
		"email=ada%40example.com&code=AAAA-AAAA",
		None,
	)
	.await;
	assert!(wrong.headers().get(header::SET_COOKIE).is_none());
	let error: ErrorResponse = json(wrong).await;
	assert_eq!(error.error, "invalid_code");

	let verified = call(
		&router,
		"verify_code",
		&format!("email=ada%40example.com&code={code}"),
		None,
	)
	.await;
	assert_eq!(verified.status(), StatusCode::OK);
	let set_cookie = verified.headers()[header::SET_COOKIE]
		.to_str()
		.unwrap()
		.to_owned();
	assert!(set_cookie.starts_with("__Host-session="));
	let session: PublicSession = json(verified).await;
	let cookie = set_cookie.split(';').next().unwrap();

	let current = call(&router, "session", "", Some(cookie)).await;
	assert_eq!(
		json::<Option<PublicSession>>(current).await.unwrap().id,
		session.id
	);

	let signed_out = call(&router, "sign_out", "", Some(cookie)).await;
	assert!(
		signed_out.headers()[header::SET_COOKIE]
			.to_str()
			.unwrap()
			.contains("Max-Age=0")
	);
	let current = call(&router, "session", "", Some(cookie)).await;
	assert_eq!(json::<Option<PublicSession>>(current).await, None);
	// Signing out again is harmless.
	let again = call(&router, "sign_out", "", None).await;
	assert_eq!(again.status(), StatusCode::OK);
}

#[test]
fn transport_failures_become_error_codes() {
	let cases = [
		(ServerFnErrorErr::Request("offline".into()), "network"),
		(ServerFnErrorErr::Args("bad".into()), "invalid"),
		(ServerFnErrorErr::MissingArg("email".into()), "invalid"),
		(ServerFnErrorErr::Deserialization("bad".into()), "invalid"),
		(ServerFnErrorErr::ServerError("boom".into()), "server"),
	];

	for (error, code) in cases {
		let response = ErrorResponse::from_server_fn_error(error);
		assert_eq!(response.error, code);
		assert!(!response.message.contains("boom"));
	}
}

#[test]
fn the_scopes_build() {
	use leptoskit_auth::client;

	let _ = (
		client::session(),
		client::send_code(),
		client::verify_code(),
		client::sign_out_mutation(),
	);
}
