//! The web agent: reports a browser page to the local hub.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::BTreeMap;

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
use leptoskit_devtools_protocol::Tree;
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::spawn_local;

/// How long one snapshot-and-poll cycle waits.
const POLL_MS: u32 = 1_000;

/// Start the web agent when the page was built with a hub URL.
pub fn install(app: &str) {
	let Some(hub) = option_env!("LEPTOSKIT_DEVTOOLS_HUB") else {
		return;
	};
	let app = app.to_owned();
	let hub = hub.to_owned();
	spawn_local(async move {
		if let Err(error) = run(hub, app).await {
			web_sys::console::warn_1(&format!("[devtools] {error}").into());
		}
	});
}

async fn run(hub: String, app: String) -> Result<(), String> {
	let hello = Hello::new(
		app,
		Platform::Web,
		None,
		[
			Capability::Tree,
			Capability::Styles,
			Capability::Queries,
			Capability::Picking,
		],
	);
	let session: SessionId = post_json(&format!("{hub}/agent/sessions"), &hello).await?;
	let _ = js_sys::Reflect::set(
		&js_sys::global(),
		&"__LEPTOSKIT_DEVTOOLS_SESSION__".into(),
		&session.0.clone().into(),
	);
	let mut after = 0;

	loop {
		let events = [Event::Snapshot {
			snapshot: snapshot(),
		}];
		let _ = post(
			&format!("{hub}/agent/sessions/{}/events", session.0),
			&events,
		)
		.await;
		let commands: Vec<Command> = get(&format!(
			"{hub}/agent/sessions/{}/commands?after={after}",
			session.0
		))
		.await
		.unwrap_or_default();

		after += commands.len();

		for command in commands {
			apply(command, &hub, &session);
		}

		gloo_timers::future::TimeoutFuture::new(POLL_MS).await;
	}
}

fn apply(command: Command, hub: &str, session: &SessionId) {
	match command {
		Command::Highlight { key } | Command::Select { key, .. } => highlight(&key.0),
		Command::Pick { on } => set_picking(on, hub, session),
		_ => {}
	}
}

type PickListener = RefCell<Option<Closure<dyn FnMut(web_sys::MouseEvent)>>>;

thread_local! {
	static PICKING: Cell<bool> = const { Cell::new(false) };
	static PICK_LISTENER: PickListener = const { RefCell::new(None) };
}

fn set_picking(on: bool, hub: &str, session: &SessionId) {
	if !on {
		stop_picking();
		return;
	}
	if PICKING.with(Cell::get) {
		return;
	}
	PICKING.with(|picking| picking.set(true));
	let hub = hub.to_owned();
	let session = session.0.clone();
	let closure =
		Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |event: web_sys::MouseEvent| {
			let Some(target) = event
				.target()
				.and_then(|target| target.dyn_into::<web_sys::Element>().ok())
			else {
				return;
			};
			let Ok(Some(element)) = target.closest("[data-lk-node]") else {
				return;
			};
			let path = element.get_attribute("data-lk-node").unwrap_or_default();
			let hub = hub.clone();
			let session = session.clone();
			spawn_local(async move {
				let event = Event::Picked {
					node: NodeRef::Web(path.clone()),
					key: MatchKey::new(path),
				};
				let _ = post(&format!("{hub}/agent/sessions/{session}/events"), &[event]).await;
			});
			stop_picking();
		});
	if let Some(document) = web_sys::window().and_then(|window| window.document()) {
		let _ =
			document.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
	}
	PICK_LISTENER.with(|listener| *listener.borrow_mut() = Some(closure));
}

fn stop_picking() {
	PICKING.with(|picking| picking.set(false));
	PICK_LISTENER.with(|listener| {
		let Some(closure) = listener.borrow_mut().take() else {
			return;
		};
		if let Some(document) = web_sys::window().and_then(|window| window.document()) {
			let _ = document
				.remove_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
		}
	});
	if let Some(document) = web_sys::window().and_then(|window| window.document())
		&& let Some(overlay) = document.get_element_by_id("leptoskit-devtools-overlay")
	{
		overlay.remove();
	}
}

fn highlight(key: &str) {
	let Some(document) = web_sys::window().and_then(|window| window.document()) else {
		return;
	};
	let Ok(Some(element)) = document.query_selector(&format!("[data-lk-node=\"{key}\"]")) else {
		return;
	};
	let rect = element.get_bounding_client_rect();
	let overlay = document
		.get_element_by_id("leptoskit-devtools-overlay")
		.or_else(|| {
			let overlay = document.create_element("div").ok()?;
			overlay.set_id("leptoskit-devtools-overlay");
			document.body()?.append_child(&overlay).ok()?;
			Some(overlay)
		});
	let Some(overlay) = overlay else {
		return;
	};

	let style = format!(
		"position:fixed;left:{}px;top:{}px;width:{}px;height:{}px;border:2px solid \
		 #e500ff;pointer-events:none;z-index:2147483647",
		rect.x(),
		rect.y(),
		rect.width(),
		rect.height()
	);
	let _ = overlay.set_attribute("style", &style);
}

fn snapshot() -> Snapshot {
	let mut nodes = Vec::new();
	let mut root = NodeRef::Web(String::new());
	let mut children: BTreeMap<String, Vec<NodeRef>> = BTreeMap::new();

	for (path, element) in dom_nodes() {
		if path.is_empty() {
			root = NodeRef::Web(path.clone());
		}

		if let Some(parent) = parent_path(&path) {
			children
				.entry(parent)
				.or_default()
				.push(NodeRef::Web(path.clone()));
		}

		nodes.push(node_for(&element, &path));
	}

	for node in &mut nodes {
		if let NodeRef::Web(path) = &node.id {
			node.children = children.get(path).cloned().unwrap_or_default();
		}
	}

	Snapshot {
		tree: Tree { root, nodes },
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

	if let Some(document) = web_sys::window().and_then(|window| window.document())
		&& let Ok(elements) = document.query_selector_all("[data-lk-node]")
	{
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
	}

	found
}

fn parent_path(path: &str) -> Option<String> {
	path.rfind('.')
		.map(|at| path[..at].to_owned())
		.or_else(|| (!path.is_empty()).then(String::new))
}

fn node_for(element: &web_sys::Element, path: &str) -> Node {
	let rect = element.get_bounding_client_rect();

	Node {
		id: NodeRef::Web(path.to_owned()),
		key: MatchKey::new(path),
		parent: parent_path(path).map(NodeRef::Web),
		kind: element.tag_name(),
		source: None,
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
		text: element.text_content(),
		children: Vec::new(),
		styles: Vec::new(),
	}
}

async fn get<R: DeserializeOwned>(url: &str) -> Result<R, String> {
	let response = Request::get(url)
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
