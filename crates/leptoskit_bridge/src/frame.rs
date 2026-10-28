//! The display frame clock: how a guest waits for the next frame.
//!
//! A guest that animates asks for a frame and renders when it arrives, the
//! way a browser runs `requestAnimationFrame` callbacks before painting. The
//! host answers from its display link (`CADisplayLink`, Android's
//! `Choreographer`) and lets the guest render inside the same callback, so a
//! render that fits in a frame reaches the screen on the next vsync.
//!
//! The clock is a reserved plugin on the ordinary bridge rather than a new
//! Wasm import. A bundle that uses it still loads on a host built before it
//! existed, where [`FRAME_PLUGIN`] does not resolve and the guest receives
//! [`Error::Unavailable`](crate::Error::Unavailable) instead of failing to
//! instantiate; code push can therefore ship it to existing installs.

use crate::MethodId;

/// The reserved plugin name the host's frame clock registers.
pub const FRAME_PLUGIN: &str = "leptoskit.frame";

/// Resolves with the next display frame.
pub const NEXT_FRAME: MethodId = MethodId(0);

/// The encoded size of a [`Frame`].
pub const FRAME_BYTES: usize = 16;

/// One display frame, as the host's display link reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
	/// When the frame began, in nanoseconds on the platform's monotonic
	/// display clock.
	pub timestamp_ns: u64,
	/// The display's refresh period, in nanoseconds; zero when unknown.
	pub period_ns: u64,
}

impl Frame {
	/// The reply payload: both fields as little-endian `u64`.
	#[must_use]
	pub fn encode(self) -> [u8; FRAME_BYTES] {
		let mut bytes = [0; FRAME_BYTES];
		bytes[..8].copy_from_slice(&self.timestamp_ns.to_le_bytes());
		bytes[8..].copy_from_slice(&self.period_ns.to_le_bytes());
		bytes
	}

	/// Reads a reply payload, or `None` when it is not [`FRAME_BYTES`] long.
	#[must_use]
	pub fn decode(bytes: &[u8]) -> Option<Self> {
		let bytes: &[u8; FRAME_BYTES] = bytes.try_into().ok()?;
		let mut timestamp = [0; 8];
		let mut period = [0; 8];
		timestamp.copy_from_slice(&bytes[..8]);
		period.copy_from_slice(&bytes[8..]);
		Some(Self {
			timestamp_ns: u64::from_le_bytes(timestamp),
			period_ns: u64::from_le_bytes(period),
		})
	}
}

#[cfg(test)]
mod tests {
	use super::Frame;

	#[test]
	fn a_frame_round_trips_through_its_payload() {
		let frame = Frame {
			timestamp_ns: 1_234_567_890,
			period_ns: 8_333_333,
		};
		assert_eq!(Frame::decode(&frame.encode()), Some(frame));
		assert_eq!(Frame::decode(&[0; 15]), None);
	}
}
