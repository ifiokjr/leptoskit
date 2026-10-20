//! Installed app identity. Native hosts read the actual running bundle/package;
//! Rust hosts supply their own packaged identity explicitly. No device IDs,
//! install referrers, or distribution-channel guesses are collected.
//!
//! Binary snapshot: version u32 (1), then four length-prefixed UTF-8 fields
//! (application ID, name, version, build). Empty fields mean unknown. Each
//! field is bounded to 4096 bytes; all integers use little-endian order.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime plugin name.
pub const NAME: &str = "application";
/// Empty request -> versioned [`Snapshot`].
pub const SNAPSHOT: MethodId = MethodId(0);

/// Metadata belonging to the installed host application, not its guest bundle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
	/// Bundle identifier on Apple or application package name on Android.
	pub id: Option<String>,
	/// Localized display name when available.
	pub name: Option<String>,
	/// User-facing release version.
	pub version: Option<String>,
	/// Build number; Android's version code is represented as a decimal string.
	pub build: Option<String>,
}

impl Snapshot {
	/// Encodes validated metadata. Empty known values are invalid.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(128);
		output.u32(1);
		for field in [&self.id, &self.name, &self.version, &self.build] {
			if field.as_ref().is_some_and(String::is_empty) {
				return Err(Error::InvalidArgument);
			}
			let text = field.as_deref().unwrap_or_default();
			if text.len() > 4096 {
				return Err(Error::TooLarge);
			}
			output.bytes(text.as_bytes())?;
		}
		Ok(output.finish())
	}

	/// Validates version, bounded text, UTF-8 and complete message consumption.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}
		let mut text = || -> Result<Option<String>, Error> {
			let value = input.string()?;
			if value.len() > 4096 {
				return Err(Error::TooLarge);
			}
			Ok((!value.is_empty()).then(|| value.to_owned()))
		};
		let result = Self {
			id: text()?,
			name: text()?,
			version: text()?,
			build: text()?,
		};
		input.finish()?;
		Ok(result)
	}
}

/// Rust host adapter whose identity is supplied by the application's packaging
/// configuration. It never reads this library crate's Cargo package variables.
#[derive(Debug, Clone)]
pub struct Application {
	encoded: Bytes,
}
impl Application {
	/// Validates and caches the host's immutable installed metadata once.
	pub fn new(identity: &Snapshot) -> Result<Self, Error> {
		Ok(Self {
			encoded: identity.encode()?,
		})
	}
}
impl Plugin for Application {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		&[SNAPSHOT]
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		if method != SNAPSHOT {
			return Err(Error::Unavailable);
		}
		if !payload.is_empty() {
			return Err(Error::InvalidArgument);
		}
		Ok(self.encoded.clone())
	}
}

/// Cached native guest client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// A resolved application plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Application(leptoskit_bridge::guest::Client);
	impl Application {
		/// Resolves the native application plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads the installed native host's identity.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn identity_round_trip_and_host_validation() {
		let identity = Snapshot {
			id: Some("dev.example.app".into()),
			name: Some("你好".into()),
			version: Some("1.2.3".into()),
			build: Some("4294967296".into()),
		};
		let mut host = Application::new(&identity).unwrap();
		assert_eq!(
			Snapshot::decode(&host.invoke(SNAPSHOT, Bytes::new()).unwrap()),
			Ok(identity)
		);
		assert_eq!(
			host.invoke(MethodId(1), Bytes::new()),
			Err(Error::Unavailable)
		);
		assert_eq!(
			host.invoke(SNAPSHOT, Bytes::from_static(&[1])),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn decoder_rejects_truncated_utf8_and_trailing_messages() {
		let encoded = Snapshot {
			id: Some("x".into()),
			..Snapshot::default()
		}
		.encode()
		.unwrap();
		for length in 0..encoded.len() {
			assert_eq!(
				Snapshot::decode(&encoded[..length]),
				Err(Error::InvalidArgument)
			);
		}
		let mut invalid = encoded.to_vec();
		invalid[8] = 255;
		assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
		let mut invalid = encoded.to_vec();
		invalid[0] = 2;
		assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
		let mut invalid = encoded.to_vec();
		invalid.push(0);
		assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
	}
	#[test]
	fn unknown_identity_and_bounded_text() {
		let unknown = Snapshot::default();
		assert_eq!(Snapshot::decode(&unknown.encode().unwrap()), Ok(unknown));
		assert_eq!(
			Snapshot {
				name: Some(String::new()),
				..Snapshot::default()
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Snapshot {
				name: Some("x".repeat(4097)),
				..Snapshot::default()
			}
			.encode(),
			Err(Error::TooLarge)
		);
		let mut wire = Snapshot::default().encode().unwrap().to_vec();
		wire.splice(4..8, 4097_u32.to_le_bytes());
		wire.splice(8..8, vec![b'x'; 4097]);
		assert_eq!(Snapshot::decode(&wire), Err(Error::TooLarge));
	}
}
