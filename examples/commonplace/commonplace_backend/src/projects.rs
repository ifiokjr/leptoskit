//! Projects: the library, one project, adding and removing them.

use commonplace_db::Clip;
use commonplace_db::NewProject;
use commonplace_db::Note;
use commonplace_db::Project;
use commonplace_db::PublicProject;
use commonplace_db::Source;
use commonplace_shared::NewProjectInput;
use commonplace_shared::ProjectDetail;
use commonplace_shared::ProjectSummary;
use leptoskit::auth::SignedIn;
use leptoskit::db::Db;
use leptoskit::db::Id;
use leptoskit::server::axum::Extension;
use leptoskit::server::axum::Json;
use leptoskit::server::axum::extract::Path;
use leptoskit::server::axum::http::StatusCode;

use crate::error::Answer;
use crate::error::Failure;

/// `GET /api/projects`: every project, most recently changed first.
pub async fn list(_me: SignedIn, Extension(db): Extension<Db>) -> Answer<Vec<ProjectSummary>> {
	let projects = Project::query()
		.order_by(|project| project.updated_at.desc())
		.all(&db)
		.await?;
	let mut summaries = Vec::with_capacity(projects.len());
	for project in projects {
		let id = project.id;
		summaries.push(ProjectSummary {
			sources: Source::query()
				.filter(|source| source.project_id.eq(id))
				.count(&db)
				.await?,
			clips: Clip::query()
				.filter(|clip| clip.project_id.eq(id))
				.count(&db)
				.await?,
			project: project.into(),
		});
	}
	Ok(Json(summaries))
}

/// `POST /api/projects`.
pub async fn create(
	me: SignedIn,
	Extension(db): Extension<Db>,
	Json(input): Json<NewProjectInput>,
) -> Answer<PublicProject> {
	let title = input.title.trim().to_owned();
	if title.is_empty() {
		return Err(Failure::invalid("A project needs a title."));
	}
	let project = NewProject {
		owner_id: me.account,
		title,
		question: input
			.question
			.map(|question| question.trim().to_owned())
			.filter(|question| !question.is_empty()),
	}
	.insert(&db)
	.await?;
	Ok(Json(project.into()))
}

/// `GET /api/projects/{project}`: the project with its sources, clips, and
/// notes.
pub async fn detail(
	_me: SignedIn,
	Extension(db): Extension<Db>,
	Path(id): Path<Id<Project>>,
) -> Answer<ProjectDetail> {
	let project = Project::find(&db, id)
		.await?
		.ok_or_else(|| Failure::not_found("project"))?;
	let sources = Source::query()
		.filter(|source| source.project_id.eq(id))
		.order_by(|source| source.accessed_at.desc())
		.all(&db)
		.await?;
	let clips = Clip::query()
		.filter(|clip| clip.project_id.eq(id))
		.order_by(|clip| clip.created_at.desc())
		.all(&db)
		.await?;
	let notes = Note::query()
		.filter(|note| note.project_id.eq(id))
		.order_by(|note| note.created_at.desc())
		.all(&db)
		.await?;

	Ok(Json(ProjectDetail {
		project: project.into(),
		sources: sources.into_iter().map(Into::into).collect(),
		clips: clips.into_iter().map(Into::into).collect(),
		notes: notes.into_iter().map(Into::into).collect(),
	}))
}

/// `DELETE /api/projects/{project}`, with everything in it.
pub async fn remove(
	_me: SignedIn,
	Extension(db): Extension<Db>,
	Path(id): Path<Id<Project>>,
) -> Result<StatusCode, Failure> {
	let removed = Project::query()
		.filter(|project| project.id.eq(id))
		.destroy(&db)
		.await?;
	if removed == 0 {
		return Err(Failure::not_found("project"));
	}
	Ok(StatusCode::NO_CONTENT)
}

/// Marks a project as changed, so the library lists it first.
pub(crate) async fn touch(db: &Db, id: Id<Project>) -> Result<(), Failure> {
	let mut project = Project::find(db, id)
		.await?
		.ok_or_else(|| Failure::not_found("project"))?;
	project.save(db).await?;
	Ok(())
}
