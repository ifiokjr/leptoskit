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

/// A quarter turn, in degrees. Display rotations are always a multiple of it.
const QUARTER_TURN: u16 = 90;
/// A full turn, in degrees: the first angle that is not a rotation.
const FULL_TURN: u16 = QUARTER_TURN * 4;
/// Bytes in a snapshot response: two little-endian u32 words.
const SNAPSHOT_BYTES: usize = 8;

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
	/// Every policy in wire order: the discriminants are the wire ids.
	const ALL: [Self; 7] = [
		Self::Portrait,
		Self::Landscape,
		Self::PortraitPrimary,
		Self::PortraitSecondary,
		Self::LandscapePrimary,
		Self::LandscapeSecondary,
		Self::Any,
	];

	/// Encodes a validated policy.
	#[must_use]
	pub fn encode(self) -> Bytes {
		Bytes::copy_from_slice(&(self as u32).to_le_bytes())
	}

	/// Decodes a policy, rejecting unknown values and trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut decoder = Decoder::new(bytes);
		let word = usize::try_from(decoder.u32()?).map_err(|_| Error::InvalidArgument)?;
		let policy = Self::ALL.get(word).copied().ok_or(Error::InvalidArgument)?;

		decoder.finish()?;
		Ok(policy)
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
/// A display rotation is a whole quarter turn below a full turn.
fn is_quarter_turn(degrees: u16) -> bool {
	degrees < FULL_TURN && degrees.is_multiple_of(QUARTER_TURN)
}

/// Decodes the rotation word, where `u32::MAX` means the platform has none.
fn decode_angle(word: u32) -> Result<Option<u16>, Error> {
	if word == u32::MAX {
		return Ok(None);
	}

	let degrees = u16::try_from(word).map_err(|_| Error::InvalidArgument)?;

	if !is_quarter_turn(degrees) {
		return Err(Error::InvalidArgument);
	}

	Ok(Some(degrees))
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
		if self.angle.is_some_and(|angle| !is_quarter_turn(angle)) {
			return Err(Error::InvalidArgument);
		}

		let mut encoder = Encoder::with_capacity(SNAPSHOT_BYTES);
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

		let angle = decode_angle(decoder.u32()?)?;

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
	fn wire_ids_are_the_declaration_order() {
		for (word, policy) in Policy::ALL.into_iter().enumerate() {
			assert_eq!(policy as u32, word as u32);
		}
	}
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

		for angle in [
			None,
			Some(0),
			Some(QUARTER_TURN),
			Some(QUARTER_TURN * 2),
			Some(QUARTER_TURN * 3),
		] {
			let value = Snapshot {
				orientation: Orientation::Portrait,
				angle,
			};
			assert_eq!(Snapshot::decode(&value.encode().unwrap()), Ok(value));
		}

		assert_eq!(
			Snapshot {
				orientation: Orientation::Unknown,
				angle: Some(FULL_TURN)
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
