//! The live connection: one WebSocket to the server's live endpoint,
//! carrying subscriptions, live calls and channels, kept up across
//! reconnects.
//!
//! It runs on Tokio. The first subscription, live call or join starts a task
//! that owns the socket, so make them from inside a Tokio runtime.

use std::collections::BTreeMap;
use std::collections::hash_map::RandomState;
use std::future::Future;
use std::hash::BuildHasher;
use std::hash::Hasher;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::task::Poll;
use std::time::Duration;

use futures_util::SinkExt;
use futures_util::Stream;
use futures_util::StreamExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderName;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::SEC_WEBSOCKET_PROTOCOL;

/// The WebSocket subprotocol the server speaks.
pub const LIVE_SUBPROTOCOL: &str = "leptoskit.live.v1.json";

/// How long opening the socket may take before it counts as a failure.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// A failure over the live connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveError {
	/// The HTTP status the same failure would carry; `0` when the connection
	/// closed or a value didn't decode.
	pub status: u16,
	/// The human-readable message.
	pub message: String,
}

impl std::fmt::Display for LiveError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str(&self.message)
	}
}

impl std::error::Error for LiveError {}

/// Where the connection is, for a "reconnecting…" banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveStatus {
	/// Opening the socket.
	Connecting {
		/// How many tries have failed since the connection was last live.
		attempt: u32,
	},
	/// Connected and signed in.
	Live {
		/// The last version the connection applied.
		version: u64,
		/// The server's name for the connection, for logs.
		connection: String,
	},
	/// Lost, and trying again after `next_in`.
	Reconnecting {
		/// How many tries have failed since the connection was last live.
		attempt: u32,
		/// How long until the next try.
		next_in: Duration,
	},
	/// Not connected, and not trying: nothing needs the connection, or it
	/// was closed.
	Offline,
}

/// Someone in a channel, and the presence they joined with.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
pub struct PresenceMember<P> {
	/// Who: the caller id the server knows them by.
	pub id: String,
	/// What they show the others.
	pub state: P,
}

/// What a joined channel delivers.
#[derive(Debug, Clone, PartialEq)]
pub enum ChannelEvent<M, P> {
	/// Another member's signal, and who sent it when the server can say.
	Signal {
		/// The message.
		message: M,
		/// The sender's caller id.
		from: Option<String>,
	},
	/// Everyone in the channel, after a join or a leave.
	Presence(Vec<PresenceMember<P>>),
	/// The server refused the join, or a value didn't decode.
	Error(LiveError),
}

type TokenFuture = Pin<Box<dyn Future<Output = Option<String>> + Send>>;
type TokenProvider = Arc<dyn Fn() -> TokenFuture + Send + Sync>;

/// How the live connection signs in and reconnects.
#[derive(Clone)]
pub struct LiveOptions {
	token: Option<TokenProvider>,
	on_auth_refused: Option<Arc<dyn Fn() + Send + Sync>>,
	read_your_writes: Duration,
	min_backoff: Duration,
	max_backoff: Duration,
}

impl Default for LiveOptions {
	fn default() -> Self {
		Self {
			token: None,
			on_auth_refused: None,
			read_your_writes: Duration::from_secs(2),
			min_backoff: Duration::from_millis(250),
			max_backoff: Duration::from_secs(30),
		}
	}
}

impl std::fmt::Debug for LiveOptions {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("LiveOptions")
			.field("token", &self.token.is_some())
			.field("read_your_writes", &self.read_your_writes)
			.field("min_backoff", &self.min_backoff)
			.field("max_backoff", &self.max_backoff)
			.finish_non_exhaustive()
	}
}

impl LiveOptions {
	/// The defaults: no token, a 2-second read-your-writes wait, and
	/// backoff from 250 milliseconds up to 30 seconds.
	pub fn new() -> Self {
		Self::default()
	}

	/// Signs in with the bearer token `token` returns, such as a native
	/// session's access token. It's called on every connect, and again when
	/// the server says the token is about to expire.
	pub fn token(self, token: impl Fn() -> Option<String> + Send + Sync + 'static) -> Self {
		self.token_async(move || std::future::ready(token()))
	}

	/// [`LiveOptions::token`], for a token that takes a request to renew.
	pub fn token_async<F, Fut>(mut self, token: F) -> Self
	where
		F: Fn() -> Fut + Send + Sync + 'static,
		Fut: Future<Output = Option<String>> + Send + 'static,
	{
		self.token = Some(Arc::new(move || Box::pin(token()) as TokenFuture));
		self
	}

	/// Called when the server refuses the token.
	pub fn on_auth_refused(mut self, refused: impl Fn() + Send + Sync + 'static) -> Self {
		self.on_auth_refused = Some(Arc::new(refused));
		self
	}

	/// How long a live call waits for its own write to reach this
	/// connection's subscriptions before it returns anyway.
	pub fn read_your_writes(mut self, wait: Duration) -> Self {
		self.read_your_writes = wait;
		self
	}

	/// The first reconnect delay's upper bound, and the longest delay.
	/// Each failed try doubles the bound, and the delay is random below it.
	pub fn backoff(mut self, min: Duration, max: Duration) -> Self {
		self.min_backoff = min;
		self.max_backoff = max;
		self
	}
}

/// One live connection, opened on first use and kept up while anything
/// uses it: the client it belongs to, a subscription, a channel, or a live
/// call on its way. Clones share it.
#[derive(Clone)]
pub struct LiveConnection {
	shared: Arc<Shared>,
	_owner: Arc<Owner>,
}

impl std::fmt::Debug for LiveConnection {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("LiveConnection")
			.field("url", &self.shared.url)
			.field("status", &*self.shared.status.borrow())
			.finish_non_exhaustive()
	}
}

/// Closes the connection when the last handle goes.
struct Owner(Arc<Shared>);

impl Drop for Owner {
	fn drop(&mut self) {
		self.0.close();
	}
}

struct Shared {
	url: String,
	headers: Vec<(String, String)>,
	options: LiveOptions,
	state: Mutex<State>,
	status: watch::Sender<LiveStatus>,
	stop: tokio::sync::Notify,
}

#[derive(Default)]
struct State {
	next_id: u32,
	next_ticket: u64,
	version: u64,
	subscriptions: BTreeMap<u32, Watched>,
	pending: BTreeMap<u32, Pending>,
	waiters: Vec<(u64, oneshot::Sender<()>)>,
	channels: BTreeMap<String, Joined>,
	/// Where frames for the open socket go; `None` while it's down.
	writer: Option<mpsc::UnboundedSender<String>>,
	running: bool,
	closed: bool,
	attempt: u32,
	reconnect_after: Option<Duration>,
}

struct Watched {
	query: String,
	args: Value,
	value: Option<Value>,
	sender: mpsc::UnboundedSender<Result<Value, LiveError>>,
}

struct Pending {
	frame: String,
	reply: oneshot::Sender<Reply>,
}

/// What a live call's `done` turned into: its result now, or once the
/// subscriptions reach its version.
enum Reply {
	Now(Result<Value, LiveError>),
	Wait(Value, oneshot::Receiver<()>),
}

struct Joined {
	name: String,
	args: Value,
	state: Value,
	members: Vec<PresenceMember<Value>>,
	ticket: u64,
	sender: mpsc::UnboundedSender<ChannelEvent<Value, Value>>,
}

type Socket =
	tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl LiveConnection {
	/// A connection to `url`, such as `wss://api.example.com/live`, whose
	/// upgrade request carries `headers`. Nothing opens until it's used.
	pub fn new(
		url: impl Into<String>,
		headers: Vec<(String, String)>,
		options: LiveOptions,
	) -> Self {
		let shared = Arc::new(Shared {
			url: url.into(),
			headers,
			options,
			state: Mutex::new(State::default()),
			status: watch::channel(LiveStatus::Offline).0,
			stop: tokio::sync::Notify::new(),
		});
		Self {
			_owner: Arc::new(Owner(Arc::clone(&shared))),
			shared,
		}
	}

	/// The URL the connection opens.
	pub fn url(&self) -> &str {
		&self.shared.url
	}

	/// Where the connection is now.
	pub fn status(&self) -> LiveStatus {
		self.shared.status.borrow().clone()
	}

	/// Where the connection is, now and as it changes.
	pub fn watch_status(&self) -> watch::Receiver<LiveStatus> {
		self.shared.status.subscribe()
	}

	/// The last version the connection applied.
	pub fn version(&self) -> u64 {
		self.shared.lock().version
	}

	/// Watches the query `name`: the subscription yields the first value and
	/// each new one, and unsubscribes when dropped. A failure arrives as an
	/// `Err`, and the subscription recovers on the next change.
	pub fn subscribe<T: DeserializeOwned>(
		&self,
		name: &str,
		args: &impl Serialize,
	) -> Subscription<T> {
		let (sender, receiver) = mpsc::unbounded_channel();
		let mut id = 0;

		match arguments(args) {
			Ok(args) => {
				let mut state = self.shared.lock();

				if !state.closed {
					id = state.next_id();
					state.send(&json!({ "type": "sub", "id": id, "query": name, "args": args }));
					state.subscriptions.insert(
						id,
						Watched {
							query: name.to_owned(),
							args,
							value: None,
							sender,
						},
					);
					self.shared.ensure(&mut state);
				}
			}
			Err(error) => {
				let _ = sender.send(Err(error));
			}
		}

		Subscription {
			connection: self.clone(),
			id,
			receiver,
			_value: PhantomData,
		}
	}

	/// Runs the mutation `name` over the socket. It returns once this
	/// connection's subscriptions show the write (read-your-writes), or after
	/// the read-your-writes wait if none of them changes. A call made while
	/// the connection is down waits for it, and one whose answer is lost is
	/// sent again with the same request id and `key`, so it runs once.
	pub fn mutate<T: DeserializeOwned>(
		&self,
		name: &str,
		args: &impl Serialize,
		key: Option<String>,
	) -> impl Future<Output = Result<T, LiveError>> + Send + 'static {
		let connection = self.clone();
		let name = name.to_owned();
		let args = arguments(args);

		async move {
			let args = args?;
			let (reply, answer) = oneshot::channel();
			let request = {
				let mut state = connection.shared.lock();

				if state.closed {
					return Err(closed());
				}

				let request = state.next_id();
				let frame = json!({
					"type": "mutate",
					"request": request,
					"name": name,
					"args": args,
					"key": key.unwrap_or_else(random_key),
				})
				.to_string();
				state.send_text(&frame);
				state.pending.insert(request, Pending { frame, reply });
				connection.shared.ensure(&mut state);
				request
			};
			// A call dropped before its answer isn't sent again.
			let _forget = Forget {
				shared: Arc::clone(&connection.shared),
				request,
			};
			let value = match answer.await.map_err(|_| closed())? {
				Reply::Now(result) => result?,
				Reply::Wait(value, seen) => {
					let _ = tokio::time::timeout(connection.shared.options.read_your_writes, seen)
						.await;
					value
				}
			};
			serde_json::from_value(value).map_err(decode_error)
		}
	}

	/// Joins the channel `name`, whose join arguments `args` have the wire
	/// names `fields` in declaration order. `presence` is what this
	/// connection shows the others, or the channel's default when `None`.
	/// The join is made again after a reconnect, until the channel is left
	/// or dropped.
	pub fn join<M, P: Serialize>(
		&self,
		name: &str,
		fields: &[&str],
		args: &impl Serialize,
		presence: Option<&P>,
	) -> Channel<M, P> {
		let (sender, receiver) = mpsc::unbounded_channel();
		let mut key = String::new();
		let mut ticket = 0;
		let state_value = presence
			.map(|presence| serde_json::to_value(presence).map_err(decode_error))
			.transpose();

		match (arguments(args), state_value) {
			(Ok(args), Ok(presence)) => {
				key = channel_key(fields, &args);
				let mut state = self.shared.lock();

				if !state.closed {
					state.next_ticket += 1;
					ticket = state.next_ticket;
					let joined = Joined {
						name: name.to_owned(),
						args,
						state: presence.unwrap_or(Value::Null),
						members: Vec::new(),
						ticket,
						sender,
					};
					state.send(&joined.frame(&key));
					state.channels.insert(key.clone(), joined);
					self.shared.ensure(&mut state);
				}
			}
			(Err(error), _) | (_, Err(error)) => {
				let _ = sender.send(ChannelEvent::Error(error));
			}
		}

		Channel {
			connection: self.clone(),
			key,
			ticket,
			receiver,
			_types: PhantomData,
		}
	}

	/// Sends the token again, such as after signing in.
	pub async fn reauthenticate(&self) {
		reauthenticate(Arc::clone(&self.shared)).await;
	}

	/// Closes the connection for good: live calls on their way fail, and
	/// subscriptions and channels end.
	pub fn close(&self) {
		self.shared.close();
	}
}

/// Removes a live call that was dropped before it finished.
struct Forget {
	shared: Arc<Shared>,
	request: u32,
}

impl Drop for Forget {
	fn drop(&mut self) {
		self.shared.lock().pending.remove(&self.request);
	}
}

/// A watched query: a [`Stream`] of its first value and every new one.
/// Dropping it unsubscribes.
pub struct Subscription<T> {
	connection: LiveConnection,
	id: u32,
	receiver: mpsc::UnboundedReceiver<Result<Value, LiveError>>,
	_value: PhantomData<fn() -> T>,
}

impl<T: DeserializeOwned> Subscription<T> {
	/// Waits for the next value; `None` once the connection closes.
	pub async fn next(&mut self) -> Option<Result<T, LiveError>> {
		StreamExt::next(self).await
	}

	/// The newest value the connection holds, without waiting: `None`
	/// before the first arrives.
	pub fn current(&self) -> Option<Result<T, LiveError>> {
		let state = self.connection.shared.lock();
		let value = state.subscriptions.get(&self.id)?.value.clone()?;
		Some(serde_json::from_value(value).map_err(decode_error))
	}
}

impl<T: DeserializeOwned> Stream for Subscription<T> {
	type Item = Result<T, LiveError>;

	fn poll_next(
		mut self: Pin<&mut Self>,
		context: &mut std::task::Context<'_>,
	) -> Poll<Option<Self::Item>> {
		self.receiver.poll_recv(context).map(|item| {
			item.map(|result| {
				result.and_then(|value| serde_json::from_value(value).map_err(decode_error))
			})
		})
	}
}

impl<T> Drop for Subscription<T> {
	fn drop(&mut self) {
		let mut state = self.connection.shared.lock();

		if state.subscriptions.remove(&self.id).is_some() {
			state.send(&json!({ "type": "unsub", "id": self.id }));
		}
	}
}

/// A joined channel: a [`Stream`] of signals and presence. Dropping it
/// leaves.
pub struct Channel<M, P> {
	connection: LiveConnection,
	key: String,
	ticket: u64,
	receiver: mpsc::UnboundedReceiver<ChannelEvent<Value, Value>>,
	_types: PhantomData<fn() -> (M, P)>,
}

impl<M: Serialize + DeserializeOwned, P: Serialize + DeserializeOwned> Channel<M, P> {
	/// The channel instance's name: its join arguments as compact JSON.
	pub fn key(&self) -> &str {
		&self.key
	}

	/// Everyone in the channel, as last reported.
	pub fn members(&self) -> Vec<PresenceMember<P>> {
		let state = self.connection.shared.lock();
		state
			.channels
			.get(&self.key)
			.filter(|joined| joined.ticket == self.ticket)
			.map(|joined| decode_members(&joined.members))
			.unwrap_or_default()
	}

	/// Sends a signal to everyone else in the channel, at most once: a
	/// signal sent while the connection is down is lost.
	pub fn signal(&self, message: &M) {
		let Ok(message) = serde_json::to_value(message) else {
			return;
		};
		let state = self.connection.shared.lock();

		if self.is_current(&state) {
			state.send(&json!({ "type": "signal", "channel": self.key, "message": message }));
		}
	}

	/// Changes what this connection shows the others.
	pub fn set_presence(&self, presence: &P) {
		let Ok(presence) = serde_json::to_value(presence) else {
			return;
		};
		let mut state = self.connection.shared.lock();

		if self.is_current(&state) {
			let frame = state.channels.get_mut(&self.key).map(|joined| {
				joined.state = presence;
				joined.frame(&self.key)
			});

			if let Some(frame) = frame {
				state.send(&frame);
			}
		}
	}

	/// Leaves the channel; dropping it does the same.
	pub fn leave(self) {}

	/// Waits for the next signal or presence; `None` once the channel is
	/// left elsewhere or the connection closes.
	pub async fn next(&mut self) -> Option<ChannelEvent<M, P>> {
		StreamExt::next(self).await
	}

	fn is_current(&self, state: &State) -> bool {
		state
			.channels
			.get(&self.key)
			.is_some_and(|joined| joined.ticket == self.ticket)
	}
}

impl<M: DeserializeOwned, P: DeserializeOwned> Stream for Channel<M, P> {
	type Item = ChannelEvent<M, P>;

	fn poll_next(
		mut self: Pin<&mut Self>,
		context: &mut std::task::Context<'_>,
	) -> Poll<Option<Self::Item>> {
		self.receiver.poll_recv(context).map(|event| {
			event.map(|event| {
				match event {
					ChannelEvent::Signal { message, from } => {
						match serde_json::from_value(message) {
							Ok(message) => ChannelEvent::Signal { message, from },
							Err(error) => ChannelEvent::Error(decode_error(error)),
						}
					}
					ChannelEvent::Presence(members) => {
						ChannelEvent::Presence(decode_members(&members))
					}
					ChannelEvent::Error(error) => ChannelEvent::Error(error),
				}
			})
		})
	}
}

impl<M, P> Drop for Channel<M, P> {
	fn drop(&mut self) {
		let mut state = self.connection.shared.lock();

		if state
			.channels
			.get(&self.key)
			.is_some_and(|joined| joined.ticket == self.ticket)
		{
			state.channels.remove(&self.key);
			state.send(&json!({ "type": "leave", "channel": self.key }));
		}
	}
}

impl Joined {
	fn frame(&self, key: &str) -> Value {
		json!({
			"type": "join",
			"channel": key,
			"name": self.name,
			"args": self.args,
			"state": self.state,
		})
	}
}

impl State {
	/// The next request or subscription id: they share one counter, because
	/// a subscription's first value arrives as a `done` for its id. Never 0.
	fn next_id(&mut self) -> u32 {
		self.next_id = self.next_id.checked_add(1).unwrap_or(1);
		self.next_id
	}

	fn send(&self, frame: &Value) {
		self.send_text(&frame.to_string());
	}

	fn send_text(&self, frame: &str) {
		if let Some(writer) = &self.writer {
			let _ = writer.send(frame.to_owned());
		}
	}

	fn busy(&self) -> bool {
		!self.subscriptions.is_empty() || !self.pending.is_empty() || !self.channels.is_empty()
	}

	/// Applies one transition: every update, then every value, then the
	/// version, so no subscription sees a state the others don't.
	fn transition(&mut self, frame: &Value) {
		let to = number(&frame["to"]).unwrap_or(self.version);
		let mut changed = Vec::new();
		let mut failed = Vec::new();

		for update in frame["updates"].as_array().into_iter().flatten() {
			let Some(id) = number(&update["id"]).and_then(|id| u32::try_from(id).ok()) else {
				continue;
			};
			let Some(watched) = self.subscriptions.get(&id) else {
				continue;
			};

			if let Some(value) = update.get("ok") {
				changed.push((id, value.clone()));
			} else if let Some(patch) = update.get("patch") {
				changed.push((id, apply_patch(watched.value.as_ref(), patch)));
			} else if let Some(error) = update.get("err") {
				failed.push((id, wire_error(error)));
			}
		}

		self.version = self.version.max(to);

		for (id, value) in &changed {
			if let Some(watched) = self.subscriptions.get_mut(id) {
				watched.value = Some(value.clone());
			}
		}

		for (id, value) in changed {
			if let Some(watched) = self.subscriptions.get(&id) {
				let _ = watched.sender.send(Ok(value));
			}
		}

		for (id, error) in failed {
			if let Some(watched) = self.subscriptions.get(&id) {
				let _ = watched.sender.send(Err(error));
			}
		}

		self.reached(to);
	}

	/// A live call finished, or a subscription got its first value.
	fn done(&mut self, frame: &Value) {
		let Some(request) = number(&frame["request"]).and_then(|id| u32::try_from(id).ok()) else {
			return;
		};
		let version = number(&frame["version"]).unwrap_or(0);
		let failure = frame
			.get("err")
			.filter(|error| !error.is_null())
			.map(wire_error);
		let value = frame.get("ok").cloned().unwrap_or(Value::Null);

		if let Some(pending) = self.pending.remove(&request) {
			let reply = match failure {
				Some(error) => Reply::Now(Err(error)),
				None if self.subscriptions.is_empty() || self.version >= version => {
					Reply::Now(Ok(value))
				}
				None => {
					let (seen, wait) = oneshot::channel();
					self.waiters.retain(|(_, waiter)| !waiter.is_closed());
					self.waiters.push((version, seen));
					Reply::Wait(value, wait)
				}
			};
			let _ = pending.reply.send(reply);
			return;
		}

		let Some(watched) = self.subscriptions.get_mut(&request) else {
			return;
		};

		match failure {
			Some(error) => {
				let _ = watched.sender.send(Err(error));
			}
			None => {
				watched.value = Some(value.clone());
				let _ = watched.sender.send(Ok(value));
				// A fresh first value reflects every write the server had
				// committed.
				self.reached(version);
			}
		}
	}

	/// Lets every live call waiting for `version` or older return.
	fn reached(&mut self, version: u64) {
		let (ready, waiting) = std::mem::take(&mut self.waiters)
			.into_iter()
			.partition(|(wanted, _)| *wanted <= version);
		self.waiters = waiting;

		for (_, waiter) in ready {
			let _ = waiter.send(());
		}
	}
}

impl Shared {
	fn lock(&self) -> MutexGuard<'_, State> {
		self.state.lock().unwrap_or_else(PoisonError::into_inner)
	}

	/// Starts the connection's task, unless it's running or closed.
	fn ensure(self: &Arc<Self>, state: &mut State) {
		if !state.running && !state.closed {
			state.running = true;
			tokio::spawn(run(Arc::clone(self)));
		}
	}

	fn close(&self) {
		let mut state = self.lock();

		if state.closed {
			return;
		}

		state.closed = true;
		state.writer = None;
		state.pending.clear();
		state.waiters.clear();
		state.subscriptions.clear();
		state.channels.clear();
		self.status.send_replace(LiveStatus::Offline);
		drop(state);
		self.stop.notify_one();
	}

	async fn connect(&self) -> Option<Socket> {
		let mut request = self.url.as_str().into_client_request().ok()?;
		let headers = request.headers_mut();

		for (name, value) in &self.headers {
			if let (Ok(name), Ok(value)) = (
				HeaderName::from_bytes(name.as_bytes()),
				HeaderValue::from_str(value),
			) {
				headers.insert(name, value);
			}
		}

		headers.insert(
			SEC_WEBSOCKET_PROTOCOL,
			HeaderValue::from_static(LIVE_SUBPROTOCOL),
		);
		let connected =
			tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request)).await;
		connected.ok()?.ok().map(|(socket, _)| socket)
	}

	/// Says hello, signs in, sends everything the connection holds, then
	/// carries frames both ways until the socket goes.
	async fn session(self: &Arc<Self>, socket: Socket) {
		let (mut sink, mut stream) = socket.split();
		let version = self.lock().version;
		let hello = if version > 0 {
			json!({ "type": "hello", "resume": { "version": version } })
		} else {
			json!({ "type": "hello" })
		};

		if sink.send(Message::text(hello.to_string())).await.is_err() {
			return;
		}

		let token = match &self.options.token {
			Some(token) => token().await,
			None => None,
		};
		let (writer, mut outgoing) = mpsc::unbounded_channel();

		{
			let mut state = self.lock();

			if state.closed {
				return;
			}

			if let Some(token) = token {
				let _ = writer.send(json!({ "type": "auth", "token": token }).to_string());
			}

			for (id, watched) in &state.subscriptions {
				let frame = json!({ "type": "sub", "id": id, "query": watched.query, "args": watched.args });
				let _ = writer.send(frame.to_string());
			}

			for (key, joined) in &state.channels {
				let _ = writer.send(joined.frame(key).to_string());
			}

			// The same request id and key: the server's receipt answers a
			// call that already ran with its first result.
			for pending in state.pending.values() {
				let _ = writer.send(pending.frame.clone());
			}

			state.writer = Some(writer);
		}

		let mut period = Duration::from_secs(30);
		let mut heartbeat = tokio::time::interval_at(Instant::now() + period, period);
		let mut heard = Instant::now();

		loop {
			tokio::select! {
				frame = outgoing.recv() => {
					let Some(frame) = frame else {
						// Closed for good.
						let _ = sink.close().await;
						return;
					};

					if sink.send(Message::text(frame)).await.is_err() {
						return;
					}
				}
				message = stream.next() => {
					let text = match message {
						Some(Ok(Message::Text(text))) => Some(text.as_str().to_owned()),
						Some(Ok(Message::Binary(bytes))) => String::from_utf8(bytes.to_vec()).ok(),
						Some(Ok(Message::Close(_)) | Err(_)) | None => return,
						Some(Ok(_)) => None,
					};
					heard = Instant::now();

					if let Some(seconds) = text.and_then(|text| self.receive(&text)) {
						period = Duration::from_secs(seconds.max(1));
						heartbeat = tokio::time::interval_at(Instant::now() + period, period);
					}
				}
				_ = heartbeat.tick() => {
					if heard.elapsed() > 2 * period + Duration::from_secs(5) {
						// Two missed heartbeats: the connection is gone
						// without saying so.
						return;
					}

					if sink.send(Message::text(r#"{"type":"ping"}"#)).await.is_err() {
						return;
					}
				}
			}
		}
	}

	/// Handles one frame from the server; returns the heartbeat period, in
	/// seconds, when the frame is a `welcome`.
	fn receive(self: &Arc<Self>, text: &str) -> Option<u64> {
		let frame: Value = serde_json::from_str(text).ok()?;
		let mut state = self.lock();

		match frame["type"].as_str().unwrap_or_default() {
			"welcome" => {
				state.attempt = 0;
				state.version = number(&frame["version"]).unwrap_or(state.version);
				self.status.send_replace(LiveStatus::Live {
					version: state.version,
					connection: frame["connection"].as_str().unwrap_or_default().to_owned(),
				});
				return Some(number(&frame["heartbeat_seconds"]).unwrap_or(30));
			}
			"transition" => state.transition(&frame),
			"done" => state.done(&frame),
			"auth_status" => {
				drop(state);

				if frame["refused"] == true {
					if let Some(refused) = &self.options.on_auth_refused {
						refused();
					}
				} else if frame["expiring"] == true {
					tokio::spawn(reauthenticate(Arc::clone(self)));
				}
			}
			"reconnect" => {
				state.reconnect_after = Some(Duration::from_millis(
					number(&frame["after_ms"]).unwrap_or(0),
				));
			}
			"signal" => {
				if let Some(joined) = state
					.channels
					.get(frame["channel"].as_str().unwrap_or_default())
				{
					let _ = joined.sender.send(ChannelEvent::Signal {
						message: frame["message"].clone(),
						from: frame["from"].as_str().map(str::to_owned),
					});
				}
			}
			"presence" => {
				let channel = frame["channel"].as_str().unwrap_or_default();

				if let (Some(joined), Some(members)) =
					(state.channels.get_mut(channel), frame["members"].as_array())
				{
					joined.members = members
						.iter()
						.map(|member| {
							PresenceMember {
								id: member["id"].as_str().unwrap_or_default().to_owned(),
								state: member["state"].clone(),
							}
						})
						.collect();
					let _ = joined
						.sender
						.send(ChannelEvent::Presence(joined.members.clone()));
				}
			}
			"channel_error" => {
				if let Some(joined) = state
					.channels
					.get(frame["channel"].as_str().unwrap_or_default())
				{
					let _ = joined
						.sender
						.send(ChannelEvent::Error(wire_error(&frame["err"])));
				}
			}
			// `pong`, and frames a newer server sends that this runtime
			// doesn't know.
			_ => {}
		}

		None
	}
}

/// The connection's task: connects, serves, and reconnects with backoff
/// while anything needs the connection.
async fn run(shared: Arc<Shared>) {
	loop {
		{
			let state = shared.lock();

			if state.closed {
				return;
			}

			shared.status.send_replace(LiveStatus::Connecting {
				attempt: state.attempt,
			});
		}

		if let Some(socket) = shared.connect().await {
			shared.session(socket).await;
		}

		let delay = {
			let mut state = shared.lock();
			state.writer = None;

			if state.closed {
				return;
			}

			if !state.busy() {
				state.running = false;
				state.reconnect_after = None;
				shared.status.send_replace(LiveStatus::Offline);
				return;
			}

			let delay = state.reconnect_after.take().unwrap_or_else(|| {
				backoff(
					shared.options.min_backoff,
					shared.options.max_backoff,
					state.attempt,
				)
			});
			state.attempt = state.attempt.saturating_add(1);
			shared.status.send_replace(LiveStatus::Reconnecting {
				attempt: state.attempt,
				next_in: delay,
			});
			delay
		};

		tokio::select! {
			() = tokio::time::sleep(delay) => {}
			() = shared.stop.notified() => {}
		}
	}
}

async fn reauthenticate(shared: Arc<Shared>) {
	let Some(token) = &shared.options.token else {
		return;
	};

	if let Some(token) = token().await {
		shared
			.lock()
			.send(&json!({ "type": "auth", "token": token }));
	}
}

/// A random delay below `min` doubled `attempt` times, at most `max`: full
/// jitter, so clients that lost the server together don't return together.
fn backoff(min: Duration, max: Duration, attempt: u32) -> Duration {
	let bound = min
		.saturating_mul(2_u32.saturating_pow(attempt.min(20)))
		.min(max);
	let nanos = u64::try_from(bound.as_nanos()).unwrap_or(u64::MAX);

	if nanos == 0 {
		return Duration::ZERO;
	}

	Duration::from_nanos(random() % nanos)
}

/// Applies a list patch or a page's row and metadata changes together.
fn apply_patch(value: Option<&Value>, patch: &Value) -> Value {
	let Some(field) = patch["field"].as_str() else {
		return apply_rows(value, patch);
	};
	let Some(mut fields) = value.and_then(Value::as_object).cloned() else {
		return value.cloned().unwrap_or(Value::Null);
	};

	if let Some(changed) = patch["set"].as_object() {
		fields.extend(changed.clone());
	}

	if let Some(items) = fields.get_mut(field) {
		*items = apply_rows(Some(items), patch);
	}

	Value::Object(fields)
}

/// Removes rows first, then replaces matching ids or inserts new rows.
fn apply_rows(value: Option<&Value>, patch: &Value) -> Value {
	let Some(Value::Array(items)) = value else {
		return value.cloned().unwrap_or(Value::Null);
	};
	let remove: Vec<&str> = patch["remove"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(Value::as_str)
		.collect();
	let mut items: Vec<Value> = items
		.iter()
		.filter(|item| item["id"].as_str().is_none_or(|id| !remove.contains(&id)))
		.cloned()
		.collect();

	for upsert in patch["upsert"].as_array().into_iter().flatten() {
		let at = number(&upsert["at"])
			.and_then(|at| usize::try_from(at).ok())
			.unwrap_or(items.len())
			.min(items.len());
		let id = upsert["value"]["id"].as_str();

		match items.get_mut(at) {
			Some(slot) if id.is_some() && slot["id"].as_str() == id => {
				*slot = upsert["value"].clone();
			}
			_ => items.insert(at, upsert["value"].clone()),
		}
	}

	Value::Array(items)
}

/// The name of a channel instance: its join arguments as compact JSON, with
/// the fields in the order the server declares them, as the app's own client
/// writes it, so every client of the same instance meets in one room.
fn channel_key(fields: &[&str], args: &Value) -> String {
	let object = args.as_object();
	let mut parts = Vec::new();

	for field in fields {
		let value = object
			.and_then(|object| object.get(*field))
			.unwrap_or(&Value::Null);
		parts.push(format!("{}:{value}", Value::from(*field)));
	}

	for (field, value) in object.into_iter().flatten() {
		if !fields.contains(&field.as_str()) {
			parts.push(format!("{}:{value}", Value::from(field.as_str())));
		}
	}

	format!("{{{}}}", parts.join(","))
}

/// The arguments as one JSON object without absent optional fields, as the
/// HTTP calls send them.
fn arguments(args: &impl Serialize) -> Result<Value, LiveError> {
	match serde_json::to_value(args).map_err(decode_error)? {
		Value::Null => Ok(json!({})),
		Value::Object(fields) => {
			Ok(Value::Object(
				fields
					.into_iter()
					.filter(|(_, item)| !item.is_null())
					.collect(),
			))
		}
		other => Ok(other),
	}
}

fn decode_members<P: DeserializeOwned>(
	members: &[PresenceMember<Value>],
) -> Vec<PresenceMember<P>> {
	members
		.iter()
		.filter_map(|member| {
			Some(PresenceMember {
				id: member.id.clone(),
				state: serde_json::from_value(member.state.clone()).ok()?,
			})
		})
		.collect()
}

fn wire_error(error: &Value) -> LiveError {
	LiveError {
		status: number(&error["status"])
			.and_then(|status| u16::try_from(status).ok())
			.unwrap_or(500),
		message: error["message"]
			.as_str()
			.unwrap_or("the server failed")
			.to_owned(),
	}
}

fn decode_error(error: serde_json::Error) -> LiveError {
	LiveError {
		status: 0,
		message: format!("the value didn't decode: {error}"),
	}
}

fn closed() -> LiveError {
	LiveError {
		status: 0,
		message: "the live connection closed".to_owned(),
	}
}

fn number(value: &Value) -> Option<u64> {
	value.as_u64()
}

/// 64 random bits, from the standard library's randomly keyed hasher.
fn random() -> u64 {
	static COUNTER: AtomicU64 = AtomicU64::new(0);
	let mut hasher = RandomState::new().build_hasher();
	hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
	hasher.write_u128(
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map_or(0, |elapsed| elapsed.as_nanos()),
	);
	hasher.finish()
}

/// A fresh idempotency key: 128 random bits as hex.
fn random_key() -> String {
	format!("{:016x}{:016x}", random(), random())
}
