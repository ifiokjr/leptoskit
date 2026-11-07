//! <!-- {=p_home_widgetsSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_home_widgets` as a direct dependency and select the
//! `home-widgets` plugin in `native.plugins`; supported generated hosts
//! discover it from Cargo metadata. Generated Windows, Linux, and terminal
//! hosts currently omit runtime plugins; use explicit registration in a custom
//! host there. Guests enable the `guest` feature and reuse
//! `guest::HomeWidgets` rather than resolving the plugin on every call. The
//! plugin publishes bounded snapshot text for `WidgetKit` and Android App
//! Widgets and asks the OS to reload them; it does not draw the widget UI or
//! scaffold the extension target.
//!
//! <!-- {/p_home_widgetsSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Stable native registration name.
pub const NAME: &str = "home-widgets";
/// Empty payload; four-byte [`Capability`] flags.
pub const CAPABILITY: MethodId = MethodId(0);
/// One [`Snapshot`]; empty reply. Replaces any prior snapshot for that kind.
pub const SET: MethodId = MethodId(1);
/// One length-prefixed kind; replies with the stored [`Snapshot`].
pub const GET: MethodId = MethodId(2);
/// One length-prefixed kind; removes that kind's snapshot.
pub const CLEAR: MethodId = MethodId(3);
/// Optional kind string (empty payload means every kind this plugin owns).
pub const RELOAD: MethodId = MethodId(4);
/// One length-prefixed kind; four-byte [`Installed`] reply.
pub const INSTALLED: MethodId = MethodId(5);
/// Independently grantable operations.
pub const METHODS: &[MethodId] = &[CAPABILITY, SET, GET, CLEAR, RELOAD, INSTALLED];

/// Maximum UTF-8 bytes for a snapshot title.
pub const MAX_TITLE_BYTES: usize = 256;
/// Maximum UTF-8 bytes for a snapshot body.
pub const MAX_BODY_BYTES: usize = 1024;
/// Maximum UTF-8 bytes for an optional deep-link URL.
pub const MAX_URL_BYTES: usize = 2048;
/// Largest encoded snapshot the wire format accepts.
pub const MAX_SNAPSHOT_BYTES: usize = 16 + 64 + MAX_TITLE_BYTES + MAX_BODY_BYTES + MAX_URL_BYTES;

/// What this host can do for home-screen and desktop widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Capability {
	/// The platform has a widget backend wired for this plugin.
	pub supported: bool,
	/// [`RELOAD`] can ask the OS to refresh timelines or app widgets.
	pub can_reload: bool,
	/// [`INSTALLED`] can distinguish none from some pinned instances.
	pub can_query_installed: bool,
	/// Shared storage the widget extension can read is available.
	pub shared_store: bool,
}

impl Capability {
	const CAN_QUERY_INSTALLED: u32 = 1 << 2;
	const CAN_RELOAD: u32 = 1 << 1;
	const SHARED_STORE: u32 = 1 << 3;
	const SUPPORTED: u32 = 1;

	/// Encodes the four capability flags as little-endian bits.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		let mut flags = 0u32;
		if self.supported {
			flags |= Self::SUPPORTED;
		}
		if self.can_reload {
			flags |= Self::CAN_RELOAD;
		}
		if self.can_query_installed {
			flags |= Self::CAN_QUERY_INSTALLED;
		}
		if self.shared_store {
			flags |= Self::SHARED_STORE;
		}
		flags.to_le_bytes()
	}

	/// Validates an exact four-byte capability reply.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let flags = u32::from_le_bytes(bytes.try_into().map_err(|_| Error::InvalidArgument)?);
		let known =
			Self::SUPPORTED | Self::CAN_RELOAD | Self::CAN_QUERY_INSTALLED | Self::SHARED_STORE;
		if flags & !known != 0 {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			supported: flags & Self::SUPPORTED != 0,
			can_reload: flags & Self::CAN_RELOAD != 0,
			can_query_installed: flags & Self::CAN_QUERY_INSTALLED != 0,
			shared_store: flags & Self::SHARED_STORE != 0,
		})
	}
}

/// Whether any instance of a kind is currently pinned on the home screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Installed {
	/// The platform cannot report pin state (`WidgetKit` has no public query).
	Unknown = 0,
	/// No pinned instances of this kind were found.
	None = 1,
	/// At least one pinned instance of this kind was found.
	Some = 2,
}

impl Installed {
	/// Validates the exact four-byte reply.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let value = u32::from_le_bytes(bytes.try_into().map_err(|_| Error::InvalidArgument)?);
		match value {
			0 => Ok(Self::Unknown),
			1 => Ok(Self::None),
			2 => Ok(Self::Some),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Encodes the installed state as four little-endian bytes.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		(self as u32).to_le_bytes()
	}
}

/// Widget family / size hint shared across `WidgetKit` and Android App Widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Family {
	/// Platform default size; the extension chooses the concrete family.
	System = 0,
	/// Small / compact.
	Small = 1,
	/// Medium.
	Medium = 2,
	/// Large.
	Large = 3,
	/// Extra-large (iPad / macOS / Android XL).
	ExtraLarge = 4,
}

impl Family {
	fn from_u32(value: u32) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::System),
			1 => Ok(Self::Small),
			2 => Ok(Self::Medium),
			3 => Ok(Self::Large),
			4 => Ok(Self::ExtraLarge),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Stable application-chosen widget kind identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Kind(String);

impl Kind {
	/// Accepts 1..=64 ASCII letters, digits, hyphens, periods or underscores.
	pub fn new(value: impl Into<String>) -> Result<Self, Error> {
		let value = value.into();
		validate_id(&value)?;
		Ok(Self(value))
	}

	/// Borrows the identifier.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}

	/// Encodes a kind-only payload for get, clear, reload, or installed.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(4 + self.0.len());
		output.bytes(self.0.as_bytes())?;
		Ok(output.finish())
	}

	/// Decodes a kind-only payload, rejecting trailing bytes.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > 68 {
			return Err(Error::InvalidArgument);
		}
		let mut input = Decoder::new(payload);
		let kind = Self::new(input.string()?)?;
		input.finish()?;
		Ok(kind)
	}
}

fn validate_id(value: &str) -> Result<(), Error> {
	if value.is_empty()
		|| value.len() > 64
		|| !value
			.bytes()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
	{
		Err(Error::InvalidArgument)
	} else {
		Ok(())
	}
}

fn validate_text(
	value: &str,
	maximum: usize,
	multiline: bool,
	required: bool,
) -> Result<(), Error> {
	if value.len() > maximum
		|| (required && value.trim().is_empty())
		|| value
			.chars()
			.any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\t')))
	{
		Err(Error::InvalidArgument)
	} else {
		Ok(())
	}
}

fn validate_url(value: &str) -> Result<(), Error> {
	if value.is_empty() {
		return Ok(());
	}
	let has_scheme =
		value.starts_with("https://") || value.starts_with("http://") || value.contains(':');
	if value.len() > MAX_URL_BYTES
		|| value.contains('\0')
		|| value.chars().any(char::is_control)
		|| !has_scheme
	{
		return Err(Error::InvalidArgument);
	}
	Ok(())
}

/// Validated plain-text snapshot the widget extension reads from shared
/// storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
	kind: Kind,
	family: Family,
	title: String,
	body: String,
	url: String,
}

impl Snapshot {
	/// Constructs a bounded snapshot. `url` may be empty; otherwise it must be
	/// a scheme-bearing deep link or absolute HTTP(S) URL without controls.
	pub fn new(
		kind: Kind,
		family: Family,
		title: impl Into<String>,
		body: impl Into<String>,
		url: impl Into<String>,
	) -> Result<Self, Error> {
		let title = title.into();
		let body = body.into();
		let url = url.into();
		validate_text(&title, MAX_TITLE_BYTES, false, true)?;
		validate_text(&body, MAX_BODY_BYTES, true, false)?;
		validate_url(&url)?;
		Ok(Self {
			kind,
			family,
			title,
			body,
			url,
		})
	}

	/// Application-chosen kind this snapshot belongs to.
	#[must_use]
	pub fn kind(&self) -> &Kind {
		&self.kind
	}

	/// Suggested widget family / size.
	#[must_use]
	pub fn family(&self) -> Family {
		self.family
	}

	/// Plain-text heading.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}

	/// Plain-text body; may be empty.
	#[must_use]
	pub fn body(&self) -> &str {
		&self.body
	}

	/// Optional deep link; empty when unset.
	#[must_use]
	pub fn url(&self) -> &str {
		&self.url
	}

	/// Encodes the snapshot without a JSON intermediary.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(
			20 + self.kind.0.len() + self.title.len() + self.body.len() + self.url.len(),
		);
		output.bytes(self.kind.as_str().as_bytes())?;
		output.u32(self.family as u32);
		for value in [&self.title, &self.body, &self.url] {
			output.bytes(value.as_bytes())?;
		}
		Ok(output.finish())
	}

	/// Validates a complete native control message, including trailing bytes.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_SNAPSHOT_BYTES {
			return Err(Error::InvalidArgument);
		}
		let mut input = Decoder::new(payload);
		let kind = Kind::new(input.string()?)?;
		let family = Family::from_u32(input.u32()?)?;
		let title = input.string()?;
		let body = input.string()?;
		let url = input.string()?;
		input.finish()?;
		Self::new(kind, family, title, body, url)
	}
}

/// Reload request: empty payload means every kind; otherwise one kind string.
pub fn decode_reload(payload: &[u8]) -> Result<Option<Kind>, Error> {
	if payload.is_empty() {
		Ok(None)
	} else {
		Kind::decode(payload).map(Some)
	}
}

/// Encodes an optional kind for [`RELOAD`].
pub fn encode_reload(kind: Option<&Kind>) -> Result<Bytes, Error> {
	match kind {
		None => Ok(Bytes::new()),
		Some(kind) => kind.encode(),
	}
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn snapshot_roundtrips_and_rejects_ambiguous_input() {
		let snapshot = Snapshot::new(
			Kind::new("status").unwrap(),
			Family::Medium,
			"Inbox ✓",
			"Three\nlines",
			"myapp://inbox",
		)
		.unwrap();
		let bytes = snapshot.encode().unwrap();
		assert_eq!(Snapshot::decode(&bytes).unwrap(), snapshot);

		for length in 0..bytes.len() {
			assert!(Snapshot::decode(&bytes[..length]).is_err());
		}
		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert!(Snapshot::decode(&trailing).is_err());

		for id in ["", "../a", "bad\0id", "é", &"a".repeat(65)] {
			assert!(Kind::new(id).is_err());
		}
		assert!(Snapshot::new(Kind::new("status").unwrap(), Family::Small, "\t", "", "").is_err());
		assert!(
			Snapshot::new(
				Kind::new("status").unwrap(),
				Family::Small,
				"Title",
				"bad\0body",
				""
			)
			.is_err()
		);
		assert!(
			Snapshot::new(
				Kind::new("status").unwrap(),
				Family::Small,
				"Title",
				"",
				"no-scheme"
			)
			.is_err()
		);
	}

	#[test]
	fn capability_and_installed_are_strict() {
		let capability = Capability {
			supported: true,
			can_reload: true,
			can_query_installed: false,
			shared_store: true,
		};
		assert_eq!(
			Capability::decode(&capability.encode()).unwrap(),
			capability
		);
		assert!(Capability::decode(&[0xff, 0, 0, 0]).is_err());
		assert!(Capability::decode(&[1, 0, 0]).is_err());

		assert_eq!(
			Installed::decode(&Installed::Some.encode()).unwrap(),
			Installed::Some
		);
		for value in [&[][..], &[3], &[3, 0, 0, 0], &[2, 0, 0, 0, 0]] {
			assert!(Installed::decode(value).is_err());
		}
	}

	#[test]
	fn kind_and_reload_payloads_roundtrip() {
		let kind = Kind::new("counter.v1").unwrap();
		assert_eq!(Kind::decode(&kind.encode().unwrap()).unwrap(), kind);
		assert_eq!(decode_reload(&[]).unwrap(), None);
		assert_eq!(
			decode_reload(&encode_reload(Some(&kind)).unwrap()).unwrap(),
			Some(kind.clone())
		);
		assert_eq!(encode_reload(None).unwrap(), Bytes::new());
		assert_eq!(kind.as_str(), "counter.v1");
		assert_eq!(
			Snapshot::new(kind, Family::System, "Count", "12", "")
				.unwrap()
				.family(),
			Family::System
		);
	}

	#[test]
	fn every_family_encodes_exactly() {
		for family in [
			Family::System,
			Family::Small,
			Family::Medium,
			Family::Large,
			Family::ExtraLarge,
		] {
			let snapshot = Snapshot::new(Kind::new("k").unwrap(), family, "T", "", "").unwrap();
			assert_eq!(
				Snapshot::decode(&snapshot.encode().unwrap())
					.unwrap()
					.family(),
				family
			);
		}
		let mut bad = Snapshot::new(Kind::new("k").unwrap(), Family::Small, "T", "", "")
			.unwrap()
			.encode()
			.unwrap()
			.to_vec();
		let family_offset = 4 + 1;
		bad[family_offset] = 9;
		assert_eq!(Snapshot::decode(&bad).unwrap_err(), Error::InvalidArgument);
	}
}
