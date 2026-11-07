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
pub use crate::Kind;
use crate::VectorReading;

/// The Generic Sensor constructor for a kind; browsers have no pressure one.
fn constructor(kind: Kind) -> Option<&'static str> {
	match kind {
		Kind::Accelerometer => Some("Accelerometer"),
		Kind::UserAccelerometer => Some("LinearAccelerationSensor"),
		Kind::Gyroscope => Some("Gyroscope"),
		Kind::Magnetometer => Some("Magnetometer"),
		Kind::Barometer => None,
	}
}

/// Reservation bit for one-shot reads of a kind.
fn read_bit(kind: Kind) -> u32 {
	1 << kind.code()
}

/// Reservation bit for a kind's stream, separate from its one-shot reads.
fn watch_bit(kind: Kind) -> u32 {
	1 << (kind.code() + WATCH_BITS_OFFSET)
}

/// Where stream reservation bits start, past every one-shot bit.
const WATCH_BITS_OFFSET: u32 = 8;

/// Whether `DeviceMotion` can stand in for a missing Generic Sensor.
fn device_motion_covers(kind: Kind) -> bool {
	matches!(
		kind,
		Kind::Accelerometer | Kind::UserAccelerometer | Kind::Gyroscope
	)
}

thread_local! { static ACTIVE: Cell<u32> = const { Cell::new(0) }; }

/// Reservation bit for the ambient light sensor, which is not an axis kind.
const LIGHT_RESERVATION: u32 = 1 << 16;
struct Reservation(u32);
impl Reservation {
	fn acquire(kind: Kind) -> Result<Self, Error> {
		Self::acquire_bit(read_bit(kind))
	}

	fn acquire_bit(bit: u32) -> Result<Self, Error> {
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
	let window = window()?;

	let Some(name) = constructor(kind) else {
		return Ok(None);
	};

	Ok(property(window.as_ref(), name)?.dyn_into::<Function>().ok())
}

/// Reports API presence, not permission or the presence of functioning
/// hardware. `next` can still fail with Unavailable or `PermissionDenied`.
pub fn available(kind: Kind) -> Result<bool, Error> {
	if generic_constructor(kind)?.is_some() {
		return Ok(true);
	}

	Ok(device_motion_covers(kind)
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

	if !device_motion_covers(kind) {
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
impl Sensor {
	/// Constructs the platform sensor with the requested rate.
	fn construct(constructor: &Function, interval: Interval) -> Result<Self, Error> {
		let options = js_sys::Object::new();
		Reflect::set(
			&options,
			&JsValue::from_str("frequency"),
			&JsValue::from_f64(1000.0 / f64::from(interval.milliseconds())),
		)
		.map_err(js_error)?;
		let arguments = js_sys::Array::new();
		arguments.push(&options);
		let value = Reflect::construct(constructor, &arguments).map_err(js_error)?;

		Ok(Self {
			stop: property(&value, "stop")?
				.dyn_into::<Function>()
				.map_err(|_| Error::Native)?,
			value,
		})
	}

	/// The event target that reports readings and errors.
	fn target(&self) -> Result<web_sys::EventTarget, Error> {
		self.value
			.clone()
			.dyn_into::<web_sys::EventTarget>()
			.map_err(|_| Error::Native)
	}

	/// Starts delivery at the requested rate.
	fn start(&self) -> Result<(), Error> {
		property(&self.value, "start")?
			.dyn_into::<Function>()
			.map_err(|_| Error::Native)?
			.call0(&self.value)
			.map_err(js_error)?;

		Ok(())
	}
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
		return generic_next(constructor, interval, parse_generic).await;
	}

	if !device_motion_covers(kind) {
		return Err(Error::Unavailable);
	}

	if !available(kind)? {
		return Err(Error::Unavailable);
	}

	next_device_motion(kind).await
}

/// Reads the generic sensor's millisecond timestamp and three axes.
fn parse_generic(source: &JsValue) -> Result<VectorReading, Error> {
	validate(VectorReading {
		timestamp: number(source, "timestamp")? / 1000.0,
		x: number(source, "x")?,
		y: number(source, "y")?,
		z: number(source, "z")?,
	})
}

/// Waits for one `devicemotion` event; the browser owns the sample rate.
async fn next_device_motion(kind: Kind) -> Result<VectorReading, Error> {
	request_permission(kind).await?;
	let (sender, receiver) = oneshot::channel();
	let sender = Rc::new(RefCell::new(Some(sender)));
	let window = window()?;

	let listener = EventListener::new(&window, "devicemotion", move |event| {
		if let Some(sender) = sender.borrow_mut().take() {
			let _ = sender.send(parse_device_motion(kind, event));
		}
	});
	let result = receiver.await.map_err(|_| Error::Cancelled)?;

	drop(listener);
	result
}

/// `DeviceMotion` reports m/s² for acceleration and degrees per second for
/// rotation, both against a millisecond clock.
fn parse_device_motion(kind: Kind, event: &web_sys::Event) -> Result<VectorReading, Error> {
	let (x, y, z) = match kind {
		Kind::Accelerometer => acceleration(event, "accelerationIncludingGravity")?,
		Kind::UserAccelerometer => acceleration(event, "acceleration")?,
		_ => rotation(event)?,
	};

	validate(VectorReading {
		timestamp: event.time_stamp() / 1000.0,
		x,
		y,
		z,
	})
}

/// `accelerationIncludingGravity` and `acceleration` report m/s² on each axis.
/// Devices without a gyroscope leave `acceleration` null, which reads as
/// unavailable rather than as zero.
fn acceleration(event: &web_sys::Event, name: &str) -> Result<(f64, f64, f64), Error> {
	let values = property(event.as_ref(), name)?;

	Ok((
		number(&values, "x")?,
		number(&values, "y")?,
		number(&values, "z")?,
	))
}

/// `rotationRate` beta/gamma/alpha are degrees per second around x/y/z.
fn rotation(event: &web_sys::Event) -> Result<(f64, f64, f64), Error> {
	let values = property(event.as_ref(), "rotationRate")?;

	Ok((
		number(&values, "beta")?.to_radians(),
		number(&values, "gamma")?.to_radians(),
		number(&values, "alpha")?.to_radians(),
	))
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

	/// A sampling rate the emulated browser sensor accepts.
	const SAMPLE_INTERVAL_MS: u32 = 16;

	#[wasm_bindgen_test]
	async fn browser_sampling_releases_hardware_on_success_denial_and_cancellation() {
		install_sensor();
		let _installed = InstalledSensor;
		let interval = Interval::new(SAMPLE_INTERVAL_MS).unwrap();
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
	let sensor = Sensor::construct(&constructor, interval)?;
	let target = sensor.target()?;
	let (sender, receiver) = oneshot::channel();
	let sender = Rc::new(RefCell::new(Some(sender)));
	let reading = reading_listener(&target, &sensor.value, Rc::clone(&sender), parse);
	let error = error_listener(&target, sender);

	sensor.start()?;
	let result = receiver.await.map_err(|_| Error::Cancelled)?;

	drop((reading, error));
	drop(sensor);
	result
}

/// The one answer a one-shot wait is still owed, shared by its listeners.
type Pending<T> = Rc<RefCell<Option<oneshot::Sender<Result<T, Error>>>>>;

/// Resolves the wait with the first sample the platform reports.
fn reading_listener<T: 'static>(
	target: &web_sys::EventTarget,
	value: &JsValue,
	sender: Pending<T>,
	parse: fn(&JsValue) -> Result<T, Error>,
) -> EventListener {
	let source = value.clone();

	EventListener::new(target, "reading", move |_| {
		if let Some(sender) = sender.borrow_mut().take() {
			let _ = sender.send(parse(&source));
		}
	})
}

/// Resolves the wait with the error the platform reports, when it does.
fn error_listener<T: 'static>(target: &web_sys::EventTarget, sender: Pending<T>) -> EventListener {
	EventListener::new(target, "error", move |event| {
		if let Some(sender) = sender.borrow_mut().take() {
			let error = property(event.as_ref(), "error").map_or_else(|error| error, js_error);
			let _ = sender.send(Err(error));
		}
	})
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
	let _reservation = Reservation::acquire_bit(LIGHT_RESERVATION)?;
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

	/// A sampling rate and deadline the emulated light sensor accepts.
	const LIGHT_INTERVAL_MS: u32 = 20;
	const LIGHT_TIMEOUT_MS: u32 = 20;

	#[wasm_bindgen_test]
	async fn light_zero_is_real_and_timeout_cancel_release_the_sensor() {
		install_light();
		let _fixture = Fixture;
		let interval = Interval::new(LIGHT_INTERVAL_MS).unwrap();
		let timeout = crate::Timeout::new(LIGHT_TIMEOUT_MS).unwrap();
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

/// Samples waiting for a browser [`Watch`], bounded like the native ring.
#[derive(Default)]
struct Queue {
	samples: std::collections::VecDeque<VectorReading>,
	dropped: u64,
	error: Option<Error>,
	waker: Option<std::task::Waker>,
}

impl Queue {
	/// Keeps the newest [`crate::stream::MAX_BATCH`] samples, counting the
	/// rest.
	fn push(&mut self, sample: Result<VectorReading, Error>) {
		match sample {
			Ok(sample) => {
				if self.samples.len() == crate::stream::MAX_BATCH {
					self.samples.pop_front();
					self.dropped += 1;
				}

				self.samples.push_back(sample);
			}
			Err(error) => {
				self.error.get_or_insert(error);
			}
		}

		if let Some(waker) = self.waker.take() {
			waker.wake();
		}
	}
}

/// A running browser sensor and the samples it has reported.
///
/// Dropping the watch stops the sensor and removes its listeners. One watch
/// per kind can run at a time, alongside one-shot reads.
pub struct Watch {
	queue: Rc<RefCell<Queue>>,
	listeners: Vec<EventListener>,
	sensor: Option<Sensor>,
	_reservation: Reservation,
}

impl std::fmt::Debug for Watch {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Watch")
			.field("buffered", &self.queue.borrow().samples.len())
			.field("dropped", &self.queue.borrow().dropped)
			.finish_non_exhaustive()
	}
}

impl Watch {
	/// Waits for the next sample, oldest first. A sensor error ends the watch:
	/// every later call returns the same error.
	pub async fn next(&mut self) -> Result<VectorReading, Error> {
		std::future::poll_fn(|context| {
			let mut queue = self.queue.borrow_mut();

			if let Some(sample) = queue.samples.pop_front() {
				return std::task::Poll::Ready(Ok(sample));
			}

			if let Some(error) = queue.error {
				return std::task::Poll::Ready(Err(error));
			}

			queue.waker = Some(context.waker().clone());
			std::task::Poll::Pending
		})
		.await
	}

	/// Waits for at least one sample, then returns every one that has arrived.
	pub async fn next_batch(&mut self) -> Result<Vec<VectorReading>, Error> {
		let first = self.next().await?;
		let mut samples = vec![first];
		samples.extend(self.queue.borrow_mut().samples.drain(..));
		Ok(samples)
	}

	/// Samples discarded because they arrived faster than this watch read them.
	#[must_use]
	pub fn dropped(&self) -> u64 {
		self.queue.borrow().dropped
	}
}

impl Drop for Watch {
	fn drop(&mut self) {
		// Listeners go first so a stopping sensor cannot report into a queue
		// nobody reads.
		self.listeners.clear();
		self.sensor.take();
	}
}

/// Starts a continuous series of samples. Generic Sensor uses the requested
/// rate; `DeviceMotion` delivers at the browser's own rate and asks for
/// Safari's permission first, so call this from a click or tap handler there.
pub async fn watch(kind: Kind, interval: Interval) -> Result<Watch, Error> {
	let reservation = Reservation::acquire_bit(watch_bit(kind))?;
	let queue = Rc::new(RefCell::new(Queue::default()));

	if let Some(constructor) = generic_constructor(kind)? {
		let sensor = Sensor::construct(&constructor, interval)?;
		let target = sensor.target()?;
		let source = sensor.value.clone();
		let readings = Rc::clone(&queue);
		let errors = Rc::clone(&queue);
		let listeners = vec![
			EventListener::new(&target, "reading", move |_| {
				readings.borrow_mut().push(parse_generic(&source));
			}),
			EventListener::new(&target, "error", move |event| {
				let error = property(event.as_ref(), "error").map_or_else(|error| error, js_error);
				errors.borrow_mut().push(Err(error));
			}),
		];
		sensor.start()?;

		return Ok(Watch {
			queue,
			listeners,
			sensor: Some(sensor),
			_reservation: reservation,
		});
	}

	if !available(kind)? {
		return Err(Error::Unavailable);
	}

	request_permission(kind).await?;
	let readings = Rc::clone(&queue);
	let window = window()?;
	let listener = EventListener::new(&window, "devicemotion", move |event| {
		readings.borrow_mut().push(parse_device_motion(kind, event));
	});

	Ok(Watch {
		queue,
		listeners: vec![listener],
		sensor: None,
		_reservation: reservation,
	})
}

#[cfg(test)]
mod watch_tests {
	use wasm_bindgen::prelude::wasm_bindgen;
	use wasm_bindgen_test::wasm_bindgen_test;

	use super::*;

	#[wasm_bindgen(inline_js = "
let saved = {}, sensors = {}, counts = {};
export function installNamed(name) {
  saved[name] = Object.getOwnPropertyDescriptor(window, name);
  counts[name] = [0, 0];
  class TestSensor extends EventTarget {
    constructor(options) { super(); this.frequency = options.frequency; sensors[name] = this; }
    start() { counts[name][0]++; }
    stop() { counts[name][1]++; }
  }
  Object.defineProperty(window, name, {configurable: true, value: TestSensor});
}
export function restoreNamed(name) {
  if (saved[name]) Object.defineProperty(window, name, saved[name]);
  else delete window[name];
}
export function namedCounts(name) { return counts[name]; }
export function namedFrequency(name) { return sensors[name].frequency; }
export function emitNamed(name, milliseconds, x) {
  const sensor = sensors[name];
  sensor.timestamp = milliseconds; sensor.x = x; sensor.y = 0; sensor.z = 9.80665;
  sensor.dispatchEvent(new Event('reading'));
}
export function failNamed(name) {
  const event = new Event('error');
  Object.defineProperty(event, 'error', {value: new DOMException('Gone', 'NotReadableError')});
  sensors[name].dispatchEvent(event);
}
")]
	extern "C" {
		#[wasm_bindgen(js_name = installNamed)]
		fn install_named(name: &str);
		#[wasm_bindgen(js_name = restoreNamed)]
		fn restore_named(name: &str);
		#[wasm_bindgen(js_name = namedCounts)]
		fn named_counts(name: &str) -> js_sys::Array;
		#[wasm_bindgen(js_name = namedFrequency)]
		fn named_frequency(name: &str) -> f64;
		#[wasm_bindgen(js_name = emitNamed)]
		fn emit_named(name: &str, milliseconds: f64, x: f64);
		#[wasm_bindgen(js_name = failNamed)]
		fn fail_named(name: &str);
	}

	struct Installed(&'static str);
	impl Installed {
		fn new(name: &'static str) -> Self {
			install_named(name);
			Self(name)
		}

		fn counts(&self) -> Vec<JsValue> {
			named_counts(self.0).to_vec()
		}
	}
	impl Drop for Installed {
		fn drop(&mut self) {
			restore_named(self.0);
		}
	}

	fn counts(started: f64, stopped: f64) -> Vec<JsValue> {
		vec![JsValue::from_f64(started), JsValue::from_f64(stopped)]
	}

	#[wasm_bindgen_test]
	async fn a_watch_streams_in_order_bounds_its_queue_and_stops_on_drop() {
		let sensor = Installed::new("Accelerometer");
		let mut watch = watch(Kind::Accelerometer, Interval::GAME).await.unwrap();
		assert_eq!(named_frequency("Accelerometer"), 50.0);
		assert_eq!(sensor.counts(), counts(1.0, 0.0));
		// A second stream of the same kind is refused, but a one-shot read is
		// independent of it.
		assert_eq!(
			super::watch(Kind::Accelerometer, Interval::GAME)
				.await
				.unwrap_err(),
			Error::Busy
		);

		for step in 0..3 {
			emit_named("Accelerometer", 1000.0 + f64::from(step), f64::from(step));
		}

		for step in 0..3 {
			let sample = watch.next().await.unwrap();
			assert_eq!(sample.x, f64::from(step));
			assert_eq!(sample.timestamp, (1000.0 + f64::from(step)) / 1000.0);
		}

		// A reader that falls behind keeps the newest samples and learns how
		// many it missed.
		let overflow = crate::stream::MAX_BATCH as u32 + 6;

		for step in 0..overflow {
			emit_named("Accelerometer", 2000.0 + f64::from(step), f64::from(step));
		}

		let batch = watch.next_batch().await.unwrap();
		assert_eq!(batch.len(), crate::stream::MAX_BATCH);
		assert_eq!(batch[0].x, 6.0);
		assert_eq!(watch.dropped(), 6);

		let mut waiting = Box::pin(watch.next());
		assert!(futures::poll!(&mut waiting).is_pending());
		drop(waiting);
		drop(watch);
		assert_eq!(sensor.counts(), counts(1.0, 1.0));
		// The reservation went with it.
		drop(
			super::watch(Kind::Accelerometer, Interval::GAME)
				.await
				.unwrap(),
		);
	}

	#[wasm_bindgen_test]
	async fn user_acceleration_uses_linear_acceleration_and_errors_end_the_watch() {
		let sensor = Installed::new("LinearAccelerationSensor");
		let mut watch = watch(Kind::UserAccelerometer, Interval::UI).await.unwrap();
		emit_named("LinearAccelerationSensor", 5.0, 0.25);
		assert_eq!(watch.next().await.unwrap().x, 0.25);
		fail_named("LinearAccelerationSensor");
		assert_eq!(watch.next().await, Err(Error::Unavailable));
		assert_eq!(watch.next().await, Err(Error::Unavailable));
		drop(watch);
		assert_eq!(sensor.counts(), counts(1.0, 1.0));
	}

	#[wasm_bindgen_test]
	async fn browsers_have_no_barometer() {
		assert_eq!(available(Kind::Barometer), Ok(false));
		assert_eq!(
			watch(Kind::Barometer, Interval::NORMAL).await.unwrap_err(),
			Error::Unavailable
		);
		assert_eq!(
			next(Kind::Barometer, Interval::NORMAL).await,
			Err(Error::Unavailable)
		);
	}
}
