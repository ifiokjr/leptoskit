//! Choosing the supported locale that serves a user best.

use crate::Locale;

/// The supported locale that best serves `preferred`, in the way Flutter
/// resolves locales.
///
/// For each preferred locale in turn, it looks for a supported locale with
/// the same tag; then one with the same language and script; then one with
/// the same language and region; and last one with the same language alone.
/// A language-only match is held back only while the next preferred locale
/// has the same language, since that one might match a region exactly:
/// `[en-GB, en-US]` picks `en-US` from `[en, en-US]`, while `[en-GB, fr]`
/// picks `en`.
///
/// When several supported locales share a language, one without a region
/// stands in for the whole language ahead of the regional ones, so a user
/// asking for `pt-PT` gets `pt` from `[pt-BR, pt]`. Otherwise the first
/// listed wins, at every step.
///
/// Returns `None` when no preferred locale shares a language with a supported
/// one; the caller then falls back to its template locale.
///
/// ```
/// use leptoskit_l10n::Locale;
/// use leptoskit_l10n::negotiate;
///
/// let supported = [
/// 	Locale::new("en")?,
/// 	Locale::new("pt-BR")?,
/// 	Locale::new("zh-Hant")?,
/// ];
/// let best = |tags: &[&str]| {
/// 	let preferred: Vec<Locale> = tags.iter().map(|tag| Locale::new(tag).unwrap()).collect();
/// 	negotiate(&preferred, &supported).map(ToString::to_string)
/// };
/// assert_eq!(best(&["pt-br"]), Some("pt-BR".into()));
/// assert_eq!(best(&["pt-PT", "en-US"]), Some("pt-BR".into()));
/// assert_eq!(best(&["zh-Hant-TW"]), Some("zh-Hant".into()));
/// assert_eq!(best(&["de", "fr"]), None);
/// # Ok::<(), leptoskit_l10n::LocaleError>(())
/// ```
#[must_use]
pub fn negotiate<'a>(preferred: &[Locale], supported: &'a [Locale]) -> Option<&'a Locale> {
	let mut language_match = None;
	for (index, want) in preferred.iter().enumerate() {
		let same_language = |have: &&Locale| have.language() == want.language();
		if let Some(found) = supported.iter().find(|have| *have == want) {
			return Some(found);
		}
		if let Some(found) = supported.iter().find(|have| {
			same_language(have)
				&& want
					.script()
					.is_some_and(|script| have.script() == Some(script))
		}) {
			return Some(found);
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
		if let Some(found) = supported
			.iter()
			.filter(same_language)
			.min_by_key(|have| have.region().is_some())
		{
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

#[cfg(test)]
mod tests {
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
		assert_eq!(best(&["zh-TW"], supported), Some("zh-Hans".into()));
		assert_eq!(best(&["en-AU"], supported), Some("en".into()));
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
		assert_eq!(best(&["de", "it"], &["en", "fr"]), None);
		assert_eq!(best(&[], &["en"]), None);
		assert_eq!(best(&["en"], &[]), None);
	}
}
