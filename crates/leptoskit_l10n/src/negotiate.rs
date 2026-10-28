//! Choosing the supported locale that serves a user best.

use crate::Locale;

/// The supported locale that best serves `preferred`, based on Flutter's
/// `basicLocaleListResolution`.
///
/// For each preferred locale in turn, it looks for a supported locale with
/// the same tag; then one with the same language and script; then one with
/// the same language and region; and last one with the same language alone.
/// For the languages written in more than one script, a tag without a script
/// is taken to have the one CLDR expects for its region, so `zh-TW` matches
/// `zh-Hant`, `zh-SG` matches `zh-Hans`, and `sr` matches `sr-Cyrl`.
///
/// A language-only match is held back only while the next preferred locale
/// has the same language, since that one might match a region exactly:
/// `[en-GB, en-US]` picks `en-US` from `[en, en-US]`, while `[en-GB, fr]`
/// picks `en`.
///
/// When several supported locales match at one step, the one with the
/// preferred locale's region comes first, then one without a region, which
/// stands in for the whole language, so a user asking for `pt-PT` gets `pt`
/// from `[pt-BR, pt]`. Otherwise the first listed wins.
///
/// Returns `None` when no preferred locale shares a language with a supported
/// one; the caller then falls back to its template locale.
///
/// Where Flutter chooses differently:
///
/// - Flutter holds back a language-only match of any preferred locale but the
///   first until the next one has had its turn, so `[de, fr-CA, en]` gets `en`
///   from `[en, fr]` there; here the user ranked French above English, and gets
///   `fr`.
/// - Flutter's language-only match is the first listed, so `pt-PT` gets `pt-BR`
///   from `[pt-BR, pt]` there.
/// - Flutter does not fill in scripts, so `zh-TW` gets whichever Chinese locale
///   is listed first.
/// - When no language matches, Flutter takes a supported locale from the same
///   region in another language (`de-CH` gets `fr-CH`); here nothing matches,
///   and the template serves.
///
/// ```
/// use leptoskit_l10n::Locale;
/// use leptoskit_l10n::negotiate;
///
/// let supported = [
/// 	Locale::new("en")?,
/// 	Locale::new("pt-BR")?,
/// 	Locale::new("zh-Hans")?,
/// 	Locale::new("zh-Hant")?,
/// ];
/// let best = |tags: &[&str]| {
/// 	let preferred: Vec<Locale> = tags.iter().map(|tag| Locale::new(tag).unwrap()).collect();
/// 	negotiate(&preferred, &supported).map(ToString::to_string)
/// };
/// assert_eq!(best(&["pt-br"]), Some("pt-BR".into()));
/// assert_eq!(best(&["pt-PT", "en-US"]), Some("pt-BR".into()));
/// assert_eq!(best(&["zh-Hant-TW"]), Some("zh-Hant".into()));
/// assert_eq!(best(&["zh-TW"]), Some("zh-Hant".into()));
/// assert_eq!(best(&["zh-CN"]), Some("zh-Hans".into()));
/// assert_eq!(best(&["de", "fr"]), None);
/// # Ok::<(), leptoskit_l10n::LocaleError>(())
/// ```
#[must_use]
pub fn negotiate<'a>(preferred: &[Locale], supported: &'a [Locale]) -> Option<&'a Locale> {
	let mut language_match = None;
	for (index, want) in preferred.iter().enumerate() {
		let same_language = |have: &&Locale| have.language() == want.language();
		// The user's region first, then a locale without one.
		let rank = |have: &&Locale| {
			match have.region() {
				Some(region) if want.region() == Some(region) => 0,
				None => 1,
				Some(_) => 2,
			}
		};
		if let Some(found) = supported.iter().find(|have| *have == want) {
			return Some(found);
		}
		if let Some(script) = written_in(want) {
			if let Some(found) = supported
				.iter()
				.filter(|have| same_language(have) && written_in(have) == Some(script))
				.min_by_key(rank)
			{
				return Some(found);
			}
		}
		if let Some(found) = supported.iter().find(|have| {
			same_language(have)
				&& want
					.region()
					.is_some_and(|region| have.region() == Some(region))
		}) {
			return Some(found);
		}
		if language_match.is_some() {
			return language_match;
		}
		if let Some(found) = supported.iter().filter(same_language).min_by_key(rank) {
			let next_shares_language = preferred
				.get(index + 1)
				.is_some_and(|next| next.language() == want.language());
			if !next_shares_language {
				return Some(found);
			}
			language_match = Some(found);
		}
	}
	language_match
}

/// The script a locale is written in: the one its tag names, or else the
/// one CLDR expects for its language and region.
fn written_in(locale: &Locale) -> Option<&str> {
	locale
		.script()
		.or_else(|| likely_script(locale.language(), locale.region()))
}

/// The script CLDR expects for a language written in more than one, in the
/// region given, from CLDR's likely subtags. `None` for every other
/// language: a script never tells their locales apart.
fn likely_script(language: &str, region: Option<&str>) -> Option<&'static str> {
	let (_, script, by_region) = LIKELY_SCRIPTS
		.iter()
		.find(|(known, ..)| *known == language)?;
	let in_region = region.and_then(|region| {
		by_region
			.iter()
			.find(|(known, _)| *known == region)
			.map(|(_, script)| *script)
	});
	Some(in_region.unwrap_or(script))
}

/// Regions, each with the script they write a language in.
type ByRegion = &'static [(&'static str, &'static str)];

/// The languages CLDR writes in more than one script: each with its usual
/// script, and the regions that write it in another. These are the
/// languages CLDR has locales with a script for, and those whose likely
/// script changes with the region, from CLDR 44's likely subtags.
const LIKELY_SCRIPTS: &[(&str, &str, ByRegion)] = &[
	(
		"az",
		"Latn",
		&[("IQ", "Arab"), ("IR", "Arab"), ("RU", "Cyrl")],
	),
	("bal", "Arab", &[]),
	("bm", "Latn", &[]),
	("bs", "Latn", &[]),
	("ff", "Latn", &[]),
	("ha", "Latn", &[("CM", "Arab"), ("SD", "Arab")]),
	("hi", "Deva", &[]),
	("iu", "Cans", &[]),
	(
		"kk",
		"Cyrl",
		&[
			("AF", "Arab"),
			("CN", "Arab"),
			("IR", "Arab"),
			("MN", "Arab"),
		],
	),
	("ks", "Arab", &[]),
	("ku", "Latn", &[("LB", "Arab")]),
	("kxv", "Latn", &[]),
	("ky", "Cyrl", &[("CN", "Arab"), ("TR", "Latn")]),
	("mn", "Cyrl", &[("CN", "Mong")]),
	("mni", "Beng", &[]),
	("ms", "Latn", &[("CC", "Arab")]),
	("pa", "Guru", &[("PK", "Arab")]),
	("sat", "Olck", &[]),
	("sd", "Arab", &[("IN", "Deva")]),
	("shi", "Tfng", &[]),
	(
		"sr",
		"Cyrl",
		&[
			("ME", "Latn"),
			("RO", "Latn"),
			("RU", "Latn"),
			("TR", "Latn"),
		],
	),
	("su", "Latn", &[]),
	("tg", "Cyrl", &[("PK", "Arab")]),
	("ug", "Arab", &[("KZ", "Cyrl"), ("MN", "Cyrl")]),
	("uz", "Latn", &[("AF", "Arab"), ("CN", "Cyrl")]),
	("vai", "Vaii", &[]),
	("yue", "Hant", &[("CN", "Hans")]),
	(
		"zh",
		"Hans",
		&[
			("AU", "Hant"),
			("BN", "Hant"),
			("GB", "Hant"),
			("GF", "Hant"),
			("HK", "Hant"),
			("ID", "Hant"),
			("MO", "Hant"),
			("PA", "Hant"),
			("PF", "Hant"),
			("PH", "Hant"),
			("SR", "Hant"),
			("TH", "Hant"),
			("TW", "Hant"),
			("US", "Hant"),
			("VN", "Hant"),
		],
	),
];

#[cfg(test)]
mod tests {
	use unic_langid::LanguageIdentifier;
	use unic_langid::subtags::Region;
	use unic_langid::subtags::Script;

	use super::*;

	fn locales(tags: &[&str]) -> Vec<Locale> {
		tags.iter().map(|tag| Locale::new(tag).unwrap()).collect()
	}

	fn best(preferred: &[&str], supported: &[&str]) -> Option<String> {
		negotiate(&locales(preferred), &locales(supported)).map(ToString::to_string)
	}

	#[test]
	fn exact_matches_win_over_everything() {
		assert_eq!(
			best(&["en-GB", "en"], &["en", "en-GB"]),
			Some("en-GB".into())
		);
		assert_eq!(best(&["EN-gb"], &["en-GB"]), Some("en-GB".into()));
	}

	#[test]
	fn script_then_region_then_language_in_that_order() {
		let supported = &["en", "zh-Hans", "zh-Hant", "pt-BR", "pt"];
		assert_eq!(best(&["zh-Hant-HK"], supported), Some("zh-Hant".into()));
		assert_eq!(best(&["pt-BR"], supported), Some("pt-BR".into()));
		assert_eq!(best(&["pt-PT"], supported), Some("pt".into()));
		assert_eq!(best(&["en-AU"], supported), Some("en".into()));
	}

	#[test]
	fn a_tag_without_a_script_has_the_one_its_region_writes() {
		let supported = &["en", "zh-Hans", "zh-Hant"];
		// Before, `zh-TW` fell through to the language and got the first
		// Chinese locale, Simplified.
		assert_eq!(best(&["zh-TW"], supported), Some("zh-Hant".into()));
		assert_eq!(best(&["zh-HK"], supported), Some("zh-Hant".into()));
		assert_eq!(best(&["zh-CN"], supported), Some("zh-Hans".into()));
		assert_eq!(
			best(&["zh"], &["zh-Hant", "zh-Hans"]),
			Some("zh-Hans".into())
		);
		assert_eq!(
			best(&["sr"], &["sr-Latn", "sr-Cyrl"]),
			Some("sr-Cyrl".into())
		);
		assert_eq!(
			best(&["sr-ME"], &["sr-Cyrl", "sr-Latn"]),
			Some("sr-Latn".into())
		);
		// The supported side counts too: `zh-TW` is written in `Hant`.
		assert_eq!(best(&["zh-HK"], &["zh-CN", "zh-TW"]), Some("zh-TW".into()));
		assert_eq!(best(&["zh-SG"], &["zh-TW", "zh-CN"]), Some("zh-CN".into()));
		// The script outranks a region written in the other one.
		assert_eq!(
			best(&["zh-TW"], &["zh-Hans-TW", "zh-Hant"]),
			Some("zh-Hant".into())
		);
		// Among locales in the script, the user's region, then none.
		assert_eq!(
			best(&["zh-TW"], &["zh-Hant-HK", "zh-Hant", "zh-Hant-TW"]),
			Some("zh-Hant-TW".into())
		);
		assert_eq!(
			best(&["zh-MO"], &["zh-Hant-HK", "zh-Hant"]),
			Some("zh-Hant".into())
		);
		// Languages written in one script are not affected.
		assert_eq!(best(&["en-AU"], &["en-GB", "en"]), Some("en".into()));
	}

	#[test]
	fn likely_scripts_agree_with_cldr() {
		let regions: Vec<String> = (b'A'..=b'Z')
			.flat_map(|first| (b'A'..=b'Z').map(move |second| [first, second]))
			.filter_map(|code| String::from_utf8(code.to_vec()).ok())
			.chain(["001", "150", "419"].map(String::from))
			.filter(|code| code.parse::<Region>().is_ok())
			.collect();
		for (language, ..) in LIKELY_SCRIPTS {
			for region in regions.iter().map(Some).chain([None]) {
				let tag = region.map_or_else(
					|| (*language).to_owned(),
					|region| format!("{language}-{region}"),
				);
				let mut cldr: LanguageIdentifier = tag.parse().unwrap();
				assert!(cldr.maximize(), "CLDR has no likely subtags for {tag}");
				assert_eq!(
					likely_script(language, region.map(String::as_str)),
					cldr.script.as_ref().map(Script::as_str),
					"{tag}"
				);
			}
		}
		assert_eq!(likely_script("en", Some("US")), None);
	}

	#[test]
	fn a_variant_matches_its_own_tag_first() {
		let supported = &["ca-ES", "ca-ES-valencia"];
		assert_eq!(
			best(&["ca-ES-valencia"], supported),
			Some("ca-ES-valencia".into())
		);
		assert_eq!(best(&["ca-ES"], supported), Some("ca-ES".into()));
		assert_eq!(
			best(&["ca-AD"], &["ca-ES-valencia", "ca"]),
			Some("ca".into())
		);
	}

	#[test]
	fn a_language_without_a_region_stands_in_for_the_language() {
		assert_eq!(best(&["pt-PT"], &["pt-BR", "pt"]), Some("pt".into()));
		assert_eq!(best(&["pt-PT"], &["pt-BR", "pt-AO"]), Some("pt-BR".into()));
		assert_eq!(best(&["pt"], &["pt-BR", "pt-AO"]), Some("pt-BR".into()));
	}

	#[test]
	fn language_match_waits_for_a_better_regional_match_of_the_same_language() {
		assert_eq!(
			best(&["en-GB", "en-US"], &["en", "en-US"]),
			Some("en-US".into())
		);
		assert_eq!(best(&["en-GB", "en-AU"], &["en", "fr"]), Some("en".into()));
		assert_eq!(best(&["en-GB", "fr"], &["en", "fr"]), Some("en".into()));
		assert_eq!(
			best(&["en-GB", "en-AU", "fr"], &["fr", "en"]),
			Some("en".into())
		);
	}

	#[test]
	fn later_preferences_serve_when_earlier_ones_cannot() {
		assert_eq!(best(&["de", "fr-CA"], &["en", "fr"]), Some("fr".into()));
		// Flutter picks `en` here: it holds `fr` back for the exact `en`. The
		// user ranked French above English.
		assert_eq!(
			best(&["de", "fr-CA", "en"], &["en", "fr"]),
			Some("fr".into())
		);
		assert_eq!(best(&["de", "it"], &["en", "fr"]), None);
		assert_eq!(best(&[], &["en"]), None);
		assert_eq!(best(&["en"], &[]), None);
	}
}
