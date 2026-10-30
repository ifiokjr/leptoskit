//! The page a content script's frame sits on.
//!
//! A content script's interface runs in a frame of the extension's own page,
//! because most large sites' content security policies stop wasm from
//! compiling in the page itself. The frame reaches the page through the
//! small script the build generates, over a private channel: it reads the
//! selection and the page's details, places and sizes the frame, and
//! highlights quotes. The page's own scripts can't see the channel or reach
//! into the frame.

use serde::Deserialize;
use serde::Serialize;

use crate::Result;

/// The page's details, from its address, title, and metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageInfo {
	/// The page's address, without its `#fragment`.
	pub url: String,
	/// The page's title, from `og:title` or the document's title.
	pub title: String,
	/// The author, from `<meta name="author">` or `article:author`.
	#[serde(default)]
	pub author: Option<String>,
	/// The site's name, from `og:site_name`.
	#[serde(default)]
	pub site: Option<String>,
	/// When the page was published, as `YYYY-MM-DD`, from
	/// `article:published_time` or a `<time datetime>`.
	#[serde(default)]
	pub published: Option<String>,
	/// The page's description.
	#[serde(default)]
	pub description: Option<String>,
}

/// A rectangle in the page's viewport, in CSS pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
	pub x: f64,
	pub y: f64,
	pub width: f64,
	pub height: f64,
}

/// Text the user selected, with a little of the text around it, which is
/// what finds it again (a W3C Web Annotation text-quote selector).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Selection {
	/// The selected text.
	pub text: String,
	/// Up to 32 characters before it.
	pub before: String,
	/// Up to 32 characters after it.
	pub after: String,
	/// Where it is on screen.
	pub rect: Rect,
}

/// A quote to highlight on the page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
	/// Names the highlight in [`PageEvent::HighlightPressed`] and
	/// [`Page::scroll_to`].
	pub id: String,
	/// The quoted text.
	pub exact: String,
	/// Text just before it, which picks the right occurrence.
	#[serde(default)]
	pub prefix: Option<String>,
	/// Text just after it.
	#[serde(default)]
	pub suffix: Option<String>,
}

/// Where the frame sits on the page.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "at", rename_all = "snake_case")]
pub enum Placement {
	/// Just below the selection, as a bubble.
	NearSelection {
		/// The frame's width in CSS pixels.
		width: f64,
		/// The frame's height.
		height: f64,
	},
	/// In the top-right corner, as a panel.
	Corner {
		/// The frame's width in CSS pixels.
		width: f64,
		/// The frame's height.
		height: f64,
	},
	/// Out of sight, while the frame waits for something to show.
	Hidden,
}

/// Something that happened on the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum PageEvent {
	/// The selection changed; `None` when nothing is selected.
	Selection(Option<Selection>),
	/// The user pressed a highlight, by its quote's ID.
	HighlightPressed(String),
}

#[cfg(target_arch = "wasm32")]
pub use imp::Page;
#[cfg(target_arch = "wasm32")]
pub use imp::connect;

#[cfg(target_arch = "wasm32")]
mod imp {
	use std::cell::Cell;
	use std::cell::RefCell;
	use std::collections::HashMap;
	use std::rc::Rc;

	use futures::channel::oneshot;
	use js_sys::Function;
	use js_sys::Reflect;
	use serde_json::Value;
	use serde_json::json;
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;
	use wasm_bindgen::closure::Closure;

	use super::*;
	use crate::Error;
	use crate::js;

	type Reply = std::result::Result<Value, String>;

	type EventHandler = Rc<dyn Fn(PageEvent)>;

	struct Inner {
		port: JsValue,
		next: Cell<u32>,
		pending: RefCell<HashMap<u32, oneshot::Sender<Reply>>>,
		events: RefCell<Option<EventHandler>>,
		// Kept alive for as long as the port can call it.
		on_message: Closure<dyn FnMut(JsValue)>,
	}

	/// The channel to the page the frame sits on. Clones share it.
	#[derive(Clone)]
	pub struct Page {
		inner: Rc<Inner>,
	}

	#[derive(Deserialize)]
	struct Frame {
		nonce: String,
	}

	/// Connects to the page this content script frame sits on.
	///
	/// # Errors
	///
	/// When this isn't a content script's frame.
	pub async fn connect() -> Result<Page> {
		let hash = web_location_hash().ok_or_else(|| Error::Unavailable("page".into()))?;
		let frame: Frame =
			serde_json::from_str(&decode(&hash)).map_err(|_| Error::Unavailable("page".into()))?;
		let tab: Value = js::call("tabs.getCurrent", &[]).await?;
		let tab = tab["id"]
			.as_i64()
			.ok_or_else(|| Error::Unavailable("tabs.getCurrent".into()))?;
		let port = js::call_raw(
			"tabs.connect",
			&[
				JsValue::from(tab as f64),
				js::to_js(
					&json!({ "name": format!("leptoskit-page:{}", frame.nonce), "frameId": 0 }),
				)?,
			],
		)
		.await?;

		let page = Rc::new_cyclic(|weak: &std::rc::Weak<Inner>| {
			let weak = weak.clone();
			let on_message = Closure::<dyn FnMut(JsValue)>::new(move |message: JsValue| {
				let Some(inner) = weak.upgrade() else { return };
				let Ok(message) = js::from_js::<Value>(&message) else {
					return;
				};
				if let Some(id) = message["id"].as_u64() {
					let sender = inner.pending.borrow_mut().remove(&(id as u32));
					if let Some(sender) = sender {
						let reply = match message.get("error").and_then(Value::as_str) {
							Some(error) => Err(error.to_owned()),
							None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
						};
						let _ = sender.send(reply);
					}
				} else if message.get("event").is_some()
					&& let Ok(event) = serde_json::from_value::<PageEvent>(message)
				{
					let handler = inner.events.borrow().clone();
					if let Some(handler) = handler {
						handler(event);
					}
				}
			});
			Inner {
				port: port.clone(),
				next: Cell::new(1),
				pending: RefCell::default(),
				events: RefCell::default(),
				on_message,
			}
		});
		let on_message = Reflect::get(&page.port, &"onMessage".into())
			.map_err(|error| Error::Browser(js::message(&error)))?;
		let add: Function = Reflect::get(&on_message, &"addListener".into())
			.ok()
			.and_then(|add| add.dyn_into().ok())
			.ok_or_else(|| Error::Unavailable("port.onMessage".into()))?;
		add.call1(&on_message, page.on_message.as_ref())
			.map_err(|error| Error::Browser(js::message(&error)))?;
		Ok(Page { inner: page })
	}

	fn web_location_hash() -> Option<String> {
		let location = Reflect::get(&js_sys::global(), &"location".into()).ok()?;
		let hash = Reflect::get(&location, &"hash".into()).ok()?.as_string()?;
		hash.strip_prefix('#').map(str::to_owned)
	}

	fn decode(text: &str) -> String {
		js_sys::decode_uri_component(text)
			.ok()
			.and_then(|text| text.as_string())
			.unwrap_or_default()
	}

	impl Page {
		async fn request<T: serde::de::DeserializeOwned>(
			&self,
			method: &str,
			params: Value,
		) -> Result<T> {
			let id = self.inner.next.get();
			self.inner.next.set(id.wrapping_add(1));
			let (sender, receiver) = oneshot::channel();
			self.inner.pending.borrow_mut().insert(id, sender);
			let message = js::to_js(&json!({ "id": id, "method": method, "params": params }))?;
			let post: Function = Reflect::get(&self.inner.port, &"postMessage".into())
				.ok()
				.and_then(|post| post.dyn_into().ok())
				.ok_or_else(|| Error::Unavailable("port.postMessage".into()))?;
			post.call1(&self.inner.port, &message)
				.map_err(|error| Error::Browser(js::message(&error)))?;
			let reply = receiver
				.await
				.map_err(|_| Error::Browser("the page closed".into()))?
				.map_err(Error::Browser)?;
			serde_json::from_value(reply).map_err(|error| Error::Json(error.to_string()))
		}

		/// The page's address, title, and metadata.
		///
		/// # Errors
		///
		/// When the page has gone.
		pub async fn info(&self) -> Result<PageInfo> {
			self.request("info", Value::Null).await
		}

		/// What the user has selected on the page, if anything.
		///
		/// # Errors
		///
		/// When the page has gone.
		pub async fn selection(&self) -> Result<Option<Selection>> {
			self.request("selection", Value::Null).await
		}

		/// Moves and sizes the frame.
		///
		/// # Errors
		///
		/// When the page has gone.
		pub async fn place(&self, placement: Placement) -> Result<()> {
			self.request(
				"place",
				serde_json::to_value(placement).unwrap_or(Value::Null),
			)
			.await
		}

		/// Highlights each quote where it appears on the page, replacing
		/// earlier highlights, and returns how many it found.
		///
		/// # Errors
		///
		/// When the page has gone.
		pub async fn highlight(&self, quotes: &[Quote]) -> Result<u32> {
			self.request(
				"highlight",
				serde_json::to_value(quotes).unwrap_or(Value::Null),
			)
			.await
		}

		/// Scrolls the highlight `id` into view. Returns whether it's on the
		/// page.
		///
		/// # Errors
		///
		/// When the page has gone.
		pub async fn scroll_to(&self, id: &str) -> Result<bool> {
			self.request("scroll_to", json!(id)).await
		}

		/// Removes the frame, and its highlights, from the page.
		///
		/// # Errors
		///
		/// When the page has gone.
		pub async fn close(&self) -> Result<()> {
			self.request("close", Value::Null).await
		}

		/// Calls `handler` with each [`PageEvent`], replacing any earlier
		/// handler.
		pub fn on_event(&self, handler: impl Fn(PageEvent) + 'static) {
			*self.inner.events.borrow_mut() = Some(Rc::new(handler));
		}
	}
}

/// Outside a browser there's no page to connect to.
///
/// # Errors
///
/// Always [`Error::Unavailable`](crate::Error::Unavailable).
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::unused_async, reason = "the same signature as in a browser")]
pub async fn connect() -> Result<std::convert::Infallible> {
	Err(crate::Error::Unavailable("page".into()))
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn events_and_placements_have_the_shim_s_shape() {
		let event: PageEvent =
			serde_json::from_value(json!({ "event": "highlight_pressed", "data": "clip-1" }))
				.unwrap();
		assert_eq!(event, PageEvent::HighlightPressed("clip-1".into()));
		let event: PageEvent =
			serde_json::from_value(json!({ "event": "selection", "data": null })).unwrap();
		assert_eq!(event, PageEvent::Selection(None));
		assert_eq!(
			serde_json::to_value(Placement::NearSelection {
				width: 320.0,
				height: 120.0
			})
			.unwrap(),
			json!({ "at": "near_selection", "width": 320.0, "height": 120.0 })
		);
	}
}
