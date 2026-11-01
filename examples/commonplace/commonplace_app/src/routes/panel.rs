//! `/panel`: the side panel. A project's clips beside whatever you're
//! reading; pressing one opens its page.

use commonplace_shared::ProjectDetail;
use leptoskit::prelude::*;

use crate::browser;
use crate::routes::Navigation;
use crate::store::Load;
use crate::store::Store;
use crate::ui;

#[component]
pub fn Page(navigation: &Navigation) -> Element {
	let _ = navigation;
	if !browser::IN_EXTENSION {
		return view! { <ui::ExtensionOnly /> };
	}
	let store = Store::current();
	let (projects, chosen) = store.with(|state| {
		(
			state.library.ready().cloned().unwrap_or_default(),
			state.extension.project,
		)
	});
	let chosen = chosen.or_else(|| projects.first().map(|summary| summary.project.id));
	let clips = match chosen {
		Some(id) => {
			store.open_project(id);
			match store.with(|state| state.project.clone()) {
				Some((open, Load::Ready(detail))) if open == id => {
					view! { <Clips detail=&detail /> }
				}
				Some((_, Load::Failed(message))) => view! { <ui::Problem message=&message /> },
				_ => view! { <ui::Loading what="the project" /> },
			}
		}
		None => view! { <View /> },
	};
	view! {
		<View class="gap-3">
			<ui::Title text="Your research" />
			<ui::ProjectPicker
				projects=&projects
				chosen=chosen
				on_pick=|project| {
					Store::current().update(|state| state.extension.project = Some(project));
					crate::tasks::spawn(browser::keep_project(project));
				}
			/>
			{clips}
		</View>
	}
}

#[component]
fn Clips(detail: &ProjectDetail) -> Element {
	if detail.clips.is_empty() {
		return view! { <Text class=ui::QUIET>"Nothing clipped into this project yet."</Text> };
	}
	view! {
		<View class="gap-2">
			{detail
				.clips
				.iter()
				.map(|clip| {
					let source = detail.source_of(clip).cloned();
					let url = source
						.as_ref()
						.map(|source| source.url.clone())
						.unwrap_or_default();
					view! {
						<Pressable
							accessibility_label=format!("Open the page for: {}", clip.quote)
							on:press=move || {
								let url = url.clone();
								crate::tasks::spawn(async move { browser::open(&url).await });
							}
						>
							{view! { <ui::PanelCard /> }
								.child(view! {
									<Text>{format!("\u{201c}{}\u{201d}", clip.quote)}</Text>
								})
								.child(match source {
									Some(source) => {
										view! { <Text class=ui::QUIET>{source.title}</Text> }
									}
									None => view! { <View /> },
								})}
						</Pressable>
					}
				})
				.collect::<Fragment>()}
		</View>
	}
}
