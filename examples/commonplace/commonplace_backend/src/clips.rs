//! Clips: saving a quote from a page, editing its note and tags, and finding
//! the clips taken from a page.

use commonplace_db::Clip;
use commonplace_db::NewClip;
use commonplace_db::NewSource;
use commonplace_db::Project;
use commonplace_db::PublicClip;
use commonplace_db::Source;
use commonplace_shared::ClipChanges;
use commonplace_shared::ClipOnPage;
use commonplace_shared::NewClipInput;
use commonplace_shared::tags;
use leptoskit::auth::SignedIn;
use leptoskit::db::Db;
use leptoskit::db::Id;
use leptoskit::server::axum::Extension;
use leptoskit::server::axum::Json;
use leptoskit::server::axum::extract::Path;
use leptoskit::server::axum::extract::Query;
use leptoskit::server::axum::http::StatusCode;
use serde::Deserialize;

use crate::error::Answer;
use crate::error::Failure;

fn optional(text: Option<String>) -> Option<String> {
	text.map(|text| text.trim().to_owned())
		.filter(|text| !text.is_empty())
}

/// `POST /api/clips`: adds the page as a source of the project, unless it is
/// one already, and the quote as a clip from it.
pub async fn create(
	me: SignedIn,
	Extension(db): Extension<Db>,
	Json(input): Json<NewClipInput>,
) -> Answer<PublicClip> {
	let project_id = input
		.project
		.ok_or_else(|| Failure::invalid("Choose a project for the clip."))?;
	let url = input.page.url.trim().to_owned();
	if !(url.starts_with("https://") || url.starts_with("http://")) {
		return Err(Failure::invalid("A clip needs the address of a web page."));
	}
	if input.quote.exact.trim().is_empty() {
		return Err(Failure::invalid("A clip needs a quote."));
	}
	Project::find(&db, project_id)
		.await?
		.ok_or_else(|| Failure::not_found("project"))?;

	let source = Source::query()
		.filter(|source| source.project_id.eq(project_id) & source.url.eq(url.clone()))
		.first(&db)
		.await?;
	let source = match source {
		Some(source) => source,
		None => {
			let title = input.page.title.trim();
			NewSource {
				owner_id: me.account,
				project_id,
				title: if title.is_empty() {
					url.clone()
				} else {
					title.to_owned()
				},
				url,
				author: optional(input.page.author),
				site: optional(input.page.site),
				published: optional(input.page.published),
			}
			.insert(&db)
			.await?
		}
	};
	let clip = NewClip {
		owner_id: me.account,
		project_id,
		source_id: source.id,
		quote: input.quote.exact.trim().to_owned(),
		prefix: input.quote.prefix.filter(|text| !text.is_empty()),
		suffix: input.quote.suffix.filter(|text| !text.is_empty()),
		note: optional(input.note),
		tags: tags::normalize(&input.tags),
	}
	.insert(&db)
	.await?;
	crate::projects::touch(&db, project_id).await?;
	Ok(Json(clip.into()))
}

/// `PATCH /api/clips/{clip}`.
pub async fn update(
	_me: SignedIn,
	Extension(db): Extension<Db>,
	Path(id): Path<Id<Clip>>,
	Json(changes): Json<ClipChanges>,
) -> Answer<PublicClip> {
	let mut clip = Clip::find(&db, id)
		.await?
		.ok_or_else(|| Failure::not_found("clip"))?;
	if let Some(note) = changes.note {
		clip.note = optional(note);
	}
	if let Some(tags) = changes.tags {
		clip.tags = tags::normalize(&tags);
	}
	clip.save(&db).await?;
	Ok(Json(clip.into()))
}

/// `DELETE /api/clips/{clip}`.
pub async fn remove(
	_me: SignedIn,
	Extension(db): Extension<Db>,
	Path(id): Path<Id<Clip>>,
) -> Result<StatusCode, Failure> {
	let removed = Clip::query()
		.filter(|clip| clip.id.eq(id))
		.destroy(&db)
		.await?;
	if removed == 0 {
		return Err(Failure::not_found("clip"));
	}
	Ok(StatusCode::NO_CONTENT)
}

/// The query of `GET /api/clips`.
#[derive(Deserialize)]
pub struct OnPage {
	pub url: String,
}

/// `GET /api/clips?url=…`: the clips taken from one page, in every project.
pub async fn on_page(
	_me: SignedIn,
	Extension(db): Extension<Db>,
	Query(page): Query<OnPage>,
) -> Answer<Vec<ClipOnPage>> {
	let url = page.url.trim().to_owned();
	let sources = Source::query()
		.filter(|source| source.url.eq(url.clone()))
		.all(&db)
		.await?;
	let mut clips = Vec::new();
	for source in sources {
		let project = Project::find(&db, source.project_id)
			.await?
			.ok_or_else(|| Failure::not_found("project"))?;
		let source_id = source.id;
		let taken = Clip::query()
			.filter(|clip| clip.source_id.eq(source_id))
			.order_by(|clip| clip.created_at.desc())
			.all(&db)
			.await?;
		let source = commonplace_db::PublicSource::from(source);
		let project = commonplace_db::PublicProject::from(project);
		clips.extend(taken.into_iter().map(|clip| {
			ClipOnPage {
				clip: clip.into(),
				source: source.clone(),
				project: project.clone(),
			}
		}));
	}
	Ok(Json(clips))
}
