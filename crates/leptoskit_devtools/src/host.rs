//! The native host agent.

use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Hello;
use leptoskit_devtools_protocol::SessionId;
use leptoskit_devtools_protocol::Snapshot;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeSurface;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A session with the local hub.
pub struct HostAgent {
	hub: String,
	session: SessionId,
	after: usize,
	agent: ureq::Agent,
}

impl HostAgent {
	/// Open a session with the hub at `hub`.
	///
	/// # Errors
	///
	/// When the hub is unreachable or refuses the session.
	pub fn connect(hub: &str, hello: &Hello) -> anyhow::Result<Self> {
		let agent: ureq::Agent = ureq::Agent::config_builder()
			.http_status_as_error(false)
			.timeout_global(Some(std::time::Duration::from_secs(5)))
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
	/// When the hub is unreachable or refuses the events.
	pub fn report(&self, events: &[Event]) -> anyhow::Result<()> {
		post(&self.agent, &self.events_url(), events)
	}

	/// Long-poll for the commands after the last one received.
	///
	/// # Errors
	///
	/// When the hub is unreachable or answers with something other than
	/// commands.
	pub fn commands(&mut self) -> anyhow::Result<Vec<Command>> {
		let url = format!("{}?after={}", self.commands_url(), self.after);
		let mut response = self.agent.get(&url).call()?;
		let status = response.status();
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
	Snapshot {
		tree: surface.devtools_tree(),
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

	if status.as_u16() != 204 {
		let text = response.body_mut().read_to_string()?;
		anyhow::bail!("the devtools hub answered {status}: {text}");
	}

	Ok(())
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
