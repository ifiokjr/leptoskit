//! Health and readiness probes, and graceful shutdown on a real listener.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::routing::get;
use http_body_util::BodyExt;
use leptoskit_axum::GracefulShutdown;
use leptoskit_axum::Readiness;
use leptoskit_axum::SHUTDOWN_CHECK;
use leptoskit_axum::ServeError;
use leptoskit_axum::serve_listener_with;
use leptoskit_axum::serve_router_with;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio::sync::oneshot;
use tower::util::ServiceExt;

async fn probe(router: &Router, path: &str) -> (StatusCode, String, Option<String>) {
	let response = router
		.clone()
		.oneshot(Request::get(path).body(Body::empty()).unwrap())
		.await
		.expect("request");
	let status = response.status();
	let cache = response
		.headers()
		.get("cache-control")
		.and_then(|value| value.to_str().ok())
		.map(str::to_owned);
	let body = response
		.into_body()
		.collect()
		.await
		.expect("body")
		.to_bytes();
	(
		status,
		String::from_utf8(body.to_vec()).expect("utf-8"),
		cache,
	)
}

#[tokio::test]
async fn healthz_answers_while_readyz_waits_for_every_check() {
	let readiness = Readiness::new();
	let router: Router = Router::new().merge(readiness.routes());

	let (status, body, cache) = probe(&router, "/healthz").await;
	assert_eq!((status, body.as_str()), (StatusCode::OK, "ok"));
	assert_eq!(cache.as_deref(), Some("no-store"));
	let (status, body, _) = probe(&router, "/readyz").await;
	assert_eq!(status, StatusCode::OK, "no checks is ready");
	assert_eq!(body, r#"{"failing":[],"ready":true}"#);

	let migrations = readiness.check("migrations");
	let listen = readiness.check("listen");
	assert_eq!(migrations.name(), "migrations");
	assert!(!migrations.is_passing());
	let (status, body, cache) = probe(&router, "/readyz").await;
	assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
	assert_eq!(body, r#"{"failing":["listen","migrations"],"ready":false}"#);
	assert_eq!(cache.as_deref(), Some("no-store"));

	migrations.pass();
	assert_eq!(readiness.failing(), ["listen"]);
	listen.set(true);
	assert!(readiness.is_ready());
	let (status, ..) = probe(&router, "/readyz").await;
	assert_eq!(status, StatusCode::OK);

	// A check can fail again, and asking for it by name again keeps its state.
	listen.fail();
	assert!(!readiness.check("listen").is_passing());
	assert!(readiness.check("migrations").is_passing());
	assert_eq!(readiness.failing(), ["listen"]);
	let (status, body, _) = probe(&router, "/healthz").await;
	assert_eq!(
		(status, body.as_str()),
		(StatusCode::OK, "ok"),
		"health ignores readiness"
	);
}

#[tokio::test]
async fn shutting_down_fails_readiness_for_good() {
	let readiness = Readiness::new();
	readiness.check("migrations").pass();
	readiness.shut_down();
	assert_eq!(readiness.failing(), [SHUTDOWN_CHECK]);
	let (status, body, _) = probe(&readiness.routes(), "/readyz").await;
	assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
	assert!(body.contains(r#""failing":["shutdown"]"#), "{body}");
}

#[test]
fn the_defaults_fit_inside_a_thirty_second_kill_timeout() {
	let shutdown = GracefulShutdown::default();
	let debug = format!("{shutdown:?}");
	assert!(debug.contains("drain_delay: 5s"), "{debug}");
	assert!(debug.contains("grace_period: Some(20s)"), "{debug}");
	assert!(
		GracefulShutdown::DEFAULT_DRAIN_DELAY + GracefulShutdown::DEFAULT_GRACE_PERIOD
			< Duration::from_secs(30)
	);
	let debug = format!("{:?}", GracefulShutdown::new().without_grace_period());
	assert!(debug.contains("grace_period: None"), "{debug}");
}

/// One HTTP/1.1 request on a fresh connection; the status code and body.
async fn get_once(address: SocketAddr, path: &str) -> std::io::Result<(u16, String)> {
	let mut stream = TcpStream::connect(address).await?;
	stream
		.write_all(
			format!("GET {path} HTTP/1.1\r\nhost: test\r\nconnection: close\r\n\r\n").as_bytes(),
		)
		.await?;
	let mut response = String::new();
	stream.read_to_string(&mut response).await?;
	let status = response
		.split(' ')
		.nth(1)
		.and_then(|code| code.parse().ok())
		.unwrap_or(0);
	let body = response
		.split_once("\r\n\r\n")
		.map(|(_, body)| body.to_owned())
		.unwrap_or_default();
	Ok((status, body))
}

async fn eventually(what: &str, mut condition: impl AsyncFnMut() -> bool) {
	let start = Instant::now();

	while !condition().await {
		assert!(
			start.elapsed() < Duration::from_secs(10),
			"timed out: {what}"
		);
		tokio::time::sleep(Duration::from_millis(10)).await;
	}
}

/// The order a load balancer depends on: readiness fails first, the server
/// keeps accepting through the drain delay, then it stops accepting and the
/// request in flight still completes.
#[tokio::test]
async fn shutdown_fails_readiness_then_drains_then_finishes_in_flight_requests() {
	let readiness = Readiness::new();
	readiness.check("migrations").pass();
	let entered = Arc::new(Notify::new());
	let release = Arc::new(Notify::new());
	let slow = {
		let entered = Arc::clone(&entered);
		let release = Arc::clone(&release);
		get(move || {
			let entered = Arc::clone(&entered);
			let release = Arc::clone(&release);
			async move {
				entered.notify_one();
				release.notified().await;
				"done"
			}
		})
	};
	let app = Router::new().route("/slow", slow).merge(readiness.routes());
	let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
	let address = listener.local_addr().expect("address");

	let (stop, stopped) = oneshot::channel::<()>();
	let drain_delay = Duration::from_millis(400);
	let shutdown = GracefulShutdown::new()
		.readiness(&readiness)
		.drain_delay(drain_delay)
		.grace_period(Duration::from_secs(10))
		.signal(async move {
			let _ = stopped.await;
		});
	let server = tokio::spawn(serve_listener_with(listener, app, shutdown));

	assert_eq!(get_once(address, "/readyz").await.expect("ready").0, 200);
	let in_flight = tokio::spawn(get_once(address, "/slow"));
	entered.notified().await;

	let signalled = Instant::now();
	stop.send(()).expect("signal");
	eventually("readiness fails", async || !readiness.is_ready()).await;
	// Still accepting during the drain delay, and saying it's going away.
	let (status, body) = get_once(address, "/readyz").await.expect("draining");
	assert_eq!(status, 503);
	assert!(body.contains("shutdown"), "{body}");
	assert!(!server.is_finished());

	eventually("the listener closes", async || {
		get_once(address, "/healthz").await.is_err()
	})
	.await;
	assert!(
		signalled.elapsed() >= drain_delay,
		"stopped accepting {:?} after the signal, before the drain delay",
		signalled.elapsed()
	);
	assert!(!server.is_finished(), "a request is still in flight");

	release.notify_one();
	let (status, body) = in_flight.await.expect("join").expect("in flight");
	assert_eq!((status, body.as_str()), (200, "done"));
	server.await.expect("join").expect("a clean stop");
}

#[tokio::test]
async fn the_server_stops_waiting_for_requests_after_the_grace_period() {
	let entered = Arc::new(Notify::new());
	let app = Router::new().route(
		"/forever",
		get({
			let entered = Arc::clone(&entered);
			move || {
				let entered = Arc::clone(&entered);
				async move {
					entered.notify_one();
					std::future::pending::<&str>().await
				}
			}
		}),
	);
	let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
	let address = listener.local_addr().expect("address");
	let (stop, stopped) = oneshot::channel::<()>();
	let shutdown = GracefulShutdown::new()
		.drain_delay(Duration::ZERO)
		.grace_period(Duration::from_millis(200))
		.signal(async move {
			let _ = stopped.await;
		});
	let server = tokio::spawn(serve_listener_with(listener, app, shutdown));
	let stuck = tokio::spawn(get_once(address, "/forever"));

	entered.notified().await;

	let signalled = Instant::now();
	stop.send(()).expect("signal");
	let error = tokio::time::timeout(Duration::from_secs(10), server)
		.await
		.expect("stops after the grace period")
		.expect("join")
		.expect_err("the request was cut off");
	assert!(signalled.elapsed() >= Duration::from_millis(200));
	assert!(matches!(
		error,
		ServeError::GracePeriodElapsed { grace_period } if grace_period == Duration::from_millis(200)
	));
	assert_eq!(
		error.to_string(),
		"stopped with requests still in flight: the 0.2s grace period ran out"
	);
	// The stuck connection ends with the runtime, as it would when `main`
	// returns the error.
	assert!(!stuck.is_finished());
	stuck.abort();
}

#[tokio::test]
async fn an_idle_server_stops_at_once_without_a_grace_period() {
	let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
	let address = listener.local_addr().expect("address");
	drop(listener);
	let (stop, stopped) = oneshot::channel::<()>();
	let shutdown = GracefulShutdown::new()
		.drain_delay(Duration::ZERO)
		.without_grace_period()
		.signal(async move {
			let _ = stopped.await;
		});
	let server = tokio::spawn(serve_router_with(address, Router::new(), shutdown));
	eventually("the server listens", async || {
		TcpStream::connect(address).await.is_ok()
	})
	.await;
	stop.send(()).expect("signal");

	tokio::time::timeout(Duration::from_secs(10), server)
		.await
		.expect("stops")
		.expect("join")
		.expect("a clean stop");
}
