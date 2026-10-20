//! Secure-context browser sensors with cancellation-owned listeners and
//! hardware.
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use futures::channel::oneshot;
use gloo_events::EventListener;
use js_sys::Function;
use js_sys::Reflect;
use leptoskit_bridge::Error;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

use crate::Interval;
use crate::VectorReading;

/// Generic Sensor types supported by the browser adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
	/// Acceleration including gravity, in metres per second squared.
	Accelerometer,
	/// Rotation in radians per second.
	Gyroscope,
	/// Magnetic field in microteslas.
	Magnetometer,
}
impl Kind {
	fn constructor(self) -> &'static str {
		match self {
			Self::Accelerometer => "Accelerometer",
			Self::Gyroscope => "Gyroscope",
			Self::Magnetometer => "Magnetometer",
		}
	}

	fn bit(self) -> u8 {
		match self {
			Self::Accelerometer => 1,
			Self::Gyroscope => 2,
			Self::Magnetometer => 4,
		}
	}
}
thread_local! { static ACTIVE: Cell<u8> = const { Cell::new(0) }; }
struct Reservation(u8);
impl Reservation {
	fn acquire(kind: Kind) -> Result<Self, Error> {
		Self::acquire_bit(kind.bit())
	}

	fn acquire_bit(bit: u8) -> Result<Self, Error> {
		ACTIVE.with(|active| {
			if active.get() & bit != 0 {
				return Err(Error::Busy);
			}
			active.set(active.get() | bit);
			Ok(Self(bit))
		})
	}
}
impl Drop for Reservation {
	fn drop(&mut self) {
		ACTIVE.with(|active| active.set(active.get() & !self.0));
	}
}

fn property(value: &JsValue, name: &str) -> Result<JsValue, Error> {
	Reflect::get(value, &JsValue::from_str(name)).map_err(js_error)
}
#[allow(
	clippy::needless_pass_by_value,
	reason = "Maps owned JavaScript exceptions directly in Result::map_err."
)]
fn js_error(error: JsValue) -> Error {
	match Reflect::get(&error, &JsValue::from_str("name"))
		.ok()
		.and_then(|v| v.as_string())
		.as_deref()
	{
		Some("NotAllowedError" | "SecurityError") => Error::PermissionDenied,
		Some("NotSupportedError" | "NotReadableError") => Error::Unavailable,
		_ => Error::Native,
	}
}
fn window() -> Result<web_sys::Window, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	if property(window.as_ref(), "isSecureContext")?.as_bool() != Some(true) {
		return Err(Error::PermissionDenied);
	}
	Ok(window)
}
fn generic_constructor(kind: Kind) -> Result<Option<Function>, Error> {
	Ok(property(window()?.as_ref(), kind.constructor())?
		.dyn_into::<Function>()
		.ok())
}

/// Reports API presence, not permission or the presence of functioning
/// hardware. `next` can still fail with Unavailable or `PermissionDenied`.
pub fn available(kind: Kind) -> Result<bool, Error> {
	if generic_constructor(kind)?.is_some() {
		return Ok(true);
	}
	Ok(kind != Kind::Magnetometer
		&& property(window()?.as_ref(), "DeviceMotionEvent")?.is_function())
}

/// Requests Safari's `DeviceMotion` permission when applicable. Call directly
/// from a click/tap handler and poll immediately, before unrelated asynchronous
/// work. Generic Sensor permissions are enforced by the browser when the sensor
/// starts.
pub async fn request_permission(kind: Kind) -> Result<(), Error> {
	if generic_constructor(kind)?.is_some() {
		return Ok(());
	}
	if kind == Kind::Magnetometer {
		return Err(Error::Unavailable);
	}
	let constructor = property(window()?.as_ref(), "DeviceMotionEvent")?;
	if !constructor.is_function() {
		return Err(Error::Unavailable);
	}
	let request = property(&constructor, "requestPermission")?;
	if request.is_undefined() {
		return Ok(());
	}
	let request = request.dyn_into::<Function>().map_err(|_| Error::Native)?;
	let promise = request
		.call0(&constructor)
		.map_err(js_error)?
		.dyn_into::<js_sys::Promise>()
		.map_err(|_| Error::Native)?;
	let status = wasm_bindgen_futures::JsFuture::from(promise)
		.await
		.map_err(js_error)?;
	if status.as_string().as_deref() == Some("granted") {
		Ok(())
	} else {
		Err(Error::PermissionDenied)
	}
}

struct Sensor {
	value: JsValue,
	stop: Function,
}
impl Drop for Sensor {
	fn drop(&mut self) {
		// Stop is synchronous and cannot report failures from Drop. Event
		// listeners are separately RAII-owned, so cancellation never
		// retains guest closures.
		let _ = self.stop.call0(&self.value);
	}
}
fn number(value: &JsValue, name: &str) -> Result<f64, Error> {
	property(value, name)?
		.as_f64()
		.filter(|v| v.is_finite())
		.ok_or(Error::Unavailable)
}

/// Acquires one sample and stops the sensor before returning. Dropping the
/// future removes listeners and stops acquisition. Only one waiter per kind is
/// accepted. Generic Sensor uses the requested rate; `DeviceMotion` uses the
/// browser's rate.
pub async fn next(kind: Kind, interval: Interval) -> Result<VectorReading, Error> {
	let _reservation = Reservation::acquire(kind)?;
	if let Some(constructor) = generic_constructor(kind)? {
		return generic_next(constructor, interval, |source| {
			validate(VectorReading {
				timestamp: number(source, "timestamp")? / 1000.0,
				x: number(source, "x")?,
				y: number(source, "y")?,
				z: number(source, "z")?,
			})
		})
		.await;
	}
	if kind == Kind::Magnetometer {
		return Err(Error::Unavailable);
	}
	if !available(kind)? {
		return Err(Error::Unavailable);
	}
	request_permission(kind).await?;
	let (sender, receiver) = oneshot::channel();
	let sender = Rc::new(RefCell::new(Some(sender)));
	let window = window()?;
	let listener = EventListener::new(&window, "devicemotion", move |event| {
		if let Some(sender) = sender.borrow_mut().take() {
			let result = (|| {
				let values = property(
					event.as_ref(),
					if kind == Kind::Accelerometer {
						"accelerationIncludingGravity"
					} else {
						"rotationRate"
					},
				)?;
				let (x, y, z) = if kind == Kind::Accelerometer {
					(
						number(&values, "x")?,
						number(&values, "y")?,
						number(&values, "z")?,
					)
				} else {
					// DeviceMotion beta/gamma/alpha are rotations around x/y/z
					// in degrees/s.
					(
						number(&values, "beta")?.to_radians(),
						number(&values, "gamma")?.to_radians(),
						number(&values, "alpha")?.to_radians(),
					)
				};
				validate(VectorReading {
					timestamp: event.time_stamp() / 1000.0,
					x,
					y,
					z,
				})
			})();
			let _ = sender.send(result);
		}
	});
	let result = receiver.await.map_err(|_| Error::Cancelled)?;
	drop(listener);
	result
}
fn validate(reading: VectorReading) -> Result<VectorReading, Error> {
	reading.encode()?;
	Ok(reading)
}

#[cfg(test)]
mod tests {
	use wasm_bindgen::prelude::wasm_bindgen;
	use wasm_bindgen_test::wasm_bindgen_test;

	use super::*;

	wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

	#[wasm_bindgen(inline_js = "
let previous, current, started, stopped;
export function installSensor() {
  previous = Object.getOwnPropertyDescriptor(window, 'Accelerometer');
  started = 0; stopped = 0;
  class TestSensor extends EventTarget {
    constructor(options) { super(); this.frequency = options.frequency; current = this; }
    start() { started++; }
    stop() { stopped++; }
  }
  Object.defineProperty(window, 'Accelerometer', {configurable: true, value: TestSensor});
}
export function restoreSensor() {
  if (previous) Object.defineProperty(window, 'Accelerometer', previous);
  else delete window.Accelerometer;
}
export function sensorCounts() { return [started, stopped]; }
export function emitReading() {
  current.timestamp = 1250; current.x = -9.80665; current.y = 0; current.z = 3.5;
  current.dispatchEvent(new Event('reading'));
}
export function emitDenied() {
  const event = new Event('error');
  Object.defineProperty(event, 'error', {value: new DOMException('Denied', 'NotAllowedError')});
  current.dispatchEvent(event);
}
")]
	extern "C" {
		#[wasm_bindgen(js_name = installSensor)]
		fn install_sensor();
		#[wasm_bindgen(js_name = restoreSensor)]
		fn restore_sensor();
		#[wasm_bindgen(js_name = sensorCounts)]
		fn sensor_counts() -> js_sys::Array;
		#[wasm_bindgen(js_name = emitReading)]
		fn emit_reading();
		#[wasm_bindgen(js_name = emitDenied)]
		fn emit_denied();
	}
	struct InstalledSensor;
	impl Drop for InstalledSensor {
		fn drop(&mut self) {
			restore_sensor();
		}
	}

	#[wasm_bindgen_test]
	async fn browser_sampling_releases_hardware_on_success_denial_and_cancellation() {
		install_sensor();
		let _installed = InstalledSensor;
		let interval = Interval::new(16).unwrap();
		let mut sample = Box::pin(next(Kind::Accelerometer, interval));
		assert!(futures::poll!(&mut sample).is_pending());
		assert_eq!(next(Kind::Accelerometer, interval).await, Err(Error::Busy));
		emit_reading();
		let reading = sample.await.unwrap();
		assert_eq!(reading.timestamp.to_bits(), 1.25_f64.to_bits());
		assert_eq!(reading.x.to_bits(), (-9.80665_f64).to_bits());
		assert_eq!(
			sensor_counts().to_vec(),
			[JsValue::from_f64(1.0), JsValue::from_f64(1.0)]
		);

		let mut denied = Box::pin(next(Kind::Accelerometer, interval));
		assert!(futures::poll!(&mut denied).is_pending());
		emit_denied();
		assert_eq!(denied.await, Err(Error::PermissionDenied));
		assert_eq!(
			sensor_counts().to_vec(),
			[JsValue::from_f64(2.0), JsValue::from_f64(2.0)]
		);

		let mut cancelled = Box::pin(next(Kind::Accelerometer, interval));
		assert!(futures::poll!(&mut cancelled).is_pending());
		drop(cancelled);
		assert_eq!(
			sensor_counts().to_vec(),
			[JsValue::from_f64(3.0), JsValue::from_f64(3.0)]
		);
		// Reservation is released alongside the hardware; a replacement call
		// starts.
		let mut replacement = Box::pin(next(Kind::Accelerometer, interval));
		assert!(futures::poll!(&mut replacement).is_pending());
		drop(replacement);
		assert_eq!(
			sensor_counts().to_vec(),
			[JsValue::from_f64(4.0), JsValue::from_f64(4.0)]
		);
	}
}

async fn generic_next<T: 'static>(
	constructor: Function,
	interval: Interval,
	parse: fn(&JsValue) -> Result<T, Error>,
) -> Result<T, Error> {
	let options = js_sys::Object::new();
	Reflect::set(
		&options,
		&JsValue::from_str("frequency"),
		&JsValue::from_f64(1000.0 / f64::from(interval.milliseconds())),
	)
	.map_err(js_error)?;
	let arguments = js_sys::Array::new();
	arguments.push(&options);
	let value = Reflect::construct(&constructor, &arguments).map_err(js_error)?;
	let sensor = Sensor {
		stop: property(&value, "stop")?
			.dyn_into::<Function>()
			.map_err(|_| Error::Native)?,
		value,
	};
	let target = sensor
		.value
		.clone()
		.dyn_into::<web_sys::EventTarget>()
		.map_err(|_| Error::Native)?;
	let (sender, receiver) = oneshot::channel();
	let sender = Rc::new(RefCell::new(Some(sender)));
	let source = sensor.value.clone();
	let reading_sender = Rc::clone(&sender);
	let reading = EventListener::new(&target, "reading", move |_| {
		if let Some(sender) = reading_sender.borrow_mut().take() {
			let sample = parse(&source);
			let _ = sender.send(sample);
		}
	});
	let error = EventListener::new(&target, "error", move |event| {
		if let Some(sender) = sender.borrow_mut().take() {
			let error = property(event.as_ref(), "error").map_or_else(|e| e, js_error);
			let _ = sender.send(Err(error));
		}
	});
	property(&sensor.value, "start")?
		.dyn_into::<Function>()
		.map_err(|_| Error::Native)?
		.call0(&sensor.value)
		.map_err(js_error)?;
	let result = receiver.await.map_err(|_| Error::Cancelled)?;
	drop((reading, error));
	drop(sensor);
	result
}

/// Detects the experimental `AmbientLightSensor` API in a secure context.
pub fn light_available() -> Result<bool, Error> {
	Ok(property(window()?.as_ref(), "AmbientLightSensor")?.is_function())
}
/// Acquires lux with a deadline. Cancellation stops hardware and removes
/// listeners.
pub async fn next_light(
	interval: Interval,
	timeout: crate::Timeout,
) -> Result<crate::LightReading, crate::SensorError> {
	let _reservation = Reservation::acquire_bit(8)?;
	let constructor = property(window()?.as_ref(), "AmbientLightSensor")?
		.dyn_into::<Function>()
		.map_err(|_| Error::Unavailable)?;
	let sample = generic_next(constructor, interval, |source| {
		let timestamp = number(source, "timestamp")? / 1000.0;
		let lux = number(source, "illuminance")?;
		if timestamp < 0.0 || lux < 0.0 {
			return Err(Error::Native);
		}
		Ok(crate::LightReading { timestamp, lux })
	});
	futures::pin_mut!(sample);
	let timer = gloo_timers::future::TimeoutFuture::new(timeout.milliseconds());
	futures::pin_mut!(timer);
	match futures::future::select(sample, timer).await {
		futures::future::Either::Left((result, _)) => result.map_err(Into::into),
		futures::future::Either::Right(_) => Err(crate::SensorError::TimedOut),
	}
}

#[cfg(test)]
mod light_tests {
	use wasm_bindgen::prelude::wasm_bindgen;
	use wasm_bindgen_test::wasm_bindgen_test;

	use super::*;
	#[wasm_bindgen(inline_js = "
let previousLight, light, stoppedLight;
export function installLight() {
 previousLight = Object.getOwnPropertyDescriptor(window, 'AmbientLightSensor'); stoppedLight = 0;
 class TestLight extends EventTarget { constructor(){ super(); light = this; } start(){} \
	                            stop(){stoppedLight++;} }
 Object.defineProperty(window, 'AmbientLightSensor', {configurable:true, value:TestLight});
}
export function restoreLight(){if(previousLight) \
	                            Object.defineProperty(window,'AmbientLightSensor',previousLight); \
	                            else delete window.AmbientLightSensor;}
export function emitLight(lux){light.timestamp=2500;light.illuminance=lux;light.dispatchEvent(new \
	                            Event('reading'));}
export function lightStops(){return stoppedLight;}
")]
	extern "C" {
		#[wasm_bindgen(js_name = installLight)]
		fn install_light();
		#[wasm_bindgen(js_name = restoreLight)]
		fn restore_light();
		#[wasm_bindgen(js_name = emitLight)]
		fn emit_light(lux: f64);
		#[wasm_bindgen(js_name = lightStops)]
		fn light_stops() -> u32;
	}
	struct Fixture;
	impl Drop for Fixture {
		fn drop(&mut self) {
			restore_light();
		}
	}
	#[wasm_bindgen_test]
	async fn light_zero_is_real_and_timeout_cancel_release_the_sensor() {
		install_light();
		let _fixture = Fixture;
		let interval = Interval::new(20).unwrap();
		let timeout = crate::Timeout::new(20).unwrap();
		assert_eq!(light_available(), Ok(true));
		let mut pending = Box::pin(next_light(interval, timeout));
		assert!(futures::poll!(&mut pending).is_pending());
		assert_eq!(next_light(interval, timeout).await, Err(Error::Busy.into()));
		emit_light(0.0);
		assert_eq!(
			pending.await,
			Ok(crate::LightReading {
				timestamp: 2.5,
				lux: 0.0
			})
		);
		assert_eq!(light_stops(), 1);
		assert_eq!(
			next_light(interval, timeout).await,
			Err(crate::SensorError::TimedOut)
		);
		assert_eq!(light_stops(), 2);
		let mut cancelled = Box::pin(next_light(interval, timeout));
		assert!(futures::poll!(&mut cancelled).is_pending());
		drop(cancelled);
		assert_eq!(light_stops(), 3);
		let mut invalid = Box::pin(next_light(interval, timeout));
		assert!(futures::poll!(&mut invalid).is_pending());
		emit_light(-1.0);
		assert_eq!(invalid.await, Err(Error::Native.into()));
		assert_eq!(light_stops(), 4);
	}
}
