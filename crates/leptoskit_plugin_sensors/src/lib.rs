//! <!-- {=p_sensorsSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_sensors` as a direct dependency and select the
//! `sensors` plugin in `native.plugins`; supported generated hosts discover it
//! from Cargo metadata. Generated Windows, Linux, and terminal hosts currently
//! omit runtime plugins; use explicit registration in a custom host there.
//! Guests enable the `guest` feature and reuse `guest::Sensors`, which
//! streams the accelerometer, gravity-free acceleration, gyroscope,
//! magnetometer, and barometer from one plugin. iOS and Android use real system
//! sensors on the same axes, desktop has no sensor, and the browser uses
//! Generic Sensor or `DeviceMotion` and has no barometer.
//!
//! <!-- {/p_sensorsSummary} -->

use leptoskit_bridge::Error;
pub use leptoskit_sensor::AVAILABLE;
pub use leptoskit_sensor::Interval;
pub use leptoskit_sensor::Kind;
pub use leptoskit_sensor::NEXT;
pub use leptoskit_sensor::PressureReading;
pub use leptoskit_sensor::STREAM_METHODS as METHODS;
pub use leptoskit_sensor::VectorReading;
pub use leptoskit_sensor::WATCH;

/// Native plugin registration name.
pub const NAME: &str = "sensors";

/// Cached guest client for every motion and pressure sensor.
///
/// Each kind has its own one-shot read and its own stream, so an app can
/// watch the gyroscope while reading the barometer.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use leptoskit_bridge::guest::Client;
	use leptoskit_sensor::Sample;
	pub use leptoskit_sensor::guest::Watch;

	use super::*;

	/// Resolved `sensors` plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Sensors(Client);

	impl Sensors {
		/// Resolves the plugin once.
		pub fn new() -> Result<Self, Error> {
			Client::new(NAME).map(Self)
		}

		/// Tests hardware availability, independently of permission status.
		pub async fn available(self, kind: Kind) -> Result<bool, Error> {
			leptoskit_sensor::decode_available(&self.0.call(AVAILABLE, &kind.prefix(&[]))?.await?)
		}

		/// Streams acceleration including gravity, in m/s².
		pub fn accelerometer(self, interval: Interval) -> Watch<VectorReading> {
			Watch::of_kind(self.0, Kind::Accelerometer, interval)
		}

		/// Streams acceleration with gravity removed, in m/s².
		pub fn user_accelerometer(self, interval: Interval) -> Watch<VectorReading> {
			Watch::of_kind(self.0, Kind::UserAccelerometer, interval)
		}

		/// Streams rotation rate, in radians per second.
		pub fn gyroscope(self, interval: Interval) -> Watch<VectorReading> {
			Watch::of_kind(self.0, Kind::Gyroscope, interval)
		}

		/// Streams the magnetic field, in microteslas.
		pub fn magnetometer(self, interval: Interval) -> Watch<VectorReading> {
			Watch::of_kind(self.0, Kind::Magnetometer, interval)
		}

		/// Streams atmospheric pressure, in hectopascals.
		pub fn barometer(self, interval: Interval) -> Watch<PressureReading> {
			Watch::of_kind(self.0, Kind::Barometer, interval)
		}

		/// Waits for one fresh acceleration sample including gravity.
		pub async fn read_accelerometer(self, interval: Interval) -> Result<VectorReading, Error> {
			self.read(Kind::Accelerometer, interval).await
		}

		/// Waits for one fresh acceleration sample with gravity removed.
		pub async fn read_user_accelerometer(
			self,
			interval: Interval,
		) -> Result<VectorReading, Error> {
			self.read(Kind::UserAccelerometer, interval).await
		}

		/// Waits for one fresh rotation-rate sample.
		pub async fn read_gyroscope(self, interval: Interval) -> Result<VectorReading, Error> {
			self.read(Kind::Gyroscope, interval).await
		}

		/// Waits for one fresh magnetic-field sample.
		pub async fn read_magnetometer(self, interval: Interval) -> Result<VectorReading, Error> {
			self.read(Kind::Magnetometer, interval).await
		}

		/// Waits for one fresh pressure sample.
		pub async fn read_barometer(self, interval: Interval) -> Result<PressureReading, Error> {
			self.read(Kind::Barometer, interval).await
		}

		async fn read<R: Sample>(self, kind: Kind, interval: Interval) -> Result<R, Error> {
			R::decode(&self.0.call(NEXT, &kind.prefix(&interval.encode()))?.await?)
		}
	}
}

/// Desktop support is explicitly unavailable where there is no portable sensor
/// API.
#[cfg(not(target_arch = "wasm32"))]
pub mod desktop {
	use super::*;

	/// Reports the absence of a portable desktop sensor backend.
	pub fn available(_kind: Kind) -> bool {
		false
	}

	/// Returns Unavailable instead of inventing a sensor reading.
	pub fn read(
		_kind: Kind,
		_interval: Interval,
	) -> std::future::Ready<Result<VectorReading, Error>> {
		std::future::ready(Err(Error::Unavailable))
	}
}

/// Browser adapter using Generic Sensor, or `DeviceMotion` for acceleration and
/// rotation. Browsers have no barometer.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use leptoskit_sensor::web as sensors;
	pub use leptoskit_sensor::web::Watch;

	use super::*;

	/// Tests API presence. Hardware and permission are checked during
	/// acquisition.
	pub fn available(kind: Kind) -> Result<bool, Error> {
		sensors::available(kind)
	}

	/// Requests any explicit browser permission. Call from a user gesture.
	pub async fn request_permission(kind: Kind) -> Result<(), Error> {
		sensors::request_permission(kind).await
	}

	/// Acquires one reading of a motion kind. Cancellation stops the sensor and
	/// removes listeners.
	pub async fn read(kind: Kind, interval: Interval) -> Result<VectorReading, Error> {
		sensors::next(kind, interval).await
	}

	/// Streams a motion kind until the returned watch is dropped.
	pub async fn watch(kind: Kind, interval: Interval) -> Result<Watch, Error> {
		sensors::watch(kind, interval).await
	}
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
	use super::*;

	#[test]
	fn the_plugin_streams_every_kind_and_desktop_invents_nothing() {
		assert_eq!(NAME, "sensors");
		assert_eq!(METHODS, &[AVAILABLE, NEXT, WATCH]);

		for kind in Kind::ALL {
			assert!(!desktop::available(kind));
			assert_eq!(
				desktop::read(kind, Interval::NORMAL).into_inner(),
				Err(Error::Unavailable)
			);
			// Every request leads with the kind so one plugin serves them all.
			assert_eq!(
				kind.prefix(&Interval::GAME.encode())[..4],
				kind.code().to_le_bytes()
			);
		}
	}
}
