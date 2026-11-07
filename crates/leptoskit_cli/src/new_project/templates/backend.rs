//! Backend implementation, kept out of browser and native application bundles.
//!
//! Its server functions, such as [`rpc::health`], are served by the
//! server's `leptos_routes`, so [`router`] holds only the routes you add.

use leptoskit::server::axum;

pub mod rpc;

/// The backend's own axum routes, for endpoints that aren't server
/// functions.
pub fn router<S>() -> axum::Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	axum::Router::new()
}
