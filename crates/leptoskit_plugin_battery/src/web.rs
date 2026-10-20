//! Browser snapshots and demand-driven Battery Status API subscriptions.
use std::cell::RefCell;
use std::future::poll_fn;
use std::rc::Rc;
use std::task::Poll;
use std::task::Waker;

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use web_sys::BatteryManager;
use web_sys::Event;

use crate::Charge;
use crate::Error;
use crate::Snapshot;
use crate::State;

fn js_error(value: &JsValue) -> Error {
	match value
		.dyn_ref::<web_sys::DomException>()
		.map(web_sys::DomException::name)
		.as_deref()
	{
		Some("NotAllowedError" | "SecurityError") => Error::PermissionDenied,
		_ => Error::Native,
	}
}

async fn manager() -> Result<BatteryManager, Error> {
	let navigator = web_sys::window().ok_or(Error::Unavailable)?.navigator();
	let function = js_sys::Reflect::get(navigator.as_ref(), &JsValue::from_str("getBattery"))
		.map_err(|error| js_error(&error))?
		.dyn_into::<js_sys::Function>()
		.map_err(|_| Error::Unavailable)?;
	let promise = function
		.call0(navigator.as_ref())
		.map_err(|error| js_error(&error))?
		.dyn_into::<js_sys::Promise>()
		.map_err(|_| Error::Native)?;
	JsFuture::from(promise)
		.await
		.map_err(|error| js_error(&error))?
		.dyn_into::<BatteryManager>()
		.map_err(|_| Error::Native)
}

fn reading(battery: &BatteryManager) -> Result<Snapshot, Error> {
	let charge = Charge::from_fraction(battery.level())?;
	Ok(Snapshot {
		present: None,
		charge: Some(charge),
		state: if battery.charging() {
			if charge.thousandths() == 1000 {
				State::Full
			} else {
				State::Charging
			}
		} else {
			State::Discharging
		},
		power_save: None,
	})
}

/// Requests a browser reading. Missing APIs return `Unavailable`; permission
/// rejection remains distinct from other native failures.
pub async fn snapshot() -> Result<Snapshot, Error> {
	reading(&manager().await?)
}

struct Listeners {
	manager: BatteryManager,
	callback: Closure<dyn FnMut(Event)>,
	names: Vec<&'static str>,
}

impl Drop for Listeners {
	fn drop(&mut self) {
		for name in &self.names {
			// EventTarget.removeEventListener is nonthrowing in the DOM
			// specification.
			let _ = self
				.manager
				.remove_event_listener_with_callback(name, self.callback.as_ref().unchecked_ref());
		}
	}
}

/// Waits for a state different from `previous`. The returned future owns the
/// two browser listeners and removes them on completion or drop. State is
/// sampled after installation to cover changes between consecutive waits.
/// Intermediate transitions can be coalesced; no notification queue grows
/// behind a slow caller.
pub async fn wait_for_change(previous: Snapshot) -> Result<Snapshot, Error> {
	let manager = manager().await?;
	let waker: Rc<RefCell<Option<Waker>>> = Rc::default();
	let callback_waker = Rc::clone(&waker);
	let callback = Closure::new(move |_: Event| {
		let waker = callback_waker.borrow_mut().take();
		if let Some(waker) = waker {
			waker.wake();
		}
	});
	let mut listeners = Listeners {
		manager,
		callback,
		names: Vec::with_capacity(2),
	};
	for name in ["chargingchange", "levelchange"] {
		listeners
			.manager
			.add_event_listener_with_callback(name, listeners.callback.as_ref().unchecked_ref())
			.map_err(|error| js_error(&error))?;
		listeners.names.push(name);
	}
	poll_fn(|cx| {
		match reading(&listeners.manager) {
			Ok(snapshot) if snapshot == previous => {
				*waker.borrow_mut() = Some(cx.waker().clone());
				Poll::Pending
			}
			result => Poll::Ready(result),
		}
	})
	.await
}
