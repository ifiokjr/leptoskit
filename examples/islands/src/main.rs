//! The server: renders every page, then serves the built site behind the
//! routes.
//!
//! `leptoskit dev` and `leptoskit serve` start it with the `LEPTOS_*`
//! environment pointing at the built site.

use islands_example::server::router;
use leptos::prelude::*;
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
	println!("listening on http://{address}");
	// Ctrl-C, or the SIGTERM a container runtime sends, finishes the requests
	// in flight, then exits cleanly.
	leptoskit::server::serve_listener(listener, router(options))
		.await
		.expect("serve");
}
