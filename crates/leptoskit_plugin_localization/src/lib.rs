//! User locale and calendar preferences, sampled afresh for each request.
//! No global locale is mutated. Re-query after the application resumes to pick
//! up native settings changes; this crate does not yet publish change events.
//!
//! Binary v1: u32 version, u32 locale count, then each locale's tag/language/
//! region as length-prefixed UTF-8. Calendar identifier and timezone follow as
//! strings, then u32 24-hour flag (unknown/no/yes = 0/1/2) and u32 first
//! weekday (unknown = 0, Monday..Sunday = 1..7). Empty optional strings mean
//! unknown. Strings are bounded to 256 bytes and locale lists to 64 entries.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime plugin name.
pub const NAME: &str = "localization";
/// Empty request -> current [`Snapshot`].
pub const SNAPSHOT: MethodId = MethodId(0);

/// One platform-reported locale, in preference order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
	/// Platform language tag, usually BCP 47.
	pub tag: String,
	/// Language subtag, if the platform/parser exposes it.
	pub language: Option<String>,
	/// Region subtag, without guessing a region from language alone.
	pub region: Option<String>,
}

/// Current locale list and calendar preferences.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
	/// Preferred locales in platform order; may be empty when unavailable.
	pub locales: Vec<Locale>,
	/// Platform calendar identifier, normalized to `gregory` for Gregorian.
	pub calendar: Option<String>,
	/// Current timezone identifier from the platform's timezone database.
	pub time_zone: Option<String>,
	/// Preferred 24-hour clock, if exposed.
	pub hour_24: Option<bool>,
	/// First weekday, Monday=1 through Sunday=7, or unknown.
	pub first_weekday: Option<u8>,
}

fn write_text(output: &mut Encoder, value: Option<&str>) -> Result<(), Error> {
	if value == Some("") {
		return Err(Error::InvalidArgument);
	}
	let value = value.unwrap_or_default();
	if value.len() > 256 {
		return Err(Error::TooLarge);
	}
	output.bytes(value.as_bytes())
}
fn read_text(input: &mut Decoder<'_>) -> Result<Option<String>, Error> {
	let value = input.string()?;
	if value.len() > 256 {
		return Err(Error::TooLarge);
	}
	Ok((!value.is_empty()).then(|| value.to_owned()))
}
impl Snapshot {
	/// Encodes bounded preferences, rejecting empty known text or invalid
	/// weekdays.
	pub fn encode(&self) -> Result<Bytes, Error> {
		if self.locales.len() > 64 {
			return Err(Error::TooLarge);
		}
		if self
			.first_weekday
			.is_some_and(|day| !(1..=7).contains(&day))
		{
			return Err(Error::InvalidArgument);
		}
		let mut output = Encoder::with_capacity(128);
		output.u32(1);
		output.u32(u32::try_from(self.locales.len()).map_err(|_| Error::TooLarge)?);
		for locale in &self.locales {
			write_text(&mut output, Some(&locale.tag))?;
			write_text(&mut output, locale.language.as_deref())?;
			write_text(&mut output, locale.region.as_deref())?;
		}
		write_text(&mut output, self.calendar.as_deref())?;
		write_text(&mut output, self.time_zone.as_deref())?;
		output.u32(match self.hour_24 {
			None => 0,
			Some(false) => 1,
			Some(true) => 2,
		});
		output.u32(self.first_weekday.map_or(0, u32::from));
		Ok(output.finish())
	}

	/// Validates every field and rejects trailing bytes before returning data.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}
		let count = input.u32()?;
		if count > 64 {
			return Err(Error::TooLarge);
		}
		let mut locales = Vec::with_capacity(count as usize);
		for _ in 0..count {
			locales.push(Locale {
				tag: read_text(&mut input)?.ok_or(Error::InvalidArgument)?,
				language: read_text(&mut input)?,
				region: read_text(&mut input)?,
			});
		}
		let calendar = read_text(&mut input)?;
		let time_zone = read_text(&mut input)?;
		let hour_24 = match input.u32()? {
			0 => None,
			1 => Some(false),
			2 => Some(true),
			_ => return Err(Error::InvalidArgument),
		};
		let first_weekday = match input.u32()? {
			0 => None,
			value @ 1..=7 => Some(u8::try_from(value).map_err(|_| Error::InvalidArgument)?),
			_ => return Err(Error::InvalidArgument),
		};
		input.finish()?;
		Ok(Self {
			locales,
			calendar,
			time_zone,
			hour_24,
			first_weekday,
		})
	}
}

/// OS locale preferences and timezone. Calendar/clock preferences remain
/// unknown where these portable APIs do not expose them. Apple native hosts use
/// Foundation.
#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
pub mod desktop {
	use super::*;
	/// Reads fresh system locale preferences and timezone, without changing
	/// them.
	pub fn snapshot() -> Result<Snapshot, Error> {
		let locales = sys_locale::get_locales()
			.map(|tag| {
				let (language, region) = match tag.parse::<unic_langid::LanguageIdentifier>() {
					Ok(parsed) => {
						(
							Some(parsed.language.to_string()),
							parsed.region.map(|value| value.to_string()),
						)
					}
					Err(_) => (None, None), // Preserve non-BCP-47 platform values such as POSIX C.
				};
				Locale {
					tag,
					language,
					region,
				}
			})
			.collect();
		let result = Snapshot {
			locales,
			time_zone: Some(iana_time_zone::get_timezone().map_err(|_| Error::Native)?),
			..Snapshot::default()
		};
		Ok(result)
	}
	/// Rust desktop bridge adapter.
	#[derive(Debug, Default)]
	pub struct Localization;
	impl leptoskit_bridge::Plugin for Localization {
		fn name(&self) -> &'static str {
			NAME
		}

		fn methods(&self) -> &'static [MethodId] {
			&[SNAPSHOT]
		}

		fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
			if method != SNAPSHOT {
				return Err(Error::Unavailable);
			}
			if !payload.is_empty() {
				return Err(Error::InvalidArgument);
			}
			snapshot()?.encode()
		}
	}
}

/// Browser Navigator/Intl implementation.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

/// Cached native guest client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// A resolved native localization plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Localization(leptoskit_bridge::guest::Client);
	impl Localization {
		/// Resolves the plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads current preferences; no locale snapshot is cached by this
		/// client.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn preserves_preference_order_regions_and_unknown_values() {
		let snapshot = Snapshot {
			locales: vec![
				Locale {
					tag: "zh-Hant-TW".into(),
					language: Some("zh".into()),
					region: Some("TW".into()),
				},
				Locale {
					tag: "en".into(),
					language: Some("en".into()),
					region: None,
				},
			],
			calendar: Some("gregory".into()),
			time_zone: Some("Asia/Taipei".into()),
			hour_24: Some(true),
			first_weekday: Some(1),
		};
		assert_eq!(Snapshot::decode(&snapshot.encode().unwrap()), Ok(snapshot));
		let unknown = Snapshot::default();
		assert_eq!(Snapshot::decode(&unknown.encode().unwrap()), Ok(unknown));
	}
	#[test]
	fn rejects_truncated_trailing_unknown_version_flags_and_weekday() {
		let encoded = Snapshot::default().encode().unwrap();
		for length in 0..encoded.len() {
			assert_eq!(
				Snapshot::decode(&encoded[..length]),
				Err(Error::InvalidArgument)
			);
		}
		for (offset, value) in [(0, 2), (16, 3), (20, 8)] {
			let mut invalid = encoded.to_vec();
			invalid[offset] = value;
			assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
		}
		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert_eq!(Snapshot::decode(&trailing), Err(Error::InvalidArgument));
	}
	#[test]
	fn bounds_locale_count_text_and_weekdays() {
		let locale = Locale {
			tag: "en".into(),
			language: None,
			region: None,
		};
		assert_eq!(
			Snapshot {
				locales: vec![locale; 65],
				..Snapshot::default()
			}
			.encode(),
			Err(Error::TooLarge)
		);
		assert_eq!(
			Snapshot {
				calendar: Some("x".repeat(257)),
				..Snapshot::default()
			}
			.encode(),
			Err(Error::TooLarge)
		);
		assert_eq!(
			Snapshot {
				calendar: Some(String::new()),
				..Snapshot::default()
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Snapshot {
				first_weekday: Some(0),
				..Snapshot::default()
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
		let mut count = Snapshot::default().encode().unwrap().to_vec();
		count[4] = 65;
		assert_eq!(Snapshot::decode(&count), Err(Error::TooLarge));
		let mut text = Snapshot::default().encode().unwrap().to_vec();
		text.splice(8..12, 257_u32.to_le_bytes());
		text.splice(12..12, vec![b'x'; 257]);
		assert_eq!(Snapshot::decode(&text), Err(Error::TooLarge));
	}

	#[test]
	fn rejects_empty_tags_and_invalid_utf8() {
		let value = Snapshot {
			locales: vec![Locale {
				tag: "en".into(),
				language: Some("en".into()),
				region: None,
			}],
			..Snapshot::default()
		};
		let mut invalid = value.encode().unwrap().to_vec();
		invalid[12] = 255;
		assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
		let mut empty = value;
		empty.locales[0].tag.clear();
		assert_eq!(empty.encode(), Err(Error::InvalidArgument));
	}

	#[test]
	#[cfg(any(
		target_os = "macos",
		target_os = "windows",
		all(target_os = "linux", not(target_env = "ohos"))
	))]
	fn desktop_adapter_validates_requests_before_reading_preferences() {
		use leptoskit_bridge::Plugin;
		let mut host = desktop::Localization;
		assert_eq!(
			host.invoke(MethodId(99), Bytes::new()),
			Err(Error::Unavailable)
		);
		assert_eq!(
			host.invoke(SNAPSHOT, Bytes::from_static(&[1])),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	#[cfg(any(
		target_os = "macos",
		target_os = "windows",
		all(target_os = "linux", not(target_env = "ohos"))
	))]
	fn desktop_adapter_reads_current_preferences() {
		use leptoskit_bridge::Plugin;
		let mut host = desktop::Localization;
		assert_eq!(host.name(), NAME);
		assert_eq!(host.methods(), &[SNAPSHOT]);
		assert_eq!(
			host.invoke(SNAPSHOT, Bytes::from_static(&[0])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			host.invoke(MethodId(9), Bytes::new()),
			Err(Error::Unavailable)
		);
		// A host without a configured zone (some minimal CI images) reports a
		// native failure rather than an invented zone; every other host must
		// return a well-formed snapshot.
		match host.invoke(SNAPSHOT, Bytes::new()) {
			Ok(reply) => {
				let result = Snapshot::decode(&reply).unwrap();
				assert!(result.time_zone.is_some_and(|zone| !zone.is_empty()));
				assert_eq!(result.hour_24, None);
				for locale in &result.locales {
					assert_ne!(locale.tag, "");
				}
			}
			Err(error) => {
				assert_eq!(error, Error::Native);
				assert!(iana_time_zone::get_timezone().is_err());
			}
		}
	}
}
