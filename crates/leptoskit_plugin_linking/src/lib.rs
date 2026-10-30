//! External HTTP(S), email, phone and SMS links, without executing shell text.
//!
//! Incoming deep links require host lifecycle integration and are not handled
//! by this crate. Application-specific schemes and file URLs are intentionally
//! not accepted by the default policy. Percent-encode non-ASCII text before
//! parsing.

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Stable plugin registration name.
pub const NAME: &str = "linking";
/// UTF-8 URL payload; empty success response.
pub const OPEN: MethodId = MethodId(0);
/// UTF-8 URL payload; one little-endian u32 boolean response.
pub const CAN_OPEN: MethodId = MethodId(1);
/// Empty payload and response; opens this application's native settings.
pub const OPEN_SETTINGS: MethodId = MethodId(2);
/// Exported methods, independently grantable by the host.
pub const METHODS: &[MethodId] = &[OPEN, CAN_OPEN, OPEN_SETTINGS];
/// Maximum encoded URL length, applied before parsing on every platform.
pub const MAX_URL_BYTES: usize = 8192;

/// An absolute URL accepted by the plugin's default scheme policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link(url::Url);

impl Link {
	/// Parses an HTTP(S), mailto, tel or sms URL. Rejects credentials,
	/// malformed escapes, whitespace, controls, backslashes and empty
	/// destinations.
	pub fn parse(value: &str) -> Result<Self, Error> {
		if value.is_empty()
			|| value.len() > MAX_URL_BYTES
			|| value
				.bytes()
				.any(|byte| !byte.is_ascii_graphic() || byte == b'\\')
		{
			return Err(Error::InvalidArgument);
		}
		let bytes = value.as_bytes();
		for (index, byte) in bytes.iter().enumerate() {
			if *byte == b'%' {
				let escape = bytes
					.get(index + 1..index + 3)
					.ok_or(Error::InvalidArgument)?;
				if !escape.iter().all(u8::is_ascii_hexdigit) {
					return Err(Error::InvalidArgument);
				}
				if matches!(escape[0], b'0' | b'1') || escape.eq_ignore_ascii_case(b"7f") {
					return Err(Error::InvalidArgument);
				}
			}
		}
		let parsed = url::Url::parse(value).map_err(|_| Error::InvalidArgument)?;
		match parsed.scheme() {
			"http" | "https" => {
				let (_, destination) = value.split_once(':').ok_or(Error::InvalidArgument)?;
				if !destination.starts_with("//")
					|| destination.starts_with("///")
					|| destination
						.trim_start_matches('/')
						.split(['/', '?', '#'])
						.next()
						.is_some_and(|authority| authority.contains('@'))
					|| parsed.host_str().is_none()
					|| !parsed.username().is_empty()
					|| parsed.password().is_some()
				{
					return Err(Error::InvalidArgument);
				}
			}
			"mailto" | "tel" | "sms"
				if !parsed.path().is_empty() && !parsed.path().starts_with('/') => {}
			_ => return Err(Error::InvalidArgument),
		}
		Ok(Self(parsed))
	}

	/// Canonical URL to pass as one native argument, never shell source.
	#[must_use]
	pub fn as_str(&self) -> &str {
		self.0.as_str()
	}

	/// Validates an incoming untrusted bridge payload.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		Self::parse(core::str::from_utf8(payload).map_err(|_| Error::InvalidArgument)?)
	}
}

/// Decodes the exact four-byte boolean response; unknown values are invalid.
pub fn decode_can_open(payload: &[u8]) -> Result<bool, Error> {
	match payload {
		[0, 0, 0, 0] => Ok(false),
		[1, 0, 0, 0] => Ok(true),
		_ => Err(Error::InvalidArgument),
	}
}

/// Desktop platform adapter. Opening may start and wait for a system helper;
/// invoke off the UI thread where launcher latency would block rendering.
#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
pub mod desktop {
	use leptoskit_bridge::Bytes;
	use leptoskit_bridge::Plugin;

	use super::*;

	/// Stateless adapter for manual desktop runtime registration.
	#[derive(Debug, Default)]
	pub struct Linking;

	impl Plugin for Linking {
		fn name(&self) -> &'static str {
			NAME
		}

		fn methods(&self) -> &'static [MethodId] {
			METHODS
		}

		fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
			match method {
				OPEN => {
					open(&Link::decode(&payload)?)?;
					Ok(Bytes::new())
				}
				CAN_OPEN => {
					Ok(Bytes::copy_from_slice(
						&u32::from(can_open(&Link::decode(&payload)?)?).to_le_bytes(),
					))
				}
				OPEN_SETTINGS if !payload.is_empty() => Err(Error::InvalidArgument),
				_ => Err(Error::Unavailable),
			}
		}
	}

	/// Requests the platform's default application. Success means the launcher
	/// accepted the request; it does not mean a page loaded or a call was made.
	pub fn open(link: &Link) -> Result<(), Error> {
		open::that(link.as_str()).map_err(|_| Error::Native)
	}

	/// Queries Launch Services for an installed handler on macOS.
	#[cfg(target_os = "macos")]
	pub fn can_open(link: &Link) -> Result<bool, Error> {
		use objc2_app_kit::NSWorkspace;
		use objc2_foundation::NSString;
		use objc2_foundation::NSURL;
		let url = NSURL::URLWithString(&NSString::from_str(link.as_str()))
			.ok_or(Error::InvalidArgument)?;
		Ok(NSWorkspace::sharedWorkspace()
			.URLForApplicationToOpenURL(&url)
			.is_some())
	}

	/// No portable handler discovery API is implemented for these desktops.
	#[cfg(not(target_os = "macos"))]
	pub fn can_open(_link: &Link) -> Result<bool, Error> {
		Err(Error::Unavailable)
	}
}

/// Browser behavior is subject to popup blocking and user activation.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Requests a new context with opener isolation. Call directly from a user
	/// gesture. Because `noopener` returns no window handle, success only means
	/// the browser accepted the API call; it cannot confirm a popup opened.
	pub fn open(link: &Link) -> Result<(), Error> {
		web_sys::window()
			.ok_or(Error::Unavailable)?
			.open_with_url_and_target_and_features(link.as_str(), "_blank", "noopener,noreferrer")
			.map_err(|_| Error::Native)?;
		Ok(())
	}

	/// Browsers do not expose installed external URL handlers.
	pub fn can_open(_link: &Link) -> Result<bool, Error> {
		Err(Error::Unavailable)
	}
}

/// Cached native bridge client for an interpreted Wasm guest.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct Linking(leptoskit_bridge::guest::Client);
	impl Linking {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Opens a validated link using the operating system.
		pub async fn open(self, link: &Link) -> Result<(), Error> {
			let response = self.0.call(OPEN, link.as_str().as_bytes())?.await?;
			if response.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}

		/// Queries installed handlers, subject to native query declarations.
		pub async fn can_open(self, link: &Link) -> Result<bool, Error> {
			decode_can_open(&self.0.call(CAN_OPEN, link.as_str().as_bytes())?.await?)
		}

		/// Opens the application's settings page on iOS or Android.
		pub async fn open_settings(self) -> Result<(), Error> {
			let response = self.0.call(OPEN_SETTINGS, &[])?.await?;
			if response.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn validates_external_destinations_without_executing_them() {
		for value in [
			"https://example.com/path?q=a%20b",
			"http://[::1]:8080/",
			"mailto:a@example.com",
			"tel:+123",
			"sms:+123?body=hi",
			"HTTPS://EXAMPLE.COM",
		] {
			assert!(Link::parse(value).is_ok(), "{value}");
		}
	}
	#[test]
	fn rejects_executable_schemes_malformed_urls_and_control_bytes() {
		for value in [
			"",
			"//example.com",
			"javascript:alert(1)",
			"file:///etc/passwd",
			"data:text/plain,a",
			"intent://x",
			"https:",
			"https:example.com",
			"https:///example.com",
			"https://user:pass@example.com",
			"https://@example.com",
			"https://a\\b",
			"https://a\n",
			"https://a/%00",
			"https://a/%1f",
			"https://a/%7F",
			"https://a/%",
			"https://a/%ab%xy",
			"mailto:",
			"tel:",
			"sms://example",
			"https://é.com",
		] {
			assert_eq!(Link::parse(value), Err(Error::InvalidArgument), "{value}");
		}
		assert_eq!(Link::decode(&[0xff]), Err(Error::InvalidArgument));
		assert_eq!(
			Link::parse(&format!("https://a/{}", "a".repeat(MAX_URL_BYTES))),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn boolean_wire_responses_are_not_truthy_coercions() {
		assert_eq!(decode_can_open(&[1, 0, 0, 0]), Ok(true));
		assert_eq!(decode_can_open(&[0, 0, 0, 0]), Ok(false));
		for value in [&[][..], &[1], &[2, 0, 0, 0], &[1, 0, 0, 0, 0]] {
			assert_eq!(decode_can_open(value), Err(Error::InvalidArgument));
		}
	}

	#[cfg(any(
		target_os = "macos",
		target_os = "windows",
		all(target_os = "linux", not(target_env = "ohos"))
	))]
	#[test]
	fn desktop_adapter_validates_before_any_launcher_runs() {
		use leptoskit_bridge::Bytes;
		use leptoskit_bridge::Plugin;
		let mut linking = desktop::Linking;
		assert_eq!(linking.name(), NAME);
		assert_eq!(linking.methods(), METHODS);
		assert_eq!(
			Link::parse("https://example.com").unwrap().as_str(),
			"https://example.com/"
		);
		// Rejected destinations fail before the OS launcher is involved.
		for method in [OPEN, CAN_OPEN] {
			assert_eq!(
				linking.invoke(method, Bytes::from_static(b"javascript:alert(1)")),
				Err(Error::InvalidArgument)
			);
		}
		assert_eq!(
			linking.invoke(OPEN_SETTINGS, Bytes::from_static(b"x")),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			linking.invoke(OPEN_SETTINGS, Bytes::new()),
			Err(Error::Unavailable)
		);
		assert_eq!(
			linking.invoke(MethodId(9), Bytes::new()),
			Err(Error::Unavailable)
		);
		// Querying handlers never opens anything. macOS answers from Launch
		// Services; other desktops have no portable discovery API.
		let reply = linking.invoke(CAN_OPEN, Bytes::from_static(b"https://example.com"));
		if cfg!(target_os = "macos") {
			assert!(decode_can_open(&reply.unwrap()).is_ok());
		} else {
			assert_eq!(reply, Err(Error::Unavailable));
		}
	}
}
