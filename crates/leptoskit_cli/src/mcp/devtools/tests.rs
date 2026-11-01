//! Every devtools tool against a fake hub on an ephemeral port.

use std::sync::Arc;
use std::sync::Mutex;

use axum::Json;
use axum::Router;
use axum::extract::Path as AxumPath;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::routing::get;
use axum::routing::post;
use serde_json::Value;
use serde_json::json;

use super::*;

/// A command the fake hub received, with the headers that matter.
#[derive(Clone, Debug)]
struct Received {
	session: String,
	content_type: Option<String>,
	origin: Option<String>,
	command: Value,
}

#[derive(Clone, Default)]
struct Fake {
	sessions: Arc<Mutex<Vec<Value>>>,
	views: Arc<Mutex<BTreeMap<String, Value>>>,
	received: Arc<Mutex<Vec<Received>>>,
}

/// A running fake hub and the project directory that points at it.
struct Harness {
	fake: Fake,
	project: tempfile::TempDir,
	_runtime: tokio::runtime::Runtime,
}

impl Harness {
	fn new(sessions: Vec<Value>) -> Self {
		let fake = Fake::default();

		for session in &sessions {
			let id = session["id"].as_str().unwrap().to_owned();
			let mut view = session.clone();
			view["tree"] = tree();
			view["queries"] = json!({
				"clips": {"type_name": "Vec<Clip>", "json": [{"id": 1}], "truncated": false},
			});
			view["mutations"] = json!({
				"save": {"type_name": "bool", "json": true, "truncated": false},
			});
			view["state"] = json!({
				"cart.count": {"type_name": "usize", "json": 2, "truncated": false},
			});

			fake.views.lock().unwrap().insert(id, view);
		}
		*fake.sessions.lock().unwrap() = sessions;

		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let listener = runtime
			.block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
			.unwrap();
		let url = format!("http://{}", listener.local_addr().unwrap());
		let router = Router::new()
			.route("/api/sessions", get(list_sessions))
			.route("/api/sessions/{id}", get(one_session))
			.route("/api/sessions/{id}/commands", post(command))
			.with_state(fake.clone());
		runtime.spawn(async move { axum::serve(listener, router).await });

		let project = tempfile::tempdir().unwrap();
		write_config(project.path(), &url, std::process::id());
		Self {
			fake,
			project,
			_runtime: runtime,
		}
	}

	#[expect(
		clippy::needless_pass_by_value,
		reason = "each call site builds one fresh JSON value, as the server's dispatcher receives \
		          it"
	)]
	fn call(&self, name: &str, arguments: Value) -> Value {
		// Through the server's dispatcher, from a directory inside the
		// project, as an agent's client would call it.
		let nested = self.project.path().join("src/screens");
		std::fs::create_dir_all(&nested).unwrap();
		call_at(&nested, name, &arguments)
	}

	fn received(&self) -> Vec<Received> {
		self.fake.received.lock().unwrap().clone()
	}
}

fn write_config(root: &Path, url: &str, pid: u32) {
	std::fs::create_dir_all(root.join(".leptoskit")).unwrap();
	std::fs::write(
		root.join(CONFIG),
		serde_json::to_vec(&json!({"url": url, "pid": pid})).unwrap(),
	)
	.unwrap();
}

async fn list_sessions(State(fake): State<Fake>) -> Json<Value> {
	Json(Value::Array(fake.sessions.lock().unwrap().clone()))
}

async fn one_session(
	State(fake): State<Fake>,
	AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, StatusCode> {
	fake.views
		.lock()
		.unwrap()
		.get(&id)
		.cloned()
		.map(Json)
		.ok_or(StatusCode::NOT_FOUND)
}

async fn command(
	State(fake): State<Fake>,
	AxumPath(id): AxumPath<String>,
	headers: HeaderMap,
	Json(command): Json<Value>,
) -> StatusCode {
	let header = |name: &str| {
		headers
			.get(name)
			.and_then(|value| value.to_str().ok())
			.map(str::to_owned)
	};
	fake.received.lock().unwrap().push(Received {
		session: id,
		content_type: header("content-type"),
		origin: header("origin"),
		command,
	});
	StatusCode::NO_CONTENT
}

fn session(id: &str, app: &str, platform: &str, connected_at_ms: u64) -> Value {
	json!({
		"id": id,
		"hello": {
			"app": app,
			"platform": platform,
			"device": (platform == "ios").then_some("iPhone 16"),
			"protocol": 1,
			"capabilities": ["tree", "styles", "queries", "state", "navigation", "editing"],
		},
		"connected_at_ms": connected_at_ms,
		"last_seen_ms": connected_at_ms,
	})
}

/// A screen, a header, and a list of three rows, each with a label.
fn tree() -> Value {
	let node = |id: u64, parent: Option<u64>, kind: &str, children: Vec<u64>| {
		json!({
			"id": id,
			"key": format!("src/app.rs:{}:5#0", 10 + id),
			"parent": parent,
			"kind": kind,
			"source": {"file": "src/app.rs", "line": 10 + id, "column": 5},
			"frame": {"x": 0.0, "y": id as f32 * 10.0, "width": 390.0, "height": 44.4},
			"classes": if kind == "Text" { json!([]) } else { json!(["flex-row", "gap-2"]) },
			"children": children,
		})
	};
	let mut nodes = vec![
		node(1, None, "View", vec![2, 3]),
		node(2, Some(1), "Text", vec![]),
		node(3, Some(1), "ScrollView", vec![4, 6, 8]),
	];
	nodes[1]["text"] = json!("Clips");

	for row in [4, 6, 8] {
		nodes.push(node(row, Some(3), "View", vec![row + 1]));
		let mut label = node(row + 1, Some(row), "Text", vec![]);
		label["text"] = json!(format!("Clip number {row}"));
		nodes.push(label);
	}

	nodes[3]["declarations"] = json!({"flex-row": "flex-direction: row;", "gap-2": "gap: 8px;"});
	json!({"root": 1, "nodes": nodes})
}

fn text(result: &Value) -> &str {
	result["content"][0]["text"].as_str().unwrap()
}

fn is_error(result: &Value) -> bool {
	result["isError"].as_bool().unwrap_or(false)
}

fn one_app() -> Harness {
	Harness::new(vec![session("aaaaaaaa-1111", "commonplace", "ios", 10)])
}

#[test]
fn sessions_lists_every_app_reporting_to_the_hub() {
	let harness = Harness::new(vec![
		session("aaaaaaaa-1111", "commonplace", "ios", 10),
		session("bbbbbbbb-2222", "commonplace", "web", 20),
	]);
	let result = harness.call("devtools_sessions", json!({}));
	assert!(!is_error(&result));
	let sessions: Value = serde_json::from_str(text(&result)).unwrap();
	assert_eq!(sessions.as_array().unwrap().len(), 2);
	assert_eq!(sessions[0]["platform"], "ios");
	assert_eq!(sessions[0]["device"], "iPhone 16");
	assert_eq!(sessions[1]["id"], "bbbbbbbb-2222");
}

#[test]
fn inspect_tree_prints_one_line_per_node_down_to_the_depth() {
	let harness = one_app();
	let result = harness.call("inspect_tree", json!({"depth": 1}));
	assert!(!is_error(&result), "{result}");
	assert_eq!(
		text(&result),
		"aaaaaaaa-1111 commonplace on ios (iPhone 16) — 9 nodes\n1 View .flex-row .gap-2 [0,10 \
		 390x44] src/app.rs:11\n\x20 2 Text \"Clips\" [0,20 390x44] src/app.rs:12\n\x20 3 \
		 ScrollView .flex-row .gap-2 [0,30 390x44] src/app.rs:13\n\x20   … 6 more below\n"
	);
}

#[test]
fn inspect_tree_starts_at_a_subtree_named_by_id_or_match_key() {
	let harness = one_app();
	let by_id = harness.call("inspect_tree", json!({"root": 4}));
	let by_key = harness.call("inspect_tree", json!({"root": "src/app.rs:14:5#0"}));
	assert_eq!(text(&by_id), text(&by_key));
	let lines: Vec<&str> = text(&by_id).lines().skip(1).collect();
	assert_eq!(
		lines,
		[
			"4 View .flex-row .gap-2 [0,40 390x44] src/app.rs:14",
			"  5 Text \"Clip number 4\" [0,50 390x44] src/app.rs:15",
		]
	);

	let missing = harness.call("inspect_tree", json!({"root": 99}));
	assert!(is_error(&missing));
	assert!(text(&missing).contains("no node 99"));
}

#[test]
fn inspect_node_returns_the_node_and_its_children() {
	let harness = one_app();
	let result = harness.call("inspect_node", json!({"node": "3"}));
	let node: Value = serde_json::from_str(text(&result)).unwrap();
	assert_eq!(node["kind"], "ScrollView");
	assert_eq!(node["parent"], 1);
	assert_eq!(node["source"]["line"], 13);
	assert_eq!(node["frame"]["width"], 390.0);
	assert_eq!(node["classes"], json!(["flex-row", "gap-2"]));

	assert_eq!(
		node["children"],
		json!([
			{"id": 4, "kind": "View"},
			{"id": 6, "kind": "View"},
			{"id": 8, "kind": "View"},
		])
	);

	let missing = harness.call("inspect_node", json!({}));
	assert!(is_error(&missing));
	assert!(text(&missing).contains("needs a node"));
}

#[test]
fn explain_styles_passes_declarations_through_when_the_hub_has_them() {
	let harness = one_app();
	let declared: Value =
		serde_json::from_str(text(&harness.call("explain_styles", json!({"node": 4})))).unwrap();
	assert_eq!(declared["classes"], json!(["flex-row", "gap-2"]));
	assert_eq!(declared["declarations"]["gap-2"], "gap: 8px;");
	assert!(declared.get("note").is_none());

	let classes_only: Value =
		serde_json::from_str(text(&harness.call("explain_styles", json!({"node": 1})))).unwrap();
	assert_eq!(classes_only["classes"], json!(["flex-row", "gap-2"]));
	assert!(
		classes_only["note"]
			.as_str()
			.unwrap()
			.contains("classes only")
	);

	let unstyled: Value =
		serde_json::from_str(text(&harness.call("explain_styles", json!({"node": 2})))).unwrap();
	assert_eq!(unstyled["note"], "this node has no Tailwind classes");
}

#[test]
fn query_cache_returns_every_entry_or_one_key() {
	let harness = one_app();
	let all: Value = serde_json::from_str(text(&harness.call("query_cache", json!({})))).unwrap();
	assert_eq!(all["queries"]["clips"]["json"], json!([{"id": 1}]));
	assert_eq!(all["mutations"]["save"]["type_name"], "bool");

	let one: Value =
		serde_json::from_str(text(&harness.call("query_cache", json!({"key": "save"})))).unwrap();
	assert_eq!(one["kind"], "mutation");
	assert_eq!(one["value"]["json"], true);

	let missing = harness.call("query_cache", json!({"key": "nope"}));
	assert!(is_error(&missing));
	assert!(text(&missing).contains("\"clips\""));
}

#[test]
fn navigate_sends_a_json_command_without_an_origin() {
	let harness = one_app();
	let result = harness.call("navigate", json!({"href": "/settings"}));
	assert!(!is_error(&result), "{result}");
	let received = harness.received();
	assert_eq!(received.len(), 1);
	assert_eq!(received[0].session, "aaaaaaaa-1111");
	assert_eq!(
		received[0].content_type.as_deref(),
		Some("application/json")
	);
	assert_eq!(received[0].origin, None);

	assert_eq!(
		received[0].command,
		json!({"type": "navigate", "href": "/settings"})
	);
}

#[test]
fn highlight_reaches_every_session_of_the_app_by_match_key() {
	let harness = Harness::new(vec![
		session("aaaaaaaa-1111", "commonplace", "ios", 10),
		session("bbbbbbbb-2222", "commonplace", "web", 20),
		session("cccccccc-3333", "another", "web", 30),
	]);
	let result = harness.call(
		"highlight",
		json!({"session": "ios", "node": 4, "all_sessions": true}),
	);
	assert!(!is_error(&result), "{result}");
	let received = harness.received();
	let sessions: Vec<&str> = received
		.iter()
		.map(|received| received.session.as_str())
		.collect();
	assert_eq!(sessions, ["aaaaaaaa-1111", "bbbbbbbb-2222"]);

	for received in &received {
		assert_eq!(
			received.command,
			json!({"type": "highlight", "key": "src/app.rs:14:5#0"})
		);
	}

	let just_one = harness.call("highlight", json!({"session": "aaaa", "node": 4}));
	assert!(!is_error(&just_one));
	assert_eq!(harness.received().len(), 3);
}

#[test]
fn set_state_sends_the_value_with_its_type() {
	let harness = one_app();
	let result = harness.call("set_state", json!({"name": "cart.count", "value": 5}));
	assert!(!is_error(&result), "{result}");
	assert_eq!(
		harness.received()[0].command,
		json!({
			"type": "set_state",
			"name": "cart.count",
			"value": {"type_name": "usize", "json": 5, "truncated": false},
		})
	);

	let unknown = harness.call("set_state", json!({"name": "nope", "value": 1}));
	assert!(is_error(&unknown));
	assert!(text(&unknown).contains("cart.count"));
}

#[test]
fn the_newest_session_of_one_app_is_the_default() {
	let harness = Harness::new(vec![
		session("aaaaaaaa-1111", "commonplace", "web", 10),
		session("bbbbbbbb-2222", "commonplace", "web", 20),
	]);
	harness.call("navigate", json!({"href": "/"}));
	assert_eq!(harness.received()[0].session, "bbbbbbbb-2222");
}

#[test]
fn several_apps_need_a_session_and_the_error_lists_them() {
	let harness = Harness::new(vec![
		session("aaaaaaaa-1111", "commonplace", "ios", 10),
		session("bbbbbbbb-2222", "commonplace", "web", 20),
	]);
	let result = harness.call("inspect_tree", json!({}));
	assert!(is_error(&result));
	let message = text(&result);
	assert!(message.contains("pass session"), "{message}");
	assert!(message.contains("aaaaaaaa-1111 commonplace on ios (iPhone 16)"));
	assert!(message.contains("bbbbbbbb-2222 commonplace on web"));

	let unknown = harness.call("inspect_tree", json!({"session": "android"}));
	assert!(text(&unknown).contains("no session matches \"android\""));
}

#[test]
fn a_hub_with_no_sessions_says_how_to_start_one() {
	let harness = Harness::new(Vec::new());
	assert!(text(&harness.call("devtools_sessions", json!({}))).contains("No app is reporting"));
	let result = harness.call("inspect_tree", json!({}));
	assert!(is_error(&result));
	assert!(text(&result).contains("no app is reporting"));
}

#[test]
fn without_a_hub_every_tool_says_to_start_leptoskit_dev() {
	let empty = tempfile::tempdir().unwrap();

	for name in NAMES {
		let result = call_at(empty.path(), name, &json!({"node": 1, "href": "/"}));
		assert!(is_error(&result), "{name}");
		assert!(
			text(&result).starts_with(NO_HUB),
			"{name}: {}",
			text(&result)
		);
	}
}

#[test]
fn a_hub_whose_process_exited_is_not_running() {
	let project = tempfile::tempdir().unwrap();
	write_config(
		project.path(),
		"http://127.0.0.1:9",
		i32::MAX.cast_unsigned(),
	);
	let result = call_at(project.path(), "devtools_sessions", &json!({}));
	assert!(is_error(&result));
	assert!(text(&result).starts_with(NO_HUB));
	assert!(text(&result).contains("has exited"));
}

#[test]
fn an_unreachable_hub_is_reported_with_its_address() {
	let project = tempfile::tempdir().unwrap();
	// A live process, but nothing listening.
	let unused = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	let url = format!("http://{}", unused.local_addr().unwrap());
	drop(unused);
	write_config(project.path(), &url, std::process::id());
	let result = call_at(project.path(), "devtools_sessions", &json!({}));
	assert!(is_error(&result));
	assert!(text(&result).contains(&url));
}
