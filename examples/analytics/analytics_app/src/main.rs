//! The server: renders every page, then serves the built site behind the
//! routes.
//!
//! `leptoskit dev` and `leptoskit serve` start it with the `LEPTOS_*`
//! environment pointing at the built site.

use analytics_app::server::router;
use leptos::prelude::*;
use leptoskit::server::GracefulShutdown;
use leptoskit::server::Readiness;
use leptoskit::server::tokio;

#[tokio::main]
async fn main() {
	let options = get_configuration(None)
		.expect("leptoskit sets the LEPTOS_* environment")
		.leptos_options;
	let address = options.site_addr;
	let listener = tokio::net::TcpListener::bind(address)
		.await
		.expect("bind the server address");
	// `/healthz` and `/readyz`, which every deployment's health checks call.
	let readiness = Readiness::new();
	let app = router(options).merge(readiness.routes());
	println!("listening on http://{address}");
	// Ctrl-C, or the SIGTERM a container runtime sends, fails `/readyz`, gives
	// the load balancer a few seconds to notice, then finishes the requests in
	// flight and exits cleanly.
	let shutdown = GracefulShutdown::new().readiness(&readiness);
	leptoskit::server::serve_listener_with(listener, app, shutdown)
		.await
		.expect("serve");
}
