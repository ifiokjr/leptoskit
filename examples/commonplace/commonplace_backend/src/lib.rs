//! Commonplace's JSON API, kept out of every client bundle.
//!
//! Every route needs a signed-in person. The auth layer resolves the session
//! from the cookie (the website) or `Authorization: Bearer` (the native apps
//! and the extension), and hands each handler a `Db` acting for that person,
//! so the tables' policies decide what each request can see and change.

mod clips;
mod error;
mod notes;
mod projects;

pub use error::Failure;
use leptoskit::server::axum;
use leptoskit::server::axum::routing::get;
use leptoskit::server::axum::routing::patch;
use leptoskit::server::axum::routing::post;

/// The API's routes. Add the auth service's router and layer around them, as
/// the server does.
pub fn router<S>() -> axum::Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	axum::Router::new()
		.route("/api/health", get(|| async { "ok" }))
		.route("/api/projects", get(projects::list).post(projects::create))
		.route(
			"/api/projects/{project}",
			get(projects::detail).delete(projects::remove),
		)
		.route("/api/projects/{project}/notes", post(notes::create))
		.route(
			"/api/notes/{note}",
			patch(notes::update).delete(notes::remove),
		)
		.route("/api/clips", get(clips::on_page).post(clips::create))
		.route(
			"/api/clips/{clip}",
			patch(clips::update).delete(clips::remove),
		)
}
