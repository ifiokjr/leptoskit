//! Bounded step-counter readings with explicit platform count domains.
//! Android reports its cumulative boot counter while active. iOS reports steps
//! for the requested observation/history interval. These are not daily totals.
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
pub use leptoskit_sensor::SensorError;
pub use leptoskit_sensor::Timeout;
/// Automatic native registration name.
pub const NAME: &str = "pedometer";
/// Hardware availability, empty request.
pub const AVAILABLE: MethodId = MethodId(0);
/// Current permission, empty request; never prompts.
pub const PERMISSION: MethodId = MethodId(1);
/// Explicit permission request, empty request; bounded to 30 seconds.
pub const REQUEST_PERMISSION: MethodId = MethodId(2);
/// One live reading, u32 deadline milliseconds.
pub const NEXT: MethodId = MethodId(3);
/// iOS history: start/end Unix milliseconds as f64, u32 deadline.
pub const HISTORY: MethodId = MethodId(4);
/// Complete native method table.
pub const METHODS: &[MethodId] = &[AVAILABLE, PERMISSION, REQUEST_PERMISSION, NEXT, HISTORY];
/// OS authorization, without guessing whether Android has permanently denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
	/// Android cannot distinguish an initial request from permanent denial.
	Unknown,
	/// An explicit request may show a prompt.
	Prompt,
	/// Access denied.
	Denied,
	/// Access currently granted.
	Granted,
	/// Access restricted by policy.
	Restricted,
}
impl Permission {
	/// Validates the complete permission reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		match payload {
			[0, 0, 0, 0] => Ok(Self::Unknown),
			[1, 0, 0, 0] => Ok(Self::Prompt),
			[2, 0, 0, 0] => Ok(Self::Denied),
			[3, 0, 0, 0] => Ok(Self::Granted),
			[4, 0, 0, 0] => Ok(Self::Restricted),
			_ => Err(Error::InvalidArgument),
		}
	}
}
/// Native reported steps, with the original counter scope preserved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reading {
	/// iOS observation interval. Algorithmic estimates, not ground truth.
	Interval {
		/// Start Unix milliseconds.
		start_ms: f64,
		/// End Unix milliseconds.
		end_ms: f64,
		/// Native reported steps for this interval.
		reported_steps: u64,
	},
	/// Android cumulative counter since reboot while the sensor was active.
	BootCounter {
		/// Event monotonic seconds since boot.
		timestamp: f64,
		/// Counter reported as a float by Android, with its precision limits.
		reported_steps: u64,
	},
}
impl Reading {
	/// Validates exact length, scope, reserved bytes and clock values.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() != 32 || payload[4..8] != [0; 4] {
			return Err(Error::InvalidArgument);
		}

		let scope = u32::from_le_bytes(
			payload[..4]
				.try_into()
				.map_err(|_| Error::InvalidArgument)?,
		);
		let reported_steps = u64::from_le_bytes(
			payload[8..16]
				.try_into()
				.map_err(|_| Error::InvalidArgument)?,
		);
		let first = f64::from_le_bytes(
			payload[16..24]
				.try_into()
				.map_err(|_| Error::InvalidArgument)?,
		);
		let second = f64::from_le_bytes(
			payload[24..]
				.try_into()
				.map_err(|_| Error::InvalidArgument)?,
		);

		if !first.is_finite() || !second.is_finite() || first < 0.0 {
			return Err(Error::InvalidArgument);
		}

		match scope {
			0 if second >= first => {
				Ok(Self::Interval {
					start_ms: first,
					end_ms: second,
					reported_steps,
				})
			}

			1 if second.to_bits() == 0 => {
				Ok(Self::BootCounter {
					timestamp: first,
					reported_steps,
				})
			}

			_ => Err(Error::InvalidArgument),
		}
	}
}
/// A history interval. iOS additionally requires it to be within the past seven
/// days.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct History {
	start_ms: f64,
	end_ms: f64,
}
impl History {
	/// Validates finite, positive duration Unix-millisecond bounds.
	pub fn new(start_ms: f64, end_ms: f64) -> Result<Self, Error> {
		if start_ms.is_finite() && end_ms.is_finite() && start_ms >= 0.0 && end_ms > start_ms {
			Ok(Self { start_ms, end_ms })
		} else {
			Err(Error::InvalidArgument)
		}
	}

	/// Exact native request payload.
	#[must_use]
	pub fn encode(self, timeout: Timeout) -> [u8; 20] {
		let mut bytes = [0; 20];
		bytes[..8].copy_from_slice(&self.start_ms.to_le_bytes());
		bytes[8..16].copy_from_slice(&self.end_ms.to_le_bytes());
		bytes[16..].copy_from_slice(&timeout.milliseconds().to_le_bytes());
		bytes
	}
}
/// Native guest client. Requests never start until polled.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// Cached bridge handle.
	#[derive(Debug, Clone, Copy)]
	pub struct Pedometer(leptoskit_bridge::guest::Client);
	impl Pedometer {
		/// Resolves the selected plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Checks native step-counter availability.
		pub async fn available(self) -> Result<bool, Error> {
			leptoskit_sensor::decode_available(&self.0.call(AVAILABLE, &[])?.await?)
		}

		/// Observes authorization without requesting access.
		pub async fn permission(self) -> Result<Permission, Error> {
			Permission::decode(&self.0.call(PERMISSION, &[])?.await?)
		}

		/// Explicitly requests access. Canceling cannot dismiss an OS prompt.
		pub async fn request_permission(self) -> Result<Permission, Error> {
			Permission::decode(&self.0.call(REQUEST_PERMISSION, &[])?.await?)
		}

		/// One bounded live observation. Cancellation stops sensor updates.
		pub async fn next(self, timeout: Timeout) -> Result<Reading, SensorError> {
			self.read(NEXT, &timeout.milliseconds().to_le_bytes()).await
		}

		/// iOS-only history query. The OS query itself cannot be canceled; the
		/// plugin retains its Busy slot until that callback arrives.
		pub async fn history(
			self,
			range: History,
			timeout: Timeout,
		) -> Result<Reading, SensorError> {
			self.read(HISTORY, &range.encode(timeout)).await
		}

		async fn read(self, method: MethodId, payload: &[u8]) -> Result<Reading, SensorError> {
			Ok(Reading::decode(leptoskit_sensor::bounded_reply(
				&self.0.call(method, payload)?.await?,
			)?)?)
		}
	}
}
/// No browser pedometer API is available.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;
	/// Browsers do not expose a standard step counter.
	pub fn available() -> bool {
		false
	}
	/// Returns unavailable without deriving steps from other sensors.
	pub fn next(_: Timeout) -> std::future::Ready<Result<Reading, SensorError>> {
		std::future::ready(Err(Error::Unavailable.into()))
	}
}
/// No desktop pedometer backend is provided.
#[cfg(not(target_arch = "wasm32"))]
pub mod desktop {
	use super::*;
	/// No portable desktop step-counter API.
	pub fn available() -> bool {
		false
	}
	/// Returns unavailable without guessing a count.
	pub fn next(_: Timeout) -> std::future::Ready<Result<Reading, SensorError>> {
		std::future::ready(Err(Error::Unavailable.into()))
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn scopes_and_invalid_clocks_remain_distinct() {
		let mut bytes = [0; 32];
		bytes[8..16].copy_from_slice(&13_u64.to_le_bytes());
		bytes[16..24].copy_from_slice(&100_f64.to_le_bytes());
		bytes[24..].copy_from_slice(&200_f64.to_le_bytes());
		assert_eq!(
			Reading::decode(&bytes),
			Ok(Reading::Interval {
				start_ms: 100.0,
				end_ms: 200.0,
				reported_steps: 13
			})
		);
		bytes[0] = 1;

		assert_eq!(Reading::decode(&bytes), Err(Error::InvalidArgument));
		bytes[24..].fill(0);
		assert_eq!(
			Reading::decode(&bytes),
			Ok(Reading::BootCounter {
				timestamp: 100.0,
				reported_steps: 13
			})
		);
		bytes[16..24].copy_from_slice(&f64::NAN.to_le_bytes());
		assert_eq!(Reading::decode(&bytes), Err(Error::InvalidArgument));
		assert!(History::new(2.0, 1.0).is_err());
		assert!(History::new(f64::INFINITY, 3.0).is_err());
	}
	#[test]
	fn reserved_bytes_and_lengths_are_rejected() {
		let mut reserved = [0; 32];
		reserved[24..].copy_from_slice(&1_f64.to_le_bytes());
		assert!(Reading::decode(&reserved).is_ok());
		reserved[4] = 1;
		assert_eq!(Reading::decode(&reserved), Err(Error::InvalidArgument));
		assert_eq!(Reading::decode(&[0; 31]), Err(Error::InvalidArgument));
		let mut negative = [0; 32];
		negative[16..24].copy_from_slice(&(-1_f64).to_le_bytes());
		assert_eq!(Reading::decode(&negative), Err(Error::InvalidArgument));
		let mut unknown_scope = [0; 32];
		unknown_scope[0] = 2;
		assert_eq!(Reading::decode(&unknown_scope), Err(Error::InvalidArgument));
	}
	#[test]
	fn every_permission_state_decodes_exactly() {
		for (byte, expected) in [
			(0, Permission::Unknown),
			(1, Permission::Prompt),
			(2, Permission::Denied),
			(3, Permission::Granted),
			(4, Permission::Restricted),
		] {
			assert_eq!(Permission::decode(&[byte, 0, 0, 0]), Ok(expected));
		}

		for invalid in [&[5, 0, 0, 0][..], &[3], &[3, 0, 0, 0, 0], &[3, 1, 0, 0]] {
			assert_eq!(Permission::decode(invalid), Err(Error::InvalidArgument));
		}
	}
	#[test]
	fn history_requests_encode_bounds_then_timeout() {
		let bytes = History::new(1_000.0, 2_000.5)
			.unwrap()
			.encode(Timeout::new(500).unwrap());
		assert_eq!(bytes[..8], 1_000_f64.to_le_bytes());
		assert_eq!(bytes[8..16], 2_000.5_f64.to_le_bytes());
		assert_eq!(bytes[16..], 500_u32.to_le_bytes());
	}
	#[cfg(not(target_arch = "wasm32"))]
	#[test]
	fn desktop_reports_unavailable_without_guessing_a_count() {
		assert!(!desktop::available());
		assert_eq!(
			desktop::next(Timeout::new(1).unwrap()).into_inner(),
			Err(SensorError::Bridge(Error::Unavailable))
		);
	}
}
