//! The extension itself: messages between its surfaces, long-lived ports,
//! its ID, and the addresses of its files.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::Error;
use crate::Result;
use crate::js;
use crate::tabs::Tab;

/// The server `[extension].server` names, such as
/// `https://api.example.com`, without a trailing slash. `leptoskit build`
/// compiles it in.
#[must_use]
pub fn server() -> Option<&'static str> {
	option_env!("LEPTOSKIT_EXTENSION_SERVER").map(|server| server.trim_end_matches('/'))
}

/// Sends `message` to the background worker and waits for its reply, which
/// the worker's [`on_message`](crate::Background::on_message) handler
/// returns. Works from every page, overlay, and the worker's own tasks.
///
/// # Errors
///
/// When no worker answers, or its handler failed, with its reason.
pub async fn send<M: Serialize, R: DeserializeOwned>(message: &M) -> Result<R> {
	let reply: Value = js::call("runtime.sendMessage", &[message]).await?;

	if let Some(error) = reply.get("__leptoskit_error").and_then(Value::as_str) {
		return Err(Error::Browser(error.to_owned()));
	}

	serde_json::from_value(reply).map_err(|error| Error::Json(error.to_string()))
}

/// Opens a long-lived port to the background worker, named `name`. Prefer
/// this over [`send`] when a surface and the worker exchange many messages,
/// such as a content script streaming highlights.
///
/// # Errors
///
/// Outside an extension.
pub fn connect(name: &str) -> Result<Port> {
	#[cfg(target_arch = "wasm32")]
	{
		use wasm_bindgen::JsCast;

		let runtime = js::object("runtime")?;
		let function = js_sys::Reflect::get(&runtime, &"connect".into())
			.ok()
			.and_then(|value| value.dyn_into::<js_sys::Function>().ok())
			.ok_or_else(|| Error::Unavailable("runtime.connect".into()))?;
		let options = js::to_js(&serde_json::json!({ "name": name }))?;
		let port = function
			.call1(&runtime, &options)
			.map_err(|error| Error::Browser(js::message(&error)))?;
		if port.is_undefined() || port.is_null() {
			return Err(Error::Unavailable("runtime.connect".into()));
		}
		Ok(Port::from_js(name.to_owned(), port))
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = name;
		Err(Error::Unavailable("runtime.connect".into()))
	}
}

/// Who opened a port.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortSender {
	/// The tab, when a content script or a page in a tab opened it.
	#[serde(default)]
	pub tab: Option<Tab>,
	/// The sending page's address.
	#[serde(default)]
	pub url: Option<String>,
	/// The sending frame, `0` for a tab's top frame.
	#[serde(default)]
	pub frame_id: Option<i64>,
}

type MessageHandler = Rc<dyn Fn(Value)>;
type DisconnectHandler = Rc<dyn Fn()>;
type MessageHandlers = Rc<RefCell<HashMap<u64, MessageHandler>>>;
type DisconnectHandlers = Rc<RefCell<HashMap<u64, DisconnectHandler>>>;

/// A long-lived channel between a surface and the background worker.
///
/// Dropping the port does not disconnect it; call [`Port::disconnect`].
#[derive(Clone)]
pub struct Port {
	name: String,
	kind: PortKind,
	message_handlers: MessageHandlers,
	disconnect_handlers: DisconnectHandlers,
	next_handler: Rc<RefCell<u64>>,
}

#[derive(Clone)]
enum PortKind {
	/// A native `runtime.Port` on a page or content script.
	#[cfg(target_arch = "wasm32")]
	Js(wasm_bindgen::JsValue),
	/// A port the background worker tracks by id.
	Background { id: i64 },
}

impl std::fmt::Debug for Port {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Port")
			.field("name", &self.name)
			.finish_non_exhaustive()
	}
}

impl Port {
	/// The name passed to [`connect`] or configured on the other side.
	#[must_use]
	pub fn name(&self) -> &str {
		&self.name
	}

	/// The background-side port id, when this port came from
	/// [`Background::on_connect`](crate::Background::on_connect).
	#[must_use]
	pub fn id(&self) -> Option<i64> {
		match &self.kind {
			PortKind::Background { id } => Some(*id),
			#[cfg(target_arch = "wasm32")]
			PortKind::Js(_) => None,
		}
	}

	fn blank(name: String, kind: PortKind) -> Self {
		Self {
			name,
			kind,
			message_handlers: Rc::new(RefCell::new(HashMap::new())),
			disconnect_handlers: Rc::new(RefCell::new(HashMap::new())),
			next_handler: Rc::new(RefCell::new(1)),
		}
	}

	#[cfg(target_arch = "wasm32")]
	fn from_js(name: String, port: wasm_bindgen::JsValue) -> Self {
		Self::blank(name, PortKind::Js(port))
	}

	pub(crate) fn from_background(id: i64, name: String) -> Self {
		let port = Self::blank(name, PortKind::Background { id });
		register_background_port(id, port.clone());
		port
	}

	fn next_id(&self) -> u64 {
		let mut next = self.next_handler.borrow_mut();
		let id = *next;
		*next = next.wrapping_add(1);
		id
	}

	/// Sends `message` to the other end of the port.
	///
	/// # Errors
	///
	/// When the port is disconnected, or outside an extension.
	pub fn post_message<M: Serialize>(&self, message: &M) -> Result<()> {
		match &self.kind {
			#[cfg(target_arch = "wasm32")]
			PortKind::Js(port) => {
				use wasm_bindgen::JsCast;
				let function = js_sys::Reflect::get(port, &"postMessage".into())
					.ok()
					.and_then(|value| value.dyn_into::<js_sys::Function>().ok())
					.ok_or_else(|| Error::Unavailable("Port.postMessage".into()))?;
				function
					.call1(port, &js::to_js(message)?)
					.map_err(|error| Error::Browser(js::message(&error)))
					.map(drop)
			}
			PortKind::Background { id } => post_background(*id, message),
		}
	}

	/// Calls `handler` for every message on this port until the returned
	/// listener is dropped.
	///
	/// # Errors
	///
	/// Outside an extension, or when the port can't listen.
	pub fn on_message<M, F>(&self, handler: F) -> Result<crate::Listener>
	where
		M: DeserializeOwned + 'static,
		F: FnMut(M) + 'static,
	{
		let handler = Rc::new(RefCell::new(handler));
		match &self.kind {
			#[cfg(target_arch = "wasm32")]
			PortKind::Js(port) => {
				let on_message = js::property(port, "onMessage")?;
				let handler = handler.clone();
				js::listen_one(&on_message, move |message| {
					if let Ok(value) = js::from_js::<Value>(&message)
						&& let Ok(message) = serde_json::from_value::<M>(value)
					{
						(handler.borrow_mut())(message);
					}
				})
			}
			PortKind::Background { .. } => {
				let id = self.next_id();
				let erased: Rc<dyn Fn(Value)> = Rc::new(move |value| {
					if let Ok(message) = serde_json::from_value::<M>(value) {
						(handler.borrow_mut())(message);
					}
				});
				self.message_handlers.borrow_mut().insert(id, erased);
				let handlers = self.message_handlers.clone();
				Ok(js::Listener::guard(move || {
					handlers.borrow_mut().remove(&id);
				}))
			}
		}
	}

	/// Calls `handler` when the other end disconnects, until the returned
	/// listener is dropped.
	///
	/// # Errors
	///
	/// Outside an extension.
	pub fn on_disconnect(&self, handler: impl FnMut() + 'static) -> Result<crate::Listener> {
		let handler = Rc::new(RefCell::new(handler));
		match &self.kind {
			#[cfg(target_arch = "wasm32")]
			PortKind::Js(port) => {
				let on_disconnect = js::property(port, "onDisconnect")?;
				let handler = handler.clone();
				js::listen_one(&on_disconnect, move |_| (handler.borrow_mut())())
			}
			PortKind::Background { .. } => {
				let id = self.next_id();
				let erased: Rc<dyn Fn()> = Rc::new(move || {
					(handler.borrow_mut())();
				});
				self.disconnect_handlers.borrow_mut().insert(id, erased);
				let handlers = self.disconnect_handlers.clone();
				Ok(js::Listener::guard(move || {
					handlers.borrow_mut().remove(&id);
				}))
			}
		}
	}

	/// Closes the port.
	///
	/// # Errors
	///
	/// Outside an extension.
	pub fn disconnect(&self) -> Result<()> {
		match &self.kind {
			#[cfg(target_arch = "wasm32")]
			PortKind::Js(port) => {
				use wasm_bindgen::JsCast;
				let function = js_sys::Reflect::get(port, &"disconnect".into())
					.ok()
					.and_then(|value| value.dyn_into::<js_sys::Function>().ok())
					.ok_or_else(|| Error::Unavailable("Port.disconnect".into()))?;
				function
					.call0(port)
					.map_err(|error| Error::Browser(js::message(&error)))
					.map(drop)
			}
			PortKind::Background { id } => disconnect_background(*id),
		}
	}

	pub(crate) fn deliver_message(&self, message: &Value) {
		for handler in self.message_handlers.borrow().values() {
			handler(message.clone());
		}
	}

	pub(crate) fn deliver_disconnect(&self) {
		for handler in self.disconnect_handlers.borrow().values() {
			handler();
		}
	}
}

thread_local! {
	static BACKGROUND_PORTS: RefCell<HashMap<i64, Port>> = RefCell::new(HashMap::new());
}

fn register_background_port(id: i64, port: Port) {
	BACKGROUND_PORTS.with_borrow_mut(|ports| {
		ports.insert(id, port);
	});
}

pub(crate) fn deliver_background_message(id: i64, message: &Value) {
	BACKGROUND_PORTS.with_borrow(|ports| {
		if let Some(port) = ports.get(&id) {
			port.deliver_message(message);
		}
	});
}

pub(crate) fn deliver_background_disconnect(id: i64) {
	BACKGROUND_PORTS.with_borrow_mut(|ports| {
		if let Some(port) = ports.remove(&id) {
			port.deliver_disconnect();
		}
	});
}

fn post_background<M: Serialize>(id: i64, message: &M) -> Result<()> {
	#[cfg(target_arch = "wasm32")]
	{
		use wasm_bindgen::JsCast;
		let global = js_sys::global();
		let function = js_sys::Reflect::get(&global, &"__leptoskit_port_post".into())
			.ok()
			.and_then(|value| value.dyn_into::<js_sys::Function>().ok())
			.ok_or_else(|| Error::Unavailable("runtime.connect".into()))?;
		function
			.call2(
				&global,
				&wasm_bindgen::JsValue::from_f64(id as f64),
				&js::to_js(message)?,
			)
			.map_err(|error| Error::Browser(js::message(&error)))
			.map(drop)
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = (id, message);
		Err(Error::Unavailable("Port.postMessage".into()))
	}
}

fn disconnect_background(id: i64) -> Result<()> {
	#[cfg(target_arch = "wasm32")]
	{
		use wasm_bindgen::JsCast;
		let global = js_sys::global();
		let function = js_sys::Reflect::get(&global, &"__leptoskit_port_disconnect".into())
			.ok()
			.and_then(|value| value.dyn_into::<js_sys::Function>().ok())
			.ok_or_else(|| Error::Unavailable("runtime.connect".into()))?;
		function
			.call1(&global, &wasm_bindgen::JsValue::from_f64(id as f64))
			.map_err(|error| Error::Browser(js::message(&error)))
			.map(drop)
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = id;
		Err(Error::Unavailable("Port.disconnect".into()))
	}
}

/// The extension's ID: assigned by the store on Chromium, the add-on ID on
/// Firefox, and generated per install on Safari.
///
/// # Errors
///
/// Outside an extension.
pub fn id() -> Result<String> {
	#[cfg(target_arch = "wasm32")]
	{
		let runtime = js::object("runtime")?;

		return js_sys::Reflect::get(&runtime, &"id".into())
			.ok()
			.and_then(|id| id.as_string())
			.ok_or_else(|| Error::Unavailable("runtime.id".into()));
	}
	#[allow(unreachable_code)]
	Err(Error::Unavailable("runtime.id".into()))
}

/// The address of a file in the package, such as `popup.html`.
///
/// # Errors
///
/// Outside an extension.
pub async fn url(path: &str) -> Result<String> {
	js::call("runtime.getURL", &[&path]).await
}

/// Opens the options page.
///
/// # Errors
///
/// When the extension has none.
pub async fn open_options() -> Result<()> {
	js::call::<Value>("runtime.openOptionsPage", &[])
		.await
		.map(drop)
}

/// Reloads the whole extension: every page closes, and the worker starts
/// again.
///
/// # Errors
///
/// Outside an extension.
pub async fn reload() -> Result<()> {
	js::call::<Value>("runtime.reload", &[]).await.map(drop)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn background_ports_listen_post_and_clean_up() {
		let port = Port::from_background(7, "stream".into());
		assert_eq!(port.name(), "stream");
		assert_eq!(port.id(), Some(7));
		assert!(format!("{port:?}").contains("stream"));

		let seen = Rc::new(std::cell::Cell::new(0_u32));
		let disconnected = Rc::new(std::cell::Cell::new(false));
		let message_seen = seen.clone();
		let listener = port
			.on_message::<String, _>(move |message| {
				assert_eq!(message, "hello");
				message_seen.set(message_seen.get() + 1);
			})
			.expect("background ports listen without a browser");
		let disconnect_seen = disconnected.clone();
		let disconnect_listener = port
			.on_disconnect(move || disconnect_seen.set(true))
			.expect("background ports watch disconnect without a browser");

		assert_eq!(
			port.post_message(&"hello"),
			Err(Error::Unavailable("Port.postMessage".into()))
		);
		assert_eq!(
			port.disconnect(),
			Err(Error::Unavailable("Port.disconnect".into()))
		);

		deliver_background_message(7, &json!("hello"));
		// Wrong shape is ignored rather than panicking.
		deliver_background_message(7, &json!({"no": "string"}));
		assert_eq!(seen.get(), 1);

		deliver_background_disconnect(7);
		assert!(disconnected.get());
		// A second disconnect finds no port.
		deliver_background_disconnect(7);

		drop(listener);
		drop(disconnect_listener);
	}

	#[test]
	fn connect_is_unavailable_outside_a_browser() {
		assert!(matches!(
			connect("highlights"),
			Err(error) if error == Error::Unavailable("runtime.connect".into())
		));
		assert_eq!(id(), Err(Error::Unavailable("runtime.id".into())));
		assert_eq!(server(), None);
	}
}
