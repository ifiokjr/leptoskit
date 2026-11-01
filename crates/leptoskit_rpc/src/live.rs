//! The live protocol: the frames a client and a server exchange over a
//! WebSocket, and the patches that keep a 2,000-message channel's update at
//! about one message's size.
//!
//! The frames are JSON, negotiated as `leptoskit.live.v1.json`. A client
//! opens `/live`, sends [`ClientFrame::Hello`], subscribes to queries with
//! [`ClientFrame::Sub`], and receives a [`ServerFrame::Transition`] whenever
//! a result changes. Every update comes with the version it reflects, so a
//! client can tell which writes it has seen.
//!
//! ```text
//! client → server                                   server → client
//! hello   { resume?: { version } }                  welcome     { connection, version, heartbeat_seconds }
//! auth    { token }                                 transition  { from, to, updates: [ { id, ok | patch | err } ] }
//! sub     { id, query, args }                       done        { request, ok | err, version }
//! unsub   { id }                                    auth_status { refused | expiring }
//! mutate  { request, name, args, key? }             reconnect   { after_ms }
//! ping                                              pong
//! ```
//!
//! [`patch`] turns two results into an [`Update::Patch`] when both are lists
//! of rows with an `id`, and into [`Update::Ok`] with the whole value
//! otherwise. Diffs keep steady-state frames small, and never lose data:
//! applying the same updates in order gives the same result as replacing
//! the value.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

/// The WebSocket subprotocol every live connection speaks.
pub const SUBPROTOCOL: &str = "leptoskit.live.v1.json";

/// How often the server sends a WebSocket ping, in seconds.
pub const HEARTBEAT_SECONDS: u64 = 30;

/// How many queries one connection may subscribe to.
pub const MAX_SUBSCRIPTIONS: usize = 256;

/// The largest result one subscription may hold, in bytes of JSON.
pub const MAX_RESULT_BYTES: usize = 512 * 1024;

/// The largest frame a connection may send, in bytes.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// What a client sends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
	/// The first frame: what the client already has, so the server can send
	/// only what changed.
	Hello {
		/// The last version the client applied, when it is resuming.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		resume: Option<Resume>,
	},
	/// Who the connection is, for a native app that can't send cookies on
	/// the upgrade request.
	Auth {
		/// The app's bearer token.
		token: String,
	},
	/// Subscribe to a query.
	Sub {
		/// The client's name for the subscription, echoed in every update.
		id: u32,
		/// The operation's name.
		query: String,
		/// Its arguments, as one JSON object.
		#[serde(default)]
		args: Value,
	},
	/// Stop a subscription.
	Unsub {
		/// The id the subscription was made with.
		id: u32,
	},
	/// Join a channel. `channel` is the instance key, the join arguments
	/// serialized; `name` is the channel operation.
	Join {
		/// The instance key: the join arguments, serialized.
		channel: String,
		/// The channel operation's name.
		name: String,
		/// The join arguments.
		#[serde(default)]
		args: Value,
		/// What this connection shows the others, or `null` for the default.
		#[serde(default)]
		state: Value,
	},
	/// Leave a channel.
	Leave {
		/// The instance key the connection joined with.
		channel: String,
	},
	/// Send a signal to everyone else in a channel.
	Signal {
		/// The instance key the connection joined with.
		channel: String,
		/// The message.
		message: Value,
	},
	/// Run a mutation. Its `request` echoes in [`ServerFrame::Done`].
	Mutate {
		/// The client's name for the call.
		request: u32,
		/// The operation's name.
		name: String,
		/// Its arguments, as one JSON object.
		#[serde(default)]
		args: Value,
		/// The idempotency key that makes a retry run once.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		key: Option<String>,
	},
	/// Check the connection is alive.
	Ping,
	/// A frame this version doesn't know: ignored, so a newer client can
	/// talk to an older server.
	#[serde(other)]
	Unknown,
}

/// What a resuming client already applied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resume {
	/// The last version the client applied.
	pub version: u64,
}

/// What a server sends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
	/// The connection is open.
	Welcome {
		/// An opaque name for the connection, for logs.
		connection: String,
		/// The version the server starts from.
		version: u64,
		/// How often the server pings, in seconds.
		heartbeat_seconds: u64,
		/// Whether the app was built against a spec the server can no longer
		/// serve.
		#[serde(default)]
		outdated_client: bool,
	},
	/// Results changed.
	Transition(Transition),
	/// A mutation or a one-shot call finished.
	Done {
		/// The client's name for the call.
		request: u32,
		/// The result, when it succeeded.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		ok: Option<Value>,
		/// Why it failed, when it did.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		err: Option<WireError>,
		/// The version the call's writes committed at.
		version: u64,
	},
	/// What happened to the connection's identity.
	AuthStatus {
		/// Whether the server refused the credentials, ending the
		/// connection's subscriptions.
		#[serde(default)]
		refused: bool,
		/// Whether the credentials expire soon, so the client renews them.
		#[serde(default)]
		expiring: bool,
	},
	/// The server is going away: reconnect after `after_ms`.
	Reconnect {
		/// How long to wait, with jitter already applied.
		after_ms: u64,
	},
	/// A signal on a channel.
	Signal {
		/// The channel's name.
		channel: String,
		/// Who sent it, when the server can say.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		from: Option<String>,
		/// The signal's message.
		message: Value,
	},
	/// Who is in a channel, after a join or a leave.
	Presence {
		/// The instance key.
		channel: String,
		/// Everyone now in the channel.
		members: Vec<PresenceMember>,
	},
	/// A channel operation was refused, such as when the caller may not
	/// join.
	ChannelError {
		/// The instance key.
		channel: String,
		/// Why it failed.
		err: WireError,
	},
	/// An item from a stream.
	Item {
		/// The stream's subscription id.
		stream: u32,
		/// The item.
		value: Value,
	},
	/// A stream ended.
	End {
		/// The stream's subscription id.
		stream: u32,
		/// Why it ended.
		reason: String,
	},
	/// The answer to [`ClientFrame::Ping`].
	Pong,
	/// A frame this version doesn't know.
	#[serde(other)]
	Unknown,
}

/// Every result that changed in one database state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transition {
	/// The version the client last had.
	pub from: u64,
	/// The version these updates reach. Applying them in order leaves the
	/// client at this version.
	pub to: u64,
	/// The results that changed.
	pub updates: Vec<SubscriptionUpdate>,
	/// Whether every update repeats its whole value, because the client
	/// missed intermediate versions.
	#[serde(default, skip_serializing_if = "is_false")]
	pub full: bool,
}

/// One subscription's new result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionUpdate {
	/// The subscription's client-chosen id.
	pub id: u32,
	/// What the result is now.
	#[serde(flatten)]
	pub update: Update,
}

/// How a result changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Update {
	/// The result, whole.
	Ok(Value),
	/// The changes to a list of rows with ids.
	Patch(Patch),
	/// The query failed; the subscription stays, and retries on the next
	/// change.
	Err(WireError),
}

/// The changes to a keyed list: rows to drop, and rows to put in order.
///
/// Apply the removals first, then each upsert at its index. An upsert at an
/// index past the end appends.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Patch {
	/// The ids of the rows to remove.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub remove: Vec<String>,
	/// The rows to insert or replace, in the order they belong.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub upsert: Vec<Upsert>,
}

/// One row of a [`Patch`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Upsert {
	/// Where the row goes, after the removals.
	pub at: usize,
	/// The row, as a JSON object with an `id`.
	pub value: Value,
}

/// Someone in a channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceMember {
	/// Who: the caller id the server signs them with.
	pub id: String,
	/// What the channel's presence says about them.
	#[serde(default)]
	pub state: Value,
}

/// A failure, as a frame reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireError {
	/// The HTTP status the same failure would carry.
	pub status: u16,
	/// The message the caller reads.
	pub message: String,
}

// The constructors compile with the server's live module, which the `live`
// feature builds (`live` implies `server`); in builds without it, such as
// an app server that depends on the auth crate alone, they are dead code.
#[cfg(feature = "live")]
impl WireError {
	/// The failure `error` carries.
	pub(crate) fn new(error: &crate::Error) -> Self {
		Self {
			status: error.status(),
			message: error.message().to_owned(),
		}
	}

	/// A failure that isn't an operation's, such as a frame that doesn't
	/// parse.
	pub(crate) fn invalid(message: impl Into<String>) -> Self {
		Self {
			status: 400,
			message: message.into(),
		}
	}
}

impl std::fmt::Display for WireError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.message)
	}
}

impl std::error::Error for WireError {}

#[allow(
	clippy::trivially_copy_pass_by_ref,
	reason = "serde calls `skip_serializing_if` with a reference"
)]
const fn is_false(value: &bool) -> bool {
	!*value
}

/// The update that turns `old` into `new`: a [`Patch`] when both are lists
/// of rows with string `id`s whose existing rows keep their order, and the
/// whole value otherwise.
#[must_use]
pub fn patch(old: &Value, new: &Value) -> Update {
	if old == new {
		return Update::Ok(new.clone());
	}

	match (rows(old), rows(new)) {
		(Some(before), Some(after)) => {
			match diff(&before, &after) {
				Some(patch) => Update::Patch(patch),
				None => Update::Ok(new.clone()),
			}
		}
		_ => Update::Ok(new.clone()),
	}
}

/// The `(id, row)` pairs of a JSON array of objects with string ids.
fn rows(value: &Value) -> Option<Vec<(String, Value)>> {
	let items = value.as_array()?;
	items
		.iter()
		.map(|item| {
			let id = item.get("id")?.as_str()?;
			Some((id.to_owned(), item.clone()))
		})
		.collect()
}

/// The patch from `before` to `after`, or `None` when a patch can't express
/// the change: a row moved relative to the rows that stayed, or two rows
/// share an id.
fn diff(before: &[(String, Value)], after: &[(String, Value)]) -> Option<Patch> {
	let before_ids: std::collections::HashSet<&str> =
		before.iter().map(|(id, _)| id.as_str()).collect();

	if before_ids.len() != before.len() {
		return None;
	}

	let after_ids: std::collections::HashSet<&str> =
		after.iter().map(|(id, _)| id.as_str()).collect();

	if after_ids.len() != after.len() {
		return None;
	}

	let remove: Vec<String> = before
		.iter()
		.filter(|(id, _)| !after_ids.contains(id.as_str()))
		.map(|(id, _)| id.clone())
		.collect();

	// Rows that stayed must keep their relative order: a patch can move a
	// row only by re-sending it, and a swap would need both.
	let stayed: Vec<&str> = before
		.iter()
		.map(|(id, _)| id.as_str())
		.filter(|id| after_ids.contains(id))
		.collect();
	let after_stayed: Vec<&str> = after
		.iter()
		.map(|(id, _)| id.as_str())
		.filter(|id| before_ids.contains(id))
		.collect();

	if stayed != after_stayed {
		return None;
	}

	let previous: std::collections::HashMap<&str, &Value> =
		before.iter().map(|(id, row)| (id.as_str(), row)).collect();
	let upsert: Vec<Upsert> = after
		.iter()
		.enumerate()
		.filter(|(_, (id, row))| previous.get(id.as_str()) != Some(&row))
		.map(|(at, (_, row))| {
			Upsert {
				at,
				value: row.clone(),
			}
		})
		.collect();

	Some(Patch { remove, upsert })
}

/// Applies an [`Update`] to a value. A patch on something that isn't a
/// keyed list leaves `value` alone: the server only sends patches for what
/// it described.
pub fn apply(value: &mut Value, update: &Update) {
	match update {
		Update::Ok(next) => *value = next.clone(),
		Update::Err(_) => {}
		Update::Patch(patch) => {
			let Some(items) = value.as_array_mut() else {
				return;
			};
			items.retain(|item| {
				item.get("id")
					.and_then(Value::as_str)
					.is_none_or(|id| !patch.remove.iter().any(|removed| removed == id))
			});
			for upsert in &patch.upsert {
				match items.get_mut(upsert.at) {
					Some(slot) => *slot = upsert.value.clone(),
					None => items.push(upsert.value.clone()),
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn frames_round_trip_through_json() {
		let frames = [
			ClientFrame::Hello {
				resume: Some(Resume { version: 7 }),
			},
			ClientFrame::Auth {
				token: "abc".into(),
			},
			ClientFrame::Sub {
				id: 1,
				query: "messages".into(),
				args: json!({ "channel": "general" }),
			},
			ClientFrame::Unsub { id: 1 },
			ClientFrame::Mutate {
				request: 2,
				name: "send_message".into(),
				args: json!({}),
				key: Some("k".into()),
			},
			ClientFrame::Ping,
		];

		for frame in frames {
			let json = serde_json::to_string(&frame).unwrap();
			let back: ClientFrame = serde_json::from_str(&json).unwrap();
			assert_eq!(back, frame);
		}

		let frames = [
			ServerFrame::Welcome {
				connection: "c1".into(),
				version: 0,
				heartbeat_seconds: 30,
				outdated_client: false,
			},
			ServerFrame::Transition(Transition {
				from: 0,
				to: 1,
				updates: vec![SubscriptionUpdate {
					id: 1,
					update: Update::Ok(json!([{ "id": "a" }])),
				}],
				full: false,
			}),
			ServerFrame::Done {
				request: 2,
				ok: Some(json!("ok")),
				err: None,
				version: 1,
			},
			ServerFrame::AuthStatus {
				refused: false,
				expiring: true,
			},
			ServerFrame::Reconnect { after_ms: 250 },
			ServerFrame::Pong,
		];

		for frame in frames {
			let json = serde_json::to_string(&frame).unwrap();
			let back: ServerFrame = serde_json::from_str(&json).unwrap();
			assert_eq!(back, frame);
		}
	}

	#[test]
	fn unknown_frames_are_ignored() {
		assert_eq!(
			serde_json::from_str::<ClientFrame>(r#"{"type":"future"}"#).unwrap(),
			ClientFrame::Unknown
		);
		assert_eq!(
			serde_json::from_str::<ServerFrame>(r#"{"type":"future"}"#).unwrap(),
			ServerFrame::Unknown
		);
	}

	#[test]
	fn lists_of_rows_change_by_patch() {
		let before = json!([
			{ "id": "a", "body": "one" },
			{ "id": "b", "body": "two" },
			{ "id": "c", "body": "three" }
		]);
		let after = json!([
			{ "id": "a", "body": "one" },
			{ "id": "b", "body": "TWO" },
			{ "id": "d", "body": "four" }
		]);

		let Update::Patch(patch) = patch(&before, &after) else {
			panic!("a keyed list patches");
		};
		assert_eq!(patch.remove, ["c"]);
		assert_eq!(
			patch.upsert,
			[
				Upsert {
					at: 1,
					value: json!({ "id": "b", "body": "TWO" })
				},
				Upsert {
					at: 2,
					value: json!({ "id": "d", "body": "four" })
				}
			]
		);

		let mut applied = before.clone();
		apply(&mut applied, &Update::Patch(patch));
		assert_eq!(applied, after);
	}

	#[test]
	fn a_reorder_or_a_shape_without_ids_sends_the_whole_value() {
		let before = json!([{ "id": "a" }, { "id": "b" }]);
		let ast = json!([{ "id": "b" }, { "id": "a" }]);
		assert_eq!(patch(&before, &ast), Update::Ok(ast.clone()));

		let object = json!({ "id": "a" });
		let other = json!({ "id": "b" });
		assert_eq!(patch(&object, &other), Update::Ok(other.clone()));

		let missing = json!([{ "body": "no id" }]);
		assert_eq!(patch(&before, &missing), Update::Ok(missing.clone()));
	}

	#[test]
	fn applying_an_ok_replaces_the_value_and_an_error_keeps_it() {
		let mut value = json!([{ "id": "a" }]);
		apply(&mut value, &Update::Ok(json!([])));
		assert_eq!(value, json!([]));

		let kept = value.clone();
		apply(
			&mut value,
			&Update::Err(WireError {
				status: 500,
				message: "no connection".to_owned(),
			}),
		);
		assert_eq!(value, kept);
	}
}
