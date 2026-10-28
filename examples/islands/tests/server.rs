//! The server end to end: requests through the axum router, rendered by
//! leptos the way `leptoskit serve` runs it.

#![cfg(feature = "ssr")]

use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use http_body_util::BodyExt;
use islands_example::server::MAX_BODY_BYTES;
use islands_example::server::MAX_IN_FLIGHT;
use islands_example::server::bounded;
use islands_example::server::router;
use leptos::prelude::LeptosOptions;
use leptoskit::server::axum;
use leptoskit::server::axum::Router;
use leptoskit::server::axum::body::Body;
use leptoskit::server::axum::http::Request;
use leptoskit::server::axum::http::StatusCode;
use leptoskit::server::axum::http::header;
use leptoskit::server::tokio;
use leptoskit::server::tower::util::ServiceExt;

/// A built site: the stylesheet the shell links to, in a directory of its
/// own that is removed afterwards.
struct Site {
	root: PathBuf,
}

impl Site {
	fn new(tag: &str) -> Self {
		let root =
			std::env::temp_dir().join(format!("islands_example_{tag}_{}", std::process::id()));
		std::fs::create_dir_all(&root).expect("site root");
		std::fs::write(root.join("style.css"), "main { color: teal; }").expect("stylesheet");
		Self { root }
	}

	fn options(&self) -> LeptosOptions {
		LeptosOptions::builder()
			.output_name("islands_example")
			.site_root(self.root.to_string_lossy().into_owned())
			.build()
	}
}

impl Drop for Site {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.root);
	}
}

/// The status, content type, and body of a GET.
async fn get(site: &Site, path: &str) -> (StatusCode, String, String) {
	let request = Request::get(path).body(Body::empty()).expect("request");
	let response = router(site.options())
		.oneshot(request)
		.await
		.expect("the router is infallible");
	let status = response.status();
	let content_type = response
		.headers()
		.get(header::CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.unwrap_or_default()
		.to_owned();
	let body = response
		.into_body()
		.collect()
		.await
		.expect("body")
		.to_bytes();
	(
		status,
		content_type,
		String::from_utf8(body.to_vec()).expect("utf-8"),
	)
}

#[tokio::test]
async fn the_notes_page_is_a_whole_document_with_one_island() {
	let site = Site::new("notes");
	let (status, content_type, html) = get(&site, "/").await;
	assert_eq!(status, StatusCode::OK);
	assert!(content_type.starts_with("text/html"), "{content_type}");
	assert!(html.starts_with("<!DOCTYPE html>"), "{html}");
	assert!(html.contains("<title>Field notes</title>"), "{html}");
	assert!(html.contains(r#"href="/style.css""#), "{html}");
	assert!(html.contains("Most of a page is just words."), "{html}");
	// Only the clap counter hydrates, with its props serialized for the
	// client.
	assert_eq!(html.matches("<leptos-island").count(), 1, "{html}");
	assert!(html.contains(r#"<span class="count">3</span>"#), "{html}");
	// The hydration scripts load the islands client.
	assert!(html.contains("islands_example"), "{html}");
}

#[tokio::test]
async fn the_lab_renders_its_lazy_mixer() {
	let site = Site::new("lab");
	let (status, _, html) = get(&site, "/lab").await;
	assert_eq!(status, StatusCode::OK);
	assert!(html.contains("This island loaded on demand."), "{html}");
	assert_eq!(html.matches("<leptos-island").count(), 1, "{html}");
	assert_eq!(html.matches(r#"type="range""#).count(), 3, "{html}");
}

/// `leptos_routes` registers only the app's own paths, so anything else
/// reaches the static files and is a plain 404, never the app's
/// `<Routes fallback>`.
#[tokio::test]
async fn unknown_paths_are_not_found() {
	let site = Site::new("fallback");
	let (status, _, body) = get(&site, "/nowhere").await;
	assert_eq!(status, StatusCode::NOT_FOUND);
	assert!(!body.contains("<leptos-island"), "{body}");
}

#[tokio::test]
async fn built_files_are_served_behind_the_routes() {
	let site = Site::new("assets");
	let (status, content_type, css) = get(&site, "/style.css").await;
	assert_eq!(status, StatusCode::OK);
	assert!(content_type.starts_with("text/css"), "{content_type}");
	assert_eq!(css, "main { color: teal; }");
}

/// A load balancer's probe: no render, no island, just `ok`.
#[tokio::test]
async fn the_health_route_answers_without_rendering() {
	let site = Site::new("health");
	let (status, _, body) = get(&site, "/_leptoskit/health").await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body, "ok");
}

/// The body limit holds on the stream, before any handler reads a byte, so
/// a declared size over the cap is refused outright.
#[tokio::test]
async fn oversized_bodies_are_refused_before_they_are_read() {
	let site = Site::new("body-limit");
	let request = Request::post("/")
		.header(header::CONTENT_LENGTH, (MAX_BODY_BYTES + 1).to_string())
		.body(Body::empty())
		.expect("request");
	let response = router(site.options())
		.oneshot(request)
		.await
		.expect("the router is infallible");
	assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

/// Past the in-flight limit, a request is shed with a 503 at once rather
/// than queued behind work the server cannot finish in time.
#[tokio::test]
async fn requests_past_the_in_flight_limit_are_shed() {
	let app = bounded(
		Router::new()
			.route(
				"/hang",
				axum::routing::get(std::future::pending::<&'static str>),
			)
			.route("/ready", axum::routing::get(|| async { "ok" })),
	);
	let request = |path: &str| Request::get(path).body(Body::empty()).expect("request");
	let hanging: Vec<_> = (0..MAX_IN_FLIGHT)
		.map(|_| {
			let app = app.clone();
			tokio::spawn(async move { app.oneshot(request("/hang")).await })
		})
		.collect();
	// Each hanging request takes its slot when it is first polled, so the
	// probe is answered until the last slot is gone, then shed.
	let deadline = Instant::now() + Duration::from_secs(10);
	let status = loop {
		let status = app
			.clone()
			.oneshot(request("/ready"))
			.await
			.expect("the router is infallible")
			.status();
		if status == StatusCode::SERVICE_UNAVAILABLE || Instant::now() > deadline {
			break status;
		}
		tokio::task::yield_now().await;
	};
	assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
	for task in hanging {
		task.abort();
	}
}
