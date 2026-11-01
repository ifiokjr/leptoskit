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

#[component]
pub fn Page(route: &ClipPage, navigation: &Navigation) -> Element {
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
		Load::Failed(message) => return view! { <ui::Problem message=&message /> },
		Load::Idle | Load::Loading => return view! { <ui::Loading what="the clip" /> },
	};
	let back = navigation
		.link(Project { project }, "Back to the project")
		.child(view! { <Text class="text-amber-700">"Back to the project"</Text> });
	let Some(clip) = detail.clips.iter().find(|clip| clip.id == clip_id) else {
		return view! {
			<View class="gap-3">
				<Text>"This clip was deleted."</Text>
				{back}
			</View>
		};
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
			view! {
				<View class="gap-2">
					{view! {
						<ui::Field
							label="Note"
							value=&text
							placeholder="What this quote shows"
							on_change=|value| {
								Store::current().update(|state| {
									if let Some((_, text)) = &mut state.editing {
										*text = value.to_owned();
									}
								});
							}
						/>
					}}
					{view! {
						<ui::Primary
							label="Save note"
							enabled=true
							on_press=move || Store::current().save_clip_note(project)
						/>
					}}
				</View>
			}
		}
		None => {
			let current = clip.note.clone().unwrap_or_default();
			view! {
				<View class="gap-2">
					<Text>
						{if current.is_empty() {
							"No note yet.".to_owned()
						} else {
							current.clone()
						}}
					</Text>
					<ui::Secondary
						label="Edit note"
						on_press=move || {
							let current = current.clone();
							Store::current()
								.update(|state| state.editing = Some((clip_id, current)));
						}
					/>
				</View>
			}
		}
	};
	let to_project = navigation.clone();

	view! {
		<View class="gap-4">
			{back}
			{view! { <ui::PanelCard /> }.child(view! {
				<Text class="text-xl">{format!("\u{201c}{}\u{201d}", clip.quote)}</Text>
			})}
			{match source {
				Some(source) => {
					view! { <ui::PanelCard /> }
						.child(view! { <Text class="font-semibold">{source.title.clone()}</Text> })
						.child(view! { <Text class=ui::QUIET>{source.url.clone()}</Text> })
						.child(view! { <Text class="italic">{context}</Text> })
				}
				None => view! { <View /> },
			}}
			<ui::Section text="Your note" /> {note}
			<ui::Secondary
				label="Delete clip"
				on_press=move || {
					let navigation = to_project.clone();
					Store::current().delete_clip(project, clip_id, move || {
						navigation.navigate(Project { project })
					});
				}
			/>
		</View>
	}
}
