//! <!-- {=p_device_policySummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_device_policy` as a direct dependency and select the
//! `device-policy` plugin in `native.plugins`; supported generated hosts
//! discover it from Cargo metadata. Generated Windows, Linux, and terminal
//! hosts currently omit runtime plugins; use explicit registration in a custom
//! host there. Guests enable the `guest` feature and reuse
//! `guest::DevicePolicy` rather than resolving the plugin on every call. Only
//! Android implements Device Admin / Device Policy Manager capabilities; every
//! other platform returns `Unavailable`. Status never prompts. Privileged
//! actions require an active device admin the user enabled, and wipe requires
//! an explicit confirmation token.
//!
//! <!-- {/p_device_policySummary} -->
//!
//! Wire integers are little-endian u32. Status is versioned. Activation takes a
//! length-prefixed UTF-8 explanation. Password quality and camera disable take
//! one word. Minimum length takes one word. Wipe takes version, confirmation
//! token, and flags. Decoders reject trailing data.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "device-policy";
/// Empty -> versioned [`Status`] without prompting.
pub const STATUS: MethodId = MethodId(0);
/// UTF-8 explanation -> [`ActivationOutcome`] after the system consent UI.
pub const REQUEST_ACTIVATION: MethodId = MethodId(1);
/// Empty -> locks the screen when admin is active.
pub const LOCK_NOW: MethodId = MethodId(2);
/// [`PasswordQuality`] -> applies that quality when admin is active.
pub const SET_PASSWORD_QUALITY: MethodId = MethodId(3);
/// Minimum length u32 -> applies that length when admin is active.
pub const SET_PASSWORD_MINIMUM_LENGTH: MethodId = MethodId(4);
/// Disabled flag u32 (0/1) -> applies camera restriction when admin is active.
pub const SET_CAMERA_DISABLED: MethodId = MethodId(5);
/// [`WipeRequest`] -> factory-reset when admin is active and confirmed.
pub const WIPE_DATA: MethodId = MethodId(6);
/// Empty -> removes this app's active device admin.
pub const REMOVE_ACTIVE: MethodId = MethodId(7);
/// Supported methods.
pub const METHODS: &[MethodId] = &[
	STATUS,
	REQUEST_ACTIVATION,
	LOCK_NOW,
	SET_PASSWORD_QUALITY,
	SET_PASSWORD_MINIMUM_LENGTH,
	SET_CAMERA_DISABLED,
	WIPE_DATA,
	REMOVE_ACTIVE,
];

/// Longest accepted activation explanation, in UTF-8 bytes.
const MAX_EXPLANATION_BYTES: usize = 512;
/// Wire confirmation token for [`WipeRequest`]. ASCII `WIPE`.
pub const WIPE_CONFIRMATION_TOKEN: u32 = 0x5749_5045;
/// Wipe may also erase shared/external storage the policy can reach.
pub const WIPE_FLAG_EXTERNAL_STORAGE: u32 = 1;

/// Whether this app's device-admin component is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum AdminState {
	/// The receiver is declared but the user has not enabled it.
	Inactive = 0,
	/// The user has enabled this app as a device admin.
	Active = 1,
	/// No usable device-admin receiver is available on this host.
	Unavailable = 2,
}

impl AdminState {
	fn decode(value: u32) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::Inactive),
			1 => Ok(Self::Active),
			2 => Ok(Self::Unavailable),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Password quality levels matching Android's documented Device Policy values.
/// Higher values are never implied by lower ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PasswordQuality {
	/// No quality constraint beyond what the user already chose.
	Unspecified = 0,
	/// Any non-empty credential.
	Something = 1,
	/// Numeric PIN.
	Numeric = 2,
	/// Alphabetic password.
	Alphabetic = 3,
	/// Alphanumeric password.
	Alphanumeric = 4,
	/// Complex password with the platform's symbol requirements.
	Complex = 5,
}

impl PasswordQuality {
	fn decode(value: u32) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::Unspecified),
			1 => Ok(Self::Something),
			2 => Ok(Self::Numeric),
			3 => Ok(Self::Alphabetic),
			4 => Ok(Self::Alphanumeric),
			5 => Ok(Self::Complex),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Encodes one quality word for [`SET_PASSWORD_QUALITY`].
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		(self as u32).to_le_bytes()
	}

	/// Decodes an exact four-byte quality request.
	pub fn decode_request(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let quality = Self::decode(input.u32()?)?;
		input.finish()?;
		Ok(quality)
	}
}

/// Methods this plugin implements when a device-admin receiver is available.
/// Bits never imply that Play Store policy or the app's `device_admin.xml`
/// granted the corresponding Android uses-policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities(u32);

impl Capabilities {
	const CAMERA: u32 = 1 << 4;
	const KNOWN: u32 =
		Self::LOCK | Self::WIPE | Self::PASSWORD_QUALITY | Self::PASSWORD_LENGTH | Self::CAMERA;
	const LOCK: u32 = 1 << 0;
	/// No implemented privileged methods.
	pub const NONE: Self = Self(0);
	const PASSWORD_LENGTH: u32 = 1 << 3;
	const PASSWORD_QUALITY: u32 = 1 << 2;
	/// Lock, wipe, password quality/length, and camera disable.
	pub const STANDARD: Self = Self(0b1_1111);
	const WIPE: u32 = 1 << 1;

	fn decode(value: u32) -> Result<Self, Error> {
		if value & !Self::KNOWN != 0 {
			return Err(Error::InvalidArgument);
		}
		Ok(Self(value))
	}

	/// Whether `lock_now` is implemented for an available receiver.
	#[must_use]
	pub const fn can_lock(self) -> bool {
		self.0 & Self::LOCK != 0
	}

	/// Whether `wipe_data` is implemented for an available receiver.
	#[must_use]
	pub const fn can_wipe(self) -> bool {
		self.0 & Self::WIPE != 0
	}

	/// Whether password quality can be set for an available receiver.
	#[must_use]
	pub const fn can_set_password_quality(self) -> bool {
		self.0 & Self::PASSWORD_QUALITY != 0
	}

	/// Whether password minimum length can be set for an available receiver.
	#[must_use]
	pub const fn can_set_password_length(self) -> bool {
		self.0 & Self::PASSWORD_LENGTH != 0
	}

	/// Whether camera disable can be set for an available receiver.
	#[must_use]
	pub const fn can_disable_camera(self) -> bool {
		self.0 & Self::CAMERA != 0
	}
}

/// Current device-admin observation. Capability bits describe this plugin's
/// implemented methods when a receiver is available; they are not Play Store
/// eligibility and do not prove the app's `device_admin.xml` declared every
/// policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
	/// Whether the app's device-admin component is active.
	pub state: AdminState,
	/// Methods this implementation exposes when a receiver is available.
	pub capabilities: Capabilities,
	/// Last observed camera-disabled policy when active; otherwise `None`.
	pub camera_disabled: Option<bool>,
	/// Last observed password quality when active; otherwise `None`.
	pub password_quality: Option<PasswordQuality>,
	/// Last observed minimum length when active; otherwise `None`.
	pub password_minimum_length: Option<u32>,
}

impl Status {
	/// Strictly validates a versioned status reply.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}

		let state = AdminState::decode(input.u32()?)?;
		let capabilities = Capabilities::decode(input.u32()?)?;
		let camera_disabled = optional_flag(input.u32()?)?;
		let password_quality = match input.u32()? {
			0 => None,
			value => Some(PasswordQuality::decode(value.wrapping_sub(1))?),
		};
		let password_minimum_length = match input.u32()? {
			u32::MAX => None,
			value => Some(value),
		};
		input.finish()?;

		if state != AdminState::Active
			&& (camera_disabled.is_some()
				|| password_quality.is_some()
				|| password_minimum_length.is_some())
		{
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			state,
			capabilities,
			camera_disabled,
			password_quality,
			password_minimum_length,
		})
	}

	/// Encodes a versioned status snapshot.
	pub fn encode(self) -> Result<Bytes, Error> {
		if self.state != AdminState::Active
			&& (self.camera_disabled.is_some()
				|| self.password_quality.is_some()
				|| self.password_minimum_length.is_some())
		{
			return Err(Error::InvalidArgument);
		}

		let mut output = Encoder::with_capacity(24);
		output.u32(1);
		output.u32(self.state as u32);
		output.u32(self.capabilities.0);
		output.u32(match self.camera_disabled {
			None => 0,
			Some(false) => 1,
			Some(true) => 2,
		});
		output.u32(match self.password_quality {
			None => 0,
			Some(quality) => (quality as u32) + 1,
		});
		output.u32(self.password_minimum_length.unwrap_or(u32::MAX));
		Ok(output.finish())
	}
}

/// Result of presenting Android's device-admin activation UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ActivationOutcome {
	/// Admin was already active; no UI was shown.
	AlreadyActive = 0,
	/// The user enabled the device admin.
	Activated = 1,
	/// The user dismissed or declined the system UI.
	Declined = 2,
}

impl ActivationOutcome {
	/// Decodes an exact four-byte activation reply.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match word(bytes)? {
			0 => Ok(Self::AlreadyActive),
			1 => Ok(Self::Activated),
			2 => Ok(Self::Declined),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Encodes the activation outcome.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		(self as u32).to_le_bytes()
	}
}

/// Validated explanation shown by Android's device-admin activation screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationRequest {
	explanation: String,
}

impl ActivationRequest {
	/// Requires a non-blank explanation of at most 512 UTF-8 bytes.
	pub fn new(explanation: impl Into<String>) -> Result<Self, Error> {
		let explanation = explanation.into();
		validate_text(&explanation, MAX_EXPLANATION_BYTES)?;
		Ok(Self { explanation })
	}

	/// User-visible explanation.
	pub fn explanation(&self) -> &str {
		&self.explanation
	}

	/// Encodes the length-prefixed explanation.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(self.explanation.len() + 4);
		output.bytes(self.explanation.as_bytes())?;
		Ok(output.finish())
	}

	/// Validates UTF-8, bounds, and complete consumption.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let explanation = input.string()?;
		validate_text(explanation, MAX_EXPLANATION_BYTES)?;
		input.finish()?;
		Self::new(explanation)
	}
}

/// Explicit factory-reset request. Construct only through
/// [`WipeRequest::factory_reset`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WipeRequest {
	wipe_external_storage: bool,
}

impl WipeRequest {
	/// Confirmed factory reset that keeps shared/external storage unless
	/// [`WipeRequest::with_external_storage`] is set.
	#[must_use]
	pub const fn factory_reset() -> Self {
		Self {
			wipe_external_storage: false,
		}
	}

	/// Also requests wiping shared/external storage the policy can reach.
	#[must_use]
	pub const fn with_external_storage(mut self) -> Self {
		self.wipe_external_storage = true;
		self
	}

	/// Whether external storage wipe was requested.
	#[must_use]
	pub const fn wipe_external_storage(self) -> bool {
		self.wipe_external_storage
	}

	/// Encodes version, confirmation token, and flags.
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(12);
		output.u32(1);
		output.u32(WIPE_CONFIRMATION_TOKEN);
		output.u32(if self.wipe_external_storage {
			WIPE_FLAG_EXTERNAL_STORAGE
		} else {
			0
		});
		output.finish()
	}

	/// Requires the confirmation token; unknown flags are rejected.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}
		if input.u32()? != WIPE_CONFIRMATION_TOKEN {
			return Err(Error::InvalidArgument);
		}
		let flags = input.u32()?;
		if flags & !WIPE_FLAG_EXTERNAL_STORAGE != 0 {
			return Err(Error::InvalidArgument);
		}
		input.finish()?;
		Ok(Self {
			wipe_external_storage: flags & WIPE_FLAG_EXTERNAL_STORAGE != 0,
		})
	}
}

/// Encodes a boolean camera-disabled request.
pub fn encode_camera_disabled(disabled: bool) -> [u8; 4] {
	u32::from(disabled).to_le_bytes()
}

/// Decodes an exact camera-disabled request.
pub fn decode_camera_disabled(bytes: &[u8]) -> Result<bool, Error> {
	flag(word(bytes)?)
}

/// Encodes a password minimum length.
pub fn encode_password_minimum_length(length: u32) -> Result<[u8; 4], Error> {
	if length == u32::MAX {
		return Err(Error::InvalidArgument);
	}
	Ok(length.to_le_bytes())
}

/// Decodes an exact password minimum length request.
pub fn decode_password_minimum_length(bytes: &[u8]) -> Result<u32, Error> {
	let length = word(bytes)?;
	if length == u32::MAX {
		return Err(Error::InvalidArgument);
	}
	Ok(length)
}

/// Validates empty payloads for methods that take no arguments.
pub fn validate_empty(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	if !matches!(method, STATUS | LOCK_NOW | REMOVE_ACTIVE) {
		return Err(Error::Unavailable);
	}
	if payload.is_empty() {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}

fn flag(value: u32) -> Result<bool, Error> {
	match value {
		0 => Ok(false),
		1 => Ok(true),
		_ => Err(Error::InvalidArgument),
	}
}

fn optional_flag(value: u32) -> Result<Option<bool>, Error> {
	match value {
		0 => Ok(None),
		1 => Ok(Some(false)),
		2 => Ok(Some(true)),
		_ => Err(Error::InvalidArgument),
	}
}

fn word(bytes: &[u8]) -> Result<u32, Error> {
	Ok(u32::from_le_bytes(
		bytes.try_into().map_err(|_| Error::InvalidArgument)?,
	))
}

fn validate_text(value: &str, limit: usize) -> Result<(), Error> {
	if value.len() > limit {
		return Err(Error::TooLarge);
	}
	if value.trim().is_empty() || value.contains('\0') {
		return Err(Error::InvalidArgument);
	}
	Ok(())
}

/// Cached native interpreter client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// One resolved device-policy plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct DevicePolicy(leptoskit_bridge::guest::Client);

	impl DevicePolicy {
		/// Resolves the installed plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads admin state without prompting.
		pub async fn status(self) -> Result<Status, Error> {
			Status::decode(&self.0.call(STATUS, &[])?.await?)
		}

		/// Opens Android's device-admin activation UI after user intent.
		pub async fn request_activation(
			self,
			request: &ActivationRequest,
		) -> Result<ActivationOutcome, Error> {
			ActivationOutcome::decode(&self.0.call(REQUEST_ACTIVATION, &request.encode()?)?.await?)
		}

		/// Locks the screen immediately when admin is active.
		pub async fn lock_now(self) -> Result<(), Error> {
			Decoder::new(&self.0.call(LOCK_NOW, &[])?.await?).finish()
		}

		/// Sets an explicit password quality when admin is active.
		pub async fn set_password_quality(self, quality: PasswordQuality) -> Result<(), Error> {
			Decoder::new(
				&self
					.0
					.call(SET_PASSWORD_QUALITY, &quality.encode())?
					.await?,
			)
			.finish()
		}

		/// Sets an explicit password minimum length when admin is active.
		pub async fn set_password_minimum_length(self, length: u32) -> Result<(), Error> {
			Decoder::new(
				&self
					.0
					.call(
						SET_PASSWORD_MINIMUM_LENGTH,
						&encode_password_minimum_length(length)?,
					)?
					.await?,
			)
			.finish()
		}

		/// Enables or clears the camera-disabled policy when admin is active.
		pub async fn set_camera_disabled(self, disabled: bool) -> Result<(), Error> {
			Decoder::new(
				&self
					.0
					.call(SET_CAMERA_DISABLED, &encode_camera_disabled(disabled))?
					.await?,
			)
			.finish()
		}

		/// Factory-resets the device. Irreversible. Requires active admin and
		/// an explicit [`WipeRequest`].
		pub async fn wipe_data(self, request: WipeRequest) -> Result<(), Error> {
			Decoder::new(&self.0.call(WIPE_DATA, &request.encode())?.await?).finish()
		}

		/// Removes this app's active device admin.
		pub async fn remove_active(self) -> Result<(), Error> {
			Decoder::new(&self.0.call(REMOVE_ACTIVE, &[])?.await?).finish()
		}
	}
}

/// Non-Android platforms have no Device Policy Manager contract.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always unavailable; browsers have no device-admin API.
	pub fn status() -> Result<Status, Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable.
	pub fn request_activation(
		_request: &ActivationRequest,
	) -> impl core::future::Future<Output = Result<ActivationOutcome, Error>> {
		std::future::ready(Err(Error::Unavailable))
	}

	/// Always unavailable.
	pub fn lock_now() -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always unavailable.
	pub fn wipe_data(_request: WipeRequest) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn flags(values: &[u32]) -> Vec<u8> {
		values
			.iter()
			.flat_map(|value| value.to_le_bytes())
			.collect()
	}

	#[test]
	fn status_round_trips_active_and_inactive_snapshots() {
		let inactive = Status {
			state: AdminState::Inactive,
			capabilities: Capabilities::STANDARD,
			camera_disabled: None,
			password_quality: None,
			password_minimum_length: None,
		};
		assert_eq!(Status::decode(&inactive.encode().unwrap()), Ok(inactive));
		assert!(inactive.capabilities.can_lock());
		assert!(inactive.capabilities.can_wipe());

		let active = Status {
			state: AdminState::Active,
			capabilities: Capabilities::decode(0b1_1101).unwrap(),
			camera_disabled: Some(true),
			password_quality: Some(PasswordQuality::Complex),
			password_minimum_length: Some(8),
		};
		assert!(!active.capabilities.can_wipe());
		let encoded = active.encode().unwrap();
		assert_eq!(Status::decode(&encoded), Ok(active));
		for length in 0..encoded.len() {
			assert!(Status::decode(&encoded[..length]).is_err());
		}
		assert!(Capabilities::decode(1 << 5).is_err());
	}

	#[test]
	fn status_rejects_policy_observations_without_active_admin() {
		assert_eq!(
			Status {
				state: AdminState::Inactive,
				capabilities: Capabilities::NONE,
				camera_disabled: Some(false),
				password_quality: None,
				password_minimum_length: None,
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
		assert!(Status::decode(&flags(&[1, 0, 0, 2, 0, u32::MAX])).is_err());
	}

	#[test]
	fn activation_requests_require_a_clear_explanation() {
		let request = ActivationRequest::new("Lock lost phones for your team").unwrap();
		assert_eq!(
			ActivationRequest::decode(&request.encode().unwrap()),
			Ok(request)
		);
		assert_eq!(ActivationRequest::new(" \n"), Err(Error::InvalidArgument));
		assert_eq!(
			ActivationRequest::new("reason\0hidden"),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			ActivationRequest::new("x".repeat(MAX_EXPLANATION_BYTES + 1)),
			Err(Error::TooLarge)
		);
	}

	#[test]
	fn wipe_requires_the_confirmation_token_and_known_flags() {
		let request = WipeRequest::factory_reset().with_external_storage();
		assert!(request.wipe_external_storage());
		assert_eq!(WipeRequest::decode(&request.encode()), Ok(request));
		assert_eq!(
			WipeRequest::decode(&flags(&[1, 0, 0])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			WipeRequest::decode(&flags(&[1, WIPE_CONFIRMATION_TOKEN, 2])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			WipeRequest::decode(&flags(&[2, WIPE_CONFIRMATION_TOKEN, 0])),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn password_camera_and_empty_payloads_are_exact() {
		assert_eq!(
			PasswordQuality::decode_request(&PasswordQuality::Numeric.encode()),
			Ok(PasswordQuality::Numeric)
		);
		assert_eq!(
			PasswordQuality::decode_request(&flags(&[6])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			decode_camera_disabled(&encode_camera_disabled(true)),
			Ok(true)
		);
		assert_eq!(
			decode_camera_disabled(&flags(&[2])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			decode_password_minimum_length(&encode_password_minimum_length(12).unwrap()),
			Ok(12)
		);
		assert_eq!(
			encode_password_minimum_length(u32::MAX),
			Err(Error::InvalidArgument)
		);
		assert_eq!(validate_empty(STATUS, &[]), Ok(()));
		assert_eq!(validate_empty(LOCK_NOW, &[0]), Err(Error::InvalidArgument));
		assert_eq!(
			validate_empty(SET_PASSWORD_QUALITY, &[]),
			Err(Error::Unavailable)
		);
		assert_eq!(
			ActivationOutcome::decode(&ActivationOutcome::Activated.encode()),
			Ok(ActivationOutcome::Activated)
		);
	}
}
