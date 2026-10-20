use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Weak;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error as BridgeError;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Reply;
use leptoskit_bridge::ReplyGuard;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use leptoskit_bridge::async_host::PluginFuture;
use serde::Deserialize;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

use crate::Client;
use crate::Config;
use crate::Connection;
use crate::Error;
use crate::Message;
use crate::protocol;
use crate::protocol::ConnectionId;

type Answer = oneshot::Sender<Result<Reply, BridgeError>>;
#[derive(Clone)]
struct Entry {
	commands: mpsc::Sender<Command>,
	closed: CancellationToken,
	done: CancellationToken,
}
struct State {
	entries: Mutex<BTreeMap<u32, Entry>>,
	connections: Arc<Semaphore>,
	queued_bytes: Arc<Semaphore>,
	next: AtomicU32,
	closed: CancellationToken,
}
/// Host-scoped sockets with bounded connections, command queues and bytes.
pub struct WebSocketPlugin {
	client: Client,
	state: Arc<State>,
	config: Config,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
	origins: Vec<String>,
	#[serde(default = "connections")]
	max_connections: usize,
	#[serde(default = "message_limit")]
	max_message_bytes: usize,
	#[serde(default = "pending_limit")]
	max_pending_bytes: usize,
	#[serde(default = "timeout")]
	timeout_ms: u64,
}
fn connections() -> usize {
	8
}
fn message_limit() -> usize {
	1024 * 1024
}
fn pending_limit() -> usize {
	4 * 1024 * 1024
}
fn timeout() -> u64 {
	10_000
}
/// Parses strict host-only options. Exact WS/WSS origins are mandatory.
pub fn from_config(options_json: &str) -> Result<WebSocketPlugin, BridgeError> {
	let options: Options =
		serde_json::from_str(options_json).map_err(|_| BridgeError::InvalidArgument)?;
	if options.max_connections == 0
		|| options.max_connections > 128
		|| options.max_message_bytes > 8 * 1024 * 1024 - 8
		|| options.max_pending_bytes > 32 * 1024 * 1024
	{
		return Err(BridgeError::InvalidArgument);
	}
	let mut config = Config::new(options.origins).map_err(BridgeError::from)?;
	config.max_message_bytes = options.max_message_bytes;
	config.max_pending_bytes = options.max_pending_bytes;
	config.connect_timeout = Duration::from_millis(options.timeout_ms);
	config.send_timeout = config.connect_timeout;
	Ok(WebSocketPlugin {
		client: Client::new(config.clone()).map_err(BridgeError::from)?,
		config,
		state: Arc::new(State {
			entries: Mutex::new(BTreeMap::new()),
			connections: Arc::new(Semaphore::new(options.max_connections)),
			queued_bytes: Arc::new(Semaphore::new(options.max_pending_bytes)),
			next: AtomicU32::new(1),
			closed: CancellationToken::new(),
		}),
	})
}

enum Command {
	Send(Message, Answer, CancellationToken, OwnedSemaphorePermit),
	Receive(Answer, CancellationToken),
	Close(u16, String, Answer),
}
struct Lease {
	closed: CancellationToken,
	committed: bool,
}
impl ReplyGuard for Lease {
	fn commit(&mut self) {
		self.committed = true;
	}
}
impl Drop for Lease {
	fn drop(&mut self) {
		if !self.committed {
			self.closed.cancel();
		}
	}
}
struct ActorLifetime {
	id: u32,
	state: Weak<State>,
	done: CancellationToken,
	permit: Option<OwnedSemaphorePermit>,
}
impl Drop for ActorLifetime {
	fn drop(&mut self) {
		drop(self.permit.take());
		if let Some(state) = self.state.upgrade()
			&& let Ok(mut entries) = state.entries.lock()
		{
			entries.remove(&self.id);
		}
		self.done.cancel();
	}
}

impl WebSocketPlugin {
	fn entry(&self, id: ConnectionId) -> Result<Entry, BridgeError> {
		self.state
			.entries
			.lock()
			.map_err(|_| BridgeError::Native)?
			.get(&id.0)
			.cloned()
			.ok_or(BridgeError::InvalidCall)
	}

	async fn open(
		&self,
		payload: Bytes,
		cancellation: CancellationToken,
	) -> Result<Reply, BridgeError> {
		let (url, protocols) = protocol::decode_open(&payload)?;
		let permit = Arc::clone(&self.state.connections)
			.try_acquire_owned()
			.map_err(|_| BridgeError::Busy)?;
		let connection = tokio::select! { biased;
			() = cancellation.cancelled() => return Err(BridgeError::Cancelled),
			() = self.state.closed.cancelled() => return Err(BridgeError::Cancelled),
			connection = self.client.connect(&url, &protocols) => connection.map_err(BridgeError::from)?,
		};
		let id = self
			.state
			.next
			.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
				value.checked_add(1)
			})
			.map_err(|_| BridgeError::Exhausted)?;
		let (sender, receiver) = mpsc::channel(32);
		let closed = cancellation.child_token();
		let done = CancellationToken::new();
		let entry = Entry {
			commands: sender,
			closed: closed.clone(),
			done: done.clone(),
		};
		{
			let mut entries = self.state.entries.lock().map_err(|_| BridgeError::Native)?;
			if self.state.closed.is_cancelled() || closed.is_cancelled() {
				return Err(BridgeError::Cancelled);
			}
			entries.insert(id, entry);
		}
		let lifetime = ActorLifetime {
			id,
			state: Arc::downgrade(&self.state),
			done,
			permit: Some(permit),
		};
		tokio::spawn(run_connection(
			connection,
			receiver,
			closed.clone(),
			self.state.closed.clone(),
			lifetime,
		));
		Ok(Reply::guarded(
			protocol::id(ConnectionId(id)),
			Lease {
				closed,
				committed: false,
			},
		))
	}

	async fn command(
		&self,
		method: MethodId,
		payload: Bytes,
		cancellation: CancellationToken,
	) -> Result<Reply, BridgeError> {
		let (answer, result) = oneshot::channel();
		let (entry, command) = match method.0 {
			1 => {
				let (id, message) = protocol::decode_send(&payload)?;
				if message.len() > self.config.max_message_bytes {
					return Err(BridgeError::TooLarge);
				}
				let bytes = Arc::clone(&self.state.queued_bytes)
					.try_acquire_many_owned(message.len() as u32)
					.map_err(|_| BridgeError::Busy)?;
				(
					self.entry(id)?,
					Command::Send(message, answer, cancellation.clone(), bytes),
				)
			}
			2 => {
				(
					self.entry(protocol::decode_id(&payload)?)?,
					Command::Receive(answer, cancellation.clone()),
				)
			}
			3 => {
				let (id, code, reason) = protocol::decode_close(&payload)?;
				crate::close_frame(code, &reason).map_err(BridgeError::from)?;
				(self.entry(id)?, Command::Close(code, reason, answer))
			}
			_ => return Err(BridgeError::Unavailable),
		};
		if cancellation.is_cancelled() || self.state.closed.is_cancelled() {
			return Err(BridgeError::Cancelled);
		}
		entry.commands.try_send(command).map_err(|error| {
			match error {
				mpsc::error::TrySendError::Full(_) => BridgeError::Busy,
				mpsc::error::TrySendError::Closed(_) => BridgeError::InvalidCall,
			}
		})?;
		tokio::pin!(result);
		tokio::select! { biased;
			() = cancellation.cancelled() => {
				if method == MethodId(3) { entry.closed.cancel(); }
				// The actor still owns queued/sending bytes. Wait until it
				// acknowledges cancellation before releasing this call's slot.
				let _ = result.await;
				Err(BridgeError::Cancelled)
			}
			result = &mut result => result.map_err(|_| BridgeError::Cancelled)?,
		}
	}
}
impl AsyncPlugin for WebSocketPlugin {
	fn name(&self) -> &'static str {
		"websocket"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[MethodId(0), MethodId(1), MethodId(2), MethodId(3)]
	}

	fn reply_limit(&self, method: MethodId) -> Result<usize, BridgeError> {
		match method.0 {
			0 => Ok(4),
			1 | 3 => Ok(0),
			2 => Ok(self.config.max_message_bytes + 8),
			_ => Err(BridgeError::Unavailable),
		}
	}

	fn invoke(
		&self,
		method: MethodId,
		payload: Bytes,
		cancellation: CancellationToken,
	) -> PluginFuture<'_> {
		Box::pin(async move {
			if method == MethodId(0) {
				self.open(payload, cancellation).await
			} else {
				self.command(method, payload, cancellation).await
			}
		})
	}

	fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
		Box::pin(async move {
			self.state.closed.cancel();
			let entries = match self.state.entries.lock() {
				Ok(entries) => entries.values().cloned().collect::<Vec<_>>(),
				Err(poisoned) => poisoned.into_inner().values().cloned().collect(),
			};
			for entry in entries {
				entry.closed.cancel();
				entry.done.cancelled().await;
			}
		})
	}
}

enum Action {
	Stop,
	Command(Option<Command>),
	Message(Result<Option<Message>, Error>),
	CancelReceive,
}
async fn run_connection(
	mut connection: Connection,
	mut commands: mpsc::Receiver<Command>,
	closed: CancellationToken,
	app_closed: CancellationToken,
	lifetime: ActorLifetime,
) {
	let mut waiter: Option<(Answer, CancellationToken)> = None;
	let mut terminal_answer = None;
	loop {
		let receive_cancel = waiter
			.as_ref()
			.map(|(_, cancellation)| cancellation.clone())
			.unwrap_or_default();
		let action = tokio::select! { biased;
			() = closed.cancelled() => Action::Stop,
			() = app_closed.cancelled() => Action::Stop,
			() = receive_cancel.cancelled(), if waiter.is_some() => Action::CancelReceive,
			message = connection.receive(), if waiter.is_some() => Action::Message(message),
			command = commands.recv() => Action::Command(command),
		};
		match action {
			Action::Stop | Action::Command(None) => break,
			Action::CancelReceive => {
				if let Some((answer, _)) = waiter.take() {
					let _ = answer.send(Err(BridgeError::Cancelled));
				}
			}
			Action::Message(message) => {
				let terminal = !matches!(&message, Ok(Some(_)));
				if let Some((answer, _)) = waiter.take() {
					let result = message
						.map_err(BridgeError::from)
						.and_then(|message| protocol::message(message.as_ref()).map(Reply::from));
					if terminal {
						terminal_answer = Some((answer, result));
					} else {
						let _ = answer.send(result);
					}
				}
				if terminal {
					break;
				}
			}
			Action::Command(Some(Command::Receive(answer, cancellation))) => {
				if cancellation.is_cancelled() {
					let _ = answer.send(Err(BridgeError::Cancelled));
				} else if waiter.is_some() {
					let _ = answer.send(Err(BridgeError::Busy));
				} else {
					waiter = Some((answer, cancellation));
				}
			}
			Action::Command(Some(Command::Send(message, answer, cancellation, bytes))) => {
				let result = tokio::select! { biased;
					() = closed.cancelled() => Err(BridgeError::Cancelled),
					() = app_closed.cancelled() => Err(BridgeError::Cancelled),
					() = cancellation.cancelled() => Err(BridgeError::Cancelled),
					result = connection.send(message) => result.map_err(BridgeError::from),
				};
				let failed = result.is_err();
				drop(bytes);
				let result = result.map(|()| Bytes::new().into());
				// A cancelled partially written send cannot safely be retried
				// on this connection. Release transport rather
				// than hide ambiguity.
				if failed {
					terminal_answer = Some((answer, result));
					break;
				}
				let _ = answer.send(result);
			}
			Action::Command(Some(Command::Close(code, reason, answer))) => {
				let result = tokio::select! { biased;
					() = closed.cancelled() => Err(BridgeError::Cancelled),
					() = app_closed.cancelled() => Err(BridgeError::Cancelled),
					result = connection.close(code, &reason) => result.map_err(BridgeError::from),
				};
				terminal_answer = Some((answer, result.map(|()| Bytes::new().into())));
				break;
			}
		}
	}
	drop(connection);
	drop(commands);
	drop(waiter);
	drop(lifetime);
	if let Some((answer, result)) = terminal_answer {
		let _ = answer.send(result);
	}
}
impl From<Error> for BridgeError {
	fn from(error: Error) -> Self {
		match error {
			Error::Invalid(_) => BridgeError::InvalidArgument,
			Error::DestinationDenied => BridgeError::PermissionDenied,
			Error::TooLarge => BridgeError::TooLarge,
			Error::Backpressure => BridgeError::Busy,
			Error::Closed => BridgeError::InvalidCall,
			Error::Timeout | Error::Transport(_) => BridgeError::Native,
		}
	}
}
