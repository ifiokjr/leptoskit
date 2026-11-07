//! <!-- {=p_picture_in_pictureSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_picture_in_picture` as a direct dependency and select
//! the `picture-in-picture` plugin in `native.plugins`; supported generated
//! hosts discover it from Cargo metadata. Generated Windows, Linux, and
//! terminal hosts currently omit runtime plugins; use explicit registration in
//! a custom host there. Guests enable the `guest` feature and reuse
//! `guest::PictureInPicture` rather than resolving the plugin on every call.
//! Android can enter activity picture-in-picture and, with
//! `SYSTEM_ALERT_WINDOW`, hold a system overlay window. iOS reports whether
//! system picture-in-picture is supported but does not yet enter it without an
//! attached video player, and has no draw-over-apps overlay. Desktop and the
//! browser return `Unavailable`.
//!
//! <!-- {/p_picture_in_pictureSummary} -->
//!
//! Picture-in-picture shrinks the app into a system floating window while
//! another app is open. A system overlay is a separate floating window drawn
//! over other apps; Android requires the special `SYSTEM_ALERT_WINDOW`
//! permission the user grants in system settings. Declaring the permission in
//! the manifest never grants it. iOS has no equivalent overlay permission.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "picture-in-picture";
/// Empty request -> versioned capability flags.
pub const CAPABILITIES: MethodId = MethodId(0);
/// Empty request -> versioned session snapshot.
pub const SNAPSHOT: MethodId = MethodId(1);
/// Empty request; remains pending while picture-in-picture is held.
pub const ENTER_PIP: MethodId = MethodId(2);
/// Empty request; remains pending while a system overlay window is held.
pub const ENTER_OVERLAY: MethodId = MethodId(3);
/// Empty request -> overlay permission state.
pub const OVERLAY_STATUS: MethodId = MethodId(4);
/// Empty request -> overlay permission after opening the system grant UI.
pub const REQUEST_OVERLAY: MethodId = MethodId(5);
/// Exported operations.
pub const METHODS: &[MethodId] = &[
	CAPABILITIES,
	SNAPSHOT,
	ENTER_PIP,
	ENTER_OVERLAY,
	OVERLAY_STATUS,
	REQUEST_OVERLAY,
];

/// Wire protocol version for capability and snapshot replies.
const WIRE_VERSION: u32 = 1;

/// What the host can do, independent of the current permission grant and of
/// whether a session is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
	/// Whether the host can enter system picture-in-picture for this app.
	pub picture_in_picture: bool,
	/// Whether the host can show a draw-over-apps overlay once permitted.
	pub system_overlay: bool,
}

impl Capabilities {
	/// No picture-in-picture or overlay support.
	pub const UNAVAILABLE: Self = Self {
		picture_in_picture: false,
		system_overlay: false,
	};

	/// Encodes version and two boolean flags.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(12);
		output.u32(WIRE_VERSION);
		output.u32(u32::from(self.picture_in_picture));
		output.u32(u32::from(self.system_overlay));
		output.finish()
	}

	/// Strictly validates a fixed 12-byte capability reply.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		require_version(&mut input)?;
		let picture_in_picture = decode_flag(input.u32()?)?;
		let system_overlay = decode_flag(input.u32()?)?;
		input.finish()?;
		Ok(Self {
			picture_in_picture,
			system_overlay,
		})
	}
}

/// Which floating presentation, if any, the host currently owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Session {
	/// Neither picture-in-picture nor a system overlay is active.
	None = 0,
	/// The app is in system picture-in-picture.
	PictureInPicture = 1,
	/// A draw-over-apps overlay window is showing.
	SystemOverlay = 2,
}

impl Session {
	fn decode(value: u32) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::None),
			1 => Ok(Self::PictureInPicture),
			2 => Ok(Self::SystemOverlay),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Current floating-presentation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
	/// Active session owned by this plugin instance.
	pub session: Session,
}

impl Snapshot {
	/// Encodes version and session.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(8);
		output.u32(WIRE_VERSION);
		output.u32(self.session as u32);
		output.finish()
	}

	/// Strictly validates a fixed 8-byte snapshot reply.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		require_version(&mut input)?;
		let session = Session::decode(input.u32()?)?;
		input.finish()?;
		Ok(Self { session })
	}
}

/// Android draw-over-apps authorization, or an explicit unsupported state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum OverlayPermission {
	/// The platform has no system overlay API.
	Unsupported = 0,
	/// Android `Settings.canDrawOverlays` is false.
	Denied = 1,
	/// Android `Settings.canDrawOverlays` is true.
	Granted = 2,
}

impl OverlayPermission {
	/// Encodes as one little-endian word.
	#[must_use]
	pub fn encode(self) -> Bytes {
		Bytes::copy_from_slice(&(self as u32).to_le_bytes())
	}

	/// Decodes exactly four bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let value = input.u32()?;
		input.finish()?;
		match value {
			0 => Ok(Self::Unsupported),
			1 => Ok(Self::Denied),
			2 => Ok(Self::Granted),
			_ => Err(Error::InvalidArgument),
		}
	}
}

fn require_version(input: &mut Decoder<'_>) -> Result<(), Error> {
	if input.u32()? == WIRE_VERSION {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}

fn decode_flag(value: u32) -> Result<bool, Error> {
	match value {
		0 => Ok(false),
		1 => Ok(true),
		_ => Err(Error::InvalidArgument),
	}
}

/// Validates empty request payloads for every exported method.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	if !METHODS.contains(&method) {
		return Err(Error::Unavailable);
	}

	if payload.is_empty() {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}

/// Browser and desktop builds expose no portable floating-window API.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Returns explicit unavailable capabilities without prompting.
	pub const fn capabilities() -> Capabilities {
		Capabilities::UNAVAILABLE
	}

	/// No browser API matches the native session contract.
	pub fn snapshot() -> Result<Snapshot, Error> {
		Err(Error::Unavailable)
	}

	/// Picture-in-picture Document PiP is not this plugin's contract.
	pub fn enter_picture_in_picture() -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Browsers cannot draw a system overlay over other applications.
	pub fn enter_overlay() -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Overlay permission is unsupported in the browser.
	pub const fn overlay_status() -> OverlayPermission {
		OverlayPermission::Unsupported
	}

	/// There is no settings screen to open for overlay permission.
	pub fn request_overlay() -> Result<OverlayPermission, Error> {
		Err(Error::Unavailable)
	}
}

/// Cached native guest client. Lease futures cancel native resources on drop.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use std::convert::Infallible;
	use std::future::Future;
	use std::pin::Pin;
	use std::task::Context;
	use std::task::Poll;

	use super::*;

	/// Resolved host plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct PictureInPicture(leptoskit_bridge::guest::Client);

	impl PictureInPicture {
		/// Resolves the registered host plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries OS support; permission grants are separate.
		pub async fn capabilities(self) -> Result<Capabilities, Error> {
			Capabilities::decode(&self.0.call(CAPABILITIES, &[])?.await?)
		}

		/// Samples the active floating session owned by this plugin.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}

		/// Enters picture-in-picture until the lease is dropped. Pending is not
		/// an acquisition acknowledgement; poll beside work for failures.
		pub fn enter_picture_in_picture(self) -> Result<Lease, Error> {
			self.0.call(ENTER_PIP, &[]).map(Lease)
		}

		/// Shows a system overlay until the lease is dropped. Requires overlay
		/// permission on Android; iOS returns `Unavailable`.
		pub fn enter_overlay(self) -> Result<Lease, Error> {
			self.0.call(ENTER_OVERLAY, &[]).map(Lease)
		}

		/// Reads draw-over-apps authorization without opening settings.
		pub async fn overlay_status(self) -> Result<OverlayPermission, Error> {
			OverlayPermission::decode(&self.0.call(OVERLAY_STATUS, &[])?.await?)
		}

		/// Opens the system overlay settings UI on Android and returns the
		/// permission observed when control returns. Never grants access
		/// itself.
		pub async fn request_overlay(self) -> Result<OverlayPermission, Error> {
			OverlayPermission::decode(&self.0.call(REQUEST_OVERLAY, &[])?.await?)
		}
	}

	/// One cancellable native floating session.
	#[must_use = "dropping the lease ends the picture-in-picture or overlay request"]
	#[derive(Debug)]
	pub struct Lease(leptoskit_bridge::guest::Call);

	impl Future for Lease {
		type Output = Result<Infallible, Error>;

		fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
			match Pin::new(&mut self.0).poll(context) {
				Poll::Pending => Poll::Pending,
				Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
				Poll::Ready(Ok(_)) => Poll::Ready(Err(Error::Native)),
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn words(values: &[u32]) -> Vec<u8> {
		values
			.iter()
			.flat_map(|value| value.to_le_bytes())
			.collect()
	}

	#[test]
	fn validate_rejects_unknown_methods_and_payloads() {
		for method in METHODS {
			assert_eq!(validate(*method, &[]), Ok(()));
			assert_eq!(validate(*method, &[0]), Err(Error::InvalidArgument));
		}

		assert_eq!(validate(MethodId(6), &[]), Err(Error::Unavailable));
	}

	#[test]
	fn capabilities_round_trip_and_reject_bad_flags() {
		let value = Capabilities {
			picture_in_picture: true,
			system_overlay: false,
		};
		assert_eq!(Capabilities::decode(&value.encode()), Ok(value));
		assert_eq!(
			Capabilities::decode(&words(&[1, 0, 1])),
			Ok(Capabilities {
				picture_in_picture: false,
				system_overlay: true,
			})
		);

		for values in [[0, 0, 0], [1, 2, 0], [1, 0, 2]] {
			assert_eq!(
				Capabilities::decode(&words(&values)),
				Err(Error::InvalidArgument)
			);
		}

		assert_eq!(
			Capabilities::decode(&words(&[1, 0, 0, 0])),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn snapshot_preserves_sessions_and_rejects_unknown() {
		for session in [
			Session::None,
			Session::PictureInPicture,
			Session::SystemOverlay,
		] {
			let value = Snapshot { session };
			assert_eq!(Snapshot::decode(&value.encode()), Ok(value));
		}

		assert_eq!(
			Snapshot::decode(&words(&[1, 3])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Snapshot::decode(&words(&[2, 0])),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn overlay_permission_is_exact_four_bytes() {
		for permission in [
			OverlayPermission::Unsupported,
			OverlayPermission::Denied,
			OverlayPermission::Granted,
		] {
			assert_eq!(
				OverlayPermission::decode(&permission.encode()),
				Ok(permission)
			);
		}

		assert_eq!(
			OverlayPermission::decode(&3_u32.to_le_bytes()),
			Err(Error::InvalidArgument)
		);
		assert_eq!(OverlayPermission::decode(&[]), Err(Error::InvalidArgument));
		assert_eq!(
			OverlayPermission::decode(&[0, 0, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
	}
}
