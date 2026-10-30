//! Notes in a project.

use commonplace_db::NewNote;
use commonplace_db::Note;
use commonplace_db::Project;
use commonplace_db::PublicNote;
use commonplace_shared::NoteInput;
use leptoskit::auth::SignedIn;
use leptoskit::db::Db;
use leptoskit::db::Id;
use leptoskit::server::axum::Extension;
use leptoskit::server::axum::Json;
use leptoskit::server::axum::extract::Path;
use leptoskit::server::axum::http::StatusCode;

use crate::error::Answer;
use crate::error::Failure;

fn body(input: NoteInput) -> Result<String, Failure> {
	let body = input.body.trim().to_owned();
	if body.is_empty() {
		return Err(Failure::invalid("A note can't be empty."));
	}
	Ok(body)
}

/// `POST /api/projects/{project}/notes`.
pub async fn create(
	me: SignedIn,
	Extension(db): Extension<Db>,
	Path(project_id): Path<Id<Project>>,
	Json(input): Json<NoteInput>,
) -> Answer<PublicNote> {
	Project::find(&db, project_id)
		.await?
		.ok_or_else(|| Failure::not_found("project"))?;
	let note = NewNote {
		owner_id: me.account,
		project_id,
		body: body(input)?,
	}
	.insert(&db)
	.await?;
	crate::projects::touch(&db, project_id).await?;
	Ok(Json(note.into()))
}

/// `PATCH /api/notes/{note}`.
pub async fn update(
	_me: SignedIn,
	Extension(db): Extension<Db>,
	Path(id): Path<Id<Note>>,
	Json(input): Json<NoteInput>,
) -> Answer<PublicNote> {
	let mut note = Note::find(&db, id)
		.await?
		.ok_or_else(|| Failure::not_found("note"))?;
	note.body = body(input)?;
	note.save(&db).await?;
	Ok(Json(note.into()))
}

/// `DELETE /api/notes/{note}`.
pub async fn remove(
	_me: SignedIn,
	Extension(db): Extension<Db>,
	Path(id): Path<Id<Note>>,
) -> Result<StatusCode, Failure> {
	let removed = Note::query()
		.filter(|note| note.id.eq(id))
		.destroy(&db)
		.await?;
	if removed == 0 {
		return Err(Failure::not_found("note"));
	}
	Ok(StatusCode::NO_CONTENT)
}
