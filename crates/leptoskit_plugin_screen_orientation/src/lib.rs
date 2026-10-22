//! Requests scoped orientation policies for the owning native window or browser
//! document. A pending hold is not an acknowledgement that the OS rotated.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "screen-orientation";
/// Empty request; two little-endian u32 words in the response.
pub const SNAPSHOT: MethodId = MethodId(0);
/// A single little-endian policy word; remains pending until failure/release.
pub const HOLD: MethodId = MethodId(1);
/// Exported operations.
pub const METHODS: &[MethodId] = &[SNAPSHOT, HOLD];

/// Requested orientation policy. Primary/secondary follow platform conventions;
/// they are not portable physical left/right directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Policy {
	/// Both portrait orientations, where supported.
	Portrait = 0,
	/// Both landscape orientations.
	Landscape = 1,
	/// Platform's primary portrait orientation.
	PortraitPrimary = 2,
	/// Platform's reverse portrait orientation.
	PortraitSecondary = 3,
	/// Platform's primary landscape orientation.
	LandscapePrimary = 4,
	/// Platform's reverse landscape orientation.
	LandscapeSecondary = 5,
	/// All orientations permitted by the application and operating system.
	Any = 6,
}
impl Policy {
	/// Encodes a validated policy.
	#[must_use]
	pub fn encode(self) -> Bytes {
		Bytes::copy_from_slice(&(self as u32).to_le_bytes())
	}

	/// Decodes a policy, rejecting unknown values and trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut decoder = Decoder::new(bytes);
		let value = match decoder.u32()? {
			0 => Self::Portrait,
			1 => Self::Landscape,
			2 => Self::PortraitPrimary,
			3 => Self::PortraitSecondary,
			4 => Self::LandscapePrimary,
			5 => Self::LandscapeSecondary,
			6 => Self::Any,
			_ => return Err(Error::InvalidArgument),
		};
		decoder.finish()?;
		Ok(value)
	}
}
/// Orientation of the presented interface, rather than a motion sensor reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Orientation {
	/// The platform has not determined an orientation.
	Unknown = 0,
	/// Tall interface.
	Portrait = 1,
	/// Wide interface.
	Landscape = 2,
}
/// Current interface orientation and optional platform display rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
	/// Interface aspect orientation.
	pub orientation: Orientation,
	/// Clockwise/anticlockwise conventions depend on the platform. iOS returns
	/// None; Android and browsers report their display's native rotation angle.
	pub angle: Option<u16>,
}
impl Snapshot {
	/// Encodes the snapshot; angles must be quarter turns below 360 degrees.
	pub fn encode(self) -> Result<Bytes, Error> {
		if self
			.angle
			.is_some_and(|angle| angle >= 360 || angle % 90 != 0)
		{
			return Err(Error::InvalidArgument);
		}
		let mut encoder = Encoder::with_capacity(8);
		encoder.u32(self.orientation as u32);
		encoder.u32(self.angle.map_or(u32::MAX, u32::from));
		Ok(encoder.finish())
	}

	/// Decodes the exact eight-byte response.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut decoder = Decoder::new(bytes);
		let orientation = match decoder.u32()? {
			0 => Orientation::Unknown,
			1 => Orientation::Portrait,
			2 => Orientation::Landscape,
			_ => return Err(Error::InvalidArgument),
		};
		let angle = match decoder.u32()? {
			u32::MAX => None,
			value @ (0 | 90 | 180 | 270) => {
				Some(u16::try_from(value).map_err(|_| Error::InvalidArgument)?)
			}
			_ => return Err(Error::InvalidArgument),
		};
		decoder.finish()?;
		Ok(Self { orientation, angle })
	}
}

/// Native guest API. Drop the lease future to restore the prior native policy.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use std::convert::Infallible;
	use std::future::Future;
	use std::pin::Pin;
	use std::task::Context;
	use std::task::Poll;

	use super::*;
	/// Cached native registration.
	#[derive(Debug, Clone, Copy)]
	pub struct ScreenOrientation(leptoskit_bridge::guest::Client);
	impl ScreenOrientation {
		/// Resolves the installed plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries the current presented orientation.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}

		/// Requests a policy until the returned future is dropped. Poll it
		/// alongside work to observe rejection or background cancellation.
		pub fn hold(self, policy: Policy) -> Result<Lease, Error> {
			self.0.call(HOLD, &policy.encode()).map(Lease)
		}
	}
	/// One cancellable native policy request, never successfully complete.
	#[must_use = "dropping the future releases the native orientation policy"]
	#[derive(Debug)]
	pub struct Lease(leptoskit_bridge::guest::Call);
	impl Future for Lease {
		type Output = Result<Infallible, Error>;

		fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
			match Pin::new(&mut self.0).poll(cx) {
				Poll::Pending => Poll::Pending,
				Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
				Poll::Ready(Ok(_)) => Poll::Ready(Err(Error::Native)),
			}
		}
	}
}
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn codec_rejects_unknown_policy_and_malformed_snapshots() {
		for policy in [
			Policy::Portrait,
			Policy::Landscape,
			Policy::PortraitPrimary,
			Policy::PortraitSecondary,
			Policy::LandscapePrimary,
			Policy::LandscapeSecondary,
			Policy::Any,
		] {
			assert_eq!(Policy::decode(&policy.encode()), Ok(policy));
		}
		for bytes in [vec![], vec![0; 3], vec![0; 5], 7_u32.to_le_bytes().to_vec()] {
			assert_eq!(Policy::decode(&bytes), Err(Error::InvalidArgument));
		}
		for angle in [None, Some(0), Some(90), Some(180), Some(270)] {
			let value = Snapshot {
				orientation: Orientation::Portrait,
				angle,
			};
			assert_eq!(Snapshot::decode(&value.encode().unwrap()), Ok(value));
		}
		assert_eq!(
			Snapshot {
				orientation: Orientation::Unknown,
				angle: Some(360)
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(Snapshot::decode(&[0; 9]), Err(Error::InvalidArgument));
		assert_eq!(
			Snapshot::decode(&[3, 0, 0, 0, 0, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Snapshot::decode(&[0, 0, 0, 0, 1, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
	}
}
