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

#[component]
pub fn Page(route: &Project, navigation: &Navigation) -> Element {
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
		Load::Idle | Load::Loading => view! { <ui::Loading what="the project" /> },
		Load::Failed(message) => view! { <ui::Problem message=&message /> },
		Load::Ready(detail) => view! { <ProjectView detail=&detail navigation=navigation /> },
	}
}

#[component]
fn ProjectView(detail: &ProjectDetail, navigation: &Navigation) -> Element {
	let id = detail.project.id;
	let to_library = navigation.clone();
	let header = view! {
		<View class="gap-1">
			<ui::Title text=detail.project.title.clone() />
			{match &detail.project.question {
				Some(question) => {
					view! { <Text class="italic">{question.clone()}</Text> }
				}
				None => view! { <View /> },
			}}
			<View class="flex-row gap-2 mt-2">
				{navigation
					.link(
						Export {
							project: id,
							style: Default::default(),
						},
						"Export bibliography",
					)
					.child(view! { <Text class="text-amber-700">"Export bibliography"</Text> })}
				<ui::Secondary
					label="Delete project"
					on_press=move || {
						let navigation = to_library.clone();
						Store::current().delete_project(id, move || navigation.navigate(Library));
					}
				/>
			</View>
		</View>
	};

	let sources = if detail.sources.is_empty() {
		view! {
			<Text class=ui::QUIET>
				"Nothing clipped yet. Clip a quote below, or with the browser extension."
			</Text>
		}
	} else {
		view! {
			<View class="gap-3">
				{detail
					.sources
					.iter()
					.map(|source| {
						let clips = detail.clips_from(source).map(|clip| {
							navigation
								.link(
									ClipPage {
										project: id,
										clip: clip.id,
									},
									format!("Clip: {}", clip.quote),
								)
								.child(view! {
									<View class="gap-1 border-l-4 border-amber-600 pl-3">
										<Text>{format!("\u{201c}{}\u{201d}", clip.quote)}</Text>
										{match &clip.note {
											Some(note) => {
												view! {
													<Text class=ui::QUIET>{note.clone()}</Text>
												}
											}
											None => view! { <View /> },
										}}
										{if clip.tags.is_empty() {
											view! { <View /> }
										} else {
											view! {
												<Text class="text-xs text-amber-800 dark:text-amber-300">
													{commonplace_shared::tags::parse(&clip.tags)
														.iter()
														.map(|tag| format!("#{tag}"))
														.collect::<Vec<_>>()
														.join(" ")}
												</Text>
											}
										}}
									</View>
								})
						});
						view! { <ui::PanelCard /> }
							.child(
								view! { <Text class="font-semibold">{source.title.clone()}</Text> },
							)
							.child(view! {
								<Text class=ui::QUIET>
									{[
										source.author.clone(),
										source.site.clone(),
										source.published.clone(),
									]
									.into_iter()
									.flatten()
									.collect::<Vec<_>>()
									.join(" · ")}
								</Text>
							})
							.child(
								view! { <View class="gap-2">{clips.collect::<Fragment>()}</View> },
							)
					})
					.collect::<Fragment>()}
			</View>
		}
	};

	view! {
		<View class="gap-4">
			{header} <ui::Section text="Sources and clips" /> {sources} <Notes detail=detail />
			<ClipForm id=id />
		</View>
	}
}

#[component]
fn Notes(detail: &ProjectDetail) -> Element {
	let id = detail.project.id;
	let draft = Store::current().with(|state| state.note.clone());
	view! { <ui::PanelCard /> }
		.child(view! { <ui::Section text="Notes" /> })
		.children(detail.notes.iter().map(|note| {
			let note_id = note.id;
			view! {
				<View class="flex-row items-start justify-between gap-2">
					<Text class="flex-1">{note.body.clone()}</Text>
					<ui::Secondary
						label="Delete note"
						on_press=move || {
							Store::current().delete_note(id, note_id);
						}
					/>
				</View>
			}
		}))
		.child(view! {
			<ui::Field
				label="New note"
				value=&draft
				placeholder="What did you notice?"
				on_change=|value| {
					Store::current().update(|state| state.note = value.to_owned());
				}
			/>
		})
		.child(view! {
			<ui::Primary
				label="Add note"
				enabled=!draft.trim().is_empty()
				on_press=move || {
					Store::current().add_note(id);
				}
			/>
		})
}

#[component]
fn ClipForm(id: ProjectId) -> Element {
	let store = Store::current();
	let (draft, busy) = store.with(|state| (state.clip.clone(), state.busy));
	let ready = !draft.url.trim().is_empty() && !draft.quote.trim().is_empty();
	let edit = |change: fn(&mut ClipDraft, &str)| {
		move |value: &str| Store::current().update(|state| change(&mut state.clip, value))
	};
	view! { <ui::PanelCard /> }
		.child(view! { <ui::Section text="Clip a quote by hand" /> })
		.child(view! {
			<ui::Field
				label="Quote"
				value=&draft.quote
				placeholder="The passage worth keeping"
				on_change=edit(|draft, value| draft.quote = value.to_owned())
			/>
		})
		.child(view! {
			<ui::Field
				label="Page address"
				value=&draft.url
				placeholder="https://"
				on_change=edit(|draft, value| draft.url = value.to_owned())
			/>
		})
		.child(view! {
			<ui::Field
				label="Page title"
				value=&draft.title
				placeholder=""
				on_change=edit(|draft, value| draft.title = value.to_owned())
			/>
		})
		.child(view! {
			<ui::Field
				label="Author"
				value=&draft.author
				placeholder="Optional"
				on_change=edit(|draft, value| draft.author = value.to_owned())
			/>
		})
		.child(view! {
			<ui::Field
				label="Site"
				value=&draft.site
				placeholder="Optional"
				on_change=edit(|draft, value| draft.site = value.to_owned())
			/>
		})
		.child(view! {
			<ui::Field
				label="Published"
				value=&draft.published
				placeholder="YYYY-MM-DD, optional"
				on_change=edit(|draft, value| draft.published = value.to_owned())
			/>
		})
		.child(view! {
			<ui::Field
				label="Your note"
				value=&draft.note
				placeholder="Optional"
				on_change=edit(|draft, value| draft.note = value.to_owned())
			/>
		})
		.child(view! {
			<ui::Field
				label="Tags"
				value=&draft.tags
				placeholder="method, cities"
				on_change=edit(|draft, value| draft.tags = value.to_owned())
			/>
		})
		.child(view! {
			<ui::Primary
				label="Save clip"
				enabled=ready && !busy
				on_press=move || Store::current().add_clip(id)
			/>
		})
}
