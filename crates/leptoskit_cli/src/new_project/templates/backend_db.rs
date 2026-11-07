//! Backend implementation, kept out of browser and native application bundles.
//!
//! The app's queries and mutations are declared in `__CRATE___shared`, so the
//! browser and the native apps call them by name; their bodies compile only
//! here, on the server, through that crate's `server` feature. The backend's
//! own server functions, such as [`rpc::health`], are served by the server's
//! `leptos_routes`.

use leptoskit::db::Db;
use leptoskit::rpc::server::Server;
use leptoskit::server::axum;

// Linking the shared crate registers its queries and mutations, for the
// routes below and for the spec `leptoskit rpc generate` exports.
extern crate __CRATE___shared as _;

pub mod rpc;

/// The app's queries and mutations, run on `db`. Every caller is anonymous
/// until [`Server::identify`] says who they are; [`Server::live`] serves the
/// same operations over the live socket.
pub fn operations(db: Db) -> Server {
	Server::new(db)
}

/// Installs `POST /api/<name>` for every query and mutation in
/// `operations`, and the backend's own axum routes.
pub fn router<S>(operations: &Server) -> axum::Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	operations.router()
}
