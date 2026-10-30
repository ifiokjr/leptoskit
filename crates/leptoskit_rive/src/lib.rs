//! Reuse app-owned Rive assets without implicit network access.
//!
//! [`Asset`] performs bounded header checks, not full file validation or
//! authentication. Only load files you trust: Rive's upstream parser decodes
//! their contents, including embedded images, fonts, and scripts.
//!
//! Browser playback uses `web` on `wasm32-unknown-unknown`. The `native`
//! feature adds thread-confined CPU vector playback on native platforms.
//! Native rendering rejects images, text, layout, scripts, and audio; see
//! the `native` module for the explicit frame lifecycle and trust boundary.

use std::fmt;
use std::sync::Arc;

mod playback;
pub use playback::Playback;
pub use playback::Update;
mod pixel;
pub use pixel::Frame;
pub use pixel::MAX_FRAME_PIXELS;

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
pub mod native;

/// The maximum encoded asset size, checked before copying into an allocation.
pub const MAX_ASSET_BYTES: usize = 8 * 1024 * 1024;

/// A failure at the asset or runtime boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
	/// The encoded file is too large.
	#[error("Rive asset exceeds the {MAX_ASSET_BYTES} byte limit")]
	AssetTooLarge,
	/// The signature or version fields are missing or malformed.
	#[error("invalid or truncated Rive header")]
	InvalidHeader,
	/// This integration supports Rive file format version 7.
	#[error("unsupported Rive major version {0}; expected 7")]
	UnsupportedVersion(u32),
	/// Names must be nonempty and bounded, with no control characters.
	#[error("Rive name must contain 1 to 256 UTF-8 bytes and no control characters")]
	InvalidName,
	/// Numeric state-machine inputs must be representable by Rive's float type.
	#[error("Rive number must be finite and representable as f32")]
	InvalidNumber,
	/// A native frame must fit the per-axis and total-pixel limits.
	#[error("Rive frame must have nonzero dimensions, at most 4096 per axis and 4194304 pixels")]
	InvalidFrame,
	/// The selected native renderer cannot faithfully display a file feature.
	#[error("Rive asset uses a feature unsupported by the native renderer")]
	UnsupportedFeature,
	/// The browser or Rive runtime rejected an operation.
	#[error("Rive runtime: {0}")]
	Runtime(String),
}

/// A cheap-to-clone, immutable, app-owned `.riv` file.
///
/// Header checks only reject obviously invalid input. They do not make an
/// untrusted upload safe, bound decoded memory, or authenticate scripts.
/// There is no URL loader, global cache, or filesystem access.
#[derive(Clone, PartialEq, Eq)]
pub struct Asset(Arc<[u8]>);

impl fmt::Debug for Asset {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		// Never dump megabytes of encoded content into an application's logs.
		formatter
			.debug_struct("Asset")
			.field("byte_len", &self.0.len())
			.finish()
	}
}

impl Asset {
	/// Copies a trusted asset after checking its size and header.
	pub fn new(bytes: &[u8]) -> Result<Self, Error> {
		validate(bytes)?;
		Ok(Self(Arc::from(bytes)))
	}

	/// Reuses an existing shared allocation after checking its size and header.
	pub fn from_shared(bytes: Arc<[u8]>) -> Result<Self, Error> {
		validate(&bytes)?;
		Ok(Self(bytes))
	}

	/// The encoded bytes, borrowed without copying.
	pub fn bytes(&self) -> &[u8] {
		&self.0
	}
}

/// A bounded artboard, animation, state-machine, or input name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(try_from = "String", into = "String")
)]
pub struct Name(String);

impl TryFrom<String> for Name {
	type Error = Error;

	fn try_from(value: String) -> Result<Self, Error> {
		Self::new(value)
	}
}

impl From<Name> for String {
	fn from(name: Name) -> String {
		name.0
	}
}

impl Name {
	/// Validates a runtime name before crossing the language boundary.
	pub fn new(name: impl Into<String>) -> Result<Self, Error> {
		let name = name.into();

		if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
			return Err(Error::InvalidName);
		}

		Ok(Self(name))
	}

	/// The original UTF-8 name.
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

/// Exactly one scene drives a player; animations and machines cannot compete.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Scene {
	/// Use the runtime's default animation.
	#[default]
	Default,
	/// Play one named timeline.
	Animation(Name),
	/// Run one named state machine.
	StateMachine(Name),
}

/// A typed state-machine update. Triggers cannot be confused with booleans.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
// The largest variant is only 16 bytes; boxing it would allocate every update.
#[allow(variant_size_differences)]
pub enum Input {
	/// Set a boolean input.
	Boolean(bool),
	/// Set a finite numeric input; checked again at the runtime boundary.
	Number(f64),
	/// Fire a trigger once.
	Trigger,
}

impl Input {
	/// Rejects non-finite values and overflow in the upstream f32 conversion.
	pub fn validate(self) -> Result<(), Error> {
		if let Self::Number(number) = self {
			if !number.is_finite() || number.abs() > f64::from(f32::MAX) {
				return Err(Error::InvalidNumber);
			}
		}

		Ok(())
	}
}

/// Checks the envelope before any allocation or call into the upstream parser.
fn validate(bytes: &[u8]) -> Result<(), Error> {
	if bytes.len() > MAX_ASSET_BYTES {
		return Err(Error::AssetTooLarge);
	}

	let mut fields = bytes.strip_prefix(b"RIVE").ok_or(Error::InvalidHeader)?;
	let major = var_u32(&mut fields)?;
	let _minor = var_u32(&mut fields)?;
	let _file_id = var_u32(&mut fields)?;

	if major != 7 {
		return Err(Error::UnsupportedVersion(major));
	}

	// A header without even a ToC terminator is necessarily truncated.
	if fields.is_empty() {
		return Err(Error::InvalidHeader);
	}

	Ok(())
}

/// Reads a bounded uint32 varint, rejecting overflow and overlong encodings.
fn var_u32(bytes: &mut &[u8]) -> Result<u32, Error> {
	let mut value = 0;

	for shift in (0..35).step_by(7) {
		let (&byte, rest) = bytes.split_first().ok_or(Error::InvalidHeader)?;
		*bytes = rest;

		if shift == 28 && byte > 15 {
			return Err(Error::InvalidHeader);
		}

		value |= u32::from(byte & 0x7f) << shift;

		if byte & 0x80 == 0 {
			if shift > 0 && byte == 0 {
				return Err(Error::InvalidHeader);
			}

			return Ok(value);
		}
	}

	Err(Error::InvalidHeader)
}

#[cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]
pub mod web;
