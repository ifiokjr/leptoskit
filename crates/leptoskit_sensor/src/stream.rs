//! Continuous sample streams over the demand-driven bridge.
//!
//! A watch call returns every buffered sample newer than the caller's cursor,
//! or waits for the next one. Between calls the native side keeps the sensor
//! running for a short grace period, so a caller that asks again promptly sees
//! an unbroken series. When the caller stops asking, or cancels its pending
//! call, the sensor stops. A bounded native ring keeps the newest samples and
//! counts the ones a slow caller missed instead of growing.

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

use crate::AVAILABLE;
use crate::Interval;
use crate::NEXT;
use crate::PressureReading;
use crate::VectorReading;

/// Stream samples newer than a cursor. Input is a u32 interval in milliseconds
/// and a u64 cursor; the reply is a [`Batch`].
pub const WATCH: MethodId = MethodId(2);
/// Operations of a sensor plugin that also streams.
pub const STREAM_METHODS: &[MethodId] = &[AVAILABLE, NEXT, WATCH];

/// The most samples one watch reply carries, and the native ring's capacity.
pub const MAX_BATCH: usize = 64;

/// Bytes before the samples in a watch reply: cursor, dropped count and count.
const BATCH_HEADER_BYTES: usize = 16;
/// Bytes in a watch request: interval and cursor.
const WATCH_REQUEST_BYTES: usize = 12;
/// Bytes in the sensor kind that prefixes every combined-plugin request.
const KIND_BYTES: usize = 4;

/// The sensors the combined `sensors` plugin multiplexes. The discriminant is
/// the little-endian u32 that prefixes each of its requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Kind {
	/// Acceleration including gravity, in metres per second squared.
	Accelerometer = 0,
	/// Acceleration with gravity removed, in metres per second squared.
	UserAccelerometer = 1,
	/// Rotation rate in radians per second.
	Gyroscope = 2,
	/// Magnetic field in microteslas.
	Magnetometer = 3,
	/// Atmospheric pressure in hectopascals.
	Barometer = 4,
}

impl Kind {
	/// Every kind, in wire order.
	pub const ALL: [Self; 5] = [
		Self::Accelerometer,
		Self::UserAccelerometer,
		Self::Gyroscope,
		Self::Magnetometer,
		Self::Barometer,
	];

	/// The wire code.
	#[must_use]
	pub fn code(self) -> u32 {
		self as u32
	}

	/// Parses a wire code, rejecting kinds this version does not know.
	pub fn from_code(code: u32) -> Result<Self, Error> {
		Self::ALL
			.into_iter()
			.find(|kind| kind.code() == code)
			.ok_or(Error::InvalidArgument)
	}

	/// Bytes in one encoded sample of this kind.
	#[must_use]
	pub fn sample_bytes(self) -> usize {
		match self {
			Self::Barometer => PressureReading::BYTES,
			_ => VectorReading::BYTES,
		}
	}

	/// Splits the kind prefix off a combined-plugin request.
	pub fn split(payload: &[u8]) -> Result<(Self, &[u8]), Error> {
		let (kind, rest) = payload
			.split_first_chunk::<KIND_BYTES>()
			.ok_or(Error::InvalidArgument)?;

		Ok((Self::from_code(u32::from_le_bytes(*kind))?, rest))
	}

	/// Prefixes a request with this kind, for the combined plugin.
	#[must_use]
	pub fn prefix(self, request: &[u8]) -> Vec<u8> {
		let mut bytes = Vec::with_capacity(KIND_BYTES + request.len());
		bytes.extend_from_slice(&self.code().to_le_bytes());
		bytes.extend_from_slice(request);
		bytes
	}
}

/// One watch request: how often to sample, and the last sample already seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchRequest {
	/// Requested sampling interval; the platform may deliver more slowly.
	pub interval: Interval,
	/// Sequence number of the newest sample the caller has, or zero to start
	/// from the next fresh sample.
	pub cursor: u64,
}

impl WatchRequest {
	/// Exact little-endian request payload.
	#[must_use]
	pub fn encode(self) -> [u8; WATCH_REQUEST_BYTES] {
		let mut bytes = [0; WATCH_REQUEST_BYTES];
		bytes[..4].copy_from_slice(&self.interval.encode());
		bytes[4..].copy_from_slice(&self.cursor.to_le_bytes());
		bytes
	}

	/// Validates the complete request payload.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let payload: &[u8; WATCH_REQUEST_BYTES] =
			payload.try_into().map_err(|_| Error::InvalidArgument)?;
		let (interval, cursor) = payload.split_at(4);

		Ok(Self {
			interval: Interval::decode(interval)?,
			cursor: u64::from_le_bytes(cursor.try_into().map_err(|_| Error::InvalidArgument)?),
		})
	}
}

/// A fixed-width sample that can travel in a watch reply.
pub trait Sample: Sized + Copy {
	/// Encoded bytes per sample.
	const BYTES: usize;
	/// Decodes one sample, rejecting invalid values.
	fn decode(payload: &[u8]) -> Result<Self, Error>;
	/// Appends one validated sample.
	fn encode_into(self, bytes: &mut Vec<u8>) -> Result<(), Error>;
}

impl Sample for VectorReading {
	const BYTES: usize = 32;

	fn decode(payload: &[u8]) -> Result<Self, Error> {
		Self::decode(payload)
	}

	fn encode_into(self, bytes: &mut Vec<u8>) -> Result<(), Error> {
		bytes.extend_from_slice(&self.encode()?);
		Ok(())
	}
}

impl Sample for PressureReading {
	const BYTES: usize = 16;

	fn decode(payload: &[u8]) -> Result<Self, Error> {
		Self::decode(payload)
	}

	fn encode_into(self, bytes: &mut Vec<u8>) -> Result<(), Error> {
		bytes.extend_from_slice(&self.encode()?);
		Ok(())
	}
}

/// One watch reply: the samples newer than the request's cursor, oldest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch<R> {
	/// Sequence number of the newest sample here; send it as the next cursor.
	pub cursor: u64,
	/// Samples the native ring overwrote before the caller asked for them.
	pub dropped: u32,
	/// One to [`MAX_BATCH`] samples, oldest first.
	pub samples: Vec<R>,
}

impl<R: Sample> Batch<R> {
	/// Decodes a reply to a request made with cursor `after`.
	///
	/// The layout is a u64 cursor, a u32 dropped count, a u32 sample count,
	/// then that many samples. A reply always carries at least one sample, and
	/// its cursor must advance past `after` by at least the sample count.
	pub fn decode(payload: &[u8], after: u64) -> Result<Self, Error> {
		let (header, body) = payload
			.split_first_chunk::<BATCH_HEADER_BYTES>()
			.ok_or(Error::InvalidArgument)?;
		let (cursor, counts) = header.split_at(8);
		let cursor = u64::from_le_bytes(cursor.try_into().map_err(|_| Error::InvalidArgument)?);
		let (dropped, count) = counts.split_at(4);
		let dropped = u32::from_le_bytes(dropped.try_into().map_err(|_| Error::InvalidArgument)?);
		let count =
			u32::from_le_bytes(count.try_into().map_err(|_| Error::InvalidArgument)?) as usize;

		if !(1..=MAX_BATCH).contains(&count) || body.len() != count * R::BYTES {
			return Err(Error::InvalidArgument);
		}

		if cursor < after.saturating_add(count as u64) {
			return Err(Error::InvalidArgument);
		}

		let samples = body
			.chunks_exact(R::BYTES)
			.map(R::decode)
			.collect::<Result<Vec<_>, _>>()?;

		Ok(Self {
			cursor,
			dropped,
			samples,
		})
	}

	/// Encodes a reply the way native hosts do, for Rust hosts and tests.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		if !(1..=MAX_BATCH).contains(&self.samples.len()) {
			return Err(Error::InvalidArgument);
		}

		let mut bytes = Vec::with_capacity(BATCH_HEADER_BYTES + self.samples.len() * R::BYTES);
		bytes.extend_from_slice(&self.cursor.to_le_bytes());
		bytes.extend_from_slice(&self.dropped.to_le_bytes());
		bytes.extend_from_slice(&(self.samples.len() as u32).to_le_bytes());

		for sample in &self.samples {
			sample.encode_into(&mut bytes)?;
		}

		Ok(bytes)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn vector(timestamp: f64) -> VectorReading {
		VectorReading {
			timestamp,
			x: 0.5,
			y: -9.80665,
			z: 3.0,
		}
	}

	#[test]
	fn kinds_round_trip_and_unknown_codes_are_refused() {
		for kind in Kind::ALL {
			assert_eq!(Kind::from_code(kind.code()), Ok(kind));
			let prefixed = kind.prefix(&[7, 8]);
			assert_eq!(Kind::split(&prefixed), Ok((kind, &[7_u8, 8][..])));
		}

		assert_eq!(Kind::from_code(5), Err(Error::InvalidArgument));
		assert_eq!(Kind::split(&[0, 0, 0]), Err(Error::InvalidArgument));
		assert_eq!(Kind::Barometer.sample_bytes(), 16);
		assert_eq!(Kind::UserAccelerometer.sample_bytes(), 32);
	}

	#[test]
	fn watch_requests_are_exact_and_bounded() {
		let request = WatchRequest {
			interval: Interval::GAME,
			cursor: u64::MAX - 1,
		};
		let bytes = request.encode();
		assert_eq!(&bytes[..4], &20_u32.to_le_bytes());
		assert_eq!(WatchRequest::decode(&bytes), Ok(request));
		assert_eq!(
			WatchRequest::decode(&bytes[..11]),
			Err(Error::InvalidArgument)
		);
		let mut too_fast = bytes;
		too_fast[..4].copy_from_slice(&4_u32.to_le_bytes());
		assert_eq!(WatchRequest::decode(&too_fast), Err(Error::InvalidArgument));
	}

	#[test]
	fn batches_round_trip_and_reject_inconsistent_replies() {
		let batch = Batch {
			cursor: 12,
			dropped: 3,
			samples: vec![vector(1.0), vector(1.02)],
		};
		let bytes = batch.encode().unwrap();
		assert_eq!(bytes.len(), 16 + 2 * 32);
		assert_eq!(
			Batch::<VectorReading>::decode(&bytes, 10),
			Ok(batch.clone())
		);

		// The cursor must advance past the request by every sample returned.
		assert_eq!(
			Batch::<VectorReading>::decode(&bytes, 11),
			Err(Error::InvalidArgument)
		);
		// Truncated, empty and oversized replies are refused.
		assert_eq!(
			Batch::<VectorReading>::decode(&bytes[..bytes.len() - 1], 0),
			Err(Error::InvalidArgument)
		);
		let mut empty = bytes[..16].to_vec();
		empty[12..16].copy_from_slice(&0_u32.to_le_bytes());
		assert_eq!(
			Batch::<VectorReading>::decode(&empty, 0),
			Err(Error::InvalidArgument)
		);
		let mut oversized = bytes.clone();
		oversized[12..16].copy_from_slice(&(MAX_BATCH as u32 + 1).to_le_bytes());
		assert_eq!(
			Batch::<VectorReading>::decode(&oversized, 0),
			Err(Error::InvalidArgument)
		);
		// A sample with an invalid value fails the whole batch.
		let mut invalid = bytes;
		invalid[16 + 8..16 + 16].copy_from_slice(&f64::NAN.to_le_bytes());
		assert_eq!(
			Batch::<VectorReading>::decode(&invalid, 0),
			Err(Error::InvalidArgument)
		);
		assert!(
			Batch::<VectorReading> {
				cursor: 1,
				dropped: 0,
				samples: Vec::new()
			}
			.encode()
			.is_err()
		);
	}

	#[test]
	fn pressure_batches_use_their_own_width() {
		let batch = Batch {
			cursor: 1,
			dropped: 0,
			samples: vec![PressureReading {
				timestamp: 4.0,
				hectopascals: 1013.25,
			}],
		};
		let bytes = batch.encode().unwrap();
		assert_eq!(bytes.len(), 16 + 16);
		assert_eq!(Batch::<PressureReading>::decode(&bytes, 0), Ok(batch));
		assert!(Batch::<VectorReading>::decode(&bytes, 0).is_err());
	}
}
