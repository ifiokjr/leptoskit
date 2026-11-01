//! Keeping a router in step with the browser's session history.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use reactive_graph::signal::ArcRwSignal;
use reactive_graph::traits::Set;
use send_wrapper::SendWrapper;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use crate::Href;
use crate::NoMatch;
use crate::Routable;
use crate::Router;

type PopListener = Closure<dyn Fn(web_sys::PopStateEvent)>;

impl<R> Router<R>
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	/// The router for a browser page. It starts at the page's URL, adds a
	/// history entry for each navigation, and follows the back and forward
	/// buttons.
	///
	/// # Errors
	///
	/// Returns [`NoMatch`] when the page's URL matches no route, which only
	/// happens without a root `+not-found.rs`. Every URL decodes: see
	/// [`RouteUrl::parse`](crate::RouteUrl::parse).
	///
	/// # Panics
	///
	/// Panics outside a browser window.
	pub fn from_browser() -> Result<Self, NoMatch> {
		let window = web_sys::window().expect("a browser window");
		let initial = R::parse(&location(&window))?;
		let history = window.history().expect("session history");
		// Every entry this app writes holds its depth, so back knows whether
		// an earlier app entry exists, even after a reload.
		let depth = history
			.state()
			.ok()
			.and_then(|state| state.as_f64())
			.map_or(0, |depth| depth as usize);
		history
			.replace_state(&JsValue::from(depth as f64), "")
			.expect("replace the history entry");

		let current = ArcRwSignal::new(initial);
		let depth = Arc::new(AtomicUsize::new(depth));
		let listener: PopListener = {
			let current = current.clone();
			let depth = Arc::clone(&depth);
			Closure::new(move |event: web_sys::PopStateEvent| {
				depth.store(
					event.state().as_f64().map_or(0, |depth| depth as usize),
					Ordering::SeqCst,
				);
				let window = web_sys::window().expect("a browser window");
				// Every URL decodes, so only an app without a root
				// `+not-found.rs` can reach one no route matches; its screen
				// stays as it was.
				if let Ok(route) = R::parse(&location(&window)) {
					current.set(route);
				}
			})
		};
		window
			.add_event_listener_with_callback("popstate", listener.as_ref().unchecked_ref())
			.expect("listen for history changes");
		Ok(Self::browser(
			current,
			Browser {
				depth,
				listener: SendWrapper::new(listener),
			},
		))
	}
}

/// The page's path and query string.
fn location(window: &web_sys::Window) -> String {
	let location = window.location();
	format!(
		"{}{}",
		location.pathname().unwrap_or_default(),
		location.search().unwrap_or_default()
	)
}

fn history() -> web_sys::History {
	web_sys::window()
		.expect("a browser window")
		.history()
		.expect("session history")
}

/// A router's link to the session history: how deep it is, and the listener
/// that follows the back and forward buttons until the router is dropped.
pub(crate) struct Browser {
	depth: Arc<AtomicUsize>,
	listener: SendWrapper<PopListener>,
}

impl Browser {
	pub(crate) fn push(&self, href: &Href) {
		let depth = self.depth.fetch_add(1, Ordering::SeqCst) + 1;
		history()
			.push_state_with_url(&JsValue::from(depth as f64), "", Some(href.as_str()))
			.expect("add a history entry");
	}

	pub(crate) fn replace(&self, href: &Href) {
		let depth = self.depth.load(Ordering::SeqCst);
		history()
			.replace_state_with_url(&JsValue::from(depth as f64), "", Some(href.as_str()))
			.expect("replace the history entry");
	}

	pub(crate) fn can_go_back(&self) -> bool {
		self.depth.load(Ordering::SeqCst) > 0
	}

	pub(crate) fn back(&self) -> bool {
		if !self.can_go_back() {
			return false;
		}

		history().back().expect("go back");
		true
	}
}

impl Drop for Browser {
	fn drop(&mut self) {
		if let Some(window) = web_sys::window() {
			let _ = window.remove_event_listener_with_callback(
				"popstate",
				self.listener.as_ref().unchecked_ref(),
			);
		}
	}
}
