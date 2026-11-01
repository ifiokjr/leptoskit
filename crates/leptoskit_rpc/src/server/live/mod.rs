//! Serving live queries: the `/live` WebSocket and its server-sent events
//! fallback, the subscriptions each connection holds, and the transitions
//! that tell it what changed.
//!
//! A connection asks for queries by name, with the same arguments the HTTP
//! route takes. The server runs each one as the caller, keeps its result,
//! and when a write commits re-runs it. Only results that changed are sent,
//! and a list of rows with ids travels as a [`Patch`](crate::live::Patch),
//! so a 2,000-message channel costs about one message per update.
//!
//! ```ignore
//! let live = rpc::server::Live::new(db.clone())
//! 	.identify(auth.clone())
//! 	.origin("https://myapp.com");
//! let app = Router::new()
//! 	.merge(live.router())
//! 	.merge(rpc.router());
//! ```
//!
//! # How it fits together
//!
//! - **One hub per server.** A single reader follows the database's change log
//!   for every connection. It matches each commit against the read set of every
//!   subscription, through an index by table, and re-runs only the queries a
//!   write could change.
//! - **Shared re-runs.** Subscriptions to the same query, with the same
//!   arguments, by the same caller, share one re-run and one diff per commit,
//!   however many connections hold them.
//! - **A session per connection**, whatever carries it: it reads client frames
//!   from a channel and writes server frames to another, and handles
//!   heartbeats, drain, limits and identity itself. The WebSocket and the
//!   server-sent events routes are thin adapters around it.
//!
//! # What it guarantees
//!
//! Every re-run reads the database as the connection's [`Identity`], through
//! the same policies as a direct call, so a subscriber only learns that a
//! query's result changed; nothing about a row they can't see is sent.
//!
//! Every transition names the change log version its results reflect, and
//! a mutation's `done` names the version it committed at, so a client knows
//! when its own writes are on screen.
//!
//! Re-runs read the database as it is when they run, not at one snapshot
//! per batch: a transition's results reflect at least its version, and a
//! later commit they already include re-runs them again, harmlessly.
//!
//! # Several servers
//!
//! Every server's hub reads each commit from the database, which suits a
//! few dozen servers. With a [`Relay`](crate::server::Relay), one server
//! publishes the change log to NATS JetStream and the others read it from
//! there, and channel signals and presence cross between servers too. When
//! the database has read replicas, re-runs read from one that has replayed
//! past the commits that caused them, and from the primary otherwise.

#[cfg(feature = "relay")]
mod cluster;
mod hub;
pub(crate) mod memory;
mod session;
mod socket;
mod sse;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use axum::Router;
use axum::http::HeaderMap;
use axum::http::header::ORIGIN;
use axum::routing::get;
use axum::routing::post;
use leptoskit_db::Db;
use leptoskit_db::ReadEntry;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::sync::mpsc;

use super::Identify;
use super::Identity;
use super::Invocation;
use super::ReadSink;
use super::registration;
use crate::Kind;
use crate::live::EVENTS_PATH;
use crate::live::FRAMES_PATH;
use crate::live::MAX_RESULT_BYTES;
use crate::live::PresenceMember;
use crate::live::WireError;

/// How many frames may queue for a slow connection before intermediate
/// transitions are dropped in favour of a full one.
const OUTGOING: usize = 32;

/// How many client frames may wait for a busy session before the transport
/// stops reading.
const INCOMING: usize = 64;

/// A live server: the database, how to tell who is calling, and the
/// connections listening for changes.
#[derive(Clone)]
pub struct Live {
	inner: Arc<Inner>,
}

struct Inner {
	db: Db,
	identify: Arc<dyn Identify>,
	origin: Option<String>,
	shared: Arc<Shared>,
}

/// What every copy of a [`Live`] shares, whatever its settings.
struct Shared {
	drain: broadcast::Sender<()>,
	stats: Stats,
	hub: hub::Hub,
	connections: Mutex<HashMap<u32, mpsc::Sender<Outbound>>>,
	rooms: Mutex<HashMap<String, Room>>,
	events: Mutex<HashMap<String, sse::Handle>>,
	/// The relay commits and channels go through, when there is one.
	#[cfg(feature = "relay")]
	relay: std::sync::OnceLock<crate::server::Relay>,
	/// The other servers' channel members, once the relay is joined.
	#[cfg(feature = "relay")]
	cluster: tokio::sync::OnceCell<Option<Arc<cluster::Cluster>>>,
}

/// Who is in one channel instance.
#[derive(Default)]
struct Room {
	members: BTreeMap<u32, PresenceMember>,
}

#[derive(Default)]
struct Stats {
	connections: AtomicU64,
	subscriptions: AtomicU64,
	transitions: AtomicU64,
	resumes: AtomicU64,
}

/// What a live server has done since it started, for metrics and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveStats {
	/// Connections opened, over either transport, including closed ones.
	pub connections: u64,
	/// Subscriptions made, including ended ones.
	pub subscriptions: u64,
	/// Queries re-run after a change. Subscriptions that share a query,
	/// its arguments and its caller re-run once between them.
	pub reruns: u64,
	/// Transitions sent, including ones dropped for a slow connection.
	pub transitions: u64,
	/// Commits read from the change log: once each, however many
	/// connections are open.
	pub commits: u64,
	/// Connections that resumed from a version the change log still
	/// reached, sending only what changed.
	pub resumes: u64,
	/// Re-runs that read from a read replica rather than the primary,
	/// counted in [`reruns`](Self::reruns) too.
	pub replica_reruns: u64,
	/// With a relay: log entries read from the database because the relay
	/// couldn't bring them, after a gap in its stream or while no server
	/// published.
	pub relay_fills: u64,
}

/// What a session writes for its transport to send.
#[derive(Clone, Debug)]
pub(crate) enum Outbound {
	/// A server frame as JSON, with the version a transition brings the
	/// client to, which server-sent events carry as the event's `id`.
	Frame { text: String, version: Option<u64> },
	/// A keep-alive: a WebSocket ping, or an event stream comment.
	Heartbeat,
}

/// What a transport hands its session.
#[derive(Debug)]
pub(crate) enum Incoming {
	/// A client frame.
	Frame(crate::live::ClientFrame),
	/// Something that isn't one, answered with a `done` for request 0.
	Invalid(WireError),
}

impl Live {
	/// Live queries on `db`. Every caller is anonymous until
	/// [`identify`](Self::identify) says otherwise.
	pub fn new(db: Db) -> Self {
		let hub = hub::Hub::new(db.system("the live hub reads every commit"));
		Self {
			inner: Arc::new(Inner {
				db,
				identify: Arc::new(super::Anonymous),
				origin: None,
				shared: Arc::new(Shared {
					drain: broadcast::channel(8).0,
					stats: Stats::default(),
					hub,
					connections: Mutex::new(HashMap::new()),
					rooms: Mutex::new(HashMap::new()),
					events: Mutex::new(HashMap::new()),
					#[cfg(feature = "relay")]
					relay: std::sync::OnceLock::new(),
					#[cfg(feature = "relay")]
					cluster: tokio::sync::OnceCell::new(),
				}),
			}),
		}
	}

	/// Tells callers apart with `identify`, such as `leptoskit_auth`'s
	/// `Auth`. The same implementation the HTTP routes use.
	#[must_use]
	pub fn identify(self, identify: impl Identify) -> Self {
		self.identify_arc(Arc::new(identify))
	}

	/// [`identify`](Self::identify) with an already shared implementation,
	/// such as the one [`Server`](super::Server) uses.
	#[must_use]
	pub fn identify_arc(self, identify: Arc<dyn Identify>) -> Self {
		self.with(|inner| inner.identify = identify)
	}

	/// The one origin a browser connection may come from, such as
	/// `https://myapp.com`. A native app sends no `Origin` and is allowed
	/// through to authenticate with
	/// [`ClientFrame::Auth`](crate::live::ClientFrame::Auth).
	///
	/// Without it, any origin may connect: development's default, and what
	/// a same-origin app gets from the page's own `Origin`.
	#[must_use]
	pub fn origin(self, origin: impl Into<String>) -> Self {
		let origin = origin.into();
		self.with(|inner| {
			inner.origin = Some(origin.trim_end_matches('/').to_owned());
		})
	}

	/// Takes commits from `relay`'s stream instead of reading the database's
	/// change log, and sends channel signals and presence to the other
	/// servers on it. One server, any server, publishes the log with
	/// [`Relay::publish`](crate::server::Relay::publish).
	///
	/// Call it before the first connection: the server joins the relay when
	/// it starts, at [`start`](Self::start) or its first connection.
	#[cfg(feature = "relay")]
	#[must_use]
	pub fn relay(self, relay: &crate::server::Relay) -> Self {
		let _ = self.inner.shared.relay.set(relay.clone());
		self.hub().use_relay(relay.clone());
		self
	}

	/// Whether re-runs may read from the database's read replicas, which
	/// they do by default when it has any (see
	/// [`Db::with_replica`](leptoskit_db::Db::with_replica)): a round of
	/// re-runs reads from a replica that has replayed past every commit in
	/// it, and from the primary when none gets there within `wait`, 20
	/// milliseconds unless [`replica_wait`](Self::replica_wait) says
	/// otherwise.
	#[must_use]
	pub fn rerun_on_replicas(self, enabled: bool) -> Self {
		self.hub().use_replicas(enabled, hub::REPLICA_WAIT);
		self
	}

	/// How long a round of re-runs waits for a replica to catch up before it
	/// reads from the primary instead.
	#[must_use]
	pub fn replica_wait(self, wait: std::time::Duration) -> Self {
		self.hub().use_replicas(true, wait);
		self
	}

	/// Starts following commits, and joins the relay if there is one,
	/// without waiting for the first connection: call it at startup, and
	/// pass the readiness check once it returns.
	///
	/// # Errors
	///
	/// When the change log can't be read, or the relay can't be reached.
	pub async fn start(&self) -> Result<(), WireError> {
		self.hub().start().await?;

		#[cfg(feature = "relay")]
		if let Some(relay) = self.inner.shared.relay.get() {
			self.inner
				.shared
				.cluster
				.get_or_init(|| cluster::Cluster::start(self, relay))
				.await;
		}

		Ok(())
	}

	/// Whether [`start`](Self::start) has finished, or a connection started
	/// the server.
	pub fn is_started(&self) -> bool {
		self.hub().is_started()
	}

	/// This server's members of `channel`, and the other servers'.
	fn presence_members(&self, channel: &str) -> Vec<PresenceMember> {
		let here: Vec<PresenceMember> = self
			.rooms()
			.get(channel)
			.map(|room| room.members.values().cloned().collect())
			.unwrap_or_default();

		#[cfg(feature = "relay")]
		if let Some(Some(cluster)) = self.inner.shared.cluster.get() {
			return here.into_iter().chain(cluster.members(channel)).collect();
		}

		here
	}

	/// This server's members of `channel` changed: tells the other servers,
	/// and every member here.
	fn presence_changed(&self, channel: &str) {
		#[cfg(feature = "relay")]
		if let Some(Some(cluster)) = self.inner.shared.cluster.get() {
			let members = self
				.rooms()
				.get(channel)
				.map(|room| room.members.values().cloned().collect())
				.unwrap_or_default();
			cluster.presence(channel, members);
		}

		let senders: Vec<mpsc::Sender<Outbound>> = {
			let rooms = self.rooms();
			let Some(room) = rooms.get(channel) else {
				return;
			};
			let connections = self.connections();
			room.members
				.keys()
				.filter_map(|id| connections.get(id).cloned())
				.collect()
		};
		let frame = crate::live::ServerFrame::Presence {
			channel: channel.to_owned(),
			members: self.presence_members(channel),
		};
		let Some(outbound) = Outbound::frame(&frame, None) else {
			return;
		};

		for sender in senders {
			let _ = sender.try_send(outbound.clone());
		}
	}

	/// Sends a signal from a member here to the other servers' members.
	#[cfg_attr(
		not(feature = "relay"),
		expect(unused_variables, reason = "only a relay reaches other servers")
	)]
	fn signal_elsewhere(&self, channel: &str, from: Option<String>, message: &Value) {
		#[cfg(feature = "relay")]
		if let Some(Some(cluster)) = self.inner.shared.cluster.get() {
			cluster.signal(channel, from, message);
		}
	}

	fn with(self, change: impl FnOnce(&mut Inner)) -> Self {
		let mut inner = Inner {
			db: self.inner.db.clone(),
			identify: Arc::clone(&self.inner.identify),
			origin: self.inner.origin.clone(),
			shared: Arc::clone(&self.inner.shared),
		};
		change(&mut inner);
		Self {
			inner: Arc::new(inner),
		}
	}

	/// The router serving `GET /live` (the WebSocket), `GET /live/events`
	/// (server-sent events) and `POST /live/frames` (their client frames),
	/// relative to wherever it is nested.
	pub fn router<S>(&self) -> Router<S>
	where
		S: Clone + Send + Sync + 'static,
	{
		Router::new()
			.route("/live", get(socket::upgrade))
			.route(EVENTS_PATH, get(sse::events))
			.route(FRAMES_PATH, post(sse::frames))
			.with_state(self.clone())
	}

	/// Sends every connection
	/// [`ServerFrame::Reconnect`](crate::live::ServerFrame::Reconnect) and
	/// closes it, with jitter so a whole deployment doesn't reconnect at
	/// once. Call it on `SIGTERM`, before the server stops accepting.
	/// Clients resume from the version they reached, so they receive only
	/// what changed while they were away.
	pub fn drain(&self) {
		let _ = self.inner.shared.drain.send(());
	}

	/// What this server has done.
	#[must_use]
	pub fn stats(&self) -> LiveStats {
		let stats = &self.inner.shared.stats;
		let (reruns, commits, replica_reruns, relay_fills) = self.hub().counts();
		LiveStats {
			connections: stats.connections.load(Ordering::Relaxed),
			subscriptions: stats.subscriptions.load(Ordering::Relaxed),
			reruns,
			transitions: stats.transitions.load(Ordering::Relaxed),
			commits,
			resumes: stats.resumes.load(Ordering::Relaxed),
			replica_reruns,
			relay_fills,
		}
	}

	fn stats_ref(&self) -> &Stats {
		&self.inner.shared.stats
	}

	fn hub(&self) -> &hub::Hub {
		&self.inner.shared.hub
	}

	/// A fresh connection number.
	fn next_number(&self) -> u32 {
		let number = self.stats_ref().connections.fetch_add(1, Ordering::Relaxed) + 1;
		u32::try_from(number).unwrap_or(u32::MAX)
	}

	fn connections(&self) -> MutexGuard<'_, HashMap<u32, mpsc::Sender<Outbound>>> {
		self.inner
			.shared
			.connections
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
	}

	fn rooms(&self) -> MutexGuard<'_, HashMap<String, Room>> {
		self.inner
			.shared
			.rooms
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
	}

	fn event_streams(&self) -> MutexGuard<'_, HashMap<String, sse::Handle>> {
		self.inner
			.shared
			.events
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
	}

	fn origin_ok(&self, headers: &HeaderMap) -> bool {
		let Some(allowed) = &self.inner.origin else {
			return true;
		};

		match headers.get(ORIGIN) {
			// Native apps send no `Origin`; they authenticate with a token.
			None => true,
			Some(origin) => {
				origin
					.to_str()
					.is_ok_and(|origin| origin.trim_end_matches('/') == allowed)
			}
		}
	}
}

impl std::fmt::Debug for Live {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Live")
			.field("version", &self.hub().applied())
			.finish_non_exhaustive()
	}
}

/// Runs the query `name` as `identity`, and returns its result with what it
/// read.
async fn run_query(
	name: &str,
	args: Value,
	db: Db,
	identity: Identity,
) -> Result<(Value, Vec<ReadEntry>), WireError> {
	let Some(registration) = registration(name) else {
		return Err(WireError::invalid(format!(
			"no operation is named `{name}`"
		)));
	};

	if registration.kind != Kind::Query {
		return Err(WireError::invalid(format!(
			"`{name}` is a {}, and only a query can be subscribed to",
			registration.kind
		)));
	}

	let sink = ReadSink::new();
	let value = (registration.handler)(Invocation {
		db,
		identity,
		request_id: None,
		arguments: args,
		reads: Some(sink.clone()),
	})
	.await
	.map_err(|error| WireError::new(&error))?;
	let size = serde_json::to_vec(&value).map_or(0, |bytes| bytes.len());

	if size > MAX_RESULT_BYTES {
		return Err(WireError {
			status: 413,
			message: format!(
				"`{name}` returned {size} bytes, more than the {MAX_RESULT_BYTES}-byte live \
				 limit; paginate it"
			),
		});
	}

	Ok((value, sink.take()))
}
