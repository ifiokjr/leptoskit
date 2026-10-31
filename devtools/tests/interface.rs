//! The interface's screens on the headless backend: every panel renders with
//! the classes the toolkit supports, and its controls send the commands the
//! hub expects.

use leptoskit_devtools_protocol::CallRecord;
use leptoskit_devtools_protocol::CallStatus;
use leptoskit_devtools_protocol::Capability;
use leptoskit_devtools_protocol::Change;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Commit;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Frame;
use leptoskit_devtools_protocol::Hello;
use leptoskit_devtools_protocol::MatchKey;
use leptoskit_devtools_protocol::NamedValue;
use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::Platform;
use leptoskit_devtools_protocol::RpcAttempt;
use leptoskit_devtools_protocol::SessionId;
use leptoskit_devtools_protocol::SessionInfo;
use leptoskit_devtools_protocol::SessionState;
use leptoskit_devtools_protocol::SessionView;
use leptoskit_devtools_protocol::Snapshot;
use leptoskit_devtools_protocol::SourceLocation;
use leptoskit_devtools_protocol::StyleRule;
use leptoskit_devtools_protocol::Tree;
use leptoskit_devtools_protocol::Value;
use leptoskit_devtools_ui::hub;
use leptoskit_devtools_ui::model::HubInfo;
use leptoskit_devtools_ui::model::Model;
use leptoskit_devtools_ui::model::Panel;
use leptoskit_devtools_ui::store;
use leptoskit_devtools_ui::view;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;

fn node(id: u64, parent: Option<u64>, kind: &str, children: &[u64], text: Option<&str>) -> Node {
	Node {
		id: NodeRef::Native(id),
		key: MatchKey::new(format!("src/screen.rs:{id}:5#0")),
		parent: parent.map(NodeRef::Native),
		kind: kind.into(),
		source: Some(SourceLocation {
			file: "src/screen.rs".into(),
			line: u32::try_from(id).unwrap(),
			column: 5,
		}),
		frame: Some(Frame {
			x: 0.0,
			y: 40.0 * id as f32,
			width: 390.0,
			height: 40.0,
		}),
		classes: vec!["p-4".into(), "dark:bg-black".into()],
		text: text.map(str::to_owned),
		children: children.iter().copied().map(NodeRef::Native).collect(),
		styles: vec![StyleRule {
			class: "p-4".into(),
			declarations: "padding-top:1rem;padding-bottom:1rem;".into(),
			variant: None,
		}],
	}
}

fn tree() -> Tree {
	Tree {
		root: NodeRef::Native(1),
		nodes: vec![
			node(1, None, "View", &[2, 3], None),
			node(2, Some(1), "Text", &[], Some("Library")),
			node(3, Some(1), "Button", &[], Some("New project")),
		],
	}
}

fn value(type_name: &str, json: serde_json::Value) -> Value {
	Value::from_json(type_name, json)
}

fn snapshot() -> Snapshot {
	Snapshot {
		tree: tree(),
		route: Some("/projects".into()),
		queries: vec![NamedValue {
			name: "projects".into(),
			value: value(
				"QueryState<Vec<Project>>",
				serde_json::json!({"status": "success", "data": [{"id": 1, "title": "Field notes"}]}),
			),
		}],
		mutations: vec![NamedValue {
			name: "create_project".into(),
			value: value("MutationState", serde_json::json!({"status": "idle"})),
		}],
		forms: vec![NamedValue {
			name: "new_project".into(),
			value: value("Form", serde_json::json!({"title": "", "touched": false})),
		}],
		state: vec![
			NamedValue {
				name: "cart.count".into(),
				value: value("usize", serde_json::json!(2)),
			},
			NamedValue {
				name: "big".into(),
				value: value("String", serde_json::json!("x".repeat(70 * 1024))),
			},
		],
		calls: vec![CallRecord {
			id: 1,
			plugin: "http".into(),
			method: "fetch".into(),
			started_ms: 120,
			finished_ms: Some(180),
			status: CallStatus::Ok,
		}],
		rpc: vec![RpcAttempt {
			id: 1,
			function: "list_projects".into(),
			attempt: 2,
			started_ms: 100,
			finished_ms: Some(160),
			error: Some("timeout".into()),
		}],
	}
}

fn info(id: &str, platform: Platform, capabilities: &[Capability], at: u64) -> SessionInfo {
	SessionInfo {
		id: SessionId::new(id),
		hello: Hello::new(
			"commonplace",
			platform,
			(platform == Platform::Ios).then(|| "iPhone 17".to_owned()),
			capabilities.iter().copied(),
		),
		connected_at_ms: at,
		last_seen_ms: at,
	}
}

const NATIVE: &[Capability] = &[
	Capability::Tree,
	Capability::Styles,
	Capability::Commits,
	Capability::Queries,
	Capability::Mutations,
	Capability::Forms,
	Capability::State,
	Capability::Calls,
	Capability::Rpc,
	Capability::Profiling,
	Capability::Picking,
	Capability::Navigation,
	Capability::Editing,
];

const WEB: &[Capability] = &[Capability::Tree, Capability::Picking];

fn state(info: SessionInfo) -> SessionState {
	let mut view = SessionView::default();
	view.apply(&Event::Snapshot {
		snapshot: snapshot(),
	});
	view.apply(&Event::Commit {
		commit: Commit {
			seq: 1,
			at_ms: 200,
			mutations: 4,
			layout_us: Some(310),
			apply_us: Some(95),
		},
	});
	SessionState { info, view }
}

/// Loads an iOS session with every capability and a web session with few.
fn load(embed: bool) {
	store::update(|model| {
		*model = Model {
			embed,
			..Model::default()
		};
		model.hub = Some(HubInfo {
			version: "0.1.0".into(),
			root: "/work/commonplace".into(),
		});
		model.load(vec![
			state(info("ios", Platform::Ios, NATIVE, 1)),
			state(info("web", Platform::Web, WEB, 2)),
		]);
	});
	let _ = hub::take_sent();
}

fn tester() -> WidgetTester {
	WidgetTester::new(view::page, 1400.0, 900.0).unwrap()
}

fn ios() -> SessionId {
	SessionId::new("ios")
}

#[test]
fn every_panel_renders_for_every_session_in_both_layouts() {
	for embed in [false, true] {
		load(embed);
		store::update(|model| model.waited = true);

		for session in ["ios", "web"] {
			store::show(&SessionId::new(session));
			store::update(|model| {
				model.select(NodeRef::Native(3), MatchKey::new("src/screen.rs:3:5#0"));
			});

			for panel in Panel::ALL {
				store::panel(panel);
				let tester = tester();
				assert!(
					tester.find(&Finder::text(panel.label())).is_ok(),
					"{session} {panel:?}:\n{}",
					tester.tree()
				);
			}
		}
	}
}

#[test]
fn an_empty_hub_says_how_to_connect_an_app() {
	store::update(|model| *model = Model::default());
	let tester = tester();
	tester
		.find(&Finder::text(
			"No apps are connected. Run your app with leptoskit dev: each platform it runs on \
			 appears here.",
		))
		.unwrap();
}

#[test]
fn the_web_session_is_shown_first_and_sessions_switch() {
	load(false);
	let mut tester = tester();
	assert_eq!(
		store::peek(|model| model.current.clone()),
		Some(SessionId::new("web"))
	);
	tester.tap(&Finder::text("iOS · iPhone 17")).unwrap();
	assert_eq!(store::peek(|model| model.current.clone()), Some(ios()));
}

#[test]
fn selecting_a_row_selects_and_highlights_it_in_the_app() {
	load(false);
	store::show(&ios());
	let mut tester = tester();
	tester
		.tap(&Finder::text("Button .p-4 .dark:bg-black “New project”"))
		.unwrap();
	let key = MatchKey::new("src/screen.rs:3:5#0");
	assert_eq!(
		hub::take_sent(),
		[
			(
				ios(),
				Command::Select {
					node: NodeRef::Native(3),
					key: key.clone(),
				}
			),
			(ios(), Command::Highlight { key }),
		]
	);
	tester.find(&Finder::text("src/screen.rs:3:5")).unwrap();
	// The node's frame, then the same frame beside the web session's.
	assert_eq!(tester.find_all(&Finder::text("0, 120 · 390 × 40")).len(), 3);

	tester.tap(&Finder::text("Open in editor")).unwrap();
	assert_eq!(
		hub::take_opened(),
		["vscode://file/work/commonplace/src/screen.rs:3:5"]
	);
}

#[test]
fn the_tree_collapses_and_searches() {
	load(false);
	store::show(&ios());
	let mut tester = tester();
	tester.tap(&Finder::label("Collapse View")).unwrap();
	assert!(tester.find(&Finder::label("Expand View")).is_ok());
	assert!(
		tester
			.find(&Finder::text("Text .p-4 .dark:bg-black “Library”"))
			.is_err()
	);

	tester
		.enter_text(&Finder::label("Search the tree"), "library")
		.unwrap();
	let rows = store::peek(|model| model.rows().len());
	assert_eq!(rows, 2);
	tester
		.enter_text(&Finder::label("Search the tree"), "nothing like this")
		.unwrap();
	tester
		.find(&Finder::text("Nothing in the tree matches the search."))
		.unwrap();
}

#[test]
fn picking_toggles_in_the_app() {
	load(false);
	store::show(&ios());
	let mut tester = tester();
	tester.tap(&Finder::text("Pick")).unwrap();
	assert_eq!(hub::take_sent(), [(ios(), Command::Pick { on: true })]);
	tester.tap(&Finder::text("Picking…")).unwrap();
	assert_eq!(hub::take_sent(), [(ios(), Command::Pick { on: false })]);
}

#[test]
fn classes_are_edited_and_applied() {
	load(false);
	store::show(&ios());
	store::update(|model| model.select(NodeRef::Native(2), MatchKey::new("src/screen.rs:2:5#0")));
	let mut tester = tester();
	tester
		.enter_text(&Finder::label("Classes"), "p-8 text-lg")
		.unwrap();
	tester.tap(&Finder::text("Apply classes")).unwrap();
	assert_eq!(
		hub::take_sent(),
		[(
			ios(),
			Command::SetClasses {
				node: NodeRef::Native(2),
				classes: vec!["p-8".into(), "text-lg".into()],
			}
		)]
	);
}

#[test]
fn styles_list_declarations_and_inactive_classes() {
	load(false);
	store::show(&ios());
	store::update(|model| model.select(NodeRef::Native(2), MatchKey::new("src/screen.rs:2:5#0")));
	store::panel(Panel::Styles);
	let tester = tester();
	tester.find(&Finder::text("padding-top:1rem;")).unwrap();
	tester.find(&Finder::text("Not active here")).unwrap();
	tester.find(&Finder::text("dark:bg-black")).unwrap();
}

#[test]
fn the_route_panel_navigates() {
	load(false);
	store::show(&ios());
	store::panel(Panel::Route);
	let mut tester = tester();
	tester.find(&Finder::text("/projects")).unwrap();
	tester
		.enter_text(&Finder::label("Route"), "/settings")
		.unwrap();
	tester.tap(&Finder::text("Navigate")).unwrap();
	assert_eq!(
		hub::take_sent(),
		[(
			ios(),
			Command::Navigate {
				href: "/settings".into()
			}
		)]
	);
}

#[test]
fn queries_refetch_invalidate_and_take_new_data() {
	load(false);
	store::show(&ios());
	store::panel(Panel::Queries);
	let mut tester = tester();
	tester.find(&Finder::text("success")).unwrap();
	tester.tap(&Finder::label("Refetch projects")).unwrap();
	tester.tap(&Finder::label("Invalidate projects")).unwrap();
	assert_eq!(
		hub::take_sent(),
		[
			(
				ios(),
				Command::RefetchQuery {
					key: "projects".into()
				}
			),
			(
				ios(),
				Command::InvalidateQuery {
					key: "projects".into()
				}
			),
		]
	);

	tester.tap(&Finder::label("Edit projects")).unwrap();
	tester
		.enter_text(&Finder::label("projects as JSON"), "{\"status\": oops")
		.unwrap();
	tester.tap(&Finder::text("Save")).unwrap();
	assert!(hub::take_sent().is_empty());
	assert!(store::peek(|model| model.notice.clone()).is_some());

	tester
		.enter_text(
			&Finder::label("projects as JSON"),
			"{\"status\":\"success\",\"data\":[]}",
		)
		.unwrap();
	tester.tap(&Finder::text("Save")).unwrap();
	let sent = hub::take_sent();
	assert_eq!(sent.len(), 1);
	let Command::SetQueryData { key, value } = &sent[0].1 else {
		panic!("saving sends the data: {sent:?}");
	};
	assert_eq!(key, "projects");
	assert_eq!(value.type_name, "QueryState<Vec<Project>>");
	assert_eq!(value.json["data"], serde_json::json!([]));
	tester.tap(&Finder::text("Dismiss")).unwrap();
	assert!(store::peek(|model| model.notice.clone()).is_none());
}

#[test]
fn named_state_is_edited_unless_truncated() {
	load(false);
	store::show(&ios());
	store::panel(Panel::State);
	let mut tester = tester();
	assert!(tester.find(&Finder::label("Edit big")).is_err());
	tester
		.find(&Finder::text(
			"Too large to show (71682 bytes), so it can't be edited here.",
		))
		.unwrap();
	tester.tap(&Finder::label("Edit cart.count")).unwrap();
	tester
		.enter_text(&Finder::label("cart.count as JSON"), "5")
		.unwrap();
	tester.tap(&Finder::text("Save")).unwrap();
	let sent = hub::take_sent();
	assert_eq!(
		sent,
		[(
			ios(),
			Command::SetState {
				name: "cart.count".into(),
				value: Value {
					type_name: "usize".into(),
					json: serde_json::json!(5),
					truncated: false,
				},
			}
		)]
	);
}

#[test]
fn a_session_without_data_panels_says_so() {
	load(false);
	store::show(&SessionId::new("web"));

	for (panel, message) in [
		(Panel::Queries, "This session doesn't report queries."),
		(Panel::Forms, "This session doesn't report forms."),
		(Panel::State, "This session doesn't report named state."),
		(
			Panel::Calls,
			"This session doesn't report plugin or server calls.",
		),
		(Panel::Commits, "This session doesn't report commits."),
	] {
		store::panel(panel);
		tester().find(&Finder::text(message)).unwrap();
	}
}

#[test]
fn calls_and_commits_show_guest_reloads() {
	load(false);
	store::show(&ios());
	store::update(|model| {
		model.apply(
			Change::Event {
				session: ios(),
				event: Event::Snapshot {
					snapshot: snapshot(),
				},
			},
			2_501,
		);
	});
	store::panel(Panel::Calls);
	let tester_calls = tester();
	tester_calls
		.find(&Finder::text("Guest reloaded at 2.500 s"))
		.unwrap();
	tester_calls.find(&Finder::text("http.fetch")).unwrap();
	tester_calls
		.find(&Finder::text("list_projects (attempt 2)"))
		.unwrap();
	tester_calls.find(&Finder::text("timeout")).unwrap();

	store::panel(Panel::Commits);
	let mut tester = tester();
	tester
		.find(&Finder::text("Guest reloaded at 2.500 s"))
		.unwrap();
	tester.tap(&Finder::text("Start profiling")).unwrap();
	assert_eq!(hub::take_sent(), [(ios(), Command::StartProfiling)]);
}

#[test]
fn a_node_on_several_platforms_is_compared_and_highlighted_everywhere() {
	load(false);
	store::show(&SessionId::new("web"));
	store::update(|model| model.select(NodeRef::Native(3), MatchKey::new("src/screen.rs:3:5#0")));
	let mut tester = tester();
	tester.find(&Finder::text("web (here)")).unwrap();
	// The sidebar's session and the frame beside the web one.
	assert_eq!(tester.find_all(&Finder::text("iOS · iPhone 17")).len(), 2);
	tester.tap(&Finder::text("Highlight everywhere")).unwrap();
	let key = MatchKey::new("src/screen.rs:3:5#0");
	assert_eq!(
		hub::take_sent(),
		[
			(
				SessionId::new("web"),
				Command::Highlight { key: key.clone() }
			),
			(ios(), Command::Highlight { key }),
		]
	);
}

#[test]
fn the_panel_layout_lists_sessions_as_chips() {
	load(true);
	assert_eq!(store::peek(|model| model.current.clone()), None);
	let mut tester = tester();
	tester
		.find(&Finder::text(
			"Waiting for the DevTools panel to name this tab's session…",
		))
		.unwrap();
	assert!(tester.find(&Finder::text("Sessions")).is_err());
	tester.tap(&Finder::text("commonplace · web")).unwrap();
	assert_eq!(
		store::peek(|model| model.current.clone()),
		Some(SessionId::new("web"))
	);
}
