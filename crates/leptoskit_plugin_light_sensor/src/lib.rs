//! Bounded ambient light acquisition, in lux. See README for platform limits.
#[cfg(any(not(target_arch = "wasm32"), feature = "guest"))]
use leptoskit_bridge::Error;
pub use leptoskit_sensor::AVAILABLE;
pub use leptoskit_sensor::Interval;
pub use leptoskit_sensor::LightReading as Reading;
pub use leptoskit_sensor::METHODS;
pub use leptoskit_sensor::NEXT;
pub use leptoskit_sensor::SensorError;
pub use leptoskit_sensor::Timeout;
/// Automatic registration name.
pub const NAME: &str = "light_sensor";
/// Cached native guest connection.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// Resolves once and reuses the native plugin handle.
	#[derive(Debug, Clone, Copy)]
	pub struct LightSensor(leptoskit_bridge::guest::Client);
	impl LightSensor {
		/// Resolves the installed plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Checks hardware presence, not authorization.
		pub async fn available(self) -> Result<bool, Error> {
			leptoskit_sensor::decode_available(&self.0.call(AVAILABLE, &[])?.await?)
		}

		/// One fresh reading; dropping this future cancels acquisition.
		pub async fn next(
			self,
			interval: Interval,
			timeout: Timeout,
		) -> Result<Reading, SensorError> {
			let mut payload = [0; 8];
			payload[..4].copy_from_slice(&interval.encode());
			payload[4..].copy_from_slice(&timeout.milliseconds().to_le_bytes());
			Ok(Reading::decode(leptoskit_sensor::bounded_reply(
				&self.0.call(NEXT, &payload)?.await?,
			)?)?)
		}
	}
}
/// `AmbientLightSensor` is experimental and may be absent or policy blocked.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	pub use leptoskit_sensor::web::light_available as available;
	pub use leptoskit_sensor::web::next_light as next;
}
/// No portable desktop ambient light API is advertised.
#[cfg(not(target_arch = "wasm32"))]
pub mod desktop {
	use super::*;
	/// No desktop backend is currently provided.
	pub fn available() -> bool {
		false
	}
	/// Returns unavailable, never an estimated light level.
	pub fn next(_: Interval, _: Timeout) -> std::future::Ready<Result<Reading, SensorError>> {
		std::future::ready(Err(Error::Unavailable.into()))
	}
}
