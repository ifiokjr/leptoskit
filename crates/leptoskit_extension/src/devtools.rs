//! The browser's developer tools: a panel of the extension's own, beside
//! Elements and Console, and the page the developer tools inspect.
//!
//! `[extension.devtools_page]` names two routes. The developer tools open the
//! first out of sight each time they open for a tab, and it adds the panel
//! with [`create_panel`]. The panel shows the second. Both can read the
//! inspected page with [`inspected_tab`] and [`eval`], and follow it to its
//! next page with [`on_navigated`]:
//!
//! ```ignore
//! /// Opened out of sight: it adds the panel and shows nothing.
//! fn register() -> Element {
//! 	wasm_bindgen_futures::spawn_local(async {
//! 		let _ = devtools::create_panel("Inspector", "assets/icon-32.png").await;
//! 	});
//! 	View()
//! }
//! ```
//!
//! Chromium has these APIs in `chrome.devtools`, and Firefox in
//! `browser.devtools`. Safari has none, so the build leaves the panel out of
//! its package, and every call here is unavailable there.
//!
//! # leptoskit's `DevTools` hub
//!
//! A page `leptoskit dev` builds reports to the `DevTools` hub on this
//! machine, and names its session in `window.__LEPTOSKIT_DEVTOOLS_SESSION__`.
//! [`watch_session`] follows the inspected page's session, and [`HubFrame`]
//! shows the hub's interface in the panel and tells it which session to show:
//!
//! 1. The framed interface posts `{type: "leptoskit-devtools-ready"}` to the
//!    panel when it loads.
//! 2. The panel answers with `{type: "leptoskit-devtools-session", session:
//!    "<id>"}`, addressed to the hub's origin, and sends it again whenever the
//!    inspected page's session changes.
//!
//! The interface comes from the hub, so it always matches the `leptoskit` that
//! runs it; the panel only frames it.

use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::json;

use crate::Error;
use crate::Result;
use crate::js;

/// The page the panel shows: the build generates the
/// `[extension.devtools_page]` panel route as this file.
pub const PANEL_PAGE: &str = "devtools_panel.html";

/// The port `leptoskit dev` runs its `DevTools` hub on unless
/// `--devtools-port` says otherwise.
pub const HUB_PORT: u16 = 7340;

/// The global a page `leptoskit dev` builds names its `DevTools` session in.
pub const SESSION_GLOBAL: &str = "__LEPTOSKIT_DEVTOOLS_SESSION__";

/// What the hub's framed interface posts to the panel when it loads.
pub const READY: &str = "leptoskit-devtools-ready";

/// What the panel answers with, naming the inspected page's session.
pub const SESSION: &str = "leptoskit-devtools-session";

/// The longest wait between two looks for the inspected page's session.
const SLOWEST_MS: u32 = 2_000;

/// Reads the session as a string, or `null` on any other page.
const SESSION_EXPRESSION: &str = "typeof window.__LEPTOSKIT_DEVTOOLS_SESSION__ === \"string\" ? \
                                  window.__LEPTOSKIT_DEVTOOLS_SESSION__ : null";

/// Whether this page runs inside the browser's developer tools.
#[must_use]
pub fn available() -> bool {
	js::has("devtools.panels.create")
}

/// Adds the extension's panel to the developer tools, titled `title`, with
/// the icon at `icon`, a file in the package such as `assets/icon-32.png`.
/// The panel shows the `[extension.devtools_page]` panel route. Call it from
/// that table's `route`, which the developer tools open for each tab.
///
/// # Errors
///
/// [`Error::Unavailable`] outside the developer tools, and in Safari.
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(clippy::unused_async, reason = "the same signature as in a browser")
)]
pub async fn create_panel(title: &str, icon: &str) -> Result<()> {
	#[cfg(target_arch = "wasm32")]
	{
		// Chromium answers through a callback and Firefox with a promise of
		// the panel. Neither answer is needed, so neither is converted.
		return js::call_raw(
			"devtools.panels.create",
			&[title.into(), icon.into(), PANEL_PAGE.into()],
		)
		.await
		.map(drop);
	}
	#[allow(unreachable_code)]
	{
		let _ = (title, icon);
		Err(Error::Unavailable("devtools.panels.create".into()))
	}
}

/// The ID of the tab the developer tools inspect.
///
/// # Errors
///
/// [`Error::Unavailable`] outside the developer tools.
pub fn inspected_tab() -> Result<i64> {
	js::property("devtools.inspectedWindow.tabId")
}

/// Evaluates `expression` in the inspected page and reads its value as a
/// `T`. The value has to be JSON: a string, a number, a list, or a plain
/// object.
///
/// # Errors
///
/// [`Error::Browser`] when the expression throws, or the page can't run it,
/// such as a browser's own page; [`Error::Unavailable`] outside the
/// developer tools.
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(clippy::unused_async, reason = "the same signature as in a browser")
)]
pub async fn eval<T: DeserializeOwned>(expression: &str) -> Result<T> {
	#[cfg(target_arch = "wasm32")]
	{
		return imp::eval(expression).await;
	}
	#[allow(unreachable_code)]
	{
		let _ = expression;
		Err(Error::Unavailable("devtools.inspectedWindow.eval".into()))
	}
}

/// Calls `navigated` with the inspected page's new address each time it
/// loads another page or reloads, until the listener is dropped.
///
/// # Errors
///
/// [`Error::Unavailable`] outside the developer tools.
pub fn on_navigated(navigated: impl Fn(String) + 'static) -> Result<crate::Listener> {
	#[cfg(target_arch = "wasm32")]
	{
		return js::listen("devtools.network.onNavigated", move |url, _, _| {
			navigated(url.as_string().unwrap_or_default());
			wasm_bindgen::JsValue::UNDEFINED
		});
	}
	#[allow(unreachable_code)]
	{
		let _ = navigated;
		Err(Error::Unavailable("devtools.network.onNavigated".into()))
	}
}

/// The session the inspected page reports to `leptoskit dev`'s `DevTools`
/// hub, or `None` when it reports none, as on a page built without
/// `leptoskit dev`.
///
/// # Errors
///
/// As [`eval`].
pub async fn inspected_session() -> Result<Option<String>> {
	eval(SESSION_EXPRESSION).await
}

/// The hub's origin on `port`. The hub listens only on loopback.
#[must_use]
pub fn hub_origin(port: u16) -> String {
	format!("http://127.0.0.1:{port}")
}

/// The hub's interface on `port`, laid out for a panel.
#[must_use]
pub fn hub_url(port: u16) -> String {
	format!("{}/?embed=panel", hub_origin(port))
}

/// A port typed into a field, such as ` 7341 `: `None` unless it's a whole
/// number from 1 to 65535.
#[must_use]
pub fn parse_port(text: &str) -> Option<u16> {
	text.trim().parse().ok().filter(|port| *port != 0)
}

/// The message that names `session` to the hub's framed interface.
#[must_use]
pub fn session_message(session: &str) -> Value {
	json!({ "type": SESSION, "session": session })
}

/// Whether `message`, from the hub's framed interface, says it's ready for a
/// session.
#[must_use]
pub fn is_ready(message: &Value) -> bool {
	message["type"] == READY
}

/// How long to wait before looking for the inspected page's session again,
/// after `attempt` looks: quickly at first, while a page that just loaded
/// opens its session with the hub, then every two seconds, which also
/// notices a reload the browser didn't report.
#[must_use]
pub fn retry_delay_ms(attempt: u32) -> u32 {
	(250 << attempt.min(3)).min(SLOWEST_MS)
}

/// Follows the inspected page's session until it's dropped: see
/// [`watch_session`].
pub struct SessionWatch {
	#[cfg(target_arch = "wasm32")]
	inner: imp::Watch,
}

/// Calls `changed` with the session the inspected page reports to the hub,
/// and again each time it changes, as when the page reloads, until the
/// returned watch is dropped. It looks right away, and keeps looking while
/// the page hasn't opened a session yet.
///
/// # Errors
///
/// [`Error::Unavailable`] outside the developer tools.
pub fn watch_session(changed: impl FnMut(String) + 'static) -> Result<SessionWatch> {
	#[cfg(target_arch = "wasm32")]
	{
		return imp::Watch::start(changed).map(|inner| SessionWatch { inner });
	}
	#[allow(unreachable_code)]
	{
		let _ = changed;
		Err(Error::Unavailable("devtools.inspectedWindow".into()))
	}
}

impl SessionWatch {
	/// The session the inspected page reported last, if any.
	#[must_use]
	pub fn session(&self) -> Option<String> {
		#[cfg(target_arch = "wasm32")]
		{
			return self.inner.session();
		}
		#[allow(unreachable_code)]
		{
			None
		}
	}
}

/// The hub's interface in a frame that fills the panel below its own
/// controls, and is told which session the inspected page reports. Dropping
/// it removes the frame.
pub struct HubFrame {
	#[cfg(target_arch = "wasm32")]
	inner: std::rc::Rc<imp::Frame>,
}

impl HubFrame {
	/// Adds a hidden frame to the page, filling it below `top`, a CSS length
	/// such as `36px`, which leaves room for the panel's own controls.
	///
	/// # Errors
	///
	/// [`Error::Unavailable`] outside a browser page.
	pub fn new(top: &str) -> Result<Self> {
		#[cfg(target_arch = "wasm32")]
		{
			return imp::Frame::new(top).map(|inner| Self { inner });
		}
		#[allow(unreachable_code)]
		{
			let _ = top;
			Err(Error::Unavailable("document".into()))
		}
	}

	/// Shows the hub on `port`, loading its interface unless the frame
	/// already shows it.
	pub fn show(&self, port: u16) {
		#[cfg(target_arch = "wasm32")]
		self.inner.show(port);
		#[cfg(not(target_arch = "wasm32"))]
		let _ = port;
	}

	/// Hides the frame and unloads the interface, so it stops following a
	/// hub that has gone.
	pub fn hide(&self) {
		#[cfg(target_arch = "wasm32")]
		self.inner.hide();
	}

	/// Names the inspected page's session to the interface, now and each
	/// time it loads again.
	pub fn name_session(&self, session: &str) {
		#[cfg(target_arch = "wasm32")]
		self.inner.name_session(session);
		#[cfg(not(target_arch = "wasm32"))]
		let _ = session;
	}
}

#[cfg(target_arch = "wasm32")]
mod imp {
	use std::cell::Cell;
	use std::cell::RefCell;
	use std::rc::Rc;
	use std::rc::Weak;

	use js_sys::Promise;
	use js_sys::Reflect;
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;
	use wasm_bindgen::closure::Closure;
	use wasm_bindgen_futures::JsFuture;
	use wasm_bindgen_futures::spawn_local;

	use super::*;

	pub(super) async fn eval<T: DeserializeOwned>(expression: &str) -> Result<T> {
		let options = js_sys::Object::new();
		let (value, failure) = js::call_back(
			"devtools.inspectedWindow.eval",
			&[JsValue::from_str(expression), options.into()],
		)
		.await?;
		// Both browsers call back with the value, and, when it failed, why:
		// the exception it threw, or the developer tools' own error.
		if failure.is_object() {
			let field = |name: &str| {
				Reflect::get(&failure, &JsValue::from_str(name))
					.ok()
					.and_then(|value| value.as_string())
			};
			return Err(Error::Browser(
				field("value")
					.or_else(|| field("description"))
					.unwrap_or_else(|| "the inspected page couldn't run the expression".to_owned()),
			));
		}
		js::from_js(&value)
	}

	/// Waits `ms` milliseconds.
	async fn sleep(ms: u32) {
		let promise = Promise::new(&mut |resolve, _| {
			let waited = web_sys::window().map(|window| {
				window.set_timeout_with_callback_and_timeout_and_arguments_0(
					&resolve,
					i32::try_from(ms).unwrap_or(i32::MAX),
				)
			});
			if !matches!(waited, Some(Ok(_))) {
				let _ = resolve.call0(&JsValue::NULL);
			}
		});
		let _ = JsFuture::from(promise).await;
	}

	struct WatchState {
		stopped: Cell<bool>,
		/// Each navigation starts a new round of looks, which ends the last.
		round: Cell<u32>,
		session: RefCell<Option<String>>,
		changed: RefCell<Box<dyn FnMut(String)>>,
	}

	pub(super) struct Watch {
		state: Rc<WatchState>,
		// Kept for as long as the watch: it starts a new round on navigation.
		_navigated: Option<crate::Listener>,
	}

	impl Watch {
		pub(super) fn start(changed: impl FnMut(String) + 'static) -> Result<Self> {
			if !js::has("devtools.inspectedWindow.eval") {
				return Err(Error::Unavailable("devtools.inspectedWindow".into()));
			}
			let state = Rc::new(WatchState {
				stopped: Cell::new(false),
				round: Cell::new(0),
				session: RefCell::new(None),
				changed: RefCell::new(Box::new(changed)),
			});
			spawn_local(look(Rc::clone(&state), 0));
			let weak = Rc::downgrade(&state);
			// Without navigation events, the slow looks still notice reloads.
			let navigated = on_navigated(move |_| {
				if let Some(state) = weak.upgrade() {
					let round = state.round.get().wrapping_add(1);
					state.round.set(round);
					spawn_local(look(state, round));
				}
			})
			.ok();
			Ok(Self {
				state,
				_navigated: navigated,
			})
		}

		pub(super) fn session(&self) -> Option<String> {
			self.state.session.borrow().clone()
		}
	}

	impl Drop for Watch {
		fn drop(&mut self) {
			self.state.stopped.set(true);
		}
	}

	/// Looks for the inspected page's session until the watch stops or a
	/// navigation starts another round.
	async fn look(state: Rc<WatchState>, round: u32) {
		let current = |state: &WatchState| !state.stopped.get() && state.round.get() == round;
		let mut attempt = 0;
		while current(&state) {
			let found = inspected_session().await;
			if !current(&state) {
				return;
			}
			if let Ok(Some(session)) = found {
				let changed = state.session.borrow().as_deref() != Some(session.as_str());
				if changed {
					state.session.replace(Some(session.clone()));
					(state.changed.borrow_mut())(session);
				}
				// Found: from now on, look only now and then.
				attempt = u32::MAX;
			}
			sleep(retry_delay_ms(attempt)).await;
			attempt = attempt.saturating_add(1);
		}
	}

	pub(super) struct Frame {
		iframe: web_sys::HtmlIFrameElement,
		/// The hub the frame shows; `None` while it's hidden.
		port: Cell<Option<u16>>,
		session: RefCell<Option<String>>,
		// Kept alive for as long as the window can call it.
		on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
	}

	impl Frame {
		pub(super) fn new(top: &str) -> Result<Rc<Self>> {
			let unavailable = || Error::Unavailable("document".into());
			let window = web_sys::window().ok_or_else(unavailable)?;
			let document = window.document().ok_or_else(unavailable)?;
			let body = document.body().ok_or_else(unavailable)?;
			let iframe: web_sys::HtmlIFrameElement = document
				.create_element("iframe")
				.map_err(|error| Error::Browser(js::message(&error)))?
				.unchecked_into();
			iframe.set_title("leptoskit DevTools");
			let style = iframe.style();
			for (property, value) in [
				("position", "fixed"),
				("left", "0"),
				("top", top),
				("width", "100%"),
				("height", &format!("calc(100% - {top})")),
				("border", "0"),
				("display", "none"),
			] {
				let _ = style.set_property(property, value);
			}

			let frame = Rc::new_cyclic(|weak: &Weak<Self>| {
				let weak = weak.clone();
				let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event| {
					if let Some(frame) = weak.upgrade() {
						frame.received(&event);
					}
				});
				Self {
					iframe,
					port: Cell::new(None),
					session: RefCell::new(None),
					on_message,
				}
			});
			window
				.add_event_listener_with_callback(
					"message",
					frame.on_message.as_ref().unchecked_ref(),
				)
				.map_err(|error| Error::Browser(js::message(&error)))?;
			body.append_child(&frame.iframe)
				.map_err(|error| Error::Browser(js::message(&error)))?;
			Ok(frame)
		}

		/// Answers the interface's ready message, and nothing else: only the
		/// framed window, on the hub's origin, is listened to.
		fn received(&self, event: &web_sys::MessageEvent) {
			let Some(port) = self.port.get() else {
				return;
			};
			let from_frame = match (event.source(), self.iframe.content_window()) {
				(Some(source), Some(window)) => js_sys::Object::is(&source, &window),
				_ => false,
			};
			if !from_frame || event.origin() != hub_origin(port) {
				return;
			}
			if js::from_js::<Value>(&event.data()).is_ok_and(|message| is_ready(&message)) {
				self.post();
			}
		}

		/// Posts the session to the interface, addressed to the hub's origin,
		/// so that no other page the frame might show ever receives it.
		fn post(&self) {
			let (Some(port), Some(session)) = (self.port.get(), self.session.borrow().clone())
			else {
				return;
			};
			if let (Some(window), Ok(message)) = (
				self.iframe.content_window(),
				js::to_js(&session_message(&session)),
			) {
				let _ = window.post_message(&message, &hub_origin(port));
			}
		}

		pub(super) fn show(&self, port: u16) {
			if self.port.get() != Some(port) {
				self.port.set(Some(port));
				self.iframe.set_src(&hub_url(port));
			}
			let _ = self.iframe.style().set_property("display", "block");
		}

		pub(super) fn hide(&self) {
			let _ = self.iframe.style().set_property("display", "none");
			if self.port.take().is_some() {
				self.iframe.set_src("about:blank");
			}
		}

		pub(super) fn name_session(&self, session: &str) {
			self.session.replace(Some(session.to_owned()));
			self.post();
		}
	}

	impl Drop for Frame {
		fn drop(&mut self) {
			if let Some(window) = web_sys::window() {
				let _ = window.remove_event_listener_with_callback(
					"message",
					self.on_message.as_ref().unchecked_ref(),
				);
			}
			self.iframe.remove();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_hub_is_framed_on_loopback_laid_out_for_a_panel() {
		assert_eq!(hub_origin(HUB_PORT), "http://127.0.0.1:7340");
		assert_eq!(hub_url(7341), "http://127.0.0.1:7341/?embed=panel");
	}

	#[test]
	fn typed_ports_are_whole_numbers_from_1_to_65535() {
		assert_eq!(parse_port(" 7341 "), Some(7341));
		assert_eq!(parse_port("65535"), Some(65_535));
		for text in ["", "0", "65536", "-1", "7340.5", "port", "73 40"] {
			assert_eq!(parse_port(text), None, "{text:?}");
		}
	}

	#[test]
	fn messages_follow_the_hub_page_s_contract() {
		assert_eq!(
			session_message("4f1c"),
			json!({ "type": "leptoskit-devtools-session", "session": "4f1c" })
		);
		assert!(is_ready(&json!({ "type": "leptoskit-devtools-ready" })));
		assert!(!is_ready(&json!({ "type": "leptoskit-devtools-session" })));
		assert!(!is_ready(&json!("leptoskit-devtools-ready")));
		assert!(!is_ready(&Value::Null));
	}

	#[test]
	fn the_session_expression_reads_only_the_session_global() {
		assert_eq!(
			SESSION_EXPRESSION.matches(SESSION_GLOBAL).count(),
			2,
			"{SESSION_EXPRESSION}"
		);
		assert!(SESSION_EXPRESSION.ends_with(": null"));
	}

	#[test]
	fn looks_for_a_session_quickly_then_every_two_seconds() {
		let delays: Vec<u32> = (0..6).map(retry_delay_ms).collect();
		assert_eq!(delays, [250, 500, 1000, 2000, 2000, 2000]);
		assert_eq!(retry_delay_ms(31), 2000);
		assert_eq!(retry_delay_ms(u32::MAX), 2000);
	}

	#[test]
	fn outside_a_browser_everything_is_unavailable() {
		assert!(!available());
		assert!(matches!(inspected_tab(), Err(Error::Unavailable(_))));
		assert!(matches!(
			watch_session(|_| {}).map(drop),
			Err(Error::Unavailable(_))
		));
		assert!(matches!(
			HubFrame::new("36px").map(drop),
			Err(Error::Unavailable(_))
		));
		assert!(matches!(
			on_navigated(|_| {}).map(drop),
			Err(Error::Unavailable(_))
		));
		let panel = futures::executor::block_on(create_panel("leptoskit", ""));
		assert!(matches!(panel, Err(Error::Unavailable(_))));
		let session = futures::executor::block_on(inspected_session());
		assert!(matches!(session, Err(Error::Unavailable(_))));
	}
}
