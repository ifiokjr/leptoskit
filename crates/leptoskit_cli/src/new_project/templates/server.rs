//! Web SSR entrypoint. Application UI and backend handlers live in their
//! crates.

use __CRATE___app::App;
use __CRATE___app::shell;
use leptos_axum::LeptosRoutes;
use leptos_axum::generate_route_list;
use tower_http::services::ServeDir;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let mut options = leptos::config::get_configuration(None)?.leptos_options;
	options.output_name = "__CRATE___app".into();

	if std::env::var_os("LEPTOS_SITE_ROOT").is_none() {
		options.site_root = "dist".into();
	}

	let address = options.site_addr;
	let package_dir = format!("{}/{}", options.site_root, options.site_pkg_dir);
	let assets_dir = format!("{}/assets", options.site_root);
	let routes = generate_route_list(App);
	let app_options = options.clone();
	let app = __CRATE___backend::router()
		.nest_service("/pkg", ServeDir::new(package_dir))
		.nest_service("/assets", ServeDir::new(assets_dir))
		.leptos_routes(&options, routes, move || shell(app_options.clone()))
		.with_state(options);
	let listener = tokio::net::TcpListener::bind(address).await?;

	println!("listening on http://{address}");
	axum::serve(listener, app).await?;

	Ok(())
}
