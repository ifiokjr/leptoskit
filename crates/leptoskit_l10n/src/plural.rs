//! CLDR plural categories, so a message can say "1 item" and "2 items" in
//! every language's own terms.

use std::fmt;

use intl_pluralrules::PluralRuleType;
use intl_pluralrules::operands::PluralOperands;
use unic_langid::LanguageIdentifier;

use crate::Locale;

/// A CLDR plural category: which form of a message a number takes.
///
/// English has `one` and `other`; Arabic uses all six; Japanese has `other`
/// alone. Every message with a plural names an `other` form, so there is
/// always something to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PluralCategory {
	/// `zero`, as in Arabic and Latvian.
	Zero,
	/// `one`: the singular, in most languages that have one.
	One,
	/// `two`: the dual, as in Arabic, Hebrew, Welsh, and Slovenian.
	Two,
	/// `few`, as in the Slavic languages, Welsh, and Arabic.
	Few,
	/// `many`, as in the Slavic languages, Arabic, and French for large
	/// numbers.
	Many,
	/// `other`: the form every language has, and where every number goes
	/// that no other rule claims.
	Other,
}

impl PluralCategory {
	/// The CLDR keyword, as written in a message: `one`, `other`.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Zero => "zero",
			Self::One => "one",
			Self::Two => "two",
			Self::Few => "few",
			Self::Many => "many",
			Self::Other => "other",
		}
	}

	fn from_cldr(category: &intl_pluralrules::PluralCategory) -> Self {
		match category {
			intl_pluralrules::PluralCategory::ZERO => Self::Zero,
			intl_pluralrules::PluralCategory::ONE => Self::One,
			intl_pluralrules::PluralCategory::TWO => Self::Two,
			intl_pluralrules::PluralCategory::FEW => Self::Few,
			intl_pluralrules::PluralCategory::MANY => Self::Many,
			intl_pluralrules::PluralCategory::OTHER => Self::Other,
		}
	}
}

impl fmt::Display for PluralCategory {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.as_str())
	}
}

/// The cardinal plural rules of one locale.
///
/// The rules are CLDR's, found by the full tag first, then by language and
/// region, then by language alone: `pt-BR` uses the `pt` rules, while
/// `pt-PT` has its own. A language CLDR has no rules for puts every number
/// in [`PluralCategory::Other`].
///
/// ```
/// use leptoskit_l10n::Locale;
/// use leptoskit_l10n::PluralCategory;
/// use leptoskit_l10n::PluralRules;
///
/// let english = PluralRules::for_locale(&Locale::new("en-US")?);
/// assert_eq!(english.category(1), PluralCategory::One);
/// assert_eq!(english.category(2), PluralCategory::Other);
/// assert_eq!(english.category(1.5), PluralCategory::Other);
///
/// let russian = PluralRules::for_locale(&Locale::new("ru")?);
/// assert_eq!(russian.category(21), PluralCategory::One);
/// assert_eq!(russian.category(3), PluralCategory::Few);
/// assert_eq!(russian.category(11), PluralCategory::Many);
/// # Ok::<(), leptoskit_l10n::LocaleError>(())
/// ```
#[derive(Clone)]
pub struct PluralRules {
	locale: Locale,
	rules: Option<intl_pluralrules::PluralRules>,
}

impl PluralRules {
	/// The cardinal rules for `locale`.
	#[must_use]
	pub fn for_locale(locale: &Locale) -> Self {
		let id = locale.identifier();
		let candidates = [
			LanguageIdentifier::from_parts(id.language, id.script, id.region, &[]),
			LanguageIdentifier::from_parts(id.language, None, id.region, &[]),
			LanguageIdentifier::from_parts(id.language, None, None, &[]),
		];
		let rules = candidates.into_iter().find_map(|candidate| {
			intl_pluralrules::PluralRules::create(candidate, PluralRuleType::CARDINAL).ok()
		});
		Self {
			locale: locale.clone(),
			rules,
		}
	}

	/// The locale the rules are for.
	#[must_use]
	pub fn locale(&self) -> &Locale {
		&self.locale
	}

	/// The category `number` takes.
	#[must_use]
	pub fn category(&self, number: impl PluralNumber) -> PluralCategory {
		number.category(self)
	}

	fn select(&self, number: impl TryInto<PluralOperands>) -> PluralCategory {
		self.rules
			.as_ref()
			.and_then(|rules| rules.select(number).ok())
			.map_or(PluralCategory::Other, |category| {
				PluralCategory::from_cldr(&category)
			})
	}
}

impl fmt::Debug for PluralRules {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("PluralRules")
			.field("locale", &self.locale)
			.field("known", &self.rules.is_some())
			.finish()
	}
}

mod sealed {
	pub trait Sealed {}
}

/// A number plural rules can place in a category: any integer, or a float.
///
/// Rules look at the absolute value, so `-1` is `one` in English just as `1`
/// is. Floats count by their shortest decimal form: `1.0` is `one` and `1.5`
/// is `other`. A float that is not a number is `other`.
pub trait PluralNumber: sealed::Sealed {
	#[doc(hidden)]
	fn category(self, rules: &PluralRules) -> PluralCategory;
}

macro_rules! unsigned_plural_numbers {
	($($ty:ty),*) => {$(
		impl sealed::Sealed for $ty {}

		impl PluralNumber for $ty {
			fn category(self, rules: &PluralRules) -> PluralCategory {
				rules.select(u64::from(self))
			}
		}
	)*};
}

macro_rules! signed_plural_numbers {
	($($ty:ty),*) => {$(
		impl sealed::Sealed for $ty {}

		impl PluralNumber for $ty {
			fn category(self, rules: &PluralRules) -> PluralCategory {
				rules.select(u64::from(self.unsigned_abs()))
			}
		}
	)*};
}

macro_rules! float_plural_numbers {
	($($ty:ty),*) => {$(
		impl sealed::Sealed for $ty {}

		impl PluralNumber for $ty {
			fn category(self, rules: &PluralRules) -> PluralCategory {
				rules.select(self)
			}
		}
	)*};
}

unsigned_plural_numbers!(u8, u16, u32, u64);
signed_plural_numbers!(i8, i16, i32, i64);
float_plural_numbers!(f32, f64);

impl sealed::Sealed for usize {}

impl PluralNumber for usize {
	fn category(self, rules: &PluralRules) -> PluralCategory {
		rules.select(self as u64)
	}
}

impl sealed::Sealed for isize {}

impl PluralNumber for isize {
	fn category(self, rules: &PluralRules) -> PluralCategory {
		rules.select(self.unsigned_abs() as u64)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn rules(tag: &str) -> PluralRules {
		PluralRules::for_locale(&Locale::new(tag).unwrap())
	}

	#[test]
	fn every_category_has_a_keyword() {
		let all = [
			PluralCategory::Zero,
			PluralCategory::One,
			PluralCategory::Two,
			PluralCategory::Few,
			PluralCategory::Many,
			PluralCategory::Other,
		];
		let keywords: Vec<String> = all.iter().map(ToString::to_string).collect();
		assert_eq!(keywords, ["zero", "one", "two", "few", "many", "other"]);
	}

	#[test]
	fn arabic_uses_all_six_categories() {
		let arabic = rules("ar");
		assert_eq!(arabic.category(0), PluralCategory::Zero);
		assert_eq!(arabic.category(1), PluralCategory::One);
		assert_eq!(arabic.category(2), PluralCategory::Two);
		assert_eq!(arabic.category(3), PluralCategory::Few);
		assert_eq!(arabic.category(11), PluralCategory::Many);
		assert_eq!(arabic.category(100), PluralCategory::Other);
	}

	#[test]
	fn regions_scripts_and_unknown_languages_fall_back_in_order() {
		assert_eq!(rules("pt").category(0), PluralCategory::One);
		assert_eq!(rules("pt-BR").category(0), PluralCategory::One);
		assert_eq!(rules("pt-PT").category(0), PluralCategory::Other);
		assert_eq!(rules("zh-Hant-TW").category(1), PluralCategory::Other);
		assert_eq!(rules("ru-Cyrl-RU").category(1), PluralCategory::One);
		let unknown = rules("xx");
		assert_eq!(unknown.category(1), PluralCategory::Other);
		assert_eq!(unknown.locale(), &Locale::new("xx").unwrap());

		assert_eq!(
			format!("{unknown:?}"),
			"PluralRules { locale: Locale(\"xx\"), known: false }"
		);
		assert!(format!("{:?}", rules("en")).ends_with("known: true }"));
	}

	#[test]
	fn every_number_type_counts_by_its_absolute_value() {
		let english = rules("en");
		assert_eq!(english.category(1_u8), PluralCategory::One);
		assert_eq!(english.category(1_u16), PluralCategory::One);
		assert_eq!(english.category(1_u32), PluralCategory::One);
		assert_eq!(english.category(1_u64), PluralCategory::One);
		assert_eq!(english.category(1_usize), PluralCategory::One);
		assert_eq!(english.category(-1_i8), PluralCategory::One);
		assert_eq!(english.category(-1_i16), PluralCategory::One);

		assert_eq!(english.category(-1_i32), PluralCategory::One);
		assert_eq!(english.category(i64::MIN), PluralCategory::Other);
		assert_eq!(english.category(-1_isize), PluralCategory::One);
		assert_eq!(english.category(1.0_f32), PluralCategory::One);
		assert_eq!(english.category(-1.0_f64), PluralCategory::One);
		assert_eq!(english.category(1.5_f64), PluralCategory::Other);
		assert_eq!(english.category(f64::NAN), PluralCategory::Other);
		assert_eq!(english.category(f64::INFINITY), PluralCategory::Other);
		assert_eq!(rules("fr").category(1.5), PluralCategory::One);
	}
}
