//! `/projects/:project/clips/:clip`: one clip in its context, with its note.

use commonplace_shared::ClipId;
use commonplace_shared::ProjectId;
use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::routes::Navigation;
use crate::routes::Project;
use crate::store::Load;
use crate::store::Store;
use crate::ui;

/// A clip within its project.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct ClipPage {
	pub project: ProjectId,
	pub clip: ClipId,
}

pub fn page(route: &ClipPage, navigation: &Navigation) -> Element {
	let store = Store::current();
	let (project, clip_id) = (route.project, route.clip);
	store.open_project(project);
	let loaded = store.with(|state| {
		match &state.project {
			Some((open, load)) if *open == project => load.clone(),
			_ => Load::Loading,
		}
	});
	let detail = match loaded {
		Load::Ready(detail) => detail,
		Load::Failed(message) => return ui::problem(&message),
		Load::Idle | Load::Loading => return ui::loading("the clip"),
	};
	let back = navigation
		.link(Project { project }, "Back to the project")
		.child(Text("Back to the project").class("text-amber-700"));
	let Some(clip) = detail.clips.iter().find(|clip| clip.id == clip_id) else {
		return View()
			.class("gap-3")
			.child(Text("This clip was deleted."))
			.child(back);
	};
	let source = detail.source_of(clip);
	let editing = store.with(|state| state.editing.clone().filter(|(id, _)| *id == clip_id));

	let context = format!(
		"\u{2026}{}{}{}\u{2026}",
		clip.prefix.clone().unwrap_or_default(),
		clip.quote,
		clip.suffix.clone().unwrap_or_default()
	);
	let note = match editing {
		Some((_, text)) => {
			View()
				.class("gap-2")
				.child(ui::field("Note", &text, "What this quote shows", |value| {
					Store::current().update(|state| {
						if let Some((_, text)) = &mut state.editing {
							*text = value.to_owned();
						}
					});
				}))
				.child(ui::primary("Save note", true, move || {
					Store::current().save_clip_note(project)
				}))
		}
		None => {
			let current = clip.note.clone().unwrap_or_default();
			View()
				.class("gap-2")
				.child(Text(if current.is_empty() {
					"No note yet.".to_owned()
				} else {
					current.clone()
				}))
				.child(ui::secondary("Edit note", move || {
					let current = current.clone();
					Store::current().update(|state| state.editing = Some((clip_id, current)));
				}))
		}
	};
	let to_project = navigation.clone();

	View()
		.class("gap-4")
		.child(back)
		.child(ui::card().child(Text(format!("\u{201c}{}\u{201d}", clip.quote)).class("text-xl")))
		.child(match source {
			Some(source) => {
				ui::card()
					.child(Text(source.title.clone()).class("font-semibold"))
					.child(Text(source.url.clone()).class(ui::QUIET))
					.child(Text(context).class("italic"))
			}
			None => View(),
		})
		.child(ui::section("Your note"))
		.child(note)
		.child(ui::secondary("Delete clip", move || {
			let navigation = to_project.clone();
			Store::current().delete_clip(project, clip_id, move || {
				navigation.navigate(Project { project })
			});
		}))
}
