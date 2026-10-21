//! Explicit camera and microphone permission status and requests.
//!
//! Querying status never displays a prompt. Request only after the user chooses
//! a feature requiring access. Platform permission and hardware availability
//! are separate: a grant does not prove a camera/microphone exists or is
//! usable.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Native registration name.
pub const NAME: &str = "permissions";
/// Permission kind u32 -> status and can-prompt u32 (eight bytes).
pub const STATUS: MethodId = MethodId(0);
/// Permission kind u32 -> resulting status, possibly after an OS prompt.
pub const REQUEST: MethodId = MethodId(1);
/// Supported methods.
pub const METHODS: &[MethodId] = &[STATUS, REQUEST];

/// Supported capabilities; additional permissions require distinct OS
/// workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Permission {
	/// Camera capture.
	Camera = 0,
	/// Microphone recording.
	Microphone = 1,
}

impl Permission {
	/// Strict, allocation-free decoding of exactly one little-endian integer.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		match payload {
			[0, 0, 0, 0] => Ok(Self::Camera),
			[1, 0, 0, 0] => Ok(Self::Microphone),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Four-byte wire representation.
	pub fn encode(self) -> [u8; 4] {
		(self as u32).to_le_bytes()
	}
}

/// An OS authorization state. Android cannot distinguish a never-requested
/// permission from a permanent denial using its status API alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Status {
	/// The platform cannot distinguish more specific non-granted states.
	Unknown = 0,
	/// No choice has been made; an explicit request can prompt.
	NotDetermined = 1,
	/// Access denied by the user or OS.
	Denied = 2,
	/// The OS has granted access (hardware can still be unavailable).
	Granted = 3,
	/// A platform restriction prevents the user granting access.
	Restricted = 4,
}

/// Permission state and whether a request can display a prompt. `None` means
/// the platform cannot establish whether it will prompt. No settings app is
/// opened implicitly when a permission cannot be requested again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Authorization {
	/// Reported permission state.
	pub status: Status,
	/// Whether a request can ask the user (unknown is not a grant).
	pub can_prompt: Option<bool>,
}

impl Authorization {
	/// Decodes exactly eight bytes, rejecting unknown enum values and trailers.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let (status, prompt) = payload.split_at_checked(4).ok_or(Error::Native)?;
		let status = match status {
			[0, 0, 0, 0] => Status::Unknown,
			[1, 0, 0, 0] => Status::NotDetermined,
			[2, 0, 0, 0] => Status::Denied,
			[3, 0, 0, 0] => Status::Granted,
			[4, 0, 0, 0] => Status::Restricted,
			_ => return Err(Error::Native),
		};
		let can_prompt = match prompt {
			[0, 0, 0, 0] => None,
			[1, 0, 0, 0] => Some(false),
			[2, 0, 0, 0] => Some(true),
			_ => return Err(Error::Native),
		};
		Ok(Self { status, can_prompt })
	}

	/// Encodes status and a tri-state can-prompt flag.
	pub fn encode(self) -> Bytes {
		let mut bytes = Vec::with_capacity(8);
		bytes.extend_from_slice(&(self.status as u32).to_le_bytes());
		bytes.extend_from_slice(
			&self
				.can_prompt
				.map_or(0_u32, |value| if value { 2 } else { 1 })
				.to_le_bytes(),
		);
		bytes.into()
	}
}

/// Browser permissions. Requesting captures a temporary media stream and stops
/// all tracks before returning; this is not a camera/recording API.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;
	use wasm_bindgen_futures::JsFuture;

	use super::*;

	fn navigator() -> Result<web_sys::Navigator, Error> {
		let window = web_sys::window().ok_or(Error::Unavailable)?;
		if !window.is_secure_context() {
			return Err(Error::PermissionDenied);
		}
		Ok(window.navigator())
	}
	fn failure(value: &JsValue) -> Error {
		if value.is_instance_of::<js_sys::TypeError>() {
			return Error::Unavailable;
		}
		match value
			.dyn_ref::<web_sys::DomException>()
			.map(web_sys::DomException::name)
			.as_deref()
		{
			Some("NotAllowedError" | "SecurityError") => Error::PermissionDenied,
			Some("NotFoundError" | "NotSupportedError") => Error::Unavailable,
			Some("NotReadableError" | "AbortError") => Error::Busy,
			_ => Error::Native,
		}
	}
	/// Queries without prompting. Browser support for camera/microphone queries
	/// varies; unsupported queries return unavailable, not an inferred grant.
	pub async fn status(permission: Permission) -> Result<Authorization, Error> {
		let navigator = navigator()?;
		let permissions = navigator.permissions().map_err(|error| failure(&error))?;
		let descriptor = js_sys::Object::new();
		js_sys::Reflect::set(
			&descriptor,
			&JsValue::from_str("name"),
			&JsValue::from_str(match permission {
				Permission::Camera => "camera",
				Permission::Microphone => "microphone",
			}),
		)
		.map_err(|_| Error::Native)?;
		let result = JsFuture::from(
			permissions
				.query(&descriptor)
				.map_err(|_| Error::Unavailable)?,
		)
		.await
		.map_err(|error| {
			if error.is_instance_of::<js_sys::TypeError>() {
				Error::Unavailable
			} else {
				failure(&error)
			}
		})?;
		let result = result
			.dyn_into::<web_sys::PermissionStatus>()
			.map_err(|_| Error::Native)?;
		let authorization = match result.state() {
			web_sys::PermissionState::Granted => {
				Authorization {
					status: Status::Granted,
					can_prompt: Some(false),
				}
			}
			web_sys::PermissionState::Denied => {
				Authorization {
					status: Status::Denied,
					can_prompt: Some(false),
				}
			}
			web_sys::PermissionState::Prompt => {
				Authorization {
					status: Status::NotDetermined,
					can_prompt: Some(true),
				}
			}
			_ => return Err(Error::Native),
		};
		Ok(authorization)
	}
	/// Requests access by opening then immediately stopping a media stream.
	/// Rejections propagate as errors; a denied query remains separately
	/// available.
	pub async fn request(permission: Permission) -> Result<Authorization, Error> {
		let devices = navigator()?
			.media_devices()
			.map_err(|error| failure(&error))?;
		let constraints = web_sys::MediaStreamConstraints::new();
		constraints.set_video(&JsValue::from_bool(permission == Permission::Camera));
		constraints.set_audio(&JsValue::from_bool(permission == Permission::Microphone));
		let capture = devices
			.get_user_media_with_constraints(&constraints)
			.map_err(|error| failure(&error))?;
		// The cleanup task outlives a dropped caller future: a late permission
		// grant must never leave a camera or microphone running.
		let cleanup = wasm_bindgen_futures::future_to_promise(async move {
			let stream = JsFuture::from(capture)
				.await?
				.dyn_into::<web_sys::MediaStream>()?;
			for track in stream.get_tracks().iter() {
				if let Some(track) = track.dyn_ref::<web_sys::MediaStreamTrack>() {
					track.stop();
				}
			}
			Ok(JsValue::UNDEFINED)
		});
		JsFuture::from(cleanup)
			.await
			.map_err(|error| failure(&error))?;
		Ok(Authorization {
			status: Status::Granted,
			can_prompt: Some(false),
		})
	}
}

/// Typed connection to a native host's permission handlers.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use leptoskit_bridge::guest::Client;

	use super::*;
	/// Resolves the installed plugin once, then uses numeric method IDs.
	#[derive(Debug, Clone, Copy)]
	pub struct Permissions(Client);
	impl Permissions {
		/// Connects to the granted plugin.
		pub fn new() -> Result<Self, Error> {
			Client::new(NAME).map(Self)
		}

		/// Reads authorization without prompting.
		pub async fn status(self, permission: Permission) -> Result<Authorization, Error> {
			Authorization::decode(&self.0.call(STATUS, &permission.encode())?.await?)
		}

		/// Explicitly asks the OS for access; call in response to user intent.
		pub async fn request(self, permission: Permission) -> Result<Authorization, Error> {
			Authorization::decode(&self.0.call(REQUEST, &permission.encode())?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn permission_input_is_exact_and_rejects_unknown_capabilities() {
		for permission in [Permission::Camera, Permission::Microphone] {
			assert_eq!(Permission::decode(&permission.encode()), Ok(permission));
		}
		for payload in [&[][..], &[0], &[2, 0, 0, 0], &[0, 0, 0, 0, 0]] {
			assert_eq!(Permission::decode(payload), Err(Error::InvalidArgument));
		}
	}
	#[test]
	fn authorization_preserves_unknown_denied_and_restricted_states() {
		for status in [
			Status::Unknown,
			Status::NotDetermined,
			Status::Denied,
			Status::Granted,
			Status::Restricted,
		] {
			for can_prompt in [None, Some(false), Some(true)] {
				let value = Authorization { status, can_prompt };
				assert_eq!(Authorization::decode(&value.encode()), Ok(value));
			}
		}
		for payload in [
			&[0; 7][..],
			&[0; 9],
			&[5, 0, 0, 0, 0, 0, 0, 0],
			&[0, 0, 0, 0, 3, 0, 0, 0],
		] {
			assert_eq!(Authorization::decode(payload), Err(Error::Native));
		}
	}
}
