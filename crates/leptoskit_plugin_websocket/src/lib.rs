//! Bounded WebSocket messages over native Tokio/tungstenite or browser
//! WebSocket. Explicit destination origins are required. Browser receive queues
//! have byte and item limits; overflow closes the socket rather than losing
//! messages or accumulating indefinitely. Native receives apply socket
//! backpressure.
//!
//! Native registrants enable `async-host`; Wasm apps enable `guest` to use the
//! cached native client. Browser clients use the default `browser` feature.

#[cfg(any(not(target_arch = "wasm32"), feature = "browser"))]
use std::time::Duration;

#[cfg(any(not(target_arch = "wasm32"), feature = "browser"))]
use url::Url;

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub mod guest;
#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
mod host;
pub mod protocol;
#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
pub use host::WebSocketPlugin;
#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
pub use host::from_config;

/// Connection and resource-limit failures.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// Invalid configuration, URL, protocol token, or close frame.
	#[error("invalid WebSocket request: {0}")]
	Invalid(String),
	/// The destination origin is not explicitly allowed.
	#[error("WebSocket destination is not allowed")]
	DestinationDenied,
	/// A frame/message exceeds its configured limit.
	#[error("WebSocket message exceeded its size limit")]
	TooLarge,
	/// Pending send/receive bytes or queued messages exceeded their budget.
	#[error("WebSocket backpressure limit reached")]
	Backpressure,
	/// Connect, send, or close deadline expired.
	#[error("WebSocket operation timed out")]
	Timeout,
	/// The connection has closed.
	#[error("WebSocket is closed")]
	Closed,
	/// A transport, TLS, handshake, or protocol failure.
	#[error("WebSocket transport failed: {0}")]
	Transport(String),
}

/// Application data frames. Control frames are handled by the transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
	/// UTF-8 text.
	Text(String),
	/// Binary bytes.
	Binary(Vec<u8>),
}
impl Message {
	/// Byte length of the encoded message.
	pub fn len(&self) -> usize {
		match self {
			Self::Text(text) => text.len(),
			Self::Binary(bytes) => bytes.len(),
		}
	}

	/// Whether the message has an empty payload.
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}
}

/// Host-owned origin allowlist and finite resource limits.
#[derive(Debug, Clone)]
#[cfg(any(not(target_arch = "wasm32"), feature = "browser"))]
pub struct Config {
	origins: Vec<url::Origin>,
	/// Maximum individual incoming or outgoing message bytes.
	pub max_message_bytes: usize,
	/// Maximum browser queued receive bytes and buffered send bytes; native
	/// tungstenite's write buffer uses the same finite bound.
	pub max_pending_bytes: usize,
	/// Maximum queued browser messages, including empty messages.
	pub max_pending_messages: usize,
	/// Maximum handshake duration.
	pub connect_timeout: Duration,
	/// Native send deadline. Browser sends immediately or returns backpressure.
	pub send_timeout: Duration,
	/// Maximum graceful close-handshake duration.
	pub close_timeout: Duration,
}
#[cfg(any(not(target_arch = "wasm32"), feature = "browser"))]
impl Config {
	/// Accepts exact ws:// or wss:// origins. Plain WebSocket access is
	/// therefore an explicit host choice. Paths/userinfo/query/fragment are
	/// forbidden in the allowlist; request paths and query strings are
	/// allowed.
	pub fn new(origins: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Self, Error> {
		let mut allowed = Vec::new();

		for origin in origins {
			let url = parse_url(origin.as_ref())?;

			if url.path() != "/" || url.query().is_some() {
				return Err(Error::Invalid("allowlist entries must be origins".into()));
			}

			if !allowed.contains(&url.origin()) {
				allowed.push(url.origin());
			}
		}

		if allowed.is_empty() {
			return Err(Error::Invalid("at least one origin is required".into()));
		}

		Ok(Self {
			origins: allowed,
			max_message_bytes: 1024 * 1024,
			max_pending_bytes: 4 * 1024 * 1024,
			max_pending_messages: 32,
			connect_timeout: Duration::from_secs(10),
			send_timeout: Duration::from_secs(10),
			close_timeout: Duration::from_secs(5),
		})
	}

	fn validate(&self) -> Result<(), Error> {
		if self.max_message_bytes == 0
			|| self.max_pending_bytes < self.max_message_bytes.saturating_add(14)
			|| self.max_pending_messages == 0
			|| self.max_pending_bytes > u32::MAX as usize
		{
			return Err(Error::Invalid(
				"positive message/queue limits and room for frame overhead required".into(),
			));
		}

		for timeout in [self.connect_timeout, self.send_timeout, self.close_timeout] {
			if timeout.is_zero() || timeout.as_millis() > i32::MAX as u128 {
				return Err(Error::Invalid("invalid operation timeout".into()));
			}
		}

		Ok(())
	}

	fn url(&self, value: &str) -> Result<Url, Error> {
		let url = parse_url(value)?;

		if !self.origins.contains(&url.origin()) {
			return Err(Error::DestinationDenied);
		}

		Ok(url)
	}
}
#[cfg(any(not(target_arch = "wasm32"), feature = "browser"))]
fn parse_url(value: &str) -> Result<Url, Error> {
	let url =
		Url::parse(value).map_err(|_| Error::Invalid("absolute WS(S) URL required".into()))?;

	if !matches!(url.scheme(), "ws" | "wss")
		|| url.host_str().is_none_or(|host| host.contains('*'))
		|| !url.username().is_empty()
		|| url.password().is_some()
		|| url.fragment().is_some()
	{
		return Err(Error::Invalid(
			"WS(S) URL without credentials or fragment required".into(),
		));
	}
	Ok(url)
}
#[cfg(any(not(target_arch = "wasm32"), feature = "browser"))]
fn protocols(values: &[String]) -> Result<(), Error> {
	for (index, value) in values.iter().enumerate() {
		if value.is_empty()
			|| !value
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
			|| values[..index].contains(value)
		{
			return Err(Error::Invalid("invalid or duplicate subprotocol".into()));
		}
	}

	Ok(())
}
#[cfg(any(not(target_arch = "wasm32"), feature = "browser"))]
fn close_frame(code: u16, reason: &str) -> Result<(), Error> {
	// Browser WebSocket.close accepts 1000 or application codes 3000..4999.
	if (code != 1000 && !(3000..5000).contains(&code)) || reason.len() > 123 {
		return Err(Error::Invalid(
			"invalid close code or reason longer than 123 bytes".into(),
		));
	}

	Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::Client;
#[cfg(not(target_arch = "wasm32"))]
pub use native::Connection;
#[cfg(all(feature = "browser", target_arch = "wasm32", target_os = "unknown"))]
mod web;
#[cfg(all(feature = "browser", target_arch = "wasm32", target_os = "unknown"))]
pub use web::Client;
#[cfg(all(feature = "browser", target_arch = "wasm32", target_os = "unknown"))]
pub use web::Connection;

#[cfg(all(test, any(not(target_arch = "wasm32"), feature = "browser")))]
mod tests {
	use super::*;
	#[test]
	fn scope_and_protocol_validation_precede_network_access() {
		let scope = Config::new(["wss://example.com"]).unwrap();
		assert!(scope.url("wss://example.com/channel?token=abc").is_ok());
		assert!(matches!(
			scope.url("ws://example.com"),
			Err(Error::DestinationDenied)
		));
		assert!(matches!(
			scope.url("wss://example.com.attacker.test"),
			Err(Error::DestinationDenied)
		));
		for value in [
			"https://example.com",
			"file:///tmp/a",
			"wss://user:pass@example.com",
			"wss://example.com/#x",
		] {
			assert!(scope.url(value).is_err());
		}
		assert!(protocols(&["chat".into(), "chat".into()]).is_err());

		assert!(protocols(&["bad protocol".into()]).is_err());
		assert!(close_frame(1006, "").is_err());
		assert!(close_frame(1000, &"x".repeat(124)).is_err());
	}
}
