#![cfg(all(feature = "server", feature = "site"))]
use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::body::to_bytes;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt as _;

fn web_assets(root: &Path) {
	std::fs::create_dir_all(root.join("pkg")).unwrap();
	std::fs::create_dir_all(root.join("console")).unwrap();
	std::fs::create_dir_all(root.join("rive")).unwrap();
	for (path, bytes) in [
		("index.html", include_str!("../web/index.html")),
		("console/index.html", include_str!("../web/index.html")),
		("bootstrap.js", include_str!("../web/bootstrap.js")),
		("landing.css", include_str!("../web/landing.css")),
		("landing.mjs", include_str!("../web/landing.mjs")),
		("console.css", include_str!("../web/console.css")),
		("pkg/leptoskit_app.js", "fixture module"),
		("pkg/leptoskit_app_bg.wasm", "fixture wasm"),
		("rive/rive.js", "fixture runtime"),
		("rive/rive.wasm", "fixture runtime wasm"),
		("rive/brand.riv", "fixture scene"),
	] {
		std::fs::write(root.join(path), bytes).unwrap();
	}
}

#[tokio::test]
async fn one_origin_serves_the_landing_real_docs_console_and_api() {
	let directory = tempfile::tempdir().unwrap();
	web_assets(directory.path());
	let docs = directory.path().join("docs");
	leptoskit_docs_site::site(leptoskit_docs_site::book())
		.url("https://deploy.example.test/docs/")
		.build(&docs)
		.unwrap();
	let api = Router::new().route("/healthz", axum::routing::get(|| async { "healthy" }));
	let app =
		leptoskit_app::site::router(api, directory.path(), &docs, "https://deploy.example.test")
			.unwrap();
	for path in [
		"/",
		"/console/",
		"/docs/",
		"/docs/guide/29-hosted-deployments.html",
		"/docs/_leptoskit/theme.js",
		"/docs/_leptoskit/search.json",
		"/docs/assets/fonts/skribble/SkribbleNotoSketch-Regular.woff2",
		"/docs/theme/skribble.css",
		"/docs/_leptoskit/generated/brand-mint.svg",
		"/rive/rive.js",
		"/rive/rive.wasm",
		"/healthz",
	] {
		let response = app
			.clone()
			.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::OK, "{path}");
		assert!(
			response.headers()["content-security-policy"]
				.to_str()
				.unwrap()
				.contains("script-src 'self'")
		);
		let body = to_bytes(response.into_body(), 2 * 1024 * 1024)
			.await
			.unwrap();
		if path == "/" {
			let html = String::from_utf8(body.to_vec()).unwrap();
			assert!(html.contains("One tree."));
			assert!(html.contains("href=\"/docs/\""));
			assert!(html.contains("href=\"/console/\""));
			assert!(html.contains("<h1"));
		}
	}
	for path in [
		"/docs/missing.html",
		"/console/missing",
		"/api/v1/missing",
		"/missing",
	] {
		let response = app
			.clone()
			.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
			.await
			.unwrap();
		assert_eq!(
			response.status(),
			StatusCode::NOT_FOUND,
			"{path} must not become a console page"
		);
	}
	for path in ["/docs", "/console"] {
		let response = app
			.clone()
			.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
		assert_eq!(response.headers()["location"], format!("{path}/"));
	}
	let owner = reactive_graph::owner::Owner::new();
	owner.with(|| check_documentation_links(&leptoskit_app::landing::screen(), &docs));
	let html = std::fs::read_to_string(docs.join("index.html")).unwrap();
	assert!(!html.contains("fonts.googleapis.com"));
	assert!(
		!html.contains("<script>"),
		"the strict CSP permits only external scripts"
	);
	assert!(html.contains("https://deploy.example.test/docs/"));
	for url in include_str!("../web/landing.css").split("url(\"").skip(1) {
		let path = url
			.split('"')
			.next()
			.unwrap()
			.strip_prefix("/docs/")
			.unwrap();
		assert!(
			docs.join(path).is_file(),
			"the landing page's {path} must be packaged"
		);
	}
}

fn check_documentation_links(element: &leptoskit::prelude::Element, docs: &Path) {
	if let Some(href) = element
		.props
		.href()
		.and_then(|href| href.strip_prefix("/docs/"))
	{
		let path = if href.is_empty() { "index.html" } else { href };
		assert!(
			docs.join(path).is_file(),
			"public link {href} must reach the real book"
		);
	}
	for child in &element.children {
		check_documentation_links(child, docs);
	}
}

#[test]
fn an_incomplete_website_artifact_cannot_be_served() {
	let directory = tempfile::tempdir().unwrap();
	web_assets(directory.path());
	let result = leptoskit_app::site::router(
		Router::new(),
		directory.path(),
		&directory.path().join("docs"),
		"https://deploy.example.test",
	);
	assert!(
		result
			.unwrap_err()
			.to_string()
			.contains("missing documentation")
	);
}
