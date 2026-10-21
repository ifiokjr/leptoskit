//! Text-to-speech with one active utterance per plugin. Overlap returns `Busy`;
//! successful completion means the engine finished, not merely queued text.
//! Dropping a guest call cancels its host job; `stop` cancels active speech.
//!
//! Speak wire v1: u32 version, rate/pitch/volume in thousandths, then bounded
//! length-prefixed UTF-8 text, language and voice ID. Voice replies: u32
//! version, count, then ID/name/language strings, u32 quality and local-service
//! flag. All integers are little-endian; empty optional strings mean absent.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime name.
pub const NAME: &str = "speech";
/// Empty request -> encoded voice list.
pub const VOICES: MethodId = MethodId(0);
/// Encoded options -> empty completion when speech finishes.
pub const SPEAK: MethodId = MethodId(1);
/// Empty request -> stop owned speech and return empty reply.
pub const STOP: MethodId = MethodId(2);
/// Empty request -> u32 boolean for owned active speech.
pub const IS_SPEAKING: MethodId = MethodId(3);

/// A validated portable speech request. Rate/pitch use thousandths (1000
/// normal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utterance {
	text: String,
	rate: u32,
	pitch: u32,
	volume: u32,
	language: Option<String>,
	voice: Option<String>,
}
impl Utterance {
	/// Uses default voice, normal rate/pitch, and full volume. Text must
	/// contain 1..=4096 UTF-8 bytes and at most 4000 UTF-16 code units.
	pub fn new(text: impl Into<String>) -> Result<Self, Error> {
		let text = text.into();
		if text.is_empty() {
			return Err(Error::InvalidArgument);
		}
		if text.len() > 4096 || text.encode_utf16().count() > 4000 {
			return Err(Error::TooLarge);
		}
		Ok(Self {
			text,
			rate: 1000,
			pitch: 1000,
			volume: 1000,
			language: None,
			voice: None,
		})
	}

	/// Sets rate and pitch in 500..=2000 and volume in 0..=1000. Rate is
	/// relative to each engine's normal rate, so durations are not
	/// identical across OSes.
	pub fn with_audio(mut self, rate: u32, pitch: u32, volume: u32) -> Result<Self, Error> {
		if !(500..=2000).contains(&rate) || !(500..=2000).contains(&pitch) || volume > 1000 {
			return Err(Error::InvalidArgument);
		}
		self.rate = rate;
		self.pitch = pitch;
		self.volume = volume;
		Ok(self)
	}

	/// Selects language[-Script][-Region], e.g. en, en-US or zh-Hant-TW. A
	/// voice identifier cannot be combined with language selection.
	pub fn with_language(mut self, language: impl Into<String>) -> Result<Self, Error> {
		let language = language.into();
		if self.voice.is_some() || !valid_language(&language) {
			return Err(Error::InvalidArgument);
		}
		self.language = Some(language);
		Ok(self)
	}

	/// Selects an identifier returned by voice discovery.
	pub fn with_voice(mut self, voice: impl Into<String>) -> Result<Self, Error> {
		let voice = voice.into();
		if self.language.is_some() || voice.is_empty() {
			return Err(Error::InvalidArgument);
		}
		if voice.len() > 512 {
			return Err(Error::TooLarge);
		}
		self.voice = Some(voice);
		Ok(self)
	}

	/// Text to synthesize.
	pub fn text(&self) -> &str {
		&self.text
	}

	/// Relative engine rate in thousandths.
	pub fn rate(&self) -> u32 {
		self.rate
	}

	/// Relative pitch in thousandths.
	pub fn pitch(&self) -> u32 {
		self.pitch
	}

	/// Output volume in thousandths; zero is muted.
	pub fn volume(&self) -> u32 {
		self.volume
	}

	/// Optional language selection.
	pub fn language(&self) -> Option<&str> {
		self.language.as_deref()
	}

	/// Optional discovered voice ID.
	pub fn voice(&self) -> Option<&str> {
		self.voice.as_deref()
	}

	/// Encodes validated fields.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(self.text.len() + 32);
		for field in [1, self.rate, self.pitch, self.volume] {
			output.u32(field);
		}
		for value in [
			&self.text,
			self.language.as_deref().unwrap_or_default(),
			self.voice.as_deref().unwrap_or_default(),
		] {
			output.bytes(value.as_bytes())?;
		}
		Ok(output.finish())
	}

	/// Decodes through the same constructors as normal callers, rejecting
	/// trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}
		let (rate, pitch, volume) = (input.u32()?, input.u32()?, input.u32()?);
		let text = input.string()?;
		if text.len() > 4096 {
			return Err(Error::TooLarge);
		}
		let mut result = Self::new(text)?.with_audio(rate, pitch, volume)?;
		let language = input.string()?;
		if language.len() > 64 {
			return Err(Error::TooLarge);
		}
		if !language.is_empty() {
			result = result.with_language(language)?;
		}
		let voice = input.string()?;
		if voice.len() > 512 {
			return Err(Error::TooLarge);
		}
		if !voice.is_empty() {
			result = result.with_voice(voice)?;
		}
		input.finish()?;
		Ok(result)
	}
}
fn valid_language(value: &str) -> bool {
	let mut parts = value.split('-');
	let Some(language) = parts.next() else {
		return false;
	};
	if !(2..=3).contains(&language.len())
		|| !language.bytes().all(|byte| byte.is_ascii_alphabetic())
	{
		return false;
	}
	let mut next = parts.next();
	if next.is_some_and(|script| {
		script.len() == 4 && script.bytes().all(|byte| byte.is_ascii_alphabetic())
	}) {
		next = parts.next();
	}
	if let Some(region) = next {
		if !((region.len() == 2 && region.bytes().all(|byte| byte.is_ascii_alphabetic()))
			|| (region.len() == 3 && region.bytes().all(|byte| byte.is_ascii_digit())))
		{
			return false;
		}
	}
	parts.next().is_none()
}

/// Platform voice quality, when exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Quality {
	/// Not exposed by the platform.
	Unknown = 0,
	/// Standard voice.
	Default = 1,
	/// Enhanced or premium voice.
	Enhanced = 2,
}
/// One engine-provided voice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voice {
	/// Stable engine voice identifier used for selection, not a device ID.
	pub id: String,
	/// Display name.
	pub name: String,
	/// Engine's language tag.
	pub language: String,
	/// Platform quality classification.
	pub quality: Quality,
	/// Whether synthesis runs locally, if the platform exposes this.
	pub local: Option<bool>,
}
/// Decodes a bounded voice list (at most 1024 voices, 512-byte text fields).
pub fn decode_voices(bytes: &[u8]) -> Result<Vec<Voice>, Error> {
	let mut input = Decoder::new(bytes);
	if input.u32()? != 1 {
		return Err(Error::InvalidArgument);
	}
	let count = input.u32()?;
	if count > 1024 {
		return Err(Error::TooLarge);
	}
	let mut result = Vec::with_capacity(count as usize);
	for _ in 0..count {
		let mut text = || -> Result<String, Error> {
			let value = input.string()?;
			if value.is_empty() {
				return Err(Error::InvalidArgument);
			}
			if value.len() > 512 {
				return Err(Error::TooLarge);
			}
			Ok(value.to_owned())
		};
		let (id, name, language) = (text()?, text()?, text()?);
		let quality = match input.u32()? {
			0 => Quality::Unknown,
			1 => Quality::Default,
			2 => Quality::Enhanced,
			_ => return Err(Error::InvalidArgument),
		};
		let local = match input.u32()? {
			0 => None,
			1 => Some(false),
			2 => Some(true),
			_ => return Err(Error::InvalidArgument),
		};
		result.push(Voice {
			id,
			name,
			language,
			quality,
			local,
		});
	}
	input.finish()?;
	Ok(result)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

/// Cached native guest client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// One resolved speech plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Speech(leptoskit_bridge::guest::Client);
	impl Speech {
		/// Resolves the installed plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Discovers currently available engine voices.
		pub async fn voices(self) -> Result<Vec<Voice>, Error> {
			decode_voices(&self.0.call(VOICES, &[])?.await?)
		}

		/// Completes at utterance end. Overlap fails with Busy;
		/// stop/cancellation fails the active speak call with
		/// Cancelled.
		pub async fn speak(self, utterance: &Utterance) -> Result<(), Error> {
			empty(&self.0.call(SPEAK, &utterance.encode()?)?.await?)
		}

		/// Stops this plugin's active utterance.
		pub async fn stop(self) -> Result<(), Error> {
			empty(&self.0.call(STOP, &[])?.await?)
		}

		/// Whether this plugin has an active utterance.
		pub async fn is_speaking(self) -> Result<bool, Error> {
			let bytes = self.0.call(IS_SPEAKING, &[])?.await?;
			let mut input = Decoder::new(&bytes);
			let result = match input.u32()? {
				0 => false,
				1 => true,
				_ => return Err(Error::InvalidArgument),
			};
			input.finish()?;
			Ok(result)
		}
	}
	fn empty(bytes: &[u8]) -> Result<(), Error> {
		Decoder::new(bytes).finish()
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn options_round_trip_and_portable_language_validation() {
		let options = Utterance::new("你好")
			.unwrap()
			.with_audio(750, 1200, 0)
			.unwrap()
			.with_language("zh-Hant-TW")
			.unwrap();
		assert_eq!(Utterance::decode(&options.encode().unwrap()), Ok(options));
		for language in ["en", "en-US", "es-419", "zh-Hant", "fil-PH"] {
			assert!(valid_language(language));
		}
		for language in [
			"",
			"e",
			"en_Us",
			"en--US",
			"en-US-extra",
			"zh-1234",
			"1n-US",
		] {
			assert!(!valid_language(language));
		}
	}
	#[test]
	fn reject_bad_options_truncation_and_trailing_bytes() {
		assert_eq!(Utterance::new(""), Err(Error::InvalidArgument));
		assert_eq!(Utterance::new("x".repeat(4001)), Err(Error::TooLarge));
		assert!(
			Utterance::new("text")
				.unwrap()
				.with_audio(499, 1000, 1000)
				.is_err()
		);
		assert!(
			Utterance::new("text")
				.unwrap()
				.with_audio(1000, 2001, 1000)
				.is_err()
		);
		assert!(
			Utterance::new("text")
				.unwrap()
				.with_audio(1000, 1000, 1001)
				.is_err()
		);
		assert!(
			Utterance::new("text")
				.unwrap()
				.with_language("en")
				.unwrap()
				.with_voice("voice")
				.is_err()
		);
		let encoded = Utterance::new("text").unwrap().encode().unwrap();
		for length in 0..encoded.len() {
			assert!(Utterance::decode(&encoded[..length]).is_err());
		}
		let mut invalid = encoded.to_vec();
		invalid[20] = 255;
		assert_eq!(Utterance::decode(&invalid), Err(Error::InvalidArgument));
		let mut invalid = encoded.to_vec();
		invalid.push(0);
		assert_eq!(Utterance::decode(&invalid), Err(Error::InvalidArgument));
	}
	#[test]
	fn voice_decoder_validates_bounds_flags_and_utf8() {
		let mut output = Encoder::with_capacity(64);
		output.u32(1);
		output.u32(1);
		for text in ["voice", "Voice", "en-US"] {
			output.bytes(text.as_bytes()).unwrap();
		}
		output.u32(2);
		output.u32(2);
		let encoded = output.finish();
		assert_eq!(
			decode_voices(&encoded).unwrap()[0].quality,
			Quality::Enhanced
		);
		for length in 0..encoded.len() {
			assert!(decode_voices(&encoded[..length]).is_err());
		}
		let mut invalid = encoded.to_vec();
		invalid[4..8].copy_from_slice(&1025_u32.to_le_bytes());
		assert_eq!(decode_voices(&invalid), Err(Error::TooLarge));
		let mut invalid = encoded.to_vec();
		let offset = invalid.len() - 4;
		invalid[offset] = 3;
		assert_eq!(decode_voices(&invalid), Err(Error::InvalidArgument));
		let mut invalid = encoded.to_vec();
		invalid[12] = 255;
		assert_eq!(decode_voices(&invalid), Err(Error::InvalidArgument));
	}

	#[test]
	fn accessors_voice_selection_and_decoded_limits() {
		let utterance = Utterance::new("hello")
			.unwrap()
			.with_audio(900, 1100, 250)
			.unwrap()
			.with_voice("com.example.voice")
			.unwrap();
		assert_eq!(utterance.text(), "hello");
		assert_eq!(
			(utterance.rate(), utterance.pitch(), utterance.volume()),
			(900, 1100, 250)
		);
		assert_eq!(utterance.voice(), Some("com.example.voice"));
		assert_eq!(utterance.language(), None);
		assert_eq!(
			Utterance::decode(&utterance.encode().unwrap()),
			Ok(utterance.clone())
		);
		// A voice cannot be combined with language selection in either order.
		assert_eq!(
			utterance.clone().with_language("en"),
			Err(Error::InvalidArgument)
		);
		let base = Utterance::new("hello").unwrap();
		assert_eq!(base.clone().with_voice(""), Err(Error::InvalidArgument));
		assert_eq!(
			base.clone().with_voice("v".repeat(513)),
			Err(Error::TooLarge)
		);
		assert_eq!(
			base.clone().with_language("not a tag"),
			Err(Error::InvalidArgument)
		);

		let wire = |text: &str, language: &str, voice: &str| {
			let mut output = Encoder::with_capacity(64);
			for field in [1, 1000, 1000, 1000] {
				output.u32(field);
			}
			for value in [text, language, voice] {
				output.bytes(value.as_bytes()).unwrap();
			}
			output.finish()
		};
		assert_eq!(
			Utterance::decode(&wire(&"x".repeat(4097), "", "")),
			Err(Error::TooLarge)
		);
		assert_eq!(
			Utterance::decode(&wire("x", &"e".repeat(65), "")),
			Err(Error::TooLarge)
		);
		assert_eq!(
			Utterance::decode(&wire("x", "", &"v".repeat(513))),
			Err(Error::TooLarge)
		);
		let mut version = wire("x", "", "").to_vec();
		version[0] = 2;
		assert_eq!(Utterance::decode(&version), Err(Error::InvalidArgument));
	}

	#[test]
	fn voice_lists_reject_empty_oversized_and_unknown_flags() {
		let voices = |quality: u32, local: u32, id: &str| {
			let mut output = Encoder::with_capacity(64);
			output.u32(1);
			output.u32(1);
			for text in [id, "Name", "en-US"] {
				output.bytes(text.as_bytes()).unwrap();
			}
			output.u32(quality);
			output.u32(local);
			output.finish()
		};
		let decoded = decode_voices(&voices(0, 0, "a")).unwrap();
		assert_eq!(
			(decoded[0].quality, decoded[0].local),
			(Quality::Unknown, None)
		);
		let decoded = decode_voices(&voices(1, 1, "a")).unwrap();
		assert_eq!(
			(decoded[0].quality, decoded[0].local),
			(Quality::Default, Some(false))
		);
		assert_eq!(
			decode_voices(&voices(1, 3, "a")),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			decode_voices(&voices(1, 1, "")),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			decode_voices(&voices(1, 1, &"v".repeat(513))),
			Err(Error::TooLarge)
		);
		let mut version = voices(1, 1, "a").to_vec();
		version[0] = 2;
		assert_eq!(decode_voices(&version), Err(Error::InvalidArgument));
	}
}
