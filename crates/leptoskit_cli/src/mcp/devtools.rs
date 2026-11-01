//! The devtools tools: a running app's tree, styles, queries, and state, read
//! from the hub `leptoskit dev` starts, and commands sent back through it.
//!
//! The hub writes `.leptoskit/devtools.json` (`{url, pid}`) in the project
//! root. Each tool finds that file from the working directory up, checks the
//! process that wrote it is still alive, and calls the hub's front-end API.
//! The requests carry no `Origin`, like every non-browser client.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::MatchKey;
use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::SessionInfo;
use leptoskit_devtools_protocol::Tree;
use leptoskit_devtools_protocol::Value as DevtoolsValue;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

/// The file the hub writes, relative to the project root.
const CONFIG: &str = ".leptoskit/devtools.json";

/// What every tool says when there is no hub to talk to.
const NO_HUB: &str = "no hub is running — start `leptoskit dev`";

/// How deep `inspect_tree` goes when the caller does not say.
const DEFAULT_DEPTH: usize = 4;

/// The most nodes `inspect_tree` prints before it stops.
const MAX_NODES: usize = 400;

/// The longest text a tree line quotes.
const MAX_TEXT: usize = 60;

/// The tools' names, so the server routes them here.
pub(super) const NAMES: &[&str] = &[
	"devtools_sessions",
	"inspect_tree",
	"inspect_node",
	"explain_styles",
	"query_cache",
	"navigate",
	"highlight",
	"set_state",
];

/// The `session` argument every tool but `devtools_sessions` takes.
fn session_property() -> Value {
	json!({
		"type": "string",
		"description": "The session: its id or an id prefix, a platform (web, ios, android, \
			macos, …), or the app's name. Defaults to the only session, or the newest when \
			every session is the same app on the same platform.",
	})
}

/// The `node` argument: a node id as `inspect_tree` prints it, or a match key.
fn node_property(description: &str) -> Value {
	json!({
		"type": ["string", "integer"],
		"description": format!(
			"{description} A node id as inspect_tree prints it (a number on native, a path \
			on the web), or a match key."
		),
	})
}

/// The tool definitions.
pub(super) fn tools() -> Vec<Value> {
	vec![
		json!({
			"name": "devtools_sessions",
			"description": "Lists the apps reporting to the devtools hub that `leptoskit dev` \
				runs: each session's id, app, platform, device, what it can show, and when it \
				was last heard from. Every other devtools tool takes one of these sessions.",
			"inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
		}),
		json!({
			"name": "inspect_tree",
			"description": "Prints a running app's element tree, one node per line: id, kind, \
				text, classes, frame (x,y width x height), and the source line that built it. \
				Deeper nodes are counted, not printed; pass root to look inside one.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"session": session_property(),
					"root": node_property("The subtree to print; the whole tree by default."),
					"depth": {
						"type": "integer",
						"minimum": 0,
						"description": format!("How many levels below the root to print; {DEFAULT_DEPTH} by default."),
					},
				},
				"additionalProperties": false,
			},
		}),
		json!({
			"name": "inspect_node",
			"description": "Returns one node of a running app as JSON: kind, match key, \
				parent, source location, frame, classes, text, and its children's ids and \
				kinds.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"session": session_property(),
					"node": node_property("The node."),
				},
				"required": ["node"],
				"additionalProperties": false,
			},
		}),
		json!({
			"name": "explain_styles",
			"description": "Explains a node's styling: the Tailwind classes active on it, the \
				declarations they resolved to when the app sends them, and the frame layout \
				gave it.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"session": session_property(),
					"node": node_property("The node."),
				},
				"required": ["node"],
				"additionalProperties": false,
			},
		}),
		json!({
			"name": "query_cache",
			"description": "Returns a running app's query cache and mutations as JSON: each \
				key with its value's type and data. Pass key for one entry.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"session": session_property(),
					"key": {"type": "string", "description": "One query or mutation key."},
				},
				"additionalProperties": false,
			},
		}),
		json!({
			"name": "navigate",
			"description": "Navigates a running app to a route, such as /settings.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"session": session_property(),
					"href": {"type": "string", "description": "The route to open."},
				},
				"required": ["href"],
				"additionalProperties": false,
			},
		}),
		json!({
			"name": "highlight",
			"description": "Highlights a node in a running app. With all_sessions, highlights \
				the same node, matched by its match key, in every session of the same app: \
				the web page, the simulator, and the desktop app at once.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"session": session_property(),
					"node": node_property("The node to highlight."),
					"all_sessions": {
						"type": "boolean",
						"description": "Highlight it in every session of the same app.",
					},
				},
				"required": ["node"],
				"additionalProperties": false,
			},
		}),
		json!({
			"name": "set_state",
			"description": "Sets a piece of named state the app exposes to the devtools, \
				such as cart.count. The value is JSON in the state's own encoding. On native, a hot \
				reload resets it.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"session": session_property(),
					"name": {"type": "string", "description": "The state's name."},
					"value": {"description": "The new value, as JSON."},
				},
				"required": ["name", "value"],
				"additionalProperties": false,
			},
		}),
	]
}

/// Runs one devtools tool, finding the hub from the working directory.
pub(super) fn call(name: &str, arguments: &Value) -> Value {
	let root = std::env::current_dir().unwrap_or_default();
	call_at(&root, name, arguments)
}

/// Runs one devtools tool with the hub `.leptoskit/devtools.json` names in
/// `start` or its nearest ancestor that has one.
pub(super) fn call_at(start: &Path, name: &str, arguments: &Value) -> Value {
	let result = Hub::find(start).and_then(|hub| run(&hub, name, arguments));

	match result {
		Ok(text) => json!({"content": [{"type": "text", "text": text}]}),
		Err(error) => {
			json!({
				"content": [{"type": "text", "text": error}],
				"isError": true,
			})
		}
	}
}

fn run(hub: &Hub, name: &str, arguments: &Value) -> Result<String, String> {
	if name == "devtools_sessions" {
		return sessions(hub);
	}

	let session = hub.session(string(arguments, "session"))?;

	match name {
		"inspect_tree" => inspect_tree(hub, &session, arguments),
		"inspect_node" => inspect_node(hub, &session, arguments),
		"explain_styles" => explain_styles(hub, &session, arguments),
		"query_cache" => query_cache(hub, &session, arguments),
		"navigate" => navigate(hub, &session, arguments),
		"highlight" => highlight(hub, &session, arguments),
		"set_state" => set_state(hub, &session, arguments),
		other => Err(format!("unknown tool {other:?}")),
	}
}

/// The `.leptoskit/devtools.json` the hub writes.
#[derive(Deserialize)]
struct HubConfig {
	url: String,
	pid: u32,
}

/// A hub that is running.
struct Hub {
	url: String,
	agent: ureq::Agent,
}

impl Hub {
	fn find(start: &Path) -> Result<Self, String> {
		let file = config_file(start).ok_or(NO_HUB)?;
		let text = std::fs::read_to_string(&file).map_err(|_| NO_HUB.to_owned())?;
		let config: HubConfig = serde_json::from_str(&text)
			.map_err(|error| format!("{} is not a hub address: {error}", file.display()))?;

		if !process_alive(config.pid) {
			return Err(format!(
				"{NO_HUB} (the hub in {} was process {}, which has exited)",
				file.display(),
				config.pid
			));
		}

		let agent = ureq::Agent::config_builder()
			.http_status_as_error(false)
			.timeout_global(Some(Duration::from_secs(5)))
			.build()
			.into();
		Ok(Self {
			url: config.url.trim_end_matches('/').to_owned(),
			agent,
		})
	}

	fn get(&self, path: &str) -> Result<Value, String> {
		let url = format!("{}{path}", self.url);
		let mut response = self
			.agent
			.get(&url)
			.call()
			.map_err(|error| self.unreachable(&error))?;
		let status = response.status();
		let body = response
			.body_mut()
			.read_to_string()
			.map_err(|error| error.to_string())?;

		if !status.is_success() {
			return Err(format!("the hub answered {status} for {path}: {body}"));
		}

		serde_json::from_str(&body).map_err(|error| format!("the hub sent bad JSON: {error}"))
	}

	fn command(&self, session: &SessionInfo, command: &Command) -> Result<(), String> {
		let path = format!("/api/sessions/{}/commands", session.id.as_str());
		let mut response = self
			.agent
			.post(&format!("{}{path}", self.url))
			.header("content-type", "application/json")
			.send(serde_json::to_vec(command).map_err(|error| error.to_string())?)
			.map_err(|error| self.unreachable(&error))?;
		let status = response.status();

		if !status.is_success() {
			let body = response.body_mut().read_to_string().unwrap_or_default();

			return Err(format!("the hub answered {status} for {path}: {body}"));
		}

		Ok(())
	}

	fn unreachable(&self, error: &ureq::Error) -> String {
		format!("{NO_HUB} (could not reach {}: {error})", self.url)
	}

	fn sessions(&self) -> Result<Vec<SessionInfo>, String> {
		serde_json::from_value(self.get("/api/sessions")?)
			.map_err(|error| format!("the hub sent sessions this CLI cannot read: {error}"))
	}

	/// The session `selector` names, or the default one.
	fn session(&self, selector: Option<&str>) -> Result<SessionInfo, String> {
		let sessions = self.sessions()?;

		if sessions.is_empty() {
			return Err(
				"the hub is running but no app is reporting to it: run the app under `leptoskit \
				 dev` with its `devtools` feature"
					.to_owned(),
			);
		}

		let matches: Vec<&SessionInfo> = match selector {
			None => sessions.iter().collect(),
			Some(selector) => {
				let exact: Vec<_> = sessions
					.iter()
					.filter(|session| session.id.as_str() == selector)
					.collect();
				if exact.is_empty() {
					sessions
						.iter()
						.filter(|session| selects(session, selector))
						.collect()
				} else {
					exact
				}
			}
		};

		let Some(newest) = matches.iter().max_by_key(|session| session.connected_at_ms) else {
			return Err(format!(
				"no session matches {:?}; the sessions are:\n{}",
				selector.unwrap_or_default(),
				list(&sessions)
			));
		};
		// Reloads leave a page's or a guest's older sessions behind until they
		// expire, so one app on one platform means its newest session.
		let one_app = matches.iter().all(|session| {
			session.hello.app == newest.hello.app && session.hello.platform == newest.hello.platform
		});

		if !one_app {
			return Err(format!(
				"more than one app is reporting; pass session as one of:\n{}",
				list(&matches.into_iter().cloned().collect::<Vec<_>>())
			));
		}

		Ok((*newest).clone())
	}

	fn view(&self, session: &SessionInfo) -> Result<Value, String> {
		self.get(&format!("/api/sessions/{}", session.id.as_str()))
	}

	fn tree(&self, session: &SessionInfo) -> Result<(Value, Tree), String> {
		let view = self.view(session)?;
		let tree = view
			.get("tree")
			.filter(|tree| !tree.is_null())
			.ok_or_else(|| {
				format!(
					"session {} has not sent its tree yet",
					short(session.id.as_str())
				)
			})?;
		let tree: Tree = serde_json::from_value(tree.clone())
			.map_err(|error| format!("the hub sent a tree this CLI cannot read: {error}"))?;
		Ok((view, tree))
	}
}

/// The nearest `.leptoskit/devtools.json` at or above `start`.
fn config_file(start: &Path) -> Option<PathBuf> {
	start
		.ancestors()
		.map(|directory| directory.join(CONFIG))
		.find(|file| file.is_file())
}

/// Whether a process is still running.
fn process_alive(pid: u32) -> bool {
	#[cfg(unix)]
	{
		let Some(pid) = i32::try_from(pid)
			.ok()
			.and_then(rustix::process::Pid::from_raw)
		else {
			return false;
		};
		// Permission denied still means the process exists.
		!matches!(
			rustix::process::test_kill_process(pid),
			Err(rustix::io::Errno::SRCH)
		)
	}
	#[cfg(windows)]
	{
		std::process::Command::new("tasklist")
			.args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
			.output()
			.map(|output| String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\"")))
			.unwrap_or(true)
	}
	#[cfg(not(any(unix, windows)))]
	{
		let _ = pid;
		true
	}
}

/// Whether `selector` names this session by id prefix, platform, or app.
fn selects(session: &SessionInfo, selector: &str) -> bool {
	session.id.as_str().starts_with(selector)
		|| platform(session) == selector
		|| session.hello.app == selector
}

fn platform(session: &SessionInfo) -> String {
	serde_json::to_value(session.hello.platform)
		.ok()
		.and_then(|value| value.as_str().map(str::to_owned))
		.unwrap_or_default()
}

/// The first eight characters of an id, enough to tell sessions apart.
fn short(id: &str) -> &str {
	id.get(..8).unwrap_or(id)
}

fn list(sessions: &[SessionInfo]) -> String {
	sessions
		.iter()
		.map(|session| format!("- {}", describe(session)))
		.collect::<Vec<_>>()
		.join("\n")
}

fn describe(session: &SessionInfo) -> String {
	let mut line = format!(
		"{} {} on {}",
		session.id.as_str(),
		session.hello.app,
		platform(session)
	);

	if let Some(device) = &session.hello.device {
		let _ = write!(line, " ({device})");
	}

	line
}

fn now_ms() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |elapsed| {
			u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
		})
}

fn sessions(hub: &Hub) -> Result<String, String> {
	let sessions = hub.sessions()?;

	if sessions.is_empty() {
		return Ok(
			"No app is reporting to the hub. Run the app under `leptoskit dev` with its \
			 `devtools` feature."
				.to_owned(),
		);
	}

	let now = now_ms();
	let sessions: Vec<Value> = sessions
		.iter()
		.map(|session| {
			json!({
				"id": session.id,
				"app": session.hello.app,
				"platform": session.hello.platform,
				"device": session.hello.device,
				"capabilities": session.hello.capabilities,
				"connected_s_ago": now.saturating_sub(session.connected_at_ms) / 1000,
				"last_seen_s_ago": now.saturating_sub(session.last_seen_ms) / 1000,
			})
		})
		.collect();
	Ok(pretty(&sessions))
}

fn string<'a>(arguments: &'a Value, name: &str) -> Option<&'a str> {
	arguments.get(name).and_then(Value::as_str)
}

fn pretty(value: &impl serde::Serialize) -> String {
	serde_json::to_string_pretty(value).unwrap_or_default()
}

/// A node's id as the tools print it.
fn node_id(id: &NodeRef) -> String {
	match id {
		NodeRef::Native(id) => id.to_string(),
		NodeRef::Web(path) => path.clone(),
	}
}

/// The nodes of a tree by id, and each node's children in order.
struct Index<'a> {
	nodes: BTreeMap<&'a NodeRef, &'a Node>,
	children: BTreeMap<&'a NodeRef, Vec<&'a NodeRef>>,
}

impl<'a> Index<'a> {
	fn new(tree: &'a Tree) -> Self {
		let nodes: BTreeMap<_, _> = tree.nodes.iter().map(|node| (&node.id, node)).collect();
		let mut children: BTreeMap<_, Vec<_>> = BTreeMap::new();

		for node in &tree.nodes {
			if node.children.is_empty() {
				// A tree that only links children to parents still prints.
				if let Some(parent) = &node.parent
					&& nodes
						.get(parent)
						.is_some_and(|parent| parent.children.is_empty())
				{
					children.entry(parent).or_default().push(&node.id);
				}
			} else {
				children.insert(
					&node.id,
					node.children
						.iter()
						.filter(|child| nodes.contains_key(child))
						.collect(),
				);
			}
		}

		Self { nodes, children }
	}

	fn children(&self, id: &NodeRef) -> &[&'a NodeRef] {
		self.children.get(id).map_or(&[], Vec::as_slice)
	}

	fn descendants(&self, id: &NodeRef) -> usize {
		self.children(id)
			.iter()
			.map(|child| 1 + self.descendants(child))
			.sum()
	}

	/// The node an argument names: an id as the tools print it, or a match
	/// key.
	fn find(&self, argument: &Value) -> Option<&'a Node> {
		let wanted = match argument {
			Value::Number(number) => number.to_string(),
			Value::String(text) => text.clone(),
			_ => return None,
		};

		self.nodes
			.values()
			.find(|node| node_id(&node.id) == wanted)
			.or_else(|| self.nodes.values().find(|node| node.key.as_str() == wanted))
			.copied()
	}
}

fn node_argument<'a>(index: &Index<'a>, arguments: &Value) -> Result<&'a Node, String> {
	let argument = arguments
		.get("node")
		.ok_or("this tool needs a node: an id from inspect_tree, or a match key")?;
	index
		.find(argument)
		.ok_or_else(|| format!("no node {argument} in this session's tree"))
}

fn inspect_tree(hub: &Hub, session: &SessionInfo, arguments: &Value) -> Result<String, String> {
	let (_, tree) = hub.tree(session)?;
	let index = Index::new(&tree);

	let root = match arguments.get("root") {
		Some(argument) => {
			index
				.find(argument)
				.ok_or_else(|| format!("no node {argument} in this session's tree"))?
		}
		None => {
			index
				.nodes
				.get(&tree.root)
				.copied()
				.ok_or("the tree has no root node")?
		}
	};

	let depth = arguments
		.get("depth")
		.and_then(Value::as_u64)
		.map_or(DEFAULT_DEPTH, |depth| {
			usize::try_from(depth).unwrap_or(usize::MAX)
		});

	let mut output = format!("{} — {} nodes\n", describe(session), index.nodes.len());
	let mut printed = 0;
	print_node(&index, root, 0, depth, &mut printed, &mut output);

	if printed >= MAX_NODES {
		let _ = writeln!(
			output,
			"… stopped after {MAX_NODES} nodes; pass root or a smaller depth"
		);
	}

	Ok(output)
}

fn print_node(
	index: &Index<'_>,
	node: &Node,
	level: usize,
	depth: usize,
	printed: &mut usize,
	output: &mut String,
) {
	if *printed >= MAX_NODES {
		return;
	}
	*printed += 1;
	let indent = "  ".repeat(level);
	let _ = writeln!(output, "{indent}{}", line(node));
	let children = index.children(&node.id);

	if children.is_empty() {
		return;
	}

	if level >= depth {
		let _ = writeln!(
			output,
			"{indent}  … {} more below",
			index.descendants(&node.id)
		);

		return;
	}

	for child in children {
		if let Some(child) = index.nodes.get(child) {
			print_node(index, child, level + 1, depth, printed, output);
		}
	}
}

/// One node on one line: `12 Text "Hello" .text-lg [16,60 200x20]
/// src/app.rs:14`.
fn line(node: &Node) -> String {
	let mut line = format!("{} {}", node_id(&node.id), node.kind);

	if let Some(text) = &node.text {
		let quoted: String = text.chars().take(MAX_TEXT).collect();
		let ellipsis = if text.chars().count() > MAX_TEXT {
			"…"
		} else {
			""
		};

		let _ = write!(line, " {:?}", format!("{quoted}{ellipsis}"));
	}

	for class in &node.classes {
		let _ = write!(line, " .{class}");
	}

	if let Some(frame) = &node.frame {
		let _ = write!(
			line,
			" [{},{} {}x{}]",
			frame.x.round(),
			frame.y.round(),
			frame.width.round(),
			frame.height.round()
		);
	}

	if let Some(source) = &node.source {
		let _ = write!(line, " {}:{}", source.file, source.line);
	}

	line
}

fn inspect_node(hub: &Hub, session: &SessionInfo, arguments: &Value) -> Result<String, String> {
	let (_, tree) = hub.tree(session)?;
	let index = Index::new(&tree);
	let node = node_argument(&index, arguments)?;
	let mut value = serde_json::to_value(node).map_err(|error| error.to_string())?;
	let children: Vec<Value> = index
		.children(&node.id)
		.iter()
		.filter_map(|child| index.nodes.get(child))
		.map(|child| json!({"id": child.id, "kind": child.kind}))
		.collect();
	value["children"] = Value::Array(children);
	Ok(pretty(&value))
}

fn explain_styles(hub: &Hub, session: &SessionInfo, arguments: &Value) -> Result<String, String> {
	let (view, tree) = hub.tree(session)?;
	let index = Index::new(&tree);
	let node = node_argument(&index, arguments)?;
	// The hub may carry more about a node than this CLI knows to read, such as
	// the declarations each class resolved to; pass those through.
	let raw = view["tree"]["nodes"]
		.as_array()
		.and_then(|nodes| {
			nodes.iter().find(|raw| {
				serde_json::from_value::<NodeRef>(raw["id"].clone()).ok() == Some(node.id.clone())
			})
		})
		.cloned()
		.unwrap_or_default();
	let mut explanation = json!({
		"node": node.id,
		"kind": node.kind,
		"classes": node.classes,
		"frame": node.frame,
		"source": node.source,
	});
	let mut declared = false;

	for field in ["declarations", "styles", "explanation"] {
		if let Some(value) = raw.get(field).filter(|value| !value.is_null()) {
			explanation[field] = value.clone();
			declared = true;
		}
	}

	if !declared {
		explanation["note"] = json!(
			"this session sends the active classes only, not the declarations they resolved to"
		);
	}

	if node.classes.is_empty() {
		explanation["note"] = json!("this node has no Tailwind classes");
	}

	Ok(pretty(&explanation))
}

fn query_cache(hub: &Hub, session: &SessionInfo, arguments: &Value) -> Result<String, String> {
	let view = hub.view(session)?;
	let entries = |field: &str| -> BTreeMap<String, DevtoolsValue> {
		view.get(field)
			.cloned()
			.and_then(|value| serde_json::from_value(value).ok())
			.unwrap_or_default()
	};
	let queries = entries("queries");
	let mutations = entries("mutations");

	if let Some(key) = string(arguments, "key") {
		let (kind, value) = queries
			.get(key)
			.map(|value| ("query", value))
			.or_else(|| mutations.get(key).map(|value| ("mutation", value)))
			.ok_or_else(|| {
				let keys: Vec<&String> = queries.keys().chain(mutations.keys()).collect();
				format!("no query or mutation {key:?}; the keys are {keys:?}")
			})?;

		return Ok(pretty(&json!({"key": key, "kind": kind, "value": value})));
	}

	if queries.is_empty() && mutations.is_empty() {
		return Ok(format!(
			"session {} has no queries or mutations yet",
			short(session.id.as_str())
		));
	}

	Ok(pretty(&json!({"queries": queries, "mutations": mutations})))
}

/// Says when a session did not declare that it accepts a command.
fn capability_warning(session: &SessionInfo, capability: &str) -> String {
	let declared = serde_json::to_value(&session.hello.capabilities)
		.ok()
		.and_then(|value| value.as_array().cloned())
		.unwrap_or_default()
		.iter()
		.any(|value| value.as_str() == Some(capability));

	if declared {
		String::new()
	} else {
		format!(" The session did not declare the {capability} capability, so it may ignore this.")
	}
}

fn navigate(hub: &Hub, session: &SessionInfo, arguments: &Value) -> Result<String, String> {
	let href = string(arguments, "href").ok_or("navigate needs an href, such as /settings")?;
	hub.command(
		session,
		&Command::Navigate {
			href: href.to_owned(),
		},
	)?;
	Ok(format!(
		"Sent {} to {}.{}",
		href,
		describe(session),
		capability_warning(session, "navigation")
	))
}

fn highlight(hub: &Hub, session: &SessionInfo, arguments: &Value) -> Result<String, String> {
	let (_, tree) = hub.tree(session)?;
	let index = Index::new(&tree);
	let key: MatchKey = node_argument(&index, arguments)?.key.clone();
	let targets = if arguments
		.get("all_sessions")
		.and_then(Value::as_bool)
		.unwrap_or(false)
	{
		hub.sessions()?
			.into_iter()
			.filter(|other| other.hello.app == session.hello.app)
			.collect()
	} else {
		vec![session.clone()]
	};

	let command = Command::Highlight { key: key.clone() };
	let mut sent = Vec::new();

	for target in &targets {
		hub.command(target, &command)?;
		sent.push(format!("- {}", describe(target)));
	}

	Ok(format!(
		"Highlighted {} in:\n{}",
		key.as_str(),
		sent.join("\n")
	))
}

fn set_state(hub: &Hub, session: &SessionInfo, arguments: &Value) -> Result<String, String> {
	let name = string(arguments, "name").ok_or("set_state needs the state's name")?;
	let value = arguments
		.get("value")
		.cloned()
		.ok_or("set_state needs a value")?;
	let view = hub.view(session)?;
	let current = view
		.get("state")
		.and_then(|state| state.get(name))
		.cloned()
		.and_then(|value| serde_json::from_value::<DevtoolsValue>(value).ok());
	let Some(current) = current else {
		let names: Vec<String> = view
			.get("state")
			.and_then(Value::as_object)
			.map(|state| state.keys().cloned().collect())
			.unwrap_or_default();
		return Err(format!(
			"session {} has no state named {name:?}; it exposes {names:?}",
			short(session.id.as_str())
		));
	};
	hub.command(
		session,
		&Command::SetState {
			name: name.to_owned(),
			value: DevtoolsValue::from_json(current.type_name.clone(), value),
		},
	)?;

	Ok(format!(
		"Set {name} ({}) in {}.{}",
		current.type_name,
		describe(session),
		capability_warning(session, "editing")
	))
}

#[cfg(test)]
mod tests;
