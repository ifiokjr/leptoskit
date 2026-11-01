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
//! The socket reconnects with exponential backoff and full jitter, resends
//! every subscription, and resends pending mutations with the same request
//! id, which the server's receipts make safe.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::OnceLock;
use std::sync::PoisonError;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
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
use leptoskit_query::QueryClient;
use leptoskit_query::QueryHandle;
use leptoskit_query::use_query_client;
use serde_json::Value;

use super::Args;
use super::ClientConfig;
use super::Credentials;
use super::FnError;
use super::query_scope;
use super::use_rpc_query;
use crate::Channel;
use crate::Operation;
use crate::live::ClientFrame;
use crate::live::PresenceMember;
use crate::live::Resume;
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
	commands: UnboundedSender<Command>,
	subscribers: Mutex<HashMap<u32, Subscriber>>,
	channels: Mutex<HashMap<String, JoinedChannel>>,
	pending: Mutex<HashMap<u32, oneshot::Sender<Result<Value, WireError>>>>,
	mutations: Mutex<HashMap<u32, PendingMutation>>,
	next_id: AtomicU32,
	version: AtomicU64,
}

struct Subscriber {
	name: String,
	args: Value,
	sender: UnboundedSender<Update>,
}

struct PendingMutation {
	name: String,
	args: Value,
	key: Option<String>,
}

struct JoinedChannel {
	name: String,
	args: Value,
	state: Value,
	sender: UnboundedSender<ChannelEvent>,
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

enum Command {
	Subscribe(u32),
	Unsubscribe(u32),
	Mutate(u32),
	Join(String),
	Leave(String),
	Signal(String, Value),
	Reconnect,
}

impl LiveClient {
	/// A client over `transport`, connected to `config`'s server.
	pub(crate) fn new(config: &ClientConfig, transport: Arc<dyn Transport>) -> Self {
		let (commands, receiver) = mpsc::unbounded();
		let inner = Arc::new(Inner {
			transport,
			credentials: config.credentials.clone(),
			commands,
			subscribers: Mutex::new(HashMap::new()),
			channels: Mutex::new(HashMap::new()),
			pending: Mutex::new(HashMap::new()),
			mutations: Mutex::new(HashMap::new()),
			next_id: AtomicU32::new(1),
			version: AtomicU64::new(0),
		});
		Executor::spawn_local(run(Arc::clone(&inner), receiver));
		Self { inner }
	}

	/// The last version the socket applied.
	#[must_use]
	pub fn version(&self) -> u64 {
		self.inner.version.load(Ordering::Relaxed)
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

	/// Runs a mutation over the socket, with its idempotency key.
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
			},
		);
		let _ = self.inner.commands.unbounded_send(Command::Mutate(id));
		async move {
			match receiver.await {
				Ok(Ok(value)) => Ok(value),
				Ok(Err(error)) => {
					Err(if error.status == 400 {
						FnError::Invalid(error.message)
					} else {
						FnError::Server(error.message)
					})
				}
				Err(_) => Err(FnError::Network("the live connection closed".to_owned())),
			}
		}
	}
}

impl Inner {
	fn subscribers(&self) -> MutexGuard<'_, HashMap<u32, Subscriber>> {
		self.subscribers
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
	}

	fn pending(&self) -> MutexGuard<'_, HashMap<u32, oneshot::Sender<Result<Value, WireError>>>> {
		self.pending.lock().unwrap_or_else(PoisonError::into_inner)
	}

	fn mutations(&self) -> MutexGuard<'_, HashMap<u32, PendingMutation>> {
		self.mutations
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
	}

	fn channels(&self) -> MutexGuard<'_, HashMap<String, JoinedChannel>> {
		self.channels.lock().unwrap_or_else(PoisonError::into_inner)
	}
}

async fn send(connection: &dyn Connection, frame: &ClientFrame) -> bool {
	let Ok(text) = serde_json::to_string(frame) else {
		return false;
	};
	connection.send(text).await.is_ok()
}

async fn run(inner: Arc<Inner>, mut commands: UnboundedReceiver<Command>) {
	let mut backoff = BACKOFF_START;

	loop {
		let Ok(connection) = inner.transport.connect().await else {
			leptoskit_query::sleep(Duration::from_millis(backoff)).await;
			backoff = (backoff * 2).min(BACKOFF_MAX);
			continue;
		};
		backoff = BACKOFF_START;

		if !handshake(inner.as_ref(), connection.as_ref()).await {
			continue;
		}

		loop {
			let command = commands.next().fuse();
			let frame = connection.next().fuse();
			futures::pin_mut!(command, frame);

			match futures::future::select(command, frame).await {
				Either::Left((Some(Command::Reconnect) | None, _)) => break,
				Either::Left((Some(command), _)) => {
					if !send_command(inner.as_ref(), connection.as_ref(), command).await {
						break;
					}
				}
				Either::Right((Ok(Some(text)), _)) => dispatch(inner.as_ref(), &text),
				Either::Right((..)) => break,
			}
		}

		leptoskit_query::sleep(Duration::from_millis(backoff)).await;
		backoff = (backoff * 2).min(BACKOFF_MAX);
	}
}

/// Says hello, authenticates, and restores what the connection held.
async fn handshake(inner: &Inner, connection: &dyn Connection) -> bool {
	let hello = ClientFrame::Hello {
		resume: Some(Resume {
			version: inner.version.load(Ordering::Relaxed),
		}),
	};

	if !send(connection, &hello).await {
		return false;
	}

	if let Some(token) = token(inner).await {
		send(connection, &ClientFrame::Auth { token }).await;
	}

	resubscribe(inner, connection).await;
	resend_mutations(inner, connection).await;
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
			match inner.mutations().get(&id) {
				Some(mutation) => {
					ClientFrame::Mutate {
						request: id,
						name: mutation.name.clone(),
						args: mutation.args.clone(),
						key: mutation.key.clone(),
					}
				}
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
		Command::Leave(key) => ClientFrame::Leave { channel: key },
		Command::Signal(key, message) => {
			ClientFrame::Signal {
				channel: key,
				message,
			}
		}
		Command::Reconnect => return false,
	};

	send(connection, &frame).await
}

fn dispatch(inner: &Inner, text: &str) {
	let Ok(frame) = serde_json::from_str::<ServerFrame>(text) else {
		return;
	};

	match frame {
		ServerFrame::Welcome { version, .. } => {
			inner.version.store(version, Ordering::Relaxed);
		}
		ServerFrame::Transition(transition) => apply_transition(inner, transition),
		ServerFrame::Done {
			request,
			ok,
			err,
			version,
		} => {
			inner.version.store(version, Ordering::Relaxed);
			finish(inner, request, ok, err);
		}
		ServerFrame::AuthStatus { refused, .. } => refuse(inner, refused),
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
		ServerFrame::Item { .. }
		| ServerFrame::End { .. }
		| ServerFrame::Pong
		| ServerFrame::Unknown => {}
	}
}

/// Hands each changed result to the subscriber watching for it.
fn apply_transition(inner: &Inner, transition: Transition) {
	inner.version.store(transition.to, Ordering::Relaxed);
	let subscribers = inner.subscribers();

	for update in transition.updates {
		if let Some(subscriber) = subscribers.get(&update.id) {
			let _ = subscriber.sender.unbounded_send(update.update);
		}
	}
}

/// Answers a finished call, or sends a subscription's first value.
fn finish(inner: &Inner, request: u32, ok: Option<Value>, err: Option<WireError>) {
	let Some(sender) = inner.pending().remove(&request) else {
		if let Some(subscriber) = inner.subscribers().get(&request)
			&& let Some(value) = ok
		{
			// A subscription's first value.
			let _ = subscriber.sender.unbounded_send(Update::Ok(value));
		}

		return;
	};
	inner.mutations().remove(&request);

	let answer = match err {
		Some(error) => Err(error),
		None => Ok(ok.unwrap_or(Value::Null)),
	};

	let _ = sender.send(answer);
}

/// Refused credentials make every result an error until the app signs in
/// again.
fn refuse(inner: &Inner, refused: bool) {
	if !refused {
		return;
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

async fn resubscribe(inner: &Inner, connection: &dyn Connection) {
	let frames: Vec<ClientFrame> = inner
		.subscribers()
		.iter()
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

async fn resend_mutations(inner: &Inner, connection: &dyn Connection) {
	let frames: Vec<ClientFrame> = inner
		.mutations()
		.iter()
		.map(|(&id, mutation)| {
			ClientFrame::Mutate {
				request: id,
				name: mutation.name.clone(),
				args: mutation.args.clone(),
				key: mutation.key.clone(),
			}
		})
		.collect();

	for frame in frames {
		if !send(connection, &frame).await {
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

/// Whether this build can hold a live connection.
#[must_use]
pub fn available() -> bool {
	CLIENT.get().is_some()
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
	Some(Arc::new(Socket {
		url: socket_url(&origin),
	}))
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

fn live_here() -> bool {
	cfg!(any(
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
	}

	impl Memory {
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
			let this = self.clone();
			Box::pin(async move { Ok(Box::new(this) as Box<dyn Connection>) })
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
						resume: Some(Resume { version: 0 })
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

	#[tokio::test]
	async fn a_mutation_resolves_from_its_done_frame() {
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
				assert_eq!(answer.await.unwrap(), json!("0190"));
				assert_eq!(client.version(), 3);
			})
			.await;
	}
}
