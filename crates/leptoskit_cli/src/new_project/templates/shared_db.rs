//! The app's contracts, shared by the app, the backend, and every generated
//! client: the types that cross the wire, and the queries and mutations the
//! server runs.
//!
//! Each `#[rpc::query]` and `#[rpc::mutation]` compiles to an argument
//! struct on every target, such as `Notes {}` and `AddNote { body }`, which
//! any screen sends with `leptoskit::rpc::client::call`. The server build
//! (the `server` feature) also keeps the bodies, so no database code reaches
//! the browser or a native app. `#[api]` publishes each one, so the RPC spec
//! and the clients `leptoskit rpc generate` writes describe it.

pub use __CRATE___db::PublicNote;
use leptoskit::rpc;
use leptoskit::rpc::api;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// The response returned by `GET /api/health`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Health {
	/// Whether the backend is ready to receive requests.
	pub ready: bool,
}

/// The 50 newest notes, newest first.
///
/// Subscribe to it over the live socket and every added note arrives as a
/// change to the list.
#[api(crate = "leptoskit::rpc", tag = "notes")]
#[rpc::query]
pub async fn notes(cx: &rpc::QueryCx) -> rpc::Result<Vec<PublicNote>> {
	let notes = __CRATE___db::Note::query()
		.order_by(|note| note.created_at.desc())
		.limit(50)
		.all(cx.db())
		.await?;
	Ok(notes.into_iter().map(PublicNote::from).collect())
}

/// Adds a note, and returns it.
///
/// Fails with `Invalid` when the body is empty or longer than 500
/// characters.
///
/// # Arguments
///
/// * `body` - What the note says.
#[api(crate = "leptoskit::rpc", tag = "notes")]
#[rpc::mutation]
pub async fn add_note(cx: &rpc::MutationCx, body: String) -> rpc::Result<PublicNote> {
	let note = __CRATE___db::NewNote { body }.insert(cx.tx()).await?;
	Ok(note.into())
}
