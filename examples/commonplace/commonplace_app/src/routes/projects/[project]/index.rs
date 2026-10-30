//! `/projects/:project`: one project's sources and clips, its notes, and a
//! form to clip a quote by hand.

use commonplace_shared::ProjectDetail;
use commonplace_shared::ProjectId;
use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::routes::ClipPage;
use crate::routes::Export;
use crate::routes::Library;
use crate::routes::Navigation;
use crate::store::ClipDraft;
use crate::store::Load;
use crate::store::Store;
use crate::ui;

/// A project, by its ID.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Project {
	pub project: ProjectId,
}

pub fn page(route: &Project, navigation: &Navigation) -> Element {
	let store = Store::current();
	let id = route.project;
	store.open_project(id);
	let loaded = store.with(|state| {
		match &state.project {
			Some((open, load)) if *open == id => load.clone(),
			_ => Load::Loading,
		}
	});
	match loaded {
		Load::Idle | Load::Loading => ui::loading("the project"),
		Load::Failed(message) => ui::problem(&message),
		Load::Ready(detail) => project(&detail, navigation),
	}
}

fn project(detail: &ProjectDetail, navigation: &Navigation) -> Element {
	let id = detail.project.id;
	let to_library = navigation.clone();
	let header = View()
		.class("gap-1")
		.child(ui::title(detail.project.title.clone()))
		.child(match &detail.project.question {
			Some(question) => Text(question.clone()).class("italic"),
			None => View(),
		})
		.child(
			View()
				.class("flex-row gap-2 mt-2")
				.child(
					navigation
						.link(
							Export {
								project: id,
								style: Default::default(),
							},
							"Export bibliography",
						)
						.child(Text("Export bibliography").class("text-amber-700")),
				)
				.child(ui::secondary("Delete project", move || {
					let navigation = to_library.clone();
					Store::current().delete_project(id, move || navigation.navigate(Library));
				})),
		);

	let sources = if detail.sources.is_empty() {
		Text("Nothing clipped yet. Clip a quote below, or with the browser extension.")
			.class(ui::QUIET)
	} else {
		View()
			.class("gap-3")
			.children(detail.sources.iter().map(|source| {
				let clips = detail.clips_from(source).map(|clip| {
					navigation
						.link(
							ClipPage {
								project: id,
								clip: clip.id,
							},
							format!("Clip: {}", clip.quote),
						)
						.child(
							View()
								.class("gap-1 border-l-4 border-amber-600 pl-3")
								.child(Text(format!("\u{201c}{}\u{201d}", clip.quote)))
								.child(match &clip.note {
									Some(note) => Text(note.clone()).class(ui::QUIET),
									None => View(),
								})
								.child(if clip.tags.is_empty() {
									View()
								} else {
									Text(
										commonplace_shared::tags::parse(&clip.tags)
											.iter()
											.map(|tag| format!("#{tag}"))
											.collect::<Vec<_>>()
											.join(" "),
									)
									.class("text-xs text-amber-800 dark:text-amber-300")
								}),
						)
				});
				ui::card()
					.child(Text(source.title.clone()).class("font-semibold"))
					.child(
						Text(
							[
								source.author.clone(),
								source.site.clone(),
								source.published.clone(),
							]
							.into_iter()
							.flatten()
							.collect::<Vec<_>>()
							.join(" · "),
						)
						.class(ui::QUIET),
					)
					.child(View().class("gap-2").children(clips))
			}))
	};

	View()
		.class("gap-4")
		.child(header)
		.child(ui::section("Sources and clips"))
		.child(sources)
		.child(notes(detail))
		.child(clip_form(id))
}

fn notes(detail: &ProjectDetail) -> Element {
	let id = detail.project.id;
	let draft = Store::current().with(|state| state.note.clone());
	ui::card()
		.child(ui::section("Notes"))
		.children(detail.notes.iter().map(|note| {
			let note_id = note.id;
			View()
				.class("flex-row items-start justify-between gap-2")
				.child(Text(note.body.clone()).class("flex-1"))
				.child(ui::secondary("Delete note", move || {
					Store::current().delete_note(id, note_id);
				}))
		}))
		.child(ui::field(
			"New note",
			&draft,
			"What did you notice?",
			|value| {
				Store::current().update(|state| state.note = value.to_owned());
			},
		))
		.child(ui::primary(
			"Add note",
			!draft.trim().is_empty(),
			move || {
				Store::current().add_note(id);
			},
		))
}

fn clip_form(id: ProjectId) -> Element {
	let store = Store::current();
	let (draft, busy) = store.with(|state| (state.clip.clone(), state.busy));
	let ready = !draft.url.trim().is_empty() && !draft.quote.trim().is_empty();
	let edit = |change: fn(&mut ClipDraft, &str)| {
		move |value: &str| Store::current().update(|state| change(&mut state.clip, value))
	};
	ui::card()
		.child(ui::section("Clip a quote by hand"))
		.child(ui::field(
			"Quote",
			&draft.quote,
			"The passage worth keeping",
			edit(|draft, value| draft.quote = value.to_owned()),
		))
		.child(ui::field(
			"Page address",
			&draft.url,
			"https://",
			edit(|draft, value| draft.url = value.to_owned()),
		))
		.child(ui::field(
			"Page title",
			&draft.title,
			"",
			edit(|draft, value| draft.title = value.to_owned()),
		))
		.child(ui::field(
			"Author",
			&draft.author,
			"Optional",
			edit(|draft, value| draft.author = value.to_owned()),
		))
		.child(ui::field(
			"Site",
			&draft.site,
			"Optional",
			edit(|draft, value| draft.site = value.to_owned()),
		))
		.child(ui::field(
			"Published",
			&draft.published,
			"YYYY-MM-DD, optional",
			edit(|draft, value| draft.published = value.to_owned()),
		))
		.child(ui::field(
			"Your note",
			&draft.note,
			"Optional",
			edit(|draft, value| draft.note = value.to_owned()),
		))
		.child(ui::field(
			"Tags",
			&draft.tags,
			"method, cities",
			edit(|draft, value| draft.tags = value.to_owned()),
		))
		.child(ui::primary("Save clip", ready && !busy, move || {
			Store::current().add_clip(id)
		}))
}
