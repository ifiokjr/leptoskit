//! Mobile brightness overrides with explicit restoration. Android changes only
//! the activity window. iOS changes its screen and restores the previous value
//! on explicit restore or plugin close. This is not a system settings writer.

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Runtime registration name.
pub const NAME: &str = "brightness";
/// Empty input; returns one u32 setting (0..=1000, or `u32::MAX` for system
/// default).
pub const GET: MethodId = MethodId(0);
/// One u32 level (0..=1000); empty response.
pub const SET: MethodId = MethodId(1);
/// Empty input and response; restores the setting before the first override.
pub const RESTORE: MethodId = MethodId(2);
/// Supported wire operations.
pub const METHODS: &[MethodId] = &[GET, SET, RESTORE];

/// A bounded brightness fraction in thousandths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level(u16);
impl Level {
	/// Constructs a validated integer brightness level.
	pub fn new(thousandths: u16) -> Result<Self, Error> {
		if thousandths <= 1000 {
			Ok(Self(thousandths))
		} else {
			Err(Error::InvalidArgument)
		}
	}

	/// Rounds a finite fraction between zero and one to thousandths.
	pub fn from_fraction(value: f64) -> Result<Self, Error> {
		if !value.is_finite() || !(0.0..=1.0).contains(&value) {
			return Err(Error::InvalidArgument);
		}

		Ok(Self((value * 1000.0).round() as u16))
	}

	/// Returns the integer brightness fraction.
	#[must_use]
	pub fn thousandths(self) -> u16 {
		self.0
	}

	/// Encodes exactly four bytes for the native boundary.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		u32::from(self.0).to_le_bytes()
	}

	/// Validates a set-level payload without accepting system-default
	/// sentinels.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let value = u32::from_le_bytes(payload.try_into().map_err(|_| Error::InvalidArgument)?);
		Self::new(u16::try_from(value).map_err(|_| Error::InvalidArgument)?)
	}
}

/// An explicit override or the Android activity's inherited system setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
	/// Window inherits the OS setting; actual adaptive brightness is unknown.
	System,
	/// Native screen/window brightness level.
	Level(Level),
}
impl Setting {
	/// Validates an exact four-byte platform response.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload == u32::MAX.to_le_bytes() {
			Ok(Self::System)
		} else {
			Level::decode(payload).map(Self::Level)
		}
	}
}

/// Browsers expose no portable screen brightness API. CSS dimming is not used
/// as a substitute because it does not control backlight/panel brightness.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;
	/// Reports the absent browser capability explicitly.
	pub fn get() -> Result<Setting, Error> {
		Err(Error::Unavailable)
	}
	/// Reports the absent browser capability explicitly.
	pub fn set(_level: Level) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}

/// Cached native guest client. The native plugin owns restoration on shutdown.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// Resolved brightness plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Brightness(leptoskit_bridge::guest::Client);
	impl Brightness {
		/// Resolves a registered host plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads the current screen/window setting.
		pub async fn get(self) -> Result<Setting, Error> {
			Setting::decode(&self.0.call(GET, &[])?.await?)
		}

		/// Applies an override, preserving the previous setting for
		/// restoration.
		pub async fn set(self, level: Level) -> Result<(), Error> {
			if self.0.call(SET, &level.encode())?.await?.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}

		/// Restores the setting captured before the first override. Idempotent.
		pub async fn restore(self) -> Result<(), Error> {
			if self.0.call(RESTORE, &[])?.await?.is_empty() {
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
	fn fractions_reject_nonfinite_and_out_of_range_values() {
		assert_eq!(Level::from_fraction(0.375).unwrap().thousandths(), 375);

		for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01, 1.01] {
			assert_eq!(Level::from_fraction(value), Err(Error::InvalidArgument));
		}
	}
	#[test]
	fn wire_distinguishes_system_default_from_explicit_levels() {
		assert_eq!(
			Setting::decode(&u32::MAX.to_le_bytes()),
			Ok(Setting::System)
		);
		assert_eq!(
			Level::decode(&u32::MAX.to_le_bytes()),
			Err(Error::InvalidArgument)
		);

		for value in [0, 375, 1000] {
			let level = Level::new(value).unwrap();
			assert_eq!(Setting::decode(&level.encode()), Ok(Setting::Level(level)));
		}

		for payload in [&[][..], &[1], &[0, 0, 0, 0, 0], &[233, 3, 0, 0]] {
			assert_eq!(Setting::decode(payload), Err(Error::InvalidArgument));
		}
	}
}
