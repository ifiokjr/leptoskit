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

#[component]
pub fn Page(route: &Export, navigation: &Navigation) -> Element {
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
		Load::Failed(message) => return view! { <ui::Problem message=&message /> },
		Load::Idle | Load::Loading => return view! { <ui::Loading what="the bibliography" /> },
	};

	let styles = view! {
		<View class="flex-row gap-2">
			{Style::ALL
				.into_iter()
				.map(|style| {
					let label = if style == route.style {
						format!("{} (shown)", style.label())
					} else {
						style.label().to_owned()
					};
					navigation
						.link(Export { project: id, style }, label.clone())
						.child(view! {
							<Text class=if style == route.style {
								"rounded-lg bg-amber-700 px-3 py-1 text-white"
							} else {
								"rounded-lg border border-stone-300 px-3 py-1 dark:border-stone-700"
							}>
								{label}
							</Text>
						})
				})
				.collect::<Fragment>()}
		</View>
	};
	let text = bibliography(&detail.sources, route.style);
	view! {
		<View class="gap-4">
			<ui::Title text=format!("Bibliography: {}", detail.project.title) />
			{navigation
				.link(Project { project: id }, "Back to the project")
				.child(view! { <Text class="text-amber-700">"Back to the project"</Text> })}
			{styles}
			{if text.is_empty() {
				view! { <Text class=ui::QUIET>"No sources yet."</Text> }
			} else {
				view! { <ui::PanelCard /> }.child(view! {
					<Text class="text-sm" accessibility_label="Bibliography">{text}</Text>
				})
			}}
		</View>
	}
}
