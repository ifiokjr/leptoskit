//! `/`: the library. Every project, and a form to start a new one.

use leptoskit::prelude::*;
use leptoskit_router::Route;

use crate::routes::Navigation;
use crate::routes::Project;
use crate::store::Load;
use crate::store::Store;
use crate::ui;

/// Names the route `Library` instead of `Index`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Library;

pub fn page(navigation: &Navigation) -> Element {
	let store = Store::current();
	let library = store.with(|state| state.library.clone());
	let (title, question, busy) = store.with(|state| {
		(
			state.new_project.title.clone(),
			state.new_project.question.clone().unwrap_or_default(),
			state.busy,
		)
	});

	let projects = match library {
		Load::Idle | Load::Loading => ui::loading("your projects"),
		Load::Failed(message) => ui::problem(&message),
		Load::Ready(projects) if projects.is_empty() => {
			Text(
				"No projects yet. Start one below, then clip quotes into it from anything you \
				 read.",
			)
			.class(ui::QUIET)
		}
		Load::Ready(projects) => {
			View()
				.class("gap-3")
				.children(projects.into_iter().map(|summary| {
					let counts = format!("{} sources · {} clips", summary.sources, summary.clips);
					navigation
						.link(
							Project {
								project: summary.project.id,
							},
							summary.project.title.clone(),
						)
						.child(
							ui::card()
								.child(Text(summary.project.title).class("text-lg font-semibold"))
								.child(match summary.project.question {
									Some(question) => Text(question),
									None => View(),
								})
								.child(Text(counts).class(ui::QUIET)),
						)
				}))
		}
	};

	let navigation = navigation.clone();
	View()
		.class("gap-4")
		.child(ui::title("Your projects"))
		.child(match crate::browser::SAFARI {
			Some(steps) => ui::card().child(Text(steps).class(ui::QUIET)),
			None => View(),
		})
		.child(projects)
		.child(
			ui::card()
				.child(ui::section("Start a project"))
				.child(ui::field("Title", &title, "Urban heat islands", |value| {
					Store::current().update(|state| state.new_project.title = value.to_owned())
				}))
				.child(ui::field(
					"Question",
					&question,
					"Why are cities hotter than the country around them?",
					|value| {
						Store::current()
							.update(|state| state.new_project.question = Some(value.to_owned()));
					},
				))
				.child(ui::primary("Create project", !busy, move || {
					let navigation = navigation.clone();
					Store::current()
						.create_project(move |project| navigation.navigate(Project { project }));
				})),
		)
}
