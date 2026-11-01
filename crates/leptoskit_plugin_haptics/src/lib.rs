//! Selection, impact and notification feedback on native actuators.
//!
//! Success means the OS accepted the request, not that hardware vibrated.
//! Accessibility/user settings, low-power mode and absent actuators can
//! suppress feedback. Android uses view haptics, respecting user settings
//! without needing the VIBRATE permission. Browser vibration requires supported
//! hardware and user activation. macOS maps effects to its trackpad's available
//! patterns.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;

/// Stable runtime registration name.
pub const NAME: &str = "haptics";
/// No payload; selection-change feedback.
pub const SELECTION: MethodId = MethodId(0);
/// One little-endian u32 [`Impact`] value.
pub const IMPACT: MethodId = MethodId(1);
/// One little-endian u32 [`Notification`] value.
pub const NOTIFICATION: MethodId = MethodId(2);
/// Platform-defined Android haptic constant, encoded as a little-endian u32.
/// Only known public constants supported by the running Android API are
/// accepted.
pub const ANDROID_FEEDBACK: MethodId = MethodId(3);
/// Supported wire operations.
pub const METHODS: &[MethodId] = &[SELECTION, IMPACT, NOTIFICATION, ANDROID_FEEDBACK];

/// Collision intensity/style; the exact sensation is hardware-dependent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Impact {
	/// Light collision.
	Light = 0,
	/// Medium collision.
	Medium = 1,
	/// Heavy collision.
	Heavy = 2,
	/// Rigid collision.
	Rigid = 3,
	/// Soft collision.
	Soft = 4,
}

/// Feedback accompanying the outcome of a user action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Notification {
	/// Action completed.
	Success = 0,
	/// Action needs attention.
	Warning = 1,
	/// Action failed.
	Error = 2,
}

/// Validated portable effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
	/// A selection changed.
	Selection,
	/// A collision.
	Impact(Impact),
	/// An action outcome.
	Notification(Notification),
}

/// Validates portable wire payloads without allocating.
pub fn decode(method: MethodId, payload: &[u8]) -> Result<Effect, Error> {
	if method == SELECTION {
		return if payload.is_empty() {
			Ok(Effect::Selection)
		} else {
			Err(Error::InvalidArgument)
		};
	}

	if !matches!(method, IMPACT | NOTIFICATION) {
		return Err(Error::Unavailable);
	}

	let value = u32::from_le_bytes(payload.try_into().map_err(|_| Error::InvalidArgument)?);

	match (method, value) {
		(IMPACT, 0) => Ok(Effect::Impact(Impact::Light)),
		(IMPACT, 1) => Ok(Effect::Impact(Impact::Medium)),
		(IMPACT, 2) => Ok(Effect::Impact(Impact::Heavy)),
		(IMPACT, 3) => Ok(Effect::Impact(Impact::Rigid)),
		(IMPACT, 4) => Ok(Effect::Impact(Impact::Soft)),
		(NOTIFICATION, 0) => Ok(Effect::Notification(Notification::Success)),
		(NOTIFICATION, 1) => Ok(Effect::Notification(Notification::Warning)),
		(NOTIFICATION, 2) => Ok(Effect::Notification(Notification::Error)),
		_ => Err(Error::InvalidArgument),
	}
}

/// Desktop bridge adapter. macOS uses the trackpad; other desktops currently
/// report `Unavailable` because no portable system actuator API exists.
#[derive(Debug, Default)]
pub struct Haptics;

impl Plugin for Haptics {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		perform(decode(method, &payload)?)?;
		Ok(Bytes::new())
	}
}

/// Requests desktop feedback. Call on the application's main thread.
#[cfg(target_os = "macos")]
pub fn perform(effect: Effect) -> Result<(), Error> {
	use objc2_app_kit::NSHapticFeedbackManager;
	use objc2_app_kit::NSHapticFeedbackPattern;
	use objc2_app_kit::NSHapticFeedbackPerformanceTime;
	use objc2_app_kit::NSHapticFeedbackPerformer as _;
	let _main = objc2::MainThreadMarker::new().ok_or(Error::InvalidArgument)?;

	let pattern = match effect {
		Effect::Selection => NSHapticFeedbackPattern::Alignment,
		Effect::Impact(_) => NSHapticFeedbackPattern::LevelChange,
		Effect::Notification(_) => NSHapticFeedbackPattern::Generic,
	};

	NSHapticFeedbackManager::defaultPerformer()
		.performFeedbackPattern_performanceTime(pattern, NSHapticFeedbackPerformanceTime::Now);
	Ok(())
}

/// No portable desktop haptic actuator is available on this platform.
#[cfg(not(target_os = "macos"))]
pub fn perform(_effect: Effect) -> Result<(), Error> {
	Err(Error::Unavailable)
}

/// Browser vibration approximations. Browser/device policy may reject them.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;

	use super::*;

	/// Plays a short vibration appropriate to the requested effect.
	pub fn perform(effect: Effect) -> Result<(), Error> {
		let navigator = web_sys::window().ok_or(Error::Unavailable)?.navigator();
		let method = js_sys::Reflect::get(navigator.as_ref(), &JsValue::from_str("vibrate"))
			.map_err(|_| Error::Native)?;

		if !method.is_instance_of::<js_sys::Function>() {
			return Err(Error::Unavailable);
		}

		let duration = match effect {
			Effect::Selection | Effect::Impact(Impact::Light | Impact::Soft) => 10,
			Effect::Impact(Impact::Medium | Impact::Rigid) => 20,
			Effect::Impact(Impact::Heavy) => 30,
			Effect::Notification(Notification::Success) => 40,
			Effect::Notification(Notification::Warning) => 60,
			Effect::Notification(Notification::Error) => 80,
		};

		if navigator.vibrate_with_duration(duration) {
			Ok(())
		} else {
			Err(Error::Unavailable)
		}
	}
}

/// Native interpreter client; resolves the plugin only once.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// Cached native plugin handle.
	#[derive(Debug, Clone, Copy)]
	pub struct Haptics(leptoskit_bridge::guest::Client);
	impl Haptics {
		/// Resolves a host-installed plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Requests selection feedback.
		pub async fn selection(self) -> Result<(), Error> {
			leptoskit_bridge::codec::Decoder::new(&self.0.call(SELECTION, &[])?.await?).finish()
		}

		/// Requests collision feedback.
		pub async fn impact(self, style: Impact) -> Result<(), Error> {
			leptoskit_bridge::codec::Decoder::new(
				&self.0.call(IMPACT, &(style as u32).to_le_bytes())?.await?,
			)
			.finish()
		}

		/// Requests action-outcome feedback.
		pub async fn notification(self, kind: Notification) -> Result<(), Error> {
			leptoskit_bridge::codec::Decoder::new(
				&self
					.0
					.call(NOTIFICATION, &(kind as u32).to_le_bytes())?
					.await?,
			)
			.finish()
		}

		/// Requests a public Android `HapticFeedbackConstants` value.
		/// Other platforms and unsupported API levels return `Unavailable`.
		pub async fn android_feedback(self, constant: u32) -> Result<(), Error> {
			leptoskit_bridge::codec::Decoder::new(
				&self
					.0
					.call(ANDROID_FEEDBACK, &constant.to_le_bytes())?
					.await?,
			)
			.finish()
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn binary_effects_reject_truncation_unknown_values_and_trailing_bytes() {
		assert_eq!(decode(SELECTION, &[]), Ok(Effect::Selection));
		assert_eq!(
			decode(IMPACT, &[4, 0, 0, 0]),
			Ok(Effect::Impact(Impact::Soft))
		);
		assert_eq!(
			decode(NOTIFICATION, &[2, 0, 0, 0]),
			Ok(Effect::Notification(Notification::Error))
		);

		for payload in [&[0][..], &[0, 0, 0], &[5, 0, 0, 0], &[0, 0, 0, 0, 0]] {
			assert_eq!(decode(IMPACT, payload), Err(Error::InvalidArgument));
		}

		assert_eq!(decode(SELECTION, &[0]), Err(Error::InvalidArgument));

		assert_eq!(decode(MethodId(99), &[]), Err(Error::Unavailable));
	}

	#[test]
	fn every_portable_value_decodes_to_its_effect() {
		let impacts = [
			Impact::Light,
			Impact::Medium,
			Impact::Heavy,
			Impact::Rigid,
			Impact::Soft,
		];

		for impact in impacts {
			assert_eq!(
				decode(IMPACT, &(impact as u32).to_le_bytes()),
				Ok(Effect::Impact(impact))
			);
		}

		let notifications = [
			Notification::Success,
			Notification::Warning,
			Notification::Error,
		];

		for notification in notifications {
			assert_eq!(
				decode(NOTIFICATION, &(notification as u32).to_le_bytes()),
				Ok(Effect::Notification(notification))
			);
		}

		assert_eq!(
			decode(NOTIFICATION, &3_u32.to_le_bytes()),
			Err(Error::InvalidArgument)
		);
		// Android constants are platform-specific and never decoded portably.
		assert_eq!(
			decode(ANDROID_FEEDBACK, &1_u32.to_le_bytes()),
			Err(Error::Unavailable)
		);
	}

	#[test]
	fn desktop_adapter_validates_before_requesting_feedback() {
		let mut haptics = Haptics;
		assert_eq!(haptics.name(), NAME);
		assert_eq!(haptics.methods(), METHODS);
		assert_eq!(
			haptics.invoke(IMPACT, Bytes::from(vec![9, 0, 0, 0])),
			Err(Error::InvalidArgument)
		);
		// Test threads are not AppKit's main thread, and other desktops have no
		// actuator: a valid effect is rejected without touching hardware.
		let expected = if cfg!(target_os = "macos") {
			Error::InvalidArgument
		} else {
			Error::Unavailable
		};

		assert_eq!(haptics.invoke(SELECTION, Bytes::new()), Err(expected));
	}
}
