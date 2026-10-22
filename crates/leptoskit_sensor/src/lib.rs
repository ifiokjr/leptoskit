//! Shared binary protocol for demand-driven sensor samples.
//!
//! Each request acquires one fresh reading and releases the native
//! subscription. Repeated calls are intentionally not a fixed-rate lossless
//! recording stream. Timestamps are monotonic seconds in the platform's own
//! clock domain, never UTC.

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Query whether the platform exposes this sensor. Empty request, u32 boolean
/// reply.
pub const AVAILABLE: MethodId = MethodId(0);
/// Read one sample. Input is a u32 sampling interval in milliseconds.
pub const NEXT: MethodId = MethodId(1);
/// Sensor operations available to the native bridge.
pub const METHODS: &[MethodId] = &[AVAILABLE, NEXT];

/// Minimum requested sampling interval. At most 200 Hz avoids Android's
/// high-rate permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval(u32);
impl Interval {
	/// Accepts 5..=60,000 milliseconds. OS scheduling may produce a slower
	/// sample.
	pub fn new(milliseconds: u32) -> Result<Self, Error> {
		if (5..=60_000).contains(&milliseconds) {
			Ok(Self(milliseconds))
		} else {
			Err(Error::InvalidArgument)
		}
	}

	/// Requested interval in milliseconds.
	#[must_use]
	pub fn milliseconds(self) -> u32 {
		self.0
	}

	/// Exact little-endian request payload.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		self.0.to_le_bytes()
	}

	/// Validates the complete request payload.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		Self::new(u32::from_le_bytes(
			payload.try_into().map_err(|_| Error::InvalidArgument)?,
		))
	}
}

/// A sample in the device's native, screen-independent coordinate frame.
/// Accelerometers include gravity (m/s²), gyroscopes use radians/s, and
/// magnetometers use microteslas. Platform axis conventions remain unchanged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorReading {
	/// Monotonic platform timestamp in seconds; not comparable across devices
	/// or APIs.
	pub timestamp: f64,
	/// Native x-axis measurement.
	pub x: f64,
	/// Native y-axis measurement.
	pub y: f64,
	/// Native z-axis measurement.
	pub z: f64,
}
impl VectorReading {
	/// Decodes four little-endian f64 values, rejecting invalid samples.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let [timestamp, x, y, z] = decode_values(payload)?;
		if timestamp < 0.0 {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { timestamp, x, y, z })
	}

	/// Validates and encodes a platform sample without allocation.
	pub fn encode(self) -> Result<[u8; 32], Error> {
		let values = [self.timestamp, self.x, self.y, self.z];
		if self.timestamp < 0.0 || values.iter().any(|v| !v.is_finite()) {
			return Err(Error::InvalidArgument);
		}
		let mut bytes = [0; 32];
		for (slot, value) in bytes.chunks_exact_mut(8).zip(values) {
			slot.copy_from_slice(&value.to_le_bytes());
		}
		Ok(bytes)
	}
}

/// Absolute atmospheric pressure. No invented relative altitude is returned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PressureReading {
	/// Monotonic platform timestamp in seconds.
	pub timestamp: f64,
	/// Atmospheric pressure in hectopascals (millibars).
	pub hectopascals: f64,
}
impl PressureReading {
	/// Decodes two finite little-endian f64 values and requires positive
	/// pressure.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let [timestamp, hectopascals] = decode_values(payload)?;
		if timestamp < 0.0 || hectopascals <= 0.0 {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			timestamp,
			hectopascals,
		})
	}

	/// Validates and encodes a platform sample without allocation.
	pub fn encode(self) -> Result<[u8; 16], Error> {
		if !self.timestamp.is_finite()
			|| self.timestamp < 0.0
			|| !self.hectopascals.is_finite()
			|| self.hectopascals <= 0.0
		{
			return Err(Error::InvalidArgument);
		}
		let mut bytes = [0; 16];
		bytes[..8].copy_from_slice(&self.timestamp.to_le_bytes());
		bytes[8..].copy_from_slice(&self.hectopascals.to_le_bytes());
		Ok(bytes)
	}
}

fn decode_values<const N: usize>(payload: &[u8]) -> Result<[f64; N], Error> {
	if payload.len() != N * 8 {
		return Err(Error::InvalidArgument);
	}
	let mut values = [0.0; N];
	for (value, bytes) in values.iter_mut().zip(payload.chunks_exact(8)) {
		*value = f64::from_le_bytes(bytes.try_into().map_err(|_| Error::InvalidArgument)?);
		if !value.is_finite() {
			return Err(Error::InvalidArgument);
		}
	}
	Ok(values)
}

/// Validates a native availability response.
pub fn decode_available(payload: &[u8]) -> Result<bool, Error> {
	match payload {
		[0, 0, 0, 0] => Ok(false),
		[1, 0, 0, 0] => Ok(true),
		_ => Err(Error::InvalidArgument),
	}
}

/// Browser sensor implementations. Permission prompts must start in a user
/// gesture.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn readings_reject_negative_clocks_and_nonpositive_pressure() {
		let vector = |timestamp: f64| {
			[timestamp, 1.0, 2.0, 3.0]
				.iter()
				.flat_map(|value| value.to_le_bytes())
				.collect::<Vec<_>>()
		};
		assert!(VectorReading::decode(&vector(0.0)).is_ok());
		assert_eq!(
			VectorReading::decode(&vector(-1.0)),
			Err(Error::InvalidArgument)
		);
		let pressure = |timestamp: f64, hectopascals: f64| {
			[timestamp, hectopascals]
				.iter()
				.flat_map(|value| value.to_le_bytes())
				.collect::<Vec<_>>()
		};
		assert!(PressureReading::decode(&pressure(0.0, 1013.25)).is_ok());
		for (timestamp, hectopascals) in [(-1.0, 1013.25), (0.0, 0.0), (0.0, -5.0)] {
			assert_eq!(
				PressureReading::decode(&pressure(timestamp, hectopascals)),
				Err(Error::InvalidArgument)
			);
		}
	}
	#[test]
	fn intervals_are_bounded_and_exact() {
		assert_eq!(Interval::new(250).unwrap().milliseconds(), 250);
		for value in [5, 16, 60_000] {
			let interval = Interval::new(value).unwrap();
			assert_eq!(Interval::decode(&interval.encode()), Ok(interval));
		}
		for value in [0, 4, 60_001, u32::MAX] {
			assert_eq!(Interval::new(value), Err(Error::InvalidArgument));
		}
		assert_eq!(
			Interval::decode(&[5, 0, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn vector_wire_preserves_sign_units_and_clock() {
		let sample = VectorReading {
			timestamp: 1.25,
			x: -9.80665,
			y: 0.0,
			z: 3.5,
		};
		let bytes = sample.encode().unwrap();
		assert_eq!(&bytes[..8], &1.25_f64.to_le_bytes());
		assert_eq!(VectorReading::decode(&bytes), Ok(sample));
		assert_eq!(
			VectorReading::decode(&bytes[..31]),
			Err(Error::InvalidArgument)
		);
		for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
			assert_eq!(
				VectorReading {
					x: invalid,
					..sample
				}
				.encode(),
				Err(Error::InvalidArgument)
			);
			let mut invalid_bytes = bytes;
			invalid_bytes[8..16].copy_from_slice(&invalid.to_le_bytes());
			assert_eq!(
				VectorReading::decode(&invalid_bytes),
				Err(Error::InvalidArgument)
			);
		}
		assert_eq!(
			VectorReading {
				timestamp: -1.0,
				..sample
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn pressure_rejects_unknown_instead_of_fabricating_zero() {
		let sample = PressureReading {
			timestamp: 25.0,
			hectopascals: 1013.25,
		};
		assert_eq!(
			PressureReading::decode(&sample.encode().unwrap()),
			Ok(sample)
		);
		for pressure in [0.0, -1.0, f64::NAN, f64::INFINITY] {
			assert_eq!(
				PressureReading {
					hectopascals: pressure,
					..sample
				}
				.encode(),
				Err(Error::InvalidArgument)
			);
		}
		assert_eq!(decode_available(&[2, 0, 0, 0]), Err(Error::InvalidArgument));
		assert_eq!(decode_available(&[1, 0, 0, 0]), Ok(true));
	}
}
