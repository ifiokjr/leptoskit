//! <!-- {=p_badgeSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_badge` as a direct dependency and select the `badge`
//! plugin in `native.plugins`; supported generated hosts discover it from Cargo
//! metadata. Generated Windows, Linux, and terminal hosts currently omit
//! runtime plugins; use explicit registration in a custom host there. Guests
//! enable the `guest` feature and reuse `guest::Badge` rather than resolving
//! the plugin on every call. Setting a count asks the OS to show an app-icon
//! badge; it never proves the badge is visible under the user's focus or
//! notification settings.
//!
//! <!-- {/p_badgeSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Native registration name.
pub const NAME: &str = "badge";
/// Empty payload; returns one little-endian count word.
pub const GET: MethodId = MethodId(0);
/// One little-endian count word; empty reply.
pub const SET: MethodId = MethodId(1);
/// Empty payload; clears the badge. Same as setting zero.
pub const CLEAR: MethodId = MethodId(2);
/// Independently grantable methods.
pub const METHODS: &[MethodId] = &[GET, SET, CLEAR];

/// Upper bound accepted by every backend. Larger values are invalid, never
/// clamped, so apps notice overflows instead of silent truncation.
pub const MAX_COUNT: u32 = 999_999;

/// Validates and returns a badge count.
pub fn decode_count(bytes: &[u8]) -> Result<u32, Error> {
	let count = u32::from_le_bytes(bytes.try_into().map_err(|_| Error::InvalidArgument)?);
	if count > MAX_COUNT {
		return Err(Error::InvalidArgument);
	}
	Ok(count)
}

/// Encodes a badge count for the wire.
pub fn encode_count(count: u32) -> Result<[u8; 4], Error> {
	if count > MAX_COUNT {
		return Err(Error::InvalidArgument);
	}
	Ok(count.to_le_bytes())
}

/// Empty successful replies only.
pub fn require_empty(reply: &[u8]) -> Result<(), Error> {
	if reply.is_empty() {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}

/// Validates a method and payload without contacting the OS.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		GET | CLEAR => {
			if payload.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}
		SET => {
			decode_count(payload)?;
			Ok(())
		}
		_ => Err(Error::Unavailable),
	}
}

/// Process-local badge count used by desktop hosts and tests.
#[derive(Debug, Default)]
pub struct Counter {
	count: u32,
}

impl Counter {
	/// Current count.
	#[must_use]
	pub fn get(&self) -> u32 {
		self.count
	}

	/// Sets the count after validation.
	pub fn set(&mut self, count: u32) -> Result<(), Error> {
		if count > MAX_COUNT {
			return Err(Error::InvalidArgument);
		}
		self.count = count;
		Ok(())
	}

	/// Clears the badge.
	pub fn clear(&mut self) {
		self.count = 0;
	}
}

impl leptoskit_bridge::Plugin for Counter {
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
			GET => Ok(Bytes::copy_from_slice(&encode_count(self.count)?)),
			SET => {
				self.set(decode_count(&payload)?)?;
				Ok(Bytes::new())
			}
			CLEAR => {
				self.clear();
				Ok(Bytes::new())
			}
			_ => Err(Error::Unavailable),
		}
	}
}

/// Linux Unity/KDE launcher badge over the session bus. Requires a desktop file
/// id such as `com.example.app.desktop`. Absence of a launcher entry host is
/// `Unavailable`, not a silent no-op.
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
pub mod linux {
	use std::collections::HashMap;
	use std::time::Duration;

	use dbus::arg::RefArg;
	use dbus::arg::Variant;
	use dbus::blocking::Connection;
	use dbus::channel::Sender;
	use dbus::strings::Interface;
	use dbus::strings::Member;
	use dbus::strings::Path;

	use super::*;

	/// Owns a session-bus connection and the desktop-file identity used in the
	/// `com.canonical.Unity.LauncherEntry` signal.
	pub struct Badge {
		connection: Connection,
		desktop_id: String,
		count: u32,
	}

	impl Badge {
		/// Connects to the session bus. `desktop_id` must end in `.desktop`.
		pub fn new(desktop_id: impl Into<String>) -> Result<Self, Error> {
			let desktop_id = desktop_id.into();
			if desktop_id.is_empty()
				|| desktop_id.len() > 256
				|| !desktop_id.ends_with(".desktop")
				|| desktop_id.chars().any(|ch| ch.is_control() || ch == '/')
			{
				return Err(Error::InvalidArgument);
			}
			let connection = Connection::new_session().map_err(|_| Error::Unavailable)?;
			Ok(Self {
				connection,
				desktop_id,
				count: 0,
			})
		}

		/// Current count remembered by this client.
		#[must_use]
		pub fn get(&self) -> u32 {
			self.count
		}

		/// Publishes the count to interested launcher hosts.
		pub fn set(&mut self, count: u32) -> Result<(), Error> {
			if count > MAX_COUNT {
				return Err(Error::InvalidArgument);
			}
			self.publish(count, count > 0)?;
			self.count = count;
			Ok(())
		}

		/// Clears the published badge.
		pub fn clear(&mut self) -> Result<(), Error> {
			self.set(0)
		}

		fn publish(&self, count: u32, visible: bool) -> Result<(), Error> {
			let hash = self
				.desktop_id
				.bytes()
				.fold(0_u32, |hash, byte| hash.wrapping_mul(31).wrapping_add(u32::from(byte)));
			let path = Path::new(format!("/com/canonical/unity/launcherentry/{hash}"))
				.map_err(|_| Error::InvalidArgument)?;
			let interface = Interface::new("com.canonical.Unity.LauncherEntry")
				.map_err(|_| Error::InvalidArgument)?;
			let member = Member::new("Update").map_err(|_| Error::InvalidArgument)?;
			let mut properties: HashMap<&str, Variant<Box<dyn RefArg>>> = HashMap::new();
			properties.insert("count", Variant(Box::new(i64::from(count))));
			properties.insert("count-visible", Variant(Box::new(visible)));
			let signal = dbus::message::Message::signal(&path, &interface, &member)
				.append2(format!("application://{}", self.desktop_id), properties);
			self.connection
				.send(signal)
				.map_err(|_| Error::Unavailable)?;
			let _ = self.connection.process(Duration::from_millis(10));
			Ok(())
		}
	}
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Cached numeric native client.
	#[derive(Debug, Clone, Copy)]
	pub struct Badge(leptoskit_bridge::guest::Client);

	impl Badge {
		/// Resolves the installed host plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads the count last accepted by the host.
		pub async fn get(self) -> Result<u32, Error> {
			decode_count(&self.0.call(GET, &[])?.await?)
		}

		/// Asks the OS to show this badge count.
		pub async fn set(self, count: u32) -> Result<(), Error> {
			require_empty(&self.0.call(SET, &encode_count(count)?)?.await?)
		}

		/// Clears the badge.
		pub async fn clear(self) -> Result<(), Error> {
			require_empty(&self.0.call(CLEAR, &[])?.await?)
		}
	}
}

/// Browser badges use the Badging API when the page is an installed app.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always unavailable until a dedicated web adapter lands.
	pub fn get() -> Result<u32, Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable until a dedicated web adapter lands.
	pub fn set(_count: u32) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable until a dedicated web adapter lands.
	pub fn clear() -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin as _;

	use super::*;

	#[test]
	fn count_codec_and_counter_plugin() {
		assert_eq!(decode_count(&encode_count(42).unwrap()).unwrap(), 42);
		assert!(decode_count(&[1, 2, 3]).is_err());
		assert!(encode_count(MAX_COUNT + 1).is_err());

		let mut plugin = Counter::default();
		assert_eq!(plugin.name(), NAME);
		assert!(plugin
			.invoke(SET, Bytes::copy_from_slice(&encode_count(7).unwrap()))
			.unwrap()
			.is_empty());
		assert_eq!(
			decode_count(&plugin.invoke(GET, Bytes::new()).unwrap()).unwrap(),
			7
		);
		assert!(plugin.invoke(CLEAR, Bytes::new()).unwrap().is_empty());
		assert_eq!(plugin.get(), 0);
		assert_eq!(
			validate(SET, &(MAX_COUNT + 1).to_le_bytes()),
			Err(Error::InvalidArgument)
		);
	}
}
