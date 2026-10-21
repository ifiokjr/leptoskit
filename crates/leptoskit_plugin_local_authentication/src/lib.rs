//! Local device-owner authentication, not account identity or server
//! authorization. Success is emitted only by the platform's
//! authentication-success callback. Android requires API 30 and Class 3
//! biometrics; credential fallback is explicit.
//!
//! Wire integers are little-endian u32. Capability request: policy.
//! Authentication request: version=1, policy, then length-prefixed UTF-8 reason
//! and cancel label. Capabilities: version, policy, readiness, modality mask,
//! device-secure flag (unknown/no/yes=0/1/2). Authentication reply: version,
//! outcome, method. Cancel takes and returns empty bytes. All decoders reject
//! trailing data.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime name.
pub const NAME: &str = "local-authentication";
/// Policy u32 -> capabilities, without showing a prompt.
pub const CAPABILITIES: MethodId = MethodId(0);
/// [`Request`] -> terminal [`Outcome`], with at most one active prompt.
pub const AUTHENTICATE: MethodId = MethodId(1);
/// Empty -> dismiss this plugin's prompt; its request returns `AppCancelled`.
pub const CANCEL: MethodId = MethodId(2);

/// Accepted authenticators. Device credential fallback is never implicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Policy {
	/// Biometrics only. Android restricts this to Class 3 (strong) sensors.
	BiometricOnly = 0,
	/// Biometrics or the operating system's device PIN, pattern, or password.
	BiometricOrCredential = 1,
}
impl Policy {
	/// Decodes the capability request.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let value = read_policy(input.u32()?)?;
		input.finish()?;
		Ok(value)
	}
}
fn read_policy(value: u32) -> Result<Policy, Error> {
	match value {
		0 => Ok(Policy::BiometricOnly),
		1 => Ok(Policy::BiometricOrCredential),
		_ => Err(Error::InvalidArgument),
	}
}

/// Validated user-visible prompt configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
	policy: Policy,
	reason: String,
	cancel_label: String,
}
impl Request {
	/// Requires a non-blank reason of at most 256 UTF-8 bytes.
	pub fn new(policy: Policy, reason: impl Into<String>) -> Result<Self, Error> {
		let reason = reason.into();
		validate_text(&reason, 256)?;
		Ok(Self {
			policy,
			reason,
			cancel_label: "Cancel".into(),
		})
	}

	/// Sets a non-blank cancel label of at most 64 UTF-8 bytes. Android
	/// credential prompts use the system's button label rather than a
	/// custom negative button.
	pub fn with_cancel_label(mut self, label: impl Into<String>) -> Result<Self, Error> {
		let label = label.into();
		validate_text(&label, 64)?;
		self.cancel_label = label;
		Ok(self)
	}

	/// Explicit authenticator policy.
	pub fn policy(&self) -> Policy {
		self.policy
	}

	/// Reason displayed by the operating system.
	pub fn reason(&self) -> &str {
		&self.reason
	}

	/// Requested cancellation label.
	pub fn cancel_label(&self) -> &str {
		&self.cancel_label
	}

	/// Encodes validated prompt options.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(self.reason.len() + self.cancel_label.len() + 16);
		output.u32(1);
		output.u32(self.policy as u32);
		output.bytes(self.reason.as_bytes())?;
		output.bytes(self.cancel_label.as_bytes())?;
		Ok(output.finish())
	}

	/// Validates version, policy, UTF-8, bounds and complete consumption.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}
		let policy = read_policy(input.u32()?)?;
		let reason = input.string()?;
		validate_text(reason, 256)?;
		let cancel = input.string()?;
		validate_text(cancel, 64)?;
		input.finish()?;
		Self::new(policy, reason)?.with_cancel_label(cancel)
	}
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

/// Current ability to evaluate the requested policy, without authenticating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Readiness {
	/// Policy can currently be evaluated.
	Ready = 0,
	/// Hardware or platform is unavailable.
	Unavailable = 1,
	/// No acceptable biometric is enrolled.
	NotEnrolled = 2,
	/// The OS has locked out authentication.
	LockedOut = 3,
	/// No device credential is configured.
	CredentialNotSet = 4,
	/// Required Android manifest permission is absent.
	PermissionDenied = 5,
	/// A security update is required before authentication.
	SecurityUpdateRequired = 6,
	/// The OS cannot establish readiness.
	Unknown = 7,
}
/// Non-identifying hardware modalities; these do not assert enrollment or
/// strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modalities(u32);
impl Modalities {
	/// Whether fingerprint hardware is reported.
	pub fn fingerprint(self) -> bool {
		self.0 & 1 != 0
	}

	/// Whether face hardware is reported.
	pub fn face(self) -> bool {
		self.0 & 2 != 0
	}

	/// Whether iris hardware is reported.
	pub fn iris(self) -> bool {
		self.0 & 4 != 0
	}

	/// Whether Apple Optic ID hardware is reported.
	pub fn optic(self) -> bool {
		self.0 & 8 != 0
	}
}
/// OS capability snapshot for one policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
	/// Policy evaluated by the native implementation.
	pub policy: Policy,
	/// Availability/enrollment/lockout status, not proof of authentication.
	pub readiness: Readiness,
	/// Reported sensor types, without implying all meet the selected security
	/// policy.
	pub modalities: Modalities,
	/// Whether a device credential is configured, if the OS exposes it.
	pub device_secure: Option<bool>,
}
impl Capabilities {
	/// Decodes and validates the fixed 20-byte capability response.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}
		let policy = read_policy(input.u32()?)?;
		let readiness = match input.u32()? {
			0 => Readiness::Ready,
			1 => Readiness::Unavailable,
			2 => Readiness::NotEnrolled,
			3 => Readiness::LockedOut,
			4 => Readiness::CredentialNotSet,
			5 => Readiness::PermissionDenied,
			6 => Readiness::SecurityUpdateRequired,
			7 => Readiness::Unknown,
			_ => return Err(Error::InvalidArgument),
		};
		let mask = input.u32()?;
		if mask & !15 != 0 {
			return Err(Error::InvalidArgument);
		}
		let device_secure = match input.u32()? {
			0 => None,
			1 => Some(false),
			2 => Some(true),
			_ => return Err(Error::InvalidArgument),
		};
		input.finish()?;
		Ok(Self {
			policy,
			readiness,
			modalities: Modalities(mask),
			device_secure,
		})
	}
}

/// Provenance of a successful authentication, when the OS exposes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum AuthenticationMethod {
	/// The OS verified the selected policy but did not disclose the method.
	Unknown = 0,
	/// Biometric verification succeeded.
	Biometric = 1,
	/// The device credential was verified by the OS.
	DeviceCredential = 2,
}
/// Terminal authentication result. Retryable Android non-matches keep the
/// prompt active; they are not interpreted as successful or terminal
/// authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
	/// OS-confirmed success and method provenance.
	Authenticated(AuthenticationMethod),
	/// The OS terminated an unsuccessful attempt.
	Failed,
	/// The user dismissed the system prompt.
	UserCancelled,
	/// The system interrupted the prompt.
	SystemCancelled,
	/// The application explicitly cancelled the prompt.
	AppCancelled,
	/// Authentication was locked out.
	LockedOut,
	/// No acceptable biometrics are enrolled.
	NotEnrolled,
	/// Authentication is unavailable.
	Unavailable,
	/// Device credential has not been configured.
	CredentialNotSet,
	/// OS authentication timed out.
	TimedOut,
	/// The platform requires a security update.
	SecurityUpdateRequired,
}
impl Outcome {
	/// Decodes a fixed 12-byte result. Failure replies must not contain a
	/// successful method, and credential fallback cannot satisfy
	/// `BiometricOnly`.
	pub fn decode(bytes: &[u8], policy: Policy) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}
		let status = input.u32()?;
		let method = input.u32()?;
		input.finish()?;
		if status != 0 && method != 0 {
			return Err(Error::InvalidArgument);
		}
		match status {
			0 => {
				let method = match method {
					0 if policy == Policy::BiometricOrCredential => AuthenticationMethod::Unknown,
					1 => AuthenticationMethod::Biometric,
					2 if policy == Policy::BiometricOrCredential => {
						AuthenticationMethod::DeviceCredential
					}
					_ => return Err(Error::InvalidArgument),
				};
				Ok(Self::Authenticated(method))
			}
			1 => Ok(Self::Failed),
			2 => Ok(Self::UserCancelled),
			3 => Ok(Self::SystemCancelled),
			4 => Ok(Self::AppCancelled),
			5 => Ok(Self::LockedOut),
			6 => Ok(Self::NotEnrolled),
			7 => Ok(Self::Unavailable),
			8 => Ok(Self::CredentialNotSet),
			9 => Ok(Self::TimedOut),
			10 => Ok(Self::SecurityUpdateRequired),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Browsers cannot provide this native device-owner authentication contract.
/// `WebAuthn` is a different relying-party protocol and is not silently
/// substituted.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;
	/// Always returns Unavailable without showing browser UI.
	pub fn capabilities(_policy: Policy) -> Result<Capabilities, Error> {
		Err(Error::Unavailable)
	}
	/// Always returns Unavailable; this never pretends to authenticate a user.
	pub fn authenticate(_request: &Request) -> impl Future<Output = Result<Outcome, Error>> {
		std::future::ready(Err(Error::Unavailable))
	}
}

/// Cached native interpreter client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// One resolved local authentication plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct LocalAuthentication(leptoskit_bridge::guest::Client);
	impl LocalAuthentication {
		/// Resolves the installed plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Checks policy capability without prompting.
		pub async fn capabilities(self, policy: Policy) -> Result<Capabilities, Error> {
			let result = Capabilities::decode(
				&self
					.0
					.call(CAPABILITIES, &(policy as u32).to_le_bytes())?
					.await?,
			)?;
			if result.policy != policy {
				return Err(Error::InvalidArgument);
			}
			Ok(result)
		}

		/// Shows the OS prompt and waits for a terminal result. Dropping the
		/// pending call cancels native work; no cached authentication is
		/// returned.
		pub async fn authenticate(self, request: &Request) -> Result<Outcome, Error> {
			Outcome::decode(
				&self.0.call(AUTHENTICATE, &request.encode()?)?.await?,
				request.policy(),
			)
		}

		/// Explicitly dismisses this plugin's active prompt, if any.
		pub async fn cancel(self) -> Result<(), Error> {
			Decoder::new(&self.0.call(CANCEL, &[])?.await?).finish()
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn fields(fields: &[u32]) -> Vec<u8> {
		fields
			.iter()
			.flat_map(|value| value.to_le_bytes())
			.collect()
	}
	#[test]
	fn requests_validate_bounds_encoding_policy_and_prompt_text() {
		let request = Request::new(Policy::BiometricOnly, "Unlock your notes")
			.unwrap()
			.with_cancel_label("Dismiss")
			.unwrap();
		let encoded = request.encode().unwrap();
		assert_eq!(Request::decode(&encoded), Ok(request));
		for length in 0..encoded.len() {
			assert!(Request::decode(&encoded[..length]).is_err());
		}
		assert!(Request::new(Policy::BiometricOnly, " \n").is_err());
		assert!(Request::new(Policy::BiometricOnly, "reason\0hidden").is_err());
		assert_eq!(
			Request::new(Policy::BiometricOnly, "x".repeat(257)),
			Err(Error::TooLarge)
		);
		for offset in [0, 4, 12] {
			let mut invalid = encoded.to_vec();
			invalid[offset] = 255;
			assert!(Request::decode(&invalid).is_err());
		}
		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert!(Request::decode(&trailing).is_err());
	}
	#[test]
	fn readiness_never_implies_authentication() {
		let bytes = fields(&[1, 0, 0, 3, 2]);
		let capabilities = Capabilities::decode(&bytes).unwrap();
		assert_eq!(capabilities.readiness, Readiness::Ready);
		assert!(capabilities.modalities.face() && capabilities.modalities.fingerprint());
		for length in 0..bytes.len() {
			assert!(Capabilities::decode(&bytes[..length]).is_err());
		}
		for values in [[1, 0, 8, 0, 0], [1, 0, 0, 16, 0], [1, 0, 0, 0, 3]] {
			assert!(Capabilities::decode(&fields(&values)).is_err());
		}
	}
	#[test]
	fn outcomes_reject_downgrades_and_inconsistent_failure_provenance() {
		assert_eq!(
			Outcome::decode(&fields(&[1, 0, 1]), Policy::BiometricOnly),
			Ok(Outcome::Authenticated(AuthenticationMethod::Biometric))
		);
		assert_eq!(
			Outcome::decode(&fields(&[1, 0, 2]), Policy::BiometricOnly),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Outcome::decode(&fields(&[1, 0, 0]), Policy::BiometricOnly),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Outcome::decode(&fields(&[1, 0, 0]), Policy::BiometricOrCredential),
			Ok(Outcome::Authenticated(AuthenticationMethod::Unknown))
		);
		assert_eq!(
			Outcome::decode(&fields(&[1, 0, 2]), Policy::BiometricOrCredential),
			Ok(Outcome::Authenticated(
				AuthenticationMethod::DeviceCredential
			))
		);
		assert_eq!(
			Outcome::decode(&fields(&[1, 2, 0]), Policy::BiometricOnly),
			Ok(Outcome::UserCancelled)
		);
		assert_eq!(
			Outcome::decode(&fields(&[1, 4, 0]), Policy::BiometricOnly),
			Ok(Outcome::AppCancelled)
		);
		for values in [[1, 1, 1], [1, 11, 0], [1, 0, 3], [2, 0, 1]] {
			assert!(Outcome::decode(&fields(&values), Policy::BiometricOnly).is_err());
		}
	}
	#[test]
	fn every_terminal_failure_status_decodes_without_provenance() {
		let expected = [
			Outcome::Failed,
			Outcome::UserCancelled,
			Outcome::SystemCancelled,
			Outcome::AppCancelled,
			Outcome::LockedOut,
			Outcome::NotEnrolled,
			Outcome::Unavailable,
			Outcome::CredentialNotSet,
			Outcome::TimedOut,
			Outcome::SecurityUpdateRequired,
		];
		for (status, outcome) in (1..).zip(expected) {
			assert_eq!(
				Outcome::decode(&fields(&[1, status, 0]), Policy::BiometricOrCredential),
				Ok(outcome)
			);
		}
	}
	#[test]
	fn every_readiness_and_modality_is_decoded_exactly() {
		let expected = [
			Readiness::Ready,
			Readiness::Unavailable,
			Readiness::NotEnrolled,
			Readiness::LockedOut,
			Readiness::CredentialNotSet,
			Readiness::PermissionDenied,
			Readiness::SecurityUpdateRequired,
			Readiness::Unknown,
		];
		for (value, readiness) in (0..).zip(expected) {
			let decoded = Capabilities::decode(&fields(&[1, 1, value, 0, 0])).unwrap();
			assert_eq!(decoded.policy, Policy::BiometricOrCredential);
			assert_eq!(decoded.readiness, readiness);
			assert_eq!(decoded.device_secure, None);
		}
		let iris_and_optic = Capabilities::decode(&fields(&[1, 0, 0, 12, 1])).unwrap();
		assert!(iris_and_optic.modalities.iris() && iris_and_optic.modalities.optic());
		assert!(!iris_and_optic.modalities.face() && !iris_and_optic.modalities.fingerprint());
		assert_eq!(iris_and_optic.device_secure, Some(false));
		assert_eq!(
			Capabilities::decode(&fields(&[2, 0, 0, 0, 0])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Capabilities::decode(&fields(&[1, 2, 0, 0, 0])),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn policy_requests_and_prompt_accessors_are_exact() {
		assert_eq!(Policy::decode(&fields(&[0])), Ok(Policy::BiometricOnly));
		assert_eq!(
			Policy::decode(&fields(&[1])),
			Ok(Policy::BiometricOrCredential)
		);
		assert_eq!(Policy::decode(&fields(&[2])), Err(Error::InvalidArgument));
		assert!(Policy::decode(&fields(&[1, 0])).is_err());
		let request = Request::new(Policy::BiometricOrCredential, "Approve transfer").unwrap();
		assert_eq!(request.policy(), Policy::BiometricOrCredential);
		assert_eq!(request.reason(), "Approve transfer");
		assert_eq!(request.cancel_label(), "Cancel");
		assert_eq!(
			request.clone().with_cancel_label("x".repeat(65)),
			Err(Error::TooLarge)
		);
		assert_eq!(request.with_cancel_label(" "), Err(Error::InvalidArgument));
	}
}
