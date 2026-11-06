//! The app's server, built with the service's bodies linked in, as
//! `cargo test --workspace` and `lk build` build it: the service's operations
//! are still the service's. The app doesn't serve them, and calling one
//! sends it to the service: as an anonymous caller with `call`, which is
//! what pages render with, and as the app only with `call_as_app`.

#![allow(
	unsafe_code,
	reason = "the test sets the variables `leptoskit dev` gives the app's server"
)]

use std::collections::BTreeMap;

use analytics_service::Capture;
use analytics_service::Recent;
use leptoskit::server::axum;
use leptoskit_rpc as rpc;
use rpc::Operation;
use rpc::client::call;
use rpc::client::call_as_app;
use rpc::server::Anonymous;
use rpc::server::ServiceCallers;

const TOKEN: &str = "app-token";

#[tokio::test(flavor = "multi_thread")]
async fn the_apps_server_sends_service_operations_to_the_service() {
	// The service's body is linked into this build (the dev-dependency turns
	// on its `service` feature), and registered, under its own service.
	assert_eq!(<Capture as Operation>::SERVICE, Some("analytics"));
	let app_scope = rpc::server::registrations_in(None).unwrap();
	assert!(
		!app_scope
			.iter()
			.any(|registration| registration.name == "capture")
	);
	let service_scope = rpc::server::registrations_in(Some("analytics")).unwrap();
	assert!(
		service_scope
			.iter()
			.any(|registration| registration.name == "capture")
	);

	// The service, on a port of its own.
	let service_db = leptoskit_db::testing::sqlite_migrated(analytics_service::MIGRATIONS).await;
	let (service, live) = analytics_service::routes_with(
		service_db,
		None,
		ServiceCallers::new(Anonymous).caller(TOKEN, rpc::server::APP_CALLER),
	);
	live.start().await.unwrap();
	let service_address = serve(service).await;

	// The app's own operation server, installed last: this process is the
	// app's. It doesn't route the service's operations.
	let app = rpc::server::Server::new(leptoskit_db::testing::sqlite().await);
	let app_address = serve(app.router()).await;
	let status = reqwest::Client::new()
		.post(format!("http://{app_address}/api/capture"))
		.json(&serde_json::json!({ "event": "signup", "distinct_id": "v" }))
		.send()
		.await
		.unwrap()
		.status();
	assert_eq!(status, 404, "the app doesn't serve the service's operation");

	// What `lk dev` and `lk serve` set on the app's server.
	// SAFETY: nextest runs each test in its own process, and these are set
	// before anything reads them.
	unsafe {
		std::env::set_var(
			"LEPTOSKIT_SERVICE_ANALYTICS_URL",
			format!("http://{service_address}"),
		);
		std::env::set_var("LEPTOSKIT_SERVICE_ANALYTICS_TOKEN", TOKEN);
	}

	call(Capture {
		event: "signup".to_owned(),
		distinct_id: "visitor-1".to_owned(),
		properties: BTreeMap::new(),
	})
	.await
	.expect("the capture reaches the service");

	// `recent` is the app's alone. An ordinary call, like a page rendering
	// on the server, goes anonymously, though the server holds the token, so
	// nothing app-only ends up in a page's HTML.
	let refused = call(Recent { limit: 10 }).await.unwrap_err();
	assert!(
		refused.message().contains("only the app reads"),
		"{refused}"
	);

	// `call_as_app` presents the token, which identifies this server as the
	// app.
	let recent = call_as_app(Recent { limit: 10 })
		.await
		.expect("the app may read events");
	assert_eq!(recent.len(), 1);
	assert_eq!(recent[0].name, "signup");
}

async fn serve(router: axum::Router) -> std::net::SocketAddr {
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
	address
}
