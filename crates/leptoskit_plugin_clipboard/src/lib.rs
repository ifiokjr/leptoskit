//! Text clipboard access through native OS APIs or the browser Clipboard API.
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

#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
mod desktop {
	use leptoskit_bridge::AppContext;
	use leptoskit_bridge::Bytes;
	use leptoskit_bridge::Error;
	use leptoskit_bridge::Plugin;

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

	impl Plugin for Clipboard {
		fn name(&self) -> &'static str {
			NAME
		}

		fn methods(&self) -> &'static [MethodId] {
			METHODS
		}

		fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
			match method {
				READ_TEXT if payload.is_empty() => {
					self.read_text()
						.map(Bytes::from)
						.map_err(|error| map_error(&error))
				}

				WRITE_TEXT => {
					let text = std::str::from_utf8(&payload).map_err(|_| Error::InvalidArgument)?;
					self.write_text(text).map_err(|error| map_error(&error))?;
					Ok(Bytes::new())
				}

				CLEAR if payload.is_empty() => {
					self.clear().map_err(|error| map_error(&error))?;
					Ok(Bytes::new())
				}

				READ_TEXT | CLEAR => Err(Error::InvalidArgument),
				_ => Err(Error::Unavailable),
			}
		}
	}

	#[cfg(test)]
	mod tests {
		use std::path::PathBuf;

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
	use leptoskit_bridge::Bytes;
	use leptoskit_bridge::Error;
	use leptoskit_bridge::Plugin;
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
			let pasteboard = UIPasteboard::generalPasteboard();
			// SAFETY: `UIPasteboard.generalPasteboard.string` returns an
			// optional retained `NSString` from the system pasteboard.
			let text = unsafe { pasteboard.string() };
			Ok(text.map(|value| value.to_string()).unwrap_or_default())
		}

		/// Replaces the general pasteboard's string.
		pub fn write_text(&mut self, text: &str) -> Result<(), Error> {
			let pasteboard = UIPasteboard::generalPasteboard();
			let value = NSString::from_str(text);
			// SAFETY: `value` is a valid `NSString` copied by `UIPasteboard`.
			unsafe {
				pasteboard.setString(Some(&value));
			}
			Ok(())
		}

		/// Clears all items on the general pasteboard.
		pub fn clear(&mut self) -> Result<(), Error> {
			let pasteboard = UIPasteboard::generalPasteboard();
			let empty: objc2::rc::Retained<
				NSArray<NSDictionary<NSString, objc2::runtime::AnyObject>>,
			> = NSArray::new();
			// SAFETY: Setting an empty item array clears the pasteboard just as
			// `UIPasteboard.general.items = []` does in Swift.
			unsafe {
				pasteboard.setItems(&empty);
			}
			Ok(())
		}
	}

	/// Constructs the iOS clipboard plugin from the host [`AppContext`].
	pub fn from_context(_context: &AppContext) -> Result<Clipboard, Error> {
		Ok(Clipboard::new())
	}

	impl Plugin for Clipboard {
		fn name(&self) -> &'static str {
			NAME
		}

		fn methods(&self) -> &'static [MethodId] {
			METHODS
		}

		fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
			match method {
				READ_TEXT if payload.is_empty() => self.read_text().map(Bytes::from),
				WRITE_TEXT => {
					let text = std::str::from_utf8(&payload).map_err(|_| Error::InvalidArgument)?;
					self.write_text(text)?;
					Ok(Bytes::new())
				}
				CLEAR if payload.is_empty() => {
					self.clear()?;
					Ok(Bytes::new())
				}
				READ_TEXT | CLEAR => Err(Error::InvalidArgument),
				_ => Err(Error::Unavailable),
			}
		}
	}
}

#[cfg(target_os = "ios")]
pub use ios::Clipboard;
#[cfg(target_os = "ios")]
pub use ios::from_context;

#[cfg(target_os = "android")]
mod android {
	use jni::JNIEnv;
	use jni::JavaVM;
	use jni::objects::JObject;
	use jni::objects::JString;
	use jni::objects::JValue;
	use leptoskit_bridge::AppContext;
	use leptoskit_bridge::Bytes;
	use leptoskit_bridge::Error;
	use leptoskit_bridge::PlatformContext;
	use leptoskit_bridge::Plugin;

	use super::*;

	/// Pure-Rust Android clipboard backed by `android.content.ClipboardManager`
	/// over JNI.
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

		/// Reads the first clip item's coerced text, or an empty string when
		/// the clipboard holds nothing.
		pub fn read_text(&mut self) -> Result<String, Error> {
			self.with_clipboard(|env, context, manager| {
				let clip = env
					.call_method(
						&manager,
						"getPrimaryClip",
						"()Landroid/content/ClipData;",
						&[],
					)
					.map_err(|error| map_jni_error(env, &error))?
					.l()
					.map_err(|_| Error::Native)?;

				if clip.is_null() {
					return Ok(String::new());
				}

				let count = env
					.call_method(&clip, "getItemCount", "()I", &[])
					.map_err(|error| map_jni_error(env, &error))?
					.i()
					.map_err(|_| Error::Native)?;

				if count <= 0 {
					return Ok(String::new());
				}

				let item = env
					.call_method(
						&clip,
						"getItemAt",
						"(I)Landroid/content/ClipData$Item;",
						&[JValue::Int(0)],
					)
					.map_err(|error| map_jni_error(env, &error))?
					.l()
					.map_err(|_| Error::Native)?;

				if item.is_null() {
					return Ok(String::new());
				}

				let sequence = env
					.call_method(
						&item,
						"coerceToText",
						"(Landroid/content/Context;)Ljava/lang/CharSequence;",
						&[JValue::Object(&context)],
					)
					.map_err(|error| map_jni_error(env, &error))?
					.l()
					.map_err(|_| Error::Native)?;

				if sequence.is_null() {
					return Ok(String::new());
				}

				let text = env
					.call_method(&sequence, "toString", "()Ljava/lang/String;", &[])
					.map_err(|error| map_jni_error(env, &error))?
					.l()
					.map_err(|_| Error::Native)?;

				if text.is_null() {
					return Ok(String::new());
				}

				let jstring = JString::from(text);
				let utf = env.get_string(&jstring).map_err(|_| Error::Native)?;
				Ok(utf.into())
			})
		}

		/// Replaces the primary clip with plain UTF-8 text.
		pub fn write_text(&mut self, text: &str) -> Result<(), Error> {
			self.with_clipboard(|env, _context, manager| {
				let label = env.new_string("").map_err(|_| Error::Native)?;
				let value = env.new_string(text).map_err(|_| Error::Native)?;
				let clip = env
					.call_static_method(
						"android/content/ClipData",
						"newPlainText",
						"(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/\
						 ClipData;",
						&[JValue::Object(&label), JValue::Object(&value)],
					)
					.map_err(|error| map_jni_error(env, &error))?
					.l()
					.map_err(|_| Error::Native)?;

				env.call_method(
					&manager,
					"setPrimaryClip",
					"(Landroid/content/ClipData;)V",
					&[JValue::Object(&clip)],
				)
				.map_err(|error| map_jni_error(env, &error))?;
				Ok(())
			})
		}

		/// Clears the primary clip on Android P (API 28+) or replaces it with
		/// an empty plain-text clip on older API levels.
		pub fn clear(&mut self) -> Result<(), Error> {
			self.with_clipboard(|env, _context, manager| {
				let sdk_int = env
					.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")
					.and_then(jni::objects::JValueOwned::i)
					.unwrap_or(28);

				if sdk_int >= 28 {
					env.call_method(&manager, "clearPrimaryClip", "()V", &[])
						.map_err(|error| map_jni_error(env, &error))?;
				} else {
					let empty = env.new_string("").map_err(|_| Error::Native)?;
					let clip = env
						.call_static_method(
							"android/content/ClipData",
							"newPlainText",
							"(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/\
							 ClipData;",
							&[JValue::Object(&empty), JValue::Object(&empty)],
						)
						.map_err(|error| map_jni_error(env, &error))?
						.l()
						.map_err(|_| Error::Native)?;

					env.call_method(
						&manager,
						"setPrimaryClip",
						"(Landroid/content/ClipData;)V",
						&[JValue::Object(&clip)],
					)
					.map_err(|error| map_jni_error(env, &error))?;
				}

				Ok(())
			})
		}

		fn with_clipboard<T>(
			&self,
			operation: impl FnOnce(&mut JNIEnv<'_>, JObject<'_>, JObject<'_>) -> Result<T, Error>,
		) -> Result<T, Error> {
			let (vm_ptr, context_ptr) =
				self.platform.android_handles().ok_or(Error::Unavailable)?;
			// SAFETY: The Android host lends its process `JavaVM*` for the
			// lifetime of `PlatformContext`.
			let vm = unsafe { JavaVM::from_raw(vm_ptr.cast()).map_err(|_| Error::Unavailable)? };
			let mut env = vm
				.attach_current_thread_as_daemon()
				.map_err(|_| Error::Native)?;

			env.with_local_frame(16, |env| -> jni::errors::Result<Result<T, Error>> {
				Ok((|| {
					// Enforce UI-thread execution just like
					// `ClipboardPlugin.kt`.
					let my_looper = env
						.call_static_method(
							"android/os/Looper",
							"myLooper",
							"()Landroid/os/Looper;",
							&[],
						)
						.map_err(|error| map_jni_error(env, &error))?
						.l()
						.map_err(|_| Error::Native)?;
					let main_looper = env
						.call_static_method(
							"android/os/Looper",
							"getMainLooper",
							"()Landroid/os/Looper;",
							&[],
						)
						.map_err(|error| map_jni_error(env, &error))?
						.l()
						.map_err(|_| Error::Native)?;

					if my_looper.is_null()
						|| !env
							.is_same_object(&my_looper, &main_looper)
							.unwrap_or(false)
					{
						return Err(Error::Native);
					}

					// SAFETY: `context_ptr` is a valid JNI global reference
					// retained by `PlatformContext`;
					// `new_local_ref` creates a frame-scoped
					// reference without consuming the global reference.
					let borrowed = unsafe { JObject::from_raw(context_ptr.cast()) };
					let context = env.new_local_ref(&borrowed).map_err(|_| Error::Native)?;

					let service_name = env.new_string("clipboard").map_err(|_| Error::Native)?;
					let service = env
						.call_method(
							&context,
							"getSystemService",
							"(Ljava/lang/String;)Ljava/lang/Object;",
							&[JValue::Object(&service_name)],
						)
						.map_err(|error| map_jni_error(env, &error))?
						.l()
						.map_err(|_| Error::Native)?;

					if service.is_null()
						|| !env
							.is_instance_of(&service, "android/content/ClipboardManager")
							.unwrap_or(false)
					{
						return Err(Error::Unavailable);
					}

					operation(env, context, service)
				})())
			})
			.map_err(|_| Error::Native)?
		}
	}

	fn map_jni_error(env: &mut JNIEnv<'_>, _error: &jni::errors::Error) -> Error {
		if env.exception_check().unwrap_or(false) {
			let thrown = env.exception_occurred().ok();
			let _ = env.exception_clear();

			if let Some(exception) = thrown
				&& env
					.is_instance_of(&exception, "java/lang/SecurityException")
					.unwrap_or(false)
			{
				return Error::PermissionDenied;
			}
		}

		Error::Native
	}

	/// Constructs the Android clipboard plugin from the host [`AppContext`].
	pub fn from_context(context: &AppContext) -> Result<Clipboard, Error> {
		let platform = context.platform().cloned().ok_or(Error::Unavailable)?;
		Clipboard::new(platform)
	}

	impl Plugin for Clipboard {
		fn name(&self) -> &'static str {
			NAME
		}

		fn methods(&self) -> &'static [MethodId] {
			METHODS
		}

		fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
			match method {
				READ_TEXT if payload.is_empty() => self.read_text().map(Bytes::from),
				WRITE_TEXT => {
					let text = std::str::from_utf8(&payload).map_err(|_| Error::InvalidArgument)?;
					self.write_text(text)?;
					Ok(Bytes::new())
				}
				CLEAR if payload.is_empty() => {
					self.clear()?;
					Ok(Bytes::new())
				}
				READ_TEXT | CLEAR => Err(Error::InvalidArgument),
				_ => Err(Error::Unavailable),
			}
		}
	}
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
