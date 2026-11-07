//! <!-- {=p_power_monitorSummary|trim|linePrefix:"//! ":true} -->
//!
//! Native hosts construct [`PowerMonitor`] and may expose it through the
//! bridge. There is no automatic `native.plugins` registration. The host pushes
//! suspend/resume and related lifecycle events from its event loop; idle-time
//! queries work on Windows today and return `Unavailable` elsewhere until a
//! platform adapter lands. Mobile and browsers return `Unavailable` for idle
//! queries; the queue and snapshot codecs still compile everywhere.
//!
//! <!-- {/p_power_monitorSummary} -->
//!
//! Maps to Electron's [`powerMonitor`](https://www.electronjs.org/docs/latest/api/power-monitor).
//! Battery charge belongs to `leptoskit_plugin_battery`; display-idle
//! inhibition belongs to `leptoskit_plugin_keep_awake`. This crate covers
//! power-source and session lifecycle events plus system idle queries.

use std::collections::VecDeque;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error as BridgeError;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Maximum queued power events. Overflow increments [`PowerMonitor::lost`].
pub const EVENT_CAPACITY: usize = 128;
/// Empty request -> versioned [`Snapshot`].
pub const SNAPSHOT: MethodId = MethodId(0);
/// Empty request -> drained event batch (lost count, then event tags).
pub const EVENTS: MethodId = MethodId(1);
/// Empty request -> idle milliseconds as little-endian u32 (`u32::MAX` if
/// over).
pub const IDLE_TIME: MethodId = MethodId(2);
/// Little-endian u32 threshold -> one byte (`0`/`1`) idle state.
pub const IS_IDLE: MethodId = MethodId(3);

/// Power and session lifecycle events a desktop host can observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Event {
	/// The system is about to sleep.
	Suspend = 1,
	/// The system woke from sleep.
	Resume = 2,
	/// The session screen locked.
	LockScreen = 3,
	/// The session screen unlocked.
	UnlockScreen = 4,
	/// External AC power connected.
	OnAc = 5,
	/// Running on battery power.
	OnBattery = 6,
	/// The system is shutting down.
	Shutdown = 7,
	/// The user became active after idle.
	UserDidBecomeActive = 8,
	/// The user crossed the configured idle threshold.
	UserDidBecomeIdle = 9,
}

impl Event {
	fn encode(self) -> u32 {
		self as u32
	}

	fn decode(value: u32) -> Result<Self, BridgeError> {
		Ok(match value {
			1 => Self::Suspend,
			2 => Self::Resume,
			3 => Self::LockScreen,
			4 => Self::UnlockScreen,
			5 => Self::OnAc,
			6 => Self::OnBattery,
			7 => Self::Shutdown,
			8 => Self::UserDidBecomeActive,
			9 => Self::UserDidBecomeIdle,
			_ => return Err(BridgeError::InvalidArgument),
		})
	}
}

/// Instantaneous power and idle snapshot. Unknown fields stay `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Snapshot {
	/// Whether the machine is on battery, when the host knows.
	pub on_battery: Option<bool>,
	/// Whether the session appears locked, when the host knows.
	pub locked: Option<bool>,
	/// Milliseconds since last user input, when the platform can measure it.
	pub idle_ms: Option<u32>,
}

impl Snapshot {
	/// Wire version 1: three optional flags/values as little-endian u32s.
	/// `u32::MAX` means unknown for each field; booleans use 0/1.
	pub fn encode(&self) -> Bytes {
		let mut output = Encoder::with_capacity(16);
		output.u32(1);
		output.u32(match self.on_battery {
			None => u32::MAX,
			Some(false) => 0,
			Some(true) => 1,
		});
		output.u32(match self.locked {
			None => u32::MAX,
			Some(false) => 0,
			Some(true) => 1,
		});
		output.u32(self.idle_ms.unwrap_or(u32::MAX));
		output.finish()
	}

	/// Decodes a versioned snapshot and rejects trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, BridgeError> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(BridgeError::InvalidArgument);
		}
		let on_battery = match input.u32()? {
			u32::MAX => None,
			0 => Some(false),
			1 => Some(true),
			_ => return Err(BridgeError::InvalidArgument),
		};
		let locked = match input.u32()? {
			u32::MAX => None,
			0 => Some(false),
			1 => Some(true),
			_ => return Err(BridgeError::InvalidArgument),
		};
		let idle = input.u32()?;
		input.finish()?;
		Ok(Self {
			on_battery,
			locked,
			idle_ms: (idle != u32::MAX).then_some(idle),
		})
	}
}

/// Failures from idle queries and bridge mapping.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// Platform cannot measure idle time or power events natively.
	#[error("power monitoring is unavailable on this platform")]
	Unavailable,
	/// Invalid configuration or payload.
	#[error("invalid power monitor argument")]
	InvalidArgument,
}

impl From<Error> for BridgeError {
	fn from(error: Error) -> Self {
		match error {
			Error::Unavailable => Self::Unavailable,
			Error::InvalidArgument => Self::InvalidArgument,
		}
	}
}

/// Host-owned power monitor: event queue plus optional idle queries.
#[derive(Debug, Default)]
pub struct PowerMonitor {
	events: VecDeque<Event>,
	lost: u64,
	on_battery: Option<bool>,
	locked: Option<bool>,
}

impl PowerMonitor {
	/// Creates an empty monitor. The host pushes OS events as they arrive.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Records an OS event, updating derived snapshot fields when relevant.
	pub fn push(&mut self, event: Event) {
		match event {
			Event::OnAc => self.on_battery = Some(false),
			Event::OnBattery => self.on_battery = Some(true),
			Event::LockScreen => self.locked = Some(true),
			Event::UnlockScreen => self.locked = Some(false),
			_ => {}
		}
		if self.events.len() >= EVENT_CAPACITY {
			self.events.pop_front();
			self.lost = self.lost.saturating_add(1);
		}
		self.events.push_back(event);
	}

	/// Number of events dropped because the queue was full.
	#[must_use]
	pub fn lost(&self) -> u64 {
		self.lost
	}

	/// Drains every queued event and clears the lost counter for this batch.
	pub fn drain(&mut self) -> (u64, Vec<Event>) {
		let lost = self.lost;
		self.lost = 0;
		(lost, self.events.drain(..).collect())
	}

	/// Snapshot combining host-pushed state with a live idle query when
	/// available.
	#[must_use]
	pub fn snapshot(&self) -> Snapshot {
		Snapshot {
			on_battery: self.on_battery,
			locked: self.locked,
			idle_ms: system_idle_time().ok(),
		}
	}

	/// Milliseconds since last user input.
	pub fn system_idle_time(&self) -> Result<u32, Error> {
		system_idle_time()
	}

	/// Whether idle time meets or exceeds `threshold_ms`.
	pub fn is_idle(&self, threshold_ms: u32) -> Result<bool, Error> {
		Ok(system_idle_time()? >= threshold_ms)
	}
}

impl Plugin for PowerMonitor {
	fn name(&self) -> &'static str {
		"power-monitor"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[SNAPSHOT, EVENTS, IDLE_TIME, IS_IDLE]
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, BridgeError> {
		match method {
			SNAPSHOT => {
				if !payload.is_empty() {
					return Err(BridgeError::InvalidArgument);
				}
				Ok(self.snapshot().encode())
			}
			EVENTS => {
				if !payload.is_empty() {
					return Err(BridgeError::InvalidArgument);
				}
				let (lost, events) = self.drain();
				let mut output = Encoder::with_capacity(16 + events.len() * 4);
				output.u32((lost >> 32) as u32);
				output.u32(lost as u32);
				output.u32(u32::try_from(events.len()).map_err(|_| BridgeError::TooLarge)?);
				for event in events {
					output.u32(event.encode());
				}
				Ok(output.finish())
			}
			IDLE_TIME => {
				if !payload.is_empty() {
					return Err(BridgeError::InvalidArgument);
				}
				let mut output = Encoder::with_capacity(4);
				output.u32(self.system_idle_time()?);
				Ok(output.finish())
			}
			IS_IDLE => {
				let mut input = Decoder::new(&payload);
				let threshold = input.u32()?;
				input.finish()?;
				Ok(Bytes::copy_from_slice(&[u8::from(
					self.is_idle(threshold)?,
				)]))
			}
			_ => Err(BridgeError::Unavailable),
		}
	}
}

/// Decodes an `EVENTS` reply for guests and tests.
pub fn decode_events(bytes: &[u8]) -> Result<(u64, Vec<Event>), BridgeError> {
	let mut input = Decoder::new(bytes);
	let lost_hi = u64::from(input.u32()?);
	let lost_lo = u64::from(input.u32()?);
	let count = input.u32()? as usize;
	let mut events = Vec::with_capacity(count);
	for _ in 0..count {
		events.push(Event::decode(input.u32()?)?);
	}
	input.finish()?;
	Ok(((lost_hi << 32) | lost_lo, events))
}

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub mod guest {
	use super::*;

	/// Cached guest client for the host's power monitor.
	#[derive(Debug, Clone, Copy)]
	pub struct PowerMonitorClient(leptoskit_bridge::guest::Client);

	impl PowerMonitorClient {
		/// Resolves the registered power-monitor plugin once.
		pub fn new() -> Result<Self, BridgeError> {
			leptoskit_bridge::guest::Client::new("power-monitor").map(Self)
		}

		/// Reads the current snapshot.
		pub async fn snapshot(self) -> Result<Snapshot, BridgeError> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}

		/// Drains queued events.
		pub async fn events(self) -> Result<(u64, Vec<Event>), BridgeError> {
			decode_events(&self.0.call(EVENTS, &[])?.await?)
		}

		/// Queries system idle milliseconds.
		pub async fn idle_time(self) -> Result<u32, BridgeError> {
			let bytes = self.0.call(IDLE_TIME, &[])?.await?;
			let mut input = Decoder::new(&bytes);
			let value = input.u32()?;
			input.finish()?;
			Ok(value)
		}
	}
}

#[cfg(windows)]
fn system_idle_time() -> Result<u32, Error> {
	use std::mem::size_of;

	use windows_sys::Win32::System::SystemInformation::GetTickCount;
	use windows_sys::Win32::UI::Input::GetLastInputInfo;
	use windows_sys::Win32::UI::Input::LASTINPUTINFO;

	let mut info = LASTINPUTINFO {
		cbSize: size_of::<LASTINPUTINFO>() as u32,
		dwTime: 0,
	};
	// SAFETY: `info` is correctly sized and writable for the Win32 API.
	if unsafe { GetLastInputInfo(&mut info) } == 0 {
		return Err(Error::Unavailable);
	}
	let ticks = unsafe { GetTickCount() };
	Ok(ticks.wrapping_sub(info.dwTime))
}

#[cfg(not(windows))]
fn system_idle_time() -> Result<u32, Error> {
	Err(Error::Unavailable)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn snapshot_round_trips_unknown_and_known_fields() {
		let original = Snapshot {
			on_battery: Some(true),
			locked: None,
			idle_ms: Some(1_500),
		};
		assert_eq!(Snapshot::decode(&original.encode()).unwrap(), original);
		assert_eq!(
			Snapshot::decode(&Snapshot::default().encode()).unwrap(),
			Snapshot::default()
		);
	}

	#[test]
	fn queue_overflow_reports_lost_events_and_bridge_drains() {
		let mut monitor = PowerMonitor::new();
		for _ in 0..EVENT_CAPACITY + 3 {
			monitor.push(Event::Suspend);
		}
		assert_eq!(monitor.lost(), 3);
		let (lost, events) = monitor.drain();
		assert_eq!(lost, 3);
		assert_eq!(events.len(), EVENT_CAPACITY);
		assert!(events.iter().all(|event| *event == Event::Suspend));

		monitor.push(Event::OnBattery);
		assert_eq!(monitor.snapshot().on_battery, Some(true));
		monitor.push(Event::Resume);
		let reply = monitor.invoke(EVENTS, Bytes::new()).unwrap();
		let (lost, events) = decode_events(&reply).unwrap();
		assert_eq!(lost, 0);
		assert_eq!(events, vec![Event::OnBattery, Event::Resume]);
		assert_eq!(
			monitor.invoke(EVENTS, Bytes::from_static(b"x")),
			Err(BridgeError::InvalidArgument)
		);
		assert_eq!(
			monitor.invoke(MethodId(9), Bytes::new()),
			Err(BridgeError::Unavailable)
		);
	}

	#[test]
	fn idle_queries_are_platform_specific() {
		let monitor = PowerMonitor::new();
		let result = monitor.system_idle_time();
		#[cfg(windows)]
		assert!(result.is_ok());
		#[cfg(not(windows))]
		assert!(matches!(result, Err(Error::Unavailable)));
	}
}
