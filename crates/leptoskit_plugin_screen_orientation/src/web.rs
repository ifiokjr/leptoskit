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

	if document.hidden() {
		return Err(Error::Unavailable);
	}

	let orientation = orientation()?;

	if !js_sys::Reflect::get(&orientation, &JsValue::from_str("lock"))
		.map_err(|value| error(&value))?
		.is_function()
	{
		return Err(Error::Unavailable);
	}

	if HELD.with(|held| held.replace(true)) {
		return Err(Error::Busy);
	}

	let guard = Guard(orientation);
	let (sender, receiver) = oneshot::channel();
	let sender = Rc::new(std::cell::RefCell::new(Some(sender)));
	let hidden_sender = sender.clone();
	let observed_document = document.clone();
	let visibility = EventListener::new(&document, "visibilitychange", move |_| {
		if observed_document.hidden() {
			if let Some(sender) = hidden_sender.borrow_mut().take() {
				let _ = sender.send(());
			}
		}
	});
	let fullscreen_sender = sender.clone();
	let observed_document = document.clone();
	let fullscreen = EventListener::new(&document, "fullscreenchange", move |_| {
		if observed_document.fullscreen_element().is_none() {
			if let Some(sender) = fullscreen_sender.borrow_mut().take() {
				let _ = sender.send(());
			}
		}
	});
	let pagehide = EventListener::new(&window, "pagehide", move |_| {
		if let Some(sender) = sender.borrow_mut().take() {
			let _ = sender.send(());
		}
	});

	let policy = match policy {
		Policy::Portrait => web_sys::OrientationLockType::Portrait,
		Policy::Landscape => web_sys::OrientationLockType::Landscape,
		Policy::PortraitPrimary => web_sys::OrientationLockType::PortraitPrimary,
		Policy::PortraitSecondary => web_sys::OrientationLockType::PortraitSecondary,
		Policy::LandscapePrimary => web_sys::OrientationLockType::LandscapePrimary,
		Policy::LandscapeSecondary => web_sys::OrientationLockType::LandscapeSecondary,
		Policy::Any => web_sys::OrientationLockType::Any,
	};

	let request = JsFuture::from(guard.0.lock(policy).map_err(|value| error(&value))?);
	futures::pin_mut!(request);
	futures::pin_mut!(receiver);

	match futures::future::select(request, receiver).await {
		futures::future::Either::Left((result, receiver)) => {
			result.map_err(|value| error(&value))?;
			let _ = receiver.await;
		}
		futures::future::Either::Right(_) => {}
	}

	drop((visibility, fullscreen, pagehide, guard));
	Err(Error::Cancelled)
}
