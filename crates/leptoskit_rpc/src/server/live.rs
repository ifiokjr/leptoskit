//! Serving live queries: the `/live` WebSocket, the subscriptions one
//! connection holds, and the transitions that tell it what changed.
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
//! # What it guarantees
//!
//! Every re-run reads the database as the connection's [`Identity`], through
//! the same policies as a direct call, so a subscriber only learns that a
//! query's result changed; nothing about a row they can't see is sent.
//!
//! Re-runs happen after the database's live feed says a transaction
//! committed. Until read sets land, a change re-runs every query on the
//! connection that could have seen it: correct, and more work than the
//! matcher in the plan will do once it exists.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::extract::FromRequestParts;
use axum::extract::State;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::Message;
use axum::extract::ws::WebSocket;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header::ORIGIN;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use futures::SinkExt;
use futures::StreamExt;
use leptoskit_db::Db;
use leptoskit_db::live::Feed;
use leptoskit_db::live::Invalidation;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::sync::mpsc;

use super::Identify;
use super::Identity;
use super::Invocation;
use super::QueryCx;
use super::registration;
use crate::Kind;
use crate::live::ClientFrame;
use crate::live::HEARTBEAT_SECONDS;
use crate::live::MAX_FRAME_BYTES;
use crate::live::MAX_RESULT_BYTES;
use crate::live::MAX_SUBSCRIPTIONS;
use crate::live::PresenceMember;
use crate::live::SUBPROTOCOL;
use crate::live::ServerFrame;
use crate::live::SubscriptionUpdate;
use crate::live::Transition;
use crate::live::Update;
use crate::live::WireError;
use crate::live::patch;

/// How many transitions may queue for a slow connection before intermediate
/// ones are dropped in favour of a full one.
const OUTGOING: usize = 32;

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
	version: AtomicU64,
	drain: broadcast::Sender<u64>,
	stats: Arc<Stats>,
	connections: Arc<Mutex<HashMap<u32, mpsc::Sender<Message>>>>,
	rooms: Arc<Mutex<HashMap<String, Room>>>,
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
	reruns: AtomicU64,
	transitions: AtomicU64,
}

/// What a live server has done since it started, for metrics and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveStats {
	/// Connections opened, including closed ones.
	pub connections: u64,
	/// Subscriptions made, including ended ones.
	pub subscriptions: u64,
	/// Queries re-run after a change.
	pub reruns: u64,
	/// Transitions sent, including ones dropped for a slow connection.
	pub transitions: u64,
}

impl Live {
	/// Live queries on `db`. Every caller is anonymous until
	/// [`identify`](Self::identify) says otherwise.
	pub fn new(db: Db) -> Self {
		Self {
			inner: Arc::new(Inner {
				db,
				identify: Arc::new(super::Anonymous),
				origin: None,
				version: AtomicU64::new(0),
				drain: broadcast::channel(8).0,
				stats: Arc::new(Stats::default()),
				connections: Arc::new(Mutex::new(HashMap::new())),
				rooms: Arc::new(Mutex::new(HashMap::new())),
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
	/// through to authenticate with [`ClientFrame::Auth`].
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

	fn with(self, change: impl FnOnce(&mut Inner)) -> Self {
		let mut inner = Inner {
			db: self.inner.db.clone(),
			identify: Arc::clone(&self.inner.identify),
			origin: self.inner.origin.clone(),
			version: AtomicU64::new(self.inner.version.load(Ordering::Relaxed)),
			drain: self.inner.drain.clone(),
			stats: Arc::clone(&self.inner.stats),
			connections: Arc::clone(&self.inner.connections),
			rooms: Arc::clone(&self.inner.rooms),
		};
		change(&mut inner);
		Self {
			inner: Arc::new(inner),
		}
	}

	/// The router serving `GET /live`, relative to wherever it is nested.
	pub fn router<S>(&self) -> Router<S>
	where
		S: Clone + Send + Sync + 'static,
	{
		Router::new()
			.route("/live", get(upgrade))
			.with_state(self.clone())
	}

	/// Sends every connection [`ServerFrame::Reconnect`] and closes it, with
	/// jitter so a whole deployment doesn't reconnect at once. Call it on
	/// `SIGTERM`, before the server stops accepting.
	pub fn drain(&self) {
		let _ = self
			.inner
			.drain
			.send(self.inner.version.load(Ordering::Relaxed));
	}

	/// What this server has done.
	#[must_use]
	pub fn stats(&self) -> LiveStats {
		LiveStats {
			connections: self.inner.stats.connections.load(Ordering::Relaxed),
			subscriptions: self.inner.stats.subscriptions.load(Ordering::Relaxed),
			reruns: self.inner.stats.reruns.load(Ordering::Relaxed),
			transitions: self.inner.stats.transitions.load(Ordering::Relaxed),
		}
	}

	fn version(&self) -> u64 {
		self.inner.version.load(Ordering::Relaxed)
	}

	fn connections(&self) -> MutexGuard<'_, HashMap<u32, mpsc::Sender<Message>>> {
		self.inner
			.connections
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
	}

	fn rooms(&self) -> MutexGuard<'_, HashMap<String, Room>> {
		self.inner
			.rooms
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
	}

	fn next_version(&self) -> u64 {
		self.inner.version.fetch_add(1, Ordering::Relaxed) + 1
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

async fn upgrade(State(live): State<Live>, request: axum::extract::Request) -> Response {
	let (mut parts, _body) = request.into_parts();
	if !live.origin_ok(&parts.headers) {
		return StatusCode::FORBIDDEN.into_response();
	}

	let identity = match live.inner.identify.identify(&parts).await {
		Ok(identity) => identity,
		Err(error) => return super::http::failure(&error),
	};

	let websocket = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
		Ok(websocket) => websocket,
		Err(rejection) => return rejection.into_response(),
	};

	let connection = live.clone();
	websocket
		.protocols([SUBPROTOCOL])
		.on_upgrade(move |socket| async move { connection.run(socket, identity).await })
}

struct Session {
	live: Live,
	identity: Identity,
	db: Db,
	feed: Feed,
	subscriptions: HashMap<u32, Subscription>,
	joined: HashMap<String, Joined>,
	number: u32,
	member: String,
	last_sent: u64,
	full_next: bool,
	out: mpsc::Sender<Message>,
	drain: bool,
}

struct Subscription {
	query: String,
	args: Value,
	result: Value,
}

struct Joined {
	name: String,
	args: Value,
	state: Value,
}

impl Live {
	async fn run(self, socket: WebSocket, identity: Identity) {
		let number = self.inner.stats.connections.fetch_add(1, Ordering::Relaxed) as u32 + 1;
		let db = self.inner.db.with_context(identity.context.clone());
		let Ok(feed) = db.changes().await else {
			return;
		};

		let (out, mut outgoing) = mpsc::channel::<Message>(OUTGOING);
		let (mut sink, mut stream) = socket.split();
		let writer = tokio::spawn(async move {
			while let Some(message) = outgoing.recv().await {
				if sink.send(message).await.is_err() {
					break;
				}
			}
			let _ = sink.close().await;
		});

		let version = self.version();
		let member = identity
			.caller
			.clone()
			.unwrap_or_else(|| format!("live-{number}"));
		let mut session = Session {
			live: self.clone(),
			identity,
			db,
			feed,
			subscriptions: HashMap::new(),
			joined: HashMap::new(),
			number,
			member,
			last_sent: version,
			full_next: false,
			out: out.clone(),
			drain: false,
		};
		self.connections().insert(number, out);

		session
			.send(ServerFrame::Welcome {
				connection: format!("live-{number}"),
				version,
				heartbeat_seconds: HEARTBEAT_SECONDS,
				outdated_client: false,
			})
			.await;

		let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECONDS));
		heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		let mut drained = self.inner.drain.subscribe();

		loop {
			tokio::select! {
				frame = stream.next() => {
					let Some(Ok(message)) = frame else { break };
					if !session.handle(message).await {
						break;
					}
				}
				invalidation = session.feed.next() => {
					let Ok(invalidation) = invalidation else { break };
					session.changed(invalidation).await;
				}
				_ = heartbeat.tick() => {
					if session.out.send(Message::Ping(Vec::new().into())).await.is_err() {
						break;
					}
				}
				_ = drained.recv() => {
					session.drain = true;
					break;
				}
			}
		}

		if session.drain {
			let after_ms = 250 + (session.last_sent % 2_000);
			session.send(ServerFrame::Reconnect { after_ms }).await;
		}
		session.leave_all();
		let _ = self.connections().remove(&number);
		drop(session.out);
		let _ = writer.await;
	}
}

impl Session {
	async fn send(&self, frame: ServerFrame) -> bool {
		let Ok(text) = serde_json::to_string(&frame) else {
			return true;
		};
		self.out.send(Message::Text(text.into())).await.is_ok()
	}

	async fn done(&self, request: u32, result: Result<Value, WireError>) -> bool {
		let (ok, err) = match result {
			Ok(value) => (Some(value), None),
			Err(error) => (None, Some(error)),
		};
		self.send(ServerFrame::Done {
			request,
			ok,
			err,
			version: self.live.version(),
		})
		.await
	}

	/// Handles one frame. Returns whether the connection stays open.
	async fn handle(&mut self, message: Message) -> bool {
		let text = match message {
			Message::Text(text) => text,
			Message::Binary(bytes) => {
				match String::from_utf8(bytes.to_vec()) {
					Ok(text) => text.into(),
					Err(_) => return true,
				}
			}
			Message::Close(_) => return false,
			Message::Ping(_) | Message::Pong(_) => return true,
		};

		if text.len() > MAX_FRAME_BYTES {
			return self
				.done(
					0,
					Err(WireError::invalid(format!(
						"the frame is larger than {MAX_FRAME_BYTES} bytes"
					))),
				)
				.await;
		}

		let frame: ClientFrame = match serde_json::from_str(&text) {
			Ok(frame) => frame,
			Err(error) => {
				return self
					.done(
						0,
						Err(WireError::invalid(format!(
							"the frame doesn't parse: {error}"
						))),
					)
					.await;
			}
		};

		match frame {
			ClientFrame::Hello { .. } | ClientFrame::Unknown => true,
			ClientFrame::Ping => self.send(ServerFrame::Pong).await,
			ClientFrame::Sub { id, query, args } => self.subscribe(id, query, args).await,
			ClientFrame::Unsub { id } => {
				self.subscriptions.remove(&id);
				true
			}
			ClientFrame::Join {
				channel,
				name,
				args,
				state,
			} => self.join(channel, &name, args, state).await,
			ClientFrame::Leave { channel } => {
				self.leave(&channel);
				true
			}
			ClientFrame::Signal { channel, message } => {
				self.signal(&channel, message);
				true
			}
			ClientFrame::Mutate {
				request,
				name,
				args,
				key,
			} => self.mutate(request, &name, args, key).await,
			ClientFrame::Auth { token } => self.authenticate(token).await,
		}
	}

	async fn subscribe(&mut self, id: u32, query: String, args: Value) -> bool {
		if self.subscriptions.len() >= MAX_SUBSCRIPTIONS && !self.subscriptions.contains_key(&id) {
			return self
				.done(
					id,
					Err(WireError::invalid(format!(
						"a connection may subscribe to {MAX_SUBSCRIPTIONS} queries"
					))),
				)
				.await;
		}

		match self.query(&query, args.clone()).await {
			Ok(result) => {
				self.live
					.inner
					.stats
					.subscriptions
					.fetch_add(1, Ordering::Relaxed);
				self.subscriptions.insert(
					id,
					Subscription {
						query,
						args,
						result: result.clone(),
					},
				);
				self.done(id, Ok(result)).await
			}
			Err(error) => self.done(id, Err(error)).await,
		}
	}

	async fn mutate(&mut self, request: u32, name: &str, args: Value, key: Option<String>) -> bool {
		let Some(registration) = registration(name) else {
			return self
				.done(
					request,
					Err(WireError::invalid(format!(
						"no operation is named `{name}`"
					))),
				)
				.await;
		};
		if registration.kind != Kind::Mutation {
			return self
				.done(
					request,
					Err(WireError::invalid(format!(
						"`{name}` is a {}, and only a mutation runs over a live connection",
						registration.kind
					))),
				)
				.await;
		}

		let result = (registration.handler)(Invocation {
			db: self.db.clone(),
			identity: self.identity.clone(),
			request_id: key,
			arguments: args,
			reads: None,
		})
		.await
		.map_err(|error| WireError::new(&error));
		// The write's own transition follows on the feed. Publish the
		// version this call reaches, so a client can hold its optimistic
		// update until it arrives.
		let version = self.live.next_version();
		let (ok, err) = match result {
			Ok(value) => (Some(value), None),
			Err(error) => (None, Some(error)),
		};
		self.send(ServerFrame::Done {
			request,
			ok,
			err,
			version,
		})
		.await
	}

	async fn authenticate(&mut self, token: String) -> bool {
		let Ok(identity) = self.live.inner.identify.identify_token(&token).await else {
			return self
				.send(ServerFrame::AuthStatus {
					refused: true,
					expiring: false,
				})
				.await;
		};

		self.identity = identity;
		self.db = self
			.live
			.inner
			.db
			.with_context(self.identity.context.clone());
		let Ok(feed) = self.db.changes().await else {
			return false;
		};
		self.feed = feed;

		// Every result is now read as somebody else: re-run them all.
		let mut updates = Vec::new();
		let ids: Vec<u32> = self.subscriptions.keys().copied().collect();
		for id in ids {
			let Some((query, args)) = self
				.subscriptions
				.get(&id)
				.map(|sub| (sub.query.clone(), sub.args.clone()))
			else {
				continue;
			};
			match self.query(&query, args).await {
				Ok(result) => {
					updates.push(SubscriptionUpdate {
						id,
						update: Update::Ok(result.clone()),
					});
					if let Some(subscription) = self.subscriptions.get_mut(&id) {
						subscription.result = result;
					}
				}
				Err(error) => {
					updates.push(SubscriptionUpdate {
						id,
						update: Update::Err(error),
					});
				}
			}
		}

		if !updates.is_empty() {
			let to = self.live.next_version();
			self.send_transition(updates, to, true);
		}

		// The identity changed, so every channel is checked again.
		self.member = self
			.identity
			.caller
			.clone()
			.unwrap_or_else(|| format!("live-{}", self.number));
		let keys: Vec<String> = self.joined.keys().cloned().collect();
		for key in keys {
			let Some((name, args, state)) = self.joined.get(&key).map(|joined| {
				(
					joined.name.clone(),
					joined.args.clone(),
					joined.state.clone(),
				)
			}) else {
				continue;
			};
			let Some(registration) = super::channel::registration(&name) else {
				self.leave(&key);
				continue;
			};
			let cx = QueryCx::new(self.db.clone(), self.identity.clone());
			match (registration.authorize)(args, cx).await {
				Ok(()) => {
					{
						let mut rooms = self.live.rooms();
						if let Some(room) = rooms.get_mut(&key)
							&& let Some(member) = room.members.get_mut(&self.number)
						{
							member.id.clone_from(&self.member);
							member.state = state;
						}
					}
					self.broadcast_presence(&key);
				}
				Err(error) => {
					self.leave(&key);
					self.channel_error(&key, WireError::new(&error)).await;
				}
			}
		}

		self.send(ServerFrame::AuthStatus {
			refused: false,
			expiring: false,
		})
		.await
	}

	async fn changed(&mut self, invalidation: Invalidation) {
		let mut updates = Vec::new();

		let ids: Vec<u32> = self.subscriptions.keys().copied().collect();
		for id in ids {
			let Some((query, args, previous)) = self
				.subscriptions
				.get(&id)
				.map(|sub| (sub.query.clone(), sub.args.clone(), sub.result.clone()))
			else {
				continue;
			};
			self.live.inner.stats.reruns.fetch_add(1, Ordering::Relaxed);
			match self.query(&query, args).await {
				Ok(result) => {
					match invalidation {
						Invalidation::Everything => {
							updates.push(SubscriptionUpdate {
								id,
								update: Update::Ok(result.clone()),
							});
							if let Some(subscription) = self.subscriptions.get_mut(&id) {
								subscription.result = result;
							}
						}
						Invalidation::Rows(_) if result != previous => {
							let update = patch(&previous, &result);
							if let Some(subscription) = self.subscriptions.get_mut(&id) {
								subscription.result = result;
							}
							updates.push(SubscriptionUpdate { id, update });
						}
						Invalidation::Rows(_) => {}
					}
				}
				Err(error) => {
					updates.push(SubscriptionUpdate {
						id,
						update: Update::Err(error),
					});
				}
			}
		}

		if updates.is_empty() {
			return;
		}
		let to = self.live.next_version();
		self.send_transition(updates, to, false);
	}

	fn send_transition(&mut self, mut updates: Vec<SubscriptionUpdate>, to: u64, full: bool) {
		let full = full || self.full_next;
		if full {
			updates = self
				.subscriptions
				.iter()
				.map(|(id, subscription)| {
					SubscriptionUpdate {
						id: *id,
						update: Update::Ok(subscription.result.clone()),
					}
				})
				.collect();
		}
		self.full_next = false;

		// A frame over the limit is split, all at the same `to`: the client
		// applies the chunks in order and lands on the same state.
		let mut chunk = Vec::new();
		for update in updates {
			chunk.push(update);
			let transition = Transition {
				from: self.last_sent,
				to,
				updates: chunk.clone(),
				full,
			};
			let Ok(text) = serde_json::to_string(&ServerFrame::Transition(transition)) else {
				continue;
			};
			if text.len() > MAX_FRAME_BYTES && chunk.len() > 1 {
				let last = chunk.pop().expect("just pushed");
				self.send_chunk(&chunk, to, full);
				chunk = vec![last];
			}
		}
		if !chunk.is_empty() {
			self.send_chunk(&chunk, to, full);
		}

		self.last_sent = to;
		self.live
			.inner
			.stats
			.transitions
			.fetch_add(1, Ordering::Relaxed);
	}

	fn send_chunk(&mut self, updates: &[SubscriptionUpdate], to: u64, full: bool) {
		let transition = Transition {
			from: self.last_sent,
			to,
			updates: updates.to_vec(),
			full,
		};
		let Ok(text) = serde_json::to_string(&ServerFrame::Transition(transition)) else {
			return;
		};
		if self.out.try_send(Message::Text(text.into())).is_err() {
			// A slow connection gets the newest state, not every step.
			self.full_next = true;
			self.live
				.inner
				.stats
				.transitions
				.fetch_add(1, Ordering::Relaxed);
		}
	}

	async fn join(&mut self, channel: String, name: &str, args: Value, state: Value) -> bool {
		if self.joined.contains_key(&channel) {
			self.leave(&channel);
		}
		let Some(registration) = super::channel::registration(name) else {
			return self
				.channel_error(
					&channel,
					WireError::invalid(format!("no channel is named `{name}`")),
				)
				.await;
		};

		let cx = QueryCx::new(self.db.clone(), self.identity.clone());
		if let Err(error) = (registration.authorize)(args.clone(), cx).await {
			return self.channel_error(&channel, WireError::new(&error)).await;
		}

		{
			let mut rooms = self.live.rooms();
			let room = rooms.entry(channel.clone()).or_default();
			room.members.insert(
				self.number,
				PresenceMember {
					id: self.member.clone(),
					state: state.clone(),
				},
			);
		}
		self.joined.insert(
			channel.clone(),
			Joined {
				name: name.to_owned(),
				args,
				state,
			},
		);
		self.broadcast_presence(&channel);
		true
	}

	fn leave(&mut self, channel: &str) {
		if self.joined.remove(channel).is_none() {
			return;
		}
		let empty = {
			let mut rooms = self.live.rooms();
			if let Some(room) = rooms.get_mut(channel) {
				room.members.remove(&self.number);
			}
			rooms
				.get(channel)
				.is_none_or(|room| room.members.is_empty())
		};
		if empty {
			self.live.rooms().remove(channel);
		} else {
			self.broadcast_presence(channel);
		}
	}

	fn leave_all(&mut self) {
		let keys: Vec<String> = self.joined.keys().cloned().collect();
		for key in keys {
			self.leave(&key);
		}
	}

	fn signal(&self, channel: &str, message: Value) {
		if !self.joined.contains_key(channel) {
			return;
		}
		let (senders, from) = {
			let rooms = self.live.rooms();
			let Some(room) = rooms.get(channel) else {
				return;
			};
			let connections = self.live.connections();
			let senders: Vec<mpsc::Sender<Message>> = room
				.members
				.keys()
				.filter(|id| **id != self.number)
				.filter_map(|id| connections.get(id).cloned())
				.collect();
			let from = room
				.members
				.get(&self.number)
				.map(|member| member.id.clone());
			(senders, from)
		};
		let frame = ServerFrame::Signal {
			channel: channel.to_owned(),
			from,
			message,
		};
		let Ok(text) = serde_json::to_string(&frame) else {
			return;
		};
		for sender in senders {
			let _ = sender.try_send(Message::Text(text.clone().into()));
		}
	}

	fn broadcast_presence(&self, channel: &str) {
		let (members, senders) = {
			let rooms = self.live.rooms();
			let Some(room) = rooms.get(channel) else {
				return;
			};
			let members: Vec<PresenceMember> = room.members.values().cloned().collect();
			let connections = self.live.connections();
			let senders: Vec<mpsc::Sender<Message>> = room
				.members
				.keys()
				.filter_map(|id| connections.get(id).cloned())
				.collect();
			(members, senders)
		};
		let frame = ServerFrame::Presence {
			channel: channel.to_owned(),
			members,
		};
		let Ok(text) = serde_json::to_string(&frame) else {
			return;
		};
		for sender in senders {
			let _ = sender.try_send(Message::Text(text.clone().into()));
		}
	}

	async fn channel_error(&self, channel: &str, err: WireError) -> bool {
		self.send(ServerFrame::ChannelError {
			channel: channel.to_owned(),
			err,
		})
		.await
	}

	async fn query(&self, name: &str, args: Value) -> Result<Value, WireError> {
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
		let value = (registration.handler)(Invocation {
			db: self.db.clone(),
			identity: self.identity.clone(),
			request_id: None,
			arguments: args,
			reads: None,
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
		Ok(value)
	}
}

impl std::fmt::Debug for Live {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Live")
			.field("version", &self.version())
			.finish_non_exhaustive()
	}
}
