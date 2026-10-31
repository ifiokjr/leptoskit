//! The wire protocol shared by leptoskit's `DevTools` agents, hub, interface,
//! and MCP tools.
//!
//! The types here are serde only, with no platform code. A web agent, a native
//! host, a native guest, the local hub, the hub page, and the MCP tools all
//! speak the same messages.
//!
//! Every session starts with a [`Hello`] that names the app, the platform, the
//! capabilities the session can feed, and the [`PROTOCOL_VERSION`] it speaks. A
//! hub refuses a session on another version with a [`VersionMismatch`] that
//! names both versions.

use std::collections::BTreeSet;

use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

/// The version of the messages in this crate.
///
/// The hub and every agent change together: a session on another version is
/// refused rather than translated.
pub const PROTOCOL_VERSION: u32 = 1;

/// The largest value an event or command carries before it is truncated.
pub const MAX_VALUE_BYTES: usize = 64 * 1024;

/// One live agent session.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub String);

impl SessionId {
	/// A session id from anything string-like.
	#[must_use]
	pub fn new(id: impl Into<String>) -> Self {
		Self(id.into())
	}

	/// The id as a string.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

/// A node's identity inside one session.
///
/// It is the surface `NodeId` on native and the path on the web, so it is only
/// meaningful to the session that sent it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NodeRef {
	/// A native surface's numeric node id.
	Native(u64),
	/// A web element's path.
	Web(String),
}

/// The cross-platform key that survives a tree rebuild.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MatchKey(pub String);

impl MatchKey {
	/// A match key from anything string-like.
	#[must_use]
	pub fn new(key: impl Into<String>) -> Self {
		Self(key.into())
	}

	/// The key as a string.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

/// Where a node was created in the app's source.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SourceLocation {
	/// The file, relative to the project root when possible.
	pub file: String,
	/// The one-based line.
	pub line: u32,
	/// The one-based column.
	pub column: u32,
}

/// The platform a session comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
	/// The web, through leptos.
	Web,
	/// A macOS app.
	Macos,
	/// An iOS app or simulator.
	Ios,
	/// An Android app or emulator.
	Android,
	/// A Windows app.
	Windows,
	/// A Linux app.
	Linux,
	/// A HarmonyOS app.
	Harmonyos,
	/// The terminal.
	Tui,
	/// A browser extension page or worker.
	Extension,
}

/// What a session can feed the hub.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
	/// The element tree and node details.
	Tree,
	/// Style explanations.
	Styles,
	/// Commits and their timings.
	Commits,
	/// Query cache state.
	Queries,
	/// Mutation state.
	Mutations,
	/// Form state.
	Forms,
	/// Named app state.
	State,
	/// Plugin bridge calls.
	Calls,
	/// Server rpc attempts.
	Rpc,
	/// Commit profiling.
	Profiling,
	/// Picking a node in the app.
	Picking,
	/// Navigating the app.
	Navigation,
	/// Editing a value through the app's codec.
	Editing,
}

/// The first message of a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
	/// The app's package name.
	pub app: String,
	/// The platform the session runs on.
	pub platform: Platform,
	/// A device or simulator name, when there is one.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub device: Option<String>,
	/// The [`PROTOCOL_VERSION`] the agent speaks.
	pub protocol: u32,
	/// What this session can feed and accept.
	pub capabilities: BTreeSet<Capability>,
}

impl Hello {
	/// A hello for the current protocol version.
	#[must_use]
	pub fn new(
		app: impl Into<String>,
		platform: Platform,
		device: Option<String>,
		capabilities: impl IntoIterator<Item = Capability>,
	) -> Self {
		Self {
			app: app.into(),
			platform,
			device,
			protocol: PROTOCOL_VERSION,
			capabilities: capabilities.into_iter().collect(),
		}
	}

	/// Refuse a session on another protocol version.
	///
	/// # Errors
	///
	/// When this hello's version is not [`PROTOCOL_VERSION`].
	pub fn check_protocol(&self) -> Result<(), VersionMismatch> {
		check_protocol(self.protocol)
	}

	/// Whether the session can feed or accept `capability`.
	#[must_use]
	pub fn has(&self, capability: Capability) -> bool {
		self.capabilities.contains(&capability)
	}
}

/// A session the hub knows about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
	/// The hub-assigned session id.
	pub id: SessionId,
	/// What the agent said when it opened the session.
	pub hello: Hello,
	/// When the session opened, in milliseconds since the Unix epoch.
	pub connected_at_ms: u64,
	/// When the hub last heard from the agent, in milliseconds since the Unix
	/// epoch.
	pub last_seen_ms: u64,
}

/// A refused protocol version, naming both sides.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("devtools protocol {client} is not supported; the hub speaks {hub}")]
pub struct VersionMismatch {
	/// The version the agent offered.
	pub client: u32,
	/// The version the hub speaks.
	pub hub: u32,
}

/// Refuse a client protocol version other than [`PROTOCOL_VERSION`].
///
/// # Errors
///
/// When `client` is not [`PROTOCOL_VERSION`].
pub fn check_protocol(client: u32) -> Result<(), VersionMismatch> {
	if client == PROTOCOL_VERSION {
		Ok(())
	} else {
		Err(VersionMismatch {
			client,
			hub: PROTOCOL_VERSION,
		})
	}
}

/// A rectangle in viewport space.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
	/// The left edge.
	pub x: f32,
	/// The top edge.
	pub y: f32,
	/// The width.
	pub width: f32,
	/// The height.
	pub height: f32,
}

/// One element in a session's tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
	/// The session-local id.
	pub id: NodeRef,
	/// The cross-platform key.
	pub key: MatchKey,
	/// The parent, when the node is not the root.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub parent: Option<NodeRef>,
	/// The component or widget kind, such as `View` or `Text`.
	pub kind: String,
	/// Where the node was created, when source locations are on.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub source: Option<SourceLocation>,
	/// The node's frame, when it has one.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub frame: Option<Frame>,
	/// The active Tailwind classes.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub classes: Vec<String>,
	/// The text a text node shows.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub text: Option<String>,
	/// The children, in order.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub children: Vec<NodeRef>,
}

/// A whole tree, sent with a snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tree {
	/// The root node.
	pub root: NodeRef,
	/// Every node, in depth-first order.
	pub nodes: Vec<Node>,
}

/// One change to a tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TreeOp {
	/// Add a node.
	Insert {
		/// The node to add.
		node: Node,
	},
	/// Replace a node's details.
	Update {
		/// The node's new details.
		node: Node,
	},
	/// Remove a node and its subtree.
	Remove {
		/// The node to remove.
		id: NodeRef,
	},
}

/// A batch of tree changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TreePatch {
	/// The changes, in the order to apply them.
	pub ops: Vec<TreeOp>,
}

/// One commit the surface applied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
	/// A session-local sequence number.
	pub seq: u64,
	/// When the commit started, in milliseconds since the session opened.
	pub at_ms: u64,
	/// How many mutations the commit applied.
	pub mutations: usize,
	/// The time spent in layout, while profiling is on.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub layout_us: Option<u64>,
	/// The time spent applying the backend batches, while profiling is on.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub apply_us: Option<u64>,
}

/// A value with the Rust type it came from.
///
/// Values larger than [`MAX_VALUE_BYTES`] are replaced with a truncation
/// marker, so a large query response cannot stall the hub or a browser tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Value {
	/// The Rust type's name, such as `Vec<Clip>`.
	pub type_name: String,
	/// The JSON encoding of the value.
	pub json: serde_json::Value,
	/// Whether the value was replaced with a truncation marker.
	pub truncated: bool,
}

impl Value {
	/// A value from JSON, truncated when it is over [`MAX_VALUE_BYTES`].
	#[must_use]
	pub fn from_json(type_name: impl Into<String>, json: serde_json::Value) -> Self {
		let bytes = json.to_string().len();

		if bytes > MAX_VALUE_BYTES {
			Self {
				type_name: type_name.into(),
				json: serde_json::json!({ "truncated": true, "bytes": bytes }),
				truncated: true,
			}
		} else {
			Self {
				type_name: type_name.into(),
				json,
				truncated: false,
			}
		}
	}

	/// A value from anything serializable.
	///
	/// # Errors
	///
	/// When the value cannot be serialized.
	pub fn from_serializable<T: Serialize>(
		type_name: impl Into<String>,
		value: &T,
	) -> serde_json::Result<Self> {
		Ok(Self::from_json(type_name, serde_json::to_value(value)?))
	}
}

/// One named value, for the data panels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NamedValue {
	/// The query key, form name, state name, or mutation key.
	pub name: String,
	/// The value.
	pub value: Value,
}

/// One plugin bridge call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallRecord {
	/// A session-local id.
	pub id: u64,
	/// The plugin's name.
	pub plugin: String,
	/// The method called.
	pub method: String,
	/// When the call started, in milliseconds since the session opened.
	pub started_ms: u64,
	/// When the call finished, when it has.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub finished_ms: Option<u64>,
	/// How the call ended.
	pub status: CallStatus,
}

/// How a plugin call ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
	/// Still running.
	Pending,
	/// Answered successfully.
	Ok,
	/// Answered with an error.
	Error,
	/// Cancelled before it answered.
	Cancelled,
}

/// One server rpc attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcAttempt {
	/// A session-local id.
	pub id: u64,
	/// The server function's name.
	pub function: String,
	/// The one-based attempt number.
	pub attempt: u32,
	/// When the attempt started, in milliseconds since the session opened.
	pub started_ms: u64,
	/// When the attempt finished, when it has.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub finished_ms: Option<u64>,
	/// The error class, when it failed.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
}

/// A snapshot of everything a session can show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
	/// The whole tree.
	pub tree: Tree,
	/// The current route, when the app has one.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub route: Option<String>,
	/// The current queries.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub queries: Vec<NamedValue>,
	/// The current mutations.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub mutations: Vec<NamedValue>,
	/// The current forms.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub forms: Vec<NamedValue>,
	/// The current named state.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub state: Vec<NamedValue>,
	/// The latest bridge calls.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub calls: Vec<CallRecord>,
	/// The latest rpc attempts.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub rpc: Vec<RpcAttempt>,
}

/// An event an agent sends to the hub.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
	/// Everything the session can show.
	Snapshot {
		/// The snapshot.
		snapshot: Snapshot,
	},
	/// A batch of tree changes.
	TreePatch {
		/// The changes.
		patch: TreePatch,
	},
	/// A commit the surface applied.
	Commit {
		/// The commit.
		commit: Commit,
	},
	/// The app navigated.
	RouteChanged {
		/// The new route.
		route: String,
	},
	/// A query changed.
	QueryChanged {
		/// The query key.
		key: String,
		/// The query's new value.
		value: Value,
	},
	/// A mutation changed.
	MutationChanged {
		/// The mutation key.
		key: String,
		/// The mutation's new value.
		value: Value,
	},
	/// A form changed.
	FormChanged {
		/// The form's name.
		name: String,
		/// The form's values.
		value: Value,
	},
	/// Named state changed.
	StateChanged {
		/// The state's name.
		name: String,
		/// The state's new value.
		value: Value,
	},
	/// A plugin call changed.
	Call {
		/// The call.
		call: CallRecord,
	},
	/// A server rpc attempt changed.
	RpcAttempt {
		/// The attempt.
		attempt: RpcAttempt,
	},
	/// The app picked a node.
	Picked {
		/// The picked node.
		node: NodeRef,
		/// The picked node's match key.
		key: MatchKey,
	},
}

/// A command the hub sends to an agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
	/// Select a node in the app.
	Select {
		/// The node to select.
		node: NodeRef,
		/// Its match key.
		key: MatchKey,
	},
	/// Highlight every node with a match key.
	Highlight {
		/// The match key.
		key: MatchKey,
	},
	/// Turn picking on or off.
	Pick {
		/// Whether picking is on.
		on: bool,
	},
	/// Replace a node's classes.
	SetClasses {
		/// The node to change.
		node: NodeRef,
		/// The classes to set.
		classes: Vec<String>,
	},
	/// Navigate the app.
	Navigate {
		/// The href to open.
		href: String,
	},
	/// Invalidate a query.
	InvalidateQuery {
		/// The query key.
		key: String,
	},
	/// Refetch a query.
	RefetchQuery {
		/// The query key.
		key: String,
	},
	/// Replace a query's data.
	SetQueryData {
		/// The query key.
		key: String,
		/// The new value.
		value: Value,
	},
	/// Set named state.
	SetState {
		/// The state's name.
		name: String,
		/// The new value.
		value: Value,
	},
	/// Start recording commit timings.
	StartProfiling,
	/// Stop recording commit timings.
	StopProfiling,
	/// Send a fresh snapshot.
	RequestSnapshot,
}

#[cfg(test)]
mod tests {
	use super::*;

	fn node(id: u64, key: &str) -> Node {
		Node {
			id: NodeRef::Native(id),
			key: MatchKey::new(key),
			parent: None,
			kind: "View".into(),
			source: Some(SourceLocation {
				file: "src/lib.rs".into(),
				line: 12,
				column: 3,
			}),
			frame: Some(Frame {
				x: 0.0,
				y: 0.0,
				width: 120.0,
				height: 44.0,
			}),
			classes: vec!["gap-2".into()],
			text: None,
			children: Vec::new(),
		}
	}

	fn snapshot() -> Snapshot {
		let tree = Tree {
			root: NodeRef::Native(1),
			nodes: vec![node(1, "root"), node(2, "row")],
		};
		Snapshot {
			tree,
			route: Some("/clips".into()),
			queries: vec![NamedValue {
				name: "clips".into(),
				value: Value::from_json("Vec<Clip>", serde_json::json!([{"id": 1}])),
			}],
			mutations: Vec::new(),
			forms: Vec::new(),
			state: Vec::new(),
			calls: vec![CallRecord {
				id: 1,
				plugin: "clipboard".into(),
				method: "read".into(),
				started_ms: 10,
				finished_ms: Some(20),
				status: CallStatus::Ok,
			}],
			rpc: vec![RpcAttempt {
				id: 1,
				function: "clips".into(),
				attempt: 1,
				started_ms: 10,
				finished_ms: Some(30),
				error: None,
			}],
		}
	}

	#[test]
	fn every_event_round_trips() {
		let events = vec![
			Event::Snapshot {
				snapshot: snapshot(),
			},
			Event::TreePatch {
				patch: TreePatch {
					ops: vec![TreeOp::Insert {
						node: node(3, "new"),
					}],
				},
			},
			Event::Commit {
				commit: Commit {
					seq: 1,
					at_ms: 40,
					mutations: 3,
					layout_us: Some(12),
					apply_us: Some(8),
				},
			},
			Event::RouteChanged {
				route: "/reading".into(),
			},
			Event::QueryChanged {
				key: "clips".into(),
				value: Value::from_json("Vec<Clip>", serde_json::json!([])),
			},
			Event::MutationChanged {
				key: "save".into(),
				value: Value::from_json("bool", serde_json::json!(true)),
			},
			Event::FormChanged {
				name: "signup".into(),
				value: Value::from_json("Form", serde_json::json!({"email": "a@b.c"})),
			},
			Event::StateChanged {
				name: "cart.count".into(),
				value: Value::from_json("usize", serde_json::json!(2)),
			},
			Event::Call {
				call: CallRecord {
					id: 2,
					plugin: "notifications".into(),
					method: "show".into(),
					started_ms: 50,
					finished_ms: None,
					status: CallStatus::Pending,
				},
			},
			Event::RpcAttempt {
				attempt: RpcAttempt {
					id: 2,
					function: "clips".into(),
					attempt: 1,
					started_ms: 50,
					finished_ms: None,
					error: Some("offline".into()),
				},
			},
			Event::Picked {
				node: NodeRef::Native(2),
				key: MatchKey::new("row"),
			},
		];

		for event in events {
			let json = serde_json::to_value(&event).unwrap();
			assert_eq!(serde_json::from_value::<Event>(json).unwrap(), event);
		}
	}

	#[test]
	fn every_command_round_trips() {
		let commands = vec![
			Command::Select {
				node: NodeRef::Native(1),
				key: MatchKey::new("root"),
			},
			Command::Highlight {
				key: MatchKey::new("root"),
			},
			Command::Pick { on: true },
			Command::SetClasses {
				node: NodeRef::Web("body/div[1]".into()),
				classes: vec!["p-4".into()],
			},
			Command::Navigate {
				href: "/clips".into(),
			},
			Command::InvalidateQuery {
				key: "clips".into(),
			},
			Command::RefetchQuery {
				key: "clips".into(),
			},
			Command::SetQueryData {
				key: "clips".into(),
				value: Value::from_json("Vec<Clip>", serde_json::json!([])),
			},
			Command::SetState {
				name: "cart.count".into(),
				value: Value::from_json("usize", serde_json::json!(3)),
			},
			Command::StartProfiling,
			Command::StopProfiling,
			Command::RequestSnapshot,
		];

		for command in commands {
			let json = serde_json::to_value(&command).unwrap();
			assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
		}
	}

	#[test]
	fn a_hello_round_trips_and_checks_its_version() {
		let hello = Hello::new(
			"commonplace",
			Platform::Ios,
			Some("iPhone 17".into()),
			[Capability::Tree, Capability::Queries, Capability::Picking],
		);
		let json = serde_json::to_value(&hello).unwrap();
		let decoded: Hello = serde_json::from_value(json).unwrap();
		assert_eq!(decoded, hello);
		assert!(decoded.has(Capability::Queries));
		assert!(!decoded.has(Capability::Calls));
		assert_eq!(decoded.check_protocol(), Ok(()));
	}

	#[test]
	fn a_version_mismatch_names_both_versions() {
		let error = check_protocol(PROTOCOL_VERSION + 1).unwrap_err();
		assert_eq!(error.client, PROTOCOL_VERSION + 1);
		assert_eq!(error.hub, PROTOCOL_VERSION);
		let message = error.to_string();
		assert!(message.contains(&(PROTOCOL_VERSION + 1).to_string()));
		assert!(message.contains(&PROTOCOL_VERSION.to_string()));

		let hello = Hello {
			app: "app".into(),
			platform: Platform::Web,
			device: None,
			protocol: 99,
			capabilities: BTreeSet::new(),
		};
		assert_eq!(hello.check_protocol().unwrap_err().client, 99);
	}

	#[test]
	fn large_values_are_truncated_with_a_marker() {
		let big = "x".repeat(MAX_VALUE_BYTES + 1);
		let value = Value::from_json("String", serde_json::json!(big));
		assert!(value.truncated);
		assert_eq!(value.json["truncated"], true);
		assert!(value.json["bytes"].as_u64().unwrap() > MAX_VALUE_BYTES as u64);
	}

	#[test]
	fn small_values_are_kept_as_they_are() {
		let value = Value::from_json("Vec<Clip>", serde_json::json!([{"id": 1}]));
		assert!(!value.truncated);
		assert_eq!(value.json, serde_json::json!([{"id": 1}]));
	}

	#[test]
	fn tree_patches_carry_each_operation() {
		let patch = TreePatch {
			ops: vec![
				TreeOp::Insert {
					node: node(3, "new"),
				},
				TreeOp::Update {
					node: node(3, "new"),
				},
				TreeOp::Remove {
					id: NodeRef::Native(3),
				},
			],
		};
		let json = serde_json::to_value(&patch).unwrap();
		assert_eq!(serde_json::from_value::<TreePatch>(json).unwrap(), patch);
	}

	#[test]
	fn node_refs_are_numbers_or_paths() {
		assert_eq!(
			serde_json::to_value(NodeRef::Native(7)).unwrap(),
			serde_json::json!(7)
		);
		assert_eq!(
			serde_json::to_value(NodeRef::Web("body/div".into())).unwrap(),
			serde_json::json!("body/div")
		);
	}
}
