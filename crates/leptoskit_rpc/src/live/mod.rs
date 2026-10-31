//! The live side of the RPC: subscriptions over queries, and the fan-out of
//! committed changes to every connection watching a table.
//!
//! A client subscribes to a query by name and arguments. The hub runs it as
//! that client, records the read set the query's own database connection
//! produced, and returns the result at the newest committed version. When a
//! write commits, [`LiveHub::poll`] reads the change log since the last
//! version, asks each subscription's read set whether the change could have
//! affected it, re-runs the ones it could, and sends the new value or a keyed
//! patch.
//!
//! Nothing here sends a change row to a client. A change only decides which
//! queries to re-run; the re-run, under the subscriber's own policies, decides
//! what they see.

use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_db::Db;
use leptoskit_db::ReadEntry;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::sync::mpsc;

use crate::Error;
use crate::Identity;
use crate::Kind;
use crate::Result;
use crate::server::Invocation;
use crate::server::Registration;
use crate::server::registration;

/// The WebSocket subprotocol this protocol speaks.
pub const SUBPROTOCOL: &str = "leptoskit.live.v1.json";

/// What a client sends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
	/// The first frame: the client says what it already has.
	Hello {
		/// The version and queries to resume, when reconnecting.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		resume: Option<Resume>,
	},
	/// A bearer token, for a client that can't set the upgrade's headers.
	Auth {
		/// The token.
		token: String,
	},
	/// Subscribe to a query, under the client's own policies.
	Sub {
		/// The connection-local id the client uses for this subscription.
		query: u64,
		/// The operation's name.
		name: String,
		/// Its arguments, as one JSON object.
		#[serde(default)]
		args: Value,
	},
	/// Stop a subscription.
	Unsub {
		/// The subscription's id.
		query: u64,
	},
	/// Call a mutation.
	Mutate {
		/// A client-generated request id: a retry with the same id runs once.
		request: String,
		/// The mutation's name.
		name: String,
		/// Its arguments.
		#[serde(default)]
		args: Value,
	},
	/// Keep the connection alive.
	Ping,
}

/// What a client resumes after reconnecting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Resume {
	/// The last version the client applied.
	pub version: i64,
	/// The queries it had open.
	#[serde(default)]
	pub queries: Vec<u64>,
}

/// What the server sends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
	/// The connection is open.
	Welcome {
		/// The connection's id.
		connection: u64,
		/// The newest committed version at the time.
		version: i64,
		/// How often the server sends a ping.
		heartbeat_seconds: u64,
	},
	/// Query results changed. Every update in one transition reflects the same
	/// re-run, and the client applies them in one batch.
	Transition {
		/// The version the connection had.
		from: i64,
		/// The version the updates are at.
		to: i64,
		/// One entry per changed subscription.
		updates: Vec<Update>,
	},
	/// A mutation finished.
	Done {
		/// The request id the client sent.
		request: String,
		/// Its result, or its failure.
		result: std::result::Result<Value, ErrorBody>,
		/// The commit's version. A client resolves the mutation once its own
		/// queries have reached this version or later.
		version: i64,
	},
	/// The connection's identity changed, or is about to expire.
	AuthStatus {
		/// What happened.
		status: AuthStatus,
	},
	/// The server is going away, and when to reconnect.
	Reconnect {
		/// A jittered delay in milliseconds.
		after_ms: u64,
	},
	/// A ping's answer.
	Pong,
}

/// What happened to a connection's identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
	/// The token is valid and the connection now acts as it.
	Authenticated,
	/// The token expires soon; renew it.
	Expiring,
	/// The identity was refused; subscriptions are gone.
	Refused,
}

/// One subscription's new state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Update {
	/// The subscription's id.
	pub query: u64,
	/// Its new value, a patch, or its failure.
	#[serde(flatten)]
	pub outcome: UpdateOutcome,
}

/// What a re-run produced for one subscription.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdateOutcome {
	/// The whole new value.
	Value {
		/// The value.
		value: Value,
	},
	/// A keyed diff of a list of rows with `id`s.
	Patch {
		/// The changes to apply.
		patch: Patch,
	},
	/// The query failed under the subscriber's policies.
	Error {
		/// Why.
		error: ErrorBody,
	},
}

/// A keyed diff of a list of rows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Patch {
	/// The ids that left the list.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub remove: Vec<Value>,
	/// The rows that entered it or changed, with their positions.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub upsert: Vec<Upsert>,
}

/// One row in a [`Patch`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Upsert {
	/// The row's position in the new list.
	pub at: usize,
	/// The row.
	pub value: Value,
}

/// A failure a client can classify.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
	/// The HTTP-like status: 400, 401, 403, 404, 409, or 500.
	pub code: u16,
	/// What went wrong.
	pub message: String,
}

impl From<Error> for ErrorBody {
	fn from(error: Error) -> Self {
		Self {
			code: error.status(),
			message: error.message().to_owned(),
		}
	}
}

/// A connection to the hub: its id and the channel its frames go out on.
#[derive(Clone)]
pub struct LiveConnection {
	/// The connection's id.
	pub id: u64,
	sender: mpsc::UnboundedSender<ServerFrame>,
}

impl LiveConnection {
	/// Sends one frame to the client.
	pub fn send(&self, frame: ServerFrame) -> bool {
		self.sender.send(frame).is_ok()
	}
}

impl std::fmt::Debug for LiveConnection {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("LiveConnection")
			.field("id", &self.id)
			.finish_non_exhaustive()
	}
}

/// The live hub: every subscription on this instance, and the change feed
/// that drives them.
pub struct LiveHub {
	db: Db,
	last_version: AtomicI64,
	next_connection: AtomicU64,
	state: Mutex<State>,
	polling: Mutex<()>,
}

struct State {
	connections: HashMap<u64, Connection>,
	subscriptions: HashMap<(u64, u64), Subscription>,
}

struct Connection {
	identity: Identity,
	sender: mpsc::UnboundedSender<ServerFrame>,
}

struct Subscription {
	query: u64,
	name: String,
	args: Value,
	identity: Identity,
	reads: Vec<ReadEntry>,
	value: Value,
}

impl LiveHub {
	/// A hub over `db`. The hub reads the change log and runs queries on the
	/// database; it never writes through it.
	pub fn new(db: Db) -> Self {
		Self {
			db,
			last_version: AtomicI64::new(0),
			next_connection: AtomicU64::new(1),
			state: Mutex::new(State {
				connections: HashMap::new(),
				subscriptions: HashMap::new(),
			}),
			polling: Mutex::new(()),
		}
	}

	/// The version the hub has read up to.
	pub fn version(&self) -> i64 {
		self.last_version.load(Ordering::Acquire)
	}

	/// Opens a connection as `identity`, and returns its frame receiver.
	pub async fn connect(
		&self,
		identity: Identity,
	) -> (LiveConnection, mpsc::UnboundedReceiver<ServerFrame>) {
		let id = self.next_connection.fetch_add(1, Ordering::Relaxed);
		let (sender, receiver) = mpsc::unbounded_channel();
		self.state.lock().await.connections.insert(
			id,
			Connection {
				identity: identity.clone(),
				sender: sender.clone(),
			},
		);
		let version = leptoskit_db::latest_version(&self.db).await.unwrap_or(0);
		let _ = sender.send(ServerFrame::Welcome {
			connection: id,
			version,
			heartbeat_seconds: 30,
		});
		(LiveConnection { id, sender }, receiver)
	}

	/// Subscribes `connection` to `name` with `args`, and returns the first
	/// result at the newest committed version.
	pub async fn subscribe(
		&self,
		connection: &LiveConnection,
		query: u64,
		name: &str,
		args: Value,
	) -> Result<Value> {
		let registration = query_registration(name)?;
		let identity = self.identity_of(connection.id).await?;
		let version = leptoskit_db::latest_version(&self.db).await?;
		let (value, reads) =
			run_query(registration, &self.db, identity.clone(), args.clone()).await?;

		self.state.lock().await.subscriptions.insert(
			(connection.id, query),
			Subscription {
				query,
				name: name.to_owned(),
				args,
				identity,
				reads,
				value: value.clone(),
			},
		);
		let _ = connection.sender.send(ServerFrame::Transition {
			from: 0,
			to: version,
			updates: vec![Update {
				query,
				outcome: UpdateOutcome::Value {
					value: value.clone(),
				},
			}],
		});
		Ok(value)
	}

	/// Stops one subscription.
	pub async fn unsubscribe(&self, connection: &LiveConnection, query: u64) {
		self.state
			.lock()
			.await
			.subscriptions
			.remove(&(connection.id, query));
	}

	/// Runs a mutation as the connection's identity, and sends its result.
	pub async fn mutate(
		&self,
		connection: &LiveConnection,
		request: &str,
		name: &str,
		args: Value,
	) -> Result<Value> {
		let registration = mutation_registration(name)?;
		let identity = self.identity_of(connection.id).await?;
		let invocation = Invocation {
			db: self.db.with_context(identity.context.clone()),
			identity,
			request_id: Some(request.to_owned()),
			arguments: args,
		};
		let result = (registration.handler)(invocation).await;
		let version = leptoskit_db::latest_version(&self.db).await?;
		let _ = connection.sender.send(ServerFrame::Done {
			request: request.to_owned(),
			result: match &result {
				Ok(value) => Ok(value.clone()),
				Err(error) => Err(ErrorBody::from(error.clone())),
			},
			version,
		});
		result
	}

	/// Reads the change log since the last poll, re-runs the subscriptions the
	/// changes could have affected, and sends every connection its updates.
	///
	/// Returns how many connections received a transition.
	pub async fn poll(&self) -> Result<usize> {
		let _polling = self.polling.lock().await;
		let from = self.last_version.load(Ordering::Acquire);
		let latest = leptoskit_db::latest_version(&self.db).await?;

		if latest <= from {
			return Ok(0);
		}

		let changes = self.db.changes().since(from).all().await?;
		let dirty: Vec<(u64, u64)> = {
			let state = self.state.lock().await;
			state
				.subscriptions
				.iter()
				.filter(|(_, subscription)| {
					changes.iter().any(|change| {
						subscription
							.reads
							.iter()
							.any(|read| read.affected_by(change).may_match())
					})
				})
				.map(|((connection, query), _)| (*connection, *query))
				.collect()
		};

		let mut updates: HashMap<u64, Vec<Update>> = HashMap::new();

		for key in dirty {
			let subscription = {
				let state = self.state.lock().await;
				let Some(subscription) = state.subscriptions.get(&key) else {
					continue;
				};
				(
					subscription.query,
					subscription.name.clone(),
					subscription.args.clone(),
					subscription.identity.clone(),
					subscription.value.clone(),
				)
			};
			let (query, name, args, identity, old) = subscription;
			let Ok(registration) = query_registration(&name) else {
				continue;
			};
			let outcome = match run_query(registration, &self.db, identity, args).await {
				Ok((value, reads)) => {
					if let Some(subscription) = self.state.lock().await.subscriptions.get_mut(&key)
					{
						subscription.reads = reads;
						subscription.value = value.clone();
					}
					update_for(&old, &value)
				}
				Err(error) => {
					Some(UpdateOutcome::Error {
						error: error.into(),
					})
				}
			};

			if let Some(outcome) = outcome {
				updates
					.entry(key.0)
					.or_default()
					.push(Update { query, outcome });
			}
		}

		self.last_version.store(latest, Ordering::Release);
		let state = self.state.lock().await;
		let mut notified = 0;

		for (connection, updates) in updates {
			if let Some(connection) = state.connections.get(&connection)
				&& connection
					.sender
					.send(ServerFrame::Transition {
						from,
						to: latest,
						updates,
					})
					.is_ok()
			{
				notified += 1;
			}
		}

		Ok(notified)
	}

	async fn identity_of(&self, connection: u64) -> Result<Identity> {
		self.state
			.lock()
			.await
			.connections
			.get(&connection)
			.map(|connection| connection.identity.clone())
			.ok_or_else(|| Error::invalid("the connection is closed"))
	}
}

fn query_registration(name: &str) -> Result<&'static Registration> {
	let registration = registration(name)
		.ok_or_else(|| Error::not_found(format!("no operation is named `{name}`")))?;

	if registration.kind != Kind::Query || registration.query.is_none() {
		return Err(Error::invalid(format!("`{name}` isn't a query")));
	}

	Ok(registration)
}

fn mutation_registration(name: &str) -> Result<&'static Registration> {
	let registration = registration(name)
		.ok_or_else(|| Error::not_found(format!("no operation is named `{name}`")))?;

	if registration.kind != Kind::Mutation {
		return Err(Error::invalid(format!("`{name}` isn't a mutation")));
	}

	Ok(registration)
}

async fn run_query(
	registration: &'static Registration,
	db: &Db,
	identity: Identity,
	args: Value,
) -> Result<(Value, Vec<ReadEntry>)> {
	let Some(query) = registration.query else {
		return Err(Error::invalid(format!(
			"`{}` has no live handler",
			registration.name
		)));
	};
	query(Invocation {
		db: db.with_context(identity.context.clone()),
		identity,
		request_id: None,
		arguments: args,
	})
	.await
}

/// What to send for a re-run, or `None` when the value didn't change.
fn update_for(old: &Value, new: &Value) -> Option<UpdateOutcome> {
	if old == new {
		return None;
	}

	if let (Some(old), Some(new)) = (old.as_array(), new.as_array())
		&& let Some(patch) = keyed_patch(old, new)
	{
		return Some(UpdateOutcome::Patch { patch });
	}

	Some(UpdateOutcome::Value { value: new.clone() })
}

/// A patch for a list of objects with `id`s, or `None` when some row has no
/// id and the list has to be sent whole.
fn keyed_patch(old: &[Value], new: &[Value]) -> Option<Patch> {
	let id = |value: &Value| value.get("id").cloned();
	let old_ids: Vec<Value> = old.iter().map(&id).collect::<Option<_>>()?;
	let new_ids: Vec<Value> = new.iter().map(&id).collect::<Option<_>>()?;
	let mut patch = Patch::default();

	for (index, (new_id, value)) in new_ids.iter().zip(new).enumerate() {
		match old
			.iter()
			.position(|value| id(value).as_ref() == Some(new_id))
		{
			Some(position) if old[position] == *value => {}
			_ => {
				patch.upsert.push(Upsert {
					at: index,
					value: value.clone(),
				});
			}
		}
	}

	for (old_id, value) in old_ids.iter().zip(old) {
		if !new_ids.contains(old_id) {
			patch
				.remove
				.push(value.get("id").cloned().unwrap_or(Value::Null));
		}
	}

	if patch.upsert.is_empty() && patch.remove.is_empty() {
		None
	} else {
		Some(patch)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_patch_removes_and_upserts_by_id() {
		let old = serde_json::json!([
			{"id": "a", "n": 1},
			{"id": "b", "n": 2},
		]);
		let new = serde_json::json!([
			{"id": "b", "n": 3},
			{"id": "c", "n": 4},
		]);
		let patch = keyed_patch(old.as_array().unwrap(), new.as_array().unwrap()).unwrap();
		assert_eq!(patch.remove, vec![serde_json::json!("a")]);
		assert_eq!(patch.upsert.len(), 2);
		assert_eq!(patch.upsert[0].at, 0);
		assert_eq!(patch.upsert[1].at, 1);
	}

	#[test]
	fn an_unchanged_list_has_no_patch() {
		let list = serde_json::json!([{"id": "a", "n": 1}]);
		assert_eq!(
			update_for(&list, &list),
			None,
			"a re-run that changed nothing sends nothing"
		);
	}
}
