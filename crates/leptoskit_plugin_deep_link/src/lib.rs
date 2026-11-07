//! <!-- {=p_deep_linkSummary|trim|linePrefix:"//! ":true} -->
//!
//! Native hosts construct [`DeepLinks`] with the schemes and file types they
//! claim, push OS open-URL / open-file activations into it, and may expose it
//! through the bridge. There is no automatic `native.plugins` registration.
//! Outbound HTTP(S)/mailto/tel/sms opening remains
//! `leptoskit_plugin_linking`. Packaging still has to declare the associations
//! in Info.plist, the Windows registry, or a `.desktop` file; this crate holds
//! the runtime queue and the association descriptors the host/packager use.
//!
//! <!-- {/p_deep_linkSummary} -->
//!
//! Covers the inbound half of Electron's
//! [`app` open-url / open-file](https://www.electronjs.org/docs/latest/api/app)
//! and custom protocol / file association handling.

use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error as BridgeError;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Maximum queued activations. Overflow increments [`DeepLinks::lost`].
pub const EVENT_CAPACITY: usize = 64;
/// Maximum UTF-8 length of a URL or file path on the wire.
pub const MAX_TARGET_BYTES: usize = 8192;
/// Empty request -> drained activation batch.
pub const EVENTS: MethodId = MethodId(0);
/// Empty request -> allowed scheme list.
pub const SCHEMES: MethodId = MethodId(1);

/// An inbound open request delivered by the operating system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activation {
	/// A URL whose scheme the host allowed.
	Url(String),
	/// An absolute local file path the OS asked the app to open.
	File(PathBuf),
}

impl Activation {
	fn encode(&self) -> Result<Bytes, BridgeError> {
		let mut output = Encoder::with_capacity(32);
		match self {
			Self::Url(url) => {
				output.u32(1);
				output.bytes(url.as_bytes())?;
			}
			Self::File(path) => {
				let text = path.to_str().ok_or(BridgeError::InvalidArgument)?;
				output.u32(2);
				output.bytes(text.as_bytes())?;
			}
		}
		Ok(output.finish())
	}

	fn decode(bytes: &[u8]) -> Result<Self, BridgeError> {
		let mut input = Decoder::new(bytes);
		let kind = input.u32()?;
		let value = input.string()?;
		if value.is_empty() || value.len() > MAX_TARGET_BYTES {
			return Err(BridgeError::InvalidArgument);
		}
		input.finish()?;
		match kind {
			1 => Ok(Self::Url(value.to_owned())),
			2 => {
				let path = PathBuf::from(value);
				if !path.is_absolute() {
					return Err(BridgeError::InvalidArgument);
				}
				Ok(Self::File(path))
			}
			_ => Err(BridgeError::InvalidArgument),
		}
	}
}

/// A custom URL scheme the packaged app claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolAssociation {
	/// Scheme without `://`, for example `myapp`.
	pub scheme: String,
	/// Short human description for installers and OS prompts.
	pub description: String,
}

impl ProtocolAssociation {
	/// Validates a lowercase alphanumeric / `.` / `+` / `-` scheme.
	pub fn new(scheme: impl Into<String>, description: impl Into<String>) -> Result<Self, Error> {
		let scheme = scheme.into();
		let description = description.into();
		if !valid_scheme(&scheme) || description.is_empty() || description.len() > 256 {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			scheme,
			description,
		})
	}
}

/// A file type the packaged app claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAssociation {
	/// Extensions without a leading dot, for example `md`.
	pub extensions: Vec<String>,
	/// Optional MIME types, for example `text/markdown`.
	pub mime_types: Vec<String>,
	/// Short human description for installers and OS prompts.
	pub description: String,
}

impl FileAssociation {
	/// Validates non-empty extension and description lists.
	pub fn new(
		extensions: Vec<String>,
		mime_types: Vec<String>,
		description: impl Into<String>,
	) -> Result<Self, Error> {
		let description = description.into();
		if extensions.is_empty()
			|| description.is_empty()
			|| description.len() > 256
			|| extensions.iter().any(|ext| {
				ext.is_empty()
					|| ext.len() > 32
					|| ext.starts_with('.')
					|| !ext
						.bytes()
						.all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'-')
			})
			|| mime_types.iter().any(|mime| {
				mime.is_empty() || mime.len() > 128 || mime.chars().any(char::is_control)
			}) {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			extensions,
			mime_types,
			description,
		})
	}
}

/// Failures from scheme validation and queue operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// Scheme, path, or association is invalid.
	#[error("invalid deep link argument")]
	InvalidArgument,
	/// URL scheme is not in the host allowlist.
	#[error("deep link scheme is not allowed")]
	ForbiddenScheme,
}

impl From<Error> for BridgeError {
	fn from(error: Error) -> Self {
		match error {
			Error::InvalidArgument | Error::ForbiddenScheme => Self::InvalidArgument,
		}
	}
}

/// Host-owned inbound activation queue with an explicit scheme allowlist.
#[derive(Debug)]
pub struct DeepLinks {
	schemes: BTreeSet<String>,
	events: VecDeque<Activation>,
	lost: u64,
	protocols: Vec<ProtocolAssociation>,
	files: Vec<FileAssociation>,
}

impl DeepLinks {
	/// Builds a queue that accepts only the listed URL schemes.
	pub fn new(schemes: impl IntoIterator<Item = String>) -> Result<Self, Error> {
		let mut allowed = BTreeSet::new();
		for scheme in schemes {
			if !valid_scheme(&scheme) {
				return Err(Error::InvalidArgument);
			}
			allowed.insert(scheme);
		}
		if allowed.is_empty() {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			schemes: allowed,
			events: VecDeque::new(),
			lost: 0,
			protocols: Vec::new(),
			files: Vec::new(),
		})
	}

	/// Declares packaging metadata for custom URL schemes (does not write OS
	/// config).
	pub fn with_protocols(mut self, protocols: Vec<ProtocolAssociation>) -> Result<Self, Error> {
		for protocol in &protocols {
			if !self.schemes.contains(&protocol.scheme) {
				return Err(Error::InvalidArgument);
			}
		}
		self.protocols = protocols;
		Ok(self)
	}

	/// Declares packaging metadata for file associations (does not write OS
	/// config).
	#[must_use = "file associations are stored on the returned value"]
	pub fn with_files(mut self, files: Vec<FileAssociation>) -> Self {
		self.files = files;
		self
	}

	/// Schemes this queue accepts.
	pub fn schemes(&self) -> impl Iterator<Item = &str> {
		self.schemes.iter().map(String::as_str)
	}

	/// Protocol association descriptors for packagers.
	#[must_use]
	pub fn protocols(&self) -> &[ProtocolAssociation] {
		&self.protocols
	}

	/// File association descriptors for packagers.
	#[must_use]
	pub fn files(&self) -> &[FileAssociation] {
		&self.files
	}

	/// Events dropped because the queue was full.
	#[must_use]
	pub fn lost(&self) -> u64 {
		self.lost
	}

	/// Validates and enqueues an inbound URL.
	pub fn push_url(&mut self, url: &str) -> Result<(), Error> {
		if url.is_empty() || url.len() > MAX_TARGET_BYTES || url.chars().any(char::is_control) {
			return Err(Error::InvalidArgument);
		}
		let parsed = url::Url::parse(url).map_err(|_| Error::InvalidArgument)?;
		if !self.schemes.contains(parsed.scheme()) {
			return Err(Error::ForbiddenScheme);
		}
		self.enqueue(Activation::Url(url.to_owned()));
		Ok(())
	}

	/// Validates and enqueues an absolute file path.
	pub fn push_file(&mut self, path: &Path) -> Result<(), Error> {
		let text = path.to_str().ok_or(Error::InvalidArgument)?;
		if !path.is_absolute()
			|| text.is_empty()
			|| text.len() > MAX_TARGET_BYTES
			|| text.chars().any(char::is_control)
		{
			return Err(Error::InvalidArgument);
		}
		self.enqueue(Activation::File(path.to_path_buf()));
		Ok(())
	}

	/// Drains every queued activation and clears the lost counter for this
	/// batch.
	pub fn drain(&mut self) -> (u64, Vec<Activation>) {
		let lost = self.lost;
		self.lost = 0;
		(lost, self.events.drain(..).collect())
	}

	fn enqueue(&mut self, activation: Activation) {
		if self.events.len() >= EVENT_CAPACITY {
			self.events.pop_front();
			self.lost = self.lost.saturating_add(1);
		}
		self.events.push_back(activation);
	}
}

impl Plugin for DeepLinks {
	fn name(&self) -> &'static str {
		"deep-link"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[EVENTS, SCHEMES]
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, BridgeError> {
		match method {
			EVENTS => {
				if !payload.is_empty() {
					return Err(BridgeError::InvalidArgument);
				}
				let (lost, events) = self.drain();
				let mut output = Encoder::with_capacity(16);
				output.u32((lost >> 32) as u32);
				output.u32(lost as u32);
				output.u32(u32::try_from(events.len()).map_err(|_| BridgeError::TooLarge)?);
				for event in events {
					let encoded = event.encode()?;
					output.bytes(&encoded)?;
				}
				Ok(output.finish())
			}
			SCHEMES => {
				if !payload.is_empty() {
					return Err(BridgeError::InvalidArgument);
				}
				let mut output = Encoder::with_capacity(32);
				output.u32(u32::try_from(self.schemes.len()).map_err(|_| BridgeError::TooLarge)?);
				for scheme in &self.schemes {
					output.bytes(scheme.as_bytes())?;
				}
				Ok(output.finish())
			}
			_ => Err(BridgeError::Unavailable),
		}
	}
}

/// Decodes an `EVENTS` reply for guests and tests.
pub fn decode_events(bytes: &[u8]) -> Result<(u64, Vec<Activation>), BridgeError> {
	let mut input = Decoder::new(bytes);
	let lost_hi = u64::from(input.u32()?);
	let lost_lo = u64::from(input.u32()?);
	let count = input.u32()? as usize;
	let mut events = Vec::with_capacity(count);
	for _ in 0..count {
		events.push(Activation::decode(input.bytes()?)?);
	}
	input.finish()?;
	Ok(((lost_hi << 32) | lost_lo, events))
}

fn valid_scheme(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 64
		&& value.as_bytes()[0].is_ascii_alphabetic()
		&& value.bytes().all(|byte| {
			byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"+.-".contains(&byte)
		})
}

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub mod guest {
	use super::*;

	/// Cached guest client for inbound deep links.
	#[derive(Debug, Clone, Copy)]
	pub struct DeepLinkClient(leptoskit_bridge::guest::Client);

	impl DeepLinkClient {
		/// Resolves the registered deep-link plugin once.
		pub fn new() -> Result<Self, BridgeError> {
			leptoskit_bridge::guest::Client::new("deep-link").map(Self)
		}

		/// Drains queued activations.
		pub async fn events(self) -> Result<(u64, Vec<Activation>), BridgeError> {
			decode_events(&self.0.call(EVENTS, &[])?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn accepts_allowed_urls_and_absolute_files() {
		let mut links = DeepLinks::new(["myapp".into(), "https".into()])
			.unwrap()
			.with_protocols(vec![
				ProtocolAssociation::new("myapp", "Open in My App").unwrap(),
			])
			.unwrap()
			.with_files(vec![
				FileAssociation::new(vec!["md".into()], vec!["text/markdown".into()], "Markdown")
					.unwrap(),
			]);
		links.push_url("myapp://item/42").unwrap();
		links.push_url("https://example.com/doc").unwrap();
		links.push_file(Path::new("/tmp/note.md")).unwrap();
		assert!(matches!(
			links.push_url("ftp://example.com"),
			Err(Error::ForbiddenScheme)
		));
		assert!(matches!(
			links.push_file(Path::new("relative.md")),
			Err(Error::InvalidArgument)
		));
		let (lost, events) = links.drain();
		assert_eq!(lost, 0);
		assert_eq!(events.len(), 3);
		assert_eq!(links.protocols().len(), 1);
		assert_eq!(links.files()[0].extensions, ["md"]);
	}

	#[test]
	fn bridge_round_trips_activations_and_schemes() {
		let mut links = DeepLinks::new(["leptoskit".into()]).unwrap();
		links.push_url("leptoskit://home").unwrap();
		let reply = links.invoke(EVENTS, Bytes::new()).unwrap();
		let (lost, events) = decode_events(&reply).unwrap();
		assert_eq!(lost, 0);
		assert_eq!(events, vec![Activation::Url("leptoskit://home".into())]);
		let schemes = links.invoke(SCHEMES, Bytes::new()).unwrap();
		let mut input = Decoder::new(&schemes);
		assert_eq!(input.u32().unwrap(), 1);
		assert_eq!(input.string().unwrap(), "leptoskit");
		input.finish().unwrap();
	}

	#[test]
	fn rejects_invalid_schemes_and_associations() {
		assert!(DeepLinks::new(["HTTP".into()]).is_err());
		assert!(ProtocolAssociation::new("1bad", "x").is_err());
		assert!(FileAssociation::new(vec![".md".into()], vec![], "Markdown").is_err());
	}
}
