//! The server's routes: every page rendered by leptos, the built site behind
//! them, and the limits that keep one bad request from costing more than one
//! request's worth of time and memory.

use std::time::Duration;

use axum::BoxError;
use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::http::StatusCode;
use axum::routing::get;
use leptos::prelude::*;
use leptos_axum::LeptosRoutes;
use leptos_axum::generate_route_list;
use tower::ServiceBuilder;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

use crate::app::App;
use crate::app::shell;

/// How long a render or server function may take before the client gets a
/// `408` instead of a connection held open.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The largest request body a server function may read. Island props and
/// form posts are small; this is far above any of them.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// How many requests may be in flight at once. Past it, the server answers
/// `503` immediately rather than queueing work it cannot finish in time.
pub const MAX_IN_FLIGHT: usize = 512;

/// Renders every route of [`App`] into [`shell`], answers the load balancer at
/// `/_leptoskit/health`, and serves anything else from `options.site_root`:
/// the wasm, its chunks, and the stylesheet.
///
/// Every request is bounded: by [`REQUEST_TIMEOUT`], by [`MAX_BODY_BYTES`]
/// on the body stream itself (so the cap holds for leptos server functions,
/// which read the body without axum's extractors), and by [`MAX_IN_FLIGHT`]
/// with load shedding.
pub fn router(options: LeptosOptions) -> Router {
	let site_root = std::path::PathBuf::from(options.site_root.as_ref());
	let routes = generate_route_list(App);
	Router::new()
		.route("/_leptoskit/health", get(|| async { "ok" }))
		.leptos_routes(&options, routes, {
			let options = options.clone();
			move || shell(options.clone())
		})
		.fallback_service(leptoskit_axum::static_files(&site_root))
		.layer(TimeoutLayer::with_status_code(
			StatusCode::REQUEST_TIMEOUT,
			REQUEST_TIMEOUT,
		))
		.layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
		.layer(
			ServiceBuilder::new()
				.layer(HandleErrorLayer::new(|_: BoxError| {
					async { StatusCode::SERVICE_UNAVAILABLE }
				}))
				.load_shed()
				.concurrency_limit(MAX_IN_FLIGHT),
		)
		.with_state(options)
}
