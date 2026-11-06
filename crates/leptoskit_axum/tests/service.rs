//! A service served from its environment: health, readiness, its public
//! prefix, and a private route behind the token.

use std::time::Duration;

use axum::Router;
use axum::routing::get;
use leptoskit_axum::GracefulShutdown;
use leptoskit_axum::service::ServiceEnv;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::oneshot;

/// A free port: bound, read, and released for the service to take.
fn free_address() -> String {
	let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
	listener.local_addr().expect("address").to_string()
}

async fn get_status(address: &str, path: &str, authorization: Option<&str>) -> u16 {
	let mut stream = TcpStream::connect(address).await.expect("connect");
	let authorization = authorization
		.map(|value| format!("Authorization: {value}\r\n"))
		.unwrap_or_default();
	let request = format!(
		"GET {path} HTTP/1.1\r\nHost: {address}\r\n{authorization}Connection: close\r\n\r\n"
	);
	stream.write_all(request.as_bytes()).await.expect("write");
	let mut response = String::new();
	stream.read_to_string(&mut response).await.expect("read");
	response
		.split_whitespace()
		.nth(1)
		.and_then(|status| status.parse().ok())
		.expect("status line")
}

#[tokio::test]
async fn a_service_serves_its_prefix_health_and_private_routes() {
	let address = free_address();
	let env = ServiceEnv::from_vars([
		("LEPTOSKIT_SERVICE_NAME", "analytics"),
		("LEPTOSKIT_SERVICE_ADDR", address.as_str()),
		("LEPTOSKIT_SERVICE_TOKEN", "secret"),
	])
	.expect("env");
	let migrations = env.readiness().check("migrations");
	let public = Router::new().route("/hello", get(|| async { "hello" }));
	let private = Router::new()
		.route("/summary", get(|| async { "{}" }))
		.route_layer(env.require_token());
	let app = Router::new().nest(&env.public_prefix(), public.merge(private));

	let (stop, stopped) = oneshot::channel::<()>();
	let shutdown = GracefulShutdown::new()
		.signal(async move {
			let _ = stopped.await;
		})
		.drain_delay(Duration::ZERO);
	let server = tokio::spawn(env.serve_with(app, shutdown));

	// Wait for the listener.
	for _ in 0..100 {
		if TcpStream::connect(&address).await.is_ok() {
			break;
		}
		tokio::time::sleep(Duration::from_millis(20)).await;
	}

	assert_eq!(get_status(&address, "/healthz", None).await, 200);
	assert_eq!(get_status(&address, "/readyz", None).await, 503);
	migrations.pass();
	assert_eq!(get_status(&address, "/readyz", None).await, 200);
	assert_eq!(
		get_status(&address, "/_leptoskit/services/analytics/hello", None).await,
		200
	);
	assert_eq!(get_status(&address, "/hello", None).await, 404);
	assert_eq!(
		get_status(&address, "/_leptoskit/services/analytics/summary", None).await,
		401
	);
	assert_eq!(
		get_status(
			&address,
			"/_leptoskit/services/analytics/summary",
			Some("Bearer secret")
		)
		.await,
		200
	);

	stop.send(()).expect("stop");
	server.await.expect("join").expect("served cleanly");
}
