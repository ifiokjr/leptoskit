//! Language tags, and the places an app reads them from.

use std::fmt;
use std::str::FromStr;

use unic_langid::LanguageIdentifier;

/// A language tag: a language, and optionally a script, a region, and
/// variants, written the way BCP 47 writes them (`en`, `pt-BR`,
/// `zh-Hant-TW`, `ca-ES-valencia`).
///
/// Tags compare case-insensitively and print in canonical form, so `pt-br`
/// and `PT-BR` are the same locale and both print as `pt-BR`.
///
/// ```
/// use leptoskit_l10n::Locale;
///
/// let locale = Locale::new("pt-br")?;
/// assert_eq!(locale.to_string(), "pt-BR");
/// assert_eq!(locale.language(), "pt");
/// assert_eq!(locale.script(), None);
/// assert_eq!(locale.region(), Some("BR"));
/// assert_eq!(locale, "PT-BR");
/// assert!(Locale::new("not a tag").is_err());
/// # Ok::<(), leptoskit_l10n::LocaleError>(())
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Locale(LanguageIdentifier);

impl Locale {
	/// Parses a tag.
	///
	/// # Errors
	///
	/// Returns [`LocaleError`] when `tag` is not a language tag.
	pub fn new(tag: &str) -> Result<Self, LocaleError> {
		tag.parse()
	}

	/// The language subtag, lowercase: `pt` for `pt-BR`.
	#[must_use]
	pub fn language(&self) -> &str {
		self.0.language.as_str()
	}

	/// The script subtag, when the tag has one: `Hant` for `zh-Hant-TW`.
	#[must_use]
	pub fn script(&self) -> Option<&str> {
		self.0
			.script
			.as_ref()
			.map(unic_langid::subtags::Script::as_str)
	}

	/// The region subtag, when the tag has one: `BR` for `pt-BR`.
	#[must_use]
	pub fn region(&self) -> Option<&str> {
		self.0
			.region
			.as_ref()
			.map(unic_langid::subtags::Region::as_str)
	}

	pub(crate) fn identifier(&self) -> &LanguageIdentifier {
		&self.0
	}
}

impl FromStr for Locale {
	type Err = LocaleError;

	fn from_str(tag: &str) -> Result<Self, LocaleError> {
		tag.parse::<LanguageIdentifier>()
			.map(Self)
			.map_err(|source| {
				LocaleError {
					tag: tag.to_owned(),
					reason: source.to_string(),
				}
			})
	}
}

impl fmt::Display for Locale {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt::Display::fmt(&self.0, formatter)
	}
}

impl fmt::Debug for Locale {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_tuple("Locale")
			.field(&self.to_string())
			.finish()
	}
}

impl PartialEq<str> for Locale {
	fn eq(&self, other: &str) -> bool {
		other
			.parse::<LanguageIdentifier>()
			.is_ok_and(|other| self.0 == other)
	}
}

impl PartialEq<&str> for Locale {
	fn eq(&self, other: &&str) -> bool {
		*self == **other
	}
}

/// A string that is not a language tag.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{tag}` is not a language tag: {reason}")]
pub struct LocaleError {
	/// The string that failed to parse.
	pub tag: String,
	/// What is wrong with it.
	pub reason: String,
}

/// The locales an `Accept-Language` header asks for, best first.
///
/// Entries are ordered by their quality value, with ties kept in the order
/// the header lists them. Wildcards, entries with `q=0`, duplicates, and
/// anything that is not a language tag are left out, and so is an entry
/// whose quality value is not one RFC 9110 allows: a number from `0` to `1`
/// with at most three decimals, so `q=5`, `q=-1`, and `q=NaN` drop their
/// entry rather than rank it.
///
/// ```
/// use leptoskit_l10n::accept_language;
///
/// let preferred = accept_language("fr-CH;q=0.8, fr;q=0.9, en-US, *;q=0.5, en;q=0, de;q=2");
/// let tags: Vec<String> = preferred.iter().map(ToString::to_string).collect();
/// assert_eq!(tags, ["en-US", "fr", "fr-CH"]);
/// ```
#[must_use]
pub fn accept_language(header: &str) -> Vec<Locale> {
	let mut entries: Vec<(u16, Locale)> = Vec::new();

	for entry in header.split(',') {
		let mut parts = entry.split(';').map(str::trim);
		let Some(tag) = parts.next().filter(|tag| !tag.is_empty() && *tag != "*") else {
			continue;
		};
		let Some(quality) = parts.try_fold(1000, |quality, parameter| {
			match parameter.split_once('=') {
				Some((name, value)) if name.trim().eq_ignore_ascii_case("q") => {
					thousandths(value.trim())
				}

				_ => Some(quality),
			}
		}) else {
			continue;
		};

		if quality == 0 {
			continue;
		}

		let Ok(locale) = Locale::new(tag) else {
			continue;
		};

		if entries.iter().any(|(_, seen)| *seen == locale) {
			continue;
		}

		entries.push((quality, locale));
	}

	entries.sort_by_key(|(quality, _)| std::cmp::Reverse(*quality));
	entries.into_iter().map(|(_, locale)| locale).collect()
}

/// A quality value in thousandths, when it is written the way RFC 9110's
/// `qvalue` is: `0` with up to three decimals, or `1` with up to three
/// zeros.
fn thousandths(value: &str) -> Option<u16> {
	let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));

	if fraction.len() > 3 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
		return None;
	}

	let decimals = format!("{fraction:0<3}").parse::<u16>().ok()?;

	match whole {
		"0" => Some(decimals),
		"1" if decimals == 0 => Some(1000),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_and_canonicalizes_tags() {
		let locale = Locale::new("ZH-hant-tw").unwrap();
		assert_eq!(locale.to_string(), "zh-Hant-TW");
		assert_eq!(locale.language(), "zh");
		assert_eq!(locale.script(), Some("Hant"));
		assert_eq!(locale.region(), Some("TW"));
		assert_eq!(locale, Locale::new("zh-Hant-TW").unwrap());
		assert_eq!(locale, "zh-hant-tw");
		assert_ne!(locale, "zh-TW");

		assert_ne!(locale, "not a tag");
		assert_eq!(format!("{locale:?}"), "Locale(\"zh-Hant-TW\")");
		assert_eq!("en".parse::<Locale>().unwrap().to_string(), "en");
	}

	#[test]
	fn rejects_strings_that_are_not_tags() {
		for tag in ["", "not a tag", "en_US!", "toolonglanguage"] {
			let error = Locale::new(tag).unwrap_err();
			assert_eq!(error.tag, tag);
			assert!(
				error
					.to_string()
					.starts_with(&format!("`{tag}` is not a language tag: "))
			);
			assert!(!error.reason.is_empty(), "{tag} needs a reason");
		}
	}

	#[test]
	fn accept_language_orders_by_quality_and_drops_what_it_cannot_use() {
		let preferred =
			accept_language("da, en-gb;q=0.8, en;q=0.7, *;q=0.1, en-GB;q=0.9, ??;q=0.9");
		let tags: Vec<String> = preferred.iter().map(ToString::to_string).collect();
		assert_eq!(tags, ["da", "en-GB", "en"]);
		assert_eq!(accept_language(""), [] as [Locale; 0]);
		assert_eq!(accept_language("*"), [] as [Locale; 0]);
		assert_eq!(accept_language("en;q=0, fr;q=-1"), [] as [Locale; 0]);
		assert_eq!(accept_language("en;q=abc"), [] as [Locale; 0]);

		let tags: Vec<String> = accept_language(" fr ;  Q = 0.5 ,de;level=1")
			.iter()
			.map(ToString::to_string)
			.collect();
		assert_eq!(tags, ["de", "fr"]);
	}

	#[test]
	fn accept_language_drops_entries_whose_quality_rfc_9110_does_not_allow() {
		let tags = |header: &str| -> Vec<String> {
			accept_language(header)
				.iter()
				.map(ToString::to_string)
				.collect()
		};
		// Before, `NaN` sorted ahead of every number and `5` outranked `q=1`.
		assert_eq!(tags("en;q=NaN, de;q=5, fr;q=inf, it"), ["it"]);
		for invalid in [
			"nan", "-0", "+1", "1.5", "1.001", "0.5555", ".5", "1e0", "0x1", "", "2",
		] {
			assert_eq!(tags(&format!("en;q={invalid}, de")), ["de"], "q={invalid}");
		}
		assert_eq!(
			tags("ja;q=0.001, de;q=1.000, fr;q=0., it;q=1., es;q=0.5, nl;q=0.50"),
			["de", "it", "es", "nl", "ja"]
		);
	}
}
