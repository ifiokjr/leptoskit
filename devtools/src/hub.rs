//! Talking to the hub that serves the interface, on the page's own origin.
//!
//! - `GET /api/hub`, `GET /api/sessions`, and `GET /api/sessions/{id}` load
//!   every session once.
//! - `GET /api/stream` streams every change after that.
//! - `POST /api/sessions/{id}/commands` sends a command, as JSON.
//!
//! # The `DevTools` panel
//!
//! With `?embed=panel`, the page is framed by the browser `DevTools` panel,
//! which knows the inspected tab's session:
//!
//! 1. On load the page posts `{type: "leptoskit-devtools-ready"}` to
//!    `window.parent`. The message carries no data, so its target origin is
//!    `*`.
//! 2. The panel answers with `{type: "leptoskit-devtools-session", session:
//!    "<id>"}`. The page accepts it only when `event.source` is
//!    `window.parent`, and shows that session, waiting for it if the hub has
//!    not reported it yet. The panel may send it again whenever the inspected
//!    tab changes session.
//! 3. When no session is named within [`PANEL_WAIT_MS`], the page shows the
//!    newest web session, and still switches to the named one if it appears.
//!
//! On the host, for tests, commands and editor links are recorded instead.

pub use imp::open;
pub use imp::send;
#[cfg(target_arch = "wasm32")]
pub use imp::start;
#[cfg(not(target_arch = "wasm32"))]
pub use imp::take_opened;
#[cfg(not(target_arch = "wasm32"))]
pub use imp::take_sent;

/// How long the panel has to name a session before the page picks one.
pub const PANEL_WAIT_MS: i32 = 1500;

/// The message the page posts to the panel when it is ready.
pub const READY: &str = "leptoskit-devtools-ready";

/// The message the panel answers with, naming the inspected tab's session.
pub const SESSION: &str = "leptoskit-devtools-session";

#[cfg(target_arch = "wasm32")]
mod imp {
	use std::cell::Cell;
	use std::cell::RefCell;

	use js_sys::Reflect;
	use leptoskit_devtools_protocol::Change;
	use leptoskit_devtools_protocol::Command;
	use leptoskit_devtools_protocol::SessionId;
	use leptoskit_devtools_protocol::SessionInfo;
	use leptoskit_devtools_protocol::SessionState;
	use serde::Serialize;
	use serde::de::DeserializeOwned;
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;
	use wasm_bindgen::closure::Closure;
	use wasm_bindgen_futures::JsFuture;
	use wasm_bindgen_futures::spawn_local;

	use super::PANEL_WAIT_MS;
	use super::READY;
	use super::SESSION;
	use crate::model::Connection;
	use crate::model::HubInfo;
	use crate::store;

	thread_local! {
		static STREAM: RefCell<Option<web_sys::EventSource>> = const { RefCell::new(None) };
		static LOADING: Cell<bool> = const { Cell::new(false) };
		static RELOAD: Cell<bool> = const { Cell::new(false) };
	}

	/// Reads the page's address, loads the sessions, and follows the stream.
	pub fn start() {
		let Some(window) = web_sys::window() else {
			return;
		};
		let search = window.location().search().unwrap_or_default();
		let params = web_sys::UrlSearchParams::new_with_str(&search).ok();
		let embed = params
			.as_ref()
			.and_then(|params| params.get("embed"))
			.is_some_and(|embed| embed == "panel");
		let session = params.and_then(|params| params.get("session"));
		store::update(|model| {
			model.embed = embed;
			model.wanted = session.map(SessionId::new);
		});

		follow();
		spawn_local(load());

		if embed {
			panel(&window);
		}
	}

	/// Sends a command to a session.
	pub fn send(session: &SessionId, command: Command) {
		let url = format!(
			"/api/sessions/{}/commands",
			js_sys::encode_uri_component(session.as_str())
		);
		spawn_local(async move {
			if let Err(error) = post(&url, &command).await {
				store::problem(format!("The command didn't reach the app: {error}"));
			}
		});
	}

	/// Opens an editor link.
	pub fn open(url: &str) {
		if let Some(window) = web_sys::window()
			&& window.location().set_href(url).is_err()
		{
			store::problem("The browser didn't open the editor link.");
		}
	}

	/// Loads every session, once at a time: a load asked for while one runs
	/// runs again after it.
	async fn load() {
		if LOADING.replace(true) {
			RELOAD.set(true);
			return;
		}

		loop {
			RELOAD.set(false);
			match sessions().await {
				Ok((hub, states)) => {
					store::update(|model| {
						model.hub = Some(hub);
						model.load(states);
					});
				}
				Err(error) => store::problem(format!("Couldn't reach the hub: {error}")),
			}

			if !RELOAD.get() {
				break;
			}
		}

		LOADING.set(false);
	}

	async fn sessions() -> Result<(HubInfo, Vec<SessionState>), String> {
		let hub: HubInfo = get("/api/hub").await?;
		let infos: Vec<SessionInfo> = get("/api/sessions").await?;
		let mut states = Vec::with_capacity(infos.len());

		for info in infos {
			let url = format!(
				"/api/sessions/{}",
				js_sys::encode_uri_component(info.id.as_str())
			);
			// A session that ended since the list was read is skipped; the
			// stream reports it closed.
			if let Ok(state) = get::<SessionState>(&url).await {
				states.push(state);
			}
		}

		Ok((hub, states))
	}

	/// Applies every change the hub streams. The browser reconnects a dropped
	/// stream itself; after that, everything is loaded again, because changes
	/// were missed.
	fn follow() {
		let Ok(source) = web_sys::EventSource::new("/api/stream") else {
			store::problem("This browser can't follow the hub's changes.");
			return;
		};

		let opened = Closure::<dyn FnMut()>::new(|| {
			let lost = store::peek(|model| model.connection == Connection::Lost);
			store::update(|model| model.connection = Connection::Live);
			if lost {
				spawn_local(load());
			}
		});
		source.set_onopen(Some(opened.as_ref().unchecked_ref()));
		opened.forget();

		let failed = Closure::<dyn FnMut()>::new(|| {
			store::update(|model| model.connection = Connection::Lost);
		});
		source.set_onerror(Some(failed.as_ref().unchecked_ref()));
		failed.forget();

		let message =
			Closure::<dyn FnMut(web_sys::MessageEvent)>::new(|event: web_sys::MessageEvent| {
				let Some(data) = event.data().as_string() else {
					return;
				};
				// A change this interface doesn't know comes from a newer hub;
				// the rest of the stream still applies.
				let Ok(change) = serde_json::from_str::<Change>(&data) else {
					return;
				};
				let now = js_sys::Date::now() as u64;
				if store::update(|model| model.apply(change, now)) {
					spawn_local(load());
				}
			});
		source.set_onmessage(Some(message.as_ref().unchecked_ref()));
		message.forget();

		STREAM.with_borrow_mut(|stream| *stream = Some(source));
	}

	/// The panel contract: see the module documentation.
	fn panel(window: &web_sys::Window) {
		let named =
			Closure::<dyn FnMut(web_sys::MessageEvent)>::new(|event: web_sys::MessageEvent| {
				let parent = web_sys::window().and_then(|window| window.parent().ok().flatten());
				let from_parent = match (event.source(), parent) {
					(Some(source), Some(parent)) => js_sys::Object::is(&source, &parent),
					_ => false,
				};

				if !from_parent {
					return;
				}

				let data = event.data();
				let field = |name: &str| {
					Reflect::get(&data, &JsValue::from_str(name))
						.ok()
						.and_then(|value| value.as_string())
				};

				if field("type").as_deref() == Some(SESSION)
					&& let Some(session) = field("session")
				{
					store::show(&SessionId::new(session));
				}
			});
		let _ = window.add_event_listener_with_callback("message", named.as_ref().unchecked_ref());
		named.forget();

		if let Ok(Some(parent)) = window.parent() {
			let ready = js_sys::Object::new();
			let _ = Reflect::set(&ready, &"type".into(), &READY.into());
			let _ = parent.post_message(&ready, "*");
		}

		let waited = Closure::once(|| store::update(|model| model.stop_waiting()));
		let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
			waited.as_ref().unchecked_ref(),
			PANEL_WAIT_MS,
		);
		waited.forget();
	}

	async fn get<T: DeserializeOwned>(url: &str) -> Result<T, String> {
		let window = web_sys::window().ok_or("there is no window")?;
		let text = finish(JsFuture::from(window.fetch_with_str(url)).await).await?;
		serde_json::from_str(&text).map_err(|error| error.to_string())
	}

	async fn post<T: Serialize>(url: &str, body: &T) -> Result<(), String> {
		let window = web_sys::window().ok_or("there is no window")?;
		let headers = web_sys::Headers::new().map_err(describe)?;
		headers
			.set("Content-Type", "application/json")
			.map_err(describe)?;
		let init = web_sys::RequestInit::new();
		init.set_method("POST");
		init.set_headers(&headers);
		let body = serde_json::to_string(body).map_err(|error| error.to_string())?;
		init.set_body(&JsValue::from_str(&body));
		let request = web_sys::Request::new_with_str_and_init(url, &init).map_err(describe)?;
		finish(JsFuture::from(window.fetch_with_request(&request)).await)
			.await
			.map(drop)
	}

	/// The body of a finished fetch, or the hub's error.
	async fn finish(response: Result<JsValue, JsValue>) -> Result<String, String> {
		let response: web_sys::Response =
			response.map_err(describe)?.dyn_into().map_err(describe)?;
		let text = JsFuture::from(response.text().map_err(describe)?)
			.await
			.map_err(describe)?
			.as_string()
			.unwrap_or_default();

		if response.ok() {
			return Ok(text);
		}

		// The hub answers API errors as `{"error": "..."}`.
		let message = serde_json::from_str::<serde_json::Value>(&text)
			.ok()
			.and_then(|json| json["error"].as_str().map(str::to_owned))
			.unwrap_or(text);
		Err(format!("{} ({})", message.trim(), response.status()))
	}

	fn describe(error: JsValue) -> String {
		error
			.dyn_ref::<js_sys::Error>()
			.map(|error| String::from(error.message()))
			.or_else(|| error.as_string())
			.unwrap_or_else(|| "the browser refused the request".to_owned())
	}
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
	use std::cell::RefCell;

	use leptoskit_devtools_protocol::Command;
	use leptoskit_devtools_protocol::SessionId;

	thread_local! {
		static SENT: RefCell<Vec<(SessionId, Command)>> = const { RefCell::new(Vec::new()) };
		static OPENED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
	}

	/// Records a command.
	pub fn send(session: &SessionId, command: Command) {
		SENT.with_borrow_mut(|sent| sent.push((session.clone(), command)));
	}

	/// Records an editor link.
	pub fn open(url: &str) {
		OPENED.with_borrow_mut(|opened| opened.push(url.to_owned()));
	}

	/// The commands sent since the last call.
	#[must_use]
	pub fn take_sent() -> Vec<(SessionId, Command)> {
		SENT.with_borrow_mut(std::mem::take)
	}

	/// The editor links opened since the last call.
	#[must_use]
	pub fn take_opened() -> Vec<String> {
		OPENED.with_borrow_mut(std::mem::take)
	}
}
