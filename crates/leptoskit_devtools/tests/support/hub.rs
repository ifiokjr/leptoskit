//! A hub on an ephemeral port that records what agents send and holds a
//! command long-poll for half a second, for the reporter's tests.
#![allow(dead_code, reason = "each test binary uses part of the fake hub")]

use std::collections::HashMap;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Hello;
use leptoskit_devtools_protocol::Snapshot;

/// How long a test waits for the reporter before failing.
pub(crate) const PATIENCE: Duration = Duration::from_secs(10);

/// A hub that records what agents send and holds a command long-poll for
/// half a second.
pub(crate) struct Hub {
	pub(crate) url: String,
	pub(crate) shared: Arc<Shared>,
}

#[derive(Default)]
pub(crate) struct Shared {
	pub(crate) state: Mutex<State>,
	pub(crate) changed: Condvar,
}

#[derive(Default)]
pub(crate) struct State {
	pub(crate) hellos: Vec<Hello>,
	/// The sessions the hub knows.
	pub(crate) open: Vec<String>,
	/// Every event, with its session.
	pub(crate) events: Vec<(String, Event)>,
	pub(crate) commands: HashMap<String, Vec<Command>>,
}

impl Hub {
	pub(crate) fn start() -> Self {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let url = format!("http://{}", listener.local_addr().unwrap());
		let shared = Arc::new(Shared::default());
		let serving = Arc::clone(&shared);
		std::thread::spawn(move || {
			for stream in listener.incoming().flatten() {
				let shared = Arc::clone(&serving);
				std::thread::spawn(move || serve(&shared, stream));
			}
		});
		Self { url, shared }
	}

	pub(crate) fn send(&self, session: &str, command: Command) {
		let mut state = self.shared.state.lock().unwrap();
		state
			.commands
			.entry(session.to_owned())
			.or_default()
			.push(command);
		self.shared.changed.notify_all();
	}

	/// Forgets a session, as a restarted hub would.
	pub(crate) fn forget(&self, session: &str) {
		let mut state = self.shared.state.lock().unwrap();
		state.open.retain(|open| open != session);
		self.shared.changed.notify_all();
	}

	/// Waits until `found` finds something in what the hub has heard.
	pub(crate) fn wait<T>(&self, what: &str, mut found: impl FnMut(&State) -> Option<T>) -> T {
		let deadline = Instant::now() + PATIENCE;
		let mut state = self.shared.state.lock().unwrap();

		loop {
			if let Some(value) = found(&state) {
				return value;
			}

			let left = deadline.saturating_duration_since(Instant::now());
			assert!(!left.is_zero(), "timed out waiting for {what}");
			state = self.shared.changed.wait_timeout(state, left).unwrap().0;
		}
	}

	/// The `index`th snapshot the hub received, with its session.
	pub(crate) fn snapshot(&self, index: usize) -> (String, Snapshot) {
		self.wait(&format!("snapshot {index}"), |state| {
			state
				.events
				.iter()
				.filter_map(|(session, event)| {
					match event {
						Event::Snapshot { snapshot } => Some((session.clone(), snapshot.clone())),
						_ => None,
					}
				})
				.nth(index)
		})
	}
}

pub(crate) fn serve(shared: &Shared, mut stream: TcpStream) {
	let Some((head, body)) = request(&mut stream) else {
		return;
	};
	let mut line = head.split_whitespace();
	let (method, path) = (line.next().unwrap_or(""), line.next().unwrap_or(""));
	let (status, body) = route(shared, method, path, &body);
	let response = format!(
		"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: \
		 close\r\n\r\n{body}",
		body.len()
	);
	let _ = stream.write_all(response.as_bytes());
}

pub(crate) fn route(
	shared: &Shared,
	method: &str,
	path: &str,
	body: &str,
) -> (&'static str, String) {
	let (path, query) = path.split_once('?').unwrap_or((path, ""));
	let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
	let mut state = shared.state.lock().unwrap();

	match (method, parts.as_slice()) {
		("POST", ["agent", "sessions"]) => {
			let id = format!("session-{}", state.hellos.len() + 1);
			state.hellos.push(serde_json::from_str(body).unwrap());
			state.open.push(id.clone());
			shared.changed.notify_all();
			("200 OK", format!("{id:?}"))
		}
		("POST", ["agent", "sessions", id, "events"]) => {
			if !state.open.iter().any(|open| open == id) {
				return ("404 Not Found", "\"no such session\"".to_owned());
			}

			let events: Vec<Event> = serde_json::from_str(body).unwrap();
			state
				.events
				.extend(events.into_iter().map(|event| ((*id).to_owned(), event)));
			shared.changed.notify_all();
			("204 No Content", String::new())
		}
		("GET", ["agent", "sessions", id, "commands"]) => {
			let after: usize = query
				.strip_prefix("after=")
				.and_then(|after| after.parse().ok())
				.unwrap_or(0);
			let deadline = Instant::now() + Duration::from_millis(500);

			loop {
				if !state.open.iter().any(|open| open == id) {
					return ("404 Not Found", "\"no such session\"".to_owned());
				}

				let queued = state.commands.get(*id).map_or(&[][..], Vec::as_slice);

				if queued.len() > after {
					return ("200 OK", serde_json::to_string(&queued[after..]).unwrap());
				}

				let left = deadline.saturating_duration_since(Instant::now());

				if left.is_zero() {
					return ("200 OK", "[]".to_owned());
				}

				state = shared.changed.wait_timeout(state, left).unwrap().0;
			}
		}
		_ => ("404 Not Found", "\"no such route\"".to_owned()),
	}
}

/// One request's head and body.
pub(crate) fn request(stream: &mut TcpStream) -> Option<(String, String)> {
	let mut buffer = Vec::new();
	let mut chunk = [0_u8; 4096];

	loop {
		let read = stream.read(&mut chunk).ok()?;

		if read == 0 {
			return None;
		}

		buffer.extend_from_slice(&chunk[..read]);
		let text = String::from_utf8_lossy(&buffer);

		if let Some(end) = text.find("\r\n\r\n") {
			let head = &text[..end];
			let length = head
				.lines()
				.find_map(|line| {
					let (name, value) = line.split_once(':')?;
					name.eq_ignore_ascii_case("content-length")
						.then(|| value.trim().parse::<usize>().ok())
						.flatten()
				})
				.unwrap_or(0);

			if buffer.len() >= end + 4 + length {
				let body = String::from_utf8_lossy(&buffer[end + 4..end + 4 + length]);
				return Some((head.to_owned(), body.into_owned()));
			}
		}
	}
}
