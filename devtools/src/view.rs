//! The screens, built from leptoskit components.
//!
//! [`screen`] draws the whole interface from a [`Model`]: the sessions, the
//! element tree, and the panel beside it. Controls run the actions in
//! [`crate::store`].

use leptoskit::prelude::*;
use leptoskit_devtools_protocol::CallStatus;
use leptoskit_devtools_protocol::Capability;
use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::Value;

use crate::model::Connection;
use crate::model::Live;
use crate::model::Model;
use crate::model::Moment;
use crate::model::Panel;
use crate::model::Row;
use crate::model::frame_text;
use crate::model::node_ref_text;
use crate::store;

/// The page.
pub(crate) const PAGE: &str =
	"h-screen bg-white text-slate-900 dark:bg-slate-950 dark:text-slate-100";
/// Quieter text: hints, types, and timings.
pub(crate) const QUIET: &str = "text-xs text-slate-500 dark:text-slate-400";
/// A section's heading.
const LABEL: &str = "text-xs font-semibold text-slate-500 dark:text-slate-400";
/// A divider between columns and rows.
pub(crate) const LINE: &str = "border-slate-200 dark:border-slate-800";
/// A small control.
pub(crate) const CONTROL: &str =
	"rounded border border-slate-300 px-2 py-1 text-xs dark:border-slate-700";
/// A small control that is on.
const CONTROL_ON: &str = "rounded border border-sky-600 bg-sky-600 px-2 py-1 text-xs text-white";
/// A text field.
pub(crate) const FIELD: &str = "rounded border border-slate-300 bg-white px-2 py-1 text-xs \
                                dark:border-slate-700 dark:bg-slate-900";
/// A raised box grouping one item's details.
const CARD: &str = "gap-1 rounded border border-slate-200 p-2 dark:border-slate-800";

/// The most lines of JSON a value shows.
const MAX_JSON_LINES: usize = 200;

/// The interface, from the current state.
#[must_use]
pub fn page() -> Element {
	store::read(screen)
}

/// The interface for `model`.
#[must_use]
pub fn screen(model: &Model) -> Element {
	View()
		.class(PAGE)
		.child(header(model))
		.children(model.notice.as_deref().map(notice))
		.child(
			View()
				.class("flex-1 flex-row min-h-0")
				.children((!model.embed).then(|| sidebar(model)))
				.child(tree_column(model))
				.child(panel_column(model)),
		)
}

fn header(model: &Model) -> Element {
	let status = match model.connection {
		Connection::Connecting => "connecting…",
		Connection::Live => "live",
		Connection::Lost => "reconnecting…",
	};
	let version = model
		.hub
		.as_ref()
		.map(|hub| format!("leptoskit {}", hub.version))
		.unwrap_or_default();

	View()
		.class(format!(
			"flex-row flex-wrap items-center gap-3 border-b px-3 py-2 {LINE}"
		))
		.child(Text("leptoskit devtools").class("text-sm font-semibold"))
		.child(Text(version).class(QUIET))
		.child(Status(status).class(QUIET))
		.children(model.embed.then(|| session_chips(model)))
}

/// The sessions as a row of chips, for the panel, which has no sidebar.
fn session_chips(model: &Model) -> Element {
	let shown = model.session().map(|live| live.info.hello.app.clone());
	View()
		.class("flex-1 flex-row flex-wrap justify-end gap-1")
		.children(model.apps().into_iter().flat_map(|(app, sessions)| {
			let named = shown.as_deref() != Some(app);
			sessions.into_iter().map(move |live| {
				let label = if named {
					format!("{app} · {}", live.label())
				} else {
					live.label()
				};
				session_button(model, live, label)
			})
		}))
}

fn session_button(model: &Model, live: &Live, label: String) -> Element {
	let id = live.info.id.clone();
	let shown = model.current.as_ref() == Some(&id);
	Button(label)
		.class(if shown { CONTROL_ON } else { CONTROL })
		.aria_pressed(shown)
		.on_press(move || store::show(&id))
}

fn notice(message: &str) -> Element {
	View()
		.class(
			"flex-row items-center gap-2 bg-amber-100 px-3 py-2 text-amber-950 dark:bg-amber-950 \
			 dark:text-amber-100",
		)
		.child(Alert(message).class("flex-1 text-xs"))
		.child(Button("Dismiss").class(CONTROL).on_press(store::dismiss))
}

fn sidebar(model: &Model) -> Element {
	let apps = model.apps();
	let body = if apps.is_empty() {
		vec![
			Text(
				"No apps are connected. Run your app with leptoskit dev: each platform it runs on \
				 appears here.",
			)
			.class(QUIET),
		]
	} else {
		apps.into_iter()
			.map(|(app, sessions)| {
				View()
					.class("gap-1")
					.child(Heading(2, app).class("text-sm font-semibold"))
					.children(sessions.into_iter().map(|live| {
						let label = if model.superseded(live) {
							format!("{} · earlier", live.label())
						} else {
							live.label()
						};
						session_button(model, live, label).class("text-start")
					}))
			})
			.collect()
	};

	ScrollView()
		.class(format!("w-56 border-r p-3 {LINE}"))
		.child(
			View()
				.class("gap-4")
				.child(Text("Sessions").class(LABEL))
				.children(body),
		)
}

fn tree_column(model: &Model) -> Element {
	let Some(live) = model.session() else {
		return View()
			.class("flex-1 min-w-0 items-center justify-center p-6")
			.child(
				Text(if model.embed && !model.waited {
					"Waiting for the DevTools panel to name this tab's session…"
				} else if model.sessions.is_empty() {
					"Waiting for an app. Run it with leptoskit dev."
				} else {
					"Choose a session."
				})
				.class(QUIET),
			);
	};

	View()
		.class(format!("flex-1 min-w-0 border-r {LINE}"))
		.child(tree_toolbar(model, live))
		.child(
			ScrollView()
				.class("flex-1 p-1")
				.child(tree_rows(model, live)),
		)
}

fn tree_toolbar(model: &Model, live: &Live) -> Element {
	let can_pick = live.has(Capability::Picking);
	View()
		.class(format!(
			"flex-row items-center gap-2 border-b px-2 py-1 {LINE}"
		))
		.child(
			Button(if model.picking { "Picking…" } else { "Pick" })
				.class(if model.picking { CONTROL_ON } else { CONTROL })
				.enabled(can_pick)
				.aria_pressed(model.picking)
				.aria_description(if can_pick {
					"Choose an element in the app to select it here."
				} else {
					"This session can't pick elements."
				})
				.on_press(store::pick),
		)
		.child(
			TextInput()
				.class(FIELD)
				.class("flex-1")
				.accessibility_label("Search the tree")
				.placeholder("Search kind, class, or text")
				.value(model.search.clone())
				.on_change(store::search),
		)
		.child(
			Button("Refresh")
				.class(CONTROL)
				.aria_description("Ask the app for a fresh snapshot.")
				.on_press(store::refresh),
		)
}

fn tree_rows(model: &Model, live: &Live) -> Element {
	if live.view.tree.is_none() {
		return Text(if live.has(Capability::Tree) {
			"No tree yet: the app reports it after its first render."
		} else {
			"This session doesn't report an element tree."
		})
		.class(format!("p-2 {QUIET}"));
	}

	let rows = model.rows();
	if rows.is_empty() {
		return Text("Nothing in the tree matches the search.").class(format!("p-2 {QUIET}"));
	}

	let hidden = model.node_count().saturating_sub(rows.len());
	let selected = model.selected.as_ref().map(|selection| &selection.node);
	View()
		.class("gap-px")
		.children(
			rows.iter()
				.map(|row| tree_row(row, selected == Some(&row.node.id))),
		)
		.children(
			(model.search.trim().is_empty() && rows.len() == crate::model::MAX_ROWS).then(|| {
				Text(format!("{hidden} more nodes: search to narrow the tree."))
					.class(format!("p-2 {QUIET}"))
			}),
		)
}

fn tree_row(row: &Row<'_>, selected: bool) -> Element {
	let node = row.node;
	let indent = format!("pl-[{}px]", row.depth * 14);
	let disclosure = if row.parent {
		let id = node.id.clone();
		Button(if row.collapsed { "▸" } else { "▾" })
			.class("w-5 text-xs")
			.accessibility_label(if row.collapsed {
				format!("Expand {}", node.kind)
			} else {
				format!("Collapse {}", node.kind)
			})
			.aria_expanded(!row.collapsed)
			.on_press(move || store::toggle(&id))
	} else {
		View().class("w-5")
	};
	let id = node.id.clone();
	let key = node.key.clone();
	let tone = if selected {
		"bg-sky-100 dark:bg-sky-900"
	} else if row.matched {
		"bg-amber-50 dark:bg-amber-950"
	} else {
		""
	};

	View()
		.class(format!("flex-row items-center rounded {indent} {tone}"))
		.child(disclosure)
		.child(
			Button(row_label(node))
				.class("flex-1 px-1 text-start text-xs")
				.aria_pressed(selected)
				.on_press(move || store::select(id.clone(), key.clone())),
		)
}

/// A tree row's text: the kind, its first classes, and its text.
fn row_label(node: &Node) -> String {
	let mut label = node.kind.clone();

	for class in node.classes.iter().take(3) {
		label.push_str(" .");
		label.push_str(class);
	}

	if node.classes.len() > 3 {
		label.push_str(" …");
	}

	if let Some(text) = node
		.text
		.as_deref()
		.map(str::trim)
		.filter(|text| !text.is_empty())
	{
		label.push_str(" “");
		label.push_str(&shorten(text, 40));
		label.push('”');
	}

	label
}

fn panel_column(model: &Model) -> Element {
	let width = if model.embed { "w-80" } else { "w-96" };
	View()
		.class(format!("{width} min-w-0"))
		.child(
			View()
				.class(format!("flex-row flex-wrap gap-1 border-b p-2 {LINE}"))
				.children(Panel::ALL.into_iter().map(|panel| {
					let on = model.panel == panel;
					Button(panel.label())
						.class(if on { CONTROL_ON } else { CONTROL })
						.aria_pressed(on)
						.on_press(move || store::panel(panel))
				})),
		)
		.child(
			ScrollView()
				.class("flex-1 p-3")
				.child(View().class("gap-3").children(panel_body(model))),
		)
}

fn panel_body(model: &Model) -> Vec<Element> {
	let Some(live) = model.session() else {
		return vec![Text("Choose a session to inspect it.").class(QUIET)];
	};

	match model.panel {
		Panel::Node => node_panel(model, live),
		Panel::Styles => styles_panel(model, live),
		Panel::Route => route_panel(model, live),
		Panel::Queries => queries_panel(model, live),
		Panel::Mutations => {
			values_panel(
				live,
				Capability::Mutations,
				"mutations",
				&live.view.mutations,
			)
		}
		Panel::Forms => values_panel(live, Capability::Forms, "forms", &live.view.forms),
		Panel::State => state_panel(model, live),
		Panel::Calls => calls_panel(model, live),
		Panel::Commits => commits_panel(model, live),
	}
}

fn unsupported(what: &str) -> Vec<Element> {
	vec![Text(format!("This session doesn't report {what}.")).class(QUIET)]
}

fn node_panel(model: &Model, live: &Live) -> Vec<Element> {
	let Some(node) = model.node() else {
		return vec![Text("Select an element in the tree, or pick one in the app.").class(QUIET)];
	};
	let mut sections = vec![
		Heading(2, node.kind.clone()).class("text-base font-semibold"),
		facts(&[
			("Key", node.key.as_str().to_owned()),
			("Id", node_ref_text(&node.id)),
			(
				"Frame",
				node.frame
					.as_ref()
					.map_or_else(|| "not laid out".to_owned(), frame_text),
			),
		]),
	];

	if let Some(text) = &node.text {
		sections.push(section("Text", vec![Text(text.clone()).class("text-xs")]));
	}

	sections.push(match &node.source {
		Some(source) => {
			let url = model.editor_url(source);
			section(
				"Source",
				vec![
					Text(format!("{}:{}:{}", source.file, source.line, source.column))
						.class("text-xs"),
					Button("Open in editor")
						.class(CONTROL)
						.class("self-start")
						.on_press(move || store::open_in_editor(&url)),
				],
			)
		}
		None => {
			section(
				"Source",
				vec![Text("This session doesn't report where elements were created.").class(QUIET)],
			)
		}
	});
	sections.push(classes_editor(model, live, node));
	sections.push(other_platforms(model));
	sections
}

fn classes_editor(model: &Model, live: &Live, node: &Node) -> Element {
	let current = node.classes.join(" ");
	let draft = model
		.drafts
		.classes
		.clone()
		.unwrap_or_else(|| current.clone());
	let changed = draft.split_whitespace().ne(current.split_whitespace());
	let editable = live.has(Capability::Styles);
	let mut body = vec![
		TextInput()
			.class(FIELD)
			.accessibility_label("Classes")
			.placeholder("No classes")
			.value(draft)
			.enabled(editable)
			.on_change(store::edit_classes),
	];

	if editable {
		body.push(
			Button("Apply classes")
				.class(if changed { CONTROL_ON } else { CONTROL })
				.class("self-start")
				.enabled(changed)
				.on_press(store::apply_classes),
		);
	} else {
		body.push(Text("This session can't change classes.").class(QUIET));
	}

	section("Classes", body)
}

fn other_platforms(model: &Model) -> Element {
	let counterparts = model.counterparts();

	if counterparts.is_empty() {
		return section(
			"Other platforms",
			vec![
				Text(
					"Run the app on another platform under the same leptoskit dev to compare this \
					 element's frame there.",
				)
				.class(QUIET),
			],
		);
	}

	let mut rows: Vec<Element> = model
		.session()
		.zip(model.node())
		.map(|(live, node)| frame_row(live, node, true))
		.into_iter()
		.collect();
	rows.extend(
		counterparts
			.iter()
			.map(|counterpart| frame_row(counterpart.session, counterpart.node, false)),
	);
	rows.push(
		Button("Highlight everywhere")
			.class(CONTROL)
			.class("self-start")
			.on_press(store::highlight_everywhere),
	);
	section("Other platforms", rows)
}

fn frame_row(live: &Live, node: &Node, shown: bool) -> Element {
	let frame = node
		.frame
		.as_ref()
		.map_or_else(|| "not laid out".to_owned(), frame_text);
	View()
		.class("flex-row justify-between gap-2")
		.child(
			Text(if shown {
				format!("{} (here)", live.label())
			} else {
				live.label()
			})
			.class("text-xs font-medium"),
		)
		.child(Text(frame).class("text-xs"))
}

fn styles_panel(model: &Model, live: &Live) -> Vec<Element> {
	let Some(node) = model.node() else {
		return vec![Text("Select an element to see its styles.").class(QUIET)];
	};

	if node.classes.is_empty() {
		return vec![Text(format!("This {} has no classes.", node.kind)).class(QUIET)];
	}

	if node.styles.is_empty() {
		let reason = if live.has(Capability::Styles) {
			"None of its classes apply here."
		} else {
			"This session doesn't explain its styles; these are its classes."
		};
		return vec![
			Text(reason).class(QUIET),
			View()
				.class("flex-row flex-wrap gap-1")
				.children(node.classes.iter().map(|class| chip(class))),
		];
	}

	let inactive: Vec<&String> = node
		.classes
		.iter()
		.filter(|class| !node.styles.iter().any(|rule| &rule.class == *class))
		.collect();
	let mut sections: Vec<Element> = node
		.styles
		.iter()
		.map(|rule| {
			View()
				.class(CARD)
				.child(
					View()
						.class("flex-row items-center justify-between gap-2")
						.child(Text(rule.class.clone()).class("text-xs font-semibold"))
						.children(
							rule.variant
								.clone()
								.map(|variant| Text(variant).class(QUIET)),
						),
				)
				.children(
					rule.declarations
						.split(';')
						.map(str::trim)
						.filter(|declaration| !declaration.is_empty())
						.map(|declaration| Text(format!("{declaration};")).class("text-xs")),
				)
		})
		.collect();

	if !inactive.is_empty() {
		sections.push(section(
			"Not active here",
			vec![
				View()
					.class("flex-row flex-wrap gap-1")
					.children(inactive.into_iter().map(|class| chip(class))),
			],
		));
	}

	sections
}

fn route_panel(model: &Model, live: &Live) -> Vec<Element> {
	let route = live.view.route.clone();
	let mut sections = vec![section(
		"Current route",
		vec![
			Text(
				route
					.clone()
					.unwrap_or_else(|| "This session hasn't reported a route.".to_owned()),
			)
			.class("text-sm"),
		],
	)];

	if live.has(Capability::Navigation) {
		sections.push(section(
			"Go to",
			vec![
				View()
					.class("flex-row gap-2")
					.child(
						TextInput()
							.class(FIELD)
							.class("flex-1")
							.accessibility_label("Route")
							.placeholder("/settings")
							.value(model.drafts.route.clone().or(route).unwrap_or_default())
							.on_change(store::edit_route),
					)
					.child(Button("Navigate").class(CONTROL).on_press(store::navigate)),
			],
		));
	} else {
		sections.push(Text("This session can't be navigated from here.").class(QUIET));
	}

	sections
}

fn queries_panel(model: &Model, live: &Live) -> Vec<Element> {
	if !live.has(Capability::Queries) {
		return unsupported("queries");
	}

	if live.view.queries.is_empty() {
		return vec![Text("No queries yet.").class(QUIET)];
	}

	let editable = live.has(Capability::Editing);
	live.view
		.queries
		.iter()
		.map(|(key, value)| {
			let status = value
				.json
				.get("status")
				.and_then(serde_json::Value::as_str)
				.map(str::to_owned);
			let (refetch, invalidate, edit) = (key.clone(), key.clone(), key.clone());
			let actions = View()
				.class("flex-row flex-wrap gap-1")
				.child(
					Button("Refetch")
						.class(CONTROL)
						.accessibility_label(format!("Refetch {key}"))
						.on_press(move || store::refetch(&refetch)),
				)
				.child(
					Button("Invalidate")
						.class(CONTROL)
						.accessibility_label(format!("Invalidate {key}"))
						.on_press(move || store::invalidate(&invalidate)),
				);
			let draft = model.drafts.queries.get(key);
			// A query whose scope has no codec says so in its value; its data
			// can't be decoded, so it isn't offered for editing.
			let read_only = value.json.get("read_only").is_some();
			let actions = if draft.is_none() && editable && !value.truncated && !read_only {
				let json = value.json.to_string();
				actions.child(
					Button("Edit data")
						.class(CONTROL)
						.accessibility_label(format!("Edit {key}"))
						.on_press(move || store::edit_query(&edit, &json)),
				)
			} else {
				actions
			};

			View()
				.class(CARD)
				.child(
					View()
						.class("flex-row items-center justify-between gap-2")
						.child(Text(key.clone()).class("text-xs font-semibold"))
						.children(status.map(|status| chip(&status))),
				)
				.child(Text(value.type_name.clone()).class(QUIET))
				.child(match draft {
					Some(draft) => {
						json_editor(
							key,
							draft,
							store::edit_query,
							store::save_query,
							store::cancel_query,
						)
					}
					None => json_view(value),
				})
				.child(actions)
		})
		.collect()
}

fn values_panel(
	live: &Live,
	capability: Capability,
	what: &str,
	values: &std::collections::BTreeMap<String, Value>,
) -> Vec<Element> {
	if !live.has(capability) {
		return unsupported(what);
	}

	if values.is_empty() {
		return vec![Text(format!("No {what} yet.")).class(QUIET)];
	}

	values
		.iter()
		.map(|(name, value)| {
			View()
				.class(CARD)
				.child(Text(name.clone()).class("text-xs font-semibold"))
				.child(Text(value.type_name.clone()).class(QUIET))
				.child(json_view(value))
		})
		.collect()
}

fn state_panel(model: &Model, live: &Live) -> Vec<Element> {
	if !live.has(Capability::State) {
		return unsupported("named state");
	}

	if live.view.state.is_empty() {
		return vec![
			Text("No named state yet. Name a signal with devtools::expose to see it here.")
				.class(QUIET),
		];
	}

	let editable = live.has(Capability::Editing);
	live.view
		.state
		.iter()
		.map(|(name, value)| {
			let draft = model.drafts.state.get(name);
			let body = match draft {
				Some(draft) => {
					json_editor(
						name,
						draft,
						store::edit_state,
						store::save_state,
						store::cancel_state,
					)
				}
				None => json_view(value),
			};
			let edit = (draft.is_none() && editable && !value.truncated).then(|| {
				let (name, json) = (name.clone(), value.json.to_string());
				Button("Edit")
					.class(CONTROL)
					.class("self-start")
					.accessibility_label(format!("Edit {name}"))
					.on_press(move || store::edit_state(&name, &json))
			});

			View()
				.class(CARD)
				.child(Text(name.clone()).class("text-xs font-semibold"))
				.child(Text(value.type_name.clone()).class(QUIET))
				.child(body)
				.children(edit)
		})
		.collect()
}

fn calls_panel(model: &Model, live: &Live) -> Vec<Element> {
	if !live.has(Capability::Calls) && !live.has(Capability::Rpc) {
		return unsupported("plugin or server calls");
	}

	let moments = model.calls();
	if moments.is_empty() {
		return vec![Text("No calls yet.").class(QUIET)];
	}

	moments
		.iter()
		.rev()
		.map(|moment| {
			match moment {
				Moment::Call(call) => {
					let status = match call.status {
						CallStatus::Pending => "pending",
						CallStatus::Ok => "ok",
						CallStatus::Error => "error",
						CallStatus::Cancelled => "cancelled",
					};
					let mut detail = duration(call.started_ms, call.finished_ms);
					if let (Some(sent), Some(back)) = (call.request_bytes, call.response_bytes) {
						detail.push_str(&format!(" · {} sent, {} back", bytes(sent), bytes(back)));
					} else if let Some(sent) = call.request_bytes {
						detail.push_str(&format!(" · {} sent", bytes(sent)));
					}
					if let Some(late) = call.completed_late_ms {
						detail.push_str(&format!(
							" · answered {} after it was cancelled",
							seconds(late.saturating_sub(call.finished_ms.unwrap_or(late)))
						));
					}
					timeline_row(
						call.started_ms,
						format!("{}.{}", call.plugin, call.method),
						status,
						detail,
					)
				}
				Moment::Rpc(attempt) => {
					let status = match (&attempt.error, attempt.finished_ms) {
						(Some(error), _) => error.clone(),
						(None, Some(_)) => "ok".to_owned(),
						(None, None) => "pending".to_owned(),
					};
					timeline_row(
						attempt.started_ms,
						format!("{} (attempt {})", attempt.function, attempt.attempt),
						&status,
						duration(attempt.started_ms, attempt.finished_ms),
					)
				}
				Moment::Commit(_) => View(),
				Moment::Reload(at) => reload_row(*at),
			}
		})
		.collect()
}

fn commits_panel(model: &Model, live: &Live) -> Vec<Element> {
	if !live.has(Capability::Commits) {
		return unsupported("commits");
	}

	let mut sections = Vec::new();

	if live.has(Capability::Profiling) {
		let on = live.profiling;
		sections.push(
			View()
				.class("flex-row items-center gap-2")
				.child(
					Button(if on {
						"Stop profiling"
					} else {
						"Start profiling"
					})
					.class(if on { CONTROL_ON } else { CONTROL })
					.aria_pressed(on)
					.on_press(move || store::profile(!on)),
				)
				.child(
					Text(if on {
						"Timing every commit."
					} else {
						"Profile to time layout and apply."
					})
					.class(QUIET),
				),
		);
	}

	let moments = model.commits();
	let commits: Vec<_> = live.view.commits.iter().collect();

	if commits.is_empty() && moments.is_empty() {
		sections.push(Text("No commits yet.").class(QUIET));
		return sections;
	}

	if !commits.is_empty() {
		let mutations: usize = commits.iter().map(|commit| commit.mutations).sum();
		sections.push(
			Text(format!(
				"{} commits, {} mutations on average",
				commits.len(),
				mutations / commits.len()
			))
			.class(QUIET),
		);
	}

	sections.extend(moments.iter().rev().map(|moment| {
		match moment {
			Moment::Commit(commit) => {
				let timing = match (commit.layout_us, commit.apply_us) {
					(Some(layout), Some(apply)) => format!("layout {layout} µs · apply {apply} µs"),
					(Some(layout), None) => format!("layout {layout} µs"),
					(None, Some(apply)) => format!("apply {apply} µs"),
					(None, None) => String::new(),
				};
				timeline_row(
					commit.at_ms,
					format!("Commit {}", commit.seq),
					&format!("{} mutations", commit.mutations),
					timing,
				)
			}
			Moment::Reload(at) => reload_row(*at),
			Moment::Call(_) | Moment::Rpc(_) => View(),
		}
	}));
	sections
}

fn timeline_row(at_ms: u64, title: String, status: &str, detail: String) -> Element {
	View()
		.class(format!("gap-px border-b pb-1 {LINE}"))
		.child(
			View()
				.class("flex-row items-center justify-between gap-2")
				.child(Text(title).class("text-xs font-medium"))
				.child(chip(status)),
		)
		.child(Text(format!("at {} {detail}", seconds(at_ms))).class(QUIET))
}

fn reload_row(at_ms: u64) -> Element {
	View()
		.class("rounded bg-violet-100 px-2 py-1 dark:bg-violet-950")
		.child(Text(format!("Guest reloaded at {}", seconds(at_ms))).class("text-xs font-medium"))
}

fn duration(started: u64, finished: Option<u64>) -> String {
	finished.map_or_else(
		|| "· running".to_owned(),
		|finished| format!("· {} ms", finished.saturating_sub(started)),
	)
}

fn bytes(count: u64) -> String {
	if count < 1024 {
		format!("{count} B")
	} else {
		format!("{:.1} KiB", count as f64 / 1024.0)
	}
}

fn seconds(ms: u64) -> String {
	format!("{}.{:03} s", ms / 1000, ms % 1000)
}

fn json_view(value: &Value) -> Element {
	if value.truncated {
		let bytes = value.json.get("bytes").and_then(serde_json::Value::as_u64);
		return Text(match bytes {
			Some(bytes) => {
				format!("Too large to show ({bytes} bytes), so it can't be edited here.")
			}
			None => "Too large to show, so it can't be edited here.".to_owned(),
		})
		.class(QUIET);
	}

	let pretty = serde_json::to_string_pretty(&value.json).unwrap_or_default();
	let lines: Vec<&str> = pretty.lines().collect();
	let more = lines.len().saturating_sub(MAX_JSON_LINES);
	View()
		.class("rounded bg-slate-50 p-2 dark:bg-slate-900")
		.children(lines.into_iter().take(MAX_JSON_LINES).map(|line| {
			let indent = line.len() - line.trim_start().len();
			Text(line.trim_start().to_owned()).class(format!("pl-[{}px] text-xs", indent * 4))
		}))
		.children((more > 0).then(|| Text(format!("… {more} more lines")).class(QUIET)))
}

fn json_editor(
	name: &str,
	draft: &str,
	edit: fn(&str, &str),
	save: fn(&str),
	cancel: fn(&str),
) -> Element {
	let (on_edit, on_save, on_cancel) = (name.to_owned(), name.to_owned(), name.to_owned());
	View()
		.class("gap-1")
		.child(
			TextInput()
				.class(FIELD)
				.accessibility_label(format!("{name} as JSON"))
				.value(draft.to_owned())
				.on_change(move |text| edit(&on_edit, text)),
		)
		.child(
			View()
				.class("flex-row gap-1")
				.child(
					Button("Save")
						.class(CONTROL_ON)
						.on_press(move || save(&on_save)),
				)
				.child(
					Button("Cancel")
						.class(CONTROL)
						.on_press(move || cancel(&on_cancel)),
				),
		)
}

fn facts(rows: &[(&str, String)]) -> Element {
	View()
		.class("gap-1")
		.children(rows.iter().map(|(name, value)| {
			View()
				.class("flex-row gap-2")
				.child(Text(*name).class(format!("w-14 {LABEL}")))
				.child(Text(value.clone()).class("flex-1 text-xs"))
		}))
}

fn section(title: &str, body: Vec<Element>) -> Element {
	View()
		.class("gap-1")
		.child(Text(title).class(LABEL))
		.children(body)
}

fn chip(text: &str) -> Element {
	Text(text.to_owned()).class(
		"rounded bg-slate-100 px-1 text-xs text-slate-700 dark:bg-slate-800 dark:text-slate-200",
	)
}

fn shorten(text: &str, limit: usize) -> String {
	if text.chars().count() <= limit {
		return text.to_owned();
	}

	let mut short: String = text.chars().take(limit).collect();
	short.push('…');
	short
}
