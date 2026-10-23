//! Shares bounded text or an HTTP(S) URL through an explicit user-selected UI.
//!
//! Successful completion only means the platform finished its API callback.
//! It never confirms delivery or identifies a recipient. Android cannot
//! reliably distinguish chooser dismissal from choosing an application.
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime capability name.
pub const NAME: &str = "sharing";
/// Present native share UI. Reply must be empty, without delivery claims.
pub const SHARE: MethodId = MethodId(0);
/// Maximum UTF-8 text size, before native string conversion.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Maximum ASCII encoded URL length.
pub const MAX_URL_BYTES: usize = 8192;

/// Validated content with private representation, shared across native and web.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Content {
	kind: u32,
	value: String,
}
impl Content {
	/// Preserves nonempty UTF-8 text, including newlines. NUL is rejected.
	pub fn text(value: impl Into<String>) -> Result<Self, Error> {
		let value = value.into();
		if value.len() > MAX_TEXT_BYTES {
			return Err(Error::TooLarge);
		}
		if value.is_empty() || value.contains('\0') {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { kind: 0, value })
	}

	/// Accepts only absolute HTTP(S) URLs without credentials, controls,
	/// whitespace, backslashes, or malformed percent escapes. Percent-encode
	/// Unicode before calling. Native URL parsers may reject a stricter subset,
	/// such as encoded hostnames or abbreviated IPv4 addresses on Android.
	pub fn url(value: impl Into<String>) -> Result<Self, Error> {
		let value = value.into();
		if value.len() > MAX_URL_BYTES {
			return Err(Error::TooLarge);
		}
		let authority = value
			.strip_prefix("https://")
			.or_else(|| value.strip_prefix("http://"))
			.ok_or(Error::InvalidArgument)?
			.split(['/', '?', '#'])
			.next()
			.ok_or(Error::InvalidArgument)?;
		if authority.is_empty()
			|| authority.contains('@')
			|| value
				.bytes()
				.any(|byte| !byte.is_ascii_graphic() || byte == b'\\')
		{
			return Err(Error::InvalidArgument);
		}
		for (index, byte) in value.bytes().enumerate() {
			if byte == b'%'
				&& !value
					.as_bytes()
					.get(index + 1..index + 3)
					.is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))
			{
				return Err(Error::InvalidArgument);
			}
		}
		let parsed = url::Url::parse(&value).map_err(|_| Error::InvalidArgument)?;
		if parsed.host_str().is_none()
			|| !parsed.username().is_empty()
			|| parsed.password().is_some()
		{
			return Err(Error::InvalidArgument);
		}
		Ok(Self { kind: 1, value })
	}

	/// Original validated content; never a filesystem path or recipient.
	pub fn as_str(&self) -> &str {
		&self.value
	}

	/// Whether this content was explicitly validated as a web URL.
	pub fn is_url(&self) -> bool {
		self.kind == 1
	}

	/// Wire: little-endian kind (0 text, 1 URL), then length-prefixed UTF-8.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut encoder = Encoder::with_capacity(self.value.len() + 8);
		encoder.u32(self.kind);
		encoder.bytes(self.value.as_bytes())?;
		Ok(encoder.finish())
	}

	/// Validates untrusted guest payloads; rejects unknown kinds/trailing data.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_TEXT_BYTES + 8 {
			return Err(Error::TooLarge);
		}
		let mut decoder = Decoder::new(payload);
		let kind = decoder.u32()?;
		let value = decoder.string()?;
		decoder.finish()?;
		match kind {
			0 => Self::text(value),
			1 => Self::url(value),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Opens the installed native share UI; never confirms message delivery.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub async fn share(content: &Content) -> Result<(), Error> {
	let reply = leptoskit_bridge::guest::Client::new(NAME)?
		.call(SHARE, &content.encode()?)?
		.await?;
	if reply.is_empty() {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod web;

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn content_bounds_and_framing() {
		for content in [
			Content::text("hello\n世界").unwrap(),
			Content::url("https://example.test/a%20b?q=one#two").unwrap(),
		] {
			let bytes = content.encode().unwrap();
			assert_eq!(Content::decode(&bytes).unwrap(), content);
			for end in 0..bytes.len() {
				assert!(Content::decode(&bytes[..end]).is_err());
			}
			let mut trailing = bytes.to_vec();
			trailing.push(0);
			assert!(Content::decode(&trailing).is_err());
		}
		assert_eq!(Content::text(""), Err(Error::InvalidArgument));
		assert_eq!(Content::text("a\0b"), Err(Error::InvalidArgument));
		assert_eq!(
			Content::text("a".repeat(MAX_TEXT_BYTES + 1)),
			Err(Error::TooLarge)
		);
		assert!(Content::text("a".repeat(MAX_TEXT_BYTES)).is_ok());
	}
	#[test]
	fn urls_do_not_grant_files_or_choose_recipients() {
		for value in [
			"file:///secret",
			"content://provider/item",
			"mailto:recipient@example.test",
			"javascript:alert(1)",
			"https://user:password@example.test",
			"https:///no-host",
			"https://example.test/a b",
			"https://example.test/%no",
			"https://example.test\\evil",
			"https://",
		] {
			assert!(Content::url(value).is_err(), "{value}");
		}
	}
	#[test]
	fn accessors_and_untrusted_payload_limits() {
		let url = Content::url("http://example.test:8080/path").unwrap();
		assert!(url.is_url());
		assert_eq!(url.as_str(), "http://example.test:8080/path");
		let text = Content::text("https://not-validated-as-a-url").unwrap();
		assert!(!text.is_url());
		assert_eq!(text.as_str(), "https://not-validated-as-a-url");
		let long = format!("https://example.test/{}", "a".repeat(MAX_URL_BYTES));
		assert_eq!(Content::url(long), Err(Error::TooLarge));
		// Passes the character checks but is not a parseable URL.
		assert_eq!(
			Content::url("https://[::1/path"),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Content::decode(&vec![0; MAX_TEXT_BYTES + 9]),
			Err(Error::TooLarge)
		);
		let mut unknown = Encoder::default();
		unknown.u32(2);
		unknown.bytes(b"hello").unwrap();
		assert_eq!(
			Content::decode(&unknown.finish()),
			Err(Error::InvalidArgument)
		);
	}
}
