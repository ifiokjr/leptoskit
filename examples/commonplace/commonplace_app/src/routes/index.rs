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

#[component]
pub fn Page(navigation: &Navigation) -> Element {
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
		Load::Idle | Load::Loading => view! { <ui::Loading what="your projects" /> },
		Load::Failed(message) => view! { <ui::Problem message=&message /> },
		Load::Ready(projects) if projects.is_empty() => {
			view! {
				<Text class=ui::QUIET>
					"No projects yet. Start one below, then clip quotes into it from anything you \
				 read."
				</Text>
			}
		}
		Load::Ready(projects) => {
			view! {
				<View class="gap-3">
					{projects
						.into_iter()
						.map(|summary| {
							let counts =
								format!("{} sources · {} clips", summary.sources, summary.clips);
							navigation
								.link(
									Project {
										project: summary.project.id,
									},
									summary.project.title.clone(),
								)
								.child(
									view! { <ui::PanelCard /> }
										.child(view! {
											<Text class="text-lg font-semibold">
												{summary.project.title}
											</Text>
										})
										.child(match summary.project.question {
											Some(question) => {
												view! { <Text>{question}</Text> }
											}
											None => view! { <View /> },
										})
										.child(view! { <Text class=ui::QUIET>{counts}</Text> }),
								)
						})
						.collect::<Fragment>()}
				</View>
			}
		}
	};

	let navigation = navigation.clone();
	view! {
		<View class="gap-4">
			<ui::Title text="Your projects" />
			{match crate::browser::SAFARI {
				Some(steps) => {
					view! { <ui::PanelCard /> }
						.child(view! { <Text class=ui::QUIET>{steps}</Text> })
				}
				None => view! { <View /> },
			}}
			{projects}
			{view! { <ui::PanelCard /> }
				.child(view! { <ui::Section text="Start a project" /> })
				.child(view! {
					<ui::Field
						label="Title"
						value=&title
						placeholder="Urban heat islands"
						on_change=|value| {
							Store::current()
								.update(|state| state.new_project.title = value.to_owned())
						}
					/>
				})
				.child(view! {
					<ui::Field
						label="Question"
						value=&question
						placeholder="Why are cities hotter than the country around them?"
						on_change=|value| {
							Store::current().update(|state| {
								state.new_project.question = Some(value.to_owned())
							});
						}
					/>
				})
				.child(view! {
					<ui::Primary
						label="Create project"
						enabled=!busy
						on_press=move || {
							let navigation = navigation.clone();
							Store::current().create_project(move |project| {
								navigation.navigate(Project { project })
							});
						}
					/>
				})}
		</View>
	}
}
