//! Selection, impact and notification feedback on native actuators.
//!
//! Success means the OS accepted the request, not that hardware vibrated.
//! Accessibility/user settings, low-power mode and absent actuators can
//! suppress feedback. Android uses view haptics, respecting user settings
//! without needing the VIBRATE permission. Browser vibration requires supported
//! hardware and user activation. macOS maps effects to its trackpad's available
//! patterns.
#![cfg_attr(target_os = "android", allow(unsafe_code))]

use leptoskit_bridge::AppContext;
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

/// Native bridge adapter. macOS uses the trackpad, iOS uses UIKit feedback
/// generators, and Android uses `View.performHapticFeedback` on the host
/// `Activity` window's decor view. Other desktops report `Unavailable` because
/// no portable system actuator API exists.
#[derive(Debug)]
pub struct Haptics {
	#[cfg(target_os = "android")]
	platform: Option<leptoskit_bridge::PlatformContext>,
}

#[allow(clippy::derivable_impls)]
impl Default for Haptics {
	fn default() -> Self {
		Self {
			#[cfg(target_os = "android")]
			platform: None,
		}
	}
}

/// Constructs the haptics plugin from the host [`AppContext`].
pub fn from_context(context: &AppContext) -> Result<Haptics, Error> {
	#[cfg(not(target_os = "android"))]
	let _ = context;
	#[cfg(target_os = "android")]
	if context
		.platform()
		.and_then(leptoskit_bridge::PlatformContext::android_handles)
		.is_none()
	{
		return Err(Error::Unavailable);
	}

	Ok(Haptics {
		#[cfg(target_os = "android")]
		platform: context.platform().cloned(),
	})
}

impl Plugin for Haptics {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		#[cfg(target_os = "android")]
		{
			let platform = self.platform.as_ref().ok_or(Error::Unavailable)?;
			android::invoke(platform, method, &payload)?;
			Ok(Bytes::new())
		}
		#[cfg(not(target_os = "android"))]
		{
			perform(decode(method, &payload)?)?;
			Ok(Bytes::new())
		}
	}
}

/// Requests feedback on macOS. Call on the application's main thread.
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

/// Requests feedback on iOS using UIKit feedback generators. Call on the
/// application's main thread.
#[cfg(target_os = "ios")]
pub fn perform(effect: Effect) -> Result<(), Error> {
	use objc2_ui_kit::UIImpactFeedbackGenerator;
	use objc2_ui_kit::UIImpactFeedbackStyle;
	use objc2_ui_kit::UINotificationFeedbackGenerator;
	use objc2_ui_kit::UINotificationFeedbackType;
	use objc2_ui_kit::UISelectionFeedbackGenerator;

	let main = objc2::MainThreadMarker::new().ok_or(Error::InvalidArgument)?;

	match effect {
		Effect::Selection => {
			let generator = UISelectionFeedbackGenerator::new(main);
			generator.selectionChanged();
		}
		Effect::Impact(style) => {
			let ui_style = match style {
				Impact::Light => UIImpactFeedbackStyle::Light,
				Impact::Medium => UIImpactFeedbackStyle::Medium,
				Impact::Heavy => UIImpactFeedbackStyle::Heavy,
				Impact::Rigid => UIImpactFeedbackStyle::Rigid,
				Impact::Soft => UIImpactFeedbackStyle::Soft,
			};
			// `initWithStyle:` is the view-independent initializer available on
			// iOS 10+ before `feedbackGeneratorWithStyle:forView:` (iOS 17.5+).
			#[allow(deprecated)]
			let generator = UIImpactFeedbackGenerator::initWithStyle(main.alloc(), ui_style);
			generator.impactOccurred();
		}
		Effect::Notification(kind) => {
			let feedback = match kind {
				Notification::Success => UINotificationFeedbackType::Success,
				Notification::Warning => UINotificationFeedbackType::Warning,
				Notification::Error => UINotificationFeedbackType::Error,
			};
			let generator = UINotificationFeedbackGenerator::new(main);
			generator.notificationOccurred(feedback);
		}
	}

	Ok(())
}

/// No portable desktop haptic actuator is available on this platform.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
pub fn perform(_effect: Effect) -> Result<(), Error> {
	Err(Error::Unavailable)
}

#[cfg(target_os = "android")]
mod android {
	use jni::JNIEnv;
	use jni::JavaVM;
	use jni::objects::JObject;
	use jni::objects::JValue;
	use leptoskit_bridge::PlatformContext;

	use super::*;

	const LONG_PRESS: i32 = 0;
	const VIRTUAL_KEY: i32 = 1;
	const KEYBOARD_TAP: i32 = 3;
	const CLOCK_TICK: i32 = 4;
	const CONTEXT_CLICK: i32 = 6;
	const KEYBOARD_RELEASE: i32 = 7;
	const VIRTUAL_KEY_RELEASE: i32 = 8;
	const TEXT_HANDLE_MOVE: i32 = 9;
	const GESTURE_START: i32 = 12;
	const GESTURE_END: i32 = 13;
	const CONFIRM: i32 = 16;
	const REJECT: i32 = 17;
	const TOGGLE_ON: i32 = 21;
	const TOGGLE_OFF: i32 = 22;
	const GESTURE_THRESHOLD_ACTIVATE: i32 = 23;
	const GESTURE_THRESHOLD_DEACTIVATE: i32 = 24;
	const DRAG_START: i32 = 25;
	const SEGMENT_TICK: i32 = 26;
	const SEGMENT_FREQUENT_TICK: i32 = 27;
	const NO_HAPTICS: i32 = -1;

	pub fn invoke(
		platform: &PlatformContext,
		method: MethodId,
		payload: &[u8],
	) -> Result<(), Error> {
		let (vm_ptr, context_ptr) = platform.android_handles().ok_or(Error::Unavailable)?;
		// SAFETY: The Android host lends its process `JavaVM*` for the
		// lifetime of `PlatformContext`.
		let vm = unsafe { JavaVM::from_raw(vm_ptr.cast()).map_err(|_| Error::Unavailable)? };
		let mut env = vm
			.attach_current_thread_as_daemon()
			.map_err(|_| Error::Native)?;

		env.with_local_frame(16, |env| -> jni::errors::Result<Result<(), Error>> {
			Ok((|| {
				ensure_main_looper(env)?;
				let sdk_int = env
					.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")
					.and_then(jni::objects::JValueOwned::i)
					.unwrap_or(26);
				let constant = resolve_constant(method, payload, sdk_int)?;

				// SAFETY: `context_ptr` is a valid JNI global reference
				// retained by `PlatformContext`;
				// `new_local_ref` creates a frame-scoped
				// reference without consuming the global reference.
				let borrowed = unsafe { JObject::from_raw(context_ptr.cast()) };
				let context = env.new_local_ref(&borrowed).map_err(|_| Error::Native)?;

				if !env
					.is_instance_of(&context, "android/app/Activity")
					.unwrap_or(false)
				{
					return Err(Error::Unavailable);
				}

				let window = env
					.call_method(&context, "getWindow", "()Landroid/view/Window;", &[])
					.map_err(|_| clear_and_fail(env))?
					.l()
					.map_err(|_| Error::Native)?;

				if window.is_null() {
					return Err(Error::Unavailable);
				}

				let decor = env
					.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])
					.map_err(|_| clear_and_fail(env))?
					.l()
					.map_err(|_| Error::Native)?;

				if decor.is_null() {
					return Err(Error::Unavailable);
				}

				env.call_method(
					&decor,
					"performHapticFeedback",
					"(I)Z",
					&[JValue::Int(constant)],
				)
				.map_err(|_| clear_and_fail(env))?;
				Ok(())
			})())
		})
		.map_err(|_| Error::Native)?
	}

	fn ensure_main_looper(env: &mut JNIEnv<'_>) -> Result<(), Error> {
		let my_looper = env
			.call_static_method(
				"android/os/Looper",
				"myLooper",
				"()Landroid/os/Looper;",
				&[],
			)
			.map_err(|_| clear_and_fail(env))?
			.l()
			.map_err(|_| Error::Native)?;
		let main_looper = env
			.call_static_method(
				"android/os/Looper",
				"getMainLooper",
				"()Landroid/os/Looper;",
				&[],
			)
			.map_err(|_| clear_and_fail(env))?
			.l()
			.map_err(|_| Error::Native)?;

		if my_looper.is_null()
			|| !env
				.is_same_object(&my_looper, &main_looper)
				.unwrap_or(false)
		{
			return Err(Error::Native);
		}

		Ok(())
	}

	fn resolve_constant(method: MethodId, payload: &[u8], sdk_int: i32) -> Result<i32, Error> {
		if method == ANDROID_FEEDBACK {
			let raw = i32::from_le_bytes(payload.try_into().map_err(|_| Error::InvalidArgument)?);
			let minimum = match raw {
				LONG_PRESS | VIRTUAL_KEY | KEYBOARD_TAP | CLOCK_TICK | CONTEXT_CLICK => 26,
				KEYBOARD_RELEASE | VIRTUAL_KEY_RELEASE | TEXT_HANDLE_MOVE => 27,
				GESTURE_START | GESTURE_END | CONFIRM | REJECT => 30,
				TOGGLE_ON
				| TOGGLE_OFF
				| GESTURE_THRESHOLD_ACTIVATE
				| GESTURE_THRESHOLD_DEACTIVATE
				| DRAG_START
				| SEGMENT_TICK
				| SEGMENT_FREQUENT_TICK
				| NO_HAPTICS => 34,
				_ => return Err(Error::InvalidArgument),
			};

			return if sdk_int >= minimum {
				Ok(raw)
			} else {
				Err(Error::Unavailable)
			};
		}

		match decode(method, payload)? {
			Effect::Selection | Effect::Impact(Impact::Soft) => Ok(CLOCK_TICK),
			Effect::Impact(Impact::Light) => Ok(KEYBOARD_TAP),
			Effect::Impact(Impact::Medium) => Ok(VIRTUAL_KEY),
			Effect::Impact(Impact::Heavy) | Effect::Notification(Notification::Warning) => {
				Ok(LONG_PRESS)
			}
			Effect::Impact(Impact::Rigid) => Ok(CONTEXT_CLICK),
			Effect::Notification(Notification::Success) => {
				Ok(if sdk_int >= 30 { CONFIRM } else { VIRTUAL_KEY })
			}
			Effect::Notification(Notification::Error) => {
				Ok(if sdk_int >= 30 { REJECT } else { LONG_PRESS })
			}
		}
	}

	fn clear_and_fail(env: &mut JNIEnv<'_>) -> Error {
		if env.exception_check().unwrap_or(false) {
			let _ = env.exception_clear();
		}

		Error::Native
	}
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
		let context = AppContext::new(
			std::path::PathBuf::from("/tmp"),
			std::path::PathBuf::from("/tmp"),
		);
		let mut haptics = from_context(&context).unwrap_or_default();
		assert_eq!(haptics.name(), NAME);
		assert_eq!(haptics.methods(), METHODS);
		assert_eq!(
			haptics.invoke(IMPACT, Bytes::from(vec![9, 0, 0, 0])),
			Err(Error::InvalidArgument)
		);
		// Test threads are not AppKit's main thread, and other desktops have no
		// actuator: a valid effect is rejected without touching hardware.
		let expected = if cfg!(any(target_os = "macos", target_os = "ios")) {
			Error::InvalidArgument
		} else {
			Error::Unavailable
		};

		assert_eq!(haptics.invoke(SELECTION, Bytes::new()), Err(expected));
	}
}
