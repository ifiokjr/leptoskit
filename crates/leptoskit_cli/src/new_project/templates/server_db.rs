//! Web SSR entrypoint. Application UI and backend handlers live in their
//! crates.
//!
//! Everything the server is built from comes through `leptoskit::server`:
//! `axum`, `tokio`, and `leptos_axum` are re-exported there, so this package
//! depends on `leptoskit` alone.

use __CRATE___app::App;
use __CRATE___app::shell;
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

	let address = options.site_addr;
	let package_dir = format!("{}/{}", options.site_root, options.site_pkg_dir);
	let assets_dir = format!("{}/assets", options.site_root);
	// The database, migrated before the first request.
	let url = __DATABASE_URL__;
	let db = leptoskit::db::Db::connect(&url).await?;

	for name in db.migrate(__CRATE___db::MIGRATIONS).await? {
		println!("applied migration {name}");
	}

	let routes = generate_route_list(App);
	let app_options = options.clone();
	// The built files are served with their precompressed brotli and gzip
	// copies when the browser accepts them.
	let app = __CRATE___backend::router(db)
		.nest_service(
			"/pkg",
			leptoskit::server::static_files(package_dir.as_ref()),
		)
		.nest_service(
			"/assets",
			leptoskit::server::static_files(assets_dir.as_ref()),
		)
		.leptos_routes(&options, routes, move || shell(app_options.clone()))
		.with_state(options);
	let listener = tokio::net::TcpListener::bind(address).await?;

	println!("listening on http://{address}");
	// Serves until Ctrl-C or the SIGTERM a container runtime sends, finishing
	// the requests in flight first.
	leptoskit::server::serve_listener(listener, app).await?;

	Ok(())
}
