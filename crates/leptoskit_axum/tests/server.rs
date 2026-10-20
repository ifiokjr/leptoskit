//! Server toolkit tests: static serving, health, SPA fallback, codepush mount.

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use http_body_util::BodyExt;
use leptoskit_axum::ServeError;
use leptoskit_axum::ServeOptions;
use leptoskit_axum::dev_router;
use leptoskit_axum::serve;
use leptoskit_codepush::ReleaseStore;
use tower::util::ServiceExt;

struct Scratch {
	path: PathBuf,
}

impl Scratch {
	fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_axum_test_{tag}_{}_{}",
			std::process::id(),
			unique
		));
		std::fs::create_dir_all(&path).expect("scratch");
		Self { path }
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.path);
	}
}

async fn body_text(response: axum::response::Response) -> String {
	let bytes = response
		.into_body()
		.collect()
		.await
		.expect("collect body")
		.to_bytes();
	String::from_utf8(bytes.to_vec()).unwrap_or_default()
}

#[tokio::test]
async fn serves_static_files() {
	let scratch = Scratch::new("static");
	std::fs::write(scratch.path.join("app.wasm"), b"wasm bytes").expect("write");
	let router = dev_router(&ServeOptions::new(&scratch.path, "127.0.0.1", 0));

	let response = router
		.clone()
		.oneshot(Request::get("/app.wasm").body(Body::empty()).unwrap())
		.await
		.expect("request");
	assert_eq!(response.status(), StatusCode::OK);
	assert_eq!(
		response
			.headers()
			.get("content-type")
			.and_then(|value| value.to_str().ok()),
		Some("application/wasm")
	);
}

#[tokio::test]
async fn health_endpoint_answers() {
	let scratch = Scratch::new("health");
	let router = dev_router(&ServeOptions::new(&scratch.path, "127.0.0.1", 0));
	let response = router
		.clone()
		.oneshot(
			Request::get("/_leptoskit/health")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.expect("request");
	assert_eq!(response.status(), StatusCode::OK);
	assert_eq!(body_text(response).await, "ok");
}

#[tokio::test]
async fn unknown_paths_fall_back_to_index() {
	let scratch = Scratch::new("spa");
	std::fs::write(scratch.path.join("index.html"), b"<html>app</html>").expect("write");
	let router = dev_router(&ServeOptions::new(&scratch.path, "127.0.0.1", 0));

	let response = router
		.clone()
		.oneshot(
			Request::get("/some/client/route")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.expect("request");
	assert_eq!(response.status(), StatusCode::OK);
	assert_eq!(body_text(response).await, "<html>app</html>");
}

#[tokio::test]
async fn codepush_routes_mount_under_api() {
	let scratch = Scratch::new("codepush");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("store");
	let router =
		dev_router(&ServeOptions::new(&scratch.path, "127.0.0.1", 0).with_release_store(store));

	let response = router
		.clone()
		.oneshot(
			Request::get("/api/codepush/app/production/check?version=1.0.0&build=1")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.expect("request");
	// No releases yet: up to date.
	assert_eq!(response.status(), StatusCode::OK);
	assert!(body_text(response).await.contains("up_to_date"));
}

fn header<'a>(response: &'a axum::response::Response, name: &str) -> Option<&'a str> {
	response
		.headers()
		.get(name)
		.and_then(|value| value.to_str().ok())
}

#[tokio::test]
async fn negotiates_precompressed_gzip_and_varies_on_encoding() {
	let scratch = Scratch::new("gzip");
	std::fs::write(scratch.path.join("app.js"), b"plain js").expect("write");
	std::fs::write(scratch.path.join("app.js.gz"), b"gzip js").expect("write");
	let router = dev_router(&ServeOptions::new(&scratch.path, "127.0.0.1", 0));

	let gzipped = router
		.clone()
		.oneshot(
			Request::get("/app.js")
				.header("accept-encoding", "gzip")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.expect("request");
	assert_eq!(gzipped.status(), StatusCode::OK);
	assert_eq!(header(&gzipped, "content-encoding"), Some("gzip"));
	assert_eq!(header(&gzipped, "vary"), Some("accept-encoding"));
	assert_eq!(body_text(gzipped).await, "gzip js");

	let plain = router
		.clone()
		.oneshot(Request::get("/app.js").body(Body::empty()).unwrap())
		.await
		.expect("request");
	assert_eq!(plain.status(), StatusCode::OK);
	assert_eq!(header(&plain, "content-encoding"), None);
	assert_eq!(header(&plain, "vary"), Some("accept-encoding"));
	assert_eq!(body_text(plain).await, "plain js");
}

#[tokio::test]
async fn missing_assets_are_not_found_instead_of_index() {
	let scratch = Scratch::new("missing_asset");
	std::fs::write(scratch.path.join("index.html"), b"<html>app</html>").expect("write");
	let router = dev_router(&ServeOptions::new(&scratch.path, "127.0.0.1", 0));

	let response = router
		.clone()
		.oneshot(
			Request::get("/pkg/app_bg.wasm")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.expect("request");
	assert_eq!(response.status(), StatusCode::NOT_FOUND);
	assert_ne!(body_text(response).await, "<html>app</html>");
}

#[tokio::test]
async fn non_navigation_methods_do_not_fall_back_to_index() {
	let scratch = Scratch::new("spa_post");
	std::fs::write(scratch.path.join("index.html"), b"<html>app</html>").expect("write");
	let router = dev_router(&ServeOptions::new(&scratch.path, "127.0.0.1", 0));

	let response = router
		.clone()
		.oneshot(
			Request::post("/some/client/route")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.expect("request");
	assert!(response.status().is_client_error(), "{}", response.status());
	assert_ne!(body_text(response).await, "<html>app</html>");
}

#[tokio::test]
async fn unknown_codepush_paths_are_not_found() {
	let scratch = Scratch::new("codepush_unknown");
	std::fs::write(scratch.path.join("index.html"), b"<html>app</html>").expect("write");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("store");
	let router =
		dev_router(&ServeOptions::new(&scratch.path, "127.0.0.1", 0).with_release_store(store));

	for path in ["/api/codepush/unknown", "/api/codepush/app/production/nope"] {
		let response = router
			.clone()
			.oneshot(Request::get(path).body(Body::empty()).unwrap())
			.await
			.expect("request");
		assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
	}
}

#[tokio::test]
async fn rejects_bind_addresses_that_are_not_ip_literals() {
	let scratch = Scratch::new("bad_address");
	let error = serve(ServeOptions::new(&scratch.path, "not an address", 0))
		.await
		.expect_err("invalid address");
	assert!(
		matches!(&error, ServeError::InvalidAddress { address, .. } if address == "not an address"),
		"{error:?}"
	);
}
