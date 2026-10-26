//! Backend implementation, kept out of browser and native application bundles.

use leptoskit::server::axum;

pub mod rpc;

/// Installs the backend's RPC endpoints in the SSR server.
pub fn router<S>() -> axum::Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	axum::Router::new().route("/api/health", axum::routing::get(rpc::health))
}
