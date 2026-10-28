//! The arena's native plugin contract and its guest client.
//!
//! The native side lives in `apple/ArenaPlugin.swift` and
//! `android/ArenaPlugin.kt`. Every payload is raw bytes: integers are
//! little-endian, strings are UTF-8, and the echo object is JSON.

use leptoskit_bridge::MethodId;

/// The runtime name listed in `leptoskit.toml`.
pub const NAME: &str = "arena";

/// Returns the launch scenario as UTF-8, empty when there is none.
pub const SCENARIO: MethodId = MethodId(0);
/// Writes each `\n`-separated UTF-8 line to the system log.
pub const REPORT: MethodId = MethodId(1);
/// Resolves at the next display frame with its timestamp and refresh period,
/// both little-endian `u64` nanoseconds.
pub const NEXT_FRAME: MethodId = MethodId(2);
/// `echo()`: no payload, no reply.
pub const ECHO_NONE: MethodId = MethodId(10);
/// `addOne(i32) -> i32`.
pub const ADD_ONE: MethodId = MethodId(11);
/// Echoes a UTF-8 string.
pub const ECHO_STRING: MethodId = MethodId(12);
/// Echoes raw bytes.
pub const ECHO_BYTES: MethodId = MethodId(13);
/// Decodes a JSON object natively and returns it re-encoded.
pub const ECHO_OBJECT: MethodId = MethodId(14);

/// One display frame, as the native frame clock reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
	/// The frame's timestamp on the platform's display clock.
	pub timestamp_ns: u64,
	/// The display's refresh period.
	pub period_ns: u64,
}

/// The guest client for the native plugin.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use leptoskit_bridge::Bytes;
	use leptoskit_bridge::Error;
	use leptoskit_bridge::guest::Client;

	use super::*;

	/// A resolved handle to the host's `arena` plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Arena(Client);

	impl Arena {
		/// Resolves the plugin the host registered.
		pub fn new() -> Result<Self, Error> {
			Client::new(NAME).map(Self)
		}

		/// The scenario the platform launched the app with, if any.
		pub async fn scenario(self) -> Result<Option<String>, Error> {
			let reply = self.0.call(SCENARIO, &[])?.await?;
			let scenario = String::from_utf8(reply.to_vec()).map_err(|_| Error::InvalidArgument)?;
			Ok((!scenario.is_empty()).then_some(scenario))
		}

		/// Writes lines to the platform's system log.
		pub async fn report(self, lines: &str) -> Result<(), Error> {
			self.0.call(REPORT, lines.as_bytes())?.await.map(drop)
		}

		/// Waits for the next display frame.
		pub async fn next_frame(self) -> Result<Frame, Error> {
			let reply = self.0.call(NEXT_FRAME, &[])?.await?;
			let bytes: [u8; 16] = reply
				.as_ref()
				.try_into()
				.map_err(|_| Error::InvalidArgument)?;
			let mut timestamp = [0; 8];
			let mut period = [0; 8];
			timestamp.copy_from_slice(&bytes[..8]);
			period.copy_from_slice(&bytes[8..]);
			Ok(Frame {
				timestamp_ns: u64::from_le_bytes(timestamp),
				period_ns: u64::from_le_bytes(period),
			})
		}

		/// `echo()`.
		pub async fn echo(self) -> Result<(), Error> {
			let reply = self.0.call(ECHO_NONE, &[])?.await?;
			if reply.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}

		/// `addOne(value)`.
		pub async fn add_one(self, value: i32) -> Result<i32, Error> {
			let reply = self.0.call(ADD_ONE, &value.to_le_bytes())?.await?;
			let bytes: [u8; 4] = reply
				.as_ref()
				.try_into()
				.map_err(|_| Error::InvalidArgument)?;
			Ok(i32::from_le_bytes(bytes))
		}

		/// Echoes a string.
		pub async fn echo_string(self, value: &str) -> Result<String, Error> {
			let reply = self.0.call(ECHO_STRING, value.as_bytes())?.await?;
			String::from_utf8(reply.to_vec()).map_err(|_| Error::InvalidArgument)
		}

		/// Echoes bytes, which cross the bridge as binary.
		pub async fn echo_bytes(self, value: &[u8]) -> Result<Bytes, Error> {
			self.0.call(ECHO_BYTES, value)?.await
		}

		/// Echoes an encoded JSON object through the native JSON codec.
		pub async fn echo_object(self, json: &[u8]) -> Result<Bytes, Error> {
			self.0.call(ECHO_OBJECT, json)?.await
		}
	}
}
