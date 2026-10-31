//! The background worker: the surface without a screen.
//!
//! The generated worker registers the browser's listeners on its first turn,
//! which a cold start requires, and queues every event until the wasm is
//! ready. Then it calls the app's `background` function, which registers a
//! handler for each event it answers, and hands over the queue.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

use futures::FutureExt;
use futures::future::LocalBoxFuture;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::tabs::Tab;

type Handler = Rc<dyn Fn(Value) -> LocalBoxFuture<'static, Result<Value, String>>>;

thread_local! {
	static HANDLERS: RefCell<HashMap<String, Handler>> = RefCell::new(HashMap::new());
}

/// Who sent a message.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sender {
	/// The tab, when a content script or a page in a tab sent it.
	#[serde(default)]
	pub tab: Option<Tab>,
	/// The sending page's address.
	#[serde(default)]
	pub url: Option<String>,
	/// The sending frame, `0` for a tab's top frame.
	#[serde(default)]
	pub frame_id: Option<i64>,
}

/// Why the extension was installed or updated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
	/// `install`, `update`, `browser_update`, or `shared_module_update`.
	pub reason: String,
	/// The version before an update.
	#[serde(default)]
	pub previous_version: Option<String>,
}

/// A context menu item was chosen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuClick {
	/// The item's ID.
	pub menu_item_id: Value,
	/// The selected text, for a `selection` item.
	#[serde(default)]
	pub selection_text: Option<String>,
	/// The page the menu was opened on.
	#[serde(default)]
	pub page_url: Option<String>,
	/// The link the menu was opened on.
	#[serde(default)]
	pub link_url: Option<String>,
	/// The tab it was opened in.
	#[serde(default)]
	pub tab: Option<Tab>,
}

/// A keyboard shortcut was pressed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Command {
	/// The command's name in `[extension.commands]`.
	pub command: String,
	/// The tab that was active.
	#[serde(default)]
	pub tab: Option<Tab>,
}

/// An alarm went off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alarm {
	/// The alarm's name.
	pub name: String,
	/// When it was due, in milliseconds since the epoch.
	pub scheduled_time: f64,
}

/// A tab finished loading, or otherwise changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabUpdate {
	/// The tab's ID.
	pub tab_id: i64,
	/// What changed, such as `{ "status": "complete" }`.
	pub change_info: Value,
	/// The tab after the change.
	pub tab: Tab,
}

#[derive(Deserialize)]
struct MessageEvent<M> {
	message: M,
	#[serde(default)]
	sender: Sender,
}

#[derive(Deserialize)]
struct ContextMenuEvent {
	info: MenuClick,
	#[serde(default)]
	tab: Option<Tab>,
}

/// Registers the worker's handlers. Each kind of event has at most one; a
/// later registration replaces it.
#[derive(Debug)]
pub struct Background {
	_private: (),
}

impl Background {
	/// Handles an event by its kind, as `[extension.background].events`
	/// names it, with its details read as `D`. What the handler returns is
	/// the reply, for events that have one.
	pub fn on<D, R, F>(&self, kind: &str, handler: impl Fn(D) -> F + 'static)
	where
		D: DeserializeOwned + 'static,
		R: Serialize + 'static,
		F: Future<Output = R> + 'static,
	{
		let handler = Rc::new(handler);
		let erased: Handler = Rc::new(move |detail: Value| {
			let handler = handler.clone();
			async move {
				let detail: D =
					serde_json::from_value(detail).map_err(|error| error.to_string())?;
				let reply = handler(detail).await;
				serde_json::to_value(reply).map_err(|error| error.to_string())
			}
			.boxed_local()
		});
		HANDLERS.with_borrow_mut(|handlers| handlers.insert(kind.to_owned(), erased));
	}

	/// Answers messages sent with [`runtime::send`](crate::runtime::send),
	/// from any surface or content script. The reply goes back to the sender.
	pub fn on_message<M, R, F>(&self, handler: impl Fn(M, Sender) -> F + 'static)
	where
		M: DeserializeOwned + 'static,
		R: Serialize + 'static,
		F: Future<Output = R> + 'static,
	{
		self.on("runtime.message", move |event: MessageEvent<M>| {
			handler(event.message, event.sender)
		});
	}

	/// Runs when the extension is installed or updated: the place to create
	/// context menus and alarms, which persist.
	pub fn on_installed<F>(&self, handler: impl Fn(Installed) -> F + 'static)
	where
		F: Future<Output = ()> + 'static,
	{
		self.on("runtime.installed", handler);
	}

	/// Runs when the browser starts with the extension installed.
	pub fn on_startup<F>(&self, handler: impl Fn() -> F + 'static)
	where
		F: Future<Output = ()> + 'static,
	{
		self.on("runtime.startup", move |_: Value| handler());
	}

	/// Runs when an alarm goes off.
	pub fn on_alarm<F>(&self, handler: impl Fn(Alarm) -> F + 'static)
	where
		F: Future<Output = ()> + 'static,
	{
		self.on("alarms", handler);
	}

	/// Runs when one of the extension's context menu items is chosen.
	pub fn on_context_menu<F>(&self, handler: impl Fn(MenuClick) -> F + 'static)
	where
		F: Future<Output = ()> + 'static,
	{
		self.on("context_menus", move |event: ContextMenuEvent| {
			let mut click = event.info;
			click.tab = click.tab.or(event.tab);
			handler(click)
		});
	}

	/// Runs when one of `[extension.commands]` is pressed.
	pub fn on_command<F>(&self, handler: impl Fn(Command) -> F + 'static)
	where
		F: Future<Output = ()> + 'static,
	{
		self.on("commands", handler);
	}

	/// Runs when a tab changes, such as finishing loading.
	pub fn on_tab_updated<F>(&self, handler: impl Fn(TabUpdate) -> F + 'static)
	where
		F: Future<Output = ()> + 'static,
	{
		self.on("tabs.updated", handler);
	}
}

/// Runs the app's `background` function, once, when the wasm is ready.
pub fn start(background: fn(&Background)) {
	background(&Background { _private: () });
}

/// Runs the handler for one event and resolves to its reply. An event
/// nothing handles resolves to `undefined`.
#[cfg(target_arch = "wasm32")]
pub fn dispatch(kind: &str, detail: wasm_bindgen::JsValue) -> js_sys::Promise {
	use wasm_bindgen::JsValue;

	let handler = HANDLERS.with_borrow(|handlers| handlers.get(kind).cloned());
	wasm_bindgen_futures::future_to_promise(async move {
		let Some(handler) = handler else {
			return Ok(JsValue::UNDEFINED);
		};
		let detail: Value =
			crate::js::from_js(&detail).map_err(|error| JsValue::from_str(&error.to_string()))?;
		let reply = handler(detail)
			.await
			.map_err(|error| JsValue::from_str(&error))?;
		crate::js::to_js(&reply).map_err(|error| JsValue::from_str(&error.to_string()))
	})
}

/// Runs the handler for `kind` with `detail`, as the worker would: for tests
/// of an app's `background` function on any host.
///
/// # Errors
///
/// When nothing handles `kind`, or the details or reply don't convert.
pub async fn simulate(kind: &str, detail: Value) -> Result<Value, String> {
	let handler = HANDLERS
		.with_borrow(|handlers| handlers.get(kind).cloned())
		.ok_or_else(|| format!("nothing handles `{kind}`"))?;
	handler(detail).await
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	fn worker(background: &Background) {
		background.on_message(|message: String, sender: Sender| {
			async move { format!("{message} from {}", sender.url.unwrap_or_default()) }
		});
		background.on_command(|command| {
			async move {
				assert_eq!(command.command, "clip");
			}
		});
	}

	#[test]
	fn handlers_answer_events() {
		start(worker);
		let reply = futures::executor::block_on(simulate(
			"runtime.message",
			json!({ "message": "hello", "sender": { "url": "https://example.org" } }),
		));
		assert_eq!(reply, Ok(json!("hello from https://example.org")));
		let reply = futures::executor::block_on(simulate("commands", json!({ "command": "clip" })));
		assert_eq!(reply, Ok(json!(null)));
		assert!(futures::executor::block_on(simulate("alarms", json!({}))).is_err());
	}
}
