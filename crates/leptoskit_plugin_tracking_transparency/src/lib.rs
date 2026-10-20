//! Explicit iOS App Tracking Transparency status and authorization requests.
//! This crate never reads advertising identifiers or starts tracking. A status
//! is a current OS observation, not a permanent grant or a promise of a prompt.

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Native registration name.
pub const NAME: &str = "tracking-transparency";
/// Empty payload; returns one little-endian authorization word without
/// prompting.
pub const STATUS: MethodId = MethodId(0);
/// Empty payload; explicitly asks the OS and returns its resulting
/// authorization.
pub const REQUEST: MethodId = MethodId(1);
/// Supported operations.
pub const METHODS: &[MethodId] = &[STATUS, REQUEST];

/// Apple's current authorization, preserving restriction and undecided states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Status {
	/// No decision is currently available; a prompt may still be ineligible.
	NotDetermined = 0,
	/// OS policy prevents granting authorization.
	Restricted = 1,
	/// Authorization is denied.
	Denied = 2,
	/// The OS currently authorizes tracking. This crate performs none itself.
	Authorized = 3,
}
impl Status {
	/// Decodes exactly four bytes. Unknown values are errors, never grants.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match bytes {
			[0, 0, 0, 0] => Ok(Self::NotDetermined),
			[1, 0, 0, 0] => Ok(Self::Restricted),
			[2, 0, 0, 0] => Ok(Self::Denied),
			[3, 0, 0, 0] => Ok(Self::Authorized),
			_ => Err(Error::Native),
		}
	}

	/// The fixed four-byte response representation.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		(self as u32).to_le_bytes()
	}
}
/// Validates the bounded, empty request without accessing platform state.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	if !METHODS.contains(&method) {
		return Err(Error::Unavailable);
	}
	if !payload.is_empty() {
		return Err(Error::InvalidArgument);
	}
	Ok(())
}

/// Typed connection to the installed native authorization handler.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// Cached plugin registration.
	#[derive(Debug, Clone, Copy)]
	pub struct TrackingTransparency(leptoskit_bridge::guest::Client);
	impl TrackingTransparency {
		/// Resolves the explicitly selected host capability.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads current OS authorization without showing a prompt.
		pub async fn status(self) -> Result<Status, Error> {
			Status::decode(&self.0.call(STATUS, &[])?.await?)
		}

		/// Requests authorization following user intent. Dropping this future
		/// cancels the caller, but Apple exposes no way to dismiss its prompt.
		/// The process-wide request gate remains occupied until OS completion.
		pub async fn request(self) -> Result<Status, Error> {
			Status::decode(&self.0.call(REQUEST, &[])?.await?)
		}
	}
}

/// Browsers have no App Tracking Transparency API. Neither operation infers a
/// grant from browser privacy preferences or platform absence.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use std::future::Ready;
	use std::future::ready;

	use super::*;
	/// Always unavailable: browsers do not expose Apple's ATT authorization.
	pub fn status() -> Ready<Result<Status, Error>> {
		ready(Err(Error::Unavailable))
	}
	/// Always unavailable; no synthetic consent or prompt is provided.
	pub fn request() -> Ready<Result<Status, Error>> {
		ready(Err(Error::Unavailable))
	}
}

/// Explicit unavailable handler for native hosts without Apple's iOS adapter.
/// It never treats an unsupported platform as authorized.
#[derive(Debug, Default)]
pub struct Unsupported;
impl leptoskit_bridge::Plugin for Unsupported {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(
		&mut self,
		method: MethodId,
		payload: leptoskit_bridge::Bytes,
	) -> Result<leptoskit_bridge::Bytes, Error> {
		validate(method, &payload)?;
		Err(Error::Unavailable)
	}
}
#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin as _;

	use super::*;
	#[test]
	fn status_codec_preserves_all_states_and_rejects_unknown_or_trailing_data() {
		for status in [
			Status::NotDetermined,
			Status::Restricted,
			Status::Denied,
			Status::Authorized,
		] {
			assert_eq!(Status::decode(&status.encode()), Ok(status));
		}
		for bytes in [&[][..], &[0; 3], &[0; 5], &[4, 0, 0, 0], &[255; 4]] {
			assert_eq!(Status::decode(bytes), Err(Error::Native));
		}
	}
	#[test]
	fn unsupported_never_grants_and_validates_empty_requests() {
		let mut plugin = Unsupported;
		for method in METHODS {
			assert_eq!(
				plugin.invoke(*method, leptoskit_bridge::Bytes::new()),
				Err(Error::Unavailable)
			);
			assert_eq!(validate(*method, &[0]), Err(Error::InvalidArgument));
		}
		assert_eq!(validate(MethodId(2), &[]), Err(Error::Unavailable));
	}
}
