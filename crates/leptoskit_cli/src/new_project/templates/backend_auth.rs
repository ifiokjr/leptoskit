//! Backend implementation, kept out of browser and native application bundles.

use leptoskit::server::axum;

pub mod rpc;

/// Installs the backend's RPC endpoints in the SSR server. Handlers reach the
/// database through `Extension<Db>`, which the sign-in layer adds to every
/// request: a database that acts for whoever is signed in, so the tables'
/// policies apply.
pub fn router<S>() -> axum::Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	axum::Router::new()
		.route("/api/health", axum::routing::get(rpc::health))
		.route(
			"/api/notes",
			axum::routing::get(rpc::notes).post(rpc::add_note),
		)
}
