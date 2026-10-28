//! Server implementations of the shared RPC contracts.

use __CRATE___db::NewNote;
use __CRATE___db::Note;
use __CRATE___db::PublicNote;
use __CRATE___shared::Health;
use leptoskit::db::Db;
use leptoskit::db::DbError;
use leptoskit::server::axum::Extension;
use leptoskit::server::axum::Json;
use leptoskit::server::axum::http::StatusCode;
use serde::Deserialize;

/// Reports backend availability using the app's shared response model.
pub async fn health() -> Json<Health> {
	Json(Health { ready: true })
}

/// The 50 newest notes, as clients may see them.
pub async fn notes(Extension(db): Extension<Db>) -> Result<Json<Vec<PublicNote>>, StatusCode> {
	let notes = Note::query()
		.order_by(|note| note.created_at.desc())
		.limit(50)
		.all(&db)
		.await
		.map_err(status)?;

	Ok(Json(notes.into_iter().map(PublicNote::from).collect()))
}

/// The body of `POST /api/notes`.
#[derive(Deserialize)]
pub struct AddNote {
	/// What the note says.
	pub body: String,
}

/// Adds a note and returns it.
pub async fn add_note(
	Extension(db): Extension<Db>,
	Json(input): Json<AddNote>,
) -> Result<Json<PublicNote>, StatusCode> {
	let note = NewNote { body: input.body }
		.insert(&db)
		.await
		.map_err(status)?;
	Ok(Json(note.into()))
}

/// A database failure as an HTTP status: broken rules are the client's to
/// fix; anything else is the server's.
fn status(error: DbError) -> StatusCode {
	match error {
		DbError::Invalid(_) | DbError::Check { .. } => StatusCode::UNPROCESSABLE_ENTITY,
		DbError::NotFound { .. } => StatusCode::NOT_FOUND,
		DbError::Conflict { .. } => StatusCode::CONFLICT,
		_ => StatusCode::INTERNAL_SERVER_ERROR,
	}
}
