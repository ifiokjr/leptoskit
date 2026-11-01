//! End to end, headless: a real app running the compiled `hooks_guest` with
//! `DevTools`, reporting to a hub on an ephemeral port. The tree and a commit
//! come from the host; the change count and the route from the guest,
//! through the `DevTools` plugin; and `SetState` and `Navigate` from the hub
//! reach the guest and change what it shows.
//!
//! The test is the platform: it runs the guest's async plugin calls and
//! hands their replies back, as the generated apps do. Build the guest with
//! `devenv shell test:plugins:guest`, or by hand:
//!
//! ```sh
//! cargo build --target wasm32-unknown-unknown --package hooks_guest_probe \
//!   --features devtools --target-dir target/devtools-guest
//! LEPTOSKIT_DEVTOOLS_GUEST=target/devtools-guest/wasm32-unknown-unknown/debug/hooks_guest_probe.wasm \
//!   cargo test --package leptoskit_bridge_ffi --features devtools --test devtools_guest -- --ignored
//! ```

#![cfg(all(feature = "devtools", not(target_arch = "wasm32")))]
#![allow(unsafe_code)]

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::ffi::c_void;
use std::io::Read;
use std::io::Write;
use std::mem::MaybeUninit;
use std::net::TcpListener;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

use leptoskit_bridge_ffi::app::*;
use leptoskit_bridge_ffi::devtools::lk_devtools_hub;
use leptoskit_bridge_ffi::*;
use leptoskit_devtools_protocol::Capability;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Hello;
use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::Tree;
use leptoskit_devtools_protocol::TreeOp;
use leptoskit_devtools_protocol::Value;
use serde_json::json;

const PATIENCE: Duration = Duration::from_secs(10);

/// A hub that records what agents send, mirrors each session as the real
/// hub does, and holds a command long-poll for half a second.
#[derive(Default)]
struct Hub {
	state: Mutex<State>,
	changed: Condvar,
}

#[derive(Default)]
struct State {
	hellos: Vec<Hello>,
	open: Vec<String>,
	events: Vec<Event>,
	commands: HashMap<String, Vec<Command>>,
	/// The session as the hub mirrors it.
	tree: Option<Tree>,
	route: Option<String>,
	values: BTreeMap<String, serde_json::Value>,
	commits: usize,
}

impl State {
	fn apply(&mut self, event: Event) {
		match &event {
			Event::Snapshot { snapshot } => {
				self.tree = Some(snapshot.tree.clone());
				self.route.clone_from(&snapshot.route);
				self.values = snapshot
					.state
					.iter()
					.map(|named| (named.name.clone(), named.value.json.clone()))
					.collect();
			}
			Event::TreePatch { patch } => {
				let Some(tree) = self.tree.as_mut() else {
					return;
				};
				for op in &patch.ops {
					match op {
						TreeOp::Insert { node } | TreeOp::Update { node } => {
							tree.nodes.retain(|old| old.id != node.id);
							tree.nodes.push(node.clone());
						}
						TreeOp::Remove { id } => tree.nodes.retain(|old| old.id != *id),
					}
				}
			}
			Event::Commit { .. } => self.commits += 1,
			Event::RouteChanged { route } => self.route = Some(route.clone()),
			Event::StateChanged { name, value } => {
				self.values.insert(name.clone(), value.json.clone());
			}
			_ => {}
		}
		self.events.push(event);
	}

	fn texts(&self) -> Vec<String> {
		self.tree
			.iter()
			.flat_map(|tree| &tree.nodes)
			.filter_map(|node| node.text.clone())
			.collect()
	}
}

impl Hub {
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

		match (method, parts.as_slice()) {
			("POST", ["agent", "sessions"]) => {
				let id = format!("session-{}", state.hellos.len() + 1);
				state.hellos.push(serde_json::from_str(body).unwrap());
				state.open.push(id.clone());
				self.changed.notify_all();
				("200 OK", format!("{id:?}"))
			}
			("POST", ["agent", "sessions", _, "events"]) => {
				let events: Vec<Event> = serde_json::from_str(body).unwrap();
				for event in events {
					state.apply(event);
				}
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

	fn send(&self, command: Command) {
		let mut state = self.state.lock().unwrap();
		let session = state.open.last().cloned().expect("a session");
		state.commands.entry(session).or_default().push(command);
		self.changed.notify_all();
	}

	fn check(&self, found: impl Fn(&State) -> bool) -> bool {
		found(&self.state.lock().unwrap())
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

/// A finished async call: its id and reply.
type Finished = (u64, Box<WorkerReply>);

/// What a started async job reports back with.
struct Job {
	id: u64,
	finished: mpsc::Sender<Finished>,
}

unsafe extern "C" fn completed(context: *mut c_void, reply: *mut WorkerReply) {
	// SAFETY: `start` hands this exactly-once callback its boxed job.
	let job = unsafe { Box::from_raw(context.cast::<Job>()) };
	// SAFETY: The callback owns the reply.
	let _ = job.finished.send((job.id, unsafe { Box::from_raw(reply) }));
}

/// The platform: starts the app's calls and hands their replies back. Its
/// frame clock is on, as on macOS, iOS, and Android.
struct Platform {
	app: *mut App,
	finished: mpsc::Receiver<Finished>,
	sender: mpsc::Sender<Finished>,
}

impl Platform {
	fn start(wasm: &[u8]) -> Self {
		let mut diagnostic = MaybeUninit::uninit();
		// SAFETY: The input lives for the call and the output is writable.
		let app = unsafe {
			create(wasm.as_ptr(), wasm.len(), diagnostic.as_mut_ptr(), |_| {
				Ok(())
			})
		};
		assert!(!app.is_null());
		let (sender, finished) = mpsc::channel();
		let platform = Self {
			app,
			finished,
			sender,
		};
		// SAFETY: The app is live and used on this thread only.
		unsafe {
			assert_eq!(lk_app_enable_frames(app), 0);
			assert_eq!(lk_app_start(app, 320.0, 480.0), 0);
		}
		platform.pump();
		platform
	}

	/// Starts every call the guest made, hands back the replies that
	/// arrived, and renders.
	fn pump(&self) {
		loop {
			let mut request = MaybeUninit::uninit();
			// SAFETY: The app is live; the output is writable.
			match unsafe { lk_app_next_request(self.app, request.as_mut_ptr()) } {
				0 => break,
				2 => continue,
				1 => {}
				status => panic!("next request: {status}"),
			}
			// SAFETY: The call wrote the request.
			let request = unsafe { request.assume_init() };
			// SAFETY: The request is live until released below.
			let job = unsafe { lk_app_async(self.app, &raw const request) };
			assert!(!job.is_null(), "only the DevTools plugin is installed");
			let context = Box::into_raw(Box::new(Job {
				id: request.id,
				finished: self.sender.clone(),
			}));
			// SAFETY: The job is consumed with its boxed context; the payload
			// is released once.
			unsafe {
				lk_buffer_release(request.payload);
				lk_async_start(job, context.cast(), completed);
			}
		}

		while let Ok((id, reply)) = self.finished.try_recv() {
			// SAFETY: The app's bridge is borrowed for the call; the reply is
			// consumed.
			unsafe {
				lk_worker_complete(lk_app_plugins(self.app), id, Box::into_raw(reply));
				lk_app_wake(self.app);
			}
		}

		// SAFETY: The app is live; the buffer is released once.
		unsafe {
			// The guest's DevTools never ask for a frame: the display link
			// stays off unless the app itself animates.
			assert_eq!(lk_app_frame_requested(self.app), 0);
			lk_buffer_release(lk_app_render(self.app));
		}
	}

	/// Pumps until the hub has what `found` looks for.
	fn until(&self, hub: &Hub, what: &str, found: impl Fn(&State) -> bool) {
		let deadline = Instant::now() + PATIENCE;

		while !hub.check(&found) {
			assert!(Instant::now() < deadline, "timed out waiting for {what}");
			self.pump();
			std::thread::sleep(Duration::from_millis(5));
		}
	}

	fn press(&self, node: &Node) {
		let NodeRef::Native(id) = node.id else {
			panic!("a native node: {node:?}");
		};
		// SAFETY: The app is live; a press carries no text.
		assert_eq!(
			unsafe { lk_app_event(self.app, id as usize, 0, std::ptr::null(), 0) },
			0
		);
		self.pump();
	}
}

impl Drop for Platform {
	fn drop(&mut self) {
		// SAFETY: The app is live and released once.
		unsafe { lk_app_destroy(self.app) };
	}
}

#[test]
#[ignore = "run test:plugins:guest to compile the hooks guest with DevTools"]
fn a_guest_reports_its_state_and_route_and_takes_the_hubs_commands() {
	let path = std::env::var("LEPTOSKIT_DEVTOOLS_GUEST").expect("the guest's wasm path");
	let wasm = std::fs::read(path).expect("the guest's wasm");
	let hub = Arc::new(Hub::default());
	let url = hub.start();
	// SAFETY: Both inputs live for the call.
	let configured =
		unsafe { lk_devtools_hub(url.as_ptr(), url.len(), b"hooks_guest".as_ptr(), 11) };
	assert_eq!(configured, 0);

	// The host reports the tree, and the guest its state and route.
	let platform = Platform::start(&wasm);
	platform.until(&hub, "the tree, the state, and the route", |state| {
		state.texts().contains(&"Changes: 0".to_owned())
			&& state.values.get("changes") == Some(&json!(0))
			&& state.route.as_deref() == Some("/")
	});
	let hello = hub.state.lock().unwrap().hellos[0].clone();
	assert_eq!(hello.app, "hooks_guest");
	for capability in [
		Capability::Tree,
		Capability::Navigation,
		Capability::State,
		Capability::Editing,
	] {
		assert!(hello.capabilities.contains(&capability), "{hello:?}");
	}

	// Each node names the component that built it, across the wire: the
	// count is a `#[component]`, its container is built outside any.
	{
		let state = hub.state.lock().unwrap();
		let nodes = &state.tree.as_ref().unwrap().nodes;
		let count = nodes
			.iter()
			.find(|node| node.text.as_deref() == Some("Changes: 0"))
			.expect("the count");
		assert_eq!(count.component.as_deref(), Some("ChangeCount"));
		assert!(count.source.is_some(), "and the line that built it");
		let root = nodes.iter().find(|node| node.parent.is_none()).unwrap();
		assert_eq!(root.component, None);
	}

	// A press commits, and the guest reports the state it changed.
	let button = hub
		.state
		.lock()
		.unwrap()
		.tree
		.as_ref()
		.unwrap()
		.nodes
		.iter()
		.find(|node| node.kind == "Button")
		.cloned()
		.expect("the switch");
	platform.press(&button);
	platform.until(&hub, "the press's commit and state", |state| {
		state.commits >= 2
			&& state.texts().contains(&"Changes: 1".to_owned())
			&& state.values.get("changes") == Some(&json!(1))
	});

	// The hub sets the state: the guest renders it and reports it.
	hub.send(Command::SetState {
		name: "changes".into(),
		value: Value::from_json("u32", json!(5)),
	});
	platform.until(&hub, "the state the hub set", |state| {
		state.texts().contains(&"Changes: 5".to_owned())
			&& state.values.get("changes") == Some(&json!(5))
	});

	// The hub navigates the guest's router.
	hub.send(Command::Navigate {
		href: "/settings".into(),
	});
	platform.until(&hub, "the new route", |state| {
		state.route.as_deref() == Some("/settings")
	});
	assert_eq!(hub.state.lock().unwrap().hellos.len(), 1, "one session");
}
