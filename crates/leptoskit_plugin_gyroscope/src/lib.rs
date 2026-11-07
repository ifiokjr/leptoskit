//! <!-- {=p_gyroscopeSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_gyroscope` as a direct dependency and select the
//! `gyroscope` plugin in `native.plugins`; supported generated hosts discover
//! it from Cargo metadata. Generated Windows, Linux, and terminal hosts
//! currently omit runtime plugins; use explicit registration in a custom host
//! there. Guests enable the `guest` feature and reuse `guest::Gyroscope` rather
//! than resolving the plugin on every call. iOS and Android use real system
//! sensors, desktop has no sensor, and the browser converts `DeviceMotion`
//! degrees to radians.
//!
//! <!-- {/p_gyroscopeSummary} -->

use leptoskit_bridge::Error;
pub use leptoskit_sensor::AVAILABLE;
pub use leptoskit_sensor::Interval;
pub use leptoskit_sensor::NEXT;
pub use leptoskit_sensor::STREAM_METHODS as METHODS;
pub use leptoskit_sensor::VectorReading as Reading;
pub use leptoskit_sensor::WATCH;

/// Native plugin registration name.
pub const NAME: &str = "gyroscope";

/// Cached guest client. Only one outstanding reading is accepted per plugin.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// Resolved gyroscope plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Gyroscope(leptoskit_bridge::guest::Client);
	impl Gyroscope {
		/// Resolves the plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Tests hardware availability, independently of permission status.
		pub async fn available(self) -> Result<bool, Error> {
			leptoskit_sensor::decode_available(&self.0.call(AVAILABLE, &[])?.await?)
		}

		/// Waits for one fresh sample. Dropping this call cancels hardware
		/// acquisition. The sampling interval is a hint; this is not a
		/// lossless recording stream.
		pub async fn next(self, interval: Interval) -> Result<Reading, Error> {
			Reading::decode(&self.0.call(NEXT, &interval.encode())?.await?)
		}

		/// Streams samples until the returned watch is dropped. Await
		/// [`Watch::next`](leptoskit_sensor::guest::Watch::next) in a loop.
		pub fn watch(self, interval: Interval) -> leptoskit_sensor::guest::Watch<Reading> {
			leptoskit_sensor::guest::Watch::new(self.0, interval)
		}
	}
}

/// Desktop support is explicitly unavailable where there is no portable sensor
/// API.
#[cfg(not(target_arch = "wasm32"))]
pub mod desktop {
	use super::*;
	/// Reports the absence of a portable desktop sensor backend.
	pub fn available() -> bool {
		false
	}
	/// Returns Unavailable instead of inventing a sensor reading.
	pub fn next(_interval: Interval) -> std::future::Ready<Result<Reading, Error>> {
		std::future::ready(Err(Error::Unavailable))
	}
}

/// Browser adapter using Generic Sensor, or `DeviceMotion` for
/// acceleration/rotation.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use leptoskit_sensor::web::Kind;
	use leptoskit_sensor::web::{self as sensors};

	use super::*;
	/// Tests API presence. Hardware and permission are checked during
	/// acquisition.
	pub fn available() -> Result<bool, Error> {
		sensors::available(Kind::Gyroscope)
	}
	/// Requests any explicit browser permission. Call from a user gesture.
	pub async fn request_permission() -> Result<(), Error> {
		sensors::request_permission(Kind::Gyroscope).await
	}
	/// Acquires one reading. Cancellation stops the sensor and removes
	/// listeners.
	pub async fn next(interval: Interval) -> Result<Reading, Error> {
		sensors::next(Kind::Gyroscope, interval).await
	}
	/// Streams samples until the returned watch is dropped. Call from a user
	/// gesture where the browser needs permission.
	pub async fn watch(interval: Interval) -> Result<sensors::Watch, Error> {
		sensors::watch(Kind::Gyroscope, interval).await
	}
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
	use super::*;

	#[test]
	fn desktop_reports_unavailable_instead_of_inventing_readings() {
		assert!(!desktop::available());
		assert_eq!(
			desktop::next(Interval::new(100).unwrap()).into_inner(),
			Err(Error::Unavailable)
		);
		assert_eq!(NAME, "gyroscope");
	}
}
