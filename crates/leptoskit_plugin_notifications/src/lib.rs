//! Immediate local notifications. Successful posting means the OS accepted a
//! request, not that the user saw it. Focus modes, notification settings and
//! foreground presentation policy can suppress visible delivery.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Stable native registration name.
pub const NAME: &str = "notifications";
/// Empty payload; four-byte permission state.
pub const PERMISSION: MethodId = MethodId(0);
/// Empty payload; requests native permission and returns its current state.
pub const REQUEST_PERMISSION: MethodId = MethodId(1);
/// Four length-prefixed strings: identifier, title, body, Android channel ID.
pub const SHOW: MethodId = MethodId(2);
/// One length-prefixed identifier; removes pending and delivered notifications.
pub const CANCEL: MethodId = MethodId(3);
/// Android only: identifier, name, description, importance u32, sound boolean
/// u32.
pub const CONFIGURE_CHANNEL: MethodId = MethodId(4);
/// Android only: one length-prefixed channel identifier.
pub const DELETE_CHANNEL: MethodId = MethodId(5);
/// Empty payload; dismisses only notifications owned by this plugin namespace.
pub const CANCEL_ALL: MethodId = MethodId(6);
/// Exported independently grantable operations.
pub const METHODS: &[MethodId] = &[
	PERMISSION,
	REQUEST_PERMISSION,
	SHOW,
	CANCEL,
	CONFIGURE_CHANNEL,
	DELETE_CHANNEL,
	CANCEL_ALL,
];

/// Current OS notification authorization, without guessing prompt history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Permission {
	/// Android cannot distinguish an initial denial from a non-promptable
	/// denial.
	Unknown = 0,
	/// The platform explicitly reports that permission has not been requested.
	NotDetermined = 1,
	/// Notifications are disabled or authorization was denied.
	Denied = 2,
	/// Authorized, subject to per-channel settings and OS delivery policy.
	Granted = 3,
	/// Apple provisional authorization, typically quiet delivery.
	Provisional = 4,
	/// Apple temporary App Clip authorization.
	Ephemeral = 5,
}
impl Permission {
	/// Validates the exact four-byte response.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match u32::from_le_bytes(bytes.try_into().map_err(|_| Error::InvalidArgument)?) {
			0 => Ok(Self::Unknown),
			1 => Ok(Self::NotDetermined),
			2 => Ok(Self::Denied),
			3 => Ok(Self::Granted),
			4 => Ok(Self::Provisional),
			5 => Ok(Self::Ephemeral),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Stable application-chosen notification or channel identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identifier(String);
impl Identifier {
	/// Accepts 1..=64 ASCII letters, digits, hyphens, periods or underscores.
	pub fn new(value: impl Into<String>) -> Result<Self, Error> {
		let value = value.into();
		validate_id(&value)?;
		Ok(Self(value))
	}

	/// Borrows the identifier.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}

	/// Encodes a cancellation/channel deletion payload.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(4 + self.0.len());
		output.bytes(self.0.as_bytes())?;
		Ok(output.finish())
	}
}

fn validate_id(value: &str) -> Result<(), Error> {
	if value.is_empty()
		|| value.len() > 64
		|| !value
			.bytes()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
	{
		Err(Error::InvalidArgument)
	} else {
		Ok(())
	}
}

fn validate_text(
	value: &str,
	maximum: usize,
	multiline: bool,
	required: bool,
) -> Result<(), Error> {
	if value.len() > maximum
		|| (required && value.trim().is_empty())
		|| value
			.chars()
			.any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\t')))
	{
		Err(Error::InvalidArgument)
	} else {
		Ok(())
	}
}

/// Validated, plain-text local notification. A repeated ID replaces that app's
/// prior notification where supported; Android requires a configured channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
	id: Identifier,
	title: String,
	body: String,
	channel: Identifier,
}
impl Notification {
	/// Constructs bounded content, rejecting NUL/control injection.
	pub fn new(
		id: Identifier,
		title: impl Into<String>,
		body: impl Into<String>,
		channel: Identifier,
	) -> Result<Self, Error> {
		let title = title.into();
		let body = body.into();
		validate_text(&title, 256, false, true)?;
		validate_text(&body, 4096, true, false)?;
		Ok(Self {
			id,
			title,
			body,
			channel,
		})
	}

	/// Application-chosen replacement/cancellation ID.
	#[must_use]
	pub fn id(&self) -> &Identifier {
		&self.id
	}

	/// Plain-text heading.
	#[must_use]
	pub fn title(&self) -> &str {
		&self.title
	}

	/// Plain-text body.
	#[must_use]
	pub fn body(&self) -> &str {
		&self.body
	}

	/// Android channel identifier; other platforms do not use channel settings.
	#[must_use]
	pub fn channel(&self) -> &Identifier {
		&self.channel
	}

	/// Encodes content without a JSON intermediary.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(
			16 + self.id.0.len() + self.title.len() + self.body.len() + self.channel.0.len(),
		);
		for value in [
			self.id.as_str(),
			&self.title,
			&self.body,
			self.channel.as_str(),
		] {
			output.bytes(value.as_bytes())?;
		}
		Ok(output.finish())
	}

	/// Validates the complete native control message, including trailing bytes.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > 4496 {
			return Err(Error::InvalidArgument);
		}
		let mut input = Decoder::new(payload);
		let result = Self::new(
			Identifier::new(input.string()?)?,
			input.string()?,
			input.string()?,
			Identifier::new(input.string()?)?,
		)?;
		input.finish()?;
		Ok(result)
	}
}

/// Android channel importance. OS/user settings can override the initial value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Importance {
	/// Quiet notification without interrupting the user.
	Low = 1,
	/// Standard notification importance.
	Default = 2,
	/// High importance; visual interruption remains OS-controlled.
	High = 3,
}

/// Initial Android channel configuration. Existing channel importance/sound
/// are user-owned and cannot be changed by recreating the same channel ID.
#[derive(Debug, Clone)]
pub struct Channel {
	id: Identifier,
	name: String,
	description: String,
	importance: Importance,
	sound: bool,
}
impl Channel {
	/// Validates channel display metadata and explicit sound preference.
	pub fn new(
		id: Identifier,
		name: impl Into<String>,
		description: impl Into<String>,
		importance: Importance,
		sound: bool,
	) -> Result<Self, Error> {
		let name = name.into();
		let description = description.into();
		validate_text(&name, 128, false, true)?;
		validate_text(&description, 1024, true, false)?;
		Ok(Self {
			id,
			name,
			description,
			importance,
			sound,
		})
	}

	/// Encodes the Android-only channel operation.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output =
			Encoder::with_capacity(20 + self.id.0.len() + self.name.len() + self.description.len());
		for value in [self.id.as_str(), &self.name, &self.description] {
			output.bytes(value.as_bytes())?;
		}
		output.u32(self.importance as u32);
		output.u32(u32::from(self.sound));
		Ok(output.finish())
	}

	/// Validates an Android channel wire message.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > 1236 {
			return Err(Error::InvalidArgument);
		}
		let mut input = Decoder::new(payload);
		let id = Identifier::new(input.string()?)?;
		let name = input.string()?;
		let description = input.string()?;
		let importance = match input.u32()? {
			1 => Importance::Low,
			2 => Importance::Default,
			3 => Importance::High,
			_ => return Err(Error::InvalidArgument),
		};
		let sound = match input.u32()? {
			0 => false,
			1 => true,
			_ => return Err(Error::InvalidArgument),
		};
		input.finish()?;
		Self::new(id, name, description, importance, sound)
	}
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn bounded_messages_roundtrip_unicode_and_reject_ambiguous_input() {
		let note = Notification::new(
			Identifier::new("job-1").unwrap(),
			"Finished ✓",
			"Two\nlines",
			Identifier::new("general").unwrap(),
		)
		.unwrap();
		let bytes = note.encode().unwrap();
		assert_eq!(Notification::decode(&bytes).unwrap(), note);
		for length in 0..bytes.len() {
			assert!(Notification::decode(&bytes[..length]).is_err());
		}
		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert!(Notification::decode(&trailing).is_err());
		for id in ["", "../a/b", "bad\0id", "é", &"a".repeat(65)] {
			assert!(Identifier::new(id).is_err());
		}
		assert!(Notification::new(note.id.clone(), "\t", "", note.channel.clone()).is_err());
		assert!(Notification::new(note.id, "Valid", "bad\0body", note.channel).is_err());
	}
	#[test]
	fn channel_enums_and_permissions_are_strict() {
		let channel = Channel::new(
			Identifier::new("messages").unwrap(),
			"Messages",
			"",
			Importance::Low,
			false,
		)
		.unwrap();
		let mut bytes = channel.encode().unwrap().to_vec();
		assert!(Channel::decode(&bytes).is_ok());
		let last = bytes.len() - 4;
		bytes[last] = 2;
		assert!(Channel::decode(&bytes).is_err());
		assert_eq!(
			Permission::decode(&[4, 0, 0, 0]),
			Ok(Permission::Provisional)
		);
		for value in [&[][..], &[3], &[6, 0, 0, 0], &[3, 0, 0, 0, 0]] {
			assert!(Permission::decode(value).is_err());
		}
	}
	#[test]
	fn every_permission_state_decodes_exactly() {
		let expected = [
			Permission::Unknown,
			Permission::NotDetermined,
			Permission::Denied,
			Permission::Granted,
			Permission::Provisional,
			Permission::Ephemeral,
		];
		for (value, permission) in (0u32..).zip(expected) {
			assert_eq!(Permission::decode(&value.to_le_bytes()), Ok(permission));
		}
	}
	#[test]
	fn identifiers_and_notification_accessors_preserve_content() {
		let id = Identifier::new("backup.done_2").unwrap();
		assert_eq!(id.as_str(), "backup.done_2");
		let mut expected = Encoder::default();
		expected.bytes(b"backup.done_2").unwrap();
		assert_eq!(id.encode().unwrap(), expected.finish());
		let channel = Identifier::new("general").unwrap();
		let note = Notification::new(id.clone(), "Backup", "Done\tok", channel.clone()).unwrap();
		assert_eq!(note.id(), &id);
		assert_eq!(note.title(), "Backup");
		assert_eq!(note.body(), "Done\tok");
		assert_eq!(note.channel(), &channel);
		assert!(Notification::new(id.clone(), "x".repeat(257), "", channel.clone()).is_err());
		assert!(Notification::new(id, "Title\nInjected", "", channel).is_err());
	}
	#[test]
	fn notification_decode_rejects_oversized_and_invalid_channel_messages() {
		assert_eq!(
			Notification::decode(&[0; 4497]),
			Err(Error::InvalidArgument)
		);
		let mut bad_channel = Encoder::default();
		for value in ["job", "Title", "", "no/slash"] {
			bad_channel.bytes(value.as_bytes()).unwrap();
		}
		assert_eq!(
			Notification::decode(&bad_channel.finish()),
			Err(Error::InvalidArgument)
		);
	}
	#[test]
	fn channel_decode_accepts_every_importance_and_sound_preference() {
		for (importance, sound) in [
			(Importance::Low, true),
			(Importance::Default, false),
			(Importance::High, true),
		] {
			let channel = Channel::new(
				Identifier::new("alerts").unwrap(),
				"Alerts",
				"Line one\nline two",
				importance,
				sound,
			)
			.unwrap();
			let encoded = channel.encode().unwrap();
			let decoded = Channel::decode(&encoded).unwrap();
			assert_eq!(decoded.importance, importance);
			assert_eq!(decoded.sound, sound);
			assert_eq!(decoded.encode().unwrap(), encoded);
			let mut bad_importance = encoded.to_vec();
			let offset = bad_importance.len() - 8;
			bad_importance[offset] = 4;
			assert_eq!(
				Channel::decode(&bad_importance).unwrap_err(),
				Error::InvalidArgument
			);
		}
		assert_eq!(
			Channel::decode(&[0; 1237]).unwrap_err(),
			Error::InvalidArgument
		);
		assert!(
			Channel::new(
				Identifier::new("alerts").unwrap(),
				" ",
				"",
				Importance::Low,
				false
			)
			.is_err()
		);
	}
}
