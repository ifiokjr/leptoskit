//! The native reporter against a hub on an ephemeral port: sessions,
//! snapshots, patches, commits, commands, reconnecting, and the values a
//! guest reports beside its tree.

#![cfg(all(feature = "host", not(target_arch = "wasm32")))]

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

use leptoskit_devtools::Batch;
use leptoskit_devtools::Change;
use leptoskit_devtools::CommitRecord;
use leptoskit_devtools::Guest;
use leptoskit_devtools::Reporter;
use leptoskit_devtools_protocol::Capability;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Frame;
use leptoskit_devtools_protocol::Hello;
use leptoskit_devtools_protocol::MatchKey;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::Platform;
use leptoskit_devtools_protocol::Snapshot;
use leptoskit_devtools_protocol::SourceLocation;
use leptoskit_devtools_protocol::TreeOp;
use leptoskit_devtools_protocol::Value;
use leptoskit_native::NativeKind;
use serde_json::json;

/// How long a test waits for the reporter before failing.
const PATIENCE: Duration = Duration::from_secs(10);

/// A hub that records what agents send and holds a command long-poll for
/// half a second.
struct Hub {
	url: String,
	shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
	state: Mutex<State>,
	changed: Condvar,
}

#[derive(Default)]
struct State {
	hellos: Vec<Hello>,
	/// The sessions the hub knows.
	open: Vec<String>,
	/// Every event, with its session.
	events: Vec<(String, Event)>,
	commands: HashMap<String, Vec<Command>>,
}

impl Hub {
	fn start() -> Self {
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

	fn send(&self, session: &str, command: Command) {
		let mut state = self.shared.state.lock().unwrap();
		state
			.commands
			.entry(session.to_owned())
			.or_default()
			.push(command);
		self.shared.changed.notify_all();
	}

	/// Forgets a session, as a restarted hub would.
	fn forget(&self, session: &str) {
		let mut state = self.shared.state.lock().unwrap();
		state.open.retain(|open| open != session);
		self.shared.changed.notify_all();
	}

	/// Waits until `found` finds something in what the hub has heard.
	fn wait<T>(&self, what: &str, mut found: impl FnMut(&State) -> Option<T>) -> T {
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
}

fn serve(shared: &Shared, mut stream: TcpStream) {
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

fn route(shared: &Shared, method: &str, path: &str, body: &str) -> (&'static str, String) {
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

fn hello() -> Hello {
	Hello::new(
		"counter_app",
		Platform::Macos,
		None,
		[Capability::Tree, Capability::Commits, Capability::Picking],
	)
}

fn frame(x: f32, y: f32, width: f32, height: f32) -> Frame {
	Frame {
		x,
		y,
		width,
		height,
	}
}

fn commit(mutations: usize) -> CommitRecord {
	CommitRecord {
		at: Instant::now(),
		mutations,
		layout_us: None,
		apply_us: None,
	}
}

/// A screen with a heading, rooted at `root`, as a host's first render
/// sends it.
fn screen(root: u64, heading: &str) -> Batch {
	let text = root + 1;
	Batch {
		root: Some(root),
		changes: vec![
			Change::Create {
				id: root,
				kind: NativeKind::View,
				text: None,
			},
			Change::Create {
				id: text,
				kind: NativeKind::Text,
				text: Some(heading.to_owned()),
			},
			Change::Insert {
				parent: root,
				child: text,
				index: 0,
			},
			Change::Layout {
				id: root,
				frame: frame(0.0, 0.0, 320.0, 480.0),
			},
			Change::Layout {
				id: text,
				frame: frame(16.0, 24.0, 100.0, 20.0),
			},
			Change::Details {
				id: root,
				classes: "flex-col p-4".to_owned(),
				source: Some("src/app.rs:10:5".to_owned()),
			},
			Change::Details {
				id: text,
				classes: "text-lg".to_owned(),
				source: Some("src/app.rs:11:9".to_owned()),
			},
		],
		commits: vec![commit(7)],
	}
}

fn retext(id: u64, text: &str) -> Batch {
	Batch {
		root: Some(id - 1),
		changes: vec![Change::Text {
			id,
			text: Some(text.to_owned()),
		}],
		commits: vec![commit(1)],
	}
}

/// Waits until `check` holds.
fn eventually(what: &str, mut check: impl FnMut() -> bool) {
	let deadline = Instant::now() + PATIENCE;

	while !check() {
		assert!(Instant::now() < deadline, "timed out waiting for {what}");
		std::thread::sleep(Duration::from_millis(10));
	}
}

fn commits(hub: &Hub) -> Vec<(String, u64, usize)> {
	let state = hub.shared.state.lock().unwrap();
	state
		.events
		.iter()
		.filter_map(|(session, event)| {
			match event {
				Event::Commit { commit } => Some((session.clone(), commit.seq, commit.mutations)),
				_ => None,
			}
		})
		.collect()
}

#[test]
fn the_first_render_is_a_snapshot_and_later_ones_patch_it() {
	let hub = Hub::start();
	let reporter = Reporter::start(&hub.url, hello()).unwrap();
	let source = reporter.source();
	source.send(screen(1, "Hello"));

	let (session, snapshot) = hub.snapshot(0);
	assert_eq!(session, "session-1");
	assert_eq!(hub.shared.state.lock().unwrap().hellos, [hello()]);
	let tree = snapshot.tree;
	assert_eq!(tree.root, NodeRef::Native(1));
	let [root, heading] = &tree.nodes[..] else {
		panic!("two nodes: {:?}", tree.nodes);
	};
	assert_eq!(root.kind, "View");
	assert_eq!(root.classes, ["flex-col", "p-4"]);
	assert_eq!(
		root.source,
		Some(SourceLocation {
			file: "src/app.rs".to_owned(),
			line: 10,
			column: 5,
		})
	);
	assert_eq!(root.frame, Some(frame(0.0, 0.0, 320.0, 480.0)));
	assert_eq!(root.children, [NodeRef::Native(2)]);
	assert_eq!(heading.kind, "Text");
	assert_eq!(heading.text.as_deref(), Some("Hello"));
	assert_eq!(heading.parent, Some(NodeRef::Native(1)));
	assert_eq!(heading.classes, ["text-lg"]);
	assert_eq!(heading.source.as_ref().map(|source| source.line), Some(11));
	assert_eq!(heading.frame, Some(frame(16.0, 24.0, 100.0, 20.0)));
	assert_ne!(root.key, heading.key);
	hub.wait("the first commit", |state| {
		state
			.events
			.iter()
			.any(|(_, event)| matches!(event, Event::Commit { .. }))
			.then_some(())
	});

	source.send(retext(2, "Goodbye"));
	let ops = hub.wait("a patch", |state| {
		state.events.iter().find_map(|(_, event)| {
			match event {
				Event::TreePatch { patch } => Some(patch.ops.clone()),
				_ => None,
			}
		})
	});
	let [TreeOp::Update { node }] = &ops[..] else {
		panic!("one update: {ops:?}");
	};
	assert_eq!(node.text.as_deref(), Some("Goodbye"));
	assert_eq!(node.key, heading.key, "an edit keeps the match key");
	hub.wait("the second commit", |state| {
		(state
			.events
			.iter()
			.filter(|(_, event)| matches!(event, Event::Commit { .. }))
			.count()
			== 2)
			.then_some(())
	});
	assert_eq!(
		commits(&hub),
		[
			("session-1".to_owned(), 0, 7),
			("session-1".to_owned(), 1, 1)
		]
	);
}

#[test]
fn commands_snapshot_highlight_pick_and_profile() {
	let hub = Hub::start();
	let reporter = Reporter::start(&hub.url, hello()).unwrap();
	let source = reporter.source();
	source.send(screen(1, "Hello"));
	let (session, snapshot) = hub.snapshot(0);
	let heading = snapshot.tree.nodes[1].clone();

	hub.send(&session, Command::RequestSnapshot);
	hub.snapshot(1);

	hub.send(
		&session,
		Command::Highlight {
			key: heading.key.clone(),
		},
	);
	eventually("the highlight", || {
		reporter
			.highlight()
			.is_some_and(|highlight| highlight.node == Some((2, frame(16.0, 24.0, 100.0, 20.0))))
	});
	hub.send(
		&session,
		Command::Highlight {
			key: MatchKey::new(""),
		},
	);
	eventually("the highlight to clear", || reporter.highlight().is_none());

	assert!(!source.pick(2), "a tap is the app's until the hub picks");
	hub.send(&session, Command::Pick { on: true });
	eventually("picking", || reporter.picking());
	assert!(source.pick(2));
	assert!(!reporter.picking(), "one tap picks one node");
	let picked = hub.wait("the pick", |state| {
		state.events.iter().find_map(|(_, event)| {
			match event {
				Event::Picked { node, key } => Some((node.clone(), key.clone())),
				_ => None,
			}
		})
	});
	assert_eq!(picked, (NodeRef::Native(2), heading.key.clone()));
	eventually("the picked node's highlight", || {
		reporter
			.highlight()
			.is_some_and(|highlight| highlight.key == heading.key)
	});

	hub.send(&session, Command::StartProfiling);
	eventually("profiling", || source.profiling());
	hub.send(&session, Command::StopProfiling);
	eventually("profiling to stop", || !source.profiling());
}

#[test]
fn a_forgotten_session_is_replaced_with_a_fresh_snapshot() {
	let hub = Hub::start();
	let reporter = Reporter::start(&hub.url, hello()).unwrap();
	let source = reporter.source();
	source.send(screen(1, "Hello"));
	let (first, _) = hub.snapshot(0);
	hub.forget(&first);

	let (second, snapshot) = hub.snapshot(1);
	assert_eq!(second, "session-2");
	assert_eq!(snapshot.tree.nodes.len(), 2);
	assert_eq!(hub.shared.state.lock().unwrap().hellos.len(), 2);

	source.send(retext(2, "Again"));
	hub.wait("a patch to the new session", |state| {
		state
			.events
			.iter()
			.any(|(session, event)| {
				session == "session-2" && matches!(event, Event::TreePatch { .. })
			})
			.then_some(())
	});
	drop(reporter);
}

#[test]
fn a_reloaded_guest_keeps_the_session_and_sends_a_snapshot_once_it_renders() {
	let hub = Hub::start();
	let reporter = Reporter::start(&hub.url, hello()).unwrap();
	let old = reporter.source();
	old.send(screen(1, "Old"));
	hub.snapshot(0);

	// macOS replaces the guest: the old one closes before the new one renders.
	let new = reporter.source();
	drop(old);
	new.send(screen(10, "New"));
	let (session, snapshot) = hub.snapshot(1);
	assert_eq!(session, "session-1", "the session survives the reload");
	assert_eq!(snapshot.tree.root, NodeRef::Native(10));
	assert_eq!(snapshot.tree.nodes[1].text.as_deref(), Some("New"));

	// iOS and Android render the next guest while the last is still running.
	let newest = reporter.source();
	newest.send(screen(20, "Newest"));
	let (_, snapshot) = hub.snapshot(2);
	assert_eq!(snapshot.tree.root, NodeRef::Native(20));
	new.send(retext(11, "Hidden"));
	newest.send(retext(21, "Shown"));
	let ops = hub.wait("the newest guest's patch", |state| {
		state.events.iter().find_map(|(_, event)| {
			match event {
				Event::TreePatch { patch } => Some(patch.ops.clone()),
				_ => None,
			}
		})
	});
	assert!(
		matches!(&ops[..], [TreeOp::Update { node }] if node.text.as_deref() == Some("Shown")),
		"{ops:?}"
	);
	assert_eq!(hub.shared.state.lock().unwrap().hellos.len(), 1);
}

fn store(count: u64) -> Event {
	Event::StateChanged {
		name: "store".into(),
		value: Value::from_json("Store", json!({ "count": count })),
	}
}

fn set_store(count: u64) -> Command {
	Command::SetState {
		name: "store".into(),
		value: Value::from_json("Store", json!({ "count": count })),
	}
}

fn navigated(route: &str) -> Event {
	Event::RouteChanged {
		route: route.into(),
	}
}

/// The values sent after the first snapshot.
fn values(hub: &Hub) -> Vec<Event> {
	let state = hub.shared.state.lock().unwrap();
	state
		.events
		.iter()
		.map(|(_, event)| event)
		.filter(|event| leptoskit_plugin_devtools::is_value(event))
		.cloned()
		.collect()
}

/// The commands a guest received, as the `DevTools` plugin's inbox would.
fn commands(guest: &Guest) -> Arc<Mutex<Vec<Command>>> {
	let received = Arc::new(Mutex::new(Vec::new()));
	let sink = Arc::clone(&received);
	guest.on_command(move |command| sink.lock().unwrap().push(command));
	received
}

#[test]
fn a_guests_values_join_its_tree_in_one_session() {
	let hub = Hub::start();
	let reporter = Reporter::with_guests(&hub.url, hello()).unwrap();
	let source = reporter.source();
	let guest = source.guest();

	// A guest says what it feeds before it renders, so the session lists it.
	guest.hello(
		[
			Capability::Navigation,
			Capability::State,
			Capability::Editing,
		]
		.into(),
	);
	guest.report(true, vec![navigated("/clips"), store(1)]);
	source.send(screen(1, "Clips"));
	let (session, snapshot) = hub.snapshot(0);
	let hello = hub.shared.state.lock().unwrap().hellos[0].clone();
	assert!(hello.capabilities.contains(&Capability::Tree));
	assert!(hello.capabilities.contains(&Capability::State));
	assert!(hello.capabilities.contains(&Capability::Navigation));
	assert_eq!(snapshot.route.as_deref(), Some("/clips"));
	assert_eq!(snapshot.state.len(), 1);
	assert_eq!(snapshot.state[0].value.json, json!({ "count": 1 }));

	// Later changes are sent as they are, beside the tree's.
	guest.report(false, vec![store(2)]);
	hub.wait("the changed state", |state| {
		state
			.events
			.iter()
			.any(|(_, event)| *event == store(2))
			.then_some(())
	});
	// An unchanged value is not sent again.
	guest.report(false, vec![store(2), navigated("/clips")]);
	source.send(retext(2, "Clips again"));
	hub.wait("the patch after it", |state| {
		state
			.events
			.iter()
			.any(|(_, event)| matches!(event, Event::TreePatch { .. }))
			.then_some(())
	});
	assert_eq!(values(&hub), [store(2)]);

	// A snapshot the hub asks for carries the values, and the guest is asked
	// to resend its own.
	let received = commands(&guest);
	hub.send(&session, Command::RequestSnapshot);
	let (_, snapshot) = hub.snapshot(1);
	assert_eq!(snapshot.state[0].value.json, json!({ "count": 2 }));
	eventually("the guest's snapshot request", || {
		*received.lock().unwrap() == [Command::RequestSnapshot]
	});
}

#[test]
fn the_hubs_commands_for_the_guest_reach_it() {
	let hub = Hub::start();
	let reporter = Reporter::with_guests(&hub.url, hello()).unwrap();
	let source = reporter.source();
	let guest = source.guest();
	let received = commands(&guest);
	guest.hello([Capability::State].into());
	source.send(screen(1, "Clips"));
	let (session, _) = hub.snapshot(0);

	hub.send(&session, set_store(5));
	hub.send(
		&session,
		Command::Navigate {
			href: "/settings".into(),
		},
	);
	// The host's own commands stay with the host.
	hub.send(&session, Command::StartProfiling);
	eventually("the guest's commands", || {
		received.lock().unwrap().len() == 2
	});
	assert_eq!(
		*received.lock().unwrap(),
		[
			set_store(5),
			Command::Navigate {
				href: "/settings".into(),
			},
		]
	);
	eventually("profiling", || source.profiling());

	// The guest applies them and reports the result.
	guest.report(false, vec![store(5), navigated("/settings")]);
	hub.wait("the guest's new values", |state| {
		let events: Vec<&Event> = state.events.iter().map(|(_, event)| event).collect();
		(events.contains(&&store(5)) && events.contains(&&navigated("/settings"))).then_some(())
	});
}

#[test]
fn a_reloaded_guest_replaces_the_values_of_the_last() {
	let hub = Hub::start();
	let reporter = Reporter::with_guests(&hub.url, hello()).unwrap();
	let old = reporter.source();
	old.guest().hello([Capability::State].into());
	old.guest()
		.report(true, vec![store(9), navigated("/edited")]);
	old.send(screen(1, "Old"));
	let (session, snapshot) = hub.snapshot(0);
	assert_eq!(snapshot.state[0].value.json, json!({ "count": 9 }));

	// The new guest starts fresh: its first report replaces every value the
	// old one had, even one it doesn't report at all.
	let new = reporter.source();
	drop(old);
	new.guest().report(true, vec![store(0)]);
	new.send(screen(10, "New"));
	let (reloaded, snapshot) = hub.snapshot(1);
	assert_eq!(reloaded, session);
	assert_eq!(snapshot.state[0].value.json, json!({ "count": 0 }));
	assert_eq!(snapshot.route, None, "the old guest's route is gone");

	// A reset that leaves a value out sends a snapshot without it.
	new.guest().report(false, vec![navigated("/clips")]);
	hub.wait("the new route", |state| {
		state
			.events
			.iter()
			.any(|(_, event)| *event == navigated("/clips"))
			.then_some(())
	});
	new.guest().report(true, vec![navigated("/clips")]);
	let (_, snapshot) = hub.snapshot(2);
	assert!(snapshot.state.is_empty(), "{:?}", snapshot.state);
	assert_eq!(snapshot.route.as_deref(), Some("/clips"));
}

#[test]
fn a_guest_that_never_says_hello_still_gets_a_session() {
	let hub = Hub::start();
	let reporter = Reporter::with_guests(&hub.url, hello()).unwrap();
	let source = reporter.source();
	let started = Instant::now();
	source.send(screen(1, "Clips"));
	hub.snapshot(0);
	assert!(
		started.elapsed() >= Duration::from_millis(400),
		"the session waited for the guest"
	);
	let hello = hub.shared.state.lock().unwrap().hellos[0].clone();
	assert!(!hello.capabilities.contains(&Capability::State));
}
