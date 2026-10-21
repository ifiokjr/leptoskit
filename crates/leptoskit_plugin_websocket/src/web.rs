use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::task::Poll;
use std::task::Waker;

use futures::future::Either;
use futures::future::poll_fn;
use futures::future::select;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use super::*;

/// Browser policy. Browsers enforce their own TLS/CSP/cookie rules; custom
/// handshake headers and native TLS options are intentionally not exposed.
#[derive(Debug, Clone)]
pub struct Client {
	config: Config,
}
impl Client {
	/// Validates scopes and finite resource limits.
	pub fn new(config: Config) -> Result<Self, Error> {
		config.validate()?;
		Ok(Self { config })
	}

	/// Opens a socket. Dropping the future closes a pending handshake.
	pub async fn connect(&self, url: &str, subprotocols: &[String]) -> Result<Connection, Error> {
		let url = self.config.url(url)?;
		protocols(subprotocols)?;
		let protocols: js_sys::Array = subprotocols
			.iter()
			.map(|value| JsValue::from_str(value))
			.collect();
		let socket = web_sys::WebSocket::new_with_str_sequence(url.as_str(), &protocols)
			.map_err(js_error)?;
		socket.set_binary_type(web_sys::BinaryType::Arraybuffer);
		let mut connection = Connection::new(socket, self.config.clone());
		{
			let timeout = gloo_timers::future::TimeoutFuture::new(
				self.config.connect_timeout.as_millis() as u32,
			);
			let opened = poll_fn(|context| {
				let mut state = connection.state.borrow_mut();
				if let Some(error) = state.error.take() {
					return Poll::Ready(Err(error));
				}
				match connection.socket.ready_state() {
					web_sys::WebSocket::OPEN => Poll::Ready(Ok(())),
					web_sys::WebSocket::CLOSED | web_sys::WebSocket::CLOSING => {
						Poll::Ready(Err(Error::Closed))
					}
					_ => {
						state.waker = Some(context.waker().clone());
						Poll::Pending
					}
				}
			});
			futures::pin_mut!(opened, timeout);
			match select(opened, timeout).await {
				Either::Left((result, _)) => result?,
				Either::Right(((), _)) => return Err(Error::Timeout),
			}
		}
		connection.protocol = match connection.socket.protocol() {
			value if value.is_empty() => None,
			value => Some(value),
		};
		Ok(connection)
	}
}
#[derive(Default)]
struct State {
	queue: VecDeque<Message>,
	bytes: usize,
	error: Option<Error>,
	closed: bool,
	waker: Option<Waker>,
}
impl State {
	fn wake(&mut self) {
		if let Some(waker) = self.waker.take() {
			waker.wake();
		}
	}
}

/// Owns browser listeners, the socket, and a bounded receive queue. Drop
/// removes callbacks and initiates closure; no forgotten wasm closures remain
/// installed.
pub struct Connection {
	socket: web_sys::WebSocket,
	state: Rc<RefCell<State>>,
	config: Config,
	protocol: Option<String>,
	_open: Closure<dyn FnMut(web_sys::Event)>,
	_error: Closure<dyn FnMut(web_sys::Event)>,
	_close: Closure<dyn FnMut(web_sys::CloseEvent)>,
	_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
}
impl Connection {
	fn new(socket: web_sys::WebSocket, config: Config) -> Self {
		let state = Rc::new(RefCell::new(State::default()));
		let open_state = Rc::clone(&state);
		let open = Closure::new(move |_: web_sys::Event| open_state.borrow_mut().wake());
		socket.set_onopen(Some(open.as_ref().unchecked_ref()));
		let error_state = Rc::clone(&state);
		let error = Closure::new(move |_: web_sys::Event| {
			let mut state = error_state.borrow_mut();
			if state.error.is_none() {
				state.error = Some(Error::Transport("browser WebSocket failed".into()));
			}
			state.wake();
		});
		socket.set_onerror(Some(error.as_ref().unchecked_ref()));
		let close_state = Rc::clone(&state);
		let close = Closure::new(move |event: web_sys::CloseEvent| {
			let mut state = close_state.borrow_mut();
			state.closed = true;
			if !event.was_clean() && state.error.is_none() {
				state.error = Some(Error::Transport(
					"WebSocket closed without completing its handshake".into(),
				));
			}
			state.wake();
		});
		socket.set_onclose(Some(close.as_ref().unchecked_ref()));
		let message_state = Rc::clone(&state);
		let message_config = config.clone();
		let message_socket = socket.clone();
		let message = Closure::new(move |event: web_sys::MessageEvent| {
			let mut state = message_state.borrow_mut();
			if state.closed || state.error.is_some() {
				return;
			}
			let data = event.data();
			let message = if let Some(text) = data.as_string() {
				Ok(Message::Text(text))
			} else if let Some(buffer) = data.dyn_ref::<js_sys::ArrayBuffer>() {
				if buffer.byte_length() as usize > message_config.max_message_bytes {
					Err(Error::TooLarge)
				} else {
					Ok(Message::Binary(js_sys::Uint8Array::new(buffer).to_vec()))
				}
			} else {
				Err(Error::Transport("unexpected WebSocket message type".into()))
			};
			match message {
				Ok(message) if message.len() > message_config.max_message_bytes => {
					state.error = Some(Error::TooLarge);
				}
				Ok(message)
					if state.queue.len() >= message_config.max_pending_messages
						|| message.len()
							> message_config.max_pending_bytes.saturating_sub(state.bytes) =>
				{
					state.error = Some(Error::Backpressure);
				}
				Ok(message) => {
					state.bytes += message.len();
					state.queue.push_back(message);
				}
				Err(error) => state.error = Some(error),
			}
			if state.error.is_some() {
				state.closed = true;
				state.queue.clear();
				state.bytes = 0;
				// Browser close() accepts only normal/application close codes.
				if let Err(error) = message_socket
					.close_with_code_and_reason(4009, "resource limit or invalid message")
				{
					state.error = Some(js_error(error));
				}
			}
			state.wake();
		});
		socket.set_onmessage(Some(message.as_ref().unchecked_ref()));
		Self {
			socket,
			state,
			config,
			protocol: None,
			_open: open,
			_error: error,
			_close: close,
			_message: message,
		}
	}

	/// Negotiated server subprotocol.
	pub fn protocol(&self) -> Option<&str> {
		self.protocol.as_deref()
	}

	/// Queues one message only if browser buffered bytes fit the finite budget.
	/// Backpressure is explicit; retry after buffered bytes drain.
	#[allow(clippy::unused_async)] // Preserve the same awaitable API as native sends.
	pub async fn send(&mut self, message: Message) -> Result<(), Error> {
		if self.socket.ready_state() != web_sys::WebSocket::OPEN || self.state.borrow().closed {
			return Err(Error::Closed);
		}
		if message.len() > self.config.max_message_bytes {
			return Err(Error::TooLarge);
		}
		if message.len()
			> self
				.config
				.max_pending_bytes
				.saturating_sub(self.socket.buffered_amount() as usize)
		{
			return Err(Error::Backpressure);
		}
		match message {
			Message::Text(text) => self.socket.send_with_str(&text),
			Message::Binary(bytes) => self.socket.send_with_u8_array(&bytes),
		}
		.map_err(js_error)
	}

	/// Returns a queued message or waits. Dropping this receive future keeps
	/// the connection alive; dropping the connection removes listeners and
	/// closes it.
	pub async fn receive(&mut self) -> Result<Option<Message>, Error> {
		poll_fn(|context| {
			let mut state = self.state.borrow_mut();
			if let Some(error) = state.error.take() {
				return Poll::Ready(Err(error));
			}
			if let Some(message) = state.queue.pop_front() {
				state.bytes -= message.len();
				return Poll::Ready(Ok(Some(message)));
			}
			if state.closed {
				return Poll::Ready(Ok(None));
			}
			state.waker = Some(context.waker().clone());
			Poll::Pending
		})
		.await
	}

	/// Initiates a normal/application close and waits for the browser close
	/// event, bounded by `close_timeout`. The socket stays closing on timeout.
	pub async fn close(&mut self, code: u16, reason: &str) -> Result<(), Error> {
		close_frame(code, reason)?;
		self.socket
			.close_with_code_and_reason(code, reason)
			.map_err(js_error)?;
		let timeout =
			gloo_timers::future::TimeoutFuture::new(self.config.close_timeout.as_millis() as u32);
		let closed = poll_fn(|context| {
			let mut state = self.state.borrow_mut();
			if let Some(error) = state.error.take() {
				return Poll::Ready(Err(error));
			}
			if self.socket.ready_state() == web_sys::WebSocket::CLOSED {
				return Poll::Ready(Ok(()));
			}
			state.waker = Some(context.waker().clone());
			Poll::Pending
		});
		futures::pin_mut!(closed, timeout);
		match select(closed, timeout).await {
			Either::Left((result, _)) => result,
			Either::Right(((), _)) => Err(Error::Timeout),
		}
	}
}
impl Drop for Connection {
	fn drop(&mut self) {
		self.socket.set_onopen(None);
		self.socket.set_onerror(None);
		self.socket.set_onclose(None);
		self.socket.set_onmessage(None);
		// close() with no code cannot fail for a valid browser WebSocket. The
		// Result remains handled because older engines may still report an
		// error.
		if let Err(error) = self.socket.close() {
			web_sys::console::error_2(
				&JsValue::from_str("Failed to close a dropped LeptosKit WebSocket"),
				&error,
			);
		}
	}
}
fn js_error(value: JsValue) -> Error {
	Error::Transport(value.dyn_into::<js_sys::Error>().map_or_else(
		|_| "browser WebSocket failed".into(),
		|error| error.message().into(),
	))
}
