//! Capture prevention and observation are distinct capabilities. Android can
//! protect its owned window; iOS can observe scene capture and app screenshots.
//! No API in this crate captures pixels or reads photos. Notifications are
//! demand-driven: one pending waiter, no event queue, and no historical replay.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime plugin name.
pub const NAME: &str = "screen-capture";
/// Empty request -> version, prevention boolean, capture scope, screenshot
/// scope.
pub const CAPABILITIES: MethodId = MethodId(0);
/// Empty request -> version, capture state, protected-window flag.
pub const SNAPSHOT: MethodId = MethodId(1);
/// [`Watch`] -> one [`Event`]. Only one waiter is permitted per native plugin.
pub const WAIT: MethodId = MethodId(2);
/// Empty request remains pending while Android protects the owned window.
/// Cancellation releases this lease; pending is not an acquisition
/// acknowledgement.
pub const PREVENT: MethodId = MethodId(3);

/// What the platform observation refers to, rather than inferred attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Scope {
	/// No observation API exists on this implementation.
	Unavailable = 0,
	/// The owning Android Activity/window.
	Window = 1,
	/// The explicitly owned Apple window scene.
	Scene = 2,
	/// App-wide: a screenshot cannot be attributed to a particular app window.
	Application = 3,
}
impl Scope {
	fn decode(value: u32) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::Unavailable),
			1 => Ok(Self::Window),
			2 => Ok(Self::Scene),
			3 => Ok(Self::Application),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// OS support, independent of current permission grants and foreground state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
	/// Whether a native lease can set the owned window's secure policy.
	pub prevention: bool,
	/// Scope of recording/mirroring state and change observation.
	pub capture: Scope,
	/// Scope of screenshot notifications, which occur after capture.
	pub screenshots: Scope,
}
impl Capabilities {
	/// No prevention or observation support. Used by the browser
	/// implementation.
	pub const UNAVAILABLE: Self = Self {
		prevention: false,
		capture: Scope::Unavailable,
		screenshots: Scope::Unavailable,
	};

	/// Strictly validates a fixed 16-byte capability reply.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		version(&mut input)?;

		let prevention = match input.u32()? {
			0 => false,
			1 => true,
			_ => return Err(Error::InvalidArgument),
		};

		let capture = Scope::decode(input.u32()?)?;
		let screenshots = Scope::decode(input.u32()?)?;
		input.finish()?;
		Ok(Self {
			prevention,
			capture,
			screenshots,
		})
	}
}

/// Native recording/mirroring state; unknown is never interpreted as inactive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CaptureState {
	/// The OS cannot report state (including Android in this implementation).
	Unknown = 0,
	/// The owned Apple scene reports no recording/mirroring.
	Inactive = 1,
	/// The owned Apple scene reports recording/mirroring.
	Active = 2,
}
impl CaptureState {
	fn decode(value: u32) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::Unknown),
			1 => Ok(Self::Inactive),
			2 => Ok(Self::Active),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Current OS state, not a guarantee against every capture mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
	/// Recording/mirroring state, with scope specified in capabilities.
	pub capture: CaptureState,
	/// Android secure-window bit; `None` on platforms without this policy.
	pub protected: Option<bool>,
}
impl Snapshot {
	/// Strictly validates a fixed 12-byte state reply.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		version(&mut input)?;
		let capture = CaptureState::decode(input.u32()?)?;

		let protected = match input.u32()? {
			0 => None,
			1 => Some(false),
			2 => Some(true),
			_ => return Err(Error::InvalidArgument),
		};

		input.finish()?;
		Ok(Self { capture, protected })
	}
}

/// One pending observation. Changes may coalesce; screenshots between calls
/// are not retained, replayed, or inferred from storage contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Watch {
	/// Return when state differs from this previous value. State is resampled
	/// after the native observer is installed to avoid a registration gap.
	CaptureChanged(CaptureState),
	/// Wait for a future OS screenshot notification while foregrounded.
	Screenshot,
}
impl Watch {
	/// Encodes kind and previous state as two little-endian words.
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(8);

		let (kind, state) = match self {
			Self::CaptureChanged(state) => (0, state as u32),
			Self::Screenshot => (1, 0),
		};

		output.u32(kind);
		output.u32(state);
		output.finish()
	}

	/// Rejects unknown kinds, states, trailing data and screenshot-state data.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let kind = input.u32()?;
		let state = input.u32()?;
		input.finish()?;

		match (kind, state) {
			(0, state) => Ok(Self::CaptureChanged(CaptureState::decode(state)?)),
			(1, 0) => Ok(Self::Screenshot),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// A terminal response to one observation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
	/// Latest changed capture state; no pixels are included.
	CaptureChanged(CaptureState),
	/// A screenshot was reported. This is notification, never prevention.
	Screenshot,
}
impl Event {
	/// Validates both the reply and its relation to the original request.
	pub fn decode(bytes: &[u8], watch: Watch) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		version(&mut input)?;
		let kind = input.u32()?;
		let state = input.u32()?;
		input.finish()?;

		match (watch, kind, state) {
			(Watch::CaptureChanged(previous), 0, state) => {
				let state = CaptureState::decode(state)?;
				if state == previous {
					return Err(Error::InvalidArgument);
				}

				Ok(Self::CaptureChanged(state))
			}
			(Watch::Screenshot, 1, 0) => Ok(Self::Screenshot),
			_ => Err(Error::InvalidArgument),
		}
	}
}
fn version(input: &mut Decoder<'_>) -> Result<(), Error> {
	if input.u32()? == 1 {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}

/// Browser screen-sharing APIs request capture rather than prevent or detect
/// arbitrary screenshots. This implementation explicitly exposes no support.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;
	/// Returns explicit unavailable capabilities without browser prompts.
	pub const fn capabilities() -> Capabilities {
		Capabilities::UNAVAILABLE
	}
	/// No standard browser API can supply this native state contract.
	pub fn snapshot() -> Result<Snapshot, Error> {
		Err(Error::Unavailable)
	}
	/// No standard browser API observes arbitrary OS screenshot events.
	pub fn wait(_watch: Watch) -> impl Future<Output = Result<Event, Error>> {
		std::future::ready(Err(Error::Unavailable))
	}
	/// Browser screen-sharing permissions do not provide screenshot prevention.
	pub fn prevent() -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}

/// Cached interpreter client. All pending calls cancel native resources on
/// drop.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use std::convert::Infallible;
	use std::future::Future;
	use std::pin::Pin;
	use std::task::Context;
	use std::task::Poll;

	use super::*;
	/// A resolved host plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct ScreenCapture(leptoskit_bridge::guest::Client);
	impl ScreenCapture {
		/// Resolves the registered host plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries OS support; permission and foreground admission are
		/// separate.
		pub async fn capabilities(self) -> Result<Capabilities, Error> {
			Capabilities::decode(&self.0.call(CAPABILITIES, &[])?.await?)
		}

		/// Samples state only for the host's explicit native window/scene.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}

		/// Installs one demand-driven observer. A concurrent wait returns
		/// `Busy`.
		pub async fn wait(self, watch: Watch) -> Result<Event, Error> {
			Event::decode(&self.0.call(WAIT, &watch.encode())?.await?, watch)
		}

		/// Requests a cancellation-owned Android secure-window lease. Poll
		/// beside work to observe failures; pending does not
		/// acknowledge acquisition.
		pub fn prevent(self) -> Result<PreventionLease, Error> {
			self.0.call(PREVENT, &[]).map(PreventionLease)
		}
	}
	/// Holds native prevention until dropped or cancelled by the host
	/// lifecycle.
	#[must_use = "dropping this lease releases the screen-capture prevention request"]
	#[derive(Debug)]
	pub struct PreventionLease(leptoskit_bridge::guest::Call);
	impl Future for PreventionLease {
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
	fn capabilities_do_not_confuse_observation_and_prevention() {
		assert_eq!(
			Capabilities::decode(&words(&[1, 0, 2, 3])),
			Ok(Capabilities {
				prevention: false,
				capture: Scope::Scene,
				screenshots: Scope::Application,
			})
		);
		assert_eq!(
			Capabilities::decode(&words(&[1, 1, 0, 1])).unwrap().capture,
			Scope::Unavailable
		);

		for values in [[2, 0, 2, 3], [1, 2, 0, 0], [1, 0, 4, 0], [1, 0, 0, 4]] {
			assert_eq!(
				Capabilities::decode(&words(&values)),
				Err(Error::InvalidArgument)
			);
		}
	}
	#[test]
	fn snapshot_preserves_unknown_and_rejects_invalid_wire_data() {
		assert_eq!(
			Snapshot::decode(&words(&[1, 0, 2])),
			Ok(Snapshot {
				capture: CaptureState::Unknown,
				protected: Some(true)
			})
		);

		let valid = words(&[1, 2, 0]);

		for size in 0..valid.len() {
			assert!(Snapshot::decode(&valid[..size]).is_err());
		}

		assert!(Snapshot::decode(&words(&[1, 2, 0, 0])).is_err());

		for values in [[0, 0, 0], [1, 3, 0], [1, 0, 3]] {
			assert!(Snapshot::decode(&words(&values)).is_err());
		}
	}
	#[test]
	fn notifications_match_the_request_and_never_report_unchanged_state() {
		for watch in [
			Watch::Screenshot,
			Watch::CaptureChanged(CaptureState::Unknown),
			Watch::CaptureChanged(CaptureState::Active),
		] {
			assert_eq!(Watch::decode(&watch.encode()), Ok(watch));
		}

		for values in [[0, 3], [1, 1], [2, 0]] {
			assert!(Watch::decode(&words(&values)).is_err());
		}

		let watch = Watch::CaptureChanged(CaptureState::Inactive);
		assert_eq!(
			Event::decode(&words(&[1, 0, 2]), watch),
			Ok(Event::CaptureChanged(CaptureState::Active))
		);

		for values in [[1, 0, 1], [1, 1, 0], [1, 0, 3], [2, 0, 2]] {
			assert!(Event::decode(&words(&values), watch).is_err());
		}

		assert_eq!(
			Event::decode(&words(&[1, 1, 0]), Watch::Screenshot),
			Ok(Event::Screenshot)
		);
		assert!(Event::decode(&words(&[1, 0, 0]), Watch::Screenshot).is_err());
		assert!(Event::decode(&words(&[1, 1, 2]), Watch::Screenshot).is_err());
		assert!(Watch::decode(&words(&[1, 0, 0])).is_err());
	}
}
