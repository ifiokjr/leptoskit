//! A monotonic clock for the guest.
//!
//! WebAssembly guests have no clock: `std::time::Instant` is unavailable on
//! `wasm32-unknown-unknown`, and the interpreter imports no time function.
//! This plugin reads [`Instant`](std::time::Instant) on the host. It is a
//! quick Rust handler, so the host answers it in the same pump that received
//! the request, but each reading is still an asynchronous bridge round trip.

use leptoskit_bridge::MethodId;

/// The runtime name listed in `leptoskit.toml`.
pub const NAME: &str = "arena-clock";
/// Nanoseconds since the plugin was created, as a little-endian `u64`.
pub const NOW: MethodId = MethodId(0);
/// Supported methods.
pub const METHODS: &[MethodId] = &[NOW];

/// The host side: an `Instant` origin taken when the host registers the plugin.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct Clock {
	origin: std::time::Instant,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for Clock {
	fn default() -> Self {
		Self {
			origin: std::time::Instant::now(),
		}
	}
}

#[cfg(not(target_arch = "wasm32"))]
impl leptoskit_bridge::Plugin for Clock {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(
		&mut self,
		method: MethodId,
		payload: leptoskit_bridge::Bytes,
	) -> Result<leptoskit_bridge::Bytes, leptoskit_bridge::Error> {
		if method != NOW || !payload.is_empty() {
			return Err(leptoskit_bridge::Error::InvalidArgument);
		}

		let elapsed = u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
		Ok(leptoskit_bridge::Bytes::copy_from_slice(
			&elapsed.to_le_bytes(),
		))
	}
}

/// The guest client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use leptoskit_bridge::Error;
	use leptoskit_bridge::guest::Client;

	use super::*;

	/// A resolved handle to the host clock.
	#[derive(Debug, Clone, Copy)]
	pub struct Clock(Client);

	impl Clock {
		/// Resolves the plugin the host registered.
		pub fn new() -> Result<Self, Error> {
			Client::new(NAME).map(Self)
		}

		/// Nanoseconds on the host's monotonic clock.
		pub async fn now_ns(self) -> Result<u64, Error> {
			let reply = self.0.call(NOW, &[])?.await?;
			let bytes: [u8; 8] = reply
				.as_ref()
				.try_into()
				.map_err(|_| Error::InvalidArgument)?;
			Ok(u64::from_le_bytes(bytes))
		}
	}
}
