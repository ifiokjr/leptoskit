//! Browser foreground geolocation. Watches own their callbacks until
//! clearWatch.
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use futures::channel::oneshot;
use futures::future::Either;
use futures::future::{self};
use gloo_events::EventListener;
use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use super::*;

thread_local! { static ACTIVE: Cell<bool> = const { Cell::new(false) }; }
struct Reservation;
impl Reservation {
	fn acquire() -> Result<Self, Error> {
		ACTIVE.with(|active| {
			if active.replace(true) {
				Err(Error::Busy)
			} else {
				Ok(Self)
			}
		})
	}
}
impl Drop for Reservation {
	fn drop(&mut self) {
		ACTIVE.with(|active| active.set(false));
	}
}

fn window() -> Result<web_sys::Window, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;

	if js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("isSecureContext"))
		.map_err(|_| Error::Native)?
		.as_bool()
		!= Some(true)
	{
		return Err(Error::PermissionDenied);
	}
	Ok(window)
}
/// Queries foreground permission without acquiring location or showing a
/// prompt. Browsers without the Permissions geolocation query return
/// Unavailable.
pub async fn permission() -> Result<Permission, Error> {
	let permissions = window()?
		.navigator()
		.permissions()
		.map_err(|_| Error::Unavailable)?;
	let descriptor = web_sys::PermissionDescriptor::new(web_sys::PermissionName::Geolocation);
	let value = wasm_bindgen_futures::JsFuture::from(
		permissions
			.query(&descriptor)
			.map_err(|_| Error::Unavailable)?,
	)
	.await
	.map_err(|_| Error::Unavailable)?;
	let status = value
		.dyn_into::<web_sys::PermissionStatus>()
		.map_err(|_| Error::Native)?;

	let status = match status.state() {
		web_sys::PermissionState::Granted => Status::Granted,
		web_sys::PermissionState::Denied => Status::Denied,
		web_sys::PermissionState::Prompt => Status::Prompt,
		_ => Status::Unknown,
	};

	Ok(Permission {
		status,
		accuracy: AccuracyPermission::Unknown,
	})
}
/// Requests location access from an intentional user action. Browsers request
/// permission by acquiring a fix; that coordinate is discarded. This is not a
/// no-location permission API. Acquisition is bounded to 30 seconds.
pub async fn request_permission(high_accuracy: bool) -> Result<Permission, Error> {
	let options = Options::new(high_accuracy, 30_000, 0)?;
	request_permission_with_options(options).await
}

async fn request_permission_with_options(options: Options) -> Result<Permission, Error> {
	match bounded(options, None, true).await {
		Ok(_) => {
			Ok(Permission {
				status: Status::Granted,
				accuracy: AccuracyPermission::Unknown,
			})
		}
		Err(LocationError::Bridge(Error::PermissionDenied)) => {
			Ok(Permission {
				status: Status::Denied,
				accuracy: AccuracyPermission::Unknown,
			})
		}
		Err(LocationError::Bridge(error)) => Err(error),
		Err(LocationError::TimedOut) => Err(Error::Unavailable),
	}
}
/// Acquires a bounded fix only after observing an existing permission grant.
/// Browsers do not atomically combine that query with acquisition: a concurrent
/// permission revocation can still cause browser-controlled permission UI.
pub async fn current(options: Options) -> Result<Fix, LocationError> {
	bounded(options, None, false).await
}
/// Waits for a newer timestamp, clearing the browser watch on completion/drop.
/// Successive calls form a demand-driven foreground watch, not a durable path
/// log.
pub async fn next(options: Options, after_timestamp_ms: f64) -> Result<Fix, LocationError> {
	if !after_timestamp_ms.is_finite() || after_timestamp_ms < 0.0 {
		return Err(Error::InvalidArgument.into());
	}

	bounded(options, Some(after_timestamp_ms), false).await
}
async fn bounded(
	options: Options,
	after: Option<f64>,
	may_prompt: bool,
) -> Result<Fix, LocationError> {
	match future::select(
		Box::pin(acquire(options, after, may_prompt)),
		Box::pin(TimeoutFuture::new(options.timeout_ms())),
	)
	.await
	{
		Either::Left((result, _)) => result,
		Either::Right(_) => Err(LocationError::TimedOut),
	}
}
struct Watch {
	geolocation: web_sys::Geolocation,
	identifier: i32,
}
impl Drop for Watch {
	fn drop(&mut self) {
		self.geolocation.clear_watch(self.identifier);
	}
}
fn position(value: &web_sys::Position) -> Result<Fix, Error> {
	let coords = value.coords();
	Fix {
		timestamp_ms: value.timestamp(),
		latitude: coords.latitude(),
		longitude: coords.longitude(),
		accuracy_m: coords.accuracy(),
		altitude_m: coords.altitude(),
		altitude_accuracy_m: coords.altitude_accuracy(),
		// Older browser implementations use NaN for stationary unknown course.
		heading_degrees: coords.heading().filter(|heading| !heading.is_nan()),
		speed_mps: coords.speed(),
		provider: Provider::Unknown,
		simulated: None,
	}
	.validate()
}
async fn acquire(
	options: Options,
	after: Option<f64>,
	may_prompt: bool,
) -> Result<Fix, LocationError> {
	let _reservation = Reservation::acquire()?;

	if !may_prompt && permission().await?.status != Status::Granted {
		return Err(Error::PermissionDenied.into());
	}

	let window = window()?;
	let document = window.document().ok_or(Error::Unavailable)?;

	if document.hidden() {
		return Err(Error::Cancelled.into());
	}

	let geolocation = window
		.navigator()
		.geolocation()
		.map_err(|_| Error::Unavailable)?;
	let (sender, receiver) = oneshot::channel();
	let sender = Rc::new(RefCell::new(Some(sender)));
	let accepted = Rc::clone(&sender);
	let started = js_sys::Date::now();
	let success = Closure::<dyn FnMut(web_sys::Position)>::new(move |value: web_sys::Position| {
		let result = position(&value).map_err(LocationError::from);
		if let Ok(fix) = &result {
			let oldest = if options.maximum_age_ms() == 0 {
				started
			} else {
				js_sys::Date::now() - f64::from(options.maximum_age_ms())
			};
			if fix.timestamp_ms < oldest || after.is_some_and(|time| fix.timestamp_ms <= time) {
				return;
			}
		}
		if let Some(sender) = accepted.borrow_mut().take() {
			let _ = sender.send(result);
		}
	});
	let failed = Rc::clone(&sender);
	let failure =
		Closure::<dyn FnMut(web_sys::PositionError)>::new(move |error: web_sys::PositionError| {
			let error = match error.code() {
				1 => Error::PermissionDenied.into(),
				2 => Error::Unavailable.into(),
				3 => LocationError::TimedOut,
				_ => Error::Native.into(),
			};
			if let Some(sender) = failed.borrow_mut().take() {
				let _ = sender.send(Err(error));
			}
		});
	let source = document.clone();
	let visibility = EventListener::new(&document, "visibilitychange", move |_| {
		if source.hidden() {
			if let Some(sender) = sender.borrow_mut().take() {
				let _ = sender.send(Err(Error::Cancelled.into()));
			}
		}
	});
	let native_options = web_sys::PositionOptions::new();
	native_options.set_enable_high_accuracy(options.high_accuracy());
	native_options.set_timeout(options.timeout_ms());
	native_options.set_maximum_age(options.maximum_age_ms());
	let identifier = geolocation
		.watch_position_with_error_callback_and_options(
			success.as_ref().unchecked_ref(),
			Some(failure.as_ref().unchecked_ref()),
			&native_options,
		)
		.map_err(|error| {
			let name = js_sys::Reflect::get(&error, &JsValue::from_str("name"))
				.ok()
				.and_then(|name| name.as_string());
			if matches!(name.as_deref(), Some("SecurityError" | "NotAllowedError")) {
				Error::PermissionDenied
			} else {
				Error::Native
			}
		})?;
	// Declare the watch last: cancellation clears it before freeing callback
	// closures.
	let watch = Watch {
		geolocation,
		identifier,
	};
	let result = receiver
		.await
		.map_err(|_| LocationError::from(Error::Cancelled))?;
	drop(watch);
	drop((success, failure, visibility));
	result
}

#[cfg(test)]
mod tests {
	use wasm_bindgen::prelude::wasm_bindgen;
	use wasm_bindgen_test::wasm_bindgen_test;

	use super::*;
	wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
	#[wasm_bindgen(inline_js = "
let oldGeo, oldPermissions, success, failure, starts, clears, permissionState;
export function installLocation() {
  oldGeo = Object.getOwnPropertyDescriptor(navigator, 'geolocation');
  oldPermissions = Object.getOwnPropertyDescriptor(navigator, 'permissions');
  starts = 0; clears = 0; permissionState = 'granted';
  Object.defineProperty(navigator, 'permissions', {configurable: true, value: {
    query() {
      if (permissionState === 'pending') return new Promise(() => {});
      const result = Object.create(PermissionStatus.prototype);
      Object.defineProperty(result, 'state', {value: permissionState});
      return Promise.resolve(result);
    }
  }});
  Object.defineProperty(navigator, 'geolocation', {configurable: true, value: {
    watchPosition(ok, bad) { starts++; success = ok; failure = bad; return starts; },
    clearWatch() { clears++; success = null; failure = null; }
  }});
}
export function restoreLocation() {
  if (oldGeo) Object.defineProperty(navigator, 'geolocation', oldGeo); else delete \
	                            navigator.geolocation;
  if (oldPermissions) Object.defineProperty(navigator, 'permissions', oldPermissions); else delete \
	                            navigator.permissions;
}
export function locationCounts() { return [starts, clears]; }
export function locationPermission(state) { permissionState = state; }
export function locationFix() {
  success({timestamp: Date.now(), coords: {latitude: 37, longitude: -122, accuracy: 5,
    altitude: null, altitudeAccuracy: null, heading: null, speed: 0}});
}
export function locationError(code) { failure({code}); }
")]
	extern "C" {
		#[wasm_bindgen(js_name = installLocation)]
		fn install();
		#[wasm_bindgen(js_name = restoreLocation)]
		fn restore();
		#[wasm_bindgen(js_name = locationCounts)]
		fn counts() -> js_sys::Array;
		#[wasm_bindgen(js_name = locationPermission)]
		fn set_permission(state: &str);
		#[wasm_bindgen(js_name = locationFix)]
		fn send_fix();
		#[wasm_bindgen(js_name = locationError)]
		fn send_error(code: u32);
	}
	struct Installed;
	impl Drop for Installed {
		fn drop(&mut self) {
			restore();
		}
	}
	#[wasm_bindgen_test]
	async fn browser_location_is_bounded_cancelled_and_requires_observed_permission() {
		install();
		let _installed = Installed;
		let options = Options::new(false, 1000, 60_000).unwrap();
		let mut reading = Box::pin(current(options));
		assert!(futures::poll!(&mut reading).is_pending());
		TimeoutFuture::new(0).await;
		assert!(futures::poll!(&mut reading).is_pending());

		assert_eq!(current(options).await, Err(Error::Busy.into()));
		send_fix();
		let fix = reading.await.unwrap();
		assert_eq!(fix.latitude.to_bits(), 37_f64.to_bits());
		assert_eq!(fix.altitude_m, None);
		assert_eq!(fix.speed_mps, Some(0.0));
		assert_eq!(
			counts().to_vec(),
			[JsValue::from_f64(1.0), JsValue::from_f64(1.0)]
		);

		let mut cancelled = Box::pin(current(options));
		assert!(futures::poll!(&mut cancelled).is_pending());
		TimeoutFuture::new(0).await;
		assert!(futures::poll!(&mut cancelled).is_pending());
		drop(cancelled);
		assert_eq!(
			counts().to_vec(),
			[JsValue::from_f64(2.0), JsValue::from_f64(2.0)]
		);

		let mut timed = Box::pin(current(options));
		assert!(futures::poll!(&mut timed).is_pending());
		TimeoutFuture::new(0).await;
		assert!(futures::poll!(&mut timed).is_pending());
		send_error(3);
		assert_eq!(timed.await, Err(LocationError::TimedOut));
		assert_eq!(
			counts().to_vec(),
			[JsValue::from_f64(3.0), JsValue::from_f64(3.0)]
		);

		set_permission("prompt");
		assert_eq!(current(options).await, Err(Error::PermissionDenied.into()));
		assert_eq!(
			counts().to_vec(),
			[JsValue::from_f64(3.0), JsValue::from_f64(3.0)]
		);
		set_permission("granted");
		assert_eq!(
			current(Options::new(false, 5, 0).unwrap()).await,
			Err(LocationError::TimedOut)
		);
		assert_eq!(
			counts().to_vec(),
			[JsValue::from_f64(4.0), JsValue::from_f64(4.0)]
		);
		set_permission("pending");
		assert_eq!(
			current(Options::new(false, 5, 0).unwrap()).await,
			Err(LocationError::TimedOut)
		);

		assert_eq!(
			counts().to_vec(),
			[JsValue::from_f64(4.0), JsValue::from_f64(4.0)]
		);
		set_permission("granted");
		let mut denied = Box::pin(current(options));
		assert!(futures::poll!(&mut denied).is_pending());
		TimeoutFuture::new(0).await;
		assert!(futures::poll!(&mut denied).is_pending());
		send_error(1);
		assert_eq!(denied.await, Err(Error::PermissionDenied.into()));
		let mut newer = Box::pin(next(options, js_sys::Date::now() + 60_000.0));
		assert!(futures::poll!(&mut newer).is_pending());
		TimeoutFuture::new(0).await;
		assert!(futures::poll!(&mut newer).is_pending());
		send_fix();
		assert!(futures::poll!(&mut newer).is_pending());
		drop(newer);
		assert_eq!(
			counts().to_vec(),
			[JsValue::from_f64(6.0), JsValue::from_f64(6.0)]
		);
		// A timed-out permission request must not issue an unbounded query.
		set_permission("pending");
		let result = future::select(
			Box::pin(request_permission_with_options(
				Options::new(false, 5, 0).unwrap(),
			)),
			Box::pin(TimeoutFuture::new(1000)),
		)
		.await;
		assert!(matches!(result, Either::Left((Err(Error::Unavailable), _))));
		assert_eq!(
			counts().to_vec(),
			[JsValue::from_f64(7.0), JsValue::from_f64(7.0)]
		);
	}
}
