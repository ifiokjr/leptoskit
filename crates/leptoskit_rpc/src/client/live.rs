//! The live client: one socket, the queries it watches, and the mutations
//! it runs, shared by every screen.
//!
//! ```ignore
//! // A cached query that updates itself:
//! let messages = rpc::use_live::<Messages>(move || Messages { channel, limit: 50 });
//! ```
//!
//! [`use_live`] runs the query once through the ordinary client — over HTTP
//! or in process while the server renders — and then keeps it current from
//! the socket. Results arrive as whole values or as keyed patches, and both
//! are written into the same `leptoskit_query` cache, so a screen renders a
//! live query exactly as it renders any other.
//!
//! The socket reconnects with exponential backoff and full jitter. It
//! resumes from the last version it applied, so the server sends only the
//! results that changed while it was away, and resends pending mutations
//! with the same request id, which the server's receipts make safe.
//!
//! A mutation or an action sent over the connection resolves only once the
//! connection has applied a transition at or past the version its writes
//! committed at, so the screen already shows them.
//!
//! Mutations and actions dispatched while the connection is down wait in
//! order and go out when it comes back. With
//! [`ClientConfig::offline_queue`](super::ClientConfig::offline_queue), the
//! queue is also kept on the device, so what was dispatched before the app
//! closed goes out when it next starts: mutations with the idempotency key
//! they were dispatched with, so a replay runs once, and actions only if
//! they were never sent.
//!
//! When the app's sign-in changes, [`reauthenticate`] has the connection
//! read as the new identity: a bearer token goes in an `auth` frame on the
//! open connection, and a cookie session reconnects, so every result is
//! read again.
//!
//! In the browser, when two WebSocket upgrades in a row fail, as behind
//! some proxies, the client switches to server-sent events and posts its
//! frames over HTTP: the same frames and the same session on the server,
//! so every hook works the same over either.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::OnceLock;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use any_spawner::Executor;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::channel::mpsc::UnboundedReceiver;
use futures::channel::mpsc::UnboundedSender;
use futures::channel::oneshot;
use futures::future::Either;
use futures::future::FutureExt;
use leptos::prelude::Update as _;
use leptos::prelude::*;
use leptoskit_query::BoxFuture;
use leptoskit_query::MutationHandle;
use leptoskit_query::MutationScope;
use leptoskit_query::QueryClient;
use leptoskit_query::QueryHandle;
use leptoskit_query::use_mutation;
use leptoskit_query::use_query_client;
use serde_json::Value;

use super::Args;
use super::ClientConfig;
use super::Credentials;
use super::FnError;
use super::offline::QueueStore;
use super::offline::QueuedCall;
use super::offline::QueuedKind;
use super::query_scope;
use super::use_rpc_query;
use crate::Channel;
use crate::Operation;
use crate::live::ClientFrame;
use crate::live::EndReason;
use crate::live::PresenceMember;
use crate::live::Resume;
use crate::live::ResumeQuery;
#[allow(unused_imports, reason = "only a socket build speaks the subprotocol")]
use crate::live::SUBPROTOCOL;
use crate::live::ServerFrame;
use crate::live::Transition;
use crate::live::Update;
use crate::live::WireError;
use crate::live::apply;

/// The smallest wait before reconnecting, in milliseconds.
const BACKOFF_START: u64 = 250;

/// The largest wait before reconnecting, in milliseconds.
const BACKOFF_MAX: u64 = 30_000;

/// What a connection's transport does: open a socket, send text, and read
/// the next frame.
pub(crate) trait Transport: Send + Sync + 'static {
	fn connect(&self) -> BoxFuture<'static, Result<Box<dyn Connection>, String>>;
}

/// One open socket.
pub(crate) trait Connection: Send + Sync {
	fn send(&self, text: String) -> BoxFuture<'static, Result<(), String>>;
	fn next(&self) -> BoxFuture<'static, Result<Option<String>, String>>;
}

/// A live connection shared by every hook in the app.
pub struct LiveClient {
	inner: Arc<Inner>,
}

struct Inner {
	transport: Arc<dyn Transport>,
	credentials: Option<Arc<dyn Credentials>>,
	/// The authorization header actually sent on this connection.
	authorization: Mutex<Option<String>>,
	commands: UnboundedSender<Command>,
	subscribers: Mutex<HashMap<u32, Subscriber>>,
	channels: Mutex<HashMap<String, JoinedChannel>>,
	pending: Mutex<HashMap<u32, oneshot::Sender<Result<Value, WireError>>>>,
	mutations: Mutex<HashMap<u32, PendingMutation>>,
	actions: Mutex<HashMap<u32, PendingAction>>,
	/// Mutations and actions waiting for an answer, in the order they were
	/// dispatched, which is the order they go out in.
	queue: Mutex<VecDeque<u32>>,
	streams: Mutex<HashMap<u32, OpenStream>>,
	/// Answered calls waiting for a transition to reach their version.
	waiting: Mutex<Vec<Waiting>>,
	next_id: AtomicU32,
	version: AtomicU64,
	/// Set when the app's sign-in changed, until the connection reads as
	/// the new identity.
	reauthenticate: AtomicBool,
	/// A new identity has been sent but has not yet been acknowledged.
	authenticating: AtomicBool,
	/// Where the queue is kept between runs of the app, when it is.
	offline: Option<Offline>,
	/// Whether what the device kept is back on the queue: until then,
	/// writing the queue would lose it.
	restored: AtomicBool,
	/// Whether calls wait for the connection rather than going over HTTP
	/// while it is down.
	queue_offline: bool,
	status: Mutex<LiveStatus>,
	watchers: Mutex<Vec<UnboundedSender<LiveStatus>>>,
}

/// The device's copy of the queue, and the task that writes it, in order.
struct Offline {
	store: Arc<dyn QueueStore>,
	saves: UnboundedSender<Save>,
}

/// One write of the queue, and who waits for it to land.
struct Save {
	calls: Vec<QueuedCall>,
	done: Option<oneshot::Sender<()>>,
}

struct Subscriber {
	name: String,
	args: Value,
	sender: UnboundedSender<Update>,
	/// Whether it has its first value, so a reconnect can resume it
	/// rather than subscribe again.
	ready: bool,
}

struct PendingAction {
	name: String,
	args: Value,
	/// Whether it went out on the current connection, which, if it drops,
	/// leaves the action's fate unknown.
	sent: bool,
}

struct Waiting {
	version: u64,
	sender: oneshot::Sender<Result<Value, WireError>>,
	answer: Value,
}

struct PendingMutation {
	name: String,
	args: Value,
	key: Option<String>,
	/// Whether it went out on the current connection.
	sent: bool,
}

struct JoinedChannel {
	name: String,
	args: Value,
	state: Value,
	sender: UnboundedSender<ChannelEvent>,
}

/// A stream the app holds open.
struct OpenStream {
	name: String,
	args: Value,
	resumable: bool,
	/// How many items the app may hold before it has taken some.
	window: u32,
	/// Where a resumable stream reached: its last item's cursor.
	cursor: Option<Value>,
	/// Whether it went out on the current connection.
	opened: bool,
	counts: Arc<StreamCounts>,
	sender: UnboundedSender<StreamEvent>,
}

/// How many of a stream's items the app holds, and has taken since the
/// last credit.
#[derive(Default)]
struct StreamCounts {
	buffered: AtomicU32,
	taken: AtomicU32,
}

/// What happened in a channel.
#[derive(Clone, Debug, PartialEq)]
pub enum ChannelEvent {
	/// Someone else sent a message.
	Signal {
		/// Who sent it, when the server can say.
		from: Option<String>,
		/// The message.
		message: Value,
	},
	/// Who is in the channel now.
	Presence(Vec<PresenceMember>),
	/// The server refused the join, or the identity lost access.
	Error(WireError),
}

/// What happened on a stream.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamEvent {
	/// The next item.
	Item(Value),
	/// It ended, and nothing more comes.
	End(EndReason),
}

/// Where the live connection is, for a connection banner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveState {
	/// Opening the first connection.
	Connecting,
	/// Connected, with every result at least as new as `version`.
	Live {
		/// The last version the connection applied.
		version: u64,
	},
	/// The connection dropped: trying again, for the `attempt`th time, in
	/// `next_in`.
	Reconnecting {
		/// How many attempts have failed in a row.
		attempt: u32,
		/// How long until the next one.
		next_in: Duration,
	},
	/// This build holds no live connection, as while the server renders.
	Offline,
}

/// The live connection and what waits for it, as [`use_live_status`]
/// reports them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveStatus {
	/// Where the connection is.
	pub state: LiveState,
	/// How many mutations and actions wait for an answer, including ones
	/// dispatched while the connection was down.
	pub queued: usize,
}

impl LiveStatus {
	/// No connection, and nothing waiting.
	pub const OFFLINE: Self = Self {
		state: LiveState::Offline,
		queued: 0,
	};

	/// Whether the connection is open.
	#[must_use]
	pub fn is_live(&self) -> bool {
		matches!(self.state, LiveState::Live { .. })
	}
}

enum Command {
	Subscribe(u32),
	Unsubscribe(u32),
	Mutate(u32),
	Act(u32),
	Join(String),
	Leave(String),
	Signal(String, Value),
	Stream(u32),
	Credit(u32, u32),
	Cancel(u32),
	Reauthenticate,
	RefreshCredentials { refused: bool },
	Reconnect,
}

impl LiveClient {
	/// A client over `transport`, connected to `config`'s server.
	pub(crate) fn new(config: &ClientConfig, transport: Arc<dyn Transport>) -> Self {
		let (commands, receiver) = mpsc::unbounded();
		let store = config.offline_store.clone().or_else(|| {
			config
				.offline_queue
				.then(super::offline::platform_store)
				.flatten()
		});
		let offline = store.map(|store| {
			let (saves, pending) = mpsc::unbounded();
			Executor::spawn_local(save_queue(Arc::clone(&store), pending));
			Offline { store, saves }
		});
		let inner = Arc::new(Inner {
			transport,
			credentials: config.credentials.clone(),
			authorization: Mutex::new(None),
			commands,
			subscribers: Mutex::new(HashMap::new()),
			channels: Mutex::new(HashMap::new()),
			pending: Mutex::new(HashMap::new()),
			mutations: Mutex::new(HashMap::new()),
			actions: Mutex::new(HashMap::new()),
			queue: Mutex::new(VecDeque::new()),
			streams: Mutex::new(HashMap::new()),
			waiting: Mutex::new(Vec::new()),
			next_id: AtomicU32::new(1),
			version: AtomicU64::new(0),
			reauthenticate: AtomicBool::new(false),
			authenticating: AtomicBool::new(false),
			queue_offline: config.offline_queue || offline.is_some(),
			restored: AtomicBool::new(offline.is_none()),
			offline,
			status: Mutex::new(LiveStatus {
				state: LiveState::Connecting,
				queued: 0,
			}),
			watchers: Mutex::new(Vec::new()),
		});
		Executor::spawn_local(run(Arc::clone(&inner), receiver));
		Self { inner }
	}

	/// The version of the last transition the connection applied: every
	/// result it holds includes every commit up to it.
	#[must_use]
	pub fn version(&self) -> u64 {
		self.inner.version.load(Ordering::Relaxed)
	}

	/// Where the connection is, and how much waits for it.
	#[must_use]
	pub fn status(&self) -> LiveStatus {
		*self.inner.status()
	}

	/// Each change of [`status`](Self::status), from now on.
	#[must_use]
	pub fn watch_status(&self) -> UnboundedReceiver<LiveStatus> {
		let (sender, receiver) = mpsc::unbounded();
		self.inner.watchers().push(sender);
		receiver
	}

	/// Whether a mutation sent now goes over this connection: it is open,
	/// or the app asked for calls to wait for it.
	#[must_use]
	pub fn carries_calls(&self) -> bool {
		self.inner.queue_offline || self.status().is_live()
	}

	/// Has the connection read as whoever the app's credentials now say,
	/// after a sign-in, a sign-out, or a renewed token: a bearer token goes
	/// in an `auth` frame, and a cookie session reconnects. Every
	/// subscription and channel is checked again as the new identity, and
	/// a stream opened as the old one ends as interrupted.
	pub fn reauthenticate(&self) {
		self.inner.reauthenticate.store(true, Ordering::SeqCst);
		let _ = self.inner.commands.unbounded_send(Command::Reauthenticate);
	}

	/// Watches `query` with `args`, starting now. The returned receiver
	/// yields each change; call [`unsubscribe`](Self::unsubscribe) to stop.
	pub fn subscribe(&self, query: &'static str, args: Value) -> (u32, UnboundedReceiver<Update>) {
		let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
		let (sender, receiver) = mpsc::unbounded();
		self.inner.subscribers().insert(
			id,
			Subscriber {
				name: query.to_owned(),
				args,
				sender,
				ready: false,
			},
		);
		let _ = self.inner.commands.unbounded_send(Command::Subscribe(id));
		(id, receiver)
	}

	/// Stops watching `id`.
	pub fn unsubscribe(&self, id: u32) {
		self.inner.subscribers().remove(&id);
		let _ = self.inner.commands.unbounded_send(Command::Unsubscribe(id));
	}

	/// Opens the stream `name` with `args`, letting the app hold up to
	/// `window` items it hasn't taken yet: the server sends no more until
	/// the app takes some. A resumable stream opens again from its last
	/// item's cursor when the connection comes back; any other ends as
	/// interrupted. Dropping the returned stream cancels it.
	pub fn stream(&self, name: &str, args: Value, window: u32, resumable: bool) -> LiveStream {
		let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
		let (sender, receiver) = mpsc::unbounded();
		let counts = Arc::new(StreamCounts::default());
		let window = window.max(1);
		self.inner.streams().insert(
			id,
			OpenStream {
				name: name.to_owned(),
				args,
				resumable,
				window,
				cursor: None,
				opened: false,
				counts: Arc::clone(&counts),
				sender,
			},
		);
		let _ = self.inner.commands.unbounded_send(Command::Stream(id));
		LiveStream {
			id,
			window,
			receiver,
			counts,
			commands: self.inner.commands.clone(),
			inner: Arc::downgrade(&self.inner),
			ended: false,
		}
	}

	/// Joins a channel, with the presence this connection shows.
	///
	/// The returned key names the join instance; the receiver yields who is
	/// there and what they send. Call [`leave`](Self::leave) to stop.
	#[expect(
		clippy::needless_pass_by_value,
		reason = "a join reads `join(Chat { channel }, …)`: the arguments are built for it"
	)]
	pub fn join<C: Channel>(
		&self,
		args: C,
		state: Option<C::Presence>,
	) -> (String, UnboundedReceiver<ChannelEvent>) {
		let key = serde_json::to_string(&args).expect("a channel's arguments serialize");
		let (sender, receiver) = mpsc::unbounded();
		let state = serde_json::to_value(state.unwrap_or_default()).unwrap_or(Value::Null);
		self.inner.channels().insert(
			key.clone(),
			JoinedChannel {
				name: C::NAME.to_owned(),
				args: serde_json::to_value(&args).unwrap_or(Value::Null),
				state,
				sender,
			},
		);
		let _ = self
			.inner
			.commands
			.unbounded_send(Command::Join(key.clone()));
		(key, receiver)
	}

	/// Leaves the channel instance `key`.
	pub fn leave(&self, key: &str) {
		self.inner.channels().remove(key);
		let _ = self
			.inner
			.commands
			.unbounded_send(Command::Leave(key.to_owned()));
	}

	/// Sends `message` to everyone else in the channel instance `key`.
	pub fn signal<C: Channel>(&self, key: &str, message: &C::Message) {
		let Ok(message) = serde_json::to_value(message) else {
			return;
		};
		let _ = self
			.inner
			.commands
			.unbounded_send(Command::Signal(key.to_owned(), message));
	}

	/// Runs a mutation over the socket, with its idempotency key. It
	/// resolves once the connection has applied the transition that
	/// includes its writes, so every live query on screen already shows
	/// them. While the connection is down it waits, in order, and goes out
	/// when it comes back.
	///
	/// # Errors
	///
	/// The operation's failure, classified like any other call's.
	pub fn mutate(
		&self,
		name: &'static str,
		args: Value,
		key: Option<String>,
	) -> impl Future<Output = Result<Value, FnError>> + 'static {
		let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
		let (sender, receiver) = oneshot::channel();
		self.inner.pending().insert(id, sender);
		self.inner.mutations().insert(
			id,
			PendingMutation {
				name: name.to_owned(),
				args,
				key,
				sent: false,
			},
		);
		self.inner.enqueue(id);
		let _ = self.inner.commands.unbounded_send(Command::Mutate(id));
		answer(receiver)
	}

	/// Runs an action over the connection. Like [`mutate`](Self::mutate),
	/// it resolves once the writes of the mutations it ran are on screen.
	/// An action is never sent twice: when the connection drops before it
	/// answers, it fails with [`FnError::Network`], and may or may not have
	/// run.
	///
	/// # Errors
	///
	/// The operation's failure, classified like any other call's.
	pub fn act(
		&self,
		name: &'static str,
		args: Value,
	) -> impl Future<Output = Result<Value, FnError>> + 'static {
		let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
		let (sender, receiver) = oneshot::channel();
		self.inner.pending().insert(id, sender);
		self.inner.actions().insert(
			id,
			PendingAction {
				name: name.to_owned(),
				args,
				sent: false,
			},
		);
		self.inner.enqueue(id);
		let _ = self.inner.commands.unbounded_send(Command::Act(id));
		answer(receiver)
	}
}

/// A stream's items as they arrive, granting the server room for more as
/// they are taken. Dropping it cancels the stream.
pub struct LiveStream {
	id: u32,
	window: u32,
	receiver: UnboundedReceiver<StreamEvent>,
	counts: Arc<StreamCounts>,
	commands: UnboundedSender<Command>,
	inner: std::sync::Weak<Inner>,
	ended: bool,
}

impl LiveStream {
	/// The stream's id on the connection.
	#[must_use]
	pub fn id(&self) -> u32 {
		self.id
	}
}

impl futures::Stream for LiveStream {
	type Item = StreamEvent;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<StreamEvent>> {
		let polled = self.receiver.poll_next_unpin(cx);

		match &polled {
			Poll::Ready(Some(StreamEvent::Item(_))) => {
				self.counts.buffered.fetch_sub(1, Ordering::Relaxed);
				let taken = self.counts.taken.fetch_add(1, Ordering::Relaxed) + 1;

				// Room for more once half the window is taken, in one frame.
				if taken >= (self.window / 2).max(1) {
					let taken = self.counts.taken.swap(0, Ordering::Relaxed);

					if taken > 0 {
						let _ = self
							.commands
							.unbounded_send(Command::Credit(self.id, taken));
					}
				}
			}
			Poll::Ready(Some(StreamEvent::End(_)) | None) => self.ended = true,
			Poll::Pending => {}
		}

		polled
	}
}

impl Drop for LiveStream {
	fn drop(&mut self) {
		if self.ended {
			return;
		}

		if let Some(inner) = self.inner.upgrade() {
			inner.streams().remove(&self.id);
		}

		let _ = self.commands.unbounded_send(Command::Cancel(self.id));
	}
}

/// A call's answer, classified like any other call's.
async fn answer(receiver: oneshot::Receiver<Result<Value, WireError>>) -> Result<Value, FnError> {
	match receiver.await {
		Ok(Ok(value)) => Ok(value),
		Ok(Err(error)) => Err(fn_error(error)),
		Err(_) => Err(FnError::Network("the live connection closed".to_owned())),
	}
}

/// A frame's failure, classified like any other call's.
pub(crate) fn fn_error(error: WireError) -> FnError {
	match error.status {
		0 => FnError::Network(error.message),
		400 => FnError::Invalid(error.message),
		_ => FnError::Server(error.message),
	}
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
	mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Inner {
	fn subscribers(&self) -> MutexGuard<'_, HashMap<u32, Subscriber>> {
		lock(&self.subscribers)
	}

	fn pending(&self) -> MutexGuard<'_, HashMap<u32, oneshot::Sender<Result<Value, WireError>>>> {
		lock(&self.pending)
	}

	fn mutations(&self) -> MutexGuard<'_, HashMap<u32, PendingMutation>> {
		lock(&self.mutations)
	}

	fn channels(&self) -> MutexGuard<'_, HashMap<String, JoinedChannel>> {
		lock(&self.channels)
	}

	fn actions(&self) -> MutexGuard<'_, HashMap<u32, PendingAction>> {
		lock(&self.actions)
	}

	fn queue(&self) -> MutexGuard<'_, VecDeque<u32>> {
		lock(&self.queue)
	}

	fn streams(&self) -> MutexGuard<'_, HashMap<u32, OpenStream>> {
		lock(&self.streams)
	}

	fn waiting(&self) -> MutexGuard<'_, Vec<Waiting>> {
		lock(&self.waiting)
	}

	fn status(&self) -> MutexGuard<'_, LiveStatus> {
		lock(&self.status)
	}

	fn watchers(&self) -> MutexGuard<'_, Vec<UnboundedSender<LiveStatus>>> {
		lock(&self.watchers)
	}

	/// Changes the status, and tells whoever watches it.
	fn update_status(&self, change: impl FnOnce(&mut LiveStatus)) {
		let status = {
			let mut status = self.status();
			let before = *status;
			change(&mut status);

			if *status == before {
				return;
			}

			*status
		};

		self.watchers()
			.retain(|watcher| watcher.unbounded_send(status).is_ok());
	}

	fn set_state(&self, state: LiveState) {
		self.update_status(|status| status.state = state);
	}

	/// Puts a dispatched call at the back of the queue.
	fn enqueue(&self, id: u32) {
		self.queue().push_back(id);
		self.queue_changed(None);
	}

	/// Takes a call off the queue, once it has answered or given up.
	fn dequeue(&self, id: u32) {
		let removed = self.mutations().remove(&id).is_some() | self.actions().remove(&id).is_some();

		if removed {
			self.queue().retain(|queued| *queued != id);
			self.queue_changed(None);
		}
	}

	/// Counts the queue for the status, and writes it to the device, with
	/// `done` told once that write has landed.
	fn queue_changed(&self, done: Option<oneshot::Sender<()>>) {
		let queued = self.queue().len();
		self.update_status(|status| status.queued = queued);

		let offline = self
			.offline
			.as_ref()
			.filter(|_| self.restored.load(Ordering::SeqCst));
		let Some(offline) = offline else {
			if let Some(done) = done {
				let _ = done.send(());
			}

			return;
		};
		let calls = self.queued_calls();
		let _ = offline.saves.unbounded_send(Save { calls, done });
	}

	/// What the device keeps: every mutation still waiting, which its
	/// receipt makes safe to send again, and every action never sent.
	fn queued_calls(&self) -> Vec<QueuedCall> {
		let queue: Vec<u32> = self.queue().iter().copied().collect();
		let mutations = self.mutations();
		let actions = self.actions();

		queue
			.iter()
			.filter_map(|id| {
				if let Some(mutation) = mutations.get(id) {
					return Some(QueuedCall {
						kind: QueuedKind::Mutation,
						name: mutation.name.clone(),
						args: mutation.args.clone(),
						key: mutation.key.clone(),
					});
				}

				actions.get(id).filter(|action| !action.sent).map(|action| {
					QueuedCall {
						kind: QueuedKind::Action,
						name: action.name.clone(),
						args: action.args.clone(),
						key: None,
					}
				})
			})
			.collect()
	}

	/// Puts what the device kept back on the queue, ahead of anything
	/// dispatched since the app started. Nobody waits for their answers.
	fn restore(&self, calls: Vec<QueuedCall>) {
		let mut restored = Vec::new();

		for call in calls {
			let id = self.next_id.fetch_add(1, Ordering::Relaxed);

			match call.kind {
				QueuedKind::Mutation => {
					self.mutations().insert(
						id,
						PendingMutation {
							name: call.name,
							args: call.args,
							key: call.key,
							sent: false,
						},
					);
				}
				QueuedKind::Action => {
					self.actions().insert(
						id,
						PendingAction {
							name: call.name,
							args: call.args,
							sent: false,
						},
					);
				}
			}

			restored.push(id);
		}

		{
			let mut queue = self.queue();

			for id in restored.into_iter().rev() {
				queue.push_front(id);
			}
		}

		// The queue may now be written: it holds both what was kept and
		// what was dispatched since.
		self.restored.store(true, Ordering::SeqCst);
		self.queue_changed(None);
	}

	/// Forgets every result, so each subscription takes a fresh first
	/// value rather than resuming: they were read as somebody else.
	fn forget_results(&self) {
		for subscriber in self.subscribers().values_mut() {
			subscriber.ready = false;
		}
	}

	/// What a dropped connection leaves: the actions it carried fail, the
	/// mutations go out again on the next one, and the streams that can't
	/// resume end.
	fn disconnected(&self) {
		self.drop_actions();

		for mutation in self.mutations().values_mut() {
			mutation.sent = false;
		}

		let ended: Vec<OpenStream> = {
			let mut streams = self.streams();

			for stream in streams.values_mut() {
				stream.opened = false;
			}

			let ids: Vec<u32> = streams
				.iter()
				.filter(|(_, stream)| !stream.resumable)
				.map(|(id, _)| *id)
				.collect();
			ids.iter().filter_map(|id| streams.remove(id)).collect()
		};

		for stream in ended {
			let _ = stream
				.sender
				.unbounded_send(StreamEvent::End(EndReason::Interrupted));
		}
	}

	/// Fails every action the dropped connection carried: whether it ran
	/// is unknown, and running it again isn't safe.
	fn drop_actions(&self) {
		let sent: Vec<u32> = self
			.actions()
			.iter()
			.filter(|(_, action)| action.sent)
			.map(|(id, _)| *id)
			.collect();

		for id in sent {
			self.dequeue(id);

			if let Some(sender) = self.pending().remove(&id) {
				let _ = sender.send(Err(WireError {
					status: 0,
					message: "the live connection dropped before the action answered; it may or \
					          may not have run"
						.to_owned(),
				}));
			}
		}
	}
}

/// Writes each change of the queue to the device, in order, skipping to
/// the newest when several wait.
async fn save_queue(store: Arc<dyn QueueStore>, mut saves: UnboundedReceiver<Save>) {
	while let Some(mut save) = saves.next().await {
		let mut done = Vec::new();

		while let Ok(newer) = saves.try_recv() {
			done.extend(save.done.take());
			save = newer;
		}

		done.extend(save.done.take());
		store.save(save.calls).await;

		for done in done {
			let _ = done.send(());
		}
	}
}

async fn send(connection: &dyn Connection, frame: &ClientFrame) -> bool {
	let Ok(text) = serde_json::to_string(frame) else {
		return false;
	};
	connection.send(text).await.is_ok()
}

/// What re-authenticating an open connection did.
enum Reauthenticated {
	/// Nothing needed, or the token went out on it.
	InPlace,
	/// The identity is a cookie's, or nobody's: connect again.
	Reconnect,
	/// The connection closed.
	Closed,
}

async fn run(inner: Arc<Inner>, mut commands: UnboundedReceiver<Command>) {
	if let Some(offline) = &inner.offline {
		let calls = offline.store.load().await;
		inner.restore(calls);
	}

	let mut backoff = BACKOFF_START;
	let mut attempt = 0_u32;

	loop {
		let Ok(connection) = inner.transport.connect().await else {
			attempt = attempt.saturating_add(1);
			inner.set_state(LiveState::Reconnecting {
				attempt,
				next_in: Duration::from_millis(backoff),
			});
			leptoskit_query::sleep(Duration::from_millis(backoff)).await;
			backoff = (backoff * 2).min(BACKOFF_MAX);
			continue;
		};
		backoff = BACKOFF_START;

		if !handshake(inner.as_ref(), connection.as_ref()).await {
			continue;
		}

		attempt = 0;
		inner.set_state(LiveState::Live {
			version: inner.version.load(Ordering::Relaxed),
		});
		let mut at_once = false;

		loop {
			let command = commands.next().fuse();
			let frame = connection.next().fuse();
			futures::pin_mut!(command, frame);

			match futures::future::select(command, frame).await {
				Either::Left((Some(Command::Reconnect) | None, _)) => break,
				Either::Left((Some(Command::RefreshCredentials { refused }), _)) => {
					let sent = lock(&inner.authorization).clone();
					let renewed = match &inner.credentials {
						Some(credentials) => credentials.refused(sent).await.unwrap_or(false),
						None => false,
					};
					if renewed {
						inner.reauthenticate.store(true, Ordering::SeqCst);
						if !matches!(
							read_as_new_identity(inner.as_ref(), connection.as_ref()).await,
							Reauthenticated::InPlace
						) {
							break;
						}
					} else if refused {
						break;
					}
				}
				Either::Left((Some(Command::Reauthenticate), _)) => {
					match read_as_new_identity(inner.as_ref(), connection.as_ref()).await {
						Reauthenticated::InPlace => {}
						Reauthenticated::Reconnect => {
							at_once = true;
							break;
						}
						Reauthenticated::Closed => break,
					}
				}
				Either::Left((Some(command), _)) => {
					if !send_command(inner.as_ref(), connection.as_ref(), command).await {
						break;
					}
				}
				Either::Right((Ok(Some(text)), _)) => dispatch(inner.as_ref(), &text),
				Either::Right((..)) => break,
			}
		}

		inner.disconnected();

		if at_once {
			inner.set_state(LiveState::Connecting);
			continue;
		}

		attempt = attempt.saturating_add(1);
		inner.set_state(LiveState::Reconnecting {
			attempt,
			next_in: Duration::from_millis(backoff),
		});
		leptoskit_query::sleep(Duration::from_millis(backoff)).await;
		backoff = (backoff * 2).min(BACKOFF_MAX);
	}
}

/// Reads as the identity the credentials now hold: a bearer token in an
/// `auth` frame on this connection, or a fresh connection for anything
/// else.
async fn read_as_new_identity(inner: &Inner, connection: &dyn Connection) -> Reauthenticated {
	if !inner.reauthenticate.swap(false, Ordering::SeqCst) {
		// A reconnect already read as the new identity.
		return Reauthenticated::InPlace;
	}

	inner.authenticating.store(true, Ordering::SeqCst);
	inner.forget_results();

	if let Some(token) = token(inner).await {
		if send(connection, &ClientFrame::Auth { token }).await {
			Reauthenticated::InPlace
		} else {
			Reauthenticated::Closed
		}
	} else {
		inner.forget_results();
		Reauthenticated::Reconnect
	}
}

/// Authenticates, says hello with what the client already holds, and
/// restores the rest.
///
/// The token goes first, so the queries resume as the identity whose
/// results the client holds. When the sign-in changed while the client was
/// away, nothing resumes: every query takes a fresh first value.
async fn handshake(inner: &Inner, connection: &dyn Connection) -> bool {
	let changed = inner.reauthenticate.swap(false, Ordering::SeqCst);
	let unfinished = inner.authenticating.swap(false, Ordering::SeqCst);
	if changed || unfinished {
		inner.forget_results();
	}

	if let Some(token) = token(inner).await
		&& !send(connection, &ClientFrame::Auth { token }).await
	{
		return false;
	}

	let queries: Vec<ResumeQuery> = inner
		.subscribers()
		.iter()
		.filter(|(_, subscriber)| subscriber.ready)
		.map(|(&id, subscriber)| {
			ResumeQuery {
				id,
				query: subscriber.name.clone(),
				args: subscriber.args.clone(),
			}
		})
		.collect();
	let hello = ClientFrame::Hello {
		resume: Some(Resume {
			version: inner.version.load(Ordering::Relaxed),
			queries,
		}),
	};

	if !send(connection, &hello).await {
		return false;
	}

	resubscribe(inner, connection).await;
	resend_calls(inner, connection).await;
	reopen_streams(inner, connection).await;
	rejoin(inner, connection).await;
	true
}

/// Sends one queued command, saying whether the connection is still open.
async fn send_command(inner: &Inner, connection: &dyn Connection, command: Command) -> bool {
	let frame = match command {
		Command::Subscribe(id) => {
			match inner.subscribers().get(&id) {
				Some(subscriber) => {
					ClientFrame::Sub {
						id,
						query: subscriber.name.clone(),
						args: subscriber.args.clone(),
					}
				}
				None => return true,
			}
		}
		Command::Unsubscribe(id) => ClientFrame::Unsub { id },
		Command::Mutate(id) => {
			match mutation_frame(inner, id) {
				Some(frame) => frame,
				None => return true,
			}
		}
		Command::Join(key) => {
			match inner.channels().get(&key) {
				Some(channel) => {
					ClientFrame::Join {
						channel: key.clone(),
						name: channel.name.clone(),
						args: channel.args.clone(),
						state: channel.state.clone(),
					}
				}
				None => return true,
			}
		}
		Command::Act(id) => {
			match action_frame(inner, id).await {
				Some(frame) => frame,
				None => return true,
			}
		}
		Command::Leave(key) => ClientFrame::Leave { channel: key },
		Command::Signal(key, message) => {
			ClientFrame::Signal {
				channel: key,
				message,
			}
		}
		Command::Stream(id) => {
			match stream_frame(inner, id) {
				Some(frame) => frame,
				None => return true,
			}
		}
		Command::Credit(id, n) => {
			if !inner.streams().contains_key(&id) {
				return true;
			}

			ClientFrame::Credit { stream: id, n }
		}
		Command::Cancel(id) => ClientFrame::Cancel { stream: id },
		Command::Reauthenticate | Command::RefreshCredentials { .. } => return true,
		Command::Reconnect => return false,
	};

	send(connection, &frame).await
}

/// A mutation's frame, the first time it goes out on this connection.
fn mutation_frame(inner: &Inner, id: u32) -> Option<ClientFrame> {
	let mut mutations = inner.mutations();
	let mutation = mutations.get_mut(&id).filter(|mutation| !mutation.sent)?;
	mutation.sent = true;
	Some(ClientFrame::Mutate {
		request: id,
		name: mutation.name.clone(),
		args: mutation.args.clone(),
		key: mutation.key.clone(),
	})
}

/// An action's frame, the only time it goes out. The device forgets it
/// first, so an app that stops while it is in flight never sends it again.
async fn action_frame(inner: &Inner, id: u32) -> Option<ClientFrame> {
	let frame = {
		let mut actions = inner.actions();
		let action = actions.get_mut(&id).filter(|action| !action.sent)?;
		action.sent = true;
		ClientFrame::Act {
			request: id,
			name: action.name.clone(),
			args: action.args.clone(),
		}
	};

	if inner.offline.is_some() {
		let (done, landed) = oneshot::channel();
		inner.queue_changed(Some(done));
		let _ = landed.await;
	}

	Some(frame)
}

/// A stream's opening frame, the first time it goes out on this
/// connection: from its last cursor when it resumes, with room for what the
/// app isn't holding.
fn stream_frame(inner: &Inner, id: u32) -> Option<ClientFrame> {
	let mut streams = inner.streams();
	let stream = streams.get_mut(&id).filter(|stream| !stream.opened)?;
	stream.opened = true;
	let mut args = stream.args.clone();

	if let (Some(cursor), Value::Object(fields)) = (&stream.cursor, &mut args) {
		fields.insert("cursor".to_owned(), cursor.clone());
	}

	stream.counts.taken.store(0, Ordering::Relaxed);
	let held = stream.counts.buffered.load(Ordering::Relaxed);
	Some(ClientFrame::Stream {
		stream: id,
		name: stream.name.clone(),
		args,
		credits: stream.window.saturating_sub(held).max(1),
	})
}

fn dispatch(inner: &Inner, text: &str) {
	let Ok(frame) = serde_json::from_str::<ServerFrame>(text) else {
		return;
	};

	match frame {
		ServerFrame::Transition(transition) => apply_transition(inner, transition),
		ServerFrame::Done {
			request,
			ok,
			err,
			version,
		} => finish(inner, request, ok, err, version),
		ServerFrame::AuthStatus { refused, expiring } => {
			if !refused && !expiring {
				inner.authenticating.store(false, Ordering::SeqCst);
			}
			refuse(inner, refused);
			if refused || expiring {
				let _ = inner
					.commands
					.unbounded_send(Command::RefreshCredentials { refused });
			}
		}
		ServerFrame::Reconnect { .. } => {
			let _ = inner.commands.unbounded_send(Command::Reconnect);
		}
		ServerFrame::Signal {
			channel,
			from,
			message,
		} => {
			send_channel(inner, &channel, ChannelEvent::Signal { from, message });
		}
		ServerFrame::Presence { channel, members } => {
			send_channel(inner, &channel, ChannelEvent::Presence(members));
		}
		ServerFrame::ChannelError { channel, err } => {
			send_channel(inner, &channel, ChannelEvent::Error(err));
		}
		ServerFrame::Item {
			stream,
			value,
			cursor,
		} => {
			let mut streams = inner.streams();

			if let Some(open) = streams.get_mut(&stream) {
				if cursor.is_some() {
					open.cursor = cursor;
				}

				open.counts.buffered.fetch_add(1, Ordering::Relaxed);
				let _ = open.sender.unbounded_send(StreamEvent::Item(value));
			}
		}
		ServerFrame::End {
			stream,
			reason,
			err,
		} => end_stream(inner, stream, EndReason::from_frame(&reason, err)),
		// The welcome's version is the server's, not this client's: its
		// results reach a version only through a transition.
		ServerFrame::Welcome { .. } | ServerFrame::Pong | ServerFrame::Unknown => {}
	}
}

/// A stream ended. One the server interrupted on an open connection, as
/// when it changed who it reads as, opens again if it can resume.
fn end_stream(inner: &Inner, stream: u32, reason: EndReason) {
	let mut streams = inner.streams();

	if reason == EndReason::Interrupted
		&& let Some(open) = streams.get_mut(&stream).filter(|open| open.resumable)
	{
		open.opened = false;
		let _ = inner.commands.unbounded_send(Command::Stream(stream));
		return;
	}

	if let Some(open) = streams.remove(&stream) {
		let _ = open.sender.unbounded_send(StreamEvent::End(reason));
	}
}

/// Hands each changed result to the subscriber watching for it, then
/// resolves the calls whose writes the transition reaches.
fn apply_transition(inner: &Inner, transition: Transition) {
	{
		let mut subscribers = inner.subscribers();

		for update in transition.updates {
			if let Some(subscriber) = subscribers.get_mut(&update.id) {
				if matches!(update.update, Update::Ok(_)) {
					subscriber.ready = true;
				}

				let _ = subscriber.sender.unbounded_send(update.update);
			}
		}
	}

	inner.version.store(transition.to, Ordering::Relaxed);
	inner.update_status(|status| {
		if let LiveState::Live { version } = &mut status.state {
			*version = transition.to;
		}
	});
	let reached: Vec<Waiting> = {
		let mut waiting = inner.waiting();
		let (reached, still): (Vec<Waiting>, Vec<Waiting>) = waiting
			.drain(..)
			.partition(|call| call.version <= transition.to);
		*waiting = still;
		reached
	};

	for call in reached {
		let _ = call.sender.send(Ok(call.answer));
	}
}

/// Answers a finished call once the client holds its writes, or sends a
/// subscription's first value.
fn finish(inner: &Inner, request: u32, ok: Option<Value>, err: Option<WireError>, version: u64) {
	let was_call =
		inner.mutations().contains_key(&request) || inner.actions().contains_key(&request);
	inner.dequeue(request);

	let Some(sender) = inner.pending().remove(&request) else {
		if !was_call
			&& let Some(subscriber) = inner.subscribers().get_mut(&request)
			&& let Some(value) = ok
		{
			// A subscription's first value.
			subscriber.ready = true;
			let _ = subscriber.sender.unbounded_send(Update::Ok(value));
		}

		return;
	};

	let answer = match err {
		// A failed call wrote nothing.
		Some(error) => Err(error),
		None => Ok(ok.unwrap_or(Value::Null)),
	};

	match answer {
		Ok(answer) if inner.version.load(Ordering::Relaxed) < version => {
			inner.waiting().push(Waiting {
				version,
				sender,
				answer,
			});
		}
		answer => {
			let _ = sender.send(answer);
		}
	}
}

/// Refused credentials make every result an error until the app signs in
/// again.
fn refuse(inner: &Inner, refused: bool) {
	if !refused {
		return;
	}

	inner.forget_results();
	let error = WireError {
		status: 401,
		message: "sign in again to watch this".to_owned(),
	};
	inner.mutations().clear();
	inner.actions().clear();
	inner.queue().clear();
	inner.queue_changed(None);
	for (_, pending) in inner.pending().drain() {
		let _ = pending.send(Err(error.clone()));
	}
	for waiting in inner.waiting().drain(..) {
		let _ = waiting.sender.send(Err(error.clone()));
	}
	let subscribers = inner.subscribers();

	for subscriber in subscribers.values() {
		let _ = subscriber.sender.unbounded_send(Update::Err(WireError {
			status: 401,
			message: "sign in again to watch this".to_owned(),
		}));
	}
}

/// Sends a channel event to the app watching that channel, if any.
fn send_channel(inner: &Inner, channel: &str, event: ChannelEvent) {
	if let Some(joined) = inner.channels().get(channel) {
		let _ = joined.sender.unbounded_send(event);
	}
}

/// Subscribes whatever hasn't had its first value: the rest resumed in
/// the hello.
async fn resubscribe(inner: &Inner, connection: &dyn Connection) {
	let frames: Vec<ClientFrame> = inner
		.subscribers()
		.iter()
		.filter(|(_, subscriber)| !subscriber.ready)
		.map(|(&id, subscriber)| {
			ClientFrame::Sub {
				id,
				query: subscriber.name.clone(),
				args: subscriber.args.clone(),
			}
		})
		.collect();

	for frame in frames {
		if !send(connection, &frame).await {
			return;
		}
	}
}

/// Sends every waiting mutation, and every action not yet sent, in the
/// order they were dispatched.
async fn resend_calls(inner: &Inner, connection: &dyn Connection) {
	let queue: Vec<u32> = inner.queue().iter().copied().collect();

	for id in queue {
		let frame = match mutation_frame(inner, id) {
			Some(frame) => Some(frame),
			None => action_frame(inner, id).await,
		};

		if let Some(frame) = frame
			&& !send(connection, &frame).await
		{
			return;
		}
	}
}

/// Opens the resumable streams again, each from its last cursor.
async fn reopen_streams(inner: &Inner, connection: &dyn Connection) {
	let ids: Vec<u32> = inner.streams().keys().copied().collect();

	for id in ids {
		if let Some(frame) = stream_frame(inner, id)
			&& !send(connection, &frame).await
		{
			return;
		}
	}
}

async fn rejoin(inner: &Inner, connection: &dyn Connection) {
	let frames: Vec<ClientFrame> = inner
		.channels()
		.iter()
		.map(|(key, channel)| {
			ClientFrame::Join {
				channel: key.clone(),
				name: channel.name.clone(),
				args: channel.args.clone(),
				state: channel.state.clone(),
			}
		})
		.collect();

	for frame in frames {
		if !send(connection, &frame).await {
			return;
		}
	}
}

async fn token(inner: &Inner) -> Option<String> {
	let credentials = inner.credentials.as_ref()?;
	let header = credentials.authorization().await.ok().flatten()?;
	*lock(&inner.authorization) = Some(header.clone());
	header.strip_prefix("Bearer ").map(str::to_owned)
}

static CLIENT: OnceLock<LiveClient> = OnceLock::new();

/// Starts the app's live client from the configuration
/// [`configure`](super::configure) was given. Does nothing when it is
/// already running, or when this platform has no socket.
pub(crate) fn install(config: &ClientConfig) {
	if CLIENT.get().is_some() {
		return;
	}

	if let Some(transport) = transport(config) {
		let _ = CLIENT.set(LiveClient::new(config, transport));
	}
}

/// The app's live client.
///
/// # Panics
///
/// If the client was never configured, or this platform has no socket.
#[must_use]
pub fn client() -> &'static LiveClient {
	CLIENT
		.get()
		.expect("call rpc::client::configure before using a live query")
}

/// The app's live client, when it has one.
pub(crate) fn client_if_any() -> Option<&'static LiveClient> {
	CLIENT.get()
}

/// Whether this build can hold a live connection.
#[must_use]
pub fn available() -> bool {
	CLIENT.get().is_some()
}

/// Makes `client` the app's live client, as [`configure`](super::configure)
/// does on a platform with a socket: how a native test drives the hooks
/// against a real server. Returns `false` when one is already installed.
#[cfg(feature = "live")]
pub(crate) fn install_client(client: LiveClient) -> bool {
	CLIENT.set(client).is_ok()
}

/// Has the app's live connection read as whoever its credentials now say:
/// call it after signing in, signing out, or renewing a token.
/// `leptoskit_auth`'s client calls it itself, with its `rpc` feature on.
/// Does nothing without a live client.
pub fn reauthenticate() {
	if let Some(live) = CLIENT.get() {
		live.reauthenticate();
	}
}

/// Where the live connection is, and how many calls wait for it, as a
/// signal: for a banner that says "Reconnecting…" or "3 changes waiting".
///
/// ```ignore
/// let status = rpc::use_live_status();
/// let banner = move || match status.get().state {
/// 	LiveState::Reconnecting { .. } => Some("Reconnecting…"),
/// 	_ => None,
/// };
/// ```
///
/// Without a live connection, as while the server renders, it stays
/// [`LiveStatus::OFFLINE`].
#[track_caller]
pub fn use_live_status() -> ReadSignal<LiveStatus> {
	let Some(live) = CLIENT.get() else {
		return RwSignal::new(LiveStatus::OFFLINE).read_only();
	};
	let status = RwSignal::new(live.status());
	let mut changes = live.watch_status();
	// It may have changed between reading it and watching it.
	status.set(live.status());
	Executor::spawn_local(async move {
		while let Some(next) = changes.next().await {
			if status.try_set(next).is_some() {
				// The signal was disposed with its component.
				break;
			}
		}
	});
	status.read_only()
}

#[cfg(any(
	all(
		feature = "web",
		not(feature = "guest"),
		target_arch = "wasm32",
		target_os = "unknown"
	),
	all(feature = "guest", target_arch = "wasm32", target_os = "unknown"),
))]
fn transport(config: &ClientConfig) -> Option<Arc<dyn Transport>> {
	let origin = config.origin.clone().or_else(page_origin)?;
	let socket = Arc::new(Socket {
		url: socket_url(&origin),
	});

	// The browser falls back to server-sent events; a native guest's HTTP
	// plugin can't stream, so it keeps the socket.
	#[cfg(all(
		feature = "web",
		not(feature = "guest"),
		target_arch = "wasm32",
		target_os = "unknown"
	))]
	return Some(Arc::new(Fallback::new(
		socket,
		Arc::new(events::EventStream::new(&origin)),
	)));

	#[cfg(not(all(
		feature = "web",
		not(feature = "guest"),
		target_arch = "wasm32",
		target_os = "unknown"
	)))]
	Some(socket)
}

#[cfg(not(any(
	all(
		feature = "web",
		not(feature = "guest"),
		target_arch = "wasm32",
		target_os = "unknown"
	),
	all(feature = "guest", target_arch = "wasm32", target_os = "unknown"),
)))]
fn transport(_config: &ClientConfig) -> Option<Arc<dyn Transport>> {
	None
}

#[cfg(any(
	all(
		feature = "web",
		not(feature = "guest"),
		target_arch = "wasm32",
		target_os = "unknown"
	),
	all(feature = "guest", target_arch = "wasm32", target_os = "unknown"),
))]
fn socket_url(origin: &str) -> String {
	let origin = origin.trim_end_matches('/');

	if let Some(rest) = origin.strip_prefix("https://") {
		format!("wss://{rest}/live")
	} else if let Some(rest) = origin.strip_prefix("http://") {
		format!("ws://{rest}/live")
	} else {
		format!("{origin}/live")
	}
}

#[cfg(all(
	feature = "web",
	not(feature = "guest"),
	target_arch = "wasm32",
	target_os = "unknown"
))]
fn page_origin() -> Option<String> {
	let window = web_sys::window()?;
	let location = window.location();
	let scheme = location.protocol().ok()?;
	let host = location.host().ok()?;
	Some(format!("{scheme}//{host}"))
}

#[cfg(not(all(
	feature = "web",
	not(feature = "guest"),
	target_arch = "wasm32",
	target_os = "unknown"
)))]
#[allow(dead_code, reason = "the stub keeps the call sites simple")]
fn page_origin() -> Option<String> {
	None
}

/// The WebSocket transport of the platform this build targets.
#[cfg(any(
	all(
		feature = "web",
		not(feature = "guest"),
		target_arch = "wasm32",
		target_os = "unknown"
	),
	all(feature = "guest", target_arch = "wasm32", target_os = "unknown"),
))]
struct Socket {
	url: String,
}

#[cfg(any(
	all(
		feature = "web",
		not(feature = "guest"),
		target_arch = "wasm32",
		target_os = "unknown"
	),
	all(feature = "guest", target_arch = "wasm32", target_os = "unknown"),
))]
impl Transport for Socket {
	fn connect(&self) -> BoxFuture<'static, Result<Box<dyn Connection>, String>> {
		let url = self.url.clone();
		Box::pin(async move { connect(&url).await })
	}
}

#[cfg(all(
	feature = "web",
	not(feature = "guest"),
	target_arch = "wasm32",
	target_os = "unknown"
))]
async fn connect(url: &str) -> Result<Box<dyn Connection>, String> {
	let origin = url
		.strip_suffix("/live")
		.ok_or_else(|| "the live URL has no path".to_owned())?;
	let config =
		leptoskit_plugin_websocket::Config::new([origin]).map_err(|error| error.to_string())?;
	let client =
		leptoskit_plugin_websocket::Client::new(config).map_err(|error| error.to_string())?;
	let connection = send_wrapper::SendWrapper::new(async move {
		client
			.connect(url, &[SUBPROTOCOL.to_owned()])
			.await
			.map_err(|error| error.to_string())
	})
	.await?;
	Ok(Box::new(BrowserConnection(Arc::new(
		send_wrapper::SendWrapper::new(connection),
	))))
}

#[cfg(all(feature = "guest", target_arch = "wasm32", target_os = "unknown"))]
async fn connect(url: &str) -> Result<Box<dyn Connection>, String> {
	let client = leptoskit_plugin_websocket::guest::WebSocketClient::connect()
		.map_err(|error| error.to_string())?;
	let connection = send_wrapper::SendWrapper::new(async move {
		client
			.open(url, &[SUBPROTOCOL.to_owned()])
			.await
			.map_err(|error| error.to_string())
	})
	.await?;
	Ok(Box::new(GuestConnection(Arc::new(
		send_wrapper::SendWrapper::new(connection),
	))))
}

#[cfg(all(
	feature = "web",
	not(feature = "guest"),
	target_arch = "wasm32",
	target_os = "unknown"
))]
struct BrowserConnection(Arc<send_wrapper::SendWrapper<leptoskit_plugin_websocket::Connection>>);

#[cfg(all(
	feature = "web",
	not(feature = "guest"),
	target_arch = "wasm32",
	target_os = "unknown"
))]
impl Connection for BrowserConnection {
	fn send(&self, text: String) -> BoxFuture<'static, Result<(), String>> {
		let connection = Arc::clone(&self.0);
		Box::pin(send_wrapper::SendWrapper::new(async move {
			connection
				.send(leptoskit_plugin_websocket::Message::Text(text))
				.await
				.map_err(|error| error.to_string())
		}))
	}

	fn next(&self) -> BoxFuture<'static, Result<Option<String>, String>> {
		let connection = Arc::clone(&self.0);
		Box::pin(send_wrapper::SendWrapper::new(async move {
			loop {
				let message = connection
					.receive()
					.await
					.map_err(|error| error.to_string())?;
				match message {
					Some(leptoskit_plugin_websocket::Message::Text(text)) => return Ok(Some(text)),
					Some(leptoskit_plugin_websocket::Message::Binary(_)) => {}
					None => return Ok(None),
				}
			}
		}))
	}
}

#[cfg(all(feature = "guest", target_arch = "wasm32", target_os = "unknown"))]
struct GuestConnection(
	Arc<send_wrapper::SendWrapper<leptoskit_plugin_websocket::guest::GuestConnection>>,
);

#[cfg(all(feature = "guest", target_arch = "wasm32", target_os = "unknown"))]
impl Connection for GuestConnection {
	fn send(&self, text: String) -> BoxFuture<'static, Result<(), String>> {
		let connection = Arc::clone(&self.0);
		Box::pin(send_wrapper::SendWrapper::new(async move {
			connection
				.send(leptoskit_plugin_websocket::Message::Text(text))
				.await
				.map_err(|error| error.to_string())
		}))
	}

	fn next(&self) -> BoxFuture<'static, Result<Option<String>, String>> {
		let connection = Arc::clone(&self.0);
		Box::pin(send_wrapper::SendWrapper::new(async move {
			loop {
				match connection
					.receive()
					.await
					.map_err(|error| error.to_string())?
				{
					Some(leptoskit_plugin_websocket::Message::Text(text)) => return Ok(Some(text)),
					Some(leptoskit_plugin_websocket::Message::Binary(_)) => {}
					None => return Ok(None),
				}
			}
		}))
	}
}

/// How many WebSocket connections in a row may fail before the browser
/// client switches to server-sent events.
#[cfg(any(
	test,
	all(
		feature = "web",
		not(feature = "guest"),
		target_arch = "wasm32",
		target_os = "unknown"
	)
))]
const UPGRADES_BEFORE_FALLBACK: u32 = 2;

/// A transport that tries `primary`, and after
/// [`UPGRADES_BEFORE_FALLBACK`] failed connections in a row uses
/// `secondary` from then on: the browser's WebSocket, then server-sent
/// events.
#[cfg(any(
	test,
	all(
		feature = "web",
		not(feature = "guest"),
		target_arch = "wasm32",
		target_os = "unknown"
	)
))]
pub(crate) struct Fallback {
	primary: Arc<dyn Transport>,
	secondary: Arc<dyn Transport>,
	state: Arc<FallbackState>,
}

#[cfg(any(
	test,
	all(
		feature = "web",
		not(feature = "guest"),
		target_arch = "wasm32",
		target_os = "unknown"
	)
))]
#[derive(Default)]
struct FallbackState {
	failures: AtomicU32,
	fallen_back: AtomicBool,
}

#[cfg(any(
	test,
	all(
		feature = "web",
		not(feature = "guest"),
		target_arch = "wasm32",
		target_os = "unknown"
	)
))]
impl Fallback {
	pub(crate) fn new(primary: Arc<dyn Transport>, secondary: Arc<dyn Transport>) -> Self {
		Self {
			primary,
			secondary,
			state: Arc::new(FallbackState::default()),
		}
	}
}

#[cfg(any(
	test,
	all(
		feature = "web",
		not(feature = "guest"),
		target_arch = "wasm32",
		target_os = "unknown"
	)
))]
impl Transport for Fallback {
	fn connect(&self) -> BoxFuture<'static, Result<Box<dyn Connection>, String>> {
		let primary = Arc::clone(&self.primary);
		let secondary = Arc::clone(&self.secondary);
		let state = Arc::clone(&self.state);
		Box::pin(async move {
			if state.fallen_back.load(Ordering::Relaxed) {
				return secondary.connect().await;
			}

			match primary.connect().await {
				Ok(connection) => {
					state.failures.store(0, Ordering::Relaxed);
					Ok(connection)
				}
				Err(error) => {
					let failures = state.failures.fetch_add(1, Ordering::Relaxed) + 1;

					if failures < UPGRADES_BEFORE_FALLBACK {
						return Err(error);
					}

					state.fallen_back.store(true, Ordering::Relaxed);
					secondary.connect().await
				}
			}
		})
	}
}

/// Server-sent events in the browser: an `EventSource` on `/live/events`
/// for the server's frames, and a same-origin `POST` to `/live/frames` for
/// each of the client's.
#[cfg(all(
	feature = "web",
	not(feature = "guest"),
	target_arch = "wasm32",
	target_os = "unknown"
))]
mod events {
	use std::sync::Arc;
	use std::sync::Mutex;
	use std::sync::PoisonError;

	use futures::StreamExt;
	use futures::channel::mpsc;
	use leptoskit_query::BoxFuture;
	use send_wrapper::SendWrapper;
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;
	use wasm_bindgen::closure::Closure;
	use wasm_bindgen_futures::JsFuture;

	use super::Connection;
	use super::Transport;
	use crate::live::EVENTS_PATH;
	use crate::live::FRAMES_PATH;
	use crate::live::ServerFrame;

	pub(super) struct EventStream {
		origin: String,
	}

	impl EventStream {
		pub(super) fn new(origin: &str) -> Self {
			Self {
				origin: origin.trim_end_matches('/').to_owned(),
			}
		}
	}

	impl Transport for EventStream {
		fn connect(&self) -> BoxFuture<'static, Result<Box<dyn Connection>, String>> {
			let origin = self.origin.clone();
			Box::pin(SendWrapper::new(async move { open(&origin).await }))
		}
	}

	/// The browser objects behind one stream, kept together on its thread.
	struct Source {
		events: web_sys::EventSource,
		_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
		_error: Closure<dyn FnMut(web_sys::Event)>,
	}

	impl Drop for Source {
		fn drop(&mut self) {
			self.events.close();
		}
	}

	struct EventConnection {
		_source: Arc<SendWrapper<Source>>,
		frames: Arc<Mutex<mpsc::UnboundedReceiver<Result<String, String>>>>,
		welcome: Arc<Mutex<Option<String>>>,
		post: String,
	}

	async fn open(origin: &str) -> Result<Box<dyn Connection>, String> {
		let source = web_sys::EventSource::new(&format!("{origin}{EVENTS_PATH}"))
			.map_err(|_| "the event stream wouldn't open".to_owned())?;
		let (sender, mut frames) = mpsc::unbounded::<Result<String, String>>();
		let on_message = sender.clone();
		let message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
			move |event: web_sys::MessageEvent| {
				if let Some(text) = event.data().as_string() {
					let _ = on_message.unbounded_send(Ok(text));
				}
			},
		);
		let closing = source.clone();
		let error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
			// The client reconnects and resumes itself, rather than letting
			// the event source retry from its last event alone.
			closing.close();
			let _ = sender.unbounded_send(Err("the event stream closed".to_owned()));
		});
		source.set_onmessage(Some(message.as_ref().unchecked_ref()));
		source.set_onerror(Some(error.as_ref().unchecked_ref()));
		let source = Source {
			events: source,
			_message: message,
			_error: error,
		};

		// The welcome names the connection that posts go to.
		let welcome = match frames.next().await {
			Some(Ok(text)) => text,
			Some(Err(error)) => return Err(error),
			None => return Err("the event stream closed".to_owned()),
		};
		let Ok(ServerFrame::Welcome { connection, .. }) = serde_json::from_str(&welcome) else {
			return Err("the event stream didn't start with a welcome".to_owned());
		};

		Ok(Box::new(EventConnection {
			_source: Arc::new(SendWrapper::new(source)),
			frames: Arc::new(Mutex::new(frames)),
			welcome: Arc::new(Mutex::new(Some(welcome))),
			post: format!("{origin}{FRAMES_PATH}?connection={connection}"),
		}))
	}

	impl Connection for EventConnection {
		fn send(&self, text: String) -> BoxFuture<'static, Result<(), String>> {
			let url = self.post.clone();
			Box::pin(SendWrapper::new(async move { post(&url, &text).await }))
		}

		fn next(&self) -> BoxFuture<'static, Result<Option<String>, String>> {
			let welcome = self
				.welcome
				.lock()
				.unwrap_or_else(PoisonError::into_inner)
				.take();
			let frames = Arc::clone(&self.frames);
			Box::pin(SendWrapper::new(async move {
				if let Some(welcome) = welcome {
					return Ok(Some(welcome));
				}

				let next = futures::future::poll_fn(|cx| {
					frames
						.lock()
						.unwrap_or_else(PoisonError::into_inner)
						.poll_next_unpin(cx)
				})
				.await;

				match next {
					Some(Ok(text)) => Ok(Some(text)),
					Some(Err(error)) => Err(error),
					None => Ok(None),
				}
			}))
		}
	}

	/// Posts one frame, as a one-item array, with the session cookie.
	async fn post(url: &str, frame: &str) -> Result<(), String> {
		let failed = |_: JsValue| "couldn't post the frame".to_owned();
		let init = web_sys::RequestInit::new();
		init.set_method("POST");
		init.set_credentials(web_sys::RequestCredentials::SameOrigin);
		let headers = web_sys::Headers::new().map_err(failed)?;
		headers
			.set("content-type", "application/json")
			.map_err(failed)?;
		init.set_headers(&headers);
		init.set_body(&JsValue::from_str(&format!("[{frame}]")));
		let window = web_sys::window().ok_or_else(|| "no window to post from".to_owned())?;
		let response: web_sys::Response =
			JsFuture::from(window.fetch_with_str_and_init(url, &init))
				.await
				.map_err(failed)?
				.dyn_into()
				.map_err(failed)?;

		if response.ok() {
			Ok(())
		} else {
			Err(format!(
				"the server refused the frame: {}",
				response.status()
			))
		}
	}
}

fn live_here() -> bool {
	CLIENT.get().is_some()
		|| cfg!(any(
			all(
				feature = "web",
				not(feature = "guest"),
				target_arch = "wasm32",
				target_os = "unknown"
			),
			all(
				feature = "guest",
				target_arch = "wasm32",
				target_os = "unknown"
			),
		))
}

/// What a channel's screen sees: who is in it, what they sent, and a way
/// to send.
pub struct ChannelHandle<C: Channel> {
	key: StoredValue<Option<String>>,
	members: RwSignal<Vec<PresenceMember>>,
	signals: RwSignal<Vec<C::Message>>,
}

impl<C: Channel> ChannelHandle<C> {
	/// Who is in the channel now, newest presence first.
	pub fn members(&self) -> ReadSignal<Vec<PresenceMember>> {
		self.members.read_only()
	}

	/// The last 100 messages the others sent, oldest first.
	pub fn signals(&self) -> ReadSignal<Vec<C::Message>> {
		self.signals.read_only()
	}

	/// Sends a message to everyone else in the channel.
	#[expect(
		clippy::needless_pass_by_value,
		reason = "a signal reads `chat.signal(Typing { … })`: the message is built for it"
	)]
	pub fn signal(&self, message: C::Message) {
		let Some(key) = self.key.get_value() else {
			return;
		};

		if let Some(live) = CLIENT.get() {
			live.signal::<C>(&key, &message);
		}
	}

	/// Forgets the messages received so far.
	pub fn clear_signals(&self) {
		self.signals.set(Vec::new());
	}
}

/// Joins a channel while the component is mounted, and leaves when the
/// join arguments change or it unmounts.
///
/// ```ignore
/// let chat = rpc::use_channel::<Chat>(move || Chat { channel });
/// let typing = move || chat.signals().get();
/// chat.signal(Typing { who: me, at: now() });
/// ```
///
/// The `authorize` function runs as the caller on the server; a refusal
/// arrives as an empty presence and no messages. Signals are at most once:
/// anything that must survive belongs in a live table and a live query.
#[track_caller]
pub fn use_channel<C>(args: impl Fn() -> C + Send + Sync + 'static) -> ChannelHandle<C>
where
	C: Channel,
{
	let members = RwSignal::new(Vec::new());
	let signals = RwSignal::new(Vec::new());
	let key = StoredValue::new(None::<String>);

	if live_here() {
		let args = Arc::new(args);
		Effect::new_isomorphic(move |_| {
			let args = args();
			let Some(live) = CLIENT.get() else {
				return;
			};
			if let Some(previous) = key.get_value() {
				live.leave(&previous);
			}

			let (joined, mut events) = live.join(args, None);
			key.set_value(Some(joined));
			Executor::spawn_local(async move {
				while let Some(event) = events.next().await {
					match event {
						ChannelEvent::Presence(next) => members.set(next),
						ChannelEvent::Signal { message, .. } => {
							push_signal::<C>(signals, message);
						}
						ChannelEvent::Error(_) => {}
					}
				}
			});
		});
	}

	ChannelHandle {
		key,
		members,
		signals,
	}
}

/// Keeps the newest 100 signals, oldest first.
fn push_signal<C: Channel>(signals: RwSignal<Vec<C::Message>>, message: Value) {
	let Ok(message) = serde_json::from_value::<C::Message>(message) else {
		return;
	};
	signals.update(|list| {
		if list.len() >= 100 {
			list.remove(0);
		}

		list.push(message);
	});
}

/// A live query: the same cached query [`use_rpc_query`] returns, kept
/// current from the socket.
///
/// It renders the value the ordinary client fetched while the server
/// rendered, or over HTTP on first use, and every write that changes the
/// result arrives without a refetch.
///
/// Without a socket — while the server renders, or on a platform with no
/// transport — this is exactly [`use_rpc_query`].
#[track_caller]
pub fn use_live<Q>(args: impl Fn() -> Q + Send + Sync + 'static) -> QueryHandle<Q::Output, FnError>
where
	Q: Operation,
	Q::Output: Clone,
{
	let args = Arc::new(args);
	let first = Arc::clone(&args);
	let handle = use_rpc_query(move || first());

	if !live_here() {
		return handle;
	}

	let key = ArcMemo::new(move |_| Args::new(args()));
	let subscription = StoredValue::new(None::<u32>);
	let cache: QueryClient = use_query_client();

	Effect::new_isomorphic(move |_| {
		let key = key.get();
		let Some(live) = CLIENT.get() else {
			return;
		};
		if let Some(previous) = subscription.get_value() {
			live.unsubscribe(previous);
		}

		let Ok(value) = serde_json::to_value(key.get()) else {
			subscription.set_value(None);
			return;
		};
		let (id, mut updates) = live.subscribe(Q::NAME, value);
		subscription.set_value(Some(id));

		let cache = cache.clone();
		let scope = query_scope::<Q>();
		let read = key.clone();
		Executor::spawn_local(async move {
			while let Some(update) = updates.next().await {
				match update {
					Update::Ok(value) => {
						if let Ok(output) = serde_json::from_value(value) {
							cache.set_query_data(&scope, read.clone(), output);
						}
					}
					Update::Patch(patch) => {
						let update = Update::Patch(patch);
						cache.update_query_data(&scope, read.clone(), |current| {
							let current = current?;
							let mut value = serde_json::to_value(current).ok()?;
							apply(&mut value, &update);
							serde_json::from_value(value).ok()
						});
					}
					Update::Err(_) => {}
				}
			}
		});
	});

	handle
}

/// The `leptoskit_query` scope of a mutation or an action sent over the
/// live connection, named `rpc:<name>` like
/// [`mutation_scope`](super::mutation_scope). Its call resolves once the
/// connection has applied the transition that includes its writes, so the
/// screen's live queries already show them when `on_success` runs.
///
/// A mutation goes with a fresh idempotency key, which a reconnect resends;
/// an action is never sent twice. Without a live connection, as while the
/// server renders, it is the ordinary call.
pub fn live_mutation_scope<M>() -> MutationScope<M, M::Output, FnError>
where
	M: Operation,
	M::Output: Clone,
{
	MutationScope::new(|op: M| send_live(op)).with_name(&format!("rpc:{}", M::NAME))
}

/// A mutation, ready to run from a screen over the live connection: the
/// same handle as [`use_mutation`](leptoskit_query::use_mutation), whose
/// result arrives once the screen shows the write.
///
/// ```ignore
/// let send = use_live_mutation::<SendMessage>();
/// send.mutate(SendMessage { channel, body });
/// // When it resolves, `use_live(Messages { channel, … })` already has it.
/// ```
#[track_caller]
pub fn use_live_mutation<M>() -> MutationHandle<M, M::Output, FnError>
where
	M: Operation,
	M::Output: Clone,
{
	use_mutation(live_mutation_scope::<M>())
}

/// An action, ready to run from a screen over the live connection, as
/// [`use_live_mutation`] runs a mutation.
#[track_caller]
pub fn use_live_action<A>() -> MutationHandle<A, A::Output, FnError>
where
	A: Operation,
	A::Output: Clone,
{
	use_mutation(live_mutation_scope::<A>())
}

/// Sends a mutation over the live connection while it carries calls, and
/// as an ordinary call otherwise: what
/// [`mutation_scope`](super::mutation_scope) runs.
pub(crate) fn send_or_call<M: Operation>(op: M) -> BoxFuture<'static, Result<M::Output, FnError>> {
	match CLIENT.get() {
		Some(live) if live.carries_calls() => send_live(op),
		_ => Box::pin(super::call(op)),
	}
}

/// Sends `op` over the live connection, or as an ordinary call without one.
fn send_live<M: Operation>(op: M) -> BoxFuture<'static, Result<M::Output, FnError>> {
	let Some(live) = CLIENT.get() else {
		return Box::pin(super::call(op));
	};
	let arguments = match serde_json::to_value(&op) {
		Ok(arguments) => arguments,
		Err(error) => {
			return Box::pin(std::future::ready(Err(FnError::Invalid(format!(
				"the arguments aren't JSON: {error}"
			)))));
		}
	};
	let work = async move {
		let value = if M::KIND == crate::Kind::Action {
			live.act(M::NAME, arguments).await?
		} else {
			let key = super::keys::new_key().await;
			live.mutate(M::NAME, arguments, Some(key)).await?
		};
		serde_json::from_value(value)
			.map_err(|error| FnError::Decode(format!("the result of `{}`: {error}", M::NAME)))
	};

	#[cfg(all(target_arch = "wasm32", any(feature = "web", feature = "guest")))]
	return Box::pin(send_wrapper::SendWrapper::new(work));

	#[cfg(not(all(target_arch = "wasm32", any(feature = "web", feature = "guest"))))]
	Box::pin(work)
}

#[cfg(test)]
mod tests {
	use std::collections::VecDeque;
	use std::sync::Once;

	use serde_json::json;
	use tokio::sync::Notify;

	use super::*;

	static START: Once = Once::new();

	fn start() {
		START.call_once(|| {
			Executor::init_tokio().unwrap();
		});
	}

	/// A socket whose two ends the test drives.
	#[derive(Clone, Default)]
	struct Memory {
		sent: Arc<Mutex<Vec<String>>>,
		incoming: Arc<Mutex<VecDeque<String>>>,
		wake: Arc<Notify>,
		written: Arc<Notify>,
		/// Drops the open connection, once.
		drop: Arc<AtomicBool>,
		connects: Arc<AtomicU32>,
	}

	impl Memory {
		fn drop_connection(&self) {
			self.drop.store(true, Ordering::Relaxed);
		}

		fn server_says(&self, frame: &ServerFrame) {
			self.incoming
				.lock()
				.unwrap()
				.push_back(serde_json::to_string(frame).unwrap());
			self.wake.notify_waiters();
		}

		async fn next_sent(&self) -> ClientFrame {
			for _ in 0..400 {
				let text = {
					let mut sent = self.sent.lock().unwrap();
					if sent.is_empty() {
						None
					} else {
						Some(sent.remove(0))
					}
				};

				if let Some(text) = text {
					return serde_json::from_str(&text).unwrap();
				}

				tokio::time::sleep(Duration::from_millis(5)).await;
			}

			panic!("the client sent nothing");
		}
	}

	impl Transport for Memory {
		fn connect(&self) -> BoxFuture<'static, Result<Box<dyn Connection>, String>> {
			self.connects.fetch_add(1, Ordering::Relaxed);
			let this = self.clone();
			Box::pin(async move { Ok(Box::new(this) as Box<dyn Connection>) })
		}
	}

	/// A WebSocket the network never lets through.
	#[derive(Clone, Default)]
	struct Refused {
		attempts: Arc<AtomicU32>,
	}

	impl Transport for Refused {
		fn connect(&self) -> BoxFuture<'static, Result<Box<dyn Connection>, String>> {
			self.attempts.fetch_add(1, Ordering::Relaxed);
			Box::pin(std::future::ready(Err("the upgrade failed".to_owned())))
		}
	}

	impl Connection for Memory {
		fn send(&self, text: String) -> BoxFuture<'static, Result<(), String>> {
			self.sent.lock().unwrap().push(text);
			self.written.notify_waiters();
			Box::pin(std::future::ready(Ok(())))
		}

		fn next(&self) -> BoxFuture<'static, Result<Option<String>, String>> {
			let this = self.clone();
			Box::pin(async move {
				loop {
					if this.drop.swap(false, Ordering::Relaxed) {
						return Ok(None);
					}

					if let Some(text) = this.incoming.lock().unwrap().pop_front() {
						return Ok(Some(text));
					}

					tokio::time::sleep(Duration::from_millis(5)).await;
				}
			})
		}
	}

	#[tokio::test]
	async fn a_subscription_takes_its_first_value_and_then_its_patches() {
		start();
		let local = tokio::task::LocalSet::new();
		local
			.run_until(async {
				let socket = Memory::default();
				let client = LiveClient::new(&ClientConfig::new(), Arc::new(socket.clone()));

				assert!(matches!(
					socket.next_sent().await,
					ClientFrame::Hello {
						resume: Some(Resume { version: 0, .. })
					}
				));

				let (id, mut updates) = client.subscribe("messages", json!({ "limit": 2 }));
				assert!(matches!(
					socket.next_sent().await,
					ClientFrame::Sub { id: sent, .. } if sent == id
				));

				socket.server_says(&ServerFrame::Done {
					request: id,
					ok: Some(json!([{ "id": "a", "body": "one" }])),
					err: None,
					version: 0,
				});
				assert_eq!(
					tokio::time::timeout(Duration::from_secs(2), updates.next())
						.await
						.unwrap()
						.unwrap(),
					Update::Ok(json!([{ "id": "a", "body": "one" }]))
				);

				socket.server_says(&ServerFrame::Transition(Transition {
					from: 0,
					to: 1,
					updates: vec![crate::live::SubscriptionUpdate {
						id,
						update: Update::Patch(crate::live::Patch {
							remove: vec![],
							upsert: vec![crate::live::Upsert {
								at: 0,
								value: json!({ "id": "a", "body": "two" }),
							}],
							..crate::live::Patch::default()
						}),
					}],
					full: false,
				}));
				assert_eq!(
					tokio::time::timeout(Duration::from_secs(2), updates.next())
						.await
						.unwrap()
						.unwrap(),
					Update::Patch(crate::live::Patch {
						remove: vec![],
						upsert: vec![crate::live::Upsert {
							at: 0,
							value: json!({ "id": "a", "body": "two" }),
						}],
						..crate::live::Patch::default()
					})
				);
				assert_eq!(client.version(), 1);
			})
			.await;
	}

	#[derive(Clone, serde::Deserialize, serde::Serialize)]
	struct Room {
		id: u32,
	}

	impl Channel for Room {
		type Message = String;
		type Presence = ();

		const NAME: &'static str = "room";
	}

	#[tokio::test]
	async fn a_channel_joins_sends_and_reads_presence() {
		start();
		let local = tokio::task::LocalSet::new();
		local
			.run_until(async {
				let socket = Memory::default();
				let client = LiveClient::new(&ClientConfig::new(), Arc::new(socket.clone()));
				assert!(matches!(
					socket.next_sent().await,
					ClientFrame::Hello { .. }
				));

				let (key, mut events) = client.join(Room { id: 7 }, None);
				let ClientFrame::Join {
					channel,
					name,
					args,
					state,
				} = socket.next_sent().await
				else {
					panic!("the join is sent");
				};
				assert_eq!(channel, key);
				assert_eq!(name, "room");
				assert_eq!(args, json!({ "id": 7 }));
				assert_eq!(state, json!(null));

				socket.server_says(&ServerFrame::Presence {
					channel: key.clone(),
					members: vec![PresenceMember {
						id: "ada".into(),
						state: json!({ "name": "Ada" }),
					}],
				});
				assert_eq!(
					tokio::time::timeout(Duration::from_secs(2), events.next())
						.await
						.unwrap()
						.unwrap(),
					ChannelEvent::Presence(vec![PresenceMember {
						id: "ada".into(),
						state: json!({ "name": "Ada" }),
					}])
				);

				client.signal::<Room>(&key, &"typing".to_owned());
				let ClientFrame::Signal {
					channel, message, ..
				} = socket.next_sent().await
				else {
					panic!("the signal is sent");
				};
				assert_eq!(channel, key);
				assert_eq!(message, json!("typing"));

				socket.server_says(&ServerFrame::Signal {
					channel: key.clone(),
					from: Some("grace".into()),
					message: json!("hi"),
				});
				assert_eq!(
					tokio::time::timeout(Duration::from_secs(2), events.next())
						.await
						.unwrap()
						.unwrap(),
					ChannelEvent::Signal {
						from: Some("grace".into()),
						message: json!("hi"),
					}
				);

				client.leave(&key);
				assert!(matches!(
					socket.next_sent().await,
					ClientFrame::Leave { .. }
				));
			})
			.await;
	}

	fn transition(to: u64) -> ServerFrame {
		ServerFrame::Transition(Transition {
			from: 0,
			to,
			updates: Vec::new(),
			full: false,
		})
	}

	#[tokio::test]
	async fn a_mutation_resolves_once_a_transition_reaches_its_commit() {
		start();
		let local = tokio::task::LocalSet::new();
		local
			.run_until(async {
				let socket = Memory::default();
				let client = LiveClient::new(&ClientConfig::new(), Arc::new(socket.clone()));
				assert!(matches!(
					socket.next_sent().await,
					ClientFrame::Hello { .. }
				));

				let answer = client.mutate("send", json!({ "body": "hi" }), Some("key".into()));
				let ClientFrame::Mutate { request, .. } = socket.next_sent().await else {
					panic!("the mutation is sent");
				};
				socket.server_says(&ServerFrame::Done {
					request,
					ok: Some(json!("0190")),
					err: None,
					version: 3,
				});

				// Answered, but the client hasn't applied version 3 yet.
				let answer = tokio::task::spawn_local(answer);
				tokio::time::sleep(Duration::from_millis(50)).await;
				assert!(!answer.is_finished());
				assert_eq!(client.version(), 0);

				socket.server_says(&transition(2));
				tokio::time::sleep(Duration::from_millis(50)).await;
				assert!(!answer.is_finished());

				socket.server_says(&transition(3));
				let value = tokio::time::timeout(Duration::from_secs(2), answer)
					.await
					.unwrap()
					.unwrap()
					.unwrap();
				assert_eq!(value, json!("0190"));
				assert_eq!(client.version(), 3);
			})
			.await;
	}

	#[tokio::test]
	async fn a_mutation_the_client_already_holds_resolves_at_once() {
		start();
		let local = tokio::task::LocalSet::new();
		local
			.run_until(async {
				let socket = Memory::default();
				let client = LiveClient::new(&ClientConfig::new(), Arc::new(socket.clone()));
				let _ = socket.next_sent().await;
				socket.server_says(&transition(4));
				tokio::time::sleep(Duration::from_millis(30)).await;

				let answer = client.mutate("noop", json!({}), None);
				let ClientFrame::Mutate { request, .. } = socket.next_sent().await else {
					panic!("the mutation is sent");
				};
				socket.server_says(&ServerFrame::Done {
					request,
					ok: Some(json!(null)),
					err: None,
					version: 4,
				});
				let value = tokio::time::timeout(Duration::from_secs(2), answer)
					.await
					.unwrap()
					.unwrap();
				assert_eq!(value, json!(null));
			})
			.await;
	}

	#[tokio::test]
	async fn a_failed_mutation_resolves_without_waiting() {
		start();
		let local = tokio::task::LocalSet::new();
		local
			.run_until(async {
				let socket = Memory::default();
				let client = LiveClient::new(&ClientConfig::new(), Arc::new(socket.clone()));
				let _ = socket.next_sent().await;

				let answer = client.mutate("send", json!({}), None);
				let ClientFrame::Mutate { request, .. } = socket.next_sent().await else {
					panic!("the mutation is sent");
				};
				socket.server_says(&ServerFrame::Done {
					request,
					ok: None,
					err: Some(WireError {
						status: 400,
						message: "no body".into(),
					}),
					version: 9,
				});
				let error = tokio::time::timeout(Duration::from_secs(2), answer)
					.await
					.unwrap()
					.unwrap_err();
				assert_eq!(error, FnError::Invalid("no body".into()));
			})
			.await;
	}

	#[tokio::test]
	async fn an_action_resolves_once_its_writes_arrive() {
		start();
		let local = tokio::task::LocalSet::new();
		local
			.run_until(async {
				let socket = Memory::default();
				let client = LiveClient::new(&ClientConfig::new(), Arc::new(socket.clone()));
				let _ = socket.next_sent().await;

				let answer =
					tokio::task::spawn_local(client.act("summarize", json!({ "channel": 1 })));
				let ClientFrame::Act {
					request,
					name,
					args,
				} = socket.next_sent().await
				else {
					panic!("the action is sent");
				};
				assert_eq!(name, "summarize");
				assert_eq!(args, json!({ "channel": 1 }));
				socket.server_says(&ServerFrame::Done {
					request,
					ok: Some(json!("a summary")),
					err: None,
					version: 2,
				});
				tokio::time::sleep(Duration::from_millis(50)).await;
				assert!(!answer.is_finished());

				socket.server_says(&transition(2));
				let value = tokio::time::timeout(Duration::from_secs(2), answer)
					.await
					.unwrap()
					.unwrap()
					.unwrap();
				assert_eq!(value, json!("a summary"));
			})
			.await;
	}

	#[tokio::test]
	async fn an_action_on_a_dropped_connection_fails_rather_than_running_twice() {
		start();
		let local = tokio::task::LocalSet::new();
		local
			.run_until(async {
				let socket = Memory::default();
				let client = LiveClient::new(&ClientConfig::new(), Arc::new(socket.clone()));
				let _ = socket.next_sent().await;

				let answer = client.act("charge", json!({}));
				assert!(matches!(socket.next_sent().await, ClientFrame::Act { .. }));
				socket.drop_connection();
				let error = tokio::time::timeout(Duration::from_secs(2), answer)
					.await
					.unwrap()
					.unwrap_err();
				assert!(matches!(error, FnError::Network(_)), "{error:?}");

				// The reconnect doesn't send it again.
				assert!(matches!(
					socket.next_sent().await,
					ClientFrame::Hello { .. }
				));
				tokio::time::sleep(Duration::from_millis(50)).await;
				assert!(socket.sent.lock().unwrap().is_empty());
			})
			.await;
	}

	#[tokio::test]
	async fn a_reconnect_resumes_from_the_last_version_with_what_it_holds() {
		start();
		let local = tokio::task::LocalSet::new();
		local
			.run_until(async {
				let socket = Memory::default();
				let client = LiveClient::new(&ClientConfig::new(), Arc::new(socket.clone()));
				let _ = socket.next_sent().await;

				let (held, _updates) = client.subscribe("messages", json!({ "limit": 2 }));
				let _ = socket.next_sent().await;
				socket.server_says(&ServerFrame::Done {
					request: held,
					ok: Some(json!([])),
					err: None,
					version: 0,
				});
				socket.server_says(&transition(5));
				tokio::time::sleep(Duration::from_millis(30)).await;

				// Subscribed while the answer hasn't come: subscribed again.
				let (waiting, _pending) = client.subscribe("tags", json!({}));
				let _ = socket.next_sent().await;

				socket.drop_connection();
				let ClientFrame::Hello {
					resume: Some(resume),
				} = socket.next_sent().await
				else {
					panic!("the reconnect says hello first");
				};
				assert_eq!(resume.version, 5);
				assert_eq!(
					resume.queries,
					[ResumeQuery {
						id: held,
						query: "messages".into(),
						args: json!({ "limit": 2 }),
					}]
				);
				let ClientFrame::Sub { id, query, .. } = socket.next_sent().await else {
					panic!("the subscription without a value subscribes again");
				};
				assert_eq!(id, waiting);
				assert_eq!(query, "tags");
				assert_eq!(client.version(), 5);
			})
			.await;
	}

	#[tokio::test]
	async fn two_failed_upgrades_fall_back_to_server_sent_events() {
		start();
		let local = tokio::task::LocalSet::new();
		local
			.run_until(async {
				let socket = Refused::default();
				let events = Memory::default();
				let transport = Fallback::new(Arc::new(socket.clone()), Arc::new(events.clone()));
				let client = LiveClient::new(&ClientConfig::new(), Arc::new(transport));

				// The same session over the second transport.
				assert!(matches!(
					events.next_sent().await,
					ClientFrame::Hello { .. }
				));
				assert_eq!(socket.attempts.load(Ordering::Relaxed), 2);

				let (id, mut updates) = client.subscribe("messages", json!({}));
				assert!(matches!(
					events.next_sent().await,
					ClientFrame::Sub { id: sent, .. } if sent == id
				));
				events.server_says(&ServerFrame::Done {
					request: id,
					ok: Some(json!([])),
					err: None,
					version: 0,
				});
				assert_eq!(
					tokio::time::timeout(Duration::from_secs(2), updates.next())
						.await
						.unwrap()
						.unwrap(),
					Update::Ok(json!([]))
				);

				// It stays there: a dropped stream reconnects over events.
				events.drop_connection();
				assert!(matches!(
					events.next_sent().await,
					ClientFrame::Hello { .. }
				));
				assert_eq!(socket.attempts.load(Ordering::Relaxed), 2);
				assert_eq!(events.connects.load(Ordering::Relaxed), 2);
			})
			.await;
	}

	#[tokio::test]
	async fn one_failed_upgrade_tries_the_socket_again() {
		let socket = Refused::default();
		let events = Memory::default();
		let transport = Fallback::new(Arc::new(socket.clone()), Arc::new(events.clone()));

		assert!(transport.connect().await.is_err());
		assert_eq!(events.connects.load(Ordering::Relaxed), 0);
		assert!(transport.connect().await.is_ok());
		assert_eq!(events.connects.load(Ordering::Relaxed), 1);
		assert_eq!(socket.attempts.load(Ordering::Relaxed), 2);
	}
	#[derive(Clone, Default)]
	struct Refreshing(Arc<Mutex<bool>>);

	impl Credentials for Refreshing {
		fn authorization(&self) -> BoxFuture<'_, Result<Option<String>, FnError>> {
			let refreshed = *lock(&self.0);
			Box::pin(async move {
				Ok(Some(format!(
					"Bearer {}",
					if refreshed { "new" } else { "old" }
				)))
			})
		}

		fn refused(&self, sent: Option<String>) -> BoxFuture<'_, Result<bool, FnError>> {
			assert_eq!(sent.as_deref(), Some("Bearer old"));
			*lock(&self.0) = true;
			Box::pin(std::future::ready(Ok(true)))
		}
	}

	#[tokio::test]
	async fn expiring_credentials_are_refreshed_on_the_open_connection() {
		start();
		tokio::task::LocalSet::new()
			.run_until(async {
				let socket = Memory::default();
				let _client = LiveClient::new(
					&ClientConfig::new().credentials(Refreshing::default()),
					Arc::new(socket.clone()),
				);
				assert!(
					matches!(socket.next_sent().await, ClientFrame::Auth { token } if token == "old")
				);
				assert!(matches!(
					socket.next_sent().await,
					ClientFrame::Hello { .. }
				));
				socket.server_says(&ServerFrame::AuthStatus {
					refused: false,
					expiring: true,
				});
				assert!(
					matches!(socket.next_sent().await, ClientFrame::Auth { token } if token == "new")
				);
				assert_eq!(socket.connects.load(Ordering::Relaxed), 1);
			})
			.await;
	}

	#[tokio::test]
	async fn refused_credentials_fail_pending_calls_and_forget_resume_values() {
		start();
		tokio::task::LocalSet::new()
			.run_until(async {
				let socket = Memory::default();
				let client = LiveClient::new(&ClientConfig::new(), Arc::new(socket.clone()));
				let _ = socket.next_sent().await;
				let (id, mut values) = client.subscribe("private_notes", json!({}));
				let _ = socket.next_sent().await;
				socket.server_says(&ServerFrame::Done {
					request: id,
					ok: Some(json!(["private"])),
					err: None,
					version: 0,
				});
				assert_eq!(values.next().await, Some(Update::Ok(json!(["private"]))));
				let answer = tokio::task::spawn_local(client.mutate(
					"post",
					json!({}),
					Some("key".to_owned()),
				));
				assert!(matches!(
					socket.next_sent().await,
					ClientFrame::Mutate { .. }
				));
				socket.server_says(&ServerFrame::AuthStatus {
					refused: true,
					expiring: false,
				});
				assert!(matches!(
					values.next().await,
					Some(Update::Err(WireError { status: 401, .. }))
				));
				assert!(
					tokio::time::timeout(Duration::from_secs(2), answer)
						.await
						.unwrap()
						.unwrap()
						.is_err()
				);
				assert_eq!(client.status().queued, 0);
				let ClientFrame::Hello {
					resume: Some(resume),
				} = socket.next_sent().await
				else {
					panic!("a fresh handshake");
				};
				assert!(
					resume.queries.is_empty(),
					"old private values must not resume under another identity"
				);
			})
			.await;
	}
	#[tokio::test]
	async fn reconnecting_during_reauthentication_does_not_resume_the_previous_identity() {
		start();
		tokio::task::LocalSet::new()
			.run_until(async {
				let socket = Memory::default();
				let credentials = Refreshing::default();
				let client = LiveClient::new(
					&ClientConfig::new().credentials(credentials.clone()),
					Arc::new(socket.clone()),
				);
				let _ = socket.next_sent().await;
				let _ = socket.next_sent().await;
				let (id, mut values) = client.subscribe("private_notes", json!({}));
				let _ = socket.next_sent().await;
				socket.server_says(&ServerFrame::Done {
					request: id,
					ok: Some(json!(["old"])),
					err: None,
					version: 0,
				});
				let _ = values.next().await;
				*lock(&credentials.0) = true;
				client.reauthenticate();
				assert!(
					matches!(socket.next_sent().await, ClientFrame::Auth { token } if token == "new")
				);
				// A frame queued before the server received auth still belongs
				// to the previous identity, even if it arrives after we sent
				// it.
				dispatch(
					&client.inner,
					&serde_json::to_string(&ServerFrame::Transition(Transition {
						from: 0,
						to: 1,
						full: true,
						updates: vec![crate::live::SubscriptionUpdate {
							id,
							update: Update::Ok(json!(["old queued result"])),
						}],
					}))
					.unwrap(),
				);
				socket.drop_connection();
				socket.wake.notify_waiters();
				assert!(
					matches!(socket.next_sent().await, ClientFrame::Auth { token } if token == "new")
				);
				let ClientFrame::Hello {
					resume: Some(resume),
				} = socket.next_sent().await
				else {
					panic!("a fresh hello");
				};
				assert!(resume.queries.is_empty());
			})
			.await;
	}
}
