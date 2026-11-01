//! What the interface shows, and how hub changes update it.
//!
//! Everything here is plain data, so the rules are tested on the host: which
//! session is shown, which tree rows are visible, how a page reload switches
//! sessions, and where guest reloads fall on the timelines.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use leptoskit_devtools_protocol::Capability;
use leptoskit_devtools_protocol::Change;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Frame;
use leptoskit_devtools_protocol::MatchKey;
use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::Platform;
use leptoskit_devtools_protocol::SessionId;
use leptoskit_devtools_protocol::SessionInfo;
use leptoskit_devtools_protocol::SessionState;
use leptoskit_devtools_protocol::SessionView;
use serde::Deserialize;

/// The most tree rows drawn at once; a search narrows a larger tree.
pub const MAX_ROWS: usize = 1500;

/// What `GET /api/hub` says about the hub.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct HubInfo {
	/// The CLI's version, which is also the interface's.
	pub version: String,
	/// The project's root, for opening source files.
	pub root: String,
}

/// Whether the change stream is connected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Connection {
	/// Loading the sessions and opening the stream.
	#[default]
	Connecting,
	/// Receiving changes.
	Live,
	/// The stream dropped; the browser reconnects on its own.
	Lost,
}

/// The panel beside the tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Panel {
	/// The selected node: source, frame, classes, other platforms.
	#[default]
	Node,
	/// The selected node's classes and their declarations.
	Styles,
	/// The current route.
	Route,
	/// The query cache.
	Queries,
	/// Mutations.
	Mutations,
	/// Tracked forms.
	Forms,
	/// Named state.
	State,
	/// Plugin and server calls.
	Calls,
	/// Render commits.
	Commits,
}

impl Panel {
	/// Every panel, in tab order.
	pub const ALL: [Self; 9] = [
		Self::Node,
		Self::Styles,
		Self::Route,
		Self::Queries,
		Self::Mutations,
		Self::Forms,
		Self::State,
		Self::Calls,
		Self::Commits,
	];

	/// The tab's label.
	#[must_use]
	pub fn label(self) -> &'static str {
		match self {
			Self::Node => "Node",
			Self::Styles => "Styles",
			Self::Route => "Route",
			Self::Queries => "Queries",
			Self::Mutations => "Mutations",
			Self::Forms => "Forms",
			Self::State => "State",
			Self::Calls => "Calls",
			Self::Commits => "Commits",
		}
	}
}

/// One session the interface knows, with its mirror.
#[derive(Clone, Debug, PartialEq)]
pub struct Live {
	/// What the agent said when it opened the session.
	pub info: SessionInfo,
	/// The mirrored state, kept current from the stream.
	pub view: SessionView,
	/// When the guest reloaded, in milliseconds since the session opened.
	pub reloads: Vec<u64>,
	/// Whether commit profiling is on.
	pub profiling: bool,
}

impl Live {
	/// A session with nothing mirrored yet.
	#[must_use]
	pub fn new(info: SessionInfo) -> Self {
		Self {
			info,
			view: SessionView::default(),
			reloads: Vec::new(),
			profiling: false,
		}
	}

	/// Whether the session feeds or accepts `capability`.
	#[must_use]
	pub fn has(&self, capability: Capability) -> bool {
		self.info.hello.has(capability)
	}

	/// The session's platform and device, for labels.
	#[must_use]
	pub fn label(&self) -> String {
		let hello = &self.info.hello;
		match &hello.device {
			Some(device) => format!("{} · {device}", platform_name(hello.platform)),
			None => platform_name(hello.platform).to_owned(),
		}
	}

	/// Whether the guest reloads in place rather than as a new session.
	fn reloads_in_place(&self) -> bool {
		!matches!(
			self.info.hello.platform,
			Platform::Web | Platform::Extension
		)
	}
}

/// The selected node: its id in the shown session, and the key that finds
/// it again after a reload or on another platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
	/// The node in the shown session.
	pub node: NodeRef,
	/// Its cross-platform key.
	pub key: MatchKey,
}

/// What the user is typing, before it is sent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Drafts {
	/// The selected node's classes.
	pub classes: Option<String>,
	/// The route to navigate to.
	pub route: Option<String>,
	/// Query data being edited, by key.
	pub queries: BTreeMap<String, String>,
	/// Named state being edited, by name.
	pub state: BTreeMap<String, String>,
}

/// One visible tree row.
#[derive(Clone, Debug, PartialEq)]
pub struct Row<'a> {
	/// The node.
	pub node: &'a Node,
	/// How deep it is.
	pub depth: usize,
	/// Whether it has children.
	pub parent: bool,
	/// Whether its children are hidden.
	pub collapsed: bool,
	/// Whether it matches the search.
	pub matched: bool,
	/// The `#[component]` whose output starts here: the node's component,
	/// when its parent belongs to another one.
	pub component: Option<&'a str>,
}

/// One entry on the calls or commits timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum Moment<'a> {
	/// A plugin bridge call.
	Call(&'a leptoskit_devtools_protocol::CallRecord),
	/// A server rpc attempt.
	Rpc(&'a leptoskit_devtools_protocol::RpcAttempt),
	/// A render commit.
	Commit(&'a leptoskit_devtools_protocol::Commit),
	/// The guest reloaded.
	Reload(u64),
}

impl Moment<'_> {
	fn at(&self) -> u64 {
		match self {
			Self::Call(call) => call.started_ms,
			Self::Rpc(attempt) => attempt.started_ms,
			Self::Commit(commit) => commit.at_ms,
			Self::Reload(at) => *at,
		}
	}
}

/// The same node in another session of the app.
#[derive(Clone, Debug, PartialEq)]
pub struct Counterpart<'a> {
	/// The other session.
	pub session: &'a Live,
	/// Its node with the same match key.
	pub node: &'a Node,
}

/// Everything the interface shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
	/// Framed by the browser `DevTools` panel: no sessions sidebar.
	pub embed: bool,
	/// Whether the change stream is connected.
	pub connection: Connection,
	/// What the hub said about itself.
	pub hub: Option<HubInfo>,
	/// Every session, by id.
	pub sessions: BTreeMap<SessionId, Live>,
	/// The session shown.
	pub current: Option<SessionId>,
	/// A session asked for before it was known, such as by the panel.
	pub wanted: Option<SessionId>,
	/// In the panel, whether it has had its chance to name a session, so the
	/// interface may choose one itself.
	pub waited: bool,
	/// The selected node.
	pub selected: Option<Selection>,
	/// Nodes whose children are hidden.
	pub collapsed: BTreeSet<NodeRef>,
	/// The tree search.
	pub search: String,
	/// Whether picking is on in the shown session.
	pub picking: bool,
	/// The panel beside the tree.
	pub panel: Panel,
	/// Unsent edits.
	pub drafts: Drafts,
	/// The last problem, until it is dismissed.
	pub notice: Option<String>,
}

impl Model {
	/// The shown session.
	#[must_use]
	pub fn session(&self) -> Option<&Live> {
		self.current.as_ref().and_then(|id| self.sessions.get(id))
	}

	/// The selected node, in the shown session's tree.
	#[must_use]
	pub fn node(&self) -> Option<&Node> {
		let selection = self.selected.as_ref()?;
		self.session()?.view.node(&selection.node)
	}

	/// Whether a newer session of the same app runs on the same platform and
	/// device: this one is an earlier page load that hasn't expired yet.
	#[must_use]
	pub fn superseded(&self, live: &Live) -> bool {
		self.sessions.values().any(|other| {
			other.info.hello.app == live.info.hello.app
				&& other.info.hello.platform == live.info.hello.platform
				&& other.info.hello.device == live.info.hello.device
				&& other.info.connected_at_ms > live.info.connected_at_ms
		})
	}

	/// Sessions grouped by app, each group newest first.
	#[must_use]
	pub fn apps(&self) -> Vec<(&str, Vec<&Live>)> {
		let mut apps: BTreeMap<&str, Vec<&Live>> = BTreeMap::new();

		for live in self.sessions.values() {
			apps.entry(live.info.hello.app.as_str())
				.or_default()
				.push(live);
		}

		apps.into_iter()
			.map(|(app, mut sessions)| {
				sessions.sort_by_key(|live| std::cmp::Reverse(live.info.connected_at_ms));
				(app, sessions)
			})
			.collect()
	}

	/// Replace every session with the hub's answers, keeping what is shown
	/// when it still exists.
	pub fn load(&mut self, states: Vec<SessionState>) {
		let previous = std::mem::take(&mut self.sessions);
		for state in states {
			let mut live = Live::new(state.info);
			live.view = state.view;

			if let Some(old) = previous.get(&live.info.id) {
				live.reloads.clone_from(&old.reloads);
				live.profiling = old.profiling;
			}

			self.sessions.insert(live.info.id.clone(), live);
		}

		if self
			.current
			.as_ref()
			.is_some_and(|id| !self.sessions.contains_key(id))
		{
			self.current = None;
		}

		if let Some(wanted) = self.wanted.clone()
			&& self.sessions.contains_key(&wanted)
		{
			self.show(&wanted);
		} else if self.current.is_none() && self.may_choose() {
			self.show_default();
		}
	}

	/// Whether the interface may pick a session itself: always, unless it is
	/// in the panel and the panel may still name one.
	#[must_use]
	pub fn may_choose(&self) -> bool {
		!self.embed || self.waited
	}

	/// In the panel, give up waiting for it to name a session: show the newest
	/// web session, and still switch if the named one appears later.
	pub fn stop_waiting(&mut self) {
		self.waited = true;

		if self.current.is_none() {
			let wanted = self.wanted.clone();
			self.show_default();
			if self.wanted.is_none() {
				self.wanted = wanted.filter(|id| !self.sessions.contains_key(id));
			}
		}
	}

	/// Show the newest web session, else the newest session.
	pub fn show_default(&mut self) {
		let newest = |web: bool| {
			self.sessions
				.values()
				.filter(|live| !web || live.info.hello.platform == Platform::Web)
				.max_by_key(|live| live.info.connected_at_ms)
				.map(|live| live.info.id.clone())
		};

		if let Some(id) = newest(true).or_else(|| newest(false)) {
			self.show(&id);
		}
	}

	/// Show a session, or remember it until the hub reports it.
	pub fn show(&mut self, id: &SessionId) {
		if !self.sessions.contains_key(id) {
			self.wanted = Some(id.clone());
			return;
		}

		self.wanted = None;

		if self.current.as_ref() == Some(id) {
			return;
		}

		let same_app = match (self.session(), self.sessions.get(id)) {
			(Some(old), Some(new)) => old.info.hello.app == new.info.hello.app,
			_ => false,
		};
		self.current = Some(id.clone());
		self.picking = false;
		self.drafts = Drafts::default();

		if same_app {
			self.reselect();
		} else {
			self.selected = None;
			self.collapsed.clear();
		}
	}

	/// Apply one change from the stream. Returns whether the interface should
	/// ask the hub for every session again, because the change named a
	/// session it does not know.
	pub fn apply(&mut self, change: Change, now_ms: u64) -> bool {
		match change {
			Change::Opened { info } => {
				self.opened(info);
				false
			}
			Change::Closed { session } => {
				self.closed(&session);
				false
			}
			Change::Event { session, event } => self.event(&session, &event, now_ms),
			Change::Command { session, command } => {
				self.command(&session, &command);
				false
			}
		}
	}

	fn opened(&mut self, info: SessionInfo) {
		let id = info.id.clone();
		let reload = self.session().is_some_and(|shown| {
			shown.info.hello.app == info.hello.app
				&& shown.info.hello.platform == info.hello.platform
				&& !shown.reloads_in_place()
		});
		self.sessions.insert(id.clone(), Live::new(info));

		if self.wanted.as_ref() == Some(&id)
			|| reload
			|| (self.current.is_none() && self.may_choose())
		{
			self.show(&id);
		}
	}

	fn closed(&mut self, id: &SessionId) {
		let Some(gone) = self.sessions.remove(id) else {
			return;
		};

		if self.current.as_ref() != Some(id) {
			return;
		}

		self.current = None;
		// Follow the app to its newest session on the same platform, which is
		// where a reloaded page reports from.
		let next = self
			.sessions
			.values()
			.filter(|live| {
				live.info.hello.app == gone.info.hello.app
					&& live.info.hello.platform == gone.info.hello.platform
			})
			.max_by_key(|live| live.info.connected_at_ms)
			.map(|live| live.info.id.clone());

		match next {
			Some(next) => self.show(&next),
			None => {
				self.selected = None;
				self.picking = false;
				if self.may_choose() {
					self.show_default();
				}
			}
		}
	}

	fn event(&mut self, id: &SessionId, event: &Event, now_ms: u64) -> bool {
		let shown = self.current.as_ref() == Some(id);
		let Some(live) = self.sessions.get_mut(id) else {
			return true;
		};

		if let Event::Snapshot { .. } = event
			&& live.view.tree.is_some()
			&& live.reloads_in_place()
		{
			let at = now_ms.saturating_sub(live.info.connected_at_ms);
			live.reloads.push(at);
		}

		live.view.apply(event);

		if !shown {
			return false;
		}

		match event {
			Event::Snapshot { .. } | Event::TreePatch { .. } => self.reselect(),
			Event::Picked { node, key } => {
				self.picking = false;
				self.select(node.clone(), key.clone());
			}
			_ => {}
		}

		false
	}

	fn command(&mut self, id: &SessionId, command: &Command) {
		if let Some(live) = self.sessions.get_mut(id) {
			match command {
				Command::StartProfiling => live.profiling = true,
				Command::StopProfiling => live.profiling = false,
				_ => {}
			}
		}

		if self.current.as_ref() == Some(id)
			&& let Command::Pick { on } = command
		{
			self.picking = *on;
		}
	}

	/// Select a node and reveal it in the tree.
	pub fn select(&mut self, node: NodeRef, key: MatchKey) {
		self.drafts.classes = None;
		let ancestors = self
			.session()
			.map(|live| ancestors(&live.view, &node))
			.unwrap_or_default();

		for ancestor in ancestors {
			self.collapsed.remove(&ancestor);
		}

		self.selected = Some(Selection { node, key });
	}

	/// Find the selected node again by its key, after the tree changed or the
	/// session did. The selection clears when the key is gone.
	fn reselect(&mut self) {
		let Some(selection) = self.selected.clone() else {
			return;
		};
		let Some(live) = self.session() else {
			return;
		};
		let Some(tree) = &live.view.tree else {
			return;
		};

		if tree
			.nodes
			.iter()
			.any(|node| node.id == selection.node && node.key == selection.key)
		{
			return;
		}

		match tree.nodes.iter().find(|node| node.key == selection.key) {
			Some(node) => {
				let node = node.id.clone();
				self.select(node, selection.key);
			}
			None => self.selected = None,
		}
	}

	/// Show or hide a node's children.
	pub fn toggle(&mut self, node: &NodeRef) {
		if !self.collapsed.remove(node) {
			self.collapsed.insert(node.clone());
		}
	}

	/// The tree rows to draw: every expanded node, or with a search, the
	/// matches and their ancestors.
	#[must_use]
	pub fn rows(&self) -> Vec<Row<'_>> {
		let Some(tree) = self.session().and_then(|live| live.view.tree.as_ref()) else {
			return Vec::new();
		};
		let nodes: BTreeMap<&NodeRef, &Node> =
			tree.nodes.iter().map(|node| (&node.id, node)).collect();
		let search = self.search.trim().to_lowercase();
		let keep = if search.is_empty() {
			BTreeSet::new()
		} else {
			matched(&nodes, &search)
		};

		let mut rows = Vec::new();
		let mut pending = vec![(&tree.root, 0)];
		let mut seen = BTreeSet::new();

		while let Some((id, depth)) = pending.pop() {
			if rows.len() == MAX_ROWS {
				break;
			}

			let Some(node) = nodes.get(id) else {
				continue;
			};

			if !seen.insert(id) || (!search.is_empty() && !keep.contains(id)) {
				continue;
			}

			let collapsed = search.is_empty() && self.collapsed.contains(id);
			let parent_component = node
				.parent
				.as_ref()
				.and_then(|parent| nodes.get(parent))
				.and_then(|parent| parent.component.as_deref());
			rows.push(Row {
				node,
				depth,
				parent: !node.children.is_empty(),
				collapsed,
				matched: !search.is_empty() && matches(node, &search),
				component: node
					.component
					.as_deref()
					.filter(|component| Some(*component) != parent_component),
			});

			if !collapsed {
				pending.extend(node.children.iter().rev().map(|child| (child, depth + 1)));
			}
		}

		rows
	}

	/// How many nodes the shown tree has.
	#[must_use]
	pub fn node_count(&self) -> usize {
		self.session()
			.and_then(|live| live.view.tree.as_ref())
			.map_or(0, |tree| tree.nodes.len())
	}

	/// The selected node in every other session of the same app.
	#[must_use]
	pub fn counterparts(&self) -> Vec<Counterpart<'_>> {
		let (Some(shown), Some(selection)) = (self.session(), &self.selected) else {
			return Vec::new();
		};

		self.sessions
			.values()
			.filter(|live| {
				live.info.id != shown.info.id
					&& live.info.hello.app == shown.info.hello.app
					&& !self.superseded(live)
			})
			.filter_map(|live| {
				let node = live
					.view
					.tree
					.as_ref()?
					.nodes
					.iter()
					.find(|node| node.key == selection.key)?;
				Some(Counterpart {
					session: live,
					node,
				})
			})
			.collect()
	}

	/// The shown session's calls and rpc attempts, with guest reloads, oldest
	/// first.
	#[must_use]
	pub fn calls(&self) -> Vec<Moment<'_>> {
		let Some(live) = self.session() else {
			return Vec::new();
		};
		let mut moments: Vec<Moment<'_>> = live
			.view
			.calls
			.iter()
			.map(Moment::Call)
			.chain(live.view.rpc.iter().map(Moment::Rpc))
			.chain(live.reloads.iter().copied().map(Moment::Reload))
			.collect();
		moments.sort_by_key(Moment::at);
		moments
	}

	/// The shown session's commits, with guest reloads, oldest first.
	#[must_use]
	pub fn commits(&self) -> Vec<Moment<'_>> {
		let Some(live) = self.session() else {
			return Vec::new();
		};
		let mut moments: Vec<Moment<'_>> = live
			.view
			.commits
			.iter()
			.map(Moment::Commit)
			.chain(live.reloads.iter().copied().map(Moment::Reload))
			.collect();
		moments.sort_by_key(Moment::at);
		moments
	}

	/// The address that opens a source location in an editor.
	#[must_use]
	pub fn editor_url(&self, source: &leptoskit_devtools_protocol::SourceLocation) -> String {
		let file = &source.file;
		let absolute = file.starts_with('/') || file.get(1..3) == Some(":\\");
		let path = match &self.hub {
			Some(hub) if !absolute && !hub.root.is_empty() => {
				format!("{}/{file}", hub.root.trim_end_matches('/'))
			}
			_ => file.clone(),
		};
		format!(
			"vscode://file/{}:{}:{}",
			path.trim_start_matches('/'),
			source.line,
			source.column
		)
	}
}

/// A platform's name, for labels.
#[must_use]
pub fn platform_name(platform: Platform) -> &'static str {
	match platform {
		Platform::Web => "web",
		Platform::Macos => "macOS",
		Platform::Ios => "iOS",
		Platform::Android => "Android",
		Platform::Windows => "Windows",
		Platform::Linux => "Linux",
		Platform::Harmonyos => "HarmonyOS",
		Platform::Tui => "terminal",
		Platform::Extension => "extension",
	}
}

/// A frame as `x, y · w × h`.
#[must_use]
pub fn frame_text(frame: &Frame) -> String {
	format!(
		"{}, {} · {} × {}",
		round(frame.x),
		round(frame.y),
		round(frame.width),
		round(frame.height)
	)
}

fn round(value: f32) -> String {
	let rounded = (value * 10.0).round() / 10.0;
	if rounded.fract() == 0.0 {
		format!("{rounded:.0}")
	} else {
		format!("{rounded:.1}")
	}
}

/// A node ref as text.
#[must_use]
pub fn node_ref_text(node: &NodeRef) -> String {
	match node {
		NodeRef::Native(id) => id.to_string(),
		NodeRef::Web(path) => path.clone(),
	}
}

fn matches(node: &Node, search: &str) -> bool {
	node.kind.to_lowercase().contains(search)
		|| node
			.classes
			.iter()
			.any(|class| class.to_lowercase().contains(search))
		|| node
			.text
			.as_ref()
			.is_some_and(|text| text.to_lowercase().contains(search))
		|| node.key.as_str().to_lowercase() == search
		|| node
			.component
			.as_ref()
			.is_some_and(|component| component.to_lowercase().contains(search))
}

/// The nodes matching `search`, with every ancestor of each.
fn matched<'a>(nodes: &BTreeMap<&'a NodeRef, &'a Node>, search: &str) -> BTreeSet<&'a NodeRef> {
	let mut keep = BTreeSet::new();

	for node in nodes.values().filter(|node| matches(node, search)) {
		let mut current = Some(&node.id);
		while let Some(id) = current
			&& keep.insert(id)
		{
			current = nodes.get(id).and_then(|node| node.parent.as_ref());
		}
	}

	keep
}

fn ancestors(view: &SessionView, node: &NodeRef) -> Vec<NodeRef> {
	let mut found = Vec::new();
	let mut current = view.node(node).and_then(|node| node.parent.clone());

	while let Some(id) = current {
		if found.contains(&id) {
			break;
		}
		current = view.node(&id).and_then(|node| node.parent.clone());
		found.push(id);
	}

	found
}

#[cfg(test)]
pub(crate) mod tests {
	use leptoskit_devtools_protocol::CallRecord;
	use leptoskit_devtools_protocol::CallStatus;
	use leptoskit_devtools_protocol::Commit;
	use leptoskit_devtools_protocol::Hello;
	use leptoskit_devtools_protocol::Snapshot;
	use leptoskit_devtools_protocol::SourceLocation;
	use leptoskit_devtools_protocol::Tree;

	use super::*;

	pub(crate) fn info(id: &str, app: &str, platform: Platform, at: u64) -> SessionInfo {
		let capabilities = if platform == Platform::Web {
			vec![
				Capability::Tree,
				Capability::Styles,
				Capability::Picking,
				Capability::Navigation,
			]
		} else {
			vec![
				Capability::Tree,
				Capability::Styles,
				Capability::Commits,
				Capability::Calls,
				Capability::Profiling,
				Capability::Queries,
				Capability::State,
				Capability::Editing,
			]
		};
		SessionInfo {
			id: SessionId::new(id),
			hello: Hello::new(
				app,
				platform,
				(platform == Platform::Ios).then(|| "iPhone 17".to_owned()),
				capabilities,
			),
			connected_at_ms: at,
			last_seen_ms: at,
		}
	}

	pub(crate) fn node(id: &str, parent: Option<&str>, kind: &str, children: &[&str]) -> Node {
		Node {
			id: NodeRef::Web(id.into()),
			key: MatchKey::new(format!("src/app.rs#{id}")),
			parent: parent.map(|parent| NodeRef::Web(parent.into())),
			kind: kind.into(),
			source: Some(SourceLocation {
				file: "src/app.rs".into(),
				line: u32::try_from(id.len()).unwrap(),
				column: 1,
			}),
			frame: Some(Frame {
				x: 0.0,
				y: 10.0 * id.len() as f32,
				width: 320.0,
				height: 44.5,
			}),
			classes: vec!["p-4".into(), format!("gap-{}", id.len())],
			text: (kind == "Text").then(|| format!("hello {id}")),
			children: children
				.iter()
				.map(|child| NodeRef::Web((*child).into()))
				.collect(),
			component: None,
			styles: Vec::new(),
		}
	}

	pub(crate) fn tree() -> Tree {
		Tree {
			root: NodeRef::Web("r".into()),
			nodes: vec![
				node("r", None, "View", &["ra", "rb"]),
				node("ra", Some("r"), "Text", &[]),
				node("rb", Some("r"), "View", &["rbc"]),
				node("rbc", Some("rb"), "Button", &[]),
			],
		}
	}

	pub(crate) fn snapshot() -> Event {
		Event::Snapshot {
			snapshot: Snapshot {
				tree: tree(),
				route: Some("/clips".into()),
				queries: Vec::new(),
				mutations: Vec::new(),
				forms: Vec::new(),
				state: Vec::new(),
				calls: Vec::new(),
				rpc: Vec::new(),
			},
		}
	}

	fn opened(model: &mut Model, id: &str, platform: Platform, at: u64) {
		model.apply(
			Change::Opened {
				info: info(id, "commonplace", platform, at),
			},
			at,
		);
		model.apply(
			Change::Event {
				session: SessionId::new(id),
				event: snapshot(),
			},
			at,
		);
	}

	#[test]
	fn the_first_session_is_shown() {
		let mut model = Model::default();
		opened(&mut model, "ios", Platform::Ios, 1);
		assert_eq!(model.current, Some(SessionId::new("ios")));
	}

	#[test]
	fn loading_prefers_the_newest_web_session() {
		let mut model = Model::default();
		let state = |id, platform, at| {
			let mut view = SessionView::default();
			view.apply(&snapshot());
			SessionState {
				info: info(id, "commonplace", platform, at),
				view,
			}
		};
		model.load(vec![
			state("old-web", Platform::Web, 1),
			state("web", Platform::Web, 2),
			state("ios", Platform::Ios, 3),
		]);
		assert_eq!(model.current, Some(SessionId::new("web")));
		assert_eq!(
			model.session().unwrap().view.route.as_deref(),
			Some("/clips")
		);
	}

	#[test]
	fn an_embedded_page_waits_for_the_panel_to_name_a_session() {
		let mut model = Model {
			embed: true,
			..Model::default()
		};
		model.show(&SessionId::new("later"));
		opened(&mut model, "web", Platform::Web, 1);
		assert_eq!(model.current, None);
		opened(&mut model, "later", Platform::Web, 2);
		assert_eq!(model.current, Some(SessionId::new("later")));
		assert_eq!(model.wanted, None);
	}

	#[test]
	fn an_embedded_page_falls_back_to_the_newest_web_session() {
		let mut model = Model {
			embed: true,
			..Model::default()
		};
		opened(&mut model, "web", Platform::Web, 1);
		opened(&mut model, "ios", Platform::Ios, 2);
		model.show(&SessionId::new("missing"));
		assert_eq!(model.current, None);
		model.stop_waiting();
		assert_eq!(model.current, Some(SessionId::new("web")));
		// The named session still wins when it turns up.
		opened(&mut model, "missing", Platform::Web, 3);
		assert_eq!(model.current, Some(SessionId::new("missing")));
	}

	#[test]
	fn a_page_reload_switches_to_the_new_session_and_keeps_the_selection() {
		let mut model = Model::default();
		opened(&mut model, "web-1", Platform::Web, 1);
		let node = NodeRef::Web("rbc".into());
		let key = model
			.session()
			.unwrap()
			.view
			.node(&node)
			.unwrap()
			.key
			.clone();
		model.select(node.clone(), key.clone());
		opened(&mut model, "ios", Platform::Ios, 2);
		assert_eq!(model.current, Some(SessionId::new("web-1")));

		opened(&mut model, "web-2", Platform::Web, 3);
		assert_eq!(model.current, Some(SessionId::new("web-2")));
		assert_eq!(model.selected, Some(Selection { node, key }));

		model.apply(
			Change::Closed {
				session: SessionId::new("web-2"),
			},
			4,
		);
		assert_eq!(model.current, Some(SessionId::new("web-1")));
	}

	#[test]
	fn a_closed_session_falls_back_to_another() {
		let mut model = Model::default();
		opened(&mut model, "ios", Platform::Ios, 1);
		opened(&mut model, "web", Platform::Web, 2);
		model.show(&SessionId::new("ios"));
		model.apply(
			Change::Closed {
				session: SessionId::new("ios"),
			},
			3,
		);
		assert_eq!(model.current, Some(SessionId::new("web")));
	}

	#[test]
	fn a_native_snapshot_after_the_first_marks_a_guest_reload() {
		let mut model = Model::default();
		opened(&mut model, "ios", Platform::Ios, 1000);
		assert!(model.session().unwrap().reloads.is_empty());
		model.apply(
			Change::Event {
				session: SessionId::new("ios"),
				event: Event::Commit {
					commit: Commit {
						seq: 1,
						at_ms: 500,
						mutations: 3,
						layout_us: Some(40),
						apply_us: Some(20),
					},
				},
			},
			1500,
		);
		model.apply(
			Change::Event {
				session: SessionId::new("ios"),
				event: snapshot(),
			},
			3000,
		);
		assert_eq!(model.session().unwrap().reloads, [2000]);
		assert_eq!(model.commits(), [Moment::Reload(2000)]);

		// A web snapshot is a new page, not a reload in place.
		opened(&mut model, "web", Platform::Web, 1);
		model.apply(
			Change::Event {
				session: SessionId::new("web"),
				event: snapshot(),
			},
			2,
		);
		assert!(model.sessions[&SessionId::new("web")].reloads.is_empty());
	}

	#[test]
	fn an_event_for_an_unknown_session_asks_for_a_refresh() {
		let mut model = Model::default();
		assert!(model.apply(
			Change::Event {
				session: SessionId::new("nobody"),
				event: snapshot(),
			},
			1,
		));
	}

	#[test]
	fn rows_follow_collapsing_and_search() {
		let mut model = Model::default();
		opened(&mut model, "web", Platform::Web, 1);
		let ids = |model: &Model| {
			model
				.rows()
				.iter()
				.map(|row| node_ref_text(&row.node.id))
				.collect::<Vec<_>>()
		};
		assert_eq!(ids(&model), ["r", "ra", "rb", "rbc"]);
		assert_eq!(model.rows()[3].depth, 2);

		model.toggle(&NodeRef::Web("rb".into()));
		assert_eq!(ids(&model), ["r", "ra", "rb"]);
		assert!(model.rows()[2].collapsed);

		// A search shows matches inside collapsed nodes, with their ancestors.
		model.search = "button".into();
		assert_eq!(ids(&model), ["r", "rb", "rbc"]);
		assert!(model.rows()[2].matched);
		assert!(!model.rows()[0].matched);

		model.search = "hello ra".into();
		assert_eq!(ids(&model), ["r", "ra"]);
		model.search = "gap-3".into();
		assert_eq!(ids(&model), ["r", "rb", "rbc"]);
	}

	#[test]
	fn picking_selects_and_reveals_the_node() {
		let mut model = Model::default();
		opened(&mut model, "web", Platform::Web, 1);
		model.toggle(&NodeRef::Web("rb".into()));
		model.picking = true;
		let node = NodeRef::Web("rbc".into());
		model.apply(
			Change::Event {
				session: SessionId::new("web"),
				event: Event::Picked {
					node: node.clone(),
					key: MatchKey::new("src/app.rs#rbc"),
				},
			},
			2,
		);
		assert!(!model.picking);
		assert_eq!(model.node().unwrap().kind, "Button");
		assert!(model.collapsed.is_empty());
	}

	#[test]
	fn counterparts_are_the_same_key_in_other_sessions_of_the_app() {
		let mut model = Model::default();
		opened(&mut model, "web", Platform::Web, 1);
		opened(&mut model, "ios", Platform::Ios, 2);
		model.apply(
			Change::Opened {
				info: info("other", "another-app", Platform::Ios, 3),
			},
			3,
		);
		model.apply(
			Change::Event {
				session: SessionId::new("other"),
				event: snapshot(),
			},
			3,
		);
		let node = NodeRef::Web("ra".into());
		let key = model
			.session()
			.unwrap()
			.view
			.node(&node)
			.unwrap()
			.key
			.clone();
		model.select(node, key);
		let counterparts = model.counterparts();
		assert_eq!(counterparts.len(), 1);
		assert_eq!(counterparts[0].session.info.id, SessionId::new("ios"));

		// An earlier load of the same platform is left out once a newer one
		// runs.
		opened(&mut model, "ios-2", Platform::Ios, 4);
		model.show(&SessionId::new("web"));
		let counterparts = model.counterparts();
		assert_eq!(counterparts.len(), 1);
		assert_eq!(counterparts[0].session.info.id, SessionId::new("ios-2"));
		assert!(model.superseded(&model.sessions[&SessionId::new("ios")]));
	}

	#[test]
	fn the_timeline_interleaves_calls_attempts_and_reloads() {
		let mut model = Model::default();
		opened(&mut model, "ios", Platform::Ios, 0);
		let live = model.sessions.get_mut(&SessionId::new("ios")).unwrap();
		live.view.calls.push(CallRecord {
			id: 1,
			plugin: "http".into(),
			method: "fetch".into(),
			started_ms: 30,
			running_ms: Some(31),
			finished_ms: Some(50),
			taken_ms: Some(51),
			completed_late_ms: None,
			status: CallStatus::Ok,
			request_bytes: None,
			response_bytes: None,
		});
		live.view.rpc.push(leptoskit_devtools_protocol::RpcAttempt {
			id: 1,
			function: "clips".into(),
			attempt: 1,
			started_ms: 10,
			finished_ms: None,
			error: None,
		});
		live.reloads.push(20);
		let order: Vec<u64> = model.calls().iter().map(Moment::at).collect();
		assert_eq!(order, [10, 20, 30]);
	}

	#[test]
	fn profiling_and_picking_follow_commands_from_any_front_end() {
		let mut model = Model::default();
		opened(&mut model, "ios", Platform::Ios, 0);
		let session = SessionId::new("ios");
		model.apply(
			Change::Command {
				session: session.clone(),
				command: Command::StartProfiling,
			},
			1,
		);
		model.apply(
			Change::Command {
				session,
				command: Command::Pick { on: true },
			},
			1,
		);
		assert!(model.session().unwrap().profiling);
		assert!(model.picking);
	}

	#[test]
	fn editor_links_resolve_against_the_project_root() {
		let model = Model {
			hub: Some(HubInfo {
				version: "0.1.0".into(),
				root: "/work/app/".into(),
			}),
			..Model::default()
		};
		let source = SourceLocation {
			file: "src/lib.rs".into(),
			line: 4,
			column: 9,
		};
		assert_eq!(
			model.editor_url(&source),
			"vscode://file/work/app/src/lib.rs:4:9"
		);
		let source = SourceLocation {
			file: "/abs/lib.rs".into(),
			..source
		};
		assert_eq!(model.editor_url(&source), "vscode://file/abs/lib.rs:4:9");
	}

	#[test]
	fn frames_read_cleanly() {
		let frame = Frame {
			x: 0.0,
			y: 12.25,
			width: 320.0,
			height: 44.5,
		};
		assert_eq!(frame_text(&frame), "0, 12.3 · 320 × 44.5");
	}
}
