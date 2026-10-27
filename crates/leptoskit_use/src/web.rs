//! Browser-only hooks, compiled for wasm32-unknown targets with the `web`
//! feature enabled.
//!
//! These hooks call browser APIs that have no renderer-independent meaning,
//! so they exist only here: shared screens keep using the portable hooks, and
//! web builds add browser behaviour behind `#[cfg(...)]` guards. Device and
//! network state belongs to the plugin crates' signal hooks instead.
//!
//! Listeners are scoped to the reactive owner: cleanup unsubscribes them, and
//! the returned stop handle unsubscribes early. Owner cleanup runs `Send`
//! closures, while JS closures are not `Send`, so listeners are tracked in a
//! thread-local registry and cleanup removes them by id.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use reactive_graph::owner::on_cleanup;
use reactive_graph::prelude::*;
use reactive_graph::signal::RwSignal;
use reactive_graph::wrappers::read::Signal;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::EventTarget;

/// A handle that unsubscribes one listener. Browser closures stay on the
/// event-loop thread, so this is not `Send`.
pub type StopListening = Arc<dyn Fn() + 'static>;

/// One installed browser listener.
struct RegisteredListener {
	target: EventTarget,
	event_type: String,
	closure: Option<Closure<dyn FnMut(web_sys::Event)>>,
}

thread_local! {
	static LISTENERS: RefCell<HashMap<u64, RegisteredListener>> = RefCell::new(HashMap::new());
}

static NEXT_LISTENER: AtomicU64 = AtomicU64::new(0);

/// Unsubscribes and forgets one listener.
fn remove_listener(id: u64) {
	LISTENERS.with(|live| {
		if let Some(registered) = live.borrow_mut().remove(&id)
			&& let Some(closure) = registered.closure
		{
			let _ = registered.target.remove_event_listener_with_callback(
				&registered.event_type,
				closure.as_ref().unchecked_ref(),
			);
		}
	});
}

/// Subscribe to one DOM event on a target until the owner is disposed or the
/// returned handle runs.
///
/// The handler receives the event cast to `E`; events that fail the cast are
/// ignored. Installation cannot fail in practice and panics if the browser
/// rejects the listener.
pub fn use_event_listener<E>(
	target: &EventTarget,
	event_type: &str,
	handler: impl Fn(E) + 'static,
) -> StopListening
where
	E: JsCast + 'static,
{
	let closure = Closure::new(move |event: web_sys::Event| {
		if let Ok(typed) = event.dyn_into::<E>() {
			handler(typed);
		}
	});
	target
		.add_event_listener_with_callback(event_type, closure.as_ref().unchecked_ref())
		.expect("the browser rejected an event listener");

	let id = NEXT_LISTENER.fetch_add(1, Ordering::Relaxed);
	LISTENERS.with(|live| {
		live.borrow_mut().insert(
			id,
			RegisteredListener {
				target: target.clone(),
				event_type: event_type.to_owned(),
				closure: Some(closure),
			},
		);
	});
	on_cleanup(move || remove_listener(id));
	Arc::new(move || remove_listener(id))
}

/// Track a CSS media query.
///
/// The signal starts with the query's current match and follows `change`
/// events. Without a browser window, or for a query the browser rejects, the
/// signal stays `false`.
pub fn use_media_query(query: impl Into<String>) -> Signal<bool> {
	let query = query.into();
	let Some(window) = web_sys::window() else {
		return Signal::derive(|| false);
	};
	let Ok(Some(list)) = window.match_media(&query) else {
		return Signal::derive(|| false);
	};
	let matches = RwSignal::new(list.matches());
	use_event_listener(
		&list.unchecked_into::<EventTarget>(),
		"change",
		move |event: web_sys::MediaQueryListEvent| matches.set(event.matches()),
	);
	matches.into()
}

/// Track the `prefers-color-scheme: dark` setting.
pub fn use_preferred_dark() -> Signal<bool> {
	use_media_query("(prefers-color-scheme: dark)")
}

/// Track the `prefers-reduced-motion: reduce` setting.
pub fn use_preferred_reduced_motion() -> Signal<bool> {
	use_media_query("(prefers-reduced-motion: reduce)")
}
