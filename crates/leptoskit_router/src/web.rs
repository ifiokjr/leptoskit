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
use crate::url::BasePath;

type PopListener = Closure<dyn Fn(web_sys::PopStateEvent)>;

impl<R> Router<R>
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	/// The router for a browser page. It starts at the page's URL, adds a
	/// history entry for each navigation, and follows the back and forward
	/// buttons.
	///
	/// An app served under a sub-path names it with a `<base href>` element,
	/// such as `<base href="/app/">`: routes then match the part of the path
	/// after it, and every link and history entry starts with it. Without
	/// one, the app is at the root of its site.
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
		let base = base_path(&window);
		let initial = R::parse(&base.route_url(&location(&window)))?;
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
			let base = base.clone();
			Closure::new(move |event: web_sys::PopStateEvent| {
				depth.store(
					event.state().as_f64().map_or(0, |depth| depth as usize),
					Ordering::SeqCst,
				);
				let window = web_sys::window().expect("a browser window");
				// Every URL decodes, so only an app without a root
				// `+not-found.rs` can reach one no route matches; its screen
				// stays as it was.
				if let Ok(route) = R::parse(&base.route_url(&location(&window))) {
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
				base,
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

/// Where the app is served: the directory of the page's `<base href>`, or
/// the site's root without one. A page's base URL is its own address unless
/// a `<base>` element names another, so only the element moves the app.
fn base_path(window: &web_sys::Window) -> BasePath {
	let Some(document) = window.document() else {
		return BasePath::root();
	};
	if !matches!(document.query_selector("base[href]"), Ok(Some(_))) {
		return BasePath::root();
	}

	document
		.base_uri()
		.ok()
		.flatten()
		.and_then(|base| web_sys::Url::new(&base).ok())
		.map_or_else(BasePath::root, |base| BasePath::of(&base.pathname()))
}

fn history() -> web_sys::History {
	web_sys::window()
		.expect("a browser window")
		.history()
		.expect("session history")
}

/// A router's link to the session history: where the app is served, how deep
/// it is, and the listener that follows the back and forward buttons until
/// the router is dropped.
pub(crate) struct Browser {
	base: BasePath,
	depth: Arc<AtomicUsize>,
	listener: SendWrapper<PopListener>,
}

impl Browser {
	/// Where `href` is on the page, under the app's base.
	pub(crate) fn page_url(&self, href: &Href) -> String {
		self.base.page_url(href)
	}

	pub(crate) fn push(&self, href: &Href) {
		let depth = self.depth.fetch_add(1, Ordering::SeqCst) + 1;
		history()
			.push_state_with_url(&JsValue::from(depth as f64), "", Some(&self.page_url(href)))
			.expect("add a history entry");
	}

	pub(crate) fn replace(&self, href: &Href) {
		let depth = self.depth.load(Ordering::SeqCst);
		history()
			.replace_state_with_url(&JsValue::from(depth as f64), "", Some(&self.page_url(href)))
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
