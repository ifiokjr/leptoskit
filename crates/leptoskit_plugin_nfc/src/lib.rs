//! <!-- {=p_nfcSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_nfc` as a direct dependency and select the `nfc`
//! plugin in `native.plugins`; supported generated hosts discover it from Cargo
//! metadata. Generated Windows, Linux, and terminal hosts currently omit
//! runtime plugins; use explicit registration in a custom host there. Guests
//! enable the `guest` feature and reuse `guest::Nfc` rather than resolving the
//! plugin on every call. A successful read returns one bounded NDEF text or URI
//! record; it never writes tags or claims background tag delivery.
//!
//! <!-- {/p_nfcSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Native registration name.
pub const NAME: &str = "nfc";
/// Empty payload; four-byte [`Availability`].
pub const AVAILABILITY: MethodId = MethodId(0);
/// Empty payload; one bounded [`Record`] reply after a user-presented tag.
pub const READ: MethodId = MethodId(1);
/// Independently grantable methods.
pub const METHODS: &[MethodId] = &[AVAILABILITY, READ];

const MAX_RECORD_BYTES: usize = 4096;

/// Whether this implementation can start a foreground NDEF read now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Availability {
	/// This platform has no NFC backend.
	Unsupported = 0,
	/// Hardware is absent, powered off, or the OS refuses access.
	HardwareAbsent = 1,
	/// A foreground read can be started.
	Ready = 2,
	/// A prior read has not finished.
	Busy = 3,
}

impl Availability {
	/// Rejects unknown values and non-exact response lengths.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match u32::from_le_bytes(bytes.try_into().map_err(|_| Error::InvalidArgument)?) {
			0 => Ok(Self::Unsupported),
			1 => Ok(Self::HardwareAbsent),
			2 => Ok(Self::Ready),
			3 => Ok(Self::Busy),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Fixed four-byte response.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		(self as u32).to_le_bytes()
	}
}

/// One NDEF text or URI record returned by a successful read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
	kind: Kind,
	value: String,
}

/// Supported well-known record kinds for the first wave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Kind {
	/// UTF-8 text.
	Text = 0,
	/// Absolute HTTP(S) or other URI the platform decoded.
	Uri = 1,
}

impl Record {
	/// Builds a validated text record.
	pub fn text(value: impl Into<String>) -> Result<Self, Error> {
		Self::new(Kind::Text, value)
	}

	/// Builds a validated URI record.
	pub fn uri(value: impl Into<String>) -> Result<Self, Error> {
		Self::new(Kind::Uri, value)
	}

	fn new(kind: Kind, value: impl Into<String>) -> Result<Self, Error> {
		let value = value.into();
		if value.is_empty() || value.len() > MAX_RECORD_BYTES {
			return Err(Error::InvalidArgument);
		}
		if value.chars().any(|ch| ch.is_control() && ch != '\n' && ch != '\t') {
			return Err(Error::InvalidArgument);
		}
		if kind == Kind::Uri {
			let ok = value.starts_with("https://")
				|| value.starts_with("http://")
				|| value.starts_with("mailto:")
				|| value.starts_with("tel:");
			if !ok {
				return Err(Error::InvalidArgument);
			}
		}
		Ok(Self { kind, value })
	}

	/// Record kind.
	#[must_use]
	pub fn kind(&self) -> Kind {
		self.kind
	}

	/// Record payload.
	#[must_use]
	pub fn value(&self) -> &str {
		&self.value
	}

	/// Wire: kind word, then length-prefixed UTF-8.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut encoder = Encoder::with_capacity(8 + self.value.len());
		encoder.u32(self.kind as u32);
		encoder.bytes(self.value.as_bytes())?;
		Ok(encoder.finish())
	}

	/// Validates an untrusted native reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_RECORD_BYTES + 8 {
			return Err(Error::TooLarge);
		}
		let mut decoder = Decoder::new(payload);
		let kind = match decoder.u32()? {
			0 => Kind::Text,
			1 => Kind::Uri,
			_ => return Err(Error::InvalidArgument),
		};
		let value = decoder.string()?;
		decoder.finish()?;
		Self::new(kind, value)
	}
}

/// Validates a method and payload without contacting NFC hardware.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	if !METHODS.contains(&method) {
		return Err(Error::Unavailable);
	}
	if payload.is_empty() {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}

/// Explicit unavailable handler for hosts without NFC hardware APIs.
#[derive(Debug, Default)]
pub struct Unsupported;

impl leptoskit_bridge::Plugin for Unsupported {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(
		&mut self,
		method: MethodId,
		payload: Bytes,
	) -> Result<Bytes, Error> {
		validate(method, &payload)?;
		match method {
			AVAILABILITY => Ok(Bytes::copy_from_slice(&Availability::Unsupported.encode())),
			READ => Err(Error::Unavailable),
			_ => Err(Error::Unavailable),
		}
	}
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Cached numeric native client.
	#[derive(Debug, Clone, Copy)]
	pub struct Nfc(leptoskit_bridge::guest::Client);

	impl Nfc {
		/// Resolves the installed host plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries hardware readiness without opening a session.
		pub async fn availability(self) -> Result<Availability, Error> {
			Availability::decode(&self.0.call(AVAILABILITY, &[])?.await?)
		}

		/// Starts one foreground NDEF read. Dropping the future cancels the
		/// caller; the OS sheet may still need the user to dismiss it.
		pub async fn read(self) -> Result<Record, Error> {
			Record::decode(&self.0.call(READ, &[])?.await?)
		}
	}
}

/// Browsers have no NFC reader in this port.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always unsupported.
	#[must_use]
	pub const fn availability() -> Availability {
		Availability::Unsupported
	}

	/// Always unavailable.
	pub fn read() -> Result<Record, Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin as _;

	use super::*;

	#[test]
	fn availability_and_record_codecs() {
		for status in [
			Availability::Unsupported,
			Availability::HardwareAbsent,
			Availability::Ready,
			Availability::Busy,
		] {
			assert_eq!(Availability::decode(&status.encode()).unwrap(), status);
		}
		assert!(Availability::decode(&[9, 0, 0, 0]).is_err());

		let text = Record::text("hello").unwrap();
		assert_eq!(Record::decode(&text.encode().unwrap()).unwrap(), text);
		let uri = Record::uri("https://example.test/a").unwrap();
		assert_eq!(Record::decode(&uri.encode().unwrap()).unwrap(), uri);
		assert!(Record::uri("ftp://example.test").is_err());
		assert!(Record::text("").is_err());
	}

	#[test]
	fn unsupported_reports_status_and_refuses_reads() {
		let mut plugin = Unsupported;
		assert_eq!(
			Availability::decode(&plugin.invoke(AVAILABILITY, Bytes::new()).unwrap()).unwrap(),
			Availability::Unsupported
		);
		assert_eq!(
			plugin.invoke(READ, Bytes::new()),
			Err(Error::Unavailable)
		);
		assert_eq!(validate(READ, &[0]), Err(Error::InvalidArgument));
	}
}
