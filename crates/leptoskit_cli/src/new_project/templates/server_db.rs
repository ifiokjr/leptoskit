//! Web SSR entrypoint. Application UI and backend handlers live in their
//! crates.
//!
//! Everything the server is built from comes through `leptoskit::server`:
//! `axum`, `tokio`, and `leptos_axum` are re-exported there, so this package
//! depends on `leptoskit` alone.

use __CRATE___app::App;
use __CRATE___app::shell;
use leptoskit::rpc::server::Roles;
use leptoskit::server::GracefulShutdown;
use leptoskit::server::Readiness;
use leptoskit::server::leptos_axum::LeptosRoutes;
use leptoskit::server::leptos_axum::generate_route_list;
use leptoskit::server::tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let mut options = leptoskit::leptos::config::get_configuration(None)?.leptos_options;
	options.output_name = "__CRATE___app".into();

	if std::env::var_os("LEPTOS_SITE_ROOT").is_none() {
		options.site_root = "dist".into();
	}

	// `--role` (or `LEPTOSKIT_ROLE`) picks which parts of the app this
	// process runs, every one unless it says: `web` serves the pages and the
	// API, and `live`, `worker` and `capture` are for the live sockets, the
	// job workers and replication capture once the app has them, so one
	// binary can run as any part of a larger deployment.
	let roles = Roles::from_env_and_args()?;
	let address = options.site_addr;
	let package_dir = format!("{}/{}", options.site_root, options.site_pkg_dir);
	let assets_dir = format!("{}/assets", options.site_root);
	// `/readyz` answers 200 once every check passes, so a load balancer only
	// sends traffic to an instance that can serve it.
	let readiness = Readiness::new();
	let migrated = readiness.check("migrations");
	// The database, migrated before the first request.
	let url = __DATABASE_URL__;
	let db = leptoskit::db::Db::connect(&url).await?;

	for name in db.migrate(__CRATE___db::MIGRATIONS).await? {
		println!("applied migration {name}");
	}

	migrated.pass();

	let routes = generate_route_list(App);
	let app_options = options.clone();
	// The pages and the API, with the `web` role. The built files are served
	// with their precompressed brotli and gzip copies when the browser
	// accepts them. Every process answers `/healthz` and `/readyz`.
	let app = if roles.web() {
		__CRATE___backend::router(db)
			.nest_service(
				"/pkg",
				leptoskit::server::static_files(package_dir.as_ref()),
			)
			.nest_service(
				"/assets",
				leptoskit::server::static_files(assets_dir.as_ref()),
			)
			.leptos_routes(&options, routes, move || shell(app_options.clone()))
			.with_state(options)
	} else {
		leptoskit::server::axum::Router::new()
	}
	.merge(readiness.routes());
	let listener = tokio::net::TcpListener::bind(address).await?;

	println!("listening on http://{address}");
	// Serves until Ctrl-C or the SIGTERM a container runtime sends. Then
	// `/readyz` fails, and after a few seconds for the load balancer to
	// notice, the server stops accepting and finishes the requests in flight.
	let shutdown = GracefulShutdown::new().readiness(&readiness);
	leptoskit::server::serve_listener_with(listener, app, shutdown).await?;

	Ok(())
}
