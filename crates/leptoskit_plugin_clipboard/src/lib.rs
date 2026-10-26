//! Text clipboard access through native OS APIs or the browser Clipboard API.
//!
//! Browser operations require a secure context and may require a user gesture.
//! Apple and Android native implementations are shipped alongside this crate.
//! Keep a desktop `Clipboard` alive while its contents should be available:
//! on X11 the clipboard's owner must stay alive.

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

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod desktop {
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
		use super::*;

		// These use the real system clipboard: CI runs Linux under Xvfb, and
		// macOS and Windows on their desktops. One test owns every write so
		// parallel test processes never race on the shared clipboard, and it
		// restores the text that was there before.
		#[test]
		fn text_round_trips_through_the_system_clipboard_and_clears() {
			let mut clipboard = Clipboard::new().expect("a desktop clipboard is available");
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

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub use desktop::Clipboard;

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
