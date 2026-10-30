//! `/panel`: the side panel. A project's clips beside whatever you're
//! reading; pressing one opens its page.

use commonplace_shared::ProjectDetail;
use leptoskit::prelude::*;

use crate::browser;
use crate::routes::Navigation;
use crate::store::Load;
use crate::store::Store;
use crate::ui;

pub fn page(navigation: &Navigation) -> Element {
	let _ = navigation;
	if !browser::IN_EXTENSION {
		return ui::extension_only();
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
				Some((open, Load::Ready(detail))) if open == id => clips(&detail),
				Some((_, Load::Failed(message))) => ui::problem(&message),
				_ => ui::loading("the project"),
			}
		}
		None => View(),
	};
	View()
		.class("gap-3")
		.child(ui::title("Your research"))
		.child(ui::project_picker(&projects, chosen, |project| {
			Store::current().update(|state| state.extension.project = Some(project));
			crate::tasks::spawn(browser::keep_project(project));
		}))
		.child(clips)
}

fn clips(detail: &ProjectDetail) -> Element {
	if detail.clips.is_empty() {
		return Text("Nothing clipped into this project yet.").class(ui::QUIET);
	}
	View()
		.class("gap-2")
		.children(detail.clips.iter().map(|clip| {
			let source = detail.source_of(clip).cloned();
			let url = source
				.as_ref()
				.map(|source| source.url.clone())
				.unwrap_or_default();
			Pressable(format!("Open the page for: {}", clip.quote))
				.on_press(move || {
					let url = url.clone();
					crate::tasks::spawn(async move { browser::open(&url).await });
				})
				.child(
					ui::card()
						.child(Text(format!("\u{201c}{}\u{201d}", clip.quote)))
						.child(match source {
							Some(source) => Text(source.title).class(ui::QUIET),
							None => View(),
						}),
				)
		}))
}
