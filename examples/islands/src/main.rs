//! The server: renders every page, then serves the built site behind the
//! routes.
//!
//! `leptoskit dev` and `leptoskit serve` start it with the `LEPTOS_*`
//! environment pointing at the built site.

use axum::Router;
use islands_example::app::App;
use islands_example::app::shell;
use leptos::prelude::*;
use leptos_axum::LeptosRoutes;
use leptos_axum::generate_route_list;

#[tokio::main]
async fn main() {
	let options = get_configuration(None)
		.expect("leptoskit sets the LEPTOS_* environment")
		.leptos_options;
	let address = options.site_addr;
	let site_root = std::path::PathBuf::from(options.site_root.as_ref());
	let routes = generate_route_list(App);
	let app = Router::new()
		.leptos_routes(&options, routes, {
			let options = options.clone();
			move || shell(options.clone())
		})
		.fallback_service(leptoskit_axum::static_files(&site_root))
		.with_state(options);
	let listener = tokio::net::TcpListener::bind(address)
		.await
		.expect("bind the server address");
	println!("listening on http://{address}");
	axum::serve(listener, app).await.expect("serve");
}
