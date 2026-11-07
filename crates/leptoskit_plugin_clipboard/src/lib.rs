//! <!-- {=p_clipboardSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_clipboard` as a direct dependency and select the
//! `clipboard` plugin in `native.plugins`; supported generated hosts discover
//! it from Cargo metadata. Generated Windows, Linux, and terminal hosts
//! currently omit runtime plugins; use explicit registration in a custom host
//! there. Guests enable the `guest` feature and reuse `guest::Clipboard` rather
//! than resolving the plugin on every call. Only plain text is supported;
//! browser operations need a secure context and may need a user gesture.
//!
//! <!-- {/p_clipboardSummary} -->
//!
//! Browser operations require a secure context and may require a user gesture.
//! Apple and Android native implementations are shipped alongside this crate,
//! and experimental pure-Rust implementations (`objc2-ui-kit` on iOS,
//! `arboard` on macOS, `jni` on Android) are selected when
//! `native.experimental-rust-plugins = true`.
//! Keep a desktop `Clipboard` alive while its contents should be available:
//! on X11 the clipboard's owner must stay alive.
#![cfg_attr(any(target_os = "ios", target_os = "android"), allow(unsafe_code))]

use leptoskit_bridge::MethodId;

/// Empty input -> UTF-8 text (empty when no text is available).
pub const READ_TEXT: MethodId = MethodId(0);
/// Raw UTF-8 input -> empty success response.
pub const WRITE_TEXT: MethodId = MethodId(1);
/// Empty input -> empty success response; clears text.
pub const CLEAR: MethodId = MethodId(2);
/// Stable native registration name.
pub const NAME: &str = "clipboard";
/// Supported text operations.
pub const METHODS: &[MethodId] = &[READ_TEXT, WRITE_TEXT, CLEAR];

/// Implements [`Plugin`](leptoskit_bridge::Plugin) for a native backend's
/// `read_text`, `write_text`, and `clear`, mapping their errors with
/// `$map_error`.
#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos")),
	target_os = "ios",
	target_os = "android"
))]
macro_rules! text_plugin {
	($clipboard:ty, $map_error:expr) => {
		impl leptoskit_bridge::Plugin for $clipboard {
			fn name(&self) -> &'static str {
				NAME
			}

			fn methods(&self) -> &'static [MethodId] {
				METHODS
			}

			fn invoke(
				&mut self,
				method: MethodId,
				payload: leptoskit_bridge::Bytes,
			) -> Result<leptoskit_bridge::Bytes, leptoskit_bridge::Error> {
				use leptoskit_bridge::Bytes;
				use leptoskit_bridge::Error;

				match method {
					READ_TEXT if payload.is_empty() => {
						self.read_text().map(Bytes::from).map_err($map_error)
					}

					WRITE_TEXT => {
						let text =
							std::str::from_utf8(&payload).map_err(|_| Error::InvalidArgument)?;
						self.write_text(text).map_err($map_error)?;
						Ok(Bytes::new())
					}

					CLEAR if payload.is_empty() => {
						self.clear().map_err($map_error)?;
						Ok(Bytes::new())
					}

					READ_TEXT | CLEAR => Err(Error::InvalidArgument),
					_ => Err(Error::Unavailable),
				}
			}
		}
	};
}

#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
mod desktop {
	use leptoskit_bridge::AppContext;
	use leptoskit_bridge::Error;

	use super::*;

	/// Persistent native clipboard ownership, including X11's selection owner.
	pub struct Clipboard(arboard::Clipboard);

	impl Clipboard {
		/// Connects to the desktop clipboard.
		pub fn new() -> Result<Self, arboard::Error> {
			arboard::Clipboard::new().map(Self)
		}

		/// Reads text, returning an empty string when there is no text content.
		pub fn read_text(&mut self) -> Result<String, arboard::Error> {
			match self.0.get_text() {
				Ok(text) => Ok(text),
				Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
				Err(error) => Err(error),
			}
		}

		/// Replaces clipboard text.
		pub fn write_text(&mut self, text: &str) -> Result<(), arboard::Error> {
			self.0.set_text(text)
		}

		/// Clears clipboard content.
		pub fn clear(&mut self) -> Result<(), arboard::Error> {
			self.0.clear()
		}
	}

	/// Constructs the desktop clipboard plugin from the host [`AppContext`].
	pub fn from_context(_context: &AppContext) -> Result<Clipboard, Error> {
		Clipboard::new().map_err(|error| map_error(&error))
	}

	fn map_error(error: &arboard::Error) -> Error {
		match error {
			arboard::Error::ClipboardOccupied => Error::Busy,
			arboard::Error::ClipboardNotSupported => Error::Unavailable,
			_ => Error::Native,
		}
	}

	text_plugin!(Clipboard, |error| map_error(&error));

	#[cfg(test)]
	mod tests {
		use std::path::PathBuf;

		use leptoskit_bridge::Bytes;
		use leptoskit_bridge::Plugin;

		use super::*;

		// These use the real system clipboard: CI runs Linux under Xvfb, and
		// macOS and Windows on their desktops. One test owns every write so
		// parallel test processes never race on the shared clipboard, and it
		// restores the text that was there before.
		#[test]
		fn text_round_trips_through_the_system_clipboard_and_clears() {
			let context = AppContext::new(PathBuf::from("/tmp"), PathBuf::from("/tmp"));
			let mut clipboard = from_context(&context).expect("a desktop clipboard is available");
			let previous = clipboard.read_text().expect("read the current text");
			let text = "LeptosKit clipboard \u{2713}";
			let outcome = (|| {
				assert_eq!(clipboard.name(), NAME);
				assert_eq!(clipboard.methods(), METHODS);
				assert!(
					clipboard
						.invoke(WRITE_TEXT, Bytes::from(text.as_bytes().to_vec()))?
						.is_empty()
				);
				assert_eq!(
					clipboard.invoke(READ_TEXT, Bytes::new())?.as_ref(),
					text.as_bytes()
				);
				assert!(clipboard.invoke(CLEAR, Bytes::new())?.is_empty());
				assert_eq!(
					clipboard.read_text().map_err(|error| map_error(&error))?,
					""
				);
				assert!(clipboard.invoke(READ_TEXT, Bytes::new())?.is_empty());
				Ok::<(), Error>(())
			})();

			if !previous.is_empty() {
				clipboard
					.write_text(&previous)
					.expect("restore the previous text");
			}

			outcome.expect("clipboard operations succeed");
		}

		#[test]
		fn malformed_requests_fail_before_reaching_the_clipboard() {
			let mut clipboard = Clipboard::new().expect("a desktop clipboard is available");
			assert_eq!(
				clipboard.invoke(READ_TEXT, Bytes::from(vec![1])),
				Err(Error::InvalidArgument)
			);
			assert_eq!(
				clipboard.invoke(CLEAR, Bytes::from(vec![1])),
				Err(Error::InvalidArgument)
			);
			assert_eq!(
				clipboard.invoke(WRITE_TEXT, Bytes::from(vec![0xff, 0xfe])),
				Err(Error::InvalidArgument)
			);
			assert_eq!(
				clipboard.invoke(MethodId(3), Bytes::new()),
				Err(Error::Unavailable)
			);
		}

		#[test]
		fn backend_failures_map_to_bridge_statuses() {
			assert_eq!(map_error(&arboard::Error::ClipboardOccupied), Error::Busy);
			assert_eq!(
				map_error(&arboard::Error::ClipboardNotSupported),
				Error::Unavailable
			);
			assert_eq!(
				map_error(&arboard::Error::ContentNotAvailable),
				Error::Native
			);
			assert_eq!(
				map_error(&arboard::Error::Unknown {
					description: "backend".to_owned()
				}),
				Error::Native
			);
		}
	}
}

#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
pub use desktop::Clipboard;
#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
pub use desktop::from_context;

#[cfg(target_os = "ios")]
mod ios {
	use leptoskit_bridge::AppContext;
	use leptoskit_bridge::Error;
	use objc2_foundation::NSArray;
	use objc2_foundation::NSDictionary;
	use objc2_foundation::NSString;
	use objc2_ui_kit::UIPasteboard;

	use super::*;

	/// Pure-Rust iOS clipboard backed by `UIPasteboard.generalPasteboard`.
	#[derive(Debug, Default)]
	pub struct Clipboard;

	impl Clipboard {
		/// Connects to `UIPasteboard.generalPasteboard`.
		#[must_use]
		pub fn new() -> Self {
			Self
		}

		/// Reads UTF-8 text from the general pasteboard, returning an empty
		/// string when no text is present.
		pub fn read_text(&mut self) -> Result<String, Error> {
			// SAFETY: `UIPasteboard.generalPasteboard.string` returns an
			// optional retained `NSString` from the system pasteboard.
			let text = unsafe { UIPasteboard::generalPasteboard().string() };
			Ok(text.map(|value| value.to_string()).unwrap_or_default())
		}

		/// Replaces the general pasteboard's string.
		pub fn write_text(&mut self, text: &str) -> Result<(), Error> {
			let value = NSString::from_str(text);
			// SAFETY: `value` is a valid `NSString` copied by `UIPasteboard`.
			unsafe { UIPasteboard::generalPasteboard().setString(Some(&value)) };
			Ok(())
		}

		/// Clears all items on the general pasteboard.
		pub fn clear(&mut self) -> Result<(), Error> {
			let empty = NSArray::<NSDictionary<NSString, objc2::runtime::AnyObject>>::new();
			// SAFETY: Setting an empty item array clears the pasteboard just as
			// `UIPasteboard.general.items = []` does in Swift.
			unsafe { UIPasteboard::generalPasteboard().setItems(&empty) };
			Ok(())
		}
	}

	/// Constructs the iOS clipboard plugin from the host [`AppContext`].
	pub fn from_context(_context: &AppContext) -> Result<Clipboard, Error> {
		Ok(Clipboard::new())
	}

	text_plugin!(Clipboard, std::convert::identity);
}

#[cfg(target_os = "ios")]
pub use ios::Clipboard;
#[cfg(target_os = "ios")]
pub use ios::from_context;

#[cfg(target_os = "android")]
mod android {
	use std::sync::OnceLock;

	use jni::JNIEnv;
	use jni::JavaVM;
	use jni::objects::JObject;
	use jni::objects::JString;
	use jni::objects::JValue;
	use jni::objects::JValueGen;
	use leptoskit_bridge::AppContext;
	use leptoskit_bridge::Error;
	use leptoskit_bridge::PlatformContext;

	use super::*;

	const NEW_PLAIN_TEXT: &str =
		"(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;";

	/// Pure-Rust Android clipboard backed by `android.content.ClipboardManager`
	/// over JNI. Like `ClipboardPlugin.kt`, it only runs on the UI thread.
	#[derive(Debug, Clone)]
	pub struct Clipboard {
		platform: PlatformContext,
	}

	impl Clipboard {
		/// Connects to the Android host context's `ClipboardManager`.
		pub fn new(platform: PlatformContext) -> Result<Self, Error> {
			if platform.android_handles().is_none() {
				return Err(Error::Unavailable);
			}

			Ok(Self { platform })
		}

		/// Reads the first clip item's plain text, or an empty string when the
		/// clipboard holds nothing or only a URI or intent.
		pub fn read_text(&mut self) -> Result<String, Error> {
			self.with_clipboard(|env, _context, manager| {
				let clip = object(
					env,
					manager,
					"getPrimaryClip",
					"()Landroid/content/ClipData;",
					&[],
				)?;

				if clip.is_null() {
					return Ok(String::new());
				}

				let count = env
					.call_method(&clip, "getItemCount", "()I", &[])
					.and_then(JValueGen::i)
					.map_err(|_| failure(env))?;

				if count <= 0 {
					return Ok(String::new());
				}

				let item = object(
					env,
					&clip,
					"getItemAt",
					"(I)Landroid/content/ClipData$Item;",
					&[JValue::Int(0)],
				)?;

				if item.is_null() {
					return Ok(String::new());
				}

				// Only plain text. `coerceToText` would open a content URI on
				// the UI thread with no size limit, which the
				// Kotlin plugin also avoids.
				let sequence = object(env, &item, "getText", "()Ljava/lang/CharSequence;", &[])?;

				if sequence.is_null() {
					return Ok(String::new());
				}

				let text = JString::from(object(
					env,
					&sequence,
					"toString",
					"()Ljava/lang/String;",
					&[],
				)?);

				if text.is_null() {
					return Ok(String::new());
				}

				Ok(env.get_string(&text).map_err(|_| Error::Native)?.into())
			})
		}

		/// Replaces the primary clip with plain UTF-8 text.
		pub fn write_text(&mut self, text: &str) -> Result<(), Error> {
			self.with_clipboard(|env, _, manager| set_plain_text(env, manager, text))
		}

		/// Clears the primary clip on Android P (API 28+) or replaces it with
		/// an empty plain-text clip on older API levels.
		pub fn clear(&mut self) -> Result<(), Error> {
			self.with_clipboard(|env, _, manager| {
				if sdk_int(env) < 28 {
					return set_plain_text(env, manager, "");
				}

				env.call_method(manager, "clearPrimaryClip", "()V", &[])
					.map_err(|_| failure(env))?;
				Ok(())
			})
		}

		fn with_clipboard<T>(
			&self,
			operation: impl FnOnce(&mut JNIEnv<'_>, &JObject<'_>, &JObject<'_>) -> Result<T, Error>,
		) -> Result<T, Error> {
			let (vm, context) = self.platform.android_handles().ok_or(Error::Unavailable)?;
			// SAFETY: The Android host lends its process `JavaVM*` for the
			// lifetime of `PlatformContext`.
			let vm = unsafe { JavaVM::from_raw(vm.cast()) }.map_err(|_| Error::Unavailable)?;
			// The UI thread is always attached; any other thread is rejected
			// rather than attached to the VM for the rest of its life.
			let mut env = vm.get_env().map_err(|_| Error::Native)?;

			env.with_local_frame(16, |env| -> jni::errors::Result<Result<T, Error>> {
				Ok(in_frame(env, context.cast(), operation))
			})
			.map_err(|_| Error::Native)?
		}
	}

	fn in_frame<T>(
		env: &mut JNIEnv<'_>,
		context: jni::sys::jobject,
		operation: impl FnOnce(&mut JNIEnv<'_>, &JObject<'_>, &JObject<'_>) -> Result<T, Error>,
	) -> Result<T, Error> {
		ensure_main_thread(env)?;
		// SAFETY: `context` is the host's JNI global reference, kept alive by
		// `PlatformContext`; `JObject` borrows it without deleting it.
		let context = unsafe { JObject::from_raw(context) };
		let name = env.new_string("clipboard").map_err(|_| Error::Native)?;
		let manager = object(
			env,
			&context,
			"getSystemService",
			"(Ljava/lang/String;)Ljava/lang/Object;",
			&[JValue::Object(&name)],
		)?;

		if manager.is_null()
			|| !env
				.is_instance_of(&manager, "android/content/ClipboardManager")
				.unwrap_or(false)
		{
			return Err(Error::Unavailable);
		}

		operation(env, &context, &manager)
	}

	/// Replaces the primary clip with an unlabeled plain-text clip.
	fn set_plain_text(
		env: &mut JNIEnv<'_>,
		manager: &JObject<'_>,
		text: &str,
	) -> Result<(), Error> {
		let label = env.new_string("").map_err(|_| Error::Native)?;
		let text = env.new_string(text).map_err(|_| Error::Native)?;
		let clip = env
			.call_static_method(
				"android/content/ClipData",
				"newPlainText",
				NEW_PLAIN_TEXT,
				&[JValue::Object(&label), JValue::Object(&text)],
			)
			.and_then(JValueGen::l)
			.map_err(|_| failure(env))?;

		env.call_method(
			manager,
			"setPrimaryClip",
			"(Landroid/content/ClipData;)V",
			&[JValue::Object(&clip)],
		)
		.map_err(|_| failure(env))?;
		Ok(())
	}

	/// Calls a method returning an object, mapping a thrown exception.
	fn object<'local>(
		env: &mut JNIEnv<'local>,
		target: &JObject<'_>,
		name: &str,
		signature: &str,
		arguments: &[JValue<'_, '_>],
	) -> Result<JObject<'local>, Error> {
		env.call_method(target, name, signature, arguments)
			.and_then(JValueGen::l)
			.map_err(|_| failure(env))
	}

	/// Rejects calls off the UI thread, whose clipboard access Android limits.
	fn ensure_main_thread(env: &mut JNIEnv<'_>) -> Result<(), Error> {
		let main = env
			.call_static_method(
				"android/os/Looper",
				"getMainLooper",
				"()Landroid/os/Looper;",
				&[],
			)
			.and_then(JValueGen::l)
			.map_err(|_| failure(env))?;
		let current = env
			.call_method(&main, "isCurrentThread", "()Z", &[])
			.and_then(JValueGen::z)
			.map_err(|_| failure(env))?;

		if current { Ok(()) } else { Err(Error::Native) }
	}

	/// `Build.VERSION.SDK_INT`, read once per process.
	fn sdk_int(env: &mut JNIEnv<'_>) -> i32 {
		static SDK_INT: OnceLock<i32> = OnceLock::new();

		*SDK_INT.get_or_init(|| {
			env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")
				.and_then(JValueGen::i)
				.unwrap_or_else(|_| {
					failure(env);
					28
				})
		})
	}

	/// Clears a pending Java exception, reporting a `SecurityException` as
	/// [`Error::PermissionDenied`].
	fn failure(env: &mut JNIEnv<'_>) -> Error {
		let Ok(exception) = env.exception_occurred() else {
			return Error::Native;
		};

		if exception.is_null() {
			return Error::Native;
		}

		let _ = env.exception_clear();

		if env
			.is_instance_of(&exception, "java/lang/SecurityException")
			.unwrap_or(false)
		{
			Error::PermissionDenied
		} else {
			Error::Native
		}
	}

	/// Constructs the Android clipboard plugin from the host [`AppContext`].
	pub fn from_context(context: &AppContext) -> Result<Clipboard, Error> {
		Clipboard::new(context.platform().cloned().ok_or(Error::Unavailable)?)
	}

	text_plugin!(Clipboard, std::convert::identity);
}

#[cfg(target_os = "android")]
pub use android::Clipboard;
#[cfg(target_os = "android")]
pub use android::from_context;

/// Reactive clipboard text over this plugin's sources.
#[cfg(feature = "hooks")]
pub mod signals;

/// Browser backend; browser permissions and user-activation requirements apply.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use leptoskit_bridge::Error;
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;
	use wasm_bindgen_futures::JsFuture;

	fn clipboard() -> Result<web_sys::Clipboard, Error> {
		let window = web_sys::window().ok_or(Error::Unavailable)?;

		if !window.is_secure_context() {
			return Err(Error::PermissionDenied);
		}

		let clipboard = window.navigator().clipboard();
		let value: &JsValue = clipboard.as_ref();

		if value.is_null() || value.is_undefined() {
			return Err(Error::Unavailable);
		}

		Ok(clipboard)
	}

	fn failure(value: &JsValue) -> Error {
		match value
			.dyn_ref::<web_sys::DomException>()
			.map(web_sys::DomException::name)
			.as_deref()
		{
			Some("NotAllowedError" | "SecurityError") => Error::PermissionDenied,
			Some("NotSupportedError") => Error::Unavailable,
			_ => Error::Native,
		}
	}

	/// Reads text from the browser clipboard.
	pub async fn read_text() -> Result<String, Error> {
		JsFuture::from(clipboard()?.read_text())
			.await
			.map_err(|error| failure(&error))?
			.as_string()
			.ok_or(Error::Native)
	}

	/// Writes clipboard text after the browser's user-activation checks.
	pub async fn write_text(text: &str) -> Result<(), Error> {
		JsFuture::from(clipboard()?.write_text(text))
			.await
			.map_err(|error| failure(&error))?;
		Ok(())
	}

	/// Clears text by replacing the browser clipboard with an empty string.
	pub async fn clear() -> Result<(), Error> {
		write_text("").await
	}
}

/// Typed client for applications interpreted by a native host.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use leptoskit_bridge::Error;
	use leptoskit_bridge::guest::Client;

	use super::*;

	/// Resolves the native clipboard once; each operation then uses numeric
	/// IDs.
	#[derive(Debug, Clone, Copy)]
	pub struct Clipboard(Client);

	impl Clipboard {
		/// Connects to the clipboard installed in the native host.
		pub fn new() -> Result<Self, Error> {
			Client::new(NAME).map(Self)
		}

		/// Reads UTF-8 text.
		pub async fn read_text(self) -> Result<String, Error> {
			let bytes = self.0.call(READ_TEXT, &[])?.await?;
			String::from_utf8(bytes.into()).map_err(|_| Error::Native)
		}

		/// Writes text without JSON encoding.
		pub async fn write_text(self, text: &str) -> Result<(), Error> {
			self.0.call(WRITE_TEXT, text.as_bytes())?.await?;
			Ok(())
		}

		/// Clears the native clipboard.
		pub async fn clear(self) -> Result<(), Error> {
			self.0.call(CLEAR, &[])?.await?;
			Ok(())
		}
	}
}
