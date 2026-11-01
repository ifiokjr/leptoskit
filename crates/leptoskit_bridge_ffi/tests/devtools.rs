//! A real app reporting to a hub on an ephemeral port: the tree it renders,
//! its commits, the hub's commands, a reloaded guest, and a hub that forgets
//! the session.
//!
//! The report is the process's, so this file holds one test that walks
//! through every step in order.

#![cfg(all(feature = "devtools", not(target_arch = "wasm32")))]
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Read;
use std::io::Write;
use std::mem::MaybeUninit;
use std::net::TcpListener;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use leptoskit_bridge_ffi::app::*;
use leptoskit_bridge_ffi::devtools::DevtoolsFrame;
use leptoskit_bridge_ffi::devtools::lk_devtools_highlight;
use leptoskit_bridge_ffi::devtools::lk_devtools_hub;
use leptoskit_bridge_ffi::devtools::lk_devtools_picking;
use leptoskit_bridge_ffi::lk_buffer_release;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Hello;
use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::Snapshot;
use leptoskit_devtools_protocol::TreeOp;
use serde_json::json;

const PATIENCE: Duration = Duration::from_secs(10);

/// A hub that records what agents send and holds a command long-poll for
/// half a second.
#[derive(Default)]
struct Hub {
	state: Mutex<State>,
	changed: Condvar,
}

#[derive(Default)]
struct State {
	hellos: Vec<Hello>,
	open: Vec<String>,
	events: Vec<(String, Event)>,
	commands: HashMap<String, Vec<Command>>,
}

impl Hub {
	/// Serves on an ephemeral port and returns the hub's URL.
	fn start(self: &Arc<Self>) -> String {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let url = format!("http://{}", listener.local_addr().unwrap());
		let hub = Arc::clone(self);
		std::thread::spawn(move || {
			for stream in listener.incoming().flatten() {
				let hub = Arc::clone(&hub);
				std::thread::spawn(move || hub.serve(stream));
			}
		});
		url
	}

	fn serve(&self, mut stream: TcpStream) {
		let Some((head, body)) = request(&mut stream) else {
			return;
		};
		let mut line = head.split_whitespace();
		let (method, path) = (line.next().unwrap_or(""), line.next().unwrap_or(""));
		let (status, body) = self.route(method, path, &body);
		let response = format!(
			"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: \
			 {}\r\nConnection: close\r\n\r\n{body}",
			body.len()
		);
		let _ = stream.write_all(response.as_bytes());
	}

	fn route(&self, method: &str, path: &str, body: &str) -> (&'static str, String) {
		let (path, query) = path.split_once('?').unwrap_or((path, ""));
		let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
		let mut state = self.state.lock().unwrap();
		let gone = ("404 Not Found", "\"no such session\"".to_owned());

		match (method, parts.as_slice()) {
			("POST", ["agent", "sessions"]) => {
				let id = format!("session-{}", state.hellos.len() + 1);
				state.hellos.push(serde_json::from_str(body).unwrap());
				state.open.push(id.clone());
				self.changed.notify_all();
				("200 OK", format!("{id:?}"))
			}
			("POST", ["agent", "sessions", id, "events"]) => {
				if !state.open.iter().any(|open| open == id) {
					return gone;
				}

				let events: Vec<Event> = serde_json::from_str(body).unwrap();
				state
					.events
					.extend(events.into_iter().map(|event| ((*id).to_owned(), event)));
				self.changed.notify_all();
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
						return gone;
					}

					let queued = state.commands.get(*id).map_or(&[][..], Vec::as_slice);

					if queued.len() > after {
						return ("200 OK", serde_json::to_string(&queued[after..]).unwrap());
					}

					let left = deadline.saturating_duration_since(Instant::now());

					if left.is_zero() {
						return ("200 OK", "[]".to_owned());
					}

					state = self.changed.wait_timeout(state, left).unwrap().0;
				}
			}
			_ => ("404 Not Found", "\"no such route\"".to_owned()),
		}
	}

	fn send(&self, session: &str, command: Command) {
		let mut state = self.state.lock().unwrap();
		state
			.commands
			.entry(session.to_owned())
			.or_default()
			.push(command);
		self.changed.notify_all();
	}

	fn forget(&self, session: &str) {
		self.state
			.lock()
			.unwrap()
			.open
			.retain(|open| open != session);
		self.changed.notify_all();
	}

	fn wait<T>(&self, what: &str, mut found: impl FnMut(&State) -> Option<T>) -> T {
		let deadline = Instant::now() + PATIENCE;
		let mut state = self.state.lock().unwrap();

		loop {
			if let Some(value) = found(&state) {
				return value;
			}

			let left = deadline.saturating_duration_since(Instant::now());
			assert!(!left.is_zero(), "timed out waiting for {what}");
			state = self.changed.wait_timeout(state, left).unwrap().0;
		}
	}

	/// The `index`th snapshot, with its session.
	fn snapshot(&self, index: usize) -> (String, Snapshot) {
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

	fn count(&self, matches: impl Fn(&Event) -> bool) -> usize {
		let state = self.state.lock().unwrap();
		state
			.events
			.iter()
			.filter(|(_, event)| matches(event))
			.count()
	}
}

/// One request's head and body.
fn request(stream: &mut TcpStream) -> Option<(String, String)> {
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

/// A node the guest builds: its wire id, kind code, parent, and props.
type GuestNode = (i64, i32, Option<i64>, serde_json::Value);

/// A guest that builds `nodes` and commits them. A press on `button` sets
/// `edited`'s props to `edit` and commits again.
fn guest(nodes: &[GuestNode], edit: Option<(i64, i64, serde_json::Value)>) -> String {
	let mut data = String::new();
	let mut offset = 0;
	let mut segment = |props: &serde_json::Value| {
		let json = props.to_string();
		let mut escaped = String::new();

		for byte in json.bytes() {
			write!(escaped, "\\{byte:02x}").unwrap();
		}

		writeln!(data, "  (data (i32.const {offset}) \"{escaped}\")").unwrap();
		let placed = (offset, json.len());
		offset += json.len().next_multiple_of(8);
		placed
	};
	let mut main = String::new();

	for (id, kind, _, props) in nodes {
		let (at, len) = segment(props);
		writeln!(
			main,
			"    (call $create (i64.const {id}) (i32.const {kind}))\n    (call $props (i64.const \
			 {id}) (i32.const {at}) (i32.const {len}))"
		)
		.unwrap();
	}

	for (id, _, parent, _) in nodes {
		if let Some(parent) = parent {
			writeln!(
				main,
				"    (call $insert (i64.const {parent}) (i64.const {id}) (i32.const 99))"
			)
			.unwrap();
		}
	}

	let event = match edit {
		Some((button, edited, props)) => {
			let (at, len) = segment(&props);
			format!(
				"(if (i32.and (i64.eq (local.get $id) (i64.const {button})) (i32.eqz (local.get \
				 $kind)))\n      (then\n        (call $props (i64.const {edited}) (i32.const \
				 {at}) (i32.const {len}))\n        (call $commit)\n        (return (i32.const \
				 1))))\n    (i32.const 0)"
			)
		}
		None => "(i32.const 0)".to_owned(),
	};

	format!(
		r#"(module
  (import "leptoskit" "node_create" (func $create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
  (import "leptoskit" "node_insert" (func $insert (param i64 i64 i32)))
  (import "leptoskit" "commit" (func $commit))
  (memory (export "memory") 1)
{data}  (func (export "leptoskit_main")
{main}    (call $commit))
  (func (export "leptoskit_event")
    (param $id i64) (param $kind i32) (param i32 i32) (result i32)
    {event}))"#
	)
}

fn counter(count: u32) -> serde_json::Value {
	json!({
		"text": format!("Count: {count}"),
		"class": "text-lg",
		"source": "src/app.rs:11:9",
	})
}

fn counter_guest() -> String {
	guest(
		&[
			(
				1,
				0,
				None,
				json!({ "class": "flex-col p-4", "source": "src/app.rs:10:5" }),
			),
			(2, 1, Some(1), counter(0)),
			(
				3,
				6,
				Some(1),
				json!({ "text": "Add one", "source": "src/app.rs:12:9" }),
			),
		],
		Some((3, 2, counter(1))),
	)
}

/// A started app running `wasm`.
fn start(wasm: &str) -> *mut App {
	let mut diagnostic = MaybeUninit::uninit();
	// SAFETY: The input lives for the call and the output is writable.
	let app = unsafe {
		create(wasm.as_ptr(), wasm.len(), diagnostic.as_mut_ptr(), |_| {
			Ok(())
		})
	};
	assert!(!app.is_null());
	// SAFETY: The app is live and used on this thread only.
	assert_eq!(unsafe { lk_app_start(app, 320.0, 480.0) }, 0);
	render(app);
	app
}

/// Sends the app's pending transaction to the "platform".
fn render(app: *mut App) {
	// SAFETY: The app is live; the buffer is released once.
	unsafe { lk_buffer_release(lk_app_render(app)) };
}

fn press(app: *mut App, node: u64) {
	// SAFETY: The app is live; a press carries no text.
	let status = unsafe { lk_app_event(app, node as usize, 0, std::ptr::null(), 0) };
	assert_eq!(status, 0);
	render(app);
}

fn node<'a>(snapshot: &'a Snapshot, kind: &str) -> &'a Node {
	snapshot
		.tree
		.nodes
		.iter()
		.find(|node| node.kind == kind)
		.unwrap_or_else(|| panic!("no {kind} in {:?}", snapshot.tree.nodes))
}

fn id(node: &Node) -> u64 {
	let NodeRef::Native(id) = node.id else {
		panic!("a native node: {node:?}");
	};
	id
}

fn eventually(what: &str, mut check: impl FnMut() -> bool) {
	let deadline = Instant::now() + PATIENCE;

	while !check() {
		assert!(Instant::now() < deadline, "timed out waiting for {what}");
		std::thread::sleep(Duration::from_millis(10));
	}
}

fn highlighted() -> Option<DevtoolsFrame> {
	let mut frame = DevtoolsFrame::default();
	// SAFETY: The output is writable.
	(unsafe { lk_devtools_highlight(&raw mut frame) } == 1).then_some(frame)
}

#[test]
fn an_app_reports_its_tree_commits_and_commands_across_a_reload() {
	let hub = Arc::new(Hub::default());
	let url = hub.start();
	// SAFETY: Both inputs live for the call.
	let configured =
		unsafe { lk_devtools_hub(url.as_ptr(), url.len(), b"counter_app".as_ptr(), 11) };
	assert_eq!(configured, 0);

	// The first render is a snapshot with the guest's classes and sources.
	let app = start(&counter_guest());
	let (session, snapshot) = hub.snapshot(0);
	let hello = hub.state.lock().unwrap().hellos[0].clone();
	assert_eq!(hello.app, "counter_app");
	let root = &snapshot.tree.nodes[0];
	assert_eq!(root.kind, "View");
	assert_eq!(root.classes, ["flex-col", "p-4"]);
	assert_eq!(root.source.as_ref().map(|source| source.line), Some(10));
	assert_eq!(root.children.len(), 2);
	let text = node(&snapshot, "Text");
	assert_eq!(text.text.as_deref(), Some("Count: 0"));
	assert_eq!(text.classes, ["text-lg"]);
	assert_eq!(text.source.as_ref().map(|source| source.line), Some(11));
	assert!(text.frame.is_some_and(|frame| frame.height > 0.0));
	let button = node(&snapshot, "Button").clone();
	hub.wait("the first commit", |state| {
		state
			.events
			.iter()
			.any(|(_, event)| matches!(event, Event::Commit { .. }))
			.then_some(())
	});

	// A press re-renders the guest: the text is patched and the commit sent.
	press(app, id(&button));
	let ops = hub.wait("the patch", |state| {
		state.events.iter().find_map(|(_, event)| {
			match event {
				Event::TreePatch { patch } => Some(patch.ops.clone()),
				_ => None,
			}
		})
	});
	assert!(
		ops.iter().any(|op| {
			matches!(op, TreeOp::Update { node } if node.text.as_deref() == Some("Count: 1"))
		}),
		"{ops:?}"
	);
	eventually("the press's commit", || {
		hub.count(|event| matches!(event, Event::Commit { .. })) >= 2
	});

	// The hub highlights a node, for the platform to draw over.
	hub.send(
		&session,
		Command::Highlight {
			key: text.key.clone(),
		},
	);
	eventually("the highlight", || {
		highlighted().is_some_and(|frame| frame.node as u64 == id(text))
	});
	// SAFETY: A null output is refused, not written.
	assert_eq!(unsafe { lk_devtools_highlight(std::ptr::null_mut()) }, -1);

	hub.send(&session, Command::RequestSnapshot);
	hub.snapshot(1);

	// While the hub picks, a press picks its node instead of reaching the
	// guest.
	hub.send(&session, Command::Pick { on: true });
	eventually("picking", || lk_devtools_picking() == 1);
	let commits = hub.count(|event| matches!(event, Event::Commit { .. }));
	press(app, id(&button));
	assert_eq!(lk_devtools_picking(), 0);
	let picked = hub.wait("the pick", |state| {
		state.events.iter().find_map(|(_, event)| {
			match event {
				Event::Picked { node, key } => Some((node.clone(), key.clone())),
				_ => None,
			}
		})
	});
	assert_eq!(picked, (button.id.clone(), button.key.clone()));
	std::thread::sleep(Duration::from_millis(300));
	assert_eq!(
		hub.count(|event| matches!(event, Event::Commit { .. })),
		commits,
		"the picking press never reached the guest"
	);

	// A reloaded guest keeps the session and sends a snapshot once it renders.
	// SAFETY: The app is live and released once.
	unsafe { lk_app_destroy(app) };
	let reloaded = start(&guest(
		&[(
			1,
			1,
			None,
			json!({ "text": "Reloaded", "source": "src/app.rs:20:5" }),
		)],
		None,
	));
	let (reloaded_session, snapshot) = hub.snapshot(2);
	assert_eq!(reloaded_session, session);
	assert_eq!(snapshot.tree.nodes.len(), 1);
	assert_eq!(snapshot.tree.nodes[0].text.as_deref(), Some("Reloaded"));
	assert_eq!(hub.state.lock().unwrap().hellos.len(), 1);

	// A hub that forgets the session gets a new one, and the tree again.
	hub.forget(&session);
	let (renewed, snapshot) = hub.snapshot(3);
	assert_eq!(renewed, "session-2");
	assert_eq!(snapshot.tree.nodes[0].text.as_deref(), Some("Reloaded"));
	// SAFETY: The app is live and released once.
	unsafe { lk_app_destroy(reloaded) };
}
