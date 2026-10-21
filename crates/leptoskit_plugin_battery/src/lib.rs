//! Battery snapshots and cancellable change observation. Unknown values stay
//! unknown; a simulator or unavailable browser API never masquerades as a fully
//! charged physical battery.
//!
//! Snapshot messages are exactly four little-endian u32 fields (16 bytes):
//! presence (unknown/absent/present = 0/1/2), charge in thousandths (`u32::MAX`
//! for unknown), state (unknown/discharging/charging/full = 0/1/2/3), and power
//! saving (unknown/off/on = 0/1/2).

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime plugin name.
pub const NAME: &str = "battery";
/// Empty input -> a 16-byte [`Snapshot`].
pub const SNAPSHOT: MethodId = MethodId(0);
/// Previous snapshot -> latest changed snapshot. Mobile only; one waiter per
/// plugin. Dropping the guest future cancels the native subscription.
pub const WAIT_FOR_CHANGE: MethodId = MethodId(1);
/// Empty input -> little-endian u32 boolean. Android's app-specific battery
/// optimization policy; other platforms return `Unavailable`.
pub const OPTIMIZATION_ENABLED: MethodId = MethodId(2);

/// Validated battery charge in thousandths, from empty to full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Charge(u16);

impl Charge {
	/// Validates an integer fraction of full capacity.
	pub fn new(thousandths: u16) -> Result<Self, Error> {
		if thousandths <= 1000 {
			Ok(Self(thousandths))
		} else {
			Err(Error::InvalidArgument)
		}
	}

	/// Returns the fraction in thousandths.
	pub fn thousandths(self) -> u16 {
		self.0
	}

	/// Returns the fraction from 0.0 to 1.0.
	pub fn fraction(self) -> f64 {
		f64::from(self.0) / 1000.0
	}

	/// Rounds a platform fraction to the binary representation.
	#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
	pub fn from_fraction(value: f64) -> Result<Self, Error> {
		if !value.is_finite() || !(0.0..=1.0).contains(&value) {
			return Err(Error::InvalidArgument);
		}
		Ok(Self((value * 1000.0).round() as u16))
	}
}

/// Whether a battery is receiving or supplying power.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum State {
	/// The platform cannot report the state.
	Unknown = 0,
	/// Running on battery / not charging.
	Discharging = 1,
	/// Charging from external power.
	Charging = 2,
	/// Fully charged.
	Full = 3,
}

/// One platform battery reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
	/// None when the platform does not distinguish absent from unknown.
	pub present: Option<bool>,
	/// None when the charge cannot be measured.
	pub charge: Option<Charge>,
	/// Charging status.
	pub state: State,
	/// System-wide low-power mode, if the platform exposes it.
	pub power_save: Option<bool>,
}

fn flag(value: Option<bool>) -> u32 {
	match value {
		None => 0,
		Some(false) => 1,
		Some(true) => 2,
	}
}
fn read_flag(value: u32) -> Result<Option<bool>, Error> {
	match value {
		0 => Ok(None),
		1 => Ok(Some(false)),
		2 => Ok(Some(true)),
		_ => Err(Error::InvalidArgument),
	}
}

impl Snapshot {
	/// Encodes a fixed-size binary reply.
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(16);
		output.u32(flag(self.present));
		output.u32(self.charge.map_or(u32::MAX, |charge| u32::from(charge.0)));
		output.u32(self.state as u32);
		output.u32(flag(self.power_save));
		output.finish()
	}

	/// Decodes and validates every field and rejects trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let present = read_flag(input.u32()?)?;
		let charge = match input.u32()? {
			u32::MAX => None,
			value => {
				Some(Charge::new(
					u16::try_from(value).map_err(|_| Error::InvalidArgument)?,
				)?)
			}
		};
		let state = match input.u32()? {
			0 => State::Unknown,
			1 => State::Discharging,
			2 => State::Charging,
			3 => State::Full,
			_ => return Err(Error::InvalidArgument),
		};
		let power_save = read_flag(input.u32()?)?;
		input.finish()?;
		Ok(Self {
			present,
			charge,
			state,
			power_save,
		})
	}
}

/// Desktop APIs use the operating system's battery enumeration. Query off the
/// UI thread if disk/device latency matters; power-save mode is not inferred
/// from battery percentage.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub mod desktop {
	use std::time::Duration;
	use std::time::Instant;

	use super::*;

	/// A bounded latest-state poller. The caller schedules [`Self::poll`]; this
	/// type creates no thread or timer and retains only one previous reading.
	#[derive(Debug)]
	pub struct Watcher {
		interval: Duration,
		next: Instant,
		previous: Option<Vec<Snapshot>>,
	}

	impl Watcher {
		/// Sets an interval of at least one second to avoid busy device
		/// polling. The first call to [`Self::poll`] immediately
		/// returns the initial state.
		pub fn new(interval: Duration) -> Result<Self, Error> {
			if interval < Duration::from_secs(1) {
				return Err(Error::InvalidArgument);
			}
			let now = Instant::now();
			now.checked_add(interval).ok_or(Error::InvalidArgument)?;
			Ok(Self {
				interval,
				next: now,
				previous: None,
			})
		}

		/// Samples at most once per interval. `None` means either not due or
		/// unchanged. This performs OS I/O synchronously; schedule it off the
		/// UI thread. This is polling, not an operating-system
		/// notification stream.
		pub fn poll(&mut self) -> Result<Option<Vec<Snapshot>>, Error> {
			let now = Instant::now();
			if now < self.next {
				return Ok(None);
			}
			self.next = now
				.checked_add(self.interval)
				.ok_or(Error::InvalidArgument)?;
			let current = batteries()?;
			if self.previous.as_ref() == Some(&current) {
				return Ok(None);
			}
			self.previous = Some(current.clone());
			Ok(Some(current))
		}
	}
	/// Returns every enumerated battery. An empty list means no battery exists.
	pub fn batteries() -> Result<Vec<Snapshot>, Error> {
		let manager = starship_battery::Manager::new().map_err(|_| Error::Native)?;
		manager
			.batteries()
			.map_err(|_| Error::Native)?
			.map(|battery| {
				let battery = battery.map_err(|_| Error::Native)?;
				let state = match battery.state() {
					starship_battery::State::Charging => State::Charging,
					starship_battery::State::Discharging | starship_battery::State::Empty => {
						State::Discharging
					}
					starship_battery::State::Full => State::Full,
					_ => State::Unknown,
				};
				Ok(Snapshot {
					present: Some(true),
					charge: Some(Charge::from_fraction(f64::from(
						battery.state_of_charge().value,
					))?),
					state,
					power_save: None,
				})
			})
			.collect()
	}

	/// Bridge adapter for the first battery enumerated by the OS. Use
	/// [`batteries`] when an application needs separate multi-battery readings.
	#[derive(Debug, Default)]
	pub struct Battery;
	impl leptoskit_bridge::Plugin for Battery {
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
			Ok(batteries()?
				.first()
				.copied()
				.unwrap_or(Snapshot {
					present: Some(false),
					charge: None,
					state: State::Unknown,
					power_save: None,
				})
				.encode())
		}
	}
}

/// Browser Battery Status API. Unsupported browsers return `Unavailable`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

/// Cached interpreter client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// One resolved host plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Battery(leptoskit_bridge::guest::Client);
	impl Battery {
		/// Resolves the installed battery plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Waits until the platform differs from `previous`. Changes between
		/// calls are detected by sampling after observers are
		/// installed. Intermediate transitions may be coalesced; this
		/// is a latest-state subscription. A second concurrent waiter
		/// returns `Busy`. Dropping the future cancels it.
		pub async fn wait_for_change(self, previous: Snapshot) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(WAIT_FOR_CHANGE, &previous.encode())?.await?)
		}

		/// Whether Android battery optimizations apply to the host application.
		pub async fn optimization_enabled(self) -> Result<bool, Error> {
			let bytes = self.0.call(OPTIMIZATION_ENABLED, &[])?.await?;
			let mut input = Decoder::new(&bytes);
			let enabled = match input.u32()? {
				0 => false,
				1 => true,
				_ => return Err(Error::InvalidArgument),
			};
			input.finish()?;
			Ok(enabled)
		}

		/// Reads and validates the platform's binary response.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn snapshots_preserve_unknowns_and_validate_wire_values() {
		let reading = Snapshot {
			present: Some(true),
			charge: Some(Charge::new(370).unwrap()),
			state: State::Discharging,
			power_save: Some(false),
		};
		assert_eq!(
			reading.encode().as_ref(),
			&[2, 0, 0, 0, 114, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]
		);
		assert_eq!(Snapshot::decode(&reading.encode()), Ok(reading));
		let unknown = Snapshot {
			present: None,
			charge: None,
			state: State::Unknown,
			power_save: None,
		};
		assert_eq!(Snapshot::decode(&unknown.encode()), Ok(unknown));
		assert_eq!(
			Snapshot::decode(&reading.encode()[..15]),
			Err(Error::InvalidArgument)
		);
		let mut invalid = reading.encode().to_vec();
		invalid[0] = 3;
		assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
		assert_eq!(Charge::new(1001), Err(Error::InvalidArgument));
		assert_eq!(Charge::from_fraction(f64::NAN), Err(Error::InvalidArgument));
		assert_eq!(Charge::from_fraction(-0.1), Err(Error::InvalidArgument));
	}

	#[test]
	fn invalid_snapshot_fields_are_rejected_independently() {
		let snapshot = Snapshot {
			present: None,
			charge: None,
			state: State::Unknown,
			power_save: None,
		}
		.encode();
		for (field, value) in [(0, 3_u32), (1, 1001), (1, 65536), (2, 4), (3, 3)] {
			let mut bytes = snapshot.to_vec();
			bytes[field * 4..field * 4 + 4].copy_from_slice(&value.to_le_bytes());
			assert_eq!(Snapshot::decode(&bytes), Err(Error::InvalidArgument));
		}
		let mut trailing = snapshot.to_vec();
		trailing.push(0);
		assert_eq!(Snapshot::decode(&trailing), Err(Error::InvalidArgument));
		assert_eq!(Charge::from_fraction(0.37049).unwrap().thousandths(), 370);
		assert_eq!(Charge::from_fraction(0.37051).unwrap().thousandths(), 371);
		assert_eq!(
			Charge::from_fraction(f64::INFINITY),
			Err(Error::InvalidArgument)
		);
		assert_eq!(Charge::from_fraction(1.0001), Err(Error::InvalidArgument));
	}

	#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
	#[test]
	fn desktop_rejects_busy_polling_and_unsupported_rpc_before_os_access() {
		use std::time::Duration;

		use leptoskit_bridge::Plugin;

		assert!(matches!(
			desktop::Watcher::new(Duration::from_millis(999)),
			Err(Error::InvalidArgument)
		));
		assert!(matches!(
			desktop::Watcher::new(Duration::MAX),
			Err(Error::InvalidArgument)
		));
		assert!(desktop::Watcher::new(Duration::from_secs(1)).is_ok());
		let mut adapter = desktop::Battery;
		assert_eq!(
			adapter.invoke(WAIT_FOR_CHANGE, Bytes::new()),
			Err(Error::Unavailable)
		);
		assert_eq!(
			adapter.invoke(SNAPSHOT, Bytes::from_static(b"bad")),
			Err(Error::InvalidArgument)
		);
	}

	#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
	#[test]
	fn desktop_reads_a_valid_snapshot_and_reports_only_changes_when_due() {
		use std::time::Duration;

		use leptoskit_bridge::Plugin;

		// Machines with and without batteries both produce a well-formed reply;
		// a battery-less host reports `present: Some(false)`.
		let mut adapter = desktop::Battery;
		assert_eq!(adapter.name(), NAME);
		assert_eq!(adapter.methods(), &[SNAPSHOT]);
		let reply = adapter.invoke(SNAPSHOT, Bytes::new()).unwrap();
		let snapshot = Snapshot::decode(&reply).unwrap();
		let batteries = desktop::batteries().unwrap();
		assert_eq!(snapshot.present, Some(!batteries.is_empty()));
		for battery in &batteries {
			assert_eq!(battery.present, Some(true));
			assert!(
				battery
					.charge
					.is_some_and(|charge| charge.fraction() <= 1.0)
			);
		}

		let mut watcher = desktop::Watcher::new(Duration::from_secs(3600)).unwrap();
		assert_eq!(watcher.poll().unwrap(), Some(batteries));
		// The next sample is an hour away, so polling again does no OS I/O.
		assert_eq!(watcher.poll().unwrap(), None);
	}
}
