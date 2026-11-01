//! The web agent: reports a browser page to the local hub.
//!
//! The renderer marks each element with `data-lk-node` (its path),
//! `data-lk-key` (its cross-platform match key), `data-lk-kind`, and
//! `data-lk-source`. The agent reads those, so it never touches the render
//! path: a `MutationObserver` marks the page dirty, and a debounced flush
//! sends a snapshot only when the tree it reads has changed. Commands arrive
//! on a long-poll that runs beside it.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use gloo_net::http::Request;
use leptoskit_devtools_protocol::Capability;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Frame;
use leptoskit_devtools_protocol::Hello;
use leptoskit_devtools_protocol::MatchKey;
use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::Platform;
use leptoskit_devtools_protocol::SessionId;
use leptoskit_devtools_protocol::Snapshot;
use leptoskit_devtools_protocol::SourceLocation;
use leptoskit_devtools_protocol::Tree;
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;

/// How long the page must be quiet before a snapshot is sent.
const DEBOUNCE_MS: u32 = 150;

/// How long to wait before retrying after the hub could not be reached.
const RETRY_MS: u32 = 2_000;

/// The id of the highlight rectangle the agent draws.
const OVERLAY: &str = "leptoskit-devtools-overlay";

/// The global the `DevTools` panel reads to find this page's session.
const SESSION_GLOBAL: &str = "__LEPTOSKIT_DEVTOOLS_SESSION__";

/// Start the web agent, reporting to `hub`.
pub(crate) fn install(hub: &str, app: &str) {
	let agent = Rc::new(Agent {
		hub: hub.to_owned(),
		app: app.to_owned(),
		session: RefCell::new(None),
		after: Cell::new(0),
		last_sent: RefCell::new(None),
		flush_pending: Cell::new(false),
	});
	spawn_local(async move {
		agent.connect().await;
		agent.observe();
		agent.schedule_flush();
		agent.poll_commands().await;
	});
}

struct Agent {
	hub: String,
	app: String,
	session: RefCell<Option<SessionId>>,
	after: Cell<usize>,
	/// The JSON of the last snapshot sent, so an unchanged tree is not sent
	/// again.
	last_sent: RefCell<Option<String>>,
	flush_pending: Cell<bool>,
}

impl Agent {
	/// Open a session, retrying until the hub answers.
	async fn connect(&self) {
		let hello = Hello::new(
			self.app.clone(),
			Platform::Web,
			None,
			[Capability::Tree, Capability::Picking],
		);
		loop {
			match post_json::<_, SessionId>(&format!("{}/agent/sessions", self.hub), &hello).await {
				Ok(session) => {
					let _ = js_sys::Reflect::set(
						&js_sys::global(),
						&SESSION_GLOBAL.into(),
						&session.0.clone().into(),
					);
					*self.session.borrow_mut() = Some(session);
					self.after.set(0);
					*self.last_sent.borrow_mut() = None;
					return;
				}
				Err(error) => {
					web_sys::console::debug_1(&format!("[devtools] {error}").into());
					gloo_timers::future::TimeoutFuture::new(RETRY_MS).await;
				}
			}
		}
	}

	fn session(&self) -> Option<String> {
		self.session
			.borrow()
			.as_ref()
			.map(|session| session.0.clone())
	}

	/// Mark the page dirty whenever the DOM changes.
	fn observe(self: &Rc<Self>) {
		let agent = Rc::clone(self);
		let callback = Closure::<dyn FnMut()>::new(move || agent.schedule_flush());
		let Ok(observer) = web_sys::MutationObserver::new(callback.as_ref().unchecked_ref()) else {
			return;
		};
		// The observer lives as long as the page.
		callback.forget();
		let options = web_sys::MutationObserverInit::new();
		options.set_child_list(true);
		options.set_subtree(true);
		options.set_attributes(true);
		options.set_character_data(true);
		if let Some(body) = document().and_then(|document| document.body()) {
			let _ = observer.observe_with_options(&body, &options);
		}
	}

	fn schedule_flush(self: &Rc<Self>) {
		if self.flush_pending.replace(true) {
			return;
		}
		let agent = Rc::clone(self);
		spawn_local(async move {
			gloo_timers::future::TimeoutFuture::new(DEBOUNCE_MS).await;
			agent.flush_pending.set(false);
			agent.flush().await;
		});
	}

	/// Send a snapshot when the tree differs from the last one sent.
	async fn flush(&self) {
		let Some(session) = self.session() else {
			return;
		};
		let snapshot = snapshot();
		let Ok(json) = serde_json::to_string(&snapshot) else {
			return;
		};
		if self.last_sent.borrow().as_deref() == Some(json.as_str()) {
			return;
		}
		let events = [Event::Snapshot { snapshot }];
		if post(&self.events_url(&session), &events).await.is_ok() {
			*self.last_sent.borrow_mut() = Some(json);
		}
	}

	fn events_url(&self, session: &str) -> String {
		format!("{}/agent/sessions/{session}/events", self.hub)
	}

	/// Long-poll for commands for as long as the page lives, opening a new
	/// session when the hub forgets this one (it restarted, or the tab slept
	/// past the session's expiry).
	async fn poll_commands(self: Rc<Self>) {
		loop {
			let Some(session) = self.session() else {
				self.connect().await;
				continue;
			};
			let url = format!(
				"{}/agent/sessions/{session}/commands?after={}",
				self.hub,
				self.after.get()
			);
			match get::<Vec<Command>>(&url).await {
				Ok(commands) => {
					self.after.set(self.after.get() + commands.len());
					for command in commands {
						self.apply(command);
					}
				}
				Err(Failure::Gone) => {
					*self.session.borrow_mut() = None;
				}
				Err(Failure::Other(error)) => {
					web_sys::console::debug_1(&format!("[devtools] {error}").into());
					gloo_timers::future::TimeoutFuture::new(RETRY_MS).await;
				}
			}
		}
	}

	fn apply(self: &Rc<Self>, command: Command) {
		match command {
			Command::Highlight { key } | Command::Select { key, .. } => highlight(&key.0),
			Command::Pick { on } if on => self.start_picking(),
			Command::Pick { .. } => stop_picking(),
			Command::RequestSnapshot => {
				*self.last_sent.borrow_mut() = None;
				self.schedule_flush();
			}
			_ => {}
		}
	}

	fn start_picking(self: &Rc<Self>) {
		if PICKING.with(Cell::get) {
			return;
		}
		PICKING.with(|picking| picking.set(true));
		let agent = Rc::clone(self);
		let closure =
			Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |event: web_sys::MouseEvent| {
				let Some(element) = event
					.target()
					.and_then(|target| target.dyn_into::<web_sys::Element>().ok())
					.and_then(|target| target.closest("[data-lk-node]").ok().flatten())
				else {
					return;
				};
				// The pick replaces the click: the app never sees it.
				event.prevent_default();
				event.stop_propagation();
				let path = element.get_attribute("data-lk-node").unwrap_or_default();
				let key = element.get_attribute("data-lk-key").unwrap_or_default();
				highlight(&key);
				stop_picking();
				let Some(session) = agent.session() else {
					return;
				};
				let url = agent.events_url(&session);
				spawn_local(async move {
					let event = Event::Picked {
						node: NodeRef::Web(path),
						key: MatchKey::new(key),
					};
					let _ = post(&url, &[event]).await;
				});
			});
		if let Some(document) = document() {
			// Capture, so the pick runs before the app's own handlers.
			let _ = document.add_event_listener_with_callback_and_bool(
				"click",
				closure.as_ref().unchecked_ref(),
				true,
			);
		}
		PICK_LISTENER.with(|listener| *listener.borrow_mut() = Some(closure));
	}
}

type PickListener = RefCell<Option<Closure<dyn FnMut(web_sys::MouseEvent)>>>;

thread_local! {
	static PICKING: Cell<bool> = const { Cell::new(false) };
	static PICK_LISTENER: PickListener = const { RefCell::new(None) };
}

fn document() -> Option<web_sys::Document> {
	web_sys::window().and_then(|window| window.document())
}

fn stop_picking() {
	PICKING.with(|picking| picking.set(false));
	PICK_LISTENER.with(|listener| {
		let Some(closure) = listener.borrow_mut().take() else {
			return;
		};
		if let Some(document) = document() {
			let _ = document.remove_event_listener_with_callback_and_bool(
				"click",
				closure.as_ref().unchecked_ref(),
				true,
			);
		}
	});
}

/// Draw the highlight rectangle over the element with `key`, or remove it
/// when no element has that key.
fn highlight(key: &str) {
	let Some(document) = document() else {
		return;
	};
	let target = (!key.is_empty())
		.then(|| {
			document
				.query_selector(&format!("[data-lk-key=\"{key}\"]"))
				.ok()
				.flatten()
		})
		.flatten();
	let Some(target) = target else {
		if let Some(overlay) = document.get_element_by_id(OVERLAY) {
			overlay.remove();
		}
		return;
	};
	let overlay = document.get_element_by_id(OVERLAY).or_else(|| {
		let overlay = document.create_element("div").ok()?;
		overlay.set_id(OVERLAY);
		document.body()?.append_child(&overlay).ok()?;
		Some(overlay)
	});
	let Some(overlay) = overlay else {
		return;
	};
	let rect = target.get_bounding_client_rect();
	let style = format!(
		"position:fixed;left:{}px;top:{}px;width:{}px;height:{}px;outline:2px solid \
		 #e500ff;background:#e500ff22;pointer-events:none;z-index:2147483647",
		rect.x(),
		rect.y(),
		rect.width(),
		rect.height()
	);
	let _ = overlay.set_attribute("style", &style);
}

fn snapshot() -> Snapshot {
	let mut nodes = Vec::new();
	let mut root = None;
	let mut children: BTreeMap<String, Vec<NodeRef>> = BTreeMap::new();

	for (path, element) in dom_nodes() {
		match parent_path(&path) {
			Some(parent) => {
				children
					.entry(parent)
					.or_default()
					.push(NodeRef::Web(path.clone()));
			}
			None => root = root.or_else(|| Some(NodeRef::Web(path.clone()))),
		}
		nodes.push(node_for(&element, &path));
	}

	for node in &mut nodes {
		if let NodeRef::Web(path) = &node.id {
			node.children = children.remove(path).unwrap_or_default();
		}
	}

	Snapshot {
		tree: Tree {
			root: root.unwrap_or_else(|| NodeRef::Web(String::new())),
			nodes,
		},
		route: web_sys::window().and_then(|window| window.location().pathname().ok()),
		queries: Vec::new(),
		mutations: Vec::new(),
		forms: Vec::new(),
		state: Vec::new(),
		calls: Vec::new(),
		rpc: Vec::new(),
	}
}

fn dom_nodes() -> Vec<(String, web_sys::Element)> {
	let mut found = Vec::new();
	let Some(elements) =
		document().and_then(|document| document.query_selector_all("[data-lk-node]").ok())
	else {
		return found;
	};
	for index in 0..elements.length() {
		let Some(element) = elements
			.item(index)
			.and_then(|node| node.dyn_into::<web_sys::Element>().ok())
		else {
			continue;
		};
		let path = element.get_attribute("data-lk-node").unwrap_or_default();
		found.push((path, element));
	}
	found
}

/// The parent of a node path: `.0.1` is a child of `.0`, which is a child of
/// the root, whose path is empty.
fn parent_path(path: &str) -> Option<String> {
	path.rfind('.').map(|at| path[..at].to_owned())
}

/// A `file:line:column` attribute as a source location.
fn source(text: &str) -> Option<SourceLocation> {
	let mut parts = text.rsplitn(3, ':');
	let column = parts.next()?.parse().ok()?;
	let line = parts.next()?.parse().ok()?;
	let file = parts.next()?.to_owned();
	Some(SourceLocation { file, line, column })
}

fn node_for(element: &web_sys::Element, path: &str) -> Node {
	let rect = element.get_bounding_client_rect();
	let text = (element.get_attribute("data-lk-kind").as_deref() == Some("Text"))
		.then(|| element.text_content())
		.flatten();

	Node {
		id: NodeRef::Web(path.to_owned()),
		key: MatchKey::new(
			element
				.get_attribute("data-lk-key")
				.unwrap_or_else(|| path.to_owned()),
		),
		parent: parent_path(path).map(NodeRef::Web),
		kind: element
			.get_attribute("data-lk-kind")
			.unwrap_or_else(|| element.tag_name().to_lowercase()),
		source: element
			.get_attribute("data-lk-source")
			.as_deref()
			.and_then(source),
		frame: Some(Frame {
			x: rect.x() as f32,
			y: rect.y() as f32,
			width: rect.width() as f32,
			height: rect.height() as f32,
		}),
		classes: element
			.class_name()
			.split_whitespace()
			.map(str::to_owned)
			.collect(),
		text,
		children: Vec::new(),
		styles: Vec::new(),
	}
}

/// Why a request to the hub failed.
enum Failure {
	/// The hub no longer knows the session.
	Gone,
	/// Anything else: the hub is down, or answered with an error.
	Other(String),
}

async fn get<R: DeserializeOwned>(url: &str) -> Result<R, Failure> {
	let response = Request::get(url)
		.send()
		.await
		.map_err(|error| Failure::Other(error.to_string()))?;
	if response.status() == 404 {
		return Err(Failure::Gone);
	}
	if !response.ok() {
		return Err(Failure::Other(response.text().await.unwrap_or_default()));
	}
	response
		.json::<R>()
		.await
		.map_err(|error| Failure::Other(error.to_string()))
}

async fn post_json<T: Serialize, R: DeserializeOwned>(url: &str, body: &T) -> Result<R, String> {
	let response = Request::post(url)
		.header("content-type", "application/json")
		.json(body)
		.map_err(|error| error.to_string())?
		.send()
		.await
		.map_err(|error| error.to_string())?;
	if !response.ok() {
		return Err(response.text().await.unwrap_or_default());
	}
	response
		.json::<R>()
		.await
		.map_err(|error| error.to_string())
}

async fn post<T: Serialize>(url: &str, body: &T) -> Result<(), String> {
	let response = Request::post(url)
		.header("content-type", "application/json")
		.json(body)
		.map_err(|error| error.to_string())?
		.send()
		.await
		.map_err(|error| error.to_string())?;
	if response.status() != 204 && !response.ok() {
		return Err(response.text().await.unwrap_or_default());
	}
	Ok(())
}
