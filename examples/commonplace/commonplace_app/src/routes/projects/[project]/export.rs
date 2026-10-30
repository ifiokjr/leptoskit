//! `/projects/:project/export?style=`: the project's bibliography, ready to
//! copy, in APA, MLA, or BibTeX.

use commonplace_shared::ProjectId;
use commonplace_shared::cite::Style;
use commonplace_shared::cite::bibliography;
use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::routes::Navigation;
use crate::routes::Project;
use crate::store::Load;
use crate::store::Store;
use crate::ui;

/// A project's bibliography in one style.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Export {
	pub project: ProjectId,
	#[route(default)]
	pub style: Style,
}

pub fn page(route: &Export, navigation: &Navigation) -> Element {
	let store = Store::current();
	let id = route.project;
	store.open_project(id);
	let loaded = store.with(|state| {
		match &state.project {
			Some((open, load)) if *open == id => load.clone(),
			_ => Load::Loading,
		}
	});
	let detail = match loaded {
		Load::Ready(detail) => detail,
		Load::Failed(message) => return ui::problem(&message),
		Load::Idle | Load::Loading => return ui::loading("the bibliography"),
	};

	let styles = View()
		.class("flex-row gap-2")
		.children(Style::ALL.into_iter().map(|style| {
			let label = if style == route.style {
				format!("{} (shown)", style.label())
			} else {
				style.label().to_owned()
			};
			navigation
				.link(Export { project: id, style }, label.clone())
				.child(Text(label).class(if style == route.style {
					"rounded-lg bg-amber-700 px-3 py-1 text-white"
				} else {
					"rounded-lg border border-stone-300 px-3 py-1 dark:border-stone-700"
				}))
		}));
	let text = bibliography(&detail.sources, route.style);
	View()
		.class("gap-4")
		.child(ui::title(format!("Bibliography: {}", detail.project.title)))
		.child(
			navigation
				.link(Project { project: id }, "Back to the project")
				.child(Text("Back to the project").class("text-amber-700")),
		)
		.child(styles)
		.child(if text.is_empty() {
			Text("No sources yet.").class(ui::QUIET)
		} else {
			ui::card().child(
				Text(text)
					.class("text-sm")
					.accessibility_label("Bibliography"),
			)
		})
}
