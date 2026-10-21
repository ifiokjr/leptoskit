//! Device information without persistent identifiers or user-assigned names.
//!
//! Wire version 1 consists of five little-endian u32 fields: version, device
//! kind, simulator flag (unknown/no/yes = 0/1/2), memory low word, memory high
//! word, followed by six length-prefixed UTF-8 strings: manufacturer, brand,
//! model, OS name, OS version, architecture. Memory zero and empty strings
//! represent unknown values. Each string is limited to 4096 bytes.

use std::num::NonZeroU64;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime plugin name.
pub const NAME: &str = "device";
/// Empty input -> versioned [`Snapshot`] binary response.
pub const SNAPSHOT: MethodId = MethodId(0);
const MAX_TEXT_BYTES: usize = 4096;

/// The form factor explicitly reported by a platform.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Kind {
	/// Unavailable or not reliably distinguishable.
	#[default]
	Unknown = 0,
	/// Handheld phone.
	Phone = 1,
	/// Tablet.
	Tablet = 2,
	/// Television.
	Television = 3,
	/// Desktop or laptop computer.
	Desktop = 4,
	/// Watch.
	Watch = 5,
	/// Car interface.
	Car = 6,
	/// Spatial headset.
	Headset = 7,
}

/// A system snapshot. Missing platform information stays `None`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Snapshot {
	/// Hardware manufacturer, if exposed by the OS.
	pub manufacturer: Option<String>,
	/// Consumer-facing hardware brand.
	pub brand: Option<String>,
	/// Platform model label. On iOS this is the product family, e.g. `iPhone`.
	pub model: Option<String>,
	/// Operating system name, not the browser name.
	pub os_name: Option<String>,
	/// Operating system version reported by the platform.
	pub os_version: Option<String>,
	/// Native plugin architecture or preferred Android ABI. Vocabulary is
	/// platform-specific.
	pub architecture: Option<String>,
	/// Total memory exposed to the OS, in bytes; not the app's memory
	/// allowance.
	pub total_memory: Option<NonZeroU64>,
	/// Platform-reported form factor.
	pub kind: Kind,
	/// Known simulator status; `None` when no reliable signal is available.
	/// Android emulator signatures can establish `Some(true)`, never
	/// `Some(false)`.
	pub simulator: Option<bool>,
}

impl Snapshot {
	/// Encodes a snapshot, rejecting empty known strings and oversized values.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(128);
		output.u32(1);
		output.u32(self.kind as u32);
		output.u32(match self.simulator {
			None => 0,
			Some(false) => 1,
			Some(true) => 2,
		});
		let memory = self.total_memory.map_or(0, NonZeroU64::get).to_le_bytes();
		output.u32(u32::from_le_bytes(
			memory[..4].try_into().map_err(|_| Error::InvalidArgument)?,
		));
		output.u32(u32::from_le_bytes(
			memory[4..].try_into().map_err(|_| Error::InvalidArgument)?,
		));
		for value in [
			&self.manufacturer,
			&self.brand,
			&self.model,
			&self.os_name,
			&self.os_version,
			&self.architecture,
		] {
			if let Some(value) = value {
				if value.is_empty() {
					return Err(Error::InvalidArgument);
				}
				if value.len() > MAX_TEXT_BYTES {
					return Err(Error::TooLarge);
				}
			}
			output.bytes(value.as_deref().unwrap_or_default().as_bytes())?;
		}
		Ok(output.finish())
	}

	/// Validates the version, flags, UTF-8, lengths, and complete consumption.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}
		let kind = match input.u32()? {
			0 => Kind::Unknown,
			1 => Kind::Phone,
			2 => Kind::Tablet,
			3 => Kind::Television,
			4 => Kind::Desktop,
			5 => Kind::Watch,
			6 => Kind::Car,
			7 => Kind::Headset,
			_ => return Err(Error::InvalidArgument),
		};
		let simulator = match input.u32()? {
			0 => None,
			1 => Some(false),
			2 => Some(true),
			_ => return Err(Error::InvalidArgument),
		};
		let low = u64::from(input.u32()?);
		let high = u64::from(input.u32()?);
		let total_memory = NonZeroU64::new(low | (high << 32));
		let mut text = || -> Result<Option<String>, Error> {
			let value = input.string()?;
			if value.len() > MAX_TEXT_BYTES {
				return Err(Error::TooLarge);
			}
			Ok((!value.is_empty()).then(|| value.to_owned()))
		};
		let result = Self {
			manufacturer: text()?,
			brand: text()?,
			model: text()?,
			os_name: text()?,
			os_version: text()?,
			architecture: text()?,
			total_memory,
			kind,
			simulator,
		};
		input.finish()?;
		Ok(result)
	}
}

/// Desktop information through OS APIs. Only memory is refreshed; no process
/// enumeration or CPU sampling runs. Hardware vendor/model stay unknown.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub mod desktop {
	use super::*;

	/// Reads current system memory and OS metadata without persistent IDs.
	pub fn snapshot() -> Snapshot {
		let mut system = sysinfo::System::new();
		system.refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());
		Snapshot {
			os_name: sysinfo::System::name().filter(|value| !value.is_empty()),
			os_version: sysinfo::System::os_version().filter(|value| !value.is_empty()),
			architecture: Some(sysinfo::System::cpu_arch()),
			total_memory: NonZeroU64::new(system.total_memory()),
			kind: Kind::Desktop,
			..Snapshot::default()
		}
	}

	/// Rust desktop bridge adapter.
	#[derive(Debug, Default)]
	pub struct Device;

	impl leptoskit_bridge::Plugin for Device {
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
			snapshot().encode()
		}
	}
}

/// Browser information limited to low-entropy User-Agent Client Hints.
/// Unsupported or withheld fields stay unknown. Reduced/coarse `deviceMemory`
/// is deliberately not presented as exact total RAM; user-agent strings are not
/// parsed.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use wasm_bindgen::JsValue;

	use super::*;

	/// Reads the browser's reported OS platform when available.
	pub fn snapshot() -> Result<Snapshot, Error> {
		let navigator = web_sys::window().ok_or(Error::Unavailable)?.navigator();
		let hints = js_sys::Reflect::get(navigator.as_ref(), &JsValue::from_str("userAgentData"))
			.map_err(|_| Error::Native)?;
		let os_name = if hints.is_object() {
			js_sys::Reflect::get(&hints, &JsValue::from_str("platform"))
				.map_err(|_| Error::Native)?
				.as_string()
				.filter(|value| !value.is_empty())
		} else {
			None
		};
		Ok(Snapshot {
			os_name,
			..Snapshot::default()
		})
	}
}

/// Cached native guest bridge client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// A resolved device plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Device(leptoskit_bridge::guest::Client);
	impl Device {
		/// Resolves the installed native plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads and validates the native snapshot.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn round_trip_preserves_unknowns_unicode_and_large_memory() {
		let unknown = Snapshot::default();
		assert_eq!(Snapshot::decode(&unknown.encode().unwrap()), Ok(unknown));
		let known = Snapshot {
			manufacturer: Some("製造元".into()),
			total_memory: NonZeroU64::new(16 << 30),
			simulator: Some(false),
			kind: Kind::Tablet,
			..Snapshot::default()
		};
		assert_eq!(Snapshot::decode(&known.encode().unwrap()), Ok(known));
	}

	#[test]
	fn every_kind_and_simulator_state_round_trips() {
		let kinds = [
			Kind::Unknown,
			Kind::Phone,
			Kind::Tablet,
			Kind::Television,
			Kind::Desktop,
			Kind::Watch,
			Kind::Car,
			Kind::Headset,
		];
		for kind in kinds {
			for simulator in [None, Some(false), Some(true)] {
				let snapshot = Snapshot {
					kind,
					simulator,
					..Snapshot::default()
				};
				assert_eq!(Snapshot::decode(&snapshot.encode().unwrap()), Ok(snapshot));
			}
		}
	}

	#[test]
	fn rejects_truncation_invalid_flags_utf8_and_trailing_data() {
		let encoded = Snapshot {
			model: Some("x".into()),
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
		for (offset, value) in [(0, 2), (4, 8), (8, 3), (32, 255)] {
			let mut invalid = encoded.to_vec();
			invalid[offset] = value;
			assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
		}
		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert_eq!(Snapshot::decode(&trailing), Err(Error::InvalidArgument));
	}

	#[test]
	fn rejects_oversized_or_empty_known_text() {
		let empty = Snapshot {
			model: Some(String::new()),
			..Snapshot::default()
		};
		assert_eq!(empty.encode(), Err(Error::InvalidArgument));
		let oversized = Snapshot {
			model: Some("x".repeat(MAX_TEXT_BYTES + 1)),
			..Snapshot::default()
		};
		assert_eq!(oversized.encode(), Err(Error::TooLarge));
		let mut wire = Snapshot::default().encode().unwrap().to_vec();
		wire.splice(28..32, 4097_u32.to_le_bytes());
		wire.splice(32..32, vec![b'x'; 4097]);
		assert_eq!(Snapshot::decode(&wire), Err(Error::TooLarge));
	}

	#[test]
	#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
	fn desktop_adapter_rejects_unknown_methods_and_payloads() {
		use leptoskit_bridge::Plugin;
		let mut plugin = desktop::Device;
		assert_eq!(
			plugin.invoke(MethodId(99), Bytes::new()),
			Err(Error::Unavailable)
		);
		assert_eq!(
			plugin.invoke(SNAPSHOT, Bytes::from_static(&[1])),
			Err(Error::InvalidArgument)
		);
		let result = Snapshot::decode(&plugin.invoke(SNAPSHOT, Bytes::new()).unwrap()).unwrap();
		assert_eq!(result.kind, Kind::Desktop);
		assert!(result.architecture.is_some());
	}
}
