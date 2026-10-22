//! The server's routes: every page rendered by leptos, and the built site
//! behind them.

use axum::Router;
use leptos::prelude::*;
use leptos_axum::LeptosRoutes;
use leptos_axum::generate_route_list;

use crate::app::App;
use crate::app::shell;

/// Renders every route of [`App`] into [`shell`], and serves anything else
/// from `options.site_root`: the wasm, its chunks, and the stylesheet.
pub fn router(options: LeptosOptions) -> Router {
	let site_root = std::path::PathBuf::from(options.site_root.as_ref());
	let routes = generate_route_list(App);
	Router::new()
		.leptos_routes(&options, routes, {
			let options = options.clone();
			move || shell(options.clone())
		})
		.fallback_service(leptoskit_axum::static_files(&site_root))
		.with_state(options)
}
