//! The native host agent.

use std::time::Duration;

use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Hello;
use leptoskit_devtools_protocol::SessionId;
use leptoskit_devtools_protocol::Snapshot;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeSurface;
use serde::Serialize;
use serde::de::DeserializeOwned;

mod mirror;
mod patch;
mod reporter;

pub use mirror::Change;
pub use mirror::Mirror;
pub use reporter::Batch;
pub use reporter::CommitRecord;
pub use reporter::Guest;
pub use reporter::Highlight;
pub use reporter::Reporter;
pub use reporter::Source;

/// How long a request other than the command long-poll may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the command long-poll may take: the hub holds it for 25
/// seconds when it has nothing to send, and the answer needs time to arrive.
const POLL_TIMEOUT: Duration = Duration::from_secs(35);

/// A session with the local hub.
///
/// Clones share the session, so one thread can long-poll for commands while
/// another reports events.
#[derive(Clone)]
pub struct HostAgent {
	hub: String,
	session: SessionId,
	after: usize,
	agent: ureq::Agent,
}

/// The hub no longer knows a session: it restarted, or the session expired
/// while the app was paused. Open a new one.
///
/// [`HostAgent::report`] and [`HostAgent::commands`] fail with it, which
/// callers find with `error.is::<SessionGone>()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionGone;

impl std::fmt::Display for SessionGone {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("the devtools hub no longer knows this session")
	}
}

impl std::error::Error for SessionGone {}

impl HostAgent {
	/// Open a session with the hub at `hub`.
	///
	/// # Errors
	///
	/// When the hub is unreachable or refuses the session.
	pub fn connect(hub: &str, hello: &Hello) -> anyhow::Result<Self> {
		let agent: ureq::Agent = ureq::Agent::config_builder()
			.http_status_as_error(false)
			.timeout_global(Some(REQUEST_TIMEOUT))
			.build()
			.into();
		let session = post_json(&agent, &format!("{hub}/agent/sessions"), hello)?;
		Ok(Self {
			hub: hub.to_owned(),
			session,
			after: 0,
			agent,
		})
	}

	/// The hub's session id.
	#[must_use]
	pub fn session(&self) -> &SessionId {
		&self.session
	}

	/// Send a batch of events.
	///
	/// # Errors
	///
	/// When the hub is unreachable or refuses the events, and with
	/// [`SessionGone`] when it no longer knows the session.
	pub fn report(&self, events: &[Event]) -> anyhow::Result<()> {
		post(&self.agent, &self.events_url(), events)
	}

	/// Long-poll for the commands after the last one received. The hub
	/// answers as soon as it has one, and with none after holding the request
	/// for 25 seconds.
	///
	/// # Errors
	///
	/// When the hub is unreachable or answers with something other than
	/// commands, and with [`SessionGone`] when it no longer knows the session.
	pub fn commands(&mut self) -> anyhow::Result<Vec<Command>> {
		let url = format!("{}?after={}", self.commands_url(), self.after);
		let mut response = self
			.agent
			.get(&url)
			.config()
			.timeout_global(Some(POLL_TIMEOUT))
			.build()
			.call()?;
		let status = response.status();

		if status.as_u16() == 404 {
			return Err(SessionGone.into());
		}

		let text = response.body_mut().read_to_string()?;
		anyhow::ensure!(
			status.as_u16() == 200,
			"the devtools hub answered {status}: {text}"
		);
		let commands: Vec<Command> = serde_json::from_str(&text)?;
		self.after += commands.len();
		Ok(commands)
	}

	fn events_url(&self) -> String {
		format!("{}/agent/sessions/{}/events", self.hub, self.session.0)
	}

	fn commands_url(&self) -> String {
		format!("{}/agent/sessions/{}/commands", self.hub, self.session.0)
	}
}

/// The snapshot a native surface reports.
#[must_use]
pub fn snapshot<B: NativeBackend>(surface: &NativeSurface<B>) -> Snapshot {
	snapshot_of(surface.devtools_tree())
}

/// A snapshot of a native tree. The guest reports the rest of the app's
/// state itself.
fn snapshot_of(tree: leptoskit_devtools_protocol::Tree) -> Snapshot {
	Snapshot {
		tree,
		route: None,
		queries: Vec::new(),
		mutations: Vec::new(),
		forms: Vec::new(),
		state: Vec::new(),
		calls: Vec::new(),
		rpc: Vec::new(),
	}
}

fn post_json<T: Serialize, R: DeserializeOwned>(
	agent: &ureq::Agent,
	url: &str,
	body: &T,
) -> anyhow::Result<R> {
	let mut response = agent
		.post(url)
		.header("content-type", "application/json")
		.send(serde_json::to_vec(body)?)?;
	let status = response.status();
	let text = response.body_mut().read_to_string()?;
	anyhow::ensure!(
		status.as_u16() == 200,
		"the devtools hub answered {status}: {text}"
	);
	Ok(serde_json::from_str(&text)?)
}

fn post<T: Serialize + ?Sized>(agent: &ureq::Agent, url: &str, body: &T) -> anyhow::Result<()> {
	let mut response = agent
		.post(url)
		.header("content-type", "application/json")
		.send(serde_json::to_vec(body)?)?;
	let status = response.status();

	match status.as_u16() {
		204 => Ok(()),
		404 => Err(SessionGone.into()),
		_ => {
			let text = response.body_mut().read_to_string()?;
			anyhow::bail!("the devtools hub answered {status}: {text}");
		}
	}
}

/// The hub at `url` without a trailing slash, when it is a plain `http` URL
/// on a loopback host: `127.0.0.1`, `[::1]`, or `localhost`, with an
/// optional port and nothing after it. The hub listens only on loopback, so
/// an app never reports itself anywhere else.
#[must_use]
pub fn loopback_hub(url: &str) -> Option<&str> {
	let hub = url.trim().trim_end_matches('/');
	let authority = hub.strip_prefix("http://")?;

	let (host, port) = match authority.rsplit_once(':') {
		Some((host, port)) if !host.is_empty() && !port.contains(']') => (host, Some(port)),
		_ => (authority, None),
	};

	if port.is_some_and(|port| port.parse::<u16>().is_err()) {
		return None;
	}

	matches!(host, "127.0.0.1" | "[::1]" | "localhost").then_some(hub)
}

#[cfg(test)]
mod tests {
	use std::io::Read;
	use std::io::Write;
	use std::net::TcpListener;

	use leptoskit_devtools_protocol::Capability;
	use leptoskit_devtools_protocol::NodeRef;
	use leptoskit_devtools_protocol::Platform;
	use leptoskit_devtools_protocol::Tree;
	use leptoskit_native::Element;
	use leptoskit_native::HeadlessBackend;

	use super::*;

	fn hello() -> Hello {
		Hello::new(
			"test",
			Platform::Macos,
			None,
			[Capability::Tree, Capability::Commits],
		)
	}

	/// One request from the agent, with its body.
	fn request(stream: &mut std::net::TcpStream) -> (String, String) {
		let mut buffer = Vec::new();
		let mut chunk = [0_u8; 1024];

		loop {
			let read = stream.read(&mut chunk).unwrap();

			if read == 0 {
				break;
			}

			buffer.extend_from_slice(&chunk[..read]);
			let text = String::from_utf8_lossy(&buffer);

			if let Some(end) = text.find("\r\n\r\n") {
				let head = &text[..end];
				let length = head
					.lines()
					.find_map(|line| {
						line.strip_prefix("content-length: ")
							.or_else(|| line.strip_prefix("Content-Length: "))
							.and_then(|value| value.trim().parse::<usize>().ok())
					})
					.unwrap_or(0);

				if buffer.len() >= end + 4 + length {
					return (head.to_owned(), text[end + 4..end + 4 + length].to_owned());
				}
			}
		}

		(String::new(), String::new())
	}

	fn respond(stream: &mut std::net::TcpStream, status: &str, body: &str) {
		let response = format!(
			"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: \
			 {}\r\n\r\n{body}",
			body.len()
		);
		stream.write_all(response.as_bytes()).unwrap();
	}

	#[test]
	fn only_loopback_hubs_are_reported_to() {
		for (url, hub) in [
			("http://127.0.0.1:7340", Some("http://127.0.0.1:7340")),
			("http://127.0.0.1:7340/", Some("http://127.0.0.1:7340")),
			("http://localhost:7340", Some("http://localhost:7340")),
			("http://[::1]:7340", Some("http://[::1]:7340")),
			("http://[::1]", Some("http://[::1]")),
			("http://127.0.0.1", Some("http://127.0.0.1")),
			("https://127.0.0.1:7340", None),
			("http://192.168.1.4:7340", None),
			("http://127.0.0.1.example.com:7340", None),
			("http://user@127.0.0.1:7340", None),
			("http://127.0.0.1:7340/agent", None),
			("http://127.0.0.1:port", None),
			("127.0.0.1:7340", None),
			("", None),
		] {
			assert_eq!(loopback_hub(url), hub, "{url}");
		}
	}

	#[test]
	fn the_snapshot_carries_the_surface_tree() {
		let mut surface = NativeSurface::new(HeadlessBackend::new());
		surface.set_viewport(100.0, 100.0);
		let element = Element::view().child(Element::text("hello"));
		surface.set_root(&element);
		let snapshot = snapshot(&surface);
		assert_eq!(snapshot.tree.nodes.len(), 2);
		assert!(snapshot.tree.nodes.iter().all(|node| node.source.is_some()));
	}

	#[test]
	fn the_agent_opens_a_session_and_polls_commands() {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let address = listener.local_addr().unwrap();
		let server = std::thread::spawn(move || {
			let (mut stream, _) = listener.accept().unwrap();
			let (head, body) = request(&mut stream);
			assert!(head.starts_with("POST /agent/sessions"));
			assert!(body.contains("test"));
			respond(&mut stream, "200 OK", "\"session-1\"");
			drop(stream);

			let (mut stream, _) = listener.accept().unwrap();
			let (head, body) = request(&mut stream);
			assert!(head.starts_with("POST /agent/sessions/session-1/events"));
			assert!(body.contains("snapshot"));
			respond(&mut stream, "204 No Content", "");
			drop(stream);

			let (mut stream, _) = listener.accept().unwrap();
			let (head, _) = request(&mut stream);
			assert!(head.starts_with("GET /agent/sessions/session-1/commands?after=0"));
			respond(&mut stream, "200 OK", "[{\"type\":\"request_snapshot\"}]");
		});

		let mut agent = HostAgent::connect(&format!("http://{address}"), &hello()).unwrap();
		assert_eq!(agent.session().as_str(), "session-1");
		let event = Event::Snapshot {
			snapshot: Snapshot {
				tree: Tree {
					root: NodeRef::Native(0),
					nodes: Vec::new(),
				},
				route: None,
				queries: Vec::new(),
				mutations: Vec::new(),
				forms: Vec::new(),
				state: Vec::new(),
				calls: Vec::new(),
				rpc: Vec::new(),
			},
		};
		agent.report(&[event]).unwrap();
		let commands = agent.commands().unwrap();
		assert_eq!(commands, [Command::RequestSnapshot]);
		server.join().unwrap();
	}
}
