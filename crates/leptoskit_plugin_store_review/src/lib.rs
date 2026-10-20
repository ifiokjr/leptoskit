//! Requests an OS-controlled store review flow. Acceptance never proves that a
//! dialog appeared, a review was submitted, or a rating was selected.

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Native registration name.
pub const NAME: &str = "store-review";
/// Empty request; exact four-byte [`Availability`] response.
pub const AVAILABILITY: MethodId = MethodId(0);
/// Empty request; exact four-byte [`RequestAcceptance`] response.
pub const REQUEST: MethodId = MethodId(1);
/// Independently grantable methods.
pub const METHODS: &[MethodId] = &[AVAILABILITY, REQUEST];

/// Whether this implementation can attempt a request now. This is not store
/// eligibility, quota information, or a prediction that a dialog will appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Availability {
	/// This platform has no implemented store review backend.
	Unsupported = 0,
	/// The backend has an active owned presentation context.
	Ready = 1,
	/// The app's owned presentation context is detached or inactive.
	Inactive = 2,
	/// A prior native flow has not completed, even if its caller cancelled.
	Busy = 3,
}
impl Availability {
	/// Rejects unknown values and non-exact response lengths.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match word(bytes)? {
			0 => Ok(Self::Unsupported),
			1 => Ok(Self::Ready),
			2 => Ok(Self::Inactive),
			3 => Ok(Self::Busy),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Acknowledgment of the platform API call, with no review outcome information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum RequestAcceptance {
	/// `StoreKit` received the request, or Google Play completed its flow task.
	/// Either platform may have displayed no UI.
	Accepted = 1,
}
impl RequestAcceptance {
	/// Validates the exact response, without interpreting an empty reply as
	/// proof that any user interaction took place.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if word(bytes)? == 1 {
			Ok(Self::Accepted)
		} else {
			Err(Error::InvalidArgument)
		}
	}
}
fn word(bytes: &[u8]) -> Result<u32, Error> {
	Ok(u32::from_le_bytes(
		bytes.try_into().map_err(|_| Error::InvalidArgument)?,
	))
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Cached numeric native client. Construction never requests a review.
	#[derive(Debug, Clone, Copy)]
	pub struct StoreReview(leptoskit_bridge::guest::Client);
	impl StoreReview {
		/// Resolves the installed host plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries context readiness without opening UI or contacting a store.
		pub async fn availability(self) -> Result<Availability, Error> {
			Availability::decode(&self.0.call(AVAILABILITY, &[])?.await?)
		}

		/// Attempts a review flow. Cancellation cannot dismiss store-owned UI
		/// after presentation has begun.
		pub async fn request(self) -> Result<RequestAcceptance, Error> {
			RequestAcceptance::decode(&self.0.call(REQUEST, &[])?.await?)
		}
	}
}

/// Browser capability is explicit. Opening a store URL is a separate action
/// provided by the linking crate; this plugin never redirects as a fallback.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;
	/// Browsers have no supported native review flow.
	#[must_use]
	pub const fn availability() -> Availability {
		Availability::Unsupported
	}
	/// Always returns Unavailable without opening a page.
	pub fn request() -> Result<RequestAcceptance, Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn responses_reject_unknown_values_truncation_and_trailing_bytes() {
		for (value, expected) in [
			(0, Availability::Unsupported),
			(1, Availability::Ready),
			(2, Availability::Inactive),
			(3, Availability::Busy),
		] {
			assert_eq!(Availability::decode(&u32::to_le_bytes(value)), Ok(expected));
		}
		for bytes in [&[][..], &[1], &[1, 0, 0], &[1, 0, 0, 0, 0], &[4, 0, 0, 0]] {
			assert_eq!(Availability::decode(bytes), Err(Error::InvalidArgument));
		}
		assert_eq!(
			RequestAcceptance::decode(&[1, 0, 0, 0]),
			Ok(RequestAcceptance::Accepted)
		);
		for bytes in [&[][..], &[0, 0, 0, 0], &[2, 0, 0, 0], &[1, 0, 0, 0, 0]] {
			assert_eq!(
				RequestAcceptance::decode(bytes),
				Err(Error::InvalidArgument)
			);
		}
	}
}
