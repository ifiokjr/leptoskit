//! Imports one user-selected document as bounded owned bytes.
//!
//! Native hosts install the Swift/Kotlin adapter with their owning presenter.
//! The guest receives no filesystem path, persistent URI grant, or bookmark.
//! Cancellation is an error, including when the host closes during selection.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::codec::Decoder;

/// Runtime capability name.
pub const NAME: &str = "document-picker";
/// Select and read one document. Request: u32 limit. Reply: length-prefixed
/// UTF-8 name, MIME type (empty when unknown), then raw length-prefixed bytes.
pub const PICK: u32 = 0;
/// Maximum content size. Framing and metadata are additional, bounded bytes.
pub const MAX_BYTES: u32 = 4 * 1024 * 1024;
/// Maximum UTF-8 bytes in each metadata field.
pub const MAX_METADATA_BYTES: usize = 4096;

/// Validated content limit; zero allows only empty documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
	max_bytes: u32,
}
impl Options {
	/// Rejects limits above the plugin's fixed allocation budget.
	pub fn new(max_bytes: u32) -> Result<Self, Error> {
		if max_bytes > MAX_BYTES {
			return Err(Error::TooLarge);
		}

		Ok(Self { max_bytes })
	}

	/// Maximum selected content bytes.
	pub fn max_bytes(self) -> u32 {
		self.max_bytes
	}

	/// Stable request wire representation.
	pub fn encode(self) -> [u8; 4] {
		self.max_bytes.to_le_bytes()
	}
}
impl Default for Options {
	fn default() -> Self {
		Self {
			max_bytes: MAX_BYTES,
		}
	}
}

/// A copied document with no retained OS access rights.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
	/// Provider display name, untrusted text; never use directly as a path.
	pub name: String,
	/// Provider MIME type, or None when unknown. Does not validate content.
	pub mime_type: Option<String>,
	/// Binary content. Native response decoding shares the reply allocation.
	pub bytes: Bytes,
}
impl Document {
	/// Validates framing and bounds before exposing a native reply.
	pub fn decode(mut reply: Bytes, options: Options) -> Result<Self, Error> {
		let mut decoder = Decoder::new(&reply);
		let name = decoder.string()?;
		let mime_type = decoder.string()?;

		if name.is_empty()
			|| name.len() > MAX_METADATA_BYTES
			|| mime_type.len() > MAX_METADATA_BYTES
		{
			return Err(Error::InvalidArgument);
		}
		let bytes = decoder.bytes()?;

		if bytes.len() > options.max_bytes as usize {
			return Err(Error::TooLarge);
		}

		let offset = 12 + name.len() + mime_type.len();
		let length = bytes.len();
		decoder.finish()?;
		let name = name.to_owned();
		let mime_type = (!mime_type.is_empty()).then(|| mime_type.to_owned());
		reply = reply.split_off(offset);
		reply.truncate(length);
		Ok(Self {
			name,
			mime_type,
			bytes: reply,
		})
	}
}

/// Selects using the native system picker registered by the host.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub async fn pick(options: Options) -> Result<Document, Error> {
	let client = leptoskit_bridge::guest::Client::new(NAME)?;
	let reply = client
		.call(leptoskit_bridge::MethodId(PICK), &options.encode())?
		.await?;
	Document::decode(reply, options)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod web;

#[cfg(test)]
mod tests {
	use leptoskit_bridge::codec::Encoder;

	use super::*;
	fn reply(name: &str, data: &[u8]) -> Bytes {
		let mut out = Encoder::default();
		out.bytes(name.as_bytes()).unwrap();
		out.bytes(b"text/plain").unwrap();
		out.bytes(data).unwrap();
		out.finish()
	}
	#[test]
	fn decode_shares_content_and_enforces_callers_limit() {
		let bytes = reply("example.txt", b"abc");
		let document = Document::decode(bytes.clone(), Options::new(3).unwrap()).unwrap();
		assert_eq!(document.bytes, b"abc"[..]);

		assert_eq!(document.bytes.as_ptr(), bytes[33..].as_ptr());
		assert_eq!(
			Document::decode(bytes, Options::new(2).unwrap()),
			Err(Error::TooLarge)
		);
	}
	#[test]
	fn rejects_invalid_limits_truncation_trailing_and_metadata() {
		assert_eq!(Options::new(MAX_BYTES + 1), Err(Error::TooLarge));
		let good = reply("a", b"");

		for len in 0..good.len() {
			assert!(Document::decode(good.slice(..len), Options::default()).is_err());
		}

		let mut trailing = good.to_vec();
		trailing.push(0);
		assert!(Document::decode(trailing.into(), Options::default()).is_err());
		assert!(Document::decode(reply("", b""), Options::default()).is_err());
		assert!(
			Document::decode(
				reply(&"a".repeat(MAX_METADATA_BYTES + 1), b""),
				Options::default()
			)
			.is_err()
		);
		assert!(Document::decode(good, Options::new(0).unwrap()).is_ok());
		assert_eq!(Options::default().max_bytes(), MAX_BYTES);
		assert_eq!(Options::default().encode(), MAX_BYTES.to_le_bytes());
	}
}

#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
pub mod desktop;
