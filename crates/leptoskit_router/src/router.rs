//! The router: the app's current route, how it changes, and how it renders.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use leptoskit_native::Element;
use leptoskit_native::components::Pressable;
use reactive_graph::signal::ArcRwSignal;
use reactive_graph::traits::Get;
use reactive_graph::traits::GetUntracked;
use reactive_graph::traits::Set;

use crate::Routable;

/// Renders a route: implemented by the generated `Route`, which calls the
/// route's page and wraps it in the layouts of every directory above it.
pub trait Render: Routable {
	/// The route's page inside its layouts. Pages and layouts that declare a
	/// `navigation` prop receive `navigation`.
	fn render(&self, navigation: &Router<Self>) -> Element;
}

/// The app's current route and the way to change it.
///
/// Cloning a router is cheap, and every clone navigates the same history.
/// The current route is a reactive signal, so a reactive closure that renders
/// it, such as `move || leptoskit::web::render(router.render())` on the web,
/// updates on its own; a native host renders again after each event.
///
/// Natively, the router keeps earlier routes in memory for [`back`]. In a
/// browser, [`Router::from_browser`] uses the session history instead, so the
/// address bar and the back button follow along.
///
/// [`back`]: Router::back
pub struct Router<R: 'static> {
	inner: Arc<Inner<R>>,
}

struct Inner<R: 'static> {
	current: ArcRwSignal<R>,
	history: History<R>,
}

enum History<R> {
	/// Earlier routes, oldest first.
	Memory(Mutex<Vec<R>>),
	/// The browser's session history.
	#[cfg(all(feature = "web", target_arch = "wasm32"))]
	Browser(crate::web::Browser),
}

impl<R> Clone for Router<R> {
	fn clone(&self) -> Self {
		Self {
			inner: Arc::clone(&self.inner),
		}
	}
}

impl<R> std::fmt::Debug for Router<R>
where
	R: Routable + Clone + PartialEq + Send + Sync + std::fmt::Debug + 'static,
{
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Router")
			.field("current", &self.inner.current.get_untracked())
			.finish_non_exhaustive()
	}
}

impl<R> Router<R>
where
	R: Routable + Clone + PartialEq + Send + Sync + 'static,
{
	/// A router that starts at `initial` and keeps its history in memory: for
	/// native apps and tests.
	pub fn new(initial: impl Into<R>) -> Self {
		Self::with_history(initial.into(), History::Memory(Mutex::new(Vec::new())))
	}

	#[cfg(all(feature = "web", target_arch = "wasm32"))]
	pub(crate) fn browser(current: ArcRwSignal<R>, browser: crate::web::Browser) -> Self {
		Self {
			inner: Arc::new(Inner {
				current,
				history: History::Browser(browser),
			}),
		}
	}

	fn with_history(initial: R, history: History<R>) -> Self {
		Self {
			inner: Arc::new(Inner {
				current: ArcRwSignal::new(initial),
				history,
			}),
		}
	}

	/// The current route. Reading it inside a reactive closure subscribes
	/// the closure to navigation.
	pub fn current(&self) -> R {
		self.inner.current.get()
	}

	/// Goes to `to`, keeping the current route to come back to. Navigating to
	/// the current route does nothing.
	pub fn navigate(&self, to: impl Into<R>) {
		let to = to.into();
		let current = self.inner.current.get_untracked();

		if to == current {
			return;
		}

		match &self.inner.history {
			History::Memory(earlier) => {
				earlier
					.lock()
					.unwrap_or_else(PoisonError::into_inner)
					.push(current);
			}
			#[cfg(all(feature = "web", target_arch = "wasm32"))]
			History::Browser(browser) => browser.push(&to.href()),
		}

		self.inner.current.set(to);
	}

	/// Goes to `to` in place of the current route, which back will not
	/// return to.
	pub fn replace(&self, to: impl Into<R>) {
		let to = to.into();

		#[cfg(all(feature = "web", target_arch = "wasm32"))]
		if let History::Browser(browser) = &self.inner.history {
			browser.replace(&to.href());
		}

		self.inner.current.set(to);
	}

	/// Whether [`back`](Self::back) has a route to return to.
	pub fn can_go_back(&self) -> bool {
		match &self.inner.history {
			History::Memory(earlier) => {
				!earlier
					.lock()
					.unwrap_or_else(PoisonError::into_inner)
					.is_empty()
			}
			#[cfg(all(feature = "web", target_arch = "wasm32"))]
			History::Browser(browser) => browser.can_go_back(),
		}
	}

	/// Returns to the previous route, and says whether there was one. In a
	/// browser this is the back button: the route changes when the browser
	/// reports it, a moment later.
	pub fn back(&self) -> bool {
		match &self.inner.history {
			History::Memory(earlier) => {
				let previous = earlier.lock().unwrap_or_else(PoisonError::into_inner).pop();
				previous.is_some_and(|previous| {
					self.inner.current.set(previous);
					true
				})
			}
			#[cfg(all(feature = "web", target_arch = "wasm32"))]
			History::Browser(browser) => browser.back(),
		}
	}

	/// A link to `to`: a [`Pressable`] named `label` that navigates when
	/// pressed. On the web it renders as an `<a href>`, so it can also open in
	/// a new tab. Add its content with `child`.
	pub fn link(&self, to: impl Into<R>, label: impl Into<String>) -> Element {
		let to = to.into();
		let href = to.href();
		let router = self.clone();
		Pressable(label)
			.href(href.as_str())
			.on_press(move || router.navigate(to.clone()))
	}

	/// The current route's page inside its layouts.
	pub fn render(&self) -> Element
	where
		R: Render,
	{
		self.current().render(self)
	}
}
