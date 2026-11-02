//! The small part of `WebDriver BiDi` needed to install temporary extensions.
use std::net::TcpStream;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use serde_json::Value;
use serde_json::json;
use tungstenite::Message;
use tungstenite::WebSocket;

const TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct Connection {
	socket: WebSocket<TcpStream>,
	next: u64,
}

impl Connection {
	pub(crate) fn connect(url: &str) -> Result<Self> {
		let address = url::Url::parse(url)?;
		anyhow::ensure!(
			address.scheme() == "ws",
			"WebDriver BiDi needs a local ws URL"
		);
		let ip: std::net::IpAddr = match address.host().context("BiDi URL has no host")? {
			url::Host::Ipv4(ip) => ip.into(),
			url::Host::Ipv6(ip) => ip.into(),
			url::Host::Domain("localhost") => std::net::Ipv4Addr::LOCALHOST.into(),
			url::Host::Domain(_) => anyhow::bail!("BiDi host must be loopback"),
		};
		anyhow::ensure!(ip.is_loopback(), "BiDi host must be loopback");
		let stream = TcpStream::connect_timeout(
			&std::net::SocketAddr::new(ip, address.port().unwrap_or(80)),
			TIMEOUT,
		)?;
		stream.set_read_timeout(Some(TIMEOUT))?;
		stream.set_write_timeout(Some(TIMEOUT))?;
		let (socket, _) = tungstenite::client(url, stream)?;
		Ok(Self { socket, next: 0 })
	}

	pub(crate) fn call(&mut self, method: &str, params: &Value) -> Result<Value> {
		self.next += 1;
		let id = self.next;
		self.socket.send(Message::Text(
			json!({"id": id, "method": method, "params": params})
				.to_string()
				.into(),
		))?;
		let deadline = Instant::now() + TIMEOUT;

		loop {
			let remaining = deadline
				.checked_duration_since(Instant::now())
				.context("WebDriver BiDi command timed out")?;
			self.socket.get_mut().set_read_timeout(Some(remaining))?;

			if let Message::Text(text) = self.socket.read()? {
				let value: Value = serde_json::from_str(&text)?;

				if value["id"].as_u64() != Some(id) {
					continue;
				}

				anyhow::ensure!(
					value["type"] == "success",
					"BiDi {method}: {}: {}",
					value["error"],
					value["message"]
				);
				return Ok(value["result"].clone());
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use std::net::TcpListener;

	use super::*;

	#[test]
	fn extension_commands_match_replies_and_report_errors() {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let address = listener.local_addr().unwrap();
		let worker = std::thread::spawn(move || {
			let (stream, _) = listener.accept().unwrap();
			let mut socket = tungstenite::accept(stream).unwrap();

			for (id, method) in [
				(1, "webExtension.install"),
				(2, "webExtension.uninstall"),
				(3, "webExtension.install"),
			] {
				let request: Value =
					serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
				assert_eq!(request["id"], id);
				assert_eq!(request["method"], method);

				if method.ends_with("uninstall") {
					assert_eq!(request["params"]["extension"], "test@example.com");
				} else {
					assert_eq!(
						request["params"]["extensionData"],
						json!({"type": "path", "path": "/tmp/test extension"})
					);
				}
				// Events and unrelated responses must not be mistaken for our
				// reply.
				socket
					.send(Message::Text(
						json!({"type":"event", "method":"log.entryAdded", "params":{}})
							.to_string()
							.into(),
					))
					.unwrap();
				let reply = if id == 3 {
					json!({"type": "error", "id": id, "error": "invalid web extension", "message": "manifest rejected"})
				} else {
					json!({"type":"success", "id":id, "result":{"extension":"test@example.com"}})
				};
				socket
					.send(Message::Text(reply.to_string().into()))
					.unwrap();
			}
		});
		let mut bidi = Connection::connect(&format!("ws://{address}/session")).unwrap();
		let params = json!({"extensionData": {"type": "path", "path": "/tmp/test extension"}});
		assert_eq!(
			bidi.call("webExtension.install", &params).unwrap()["extension"],
			"test@example.com"
		);
		bidi.call(
			"webExtension.uninstall",
			&json!({"extension": "test@example.com"}),
		)
		.unwrap();
		let error = bidi
			.call("webExtension.install", &params)
			.unwrap_err()
			.to_string();
		assert!(error.contains("manifest rejected"), "{error}");
		worker.join().unwrap();
	}

	#[test]
	fn refuses_non_loopback_and_non_websocket_endpoints() {
		assert!(Connection::connect("ws://192.0.2.1/session").is_err());
		assert!(Connection::connect("https://127.0.0.1/session").is_err());
	}

	#[test]
	fn a_closed_connection_is_an_error() {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let address = listener.local_addr().unwrap();
		let worker = std::thread::spawn(move || {
			let (stream, _) = listener.accept().unwrap();
			let mut socket = tungstenite::accept(stream).unwrap();
			socket.read().unwrap();
			socket.close(None).unwrap();
		});
		let mut bidi = Connection::connect(&format!("ws://{address}")).unwrap();
		assert!(bidi.call("webExtension.install", &json!({})).is_err());
		worker.join().unwrap();
	}
}
