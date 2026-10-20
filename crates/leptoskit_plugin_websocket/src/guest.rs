use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::guest::Client;

use crate::Message;
use crate::protocol;
use crate::protocol::ConnectionId;

/// Cached native WebSocket registration.
#[derive(Debug, Clone, Copy)]
pub struct WebSocketClient(Client);
impl WebSocketClient {
	/// Resolves the installed, host-scoped plugin once.
	pub fn connect() -> Result<Self, Error> {
		Client::new("websocket").map(Self)
	}

	/// Opens a connection. Cancelled opens cannot leak unclaimed native
	/// sockets.
	pub async fn open(&self, url: &str, subprotocols: &[String]) -> Result<GuestConnection, Error> {
		let payload = protocol::open(url, subprotocols)?;
		let reply = self.0.call(MethodId(0), &payload)?.await?;
		Ok(GuestConnection {
			client: self.0,
			id: protocol::decode_id(&reply)?,
		})
	}
}

/// An explicitly closed native connection. The host bounds retained sockets and
/// closes every remaining socket when the app unloads or its grant is revoked.
#[derive(Debug)]
#[must_use = "close the connection when it is no longer needed"]
pub struct GuestConnection {
	client: Client,
	id: ConnectionId,
}
impl GuestConnection {
	/// Sends a bounded message. A pending receive does not block sending.
	pub async fn send(&self, message: Message) -> Result<(), Error> {
		let payload = protocol::send(self.id, &message)?;
		self.client.call(MethodId(1), &payload)?.await?;
		Ok(())
	}

	/// Waits for one message. One receive may be pending per connection;
	/// another receives Busy. Dropping this future cancels only its waiter.
	pub async fn receive(&self) -> Result<Option<Message>, Error> {
		let response = self
			.client
			.call(MethodId(2), &protocol::id(self.id))?
			.await?;
		protocol::decode_message(&response)
	}

	/// Closes the socket. Cancellation also closes its transport.
	pub async fn close(&self, code: u16, reason: &str) -> Result<(), Error> {
		let payload = protocol::close(self.id, code, reason)?;
		self.client.call(MethodId(3), &payload)?.await?;
		Ok(())
	}
}
