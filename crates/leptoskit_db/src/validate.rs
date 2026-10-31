//! The rules columns declare, checked in Rust: on the server before a write,
//! and on the client before a form is sent.

use std::fmt;

/// Every rule a value broke, by field.
#[derive(
	Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct ValidationErrors {
	/// The broken rules, in field order.
	pub errors: Vec<FieldError>,
}

impl ValidationErrors {
	/// Whether every rule held.
	pub fn is_empty(&self) -> bool {
		self.errors.is_empty()
	}

	/// The broken rules for one field.
	pub fn field(&self, field: &str) -> impl Iterator<Item = &FieldError> {
		self.errors.iter().filter(move |error| error.field == field)
	}

	/// `Ok` when every rule held.
	pub fn into_result(self) -> Result<(), Self> {
		if self.is_empty() { Ok(()) } else { Err(self) }
	}

	pub(crate) fn push(&mut self, field: &str, rule: Rule) {
		self.errors.push(FieldError {
			field: field.to_string(),
			rule,
		});
	}
}

impl fmt::Display for ValidationErrors {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let messages: Vec<String> = self.errors.iter().map(ToString::to_string).collect();
		f.write_str(&messages.join("; "))
	}
}

impl std::error::Error for ValidationErrors {}

/// One broken rule.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct FieldError {
	/// The Rust field's name, which forms use too.
	pub field: String,
	/// The rule it broke.
	pub rule: Rule,
}

impl fmt::Display for FieldError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let field = &self.field;

		match &self.rule {
			Rule::MinLength(min) => write!(f, "`{field}` needs at least {}", characters(*min)),
			Rule::MaxLength(max) => write!(f, "`{field}` can have at most {}", characters(*max)),
			Rule::Min(min) => write!(f, "`{field}` can't be less than {min}"),
			Rule::Max(max) => write!(f, "`{field}` can't be more than {max}"),
			Rule::OneOf(values) => write!(f, "`{field}` must be one of {}", values.join(", ")),
			Rule::Pattern(pattern) => write!(f, "`{field}` must match `{pattern}`"),
			Rule::Coordinates(problem) => write!(f, "`{field}`: {problem}"),
			Rule::ValidGeometry => {
				write!(
					f,
					"`{field}` must be a valid shape: rings closed and not crossing themselves"
				)
			}
			Rule::Format(expected) => write!(f, "`{field}` must be {expected}"),
		}
	}
}

fn characters(count: u32) -> String {
	if count == 1 {
		"1 character".into()
	} else {
		format!("{count} characters")
	}
}

/// A rule a column declares.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", tag = "rule", content = "limit")]
pub enum Rule {
	/// At least this many characters.
	MinLength(u32),
	/// At most this many characters.
	MaxLength(u32),
	/// At least this number.
	Min(f64),
	/// At most this number.
	Max(f64),
	/// One of these values.
	OneOf(Vec<String>),
	/// Matching this regular expression.
	Pattern(String),
	/// Positions with finite coordinates, and for geography a longitude
	/// from −180 to 180 and a latitude from −90 to 90; the limit says what
	/// was wrong.
	Coordinates(String),
	/// A valid shape, as OGC defines it.
	ValidGeometry,
	/// Being what this describes, such as "a Solana address: 32 bytes in
	/// base58", which a scalar's own check decided.
	Format(String),
}

/// The checks generated `validate()` methods call. Optional values are only
/// checked when present.
#[doc(hidden)]
pub mod rules {
	use super::Rule;
	use super::ValidationErrors;

	/// A value with a length in characters.
	pub trait Text {
		/// The text, or `None` to skip the check.
		fn text(&self) -> Option<&str>;
	}

	impl Text for String {
		fn text(&self) -> Option<&str> {
			Some(self)
		}
	}

	impl Text for str {
		fn text(&self) -> Option<&str> {
			Some(self)
		}
	}

	impl<T: Text> Text for Option<T> {
		fn text(&self) -> Option<&str> {
			self.as_ref().and_then(Text::text)
		}
	}

	/// A value with a numeric size.
	pub trait Number {
		/// The number, or `None` to skip the check.
		fn number(&self) -> Option<f64>;
	}

	macro_rules! numbers {
		($($ty:ty),*) => {
			$(impl Number for $ty {
				fn number(&self) -> Option<f64> {
					Some(f64::from(*self))
				}
			})*
		};
	}

	numbers!(i8, i16, i32, u8, u16, u32, f32, f64);

	// Beyond 2^53 these round, which a range check on them can afford.
	impl Number for i64 {
		fn number(&self) -> Option<f64> {
			Some(*self as f64)
		}
	}

	impl Number for u64 {
		fn number(&self) -> Option<f64> {
			Some(*self as f64)
		}
	}

	impl<T: Number> Number for Option<T> {
		fn number(&self) -> Option<f64> {
			self.as_ref().and_then(Number::number)
		}
	}

	pub fn length(
		errors: &mut ValidationErrors,
		field: &str,
		value: &(impl Text + ?Sized),
		min: Option<u32>,
		max: Option<u32>,
	) {
		let Some(text) = value.text() else {
			return;
		};
		let length = text.chars().count();

		if let Some(min) = min.filter(|min| length < *min as usize) {
			errors.push(field, Rule::MinLength(min));
		}

		if let Some(max) = max.filter(|max| length > *max as usize) {
			errors.push(field, Rule::MaxLength(max));
		}
	}

	pub fn range(
		errors: &mut ValidationErrors,
		field: &str,
		value: &impl Number,
		min: Option<f64>,
		max: Option<f64>,
	) {
		let Some(number) = value.number() else {
			return;
		};

		if let Some(min) = min.filter(|min| number < *min) {
			errors.push(field, Rule::Min(min));
		}

		if let Some(max) = max.filter(|max| number > *max) {
			errors.push(field, Rule::Max(max));
		}
	}

	pub fn one_of(
		errors: &mut ValidationErrors,
		field: &str,
		value: &(impl Text + ?Sized),
		allowed: &[&str],
	) {
		if value.text().is_some_and(|text| !allowed.contains(&text)) {
			errors.push(
				field,
				Rule::OneOf(allowed.iter().map(ToString::to_string).collect()),
			);
		}
	}

	/// Records what a scalar's own check (`validate_with`) said the value
	/// must be.
	pub fn format(errors: &mut ValidationErrors, field: &str, result: Result<(), &'static str>) {
		if let Err(expected) = result {
			errors.push(field, Rule::Format(expected.to_string()));
		}
	}

	/// Checks a regular expression when the `regex` feature is on (always on
	/// the server). Patterns are compiled once per thread.
	pub fn pattern(
		errors: &mut ValidationErrors,
		field: &str,
		value: &(impl Text + ?Sized),
		pattern: &'static str,
	) {
		let Some(text) = value.text() else {
			return;
		};

		if !matches(text, pattern) {
			errors.push(field, Rule::Pattern(pattern.to_string()));
		}
	}

	#[cfg(feature = "regex")]
	fn matches(text: &str, pattern: &'static str) -> bool {
		use std::cell::RefCell;
		use std::collections::HashMap;

		thread_local! {
			static COMPILED: RefCell<HashMap<&'static str, regex::Regex>> = RefCell::new(HashMap::new());
		}

		COMPILED.with_borrow_mut(|compiled| {
			let regex = compiled.entry(pattern).or_insert_with(|| {
				regex::Regex::new(pattern)
					.expect("`#[db::table]` checked the pattern when it compiled")
			});
			regex.is_match(text)
		})
	}

	#[cfg(not(feature = "regex"))]
	fn matches(_: &str, _: &'static str) -> bool {
		true
	}
}
