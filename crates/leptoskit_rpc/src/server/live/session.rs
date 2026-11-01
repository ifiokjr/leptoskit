//! One live connection, whatever carries it.
//!
//! A session reads [`Incoming`] client frames from a channel and writes
//! [`Outbound`] server frames to another; the WebSocket and server-sent
//! events routes only move frames between those channels and the network.
//! Everything else is here, the same for both: the welcome, heartbeats,
//! drain, limits, identity, subscriptions, mutations, actions, channels,
//! resuming, and the transitions the hub's batches become.
//!
//! The session is the only writer of its connection's transitions and
//! answers, so a mutation's `done` always goes out before the transition
//! its own write causes, and a subscription's first value before any
//! change to it.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::ReadEntry;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::watch;

use super::Identity;
use super::Incoming;
use super::Invocation;
use super::Live;
use super::Outbound;
use super::hub::Batch;
use super::hub::Registration;
use super::hub::ShareKey;
use super::run_query;
use crate::Kind;
use crate::live::ClientFrame;
use crate::live::EndReason;
use crate::live::HEARTBEAT_SECONDS;
use crate::live::MAX_FRAME_BYTES;
use crate::live::MAX_STREAMS;
use crate::live::MAX_SUBSCRIPTIONS;
use crate::live::PresenceMember;
use crate::live::Resume;
use crate::live::ServerFrame;
use crate::live::SubscriptionUpdate;
use crate::live::Transition;
use crate::live::Update;
use crate::live::WireError;
use crate::server::QueryCx;
use crate::server::channel;
use crate::server::registration;
use crate::server::run::recording_commits;
use crate::server::stream;
use crate::server::stream::StreamSink;

/// How often a subscription's first run may find a round applied under it
/// before it registers anyway and has the hub run it again.
const REGISTER_ATTEMPTS: usize = 3;

/// The most credits a stream may hold at once, however many the client
/// grants.
const MAX_CREDITS: usize = 1 << 20;

/// Who may post frames to a connection: the caller its identity names,
/// which changes when the connection authenticates.
pub(super) type CallerSlot = Arc<Mutex<Option<String>>>;

/// A connection a transport has opened, for [`Live::serve`].
pub(super) struct Opened {
	/// The connection's number, from [`Live::next_number`].
	pub number: u32,
	/// Its name in `welcome`.
	pub connection: String,
	/// Who opened it.
	pub identity: Identity,
	/// The client's frames.
	pub frames: mpsc::Receiver<Incoming>,
	/// Where the session writes.
	pub out: mpsc::Sender<Outbound>,
	/// A version the transport already knows the client reached, such as
	/// server-sent events' `Last-Event-ID`.
	pub resume_floor: Option<u64>,
	/// Kept current with the identity's caller.
	pub caller: CallerSlot,
}

struct Session {
	live: Live,
	number: u32,
	identity: Identity,
	db: Db,
	member: String,
	caller: CallerSlot,
	out: mpsc::Sender<Outbound>,
	batches: mpsc::UnboundedReceiver<Batch>,
	finished: mpsc::UnboundedReceiver<Finished>,
	acts: mpsc::UnboundedSender<Finished>,
	applied: watch::Receiver<u64>,
	subscriptions: HashMap<u32, Subscription>,
	joined: HashMap<String, Joined>,
	streams: HashMap<u32, Running>,
	/// What running streams send, forwarded in order.
	stream_frames: mpsc::UnboundedReceiver<String>,
	stream_out: mpsc::UnboundedSender<String>,
	/// The version of the last transition sent.
	last_sent: u64,
	/// Whether a transition was dropped for a slow connection, so the next
	/// repeats every value.
	full_next: bool,
	/// The newest commit a mutation or action answered with that no
	/// transition has reached yet.
	awaiting: Option<u64>,
	resume_floor: Option<u64>,
}

struct Subscription {
	query: String,
	args: Value,
	value: Arc<Value>,
}

struct Joined {
	name: String,
	args: Value,
	state: Value,
}

/// A stream the connection opened, and the room its client has left.
struct Running {
	task: tokio::task::JoinHandle<()>,
	credits: Arc<tokio::sync::Semaphore>,
}

/// A mutation or action that finished, and the commit it reached.
struct Finished {
	request: u32,
	result: Result<Value, WireError>,
	version: Option<i64>,
}

/// A subscription's first result, and where it came from.
struct Started {
	value: Arc<Value>,
	reads: Vec<ReadEntry>,
	/// The version the hub had applied when the value was registered: the
	/// value includes every commit up to it.
	at: u64,
}

impl Outbound {
	fn frame(frame: &ServerFrame, version: Option<u64>) -> Option<Self> {
		let text = serde_json::to_string(frame).ok()?;
		Some(Self::Frame { text, version })
	}
}

impl Live {
	/// Runs a connection until its client goes, its transport closes, or
	/// the server drains it.
	pub(super) async fn serve(&self, opened: Opened) {
		let Opened {
			number,
			connection,
			identity,
			mut frames,
			out,
			resume_floor,
			caller,
		} = opened;

		if let Err(error) = self.hub().start().await {
			if let Some(frame) = Outbound::frame(
				&ServerFrame::Done {
					request: 0,
					ok: None,
					err: Some(error),
					version: 0,
				},
				None,
			) {
				let _ = out.send(frame).await;
			}

			return;
		}

		let (events, batches) = mpsc::unbounded_channel();
		let (acts, finished) = mpsc::unbounded_channel();
		let (stream_out, stream_frames) = mpsc::unbounded_channel();
		self.hub().connect(number, events);
		self.connections().insert(number, out.clone());
		let version = self.hub().applied();
		let member = identity
			.caller
			.clone()
			.unwrap_or_else(|| format!("live-{number}"));
		let mut session = Session {
			live: self.clone(),
			number,
			db: self.inner.db.with_context(identity.context.clone()),
			identity,
			member,
			caller,
			out,
			batches,
			finished,
			acts,
			applied: self.hub().watch(),
			subscriptions: HashMap::new(),
			joined: HashMap::new(),
			streams: HashMap::new(),
			stream_frames,
			stream_out,
			last_sent: version,
			full_next: false,
			awaiting: None,
			resume_floor,
		};
		session.note_caller();

		session
			.send(ServerFrame::Welcome {
				connection,
				version,
				heartbeat_seconds: HEARTBEAT_SECONDS,
				outdated_client: false,
			})
			.await;

		let mut drained = self.inner.shared.drain.subscribe();

		if session.run(&mut frames, &mut drained).await {
			session.interrupt_streams().await;
			let after_ms = 250 + u64::from(number).wrapping_mul(7919) % 2_000;
			session.send(ServerFrame::Reconnect { after_ms }).await;
		}

		for running in session.streams.values() {
			running.task.abort();
		}

		session.leave_all();
		self.hub()
			.disconnect(number, session.subscriptions.keys().copied());
		self.connections().remove(&number);
	}
}

impl Session {
	/// Reads frames and batches, answers them, and says whether the server
	/// is draining and wants the client back.
	async fn run(
		&mut self,
		frames: &mut mpsc::Receiver<Incoming>,
		drained: &mut broadcast::Receiver<()>,
	) -> bool {
		let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECONDS));
		heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

		loop {
			tokio::select! {
				incoming = frames.recv() => {
					let Some(incoming) = incoming else { return false };
					if !self.handle(incoming).await {
						return false;
					}
				}
				Some(batch) = self.batches.recv() => self.apply(batch),
				Some(finished) = self.finished.recv() => {
					if !self.answer(finished).await {
						return false;
					}
				}
				Some(text) = self.stream_frames.recv() => {
					if self.out.send(Outbound::Frame { text, version: None }).await.is_err() {
						return false;
					}
				}
				Ok(()) = self.applied.changed(), if self.awaiting.is_some() => {}
				_ = heartbeat.tick() => {
					// An idle connection still learns how far the log has
					// gone, so it resumes from somewhere recent.
					self.settle(true);
					if self.out.send(Outbound::Heartbeat).await.is_err() {
						return false;
					}
				}
				() = self.out.closed() => return false,
				_ = drained.recv() => return true,
			}

			self.settle(false);
		}
	}

	/// Sends what the hub has queued, and an empty transition when a
	/// mutation is waiting for a commit that changed none of this
	/// connection's results, or when `catch_up` asks for one.
	fn settle(&mut self, catch_up: bool) {
		while let Ok(batch) = self.batches.try_recv() {
			self.apply(batch);
		}

		let applied = *self.applied.borrow_and_update();
		let due = catch_up || self.awaiting.is_some_and(|waiting| waiting <= applied);

		if due && self.last_sent < applied {
			self.send_transition(self.last_sent, Vec::new(), applied, false);
		}

		if self
			.awaiting
			.is_some_and(|waiting| waiting <= self.last_sent)
		{
			self.awaiting = None;
		}
	}

	async fn send(&self, frame: ServerFrame) -> bool {
		let Some(outbound) = Outbound::frame(&frame, None) else {
			return true;
		};
		self.out.send(outbound).await.is_ok()
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
			version: self.last_sent,
		})
		.await
	}

	/// Handles one frame. Returns whether the connection stays open.
	async fn handle(&mut self, incoming: Incoming) -> bool {
		let frame = match incoming {
			Incoming::Frame(frame) => frame,
			Incoming::Invalid(error) => return self.done(0, Err(error)).await,
		};

		match frame {
			ClientFrame::Hello { resume } => {
				match resume {
					Some(resume) => self.resume(resume).await,
					None => true,
				}
			}
			ClientFrame::Unknown => true,
			ClientFrame::Ping => self.send(ServerFrame::Pong).await,
			ClientFrame::Sub { id, query, args } => self.subscribe(id, query, args).await,
			ClientFrame::Unsub { id } => {
				self.subscriptions.remove(&id);
				self.live.hub().leave(self.number, id);
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
			ClientFrame::Act {
				request,
				name,
				args,
			} => self.act(request, &name, args).await,
			ClientFrame::Auth { token } => self.authenticate(token).await,
			ClientFrame::Stream {
				stream,
				name,
				args,
				credits,
			} => self.open_stream(stream, &name, args, credits).await,
			ClientFrame::Credit { stream, n } => {
				self.credit(stream, n);
				true
			}
			ClientFrame::Cancel { stream } => {
				if let Some(running) = self.streams.remove(&stream) {
					running.task.abort();
				}

				true
			}
		}
	}

	/// Opens a stream as the connection's identity, with room for
	/// `credits` items.
	async fn open_stream(&mut self, stream: u32, name: &str, args: Value, credits: u32) -> bool {
		self.streams
			.retain(|_, running| !running.task.is_finished());

		if let Some(previous) = self.streams.remove(&stream) {
			previous.task.abort();
		}

		let Some(registration) = stream::registration(name) else {
			let error = WireError::invalid(format!("no stream is named `{name}`"));
			return self.send(EndReason::Failed(error).frame(stream)).await;
		};

		if self.streams.len() >= MAX_STREAMS {
			let error =
				WireError::invalid(format!("a connection may hold {MAX_STREAMS} streams open"));
			return self.send(EndReason::Failed(error).frame(stream)).await;
		}

		let credits = Arc::new(tokio::sync::Semaphore::new(
			usize::try_from(credits)
				.unwrap_or(MAX_CREDITS)
				.min(MAX_CREDITS),
		));
		let sink = StreamSink::new(stream, self.stream_out.clone(), Arc::clone(&credits));
		let cx = QueryCx::new(self.db.clone(), self.identity.clone());
		let task = tokio::spawn((registration.open)(args, cx, sink));
		self.streams.insert(stream, Running { task, credits });
		true
	}

	/// Gives a stream room for `n` more items.
	fn credit(&self, stream: u32, n: u32) {
		let Some(running) = self.streams.get(&stream) else {
			return;
		};
		let room = MAX_CREDITS.saturating_sub(running.credits.available_permits());
		let n = usize::try_from(n).unwrap_or(room).min(room);

		if n > 0 {
			running.credits.add_permits(n);
		}
	}

	/// Stops every stream still running, after sending what they already
	/// produced, and tells the client each was interrupted: the connection
	/// is going, or is about to read as somebody else.
	async fn interrupt_streams(&mut self) {
		let interrupted: Vec<u32> = self
			.streams
			.drain()
			.filter(|(_, running)| {
				running.task.abort();
				!running.task.is_finished()
			})
			.map(|(stream, _)| stream)
			.collect();

		while let Ok(text) = self.stream_frames.try_recv() {
			let _ = self
				.out
				.send(Outbound::Frame {
					text,
					version: None,
				})
				.await;
		}

		for stream in interrupted {
			self.send(EndReason::Interrupted.frame(stream)).await;
		}
	}

	fn over_limit(&self, id: u32) -> Option<WireError> {
		(self.subscriptions.len() >= MAX_SUBSCRIPTIONS && !self.subscriptions.contains_key(&id))
			.then(|| {
				WireError::invalid(format!(
					"a connection may subscribe to {MAX_SUBSCRIPTIONS} queries"
				))
			})
	}

	async fn subscribe(&mut self, id: u32, query: String, args: Value) -> bool {
		if let Some(error) = self.over_limit(id) {
			return self.done(id, Err(error)).await;
		}

		match self.start(id, &query, &args, true).await {
			Ok(started) => {
				let value = (*started.value).clone();
				self.keep(id, query, args, started.value);
				self.done(id, Ok(value)).await
			}
			Err(error) => self.done(id, Err(error)).await,
		}
	}

	fn keep(&mut self, id: u32, query: String, args: Value, value: Arc<Value>) {
		self.live
			.stats_ref()
			.subscriptions
			.fetch_add(1, Ordering::Relaxed);
		self.subscriptions
			.insert(id, Subscription { query, args, value });
	}

	/// Subscribes `id` through the hub: joins the group already watching
	/// the same query as the same caller, or runs it and starts one.
	/// `known` says whether the client will have the value this returns.
	async fn start(
		&self,
		id: u32,
		query: &str,
		args: &Value,
		known: bool,
	) -> Result<Started, WireError> {
		let hub = self.live.hub();
		let key = ShareKey::new(query, args, &self.identity, self.number, id);

		if let Some((_, reads)) = hub.peek(&key) {
			let at = hub.applied();

			if let Some(value) = hub.join(self.number, id, &key, known) {
				return Ok(Started { value, reads, at });
			}
		}

		for attempt in 0..=REGISTER_ATTEMPTS {
			let since = hub.applied();
			let (value, reads) =
				run_query(query, args.clone(), self.db.clone(), self.identity.clone()).await?;
			let value = Arc::new(value);
			let registration = Registration {
				key: key.clone(),
				query: query.to_owned(),
				args: args.clone(),
				identity: self.identity.clone(),
				db: self.db.clone(),
				result: Arc::clone(&value),
				reads: reads.clone(),
			};
			let checked = (attempt < REGISTER_ATTEMPTS).then_some(since);

			if hub.register(self.number, id, checked, known, registration) {
				return Ok(Started {
					value,
					reads,
					at: since,
				});
			}
		}

		unreachable!("the last attempt registers whatever the hub has applied")
	}

	/// Subscribes again to what a reconnecting client holds, and sends one
	/// transition with only what changed since the version it reached, or
	/// with everything when the change log no longer reaches back that far.
	async fn resume(&mut self, resume: Resume) -> bool {
		if resume.queries.is_empty() {
			return true;
		}

		let hub = self.live.hub();
		let from = resume.version.max(self.resume_floor.unwrap_or(0));
		let to = hub.applied();
		// What changed after `from`, through `covered`.
		let mut seen_changes = hub.changes_between(from, to).await;
		let full = seen_changes.is_none();
		let mut covered = to;
		let dialect = hub.dialect();

		if !full {
			self.live
				.stats_ref()
				.resumes
				.fetch_add(1, Ordering::Relaxed);
		}

		let mut updates = Vec::new();

		for query in resume.queries {
			if let Some(error) = self.over_limit(query.id) {
				updates.push(SubscriptionUpdate {
					id: query.id,
					update: Update::Err(error),
				});
				continue;
			}

			match self.start(query.id, &query.query, &query.args, false).await {
				Ok(started) => {
					// A round applied while the client resumed: read what it
					// had, too.
					if started.at > covered
						&& let Some(seen) = &mut seen_changes
					{
						match self.live.hub().changes_between(covered, started.at).await {
							Some(more) => {
								seen.extend(more);
								covered = started.at;
							}
							None => seen_changes = None,
						}
					}

					let changed = match &seen_changes {
						Some(changes) if started.at <= covered => {
							started.reads.is_empty()
								|| started.reads.iter().any(|read| {
									changes
										.iter()
										.any(|change| read.could_change(change, dialect))
								})
						}
						_ => true,
					};

					if changed {
						self.live.hub().known(self.number, query.id, &started.value);
						updates.push(SubscriptionUpdate {
							id: query.id,
							update: Update::Ok((*started.value).clone()),
						});
					}

					self.keep(query.id, query.query, query.args, started.value);
				}
				Err(error) => {
					updates.push(SubscriptionUpdate {
						id: query.id,
						update: Update::Err(error),
					});
				}
			}
		}

		// The transition claims the version the resume started from, even
		// where a result includes later commits: claiming less is safe, and
		// the hub's next batches bring the rest.
		let to = to.max(self.last_sent);
		self.send_transition(from, updates, to, full);
		true
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
						"`{name}` is a {}, and a live connection sends only a mutation with \
						 `mutate`",
						registration.kind
					))),
				)
				.await;
		}

		let (result, version) = recording_commits((registration.handler)(Invocation {
			db: self.db.clone(),
			identity: self.identity.clone(),
			request_id: key,
			arguments: args,
			reads: None,
		}))
		.await;

		self.answer(Finished {
			request,
			result: result.map_err(|error| WireError::new(&error)),
			version,
		})
		.await
	}

	/// Runs an action in its own task, so a slow one doesn't hold up the
	/// connection's other frames; its answer comes back to the session.
	async fn act(&mut self, request: u32, name: &str, args: Value) -> bool {
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

		if registration.kind != Kind::Action {
			return self
				.done(
					request,
					Err(WireError::invalid(format!(
						"`{name}` is a {}, and a live connection runs only an action with `act`",
						registration.kind
					))),
				)
				.await;
		}

		let invocation = Invocation {
			db: self.db.clone(),
			identity: self.identity.clone(),
			request_id: None,
			arguments: args,
			reads: None,
		};
		let handler = registration.handler;
		let finished = self.acts.clone();
		tokio::spawn(async move {
			let (result, version) = recording_commits(handler(invocation)).await;
			let _ = finished.send(Finished {
				request,
				result: result.map_err(|error| WireError::new(&error)),
				version,
			});
		});
		true
	}

	/// Answers a mutation or an action with the version its writes
	/// committed at, and waits for a transition to reach it.
	async fn answer(&mut self, finished: Finished) -> bool {
		let version = match finished
			.version
			.and_then(|version| u64::try_from(version).ok())
		{
			Some(version) => {
				if version > self.last_sent {
					self.awaiting = Some(self.awaiting.map_or(version, |seen| seen.max(version)));
				}

				version
			}
			None => self.last_sent,
		};
		let (ok, err) = match finished.result {
			Ok(value) => (Some(value), None),
			Err(error) => (None, Some(error)),
		};

		self.send(ServerFrame::Done {
			request: finished.request,
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
		self.note_caller();
		// A stream reads as whoever opened it, so none outlives the change.
		self.interrupt_streams().await;

		// Every result is now read as somebody else: subscribe them all
		// again, under the new identity's own groups.
		let to = self.live.hub().applied();
		let ids: Vec<u32> = self.subscriptions.keys().copied().collect();
		let mut updates = Vec::new();

		for id in ids {
			let Some((query, args)) = self
				.subscriptions
				.get(&id)
				.map(|subscription| (subscription.query.clone(), subscription.args.clone()))
			else {
				continue;
			};
			self.live.hub().leave(self.number, id);

			match self.start(id, &query, &args, true).await {
				Ok(started) => {
					updates.push(SubscriptionUpdate {
						id,
						update: Update::Ok((*started.value).clone()),
					});

					if let Some(subscription) = self.subscriptions.get_mut(&id) {
						subscription.value = started.value;
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
			let to = to.max(self.last_sent);
			self.send_transition(self.last_sent, updates, to, true);
		}

		// The identity changed, so every channel is checked again.
		self.member = self
			.identity
			.caller
			.clone()
			.unwrap_or_else(|| format!("live-{}", self.number));
		self.reauthorize().await;

		self.send(ServerFrame::AuthStatus {
			refused: false,
			expiring: false,
		})
		.await
	}

	/// Records who may post frames to this connection.
	fn note_caller(&self) {
		self.caller
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.clone_from(&self.identity.caller);
	}

	/// Turns one of the hub's batches into a transition.
	fn apply(&mut self, batch: Batch) {
		let mut updates = Vec::new();

		for (id, changed) in batch.changes {
			let Some(subscription) = self.subscriptions.get_mut(&id) else {
				// Unsubscribed while the batch was on its way.
				continue;
			};
			let (update, value) = changed.into_update();

			if let Some(value) = value {
				subscription.value = value;
			}

			updates.push(SubscriptionUpdate { id, update });
		}

		let to = batch.to.max(self.last_sent);
		self.send_transition(self.last_sent, updates, to, batch.full);
	}

	/// Sends one transition to `to`, split into frames under the size
	/// limit, all at the same `to`, and repeating every value when an
	/// earlier one was dropped.
	fn send_transition(
		&mut self,
		from: u64,
		mut updates: Vec<SubscriptionUpdate>,
		to: u64,
		full: bool,
	) {
		let full = full || self.full_next;

		if self.full_next {
			updates = self
				.subscriptions
				.iter()
				.map(|(id, subscription)| {
					SubscriptionUpdate {
						id: *id,
						update: Update::Ok((*subscription.value).clone()),
					}
				})
				.collect();
		}

		self.full_next = false;

		// A frame over the limit is split, all at the same `to`: the client
		// applies the chunks in order and lands on the same state.
		let mut chunk = Vec::new();
		let mut sent = false;

		for update in updates {
			chunk.push(update);
			let transition = Transition {
				from,
				to,
				updates: chunk.clone(),
				full,
			};
			let Ok(text) = serde_json::to_string(&ServerFrame::Transition(transition)) else {
				continue;
			};

			if text.len() > MAX_FRAME_BYTES && chunk.len() > 1 {
				let last = chunk.pop().expect("just pushed");

				self.send_chunk(from, &chunk, to, full);
				sent = true;
				chunk = vec![last];
			}
		}

		if !chunk.is_empty() || !sent {
			self.send_chunk(from, &chunk, to, full);
		}

		self.last_sent = to;
		self.live
			.stats_ref()
			.transitions
			.fetch_add(1, Ordering::Relaxed);
	}

	fn send_chunk(&mut self, from: u64, updates: &[SubscriptionUpdate], to: u64, full: bool) {
		let transition = Transition {
			from,
			to,
			updates: updates.to_vec(),
			full,
		};
		let Some(outbound) = Outbound::frame(&ServerFrame::Transition(transition), Some(to)) else {
			return;
		};

		if self.out.try_send(outbound).is_err() {
			// A slow connection gets the newest state, not every step.
			self.full_next = true;
		}
	}

	/// Checks every joined channel again, for the new identity.
	async fn reauthorize(&mut self) {
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
			let Some(registration) = channel::registration(&name) else {
				self.leave(&key);
				continue;
			};
			let cx = QueryCx::new(self.db.clone(), self.identity.clone());

			match (registration.authorize)(args, cx).await {
				Ok(()) => {
					self.refresh_member(&key, state);
					self.broadcast_presence(&key);
				}
				Err(error) => {
					self.leave(&key);
					self.channel_error(&key, WireError::new(&error)).await;
				}
			}
		}
	}

	/// Records the new identity in the channel's room.
	fn refresh_member(&mut self, key: &str, state: Value) {
		let mut rooms = self.live.rooms();

		if let Some(room) = rooms.get_mut(key)
			&& let Some(member) = room.members.get_mut(&self.number)
		{
			member.id.clone_from(&self.member);
			member.state = state;
		}
	}

	async fn join(&mut self, channel: String, name: &str, args: Value, state: Value) -> bool {
		if self.joined.contains_key(&channel) {
			self.leave(&channel);
		}

		let Some(registration) = channel::registration(name) else {
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
			let senders: Vec<mpsc::Sender<Outbound>> = room
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
		let Some(outbound) = Outbound::frame(&frame, None) else {
			return;
		};

		for sender in senders {
			let _ = sender.try_send(outbound.clone());
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
			let senders: Vec<mpsc::Sender<Outbound>> = room
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
		let Some(outbound) = Outbound::frame(&frame, None) else {
			return;
		};

		for sender in senders {
			let _ = sender.try_send(outbound.clone());
		}
	}

	async fn channel_error(&self, channel: &str, err: WireError) -> bool {
		self.send(ServerFrame::ChannelError {
			channel: channel.to_owned(),
			err,
		})
		.await
	}
}
