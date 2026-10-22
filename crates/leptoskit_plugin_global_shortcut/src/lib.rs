//! Explicitly owned desktop global shortcuts.
//!
//! Native hosts construct [`GlobalShortcuts`] on their event-loop thread after
//! application startup. Registration is fallible and scoped by a
//! [`Registration`] guard. A host can install [`ShortcutPlugin`] into its
//! bridge after granting the capability deliberately. There is no
//! infallible/default registration.
//!
//! macOS, Windows and X11 are supported. Native Wayland, browsers and mobile
//! return [`Error::Unavailable`]. On macOS creation must be on the main thread.

use std::collections::BTreeMap;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod desktop;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub use desktop::GlobalShortcuts;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub use desktop::Registration;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod unsupported;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub use unsupported::GlobalShortcuts;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub use unsupported::Registration;

/// Maximum queued native events. Overflow is reported explicitly in each batch.
pub const EVENT_CAPACITY: usize = 128;
/// Maximum concurrently registered shortcuts.
pub const REGISTRATION_CAPACITY: usize = 64;
/// Register a UTF-8 accelerator, returning its u32 identifier.
pub const REGISTER: MethodId = MethodId(0);
/// Unregister a u32 identifier owned by this plugin.
pub const UNREGISTER: MethodId = MethodId(1);
/// Drain a bounded batch: lost u64 (two u32), count u32, then id/state pairs.
pub const EVENTS: MethodId = MethodId(2);

/// A native registration or lifecycle failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// Platform/session is unsupported.
	#[error("global shortcuts are unavailable on this platform or session")]
	Unavailable,
	/// AppKit registration must occur on the main thread.
	#[error("global shortcuts must be constructed on the main thread")]
	WrongThread,
	/// Another owner in this process already uses the global backend.
	#[error("a global shortcut owner is already active")]
	AlreadyOwned,
	/// Invalid accelerator syntax or excessive length.
	#[error("invalid accelerator")]
	InvalidShortcut,
	/// The accelerator is already owned or the OS refused registration.
	#[error("shortcut registration conflict: {0}")]
	Conflict(String),
	/// The application reached its configured fixed registration limit.
	#[error("global shortcut capacity exhausted")]
	Capacity,
	/// Native failure, including unregister failures.
	#[error("global shortcut operation failed: {0}")]
	Native(String),
}

impl From<Error> for leptoskit_bridge::Error {
	fn from(error: Error) -> Self {
		match error {
			Error::Unavailable => Self::Unavailable,
			Error::WrongThread | Error::InvalidShortcut => Self::InvalidArgument,
			Error::AlreadyOwned | Error::Conflict(_) | Error::Capacity => Self::Busy,
			Error::Native(_) => Self::Native,
		}
	}
}

/// Press/release state delivered by the operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum KeyState {
	/// Key pressed.
	Pressed = 1,
	/// Key released.
	Released = 2,
}

/// One event for a currently owned shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
	/// Native shortcut identifier returned at registration.
	pub id: u32,
	/// Native key transition.
	pub state: KeyState,
}

/// A bounded drain. A nonzero `lost` means consumers must discard assumptions
/// about held keys; press/release pairs may have been interrupted by overflow.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EventBatch {
	/// Events in native arrival order.
	pub events: Vec<Event>,
	/// Events lost since the previous drain, saturating only at `u64::MAX`.
	pub lost: u64,
}

impl EventBatch {
	fn encode(&self) -> Bytes {
		let mut output = Encoder::with_capacity(12 + self.events.len() * 8);
		output.u32(self.lost as u32);
		output.u32((self.lost >> 32) as u32);
		output.u32(self.events.len() as u32);
		for event in &self.events {
			output.u32(event.id);
			output.u32(event.state as u32);
		}
		output.finish()
	}

	/// Decode a complete bounded event batch.
	pub fn decode(bytes: &[u8]) -> Result<Self, leptoskit_bridge::Error> {
		let mut input = Decoder::new(bytes);
		let low = u64::from(input.u32()?);
		let high = u64::from(input.u32()?);
		let count = input.u32()? as usize;
		if count > EVENT_CAPACITY {
			return Err(leptoskit_bridge::Error::TooLarge);
		}
		let mut events = Vec::with_capacity(count);
		for _ in 0..count {
			let id = input.u32()?;
			let state = match input.u32()? {
				1 => KeyState::Pressed,
				2 => KeyState::Released,
				_ => return Err(leptoskit_bridge::Error::InvalidArgument),
			};
			events.push(Event { id, state });
		}
		input.finish()?;
		Ok(Self {
			events,
			lost: low | (high << 32),
		})
	}
}

/// A plugin installed explicitly by the native host on its event-loop thread.
///
/// A host should use `with_waker` and post a wake message to its own event
/// loop. The callback must not directly enter a Wasm interpreter or touch UI
/// objects. Guests can drain events with `guest::GlobalShortcut::events` when
/// enabled.
pub struct ShortcutPlugin {
	registrations: BTreeMap<u32, Registration>,
	manager: GlobalShortcuts,
}

impl ShortcutPlugin {
	/// Takes a previously initialized native owner.
	pub fn new(manager: GlobalShortcuts) -> Self {
		Self {
			registrations: BTreeMap::new(),
			manager,
		}
	}

	/// Report native RAII cleanup errors to the owning host.
	pub fn take_cleanup_error(&self) -> Option<Error> {
		self.manager.take_cleanup_error()
	}
}

impl Plugin for ShortcutPlugin {
	fn name(&self) -> &'static str {
		"global-shortcut"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[REGISTER, UNREGISTER, EVENTS]
	}

	fn invoke(
		&mut self,
		method: MethodId,
		payload: Bytes,
	) -> Result<Bytes, leptoskit_bridge::Error> {
		match method {
			REGISTER => {
				let text = std::str::from_utf8(&payload)
					.map_err(|_| leptoskit_bridge::Error::InvalidArgument)?;
				let registration = self.manager.register(text)?;
				let id = registration.id();
				self.registrations.insert(id, registration);
				Ok(id.to_le_bytes().to_vec().into())
			}
			UNREGISTER => {
				let mut input = Decoder::new(&payload);
				let id = input.u32()?;
				input.finish()?;
				let registration = self
					.registrations
					.get_mut(&id)
					.ok_or(leptoskit_bridge::Error::InvalidCall)?;
				registration.unregister()?;
				self.registrations.remove(&id);
				Ok(Bytes::new())
			}
			EVENTS if payload.is_empty() => {
				if let Some(error) = self.manager.take_cleanup_error() {
					return Err(error.into());
				}
				Ok(self.manager.drain().encode())
			}
			EVENTS => Err(leptoskit_bridge::Error::InvalidArgument),
			_ => Err(leptoskit_bridge::Error::Unavailable),
		}
	}
}

/// Typed guest calls. The native host must explicitly install/grant the plugin.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub mod guest {
	use leptoskit_bridge::Error;
	use leptoskit_bridge::guest::Client;

	use super::EVENTS;
	use super::EventBatch;
	use super::REGISTER;
	use super::UNREGISTER;

	/// Resolved guest-side client.
	#[derive(Clone, Copy)]
	pub struct GlobalShortcut(Client);
	impl GlobalShortcut {
		/// Resolve an installed native plugin.
		pub fn new() -> Result<Self, Error> {
			Client::new("global-shortcut").map(Self)
		}

		/// Register an accelerator. The host owns it until unregister or
		/// teardown.
		pub async fn register(self, accelerator: &str) -> Result<u32, Error> {
			if accelerator.is_empty() || accelerator.len() > 256 {
				return Err(Error::InvalidArgument);
			}
			let bytes = self.0.call(REGISTER, accelerator.as_bytes())?.await?;
			Ok(u32::from_le_bytes(
				bytes
					.as_ref()
					.try_into()
					.map_err(|_| Error::InvalidArgument)?,
			))
		}

		/// Unregister a previously returned identifier.
		pub async fn unregister(self, id: u32) -> Result<(), Error> {
			let output = self.0.call(UNREGISTER, &id.to_le_bytes())?.await?;
			if !output.is_empty() {
				return Err(Error::InvalidArgument);
			}
			Ok(())
		}

		/// Drain events without an unbounded subscription queue.
		pub async fn events(self) -> Result<EventBatch, Error> {
			EventBatch::decode(&self.0.call(EVENTS, &[])?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn event_codec_reports_loss_and_rejects_invalid_batches() {
		let batch = EventBatch {
			events: vec![Event {
				id: 17,
				state: KeyState::Released,
			}],
			lost: (1 << 32) + 7,
		};
		assert_eq!(EventBatch::decode(&batch.encode()).unwrap(), batch);
		let mut malformed = batch.encode().to_vec();
		malformed[16] = 3;
		assert!(EventBatch::decode(&malformed).is_err());
		assert!(EventBatch::decode(&batch.encode()[..19]).is_err());
		malformed = batch.encode().to_vec();
		malformed.push(0);
		assert!(EventBatch::decode(&malformed).is_err());
	}
}
