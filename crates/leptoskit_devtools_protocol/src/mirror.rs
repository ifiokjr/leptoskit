//! The hub's mirror of each session, and the changes it streams to front
//! ends.
//!
//! The hub applies every event an agent posts to that session's
//! [`SessionView`], and front ends apply the same events from
//! `GET /api/stream`, so both hold the same state without asking again.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::Serialize;

use crate::CallRecord;
use crate::Command;
use crate::Commit;
use crate::Event;
use crate::Node;
use crate::NodeRef;
use crate::RpcAttempt;
use crate::SessionId;
use crate::SessionInfo;
use crate::Snapshot;
use crate::Tree;
use crate::TreeOp;
use crate::TreePatch;
use crate::Value;

/// How many commits, calls, or rpc attempts a mirror keeps.
pub const HISTORY: usize = 256;

/// The mirrored state of one session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionView {
	/// The latest tree.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tree: Option<Tree>,
	/// The current route.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub route: Option<String>,
	/// The current queries, by key.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub queries: BTreeMap<String, Value>,
	/// The current mutations, by key.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub mutations: BTreeMap<String, Value>,
	/// The current forms, by name.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub forms: BTreeMap<String, Value>,
	/// The current named state, by name.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub state: BTreeMap<String, Value>,
	/// The latest commits.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub commits: Vec<Commit>,
	/// The latest bridge calls.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub calls: Vec<CallRecord>,
	/// The latest rpc attempts.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub rpc: Vec<RpcAttempt>,
}

impl SessionView {
	/// Mirror one event.
	pub fn apply(&mut self, event: &Event) {
		match event {
			Event::Snapshot { snapshot } => self.apply_snapshot(snapshot),
			Event::TreePatch { patch } => {
				if let Some(tree) = &mut self.tree {
					apply_patch(tree, patch);
				}
			}
			Event::Commit { commit } => push(&mut self.commits, commit.clone()),
			Event::RouteChanged { route } => self.route = Some(route.clone()),
			Event::QueryChanged { key, value } => {
				self.queries.insert(key.clone(), value.clone());
			}
			Event::MutationChanged { key, value } => {
				self.mutations.insert(key.clone(), value.clone());
			}
			Event::FormChanged { name, value } => {
				self.forms.insert(name.clone(), value.clone());
			}
			Event::StateChanged { name, value } => {
				self.state.insert(name.clone(), value.clone());
			}
			Event::Call { call } => {
				match self.calls.iter_mut().find(|item| item.id == call.id) {
					Some(item) => *item = call.clone(),
					None => push(&mut self.calls, call.clone()),
				}
			}
			Event::RpcAttempt { attempt } => {
				match self.rpc.iter_mut().find(|item| item.id == attempt.id) {
					Some(item) => *item = attempt.clone(),
					None => push(&mut self.rpc, attempt.clone()),
				}
			}
			Event::Picked { .. } => {}
		}
	}

	/// A node of the current tree.
	#[must_use]
	pub fn node(&self, id: &NodeRef) -> Option<&Node> {
		self.tree
			.as_ref()
			.and_then(|tree| tree.nodes.iter().find(|node| &node.id == id))
	}

	fn apply_snapshot(&mut self, snapshot: &Snapshot) {
		let named = |values: &[crate::NamedValue]| {
			values
				.iter()
				.map(|value| (value.name.clone(), value.value.clone()))
				.collect()
		};
		self.tree = Some(snapshot.tree.clone());
		self.route.clone_from(&snapshot.route);
		self.queries = named(&snapshot.queries);
		self.mutations = named(&snapshot.mutations);
		self.forms = named(&snapshot.forms);
		self.state = named(&snapshot.state);
		self.calls.clone_from(&snapshot.calls);
		self.rpc.clone_from(&snapshot.rpc);
		self.commits.clear();
	}
}

/// One session as `GET /api/sessions/{id}` returns it: what the agent said
/// when it opened, and the mirror.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
	/// The session.
	#[serde(flatten)]
	pub info: SessionInfo,
	/// Its mirrored state.
	#[serde(flatten)]
	pub view: SessionView,
}

/// A change a front end sees on `GET /api/stream`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
	/// An agent opened a session.
	Opened {
		/// The new session.
		info: SessionInfo,
	},
	/// A session ended: its agent stopped polling.
	Closed {
		/// The session that ended.
		session: SessionId,
	},
	/// An agent event.
	Event {
		/// The session it belongs to.
		session: SessionId,
		/// The event.
		event: Event,
	},
	/// A command sent to an agent.
	Command {
		/// The session it belongs to.
		session: SessionId,
		/// The command.
		command: Command,
	},
}

fn apply_patch(tree: &mut Tree, patch: &TreePatch) {
	for operation in &patch.ops {
		match operation {
			TreeOp::Insert { node } => insert_node(tree, node),
			TreeOp::Update { node } => update_node(tree, node),
			TreeOp::Remove { id } => remove_subtree(tree, id),
		}
	}
}

fn insert_node(tree: &mut Tree, node: &Node) {
	if tree.nodes.iter().any(|item| item.id == node.id) {
		return;
	}

	match &node.parent {
		Some(parent) => {
			if let Some(slot) = tree.nodes.iter_mut().find(|item| &item.id == parent) {
				slot.children.push(node.id.clone());
			}
		}
		None => tree.root = node.id.clone(),
	}

	tree.nodes.push(node.clone());
}

fn update_node(tree: &mut Tree, node: &Node) {
	if let Some(slot) = tree.nodes.iter_mut().find(|item| item.id == node.id) {
		*slot = node.clone();
	}
}

fn remove_subtree(tree: &mut Tree, id: &NodeRef) {
	let remove = descendants(tree, id);
	tree.nodes.retain(|node| !remove.contains(&node.id));

	for node in &mut tree.nodes {
		node.children.retain(|child| !remove.contains(child));
	}
}

fn descendants(tree: &Tree, root: &NodeRef) -> Vec<NodeRef> {
	let mut found = vec![root.clone()];
	let mut index = 0;

	while index < found.len() {
		let current = found[index].clone();
		found.extend(
			tree.nodes
				.iter()
				.filter(|node| node.parent.as_ref() == Some(&current))
				.map(|node| node.id.clone()),
		);
		index += 1;
	}

	found
}

fn push<T>(items: &mut Vec<T>, item: T) {
	if items.len() == HISTORY {
		items.remove(0);
	}

	items.push(item);
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::CallStatus;
	use crate::Hello;
	use crate::MatchKey;
	use crate::Platform;

	fn node(id: u64, parent: Option<u64>) -> Node {
		Node {
			id: NodeRef::Native(id),
			key: MatchKey::new(format!("k{id}")),
			parent: parent.map(NodeRef::Native),
			kind: "View".into(),
			source: None,
			frame: None,
			classes: Vec::new(),
			text: None,
			children: Vec::new(),
			styles: Vec::new(),
		}
	}

	fn tree() -> Tree {
		let mut root = node(1, None);
		root.children = vec![NodeRef::Native(2)];
		let mut child = node(2, Some(1));
		child.children = vec![NodeRef::Native(3)];
		Tree {
			root: NodeRef::Native(1),
			nodes: vec![root, child, node(3, Some(2))],
		}
	}

	fn snapshot() -> Event {
		Event::Snapshot {
			snapshot: Snapshot {
				tree: tree(),
				route: Some("/".into()),
				queries: Vec::new(),
				mutations: Vec::new(),
				forms: Vec::new(),
				state: Vec::new(),
				calls: Vec::new(),
				rpc: Vec::new(),
			},
		}
	}

	#[test]
	fn a_snapshot_replaces_the_view_and_clears_commits() {
		let mut view = SessionView::default();
		view.apply(&Event::Commit {
			commit: Commit {
				seq: 1,
				at_ms: 1,
				mutations: 1,
				layout_us: None,
				apply_us: None,
			},
		});
		view.apply(&snapshot());
		assert_eq!(view.tree.as_ref().unwrap().nodes.len(), 3);
		assert_eq!(view.route.as_deref(), Some("/"));
		assert_eq!(view.commits, []);
	}

	#[test]
	fn patches_insert_update_and_remove_subtrees() {
		let mut view = SessionView::default();
		view.apply(&snapshot());
		let mut updated = node(3, Some(2));
		updated.text = Some("hello".into());
		view.apply(&Event::TreePatch {
			patch: TreePatch {
				ops: vec![
					TreeOp::Insert {
						node: node(4, Some(1)),
					},
					TreeOp::Update { node: updated },
				],
			},
		});
		assert_eq!(
			view.node(&NodeRef::Native(3)).unwrap().text.as_deref(),
			Some("hello")
		);
		assert_eq!(
			view.node(&NodeRef::Native(1)).unwrap().children,
			[NodeRef::Native(2), NodeRef::Native(4)]
		);
		view.apply(&Event::TreePatch {
			patch: TreePatch {
				ops: vec![TreeOp::Remove {
					id: NodeRef::Native(2),
				}],
			},
		});
		let tree = view.tree.as_ref().unwrap();
		assert_eq!(tree.nodes.len(), 2);
		assert_eq!(
			view.node(&NodeRef::Native(1)).unwrap().children,
			[NodeRef::Native(4)]
		);
	}

	#[test]
	fn calls_update_in_place_and_history_is_bounded() {
		let mut view = SessionView::default();
		let call = |id, status| {
			Event::Call {
				call: CallRecord {
					id,
					plugin: "p".into(),
					method: "m".into(),
					started_ms: 0,
					running_ms: None,
					finished_ms: None,
					taken_ms: None,
					completed_late_ms: None,
					status,
					request_bytes: None,
					response_bytes: None,
				},
			}
		};
		view.apply(&call(1, CallStatus::Pending));
		view.apply(&call(1, CallStatus::Ok));
		assert_eq!(view.calls.len(), 1);
		assert_eq!(view.calls[0].status, CallStatus::Ok);

		for id in 0..(HISTORY as u64 + 10) {
			view.apply(&Event::Commit {
				commit: Commit {
					seq: id,
					at_ms: id,
					mutations: 0,
					layout_us: None,
					apply_us: None,
				},
			});
		}
		assert_eq!(view.commits.len(), HISTORY);
		assert_eq!(view.commits[0].seq, 10);
	}

	#[test]
	fn changes_round_trip_with_their_kind() {
		let info = SessionInfo {
			id: SessionId::new("a"),
			hello: Hello::new("app", Platform::Web, None, []),
			connected_at_ms: 1,
			last_seen_ms: 2,
		};
		let changes = vec![
			Change::Opened { info },
			Change::Closed {
				session: SessionId::new("a"),
			},
			Change::Event {
				session: SessionId::new("a"),
				event: snapshot(),
			},
			Change::Command {
				session: SessionId::new("a"),
				command: Command::RequestSnapshot,
			},
		];

		for change in changes {
			let json = serde_json::to_value(&change).unwrap();
			assert!(json["kind"].is_string());
			assert_eq!(serde_json::from_value::<Change>(json).unwrap(), change);
		}
	}

	#[test]
	fn a_session_state_flattens_the_info_and_the_view() {
		let mut view = SessionView::default();
		view.apply(&snapshot());
		let state = SessionState {
			info: SessionInfo {
				id: SessionId::new("a"),
				hello: Hello::new("app", Platform::Ios, Some("iPhone".into()), []),
				connected_at_ms: 1,
				last_seen_ms: 2,
			},
			view,
		};
		let json = serde_json::to_value(&state).unwrap();
		assert_eq!(json["id"], "a");
		assert_eq!(json["route"], "/");
		assert_eq!(serde_json::from_value::<SessionState>(json).unwrap(), state);
	}
}
