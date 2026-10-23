use std::net::SocketAddr;
use std::net::ToSocketAddrs as _;
use std::sync::Arc;

use futures::SinkExt;
use futures::StreamExt;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{self};

use super::*;

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
const MAX_RESOLVERS: usize = 4;

/// Reusable native connection policy. Operations require a Tokio runtime.
#[derive(Debug, Clone)]
pub struct Client {
	config: Config,
	resolvers: Arc<tokio::sync::Semaphore>,
}
impl Client {
	/// Validates finite resource limits and explicit destination scopes.
	pub fn new(config: Config) -> Result<Self, Error> {
		config.validate()?;
		Ok(Self {
			config,
			resolvers: Arc::new(tokio::sync::Semaphore::new(MAX_RESOLVERS)),
		})
	}

	async fn resolve(&self, url: &Url) -> Result<Vec<SocketAddr>, Error> {
		let port = url
			.port_or_known_default()
			.ok_or_else(|| Error::Invalid("missing WebSocket port".into()))?;
		match url.host() {
			Some(url::Host::Ipv4(address)) => Ok(vec![SocketAddr::new(address.into(), port)]),
			Some(url::Host::Ipv6(address)) => Ok(vec![SocketAddr::new(address.into(), port)]),
			Some(url::Host::Domain(host)) => {
				let permit = Arc::clone(&self.resolvers)
					.try_acquire_owned()
					.map_err(|_| Error::Backpressure)?;
				let host = host.to_owned();
				// OS resolution cannot be interrupted once running. The closure
				// retains admission even when cancellation drops its
				// JoinHandle, bounding both queued and running
				// resolver work across clones.
				tokio::task::spawn_blocking(move || {
					let _permit = permit;
					(host.as_str(), port)
						.to_socket_addrs()
						.map(Iterator::collect)
				})
				.await
				.map_err(|error| Error::Transport(format!("DNS worker failed: {error}")))?
				.map_err(|error| transport(error.into()))
			}
			None => Err(Error::Invalid("missing WebSocket host".into())),
		}
	}

	/// Opens one TLS-validated connection; HTTP redirects are not followed.
	pub async fn connect(&self, url: &str, subprotocols: &[String]) -> Result<Connection, Error> {
		let url = self.config.url(url)?;
		protocols(subprotocols)?;
		let mut request = url.as_str().into_client_request().map_err(transport)?;
		if !subprotocols.is_empty() {
			request.headers_mut().insert(
				"Sec-WebSocket-Protocol",
				subprotocols
					.join(", ")
					.parse()
					.map_err(|_| Error::Invalid("invalid subprotocol".into()))?,
			);
		}
		let settings = WebSocketConfig::default()
			.write_buffer_size(0)
			.max_write_buffer_size(self.config.max_pending_bytes)
			.max_message_size(Some(self.config.max_message_bytes))
			.max_frame_size(Some(self.config.max_message_bytes));
		let (socket, response) = tokio::time::timeout(self.config.connect_timeout, async {
			let addresses = self.resolve(&url).await?;
			let stream = tokio::net::TcpStream::connect(addresses.as_slice())
				.await
				.map_err(|error| transport(error.into()))?;
			stream
				.set_nodelay(true)
				.map_err(|error| transport(error.into()))?;
			// Keep the original URL for Host, TLS certificate validation and
			// SNI; only TCP connection establishment uses resolved addresses.
			tokio_tungstenite::client_async_tls_with_config(request, stream, Some(settings), None)
				.await
				.map_err(transport)
		})
		.await
		.map_err(|_| Error::Timeout)??;
		let protocol = response
			.headers()
			.get("Sec-WebSocket-Protocol")
			.map(|value| value.to_str().map(str::to_owned))
			.transpose()
			.map_err(|_| Error::Transport("invalid server subprotocol".into()))?;
		Ok(Connection {
			socket: Some(socket),
			config: self.config.clone(),
			protocol,
		})
	}
}

/// A single owned socket. Drop closes its transport. There is no background
/// receive task or application queue: reads exert transport backpressure.
#[derive(Debug)]
pub struct Connection {
	socket: Option<Socket>,
	config: Config,
	protocol: Option<String>,
}
impl Connection {
	/// Server-selected subprotocol, when one was negotiated.
	pub fn protocol(&self) -> Option<&str> {
		self.protocol.as_deref()
	}

	/// Sends one bounded message. Cancellation cannot retract bytes already
	/// written; applications must not retry blindly after a dropped send
	/// future.
	pub async fn send(&mut self, message: Message) -> Result<(), Error> {
		if message.len() > self.config.max_message_bytes {
			return Err(Error::TooLarge);
		}
		let socket = self.socket.as_mut().ok_or(Error::Closed)?;
		let message = match message {
			Message::Text(value) => tungstenite::Message::Text(value.into()),
			Message::Binary(value) => tungstenite::Message::Binary(value.into()),
		};
		let result = tokio::time::timeout(self.config.send_timeout, socket.send(message))
			.await
			.map_err(|_| Error::Timeout)
			.and_then(|result| result.map_err(transport));
		if result.is_err() {
			// Like receive, never reuse a socket whose stream state is unknown.
			self.socket = None;
		}
		result
	}

	/// Receives the next application message, handling ping/pong internally.
	/// Returns None on orderly close. Oversized/protocol-failed sockets close.
	pub async fn receive(&mut self) -> Result<Option<Message>, Error> {
		loop {
			let Some(socket) = self.socket.as_mut() else {
				return Ok(None);
			};
			match socket.next().await {
				Some(Ok(tungstenite::Message::Text(text))) => {
					return Ok(Some(Message::Text(text.to_string())));
				}
				Some(Ok(tungstenite::Message::Binary(bytes))) => {
					return Ok(Some(Message::Binary(bytes.to_vec())));
				}
				Some(Ok(tungstenite::Message::Ping(_) | tungstenite::Message::Pong(_))) => {
					let result = tokio::time::timeout(self.config.send_timeout, socket.flush())
						.await
						.map_err(|_| Error::Timeout)
						.and_then(|result| result.map_err(transport));
					if let Err(error) = result {
						self.socket = None;
						return Err(error);
					}
				}
				Some(Ok(tungstenite::Message::Close(_))) => {
					// Flush the automatic close acknowledgement before
					// releasing TCP.
					let result =
						tokio::time::timeout(self.config.close_timeout, socket.flush()).await;
					self.socket = None;
					result.map_err(|_| Error::Timeout)?.map_err(transport)?;
					return Ok(None);
				}
				None => {
					self.socket = None;
					return Ok(None);
				}
				Some(Err(error)) => {
					self.socket = None;
					return Err(transport(error));
				}
				Some(Ok(tungstenite::Message::Frame(_))) => {}
			}
		}
	}

	/// Performs the close handshake and releases the socket. Dropping this
	/// future or exceeding its deadline also releases the transport.
	pub async fn close(&mut self, code: u16, reason: &str) -> Result<(), Error> {
		close_frame(code, reason)?;
		let Some(mut socket) = self.socket.take() else {
			return Ok(());
		};
		let close = async {
			match socket
				.close(Some(tungstenite::protocol::CloseFrame {
					code: code.into(),
					reason: reason.into(),
				}))
				.await
			{
				// The peer closed first: the connection is already closed.
				Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
					return Ok(());
				}
				result => result.map_err(transport)?,
			}
			while let Some(message) = socket.next().await {
				match message {
					Ok(tungstenite::Message::Close(_))
					| Err(tungstenite::Error::ConnectionClosed) => return Ok(()),
					Err(error) => return Err(transport(error)),
					_ => {}
				}
			}
			Ok(())
		};
		tokio::time::timeout(self.config.close_timeout, close)
			.await
			.map_err(|_| Error::Timeout)?
	}
}
fn transport(error: tungstenite::Error) -> Error {
	match error {
		tungstenite::Error::Capacity(_) => Error::TooLarge,
		tungstenite::Error::WriteBufferFull(_) => Error::Backpressure,
		tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => Error::Closed,
		error => Error::Transport(error.to_string()),
	}
}
