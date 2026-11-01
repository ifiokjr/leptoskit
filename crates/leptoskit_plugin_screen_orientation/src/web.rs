//! Document-scoped browser orientation requests. Browsers do not expose the
//! prior lock: release calls `unlock()`, restoring the browser's default
//! policy. Do not mix this adapter with external orientation locks on the same
//! document.

use std::cell::Cell;
use std::convert::Infallible;
use std::rc::Rc;

use futures::channel::oneshot;
use gloo_events::EventListener;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use super::*;

thread_local! { static HELD: Cell<bool> = const { Cell::new(false) }; }

fn error(value: &JsValue) -> Error {
	match value
		.dyn_ref::<web_sys::DomException>()
		.map(web_sys::DomException::name)
		.as_deref()
	{
		Some("NotAllowedError" | "SecurityError") => Error::PermissionDenied,
		Some("NotSupportedError") => Error::Unavailable,
		Some("AbortError" | "InvalidStateError") => Error::Cancelled,
		_ => Error::Native,
	}
}
fn orientation() -> Result<web_sys::ScreenOrientation, Error> {
	let screen = web_sys::window()
		.ok_or(Error::Unavailable)?
		.screen()
		.map_err(|value| error(&value))?;
	let orientation = js_sys::Reflect::get(&screen, &JsValue::from_str("orientation"))
		.map_err(|value| error(&value))?;
	orientation.dyn_into().map_err(|_| Error::Unavailable)
}
/// Reads the actual browser screen orientation, independently of a requested
/// lock.
pub fn snapshot() -> Result<Snapshot, Error> {
	let orientation = orientation()?;

	let kind = match orientation.type_().map_err(|value| error(&value))? {
		web_sys::OrientationType::PortraitPrimary | web_sys::OrientationType::PortraitSecondary => {
			Orientation::Portrait
		}
		web_sys::OrientationType::LandscapePrimary
		| web_sys::OrientationType::LandscapeSecondary => Orientation::Landscape,
		_ => Orientation::Unknown,
	};

	let result = Snapshot {
		orientation: kind,
		angle: Some(orientation.angle().map_err(|value| error(&value))?),
	};
	result.encode()?;
	Ok(result)
}
struct Guard(web_sys::ScreenOrientation);
impl Drop for Guard {
	fn drop(&mut self) {
		if let Err(failure) = self.0.unlock() {
			web_sys::console::error_1(&failure);
		}

		HELD.with(|held| held.set(false));
	}
}
/// Holds the document policy until this future is dropped, hidden, navigated
/// away, or exits fullscreen. Only one hold is permitted. Browser support,
/// fullscreen and user activation requirements still apply. This function does
/// not enter fullscreen itself. Poll alongside work to observe failures.
pub async fn hold(policy: Policy) -> Result<Infallible, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	let document = window.document().ok_or(Error::Unavailable)?;

	require_visible(&document)?;

	let orientation = lockable_orientation()?;

	claim_hold()?;

	let guard = Guard(orientation);
	let (sender, receiver) = oneshot::channel();
	let sender: Release = Rc::new(std::cell::RefCell::new(Some(sender)));

	let visibility = visibility_listener(&document, Rc::clone(&sender));
	let fullscreen = fullscreen_listener(&document, Rc::clone(&sender));
	let pagehide = pagehide_listener(&window, sender);

	let request = JsFuture::from(
		guard
			.0
			.lock(lock_type(policy))
			.map_err(|value| error(&value))?,
	);
	futures::pin_mut!(request);
	futures::pin_mut!(receiver);

	if let futures::future::Either::Left((result, receiver)) =
		futures::future::select(request, receiver).await
	{
		result.map_err(|value| error(&value))?;
		let _ = receiver.await;
	}

	drop((visibility, fullscreen, pagehide, guard));
	Err(Error::Cancelled)
}

/// The channel that ends a hold early, taken by whichever listener fires first.
type Release = Rc<std::cell::RefCell<Option<oneshot::Sender<()>>>>;

/// Refuses to lock a hidden document: the browser would ignore the request.
fn require_visible(document: &web_sys::Document) -> Result<(), Error> {
	if document.hidden() {
		return Err(Error::Unavailable);
	}

	Ok(())
}

/// The screen orientation, when the browser lets a page lock it.
fn lockable_orientation() -> Result<web_sys::ScreenOrientation, Error> {
	let orientation = orientation()?;

	if !js_sys::Reflect::get(&orientation, &JsValue::from_str("lock"))
		.map_err(|value| error(&value))?
		.is_function()
	{
		return Err(Error::Unavailable);
	}

	Ok(orientation)
}

/// Takes the single document hold, reporting a second one as busy.
fn claim_hold() -> Result<(), Error> {
	if HELD.with(|held| held.replace(true)) {
		return Err(Error::Busy);
	}

	Ok(())
}

/// The browser's lock type for a policy.
fn lock_type(policy: Policy) -> web_sys::OrientationLockType {
	match policy {
		Policy::Portrait => web_sys::OrientationLockType::Portrait,
		Policy::Landscape => web_sys::OrientationLockType::Landscape,
		Policy::PortraitPrimary => web_sys::OrientationLockType::PortraitPrimary,
		Policy::PortraitSecondary => web_sys::OrientationLockType::PortraitSecondary,
		Policy::LandscapePrimary => web_sys::OrientationLockType::LandscapePrimary,
		Policy::LandscapeSecondary => web_sys::OrientationLockType::LandscapeSecondary,
		Policy::Any => web_sys::OrientationLockType::Any,
	}
}

/// Announces the end of the hold, once.
fn release(sender: &Release) {
	if let Some(sender) = sender.borrow_mut().take() {
		let _ = sender.send(());
	}
}

/// Ends the hold when the page is hidden.
fn visibility_listener(document: &web_sys::Document, sender: Release) -> EventListener {
	let observed = document.clone();

	EventListener::new(document, "visibilitychange", move |_| {
		if observed.hidden() {
			release(&sender);
		}
	})
}

/// Ends the hold when the document leaves fullscreen.
fn fullscreen_listener(document: &web_sys::Document, sender: Release) -> EventListener {
	let observed = document.clone();

	EventListener::new(document, "fullscreenchange", move |_| {
		if observed.fullscreen_element().is_none() {
			release(&sender);
		}
	})
}

/// Ends the hold when the page is navigated away.
fn pagehide_listener(window: &web_sys::Window, sender: Release) -> EventListener {
	EventListener::new(window, "pagehide", move |_| release(&sender))
}
