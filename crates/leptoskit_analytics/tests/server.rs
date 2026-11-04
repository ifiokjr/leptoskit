#![cfg(feature = "server")]

use axum::Router;
use axum::body::Body;
use axum::body::to_bytes;
use axum::extract::Extension;
use axum::http::Request;
use axum::http::StatusCode;
use axum::routing::get;
use leptoskit_analytics::server::ServerConfig;
use leptoskit_analytics::server::Timer;
use leptoskit_analytics::server::{self};
use leptoskit_analytics::*;
use tower::ServiceExt as _;

const KEY: &str = "server-only-test-read-key-0123456789";
const CONTEXT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

fn config() -> ServerConfig {
	ServerConfig {
		export: None,
		resource: Resource {
			service: "notes".into(),
			environment: "test".into(),
			release: "a1".into(),
			instance: "one".into(),
		},
		limits: Limits::default(),
		read_key: KEY.into(),
		public_origin: "https://app.example.com".into(),
		browser_routes: vec!["/notes/{id}".into()],
	}
}

fn app() -> (Router, Analytics) {
	server::mount(
		Router::new().route(
			"/notes/{id}",
			get(
				|Extension(analytics): Extension<Analytics>,
				 Extension(context): Extension<TraceContext>| {
					async move {
						Timer::new(analytics, Signal::Database, "load_note", Some(context))
							.finish(Outcome::Ok);
						StatusCode::SERVICE_UNAVAILABLE
					}
				},
			),
		),
		config(),
	)
	.unwrap()
}

async fn json(response: axum::response::Response) -> serde_json::Value {
	serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}

fn browser(payload: &serde_json::Value) -> Request<Body> {
	Request::post("/_leptoskit/analytics/browser")
		.header("content-type", "application/json")
		.header("origin", "https://app.example.com")
		.body(Body::from(payload.to_string()))
		.unwrap()
}

#[tokio::test]
async fn api_requires_a_key_and_http_uses_templates_and_correlates_explicit_timers() {
	let (app, analytics) = app();
	let response = app
		.clone()
		.oneshot(
			Request::get("/notes/private-id?token=private-query")
				.header("traceparent", CONTEXT)
				.header("cookie", "secret=cookie-value")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
	let continued =
		TraceContext::parse(response.headers()["traceparent"].to_str().unwrap()).unwrap();
	assert_eq!(
		continued.trace_id(),
		TraceContext::parse(CONTEXT).unwrap().trace_id()
	);
	assert_ne!(
		continued.span_id(),
		TraceContext::parse(CONTEXT).unwrap().span_id()
	);
	assert!(continued.as_str().ends_with("-00"));
	for authorization in [None, Some("Bearer wrong"), Some(KEY)] {
		let mut request = Request::get("/_leptoskit/analytics");
		if let Some(value) = authorization {
			request = request.header("authorization", value);
		}
		let response = app
			.clone()
			.oneshot(request.body(Body::empty()).unwrap())
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
		assert_eq!(response.headers()["cache-control"], "no-store");
	}
	let response = app
		.clone()
		.oneshot(
			Request::get("/_leptoskit/analytics")
				.header("authorization", format!("Bearer {KEY}"))
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(response.status(), StatusCode::OK);
	let evidence = json(response).await;
	let encoded = evidence.to_string();
	for secret in [KEY, "private-id", "private-query", "cookie-value"] {
		assert!(!encoded.contains(secret));
	}
	assert_eq!(evidence["snapshot"]["resource"]["release"], "a1");
	assert_eq!(evidence["snapshot"]["recent"].as_array().unwrap().len(), 2);
	assert_eq!(analytics.snapshot().series.len(), 2);
	assert!(
		analytics
			.snapshot()
			.series
			.iter()
			.any(|s| s.key.operation == "/notes/{id}" && s.key.outcome == Outcome::ServerError)
	);
	assert!(
		!evidence["infrastructure_changes_authorized"]
			.as_bool()
			.unwrap()
	);
	app.oneshot(
		Request::get("/unknown-private-path")
			.body(Body::empty())
			.unwrap(),
	)
	.await
	.unwrap();
	assert!(
		analytics
			.snapshot()
			.series
			.iter()
			.any(|s| s.key.operation == "unmatched")
	);
}

#[tokio::test]
async fn ingestion_is_atomic_allowlisted_origin_checked_and_never_backend_evidence() {
	let (app, analytics) = app();
	let good = serde_json::json!({"kind":"navigation", "route":"/notes/{id}", "duration_ms":125, "traceparent":CONTEXT});
	let payload = serde_json::json!({"events":[good.clone()]});
	assert_eq!(
		app.clone()
			.oneshot(browser(&payload))
			.await
			.unwrap()
			.status(),
		StatusCode::NO_CONTENT
	);
	assert_eq!(analytics.snapshot().series[0].key.source, Source::Browser);
	let mut wrong = browser(&payload);
	wrong
		.headers_mut()
		.insert("origin", "https://another.example.com".parse().unwrap());
	assert_eq!(
		app.clone().oneshot(wrong).await.unwrap().status(),
		StatusCode::BAD_REQUEST
	);
	let mut bad = good.clone();
	bad["route"] = serde_json::json!("/notes/private-user-id");
	assert_eq!(
		app.clone()
			.oneshot(browser(&serde_json::json!({"events":[good.clone(),bad]})))
			.await
			.unwrap()
			.status(),
		StatusCode::BAD_REQUEST
	);
	let mut spoofed = good.clone();
	spoofed["source"] = serde_json::json!("backend");
	assert_eq!(
		app.clone()
			.oneshot(browser(&serde_json::json!({"events":[spoofed]})))
			.await
			.unwrap()
			.status(),
		StatusCode::UNPROCESSABLE_ENTITY
	);
	let mut context = good.clone();
	context["traceparent"] = serde_json::json!("invalid");
	assert_eq!(
		app.clone()
			.oneshot(browser(&serde_json::json!({"events":[context]})))
			.await
			.unwrap()
			.status(),
		StatusCode::UNPROCESSABLE_ENTITY
	);
	assert_eq!(analytics.snapshot().series[0].windows[0].count, 1);
	assert_eq!(analytics.snapshot().browser_batches_rejected, 4);
	assert_eq!(
		app.clone()
			.oneshot(browser(
				&serde_json::json!({"events":vec![good.clone();33]})
			))
			.await
			.unwrap()
			.status(),
		StatusCode::BAD_REQUEST
	);
	let oversized = serde_json::json!({"events":[],"padding":"x".repeat(17000)});
	assert_eq!(
		app.clone()
			.oneshot(browser(&oversized))
			.await
			.unwrap()
			.status(),
		StatusCode::PAYLOAD_TOO_LARGE
	);
	let sdk = app
		.oneshot(
			Request::get("/_leptoskit/analytics.mjs")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(sdk.status(), StatusCode::OK);
	let sdk = to_bytes(sdk.into_body(), 16000).await.unwrap();
	assert!(!String::from_utf8_lossy(&sdk).contains(KEY));
}

#[tokio::test]
async fn ingest_has_a_process_budget_and_timers_finish_once() {
	let (app, analytics) = app();
	for _ in 0..30 {
		assert_eq!(
			app.clone()
				.oneshot(browser(
					&serde_json::json!({"events":[{"kind":"error","route":"/notes/{id}","duration_ms":0}]})
				))
				.await
				.unwrap()
				.status(),
			StatusCode::NO_CONTENT
		);
	}
	assert_eq!(
		app.oneshot(browser(
			&serde_json::json!({"events":[{"kind":"error","route":"/notes/{id}","duration_ms":0}]})
		))
		.await
		.unwrap()
		.status(),
		StatusCode::TOO_MANY_REQUESTS
	);
	assert_eq!(analytics.snapshot().browser_batches_rejected, 1);
	Timer::new(analytics.clone(), Signal::Job, "refresh", None).finish(Outcome::Ok);
	drop(Timer::new(analytics.clone(), Signal::Job, "refresh", None));
	let snapshot = analytics.snapshot();
	let jobs: Vec<_> = snapshot
		.series
		.iter()
		.filter(|s| s.key.signal == Signal::Job)
		.collect();
	assert_eq!(jobs.len(), 2);
	assert!(jobs.iter().all(|s| s.windows[0].count == 1));
	assert!(jobs.iter().any(|s| s.key.outcome == Outcome::Cancelled));
}

#[test]
fn mounting_rejects_invalid_settings_without_exposing_the_key() {
	let mut c = config();
	c.read_key = "private-short-key".into();
	let error = server::mount(Router::new(), c).err().unwrap();
	assert!(!error.to_string().contains("private-short-key"));
	for origin in [
		"http://example.com",
		"https://app.example.com/path",
		"https://user:password@app.example.com",
		"https://app.example.com?token=secret",
	] {
		let mut c = config();
		c.public_origin = origin.into();
		assert!(server::mount(Router::new(), c).is_err());
	}
	let mut c = config();
	c.browser_routes = vec!["/path?secret=value".into()];
	assert!(server::mount(Router::new(), c).is_err());
}
