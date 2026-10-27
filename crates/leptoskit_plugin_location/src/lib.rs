//! Foreground location permission and bounded, cancellable acquisition.
//! Native reads never prompt. Call the separate permission request from an
//! intentional user action. No background tracking or geofence registration.

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Runtime registration name.
pub const NAME: &str = "location";
/// Empty request, eight-byte permission reply; never prompts.
pub const PERMISSION: MethodId = MethodId(0);
/// A u32 boolean requests fine accuracy where the OS supports that choice.
pub const REQUEST_PERMISSION: MethodId = MethodId(1);
/// Twelve-byte options request, tagged fix/timeout reply.
pub const CURRENT: MethodId = MethodId(2);
/// Options followed by a previous Unix timestamp as f64; returns a newer fix.
pub const NEXT: MethodId = MethodId(3);
/// Runtime methods.
pub const METHODS: &[MethodId] = &[PERMISSION, REQUEST_PERMISSION, CURRENT, NEXT];

/// Foreground permission state, distinct from device service availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Status {
	Unknown = 0,
	Prompt = 1,
	Denied = 2,
	Granted = 3,
	Restricted = 4,
}
/// Permission's accuracy scope, not the measurement's horizontal error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum AccuracyPermission {
	Unknown = 0,
	Reduced = 1,
	Full = 2,
}
/// Foreground authorization. Browser permission queries cannot report accuracy
/// scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permission {
	/// Current authorization state.
	pub status: Status,
	/// Whether the OS grants approximate or precise access.
	pub accuracy: AccuracyPermission,
}
impl Permission {
	/// Decodes the exact native permission response.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() != 8 {
			return Err(Error::InvalidArgument);
		}
		let status = match u32_value(&bytes[..4])? {
			0 => Status::Unknown,
			1 => Status::Prompt,
			2 => Status::Denied,
			3 => Status::Granted,
			4 => Status::Restricted,
			_ => return Err(Error::InvalidArgument),
		};
		let accuracy = match u32_value(&bytes[4..])? {
			0 => AccuracyPermission::Unknown,
			1 => AccuracyPermission::Reduced,
			2 => AccuracyPermission::Full,
			_ => return Err(Error::InvalidArgument),
		};
		if status != Status::Granted && accuracy != AccuracyPermission::Unknown {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { status, accuracy })
	}
}

/// The provider reported by Android. Other platforms explicitly return Unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Provider {
	Unknown = 0,
	Gps = 1,
	Network = 2,
	Passive = 3,
	Fused = 4,
}
/// A measured coordinate. Optional fields stay unknown instead of becoming
/// zeros.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
	/// Unix timestamp in milliseconds, as provided by the platform.
	pub timestamp_ms: f64,
	/// Latitude in degrees, -90..=90.
	pub latitude: f64,
	/// Longitude in degrees, -180..=180.
	pub longitude: f64,
	/// Horizontal uncertainty in metres, not a promise of actual error.
	pub accuracy_m: f64,
	/// Altitude in metres. Vertical datum is platform-specific.
	pub altitude_m: Option<f64>,
	/// Vertical uncertainty in metres when reported.
	pub altitude_accuracy_m: Option<f64>,
	/// Course of travel clockwise from north, 0..<360 degrees; not compass
	/// heading.
	pub heading_degrees: Option<f64>,
	/// Speed in metres per second.
	pub speed_mps: Option<f64>,
	/// Provider only when reported by the platform.
	pub provider: Provider,
	/// Native software-simulated/mock flag. Unknown on browsers.
	pub simulated: Option<bool>,
}
impl Fix {
	/// Validates coordinates and optional values before serializing or using a
	/// fix.
	pub fn validate(self) -> Result<Self, Error> {
		if !self.timestamp_ms.is_finite()
			|| self.timestamp_ms < 0.0
			|| !self.latitude.is_finite()
			|| !(-90.0..=90.0).contains(&self.latitude)
			|| !self.longitude.is_finite()
			|| !(-180.0..=180.0).contains(&self.longitude)
			|| !self.accuracy_m.is_finite()
			|| self.accuracy_m < 0.0
		{
			return Err(Error::InvalidArgument);
		}
		for value in [
			self.altitude_m,
			self.altitude_accuracy_m,
			self.heading_degrees,
			self.speed_mps,
		]
		.into_iter()
		.flatten()
		{
			if !value.is_finite() {
				return Err(Error::InvalidArgument);
			}
		}
		if self.altitude_accuracy_m.is_some_and(|v| v < 0.0)
			|| self.speed_mps.is_some_and(|v| v < 0.0)
			|| self
				.heading_degrees
				.is_some_and(|v| !(0.0..360.0).contains(&v))
		{
			return Err(Error::InvalidArgument);
		}
		Ok(self)
	}

	/// Fixed 80-byte little-endian wire representation, with explicit
	/// optional-value bits.
	pub fn encode(self) -> Result<[u8; 80], Error> {
		self.validate()?;
		let optional = [
			self.altitude_m,
			self.altitude_accuracy_m,
			self.heading_degrees,
			self.speed_mps,
		];
		let mask = optional
			.iter()
			.enumerate()
			.fold(0u32, |mask, (bit, value)| {
				mask | if value.is_some() { 1 << bit } else { 0 }
			});
		let mut bytes = [0; 80];
		bytes[..4].copy_from_slice(&mask.to_le_bytes());
		bytes[4..8].copy_from_slice(&(self.provider as u32).to_le_bytes());
		bytes[8..12].copy_from_slice(
			&self
				.simulated
				.map_or(0u32, |v| if v { 2 } else { 1 })
				.to_le_bytes(),
		);
		// Reserved field at 12..16 must remain zero.
		for (output, value) in bytes[16..].chunks_exact_mut(8).zip(
			[
				self.timestamp_ms,
				self.latitude,
				self.longitude,
				self.accuracy_m,
			]
			.into_iter()
			.chain(optional.into_iter().map(|v| v.unwrap_or(0.0))),
		) {
			output.copy_from_slice(&value.to_le_bytes());
		}
		Ok(bytes)
	}

	/// Strictly decodes the 80-byte fix payload, including reserved fields.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() != 80 {
			return Err(Error::InvalidArgument);
		}
		let mask = u32_value(&bytes[..4])?;
		if mask > 15 || u32_value(&bytes[12..16])? != 0 {
			return Err(Error::InvalidArgument);
		}
		let provider = match u32_value(&bytes[4..8])? {
			0 => Provider::Unknown,
			1 => Provider::Gps,
			2 => Provider::Network,
			3 => Provider::Passive,
			4 => Provider::Fused,
			_ => return Err(Error::InvalidArgument),
		};
		let simulated = match u32_value(&bytes[8..12])? {
			0 => None,
			1 => Some(false),
			2 => Some(true),
			_ => return Err(Error::InvalidArgument),
		};
		let mut values = [0.0; 8];
		for (value, input) in values.iter_mut().zip(bytes[16..].chunks_exact(8)) {
			*value = f64::from_le_bytes(input.try_into().map_err(|_| Error::InvalidArgument)?);
		}
		let mut optional = [None; 4];
		for (index, value) in values[4..].iter().enumerate() {
			if mask & (1 << index) != 0 {
				optional[index] = Some(*value);
			} else if value.to_bits() != 0 {
				return Err(Error::InvalidArgument);
			}
		}
		Self {
			timestamp_ms: values[0],
			latitude: values[1],
			longitude: values[2],
			accuracy_m: values[3],
			altitude_m: optional[0],
			altitude_accuracy_m: optional[1],
			heading_degrees: optional[2],
			speed_mps: optional[3],
			provider,
			simulated,
		}
		.validate()
	}
}
/// Validated acquisition bounds. Maximum age is an explicit cache tolerance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
	high_accuracy: bool,
	timeout_ms: u32,
	maximum_age_ms: u32,
}
impl Options {
	/// Timeout is 1..=120,000ms and maximum cached age is 0..=3,600,000ms.
	pub fn new(high_accuracy: bool, timeout_ms: u32, maximum_age_ms: u32) -> Result<Self, Error> {
		if timeout_ms == 0 || timeout_ms > 120_000 || maximum_age_ms > 3_600_000 {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			high_accuracy,
			timeout_ms,
			maximum_age_ms,
		})
	}

	/// Requests the best available precision without overriding approximate
	/// permission.
	pub fn high_accuracy(self) -> bool {
		self.high_accuracy
	}

	/// Total acquisition time budget in milliseconds.
	pub fn timeout_ms(self) -> u32 {
		self.timeout_ms
	}

	/// Maximum accepted cached fix age. Zero requests a fresh measurement.
	pub fn maximum_age_ms(self) -> u32 {
		self.maximum_age_ms
	}

	/// Exact request representation.
	pub fn encode(self) -> [u8; 12] {
		let mut bytes = [0; 12];
		for (slot, value) in bytes.chunks_exact_mut(4).zip([
			u32::from(self.high_accuracy),
			self.timeout_ms,
			self.maximum_age_ms,
		]) {
			slot.copy_from_slice(&value.to_le_bytes());
		}
		bytes
	}

	/// Validates options from a complete native request.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() != 12 {
			return Err(Error::InvalidArgument);
		}
		let accuracy = u32_value(&bytes[..4])?;
		if accuracy > 1 {
			return Err(Error::InvalidArgument);
		}
		Self::new(
			accuracy == 1,
			u32_value(&bytes[4..8])?,
			u32_value(&bytes[8..])?,
		)
	}
}
/// Acquisition failure. Timeout is distinct from permission denial or
/// unavailability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationError {
	Bridge(Error),
	TimedOut,
}
impl From<Error> for LocationError {
	fn from(value: Error) -> Self {
		Self::Bridge(value)
	}
}
impl std::fmt::Display for LocationError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::TimedOut => formatter.write_str("location acquisition timed out"),
			Self::Bridge(error) => write!(formatter, "location bridge error: {error}"),
		}
	}
}
impl std::error::Error for LocationError {}
/// Decodes a tagged native acquisition response.
pub fn decode_response(bytes: &[u8]) -> Result<Fix, LocationError> {
	if bytes.len() == 4 && bytes == 1u32.to_le_bytes() {
		return Err(LocationError::TimedOut);
	}
	if bytes.len() != 84 || u32_value(&bytes[..4])? != 0 {
		return Err(Error::InvalidArgument.into());
	}
	Ok(Fix::decode(&bytes[4..])?)
}
fn u32_value(bytes: &[u8]) -> Result<u32, Error> {
	Ok(u32::from_le_bytes(
		bytes.try_into().map_err(|_| Error::InvalidArgument)?,
	))
}

/// Reactive location fixes over this plugin's sources.
#[cfg(feature = "hooks")]
pub mod signals;

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

/// Cached native guest client; one outstanding acquisition per plugin.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// Foreground location capability.
	#[derive(Debug, Clone, Copy)]
	pub struct Location(leptoskit_bridge::guest::Client);
	impl Location {
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		pub async fn permission(self) -> Result<Permission, Error> {
			Permission::decode(&self.0.call(PERMISSION, &[])?.await?)
		}

		/// Requests foreground access; Android may grant approximate access
		/// only.
		pub async fn request_permission(self, precise: bool) -> Result<Permission, Error> {
			Permission::decode(
				&self
					.0
					.call(REQUEST_PERMISSION, &u32::from(precise).to_le_bytes())?
					.await?,
			)
		}

		/// Returns a fresh or explicitly age-tolerated fix. Reads never prompt.
		pub async fn current(self, options: Options) -> Result<Fix, LocationError> {
			decode_response(&self.0.call(CURRENT, &options.encode())?.await?)
		}

		/// Waits for a strictly newer timestamp. Dropping the call stops native
		/// tracking. Repeating this bounded operation is a foreground
		/// watch, not a durable path log.
		pub async fn next(
			self,
			options: Options,
			after_timestamp_ms: f64,
		) -> Result<Fix, LocationError> {
			if !after_timestamp_ms.is_finite() || after_timestamp_ms < 0.0 {
				return Err(Error::InvalidArgument.into());
			}
			let mut payload = [0; 20];
			payload[..12].copy_from_slice(&options.encode());
			payload[12..].copy_from_slice(&after_timestamp_ms.to_le_bytes());
			decode_response(&self.0.call(NEXT, &payload)?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn fix() -> Fix {
		Fix {
			timestamp_ms: 1_800_000_000_000.0,
			latitude: 37.0,
			longitude: -122.0,
			accuracy_m: 5.0,
			altitude_m: None,
			altitude_accuracy_m: None,
			heading_degrees: None,
			speed_mps: None,
			provider: Provider::Unknown,
			simulated: None,
		}
	}
	#[test]
	fn fix_roundtrip_preserves_unknown_and_zero_measurements() {
		let unknown = fix();
		assert_eq!(Fix::decode(&unknown.encode().unwrap()), Ok(unknown));
		let zero = Fix {
			altitude_m: Some(0.0),
			speed_mps: Some(0.0),
			heading_degrees: Some(0.0),
			altitude_accuracy_m: Some(0.0),
			simulated: Some(false),
			..unknown
		};
		assert_eq!(Fix::decode(&zero.encode().unwrap()), Ok(zero));
		assert_ne!(unknown.encode().unwrap(), zero.encode().unwrap());
	}
	#[test]
	fn invalid_wire_and_coordinate_data_are_rejected() {
		for latitude in [-91.0, 91.0, f64::NAN, f64::INFINITY] {
			assert_eq!(
				Fix { latitude, ..fix() }.encode(),
				Err(Error::InvalidArgument)
			);
		}
		for heading in [-1.0, 360.0, f64::NAN] {
			assert_eq!(
				Fix {
					heading_degrees: Some(heading),
					..fix()
				}
				.encode(),
				Err(Error::InvalidArgument)
			);
		}
		let encoded = fix().encode().unwrap();
		assert_eq!(Fix::decode(&encoded[..79]), Err(Error::InvalidArgument));
		for index in [0, 4, 8, 12, 79] {
			let mut corrupt = encoded;
			corrupt[index] = 255;
			assert_eq!(Fix::decode(&corrupt), Err(Error::InvalidArgument));
		}
		assert_eq!(decode_response(&[1, 0, 0, 0]), Err(LocationError::TimedOut));
		assert_eq!(
			decode_response(&[1, 0, 0, 0, 0]),
			Err(Error::InvalidArgument.into())
		);
	}
	#[test]
	fn request_bounds_and_authorization_scope_are_strict() {
		assert!(Options::new(true, 0, 0).is_err());
		assert!(Options::new(false, 120_001, 0).is_err());
		assert!(Options::new(false, 1000, 3_600_001).is_err());
		let options = Options::new(true, 30_000, 0).unwrap();
		assert_eq!(Options::decode(&options.encode()), Ok(options));
		assert!(Permission::decode(&[2, 0, 0, 0, 2, 0, 0, 0]).is_err());
		assert_eq!(
			Permission::decode(&[3, 0, 0, 0, 1, 0, 0, 0])
				.unwrap()
				.accuracy,
			AccuracyPermission::Reduced
		);
	}
	#[test]
	fn every_permission_status_and_scope_decodes_exactly() {
		let pair = |status: u32, accuracy: u32| {
			let mut bytes = status.to_le_bytes().to_vec();
			bytes.extend(accuracy.to_le_bytes());
			Permission::decode(&bytes)
		};
		for (value, status) in (0..).zip([
			Status::Unknown,
			Status::Prompt,
			Status::Denied,
			Status::Granted,
			Status::Restricted,
		]) {
			assert_eq!(
				pair(value, 0),
				Ok(Permission {
					status,
					accuracy: AccuracyPermission::Unknown
				})
			);
		}
		assert_eq!(pair(3, 2).unwrap().accuracy, AccuracyPermission::Full);
		assert_eq!(pair(5, 0), Err(Error::InvalidArgument));
		assert_eq!(pair(3, 3), Err(Error::InvalidArgument));
		assert_eq!(
			Permission::decode(&[3, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn every_provider_and_simulation_flag_roundtrips() {
		for provider in [
			Provider::Unknown,
			Provider::Gps,
			Provider::Network,
			Provider::Passive,
			Provider::Fused,
		] {
			for simulated in [None, Some(false), Some(true)] {
				let measured = Fix {
					provider,
					simulated,
					altitude_m: Some(-12.5),
					speed_mps: Some(3.0),
					..fix()
				};
				assert_eq!(Fix::decode(&measured.encode().unwrap()), Ok(measured));
			}
		}
		let mut unknown_provider = fix().encode().unwrap();
		unknown_provider[4] = 5;
		assert_eq!(Fix::decode(&unknown_provider), Err(Error::InvalidArgument));
		let mut unknown_simulation = fix().encode().unwrap();
		unknown_simulation[8] = 3;
		assert_eq!(
			Fix::decode(&unknown_simulation),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn options_accessors_and_strict_decoding() {
		let options = Options::new(false, 1, 3_600_000).unwrap();
		assert!(!options.high_accuracy());
		assert_eq!(options.timeout_ms(), 1);
		assert_eq!(options.maximum_age_ms(), 3_600_000);
		assert_eq!(Options::decode(&options.encode()), Ok(options));
		let mut invalid_accuracy = options.encode();
		invalid_accuracy[0] = 2;
		assert_eq!(
			Options::decode(&invalid_accuracy),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Options::decode(&options.encode()[..11]),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn tagged_responses_carry_fixes_or_typed_failures() {
		let mut response = 0u32.to_le_bytes().to_vec();
		response.extend(fix().encode().unwrap());
		assert_eq!(decode_response(&response), Ok(fix()));
		response[0] = 2;
		assert_eq!(
			decode_response(&response),
			Err(LocationError::Bridge(Error::InvalidArgument))
		);
		assert_eq!(
			LocationError::TimedOut.to_string(),
			"location acquisition timed out"
		);
		assert_eq!(
			LocationError::from(Error::Unavailable).to_string(),
			format!("location bridge error: {}", Error::Unavailable)
		);
	}
}
