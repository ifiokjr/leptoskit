//! Demand-driven barometer readings with bounded native acquisition.
//! See the crate readme for units, permission requirements, and platform
//! limits.

use leptoskit_bridge::Error;
pub use leptoskit_sensor::AVAILABLE;
pub use leptoskit_sensor::Interval;
pub use leptoskit_sensor::METHODS;
pub use leptoskit_sensor::NEXT;
pub use leptoskit_sensor::PressureReading as Reading;

/// Native plugin registration name.
pub const NAME: &str = "barometer";

/// Cached guest client. Only one outstanding reading is accepted per plugin.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// Resolved barometer plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Barometer(leptoskit_bridge::guest::Client);
	impl Barometer {
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

/// Browsers currently have no portable pressure sensor API.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;
	/// No browser barometer capability is advertised.
	pub fn available() -> bool {
		false
	}
	/// Returns Unavailable rather than estimating pressure from unrelated data.
	pub fn next(_interval: Interval) -> std::future::Ready<Result<Reading, Error>> {
		std::future::ready(Err(Error::Unavailable))
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
		assert_eq!(NAME, "barometer");
	}
}
