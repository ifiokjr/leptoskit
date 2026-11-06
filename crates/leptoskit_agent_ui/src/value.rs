//! The values a document's variables and formulas hold.

use std::collections::BTreeMap;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;

/// A value in an agent document: what a variable holds and a formula returns.
///
/// Numbers are `f64`, as in JSON. Records come from the application, through
/// [`AgentUi::data`](crate::AgentUi::data), and are read with `budget.rent`.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
	/// No value: a missing attribute, or `null`.
	#[default]
	Null,
	/// `true` or `false`.
	Bool(bool),
	/// A number.
	Number(f64),
	/// Text.
	Text(String),
	/// An ordered list, written `[1, 2, 3]`.
	List(Vec<Self>),
	/// Named fields, read with `record.field`.
	Record(BTreeMap<String, Self>),
}

impl Value {
	/// The value as a number: a number, a boolean as `0` or `1`, or text that
	/// parses as one, ignoring a leading currency sign and thousands commas.
	pub fn as_number(&self) -> Option<f64> {
		match self {
			Self::Number(number) => Some(*number),
			Self::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
			Self::Text(text) => parse_number(text),
			Self::Null | Self::List(_) | Self::Record(_) => None,
		}
	}

	/// Whether the value counts as true in a condition: `false`, `null`,
	/// `0`, empty text, and an empty list do not.
	pub fn truthy(&self) -> bool {
		match self {
			Self::Null => false,
			Self::Bool(flag) => *flag,
			Self::Number(number) => *number != 0.0 && !number.is_nan(),
			Self::Text(text) => !text.is_empty(),
			Self::List(items) => !items.is_empty(),
			Self::Record(fields) => !fields.is_empty(),
		}
	}

	/// The value's type, for diagnostics.
	pub const fn kind(&self) -> &'static str {
		match self {
			Self::Null => "null",
			Self::Bool(_) => "boolean",
			Self::Number(_) => "number",
			Self::Text(_) => "text",
			Self::List(_) => "list",
			Self::Record(_) => "record",
		}
	}

	/// A rough size in bytes, to keep values within the document's limits.
	pub(crate) fn weight(&self) -> usize {
		match self {
			Self::Null | Self::Bool(_) | Self::Number(_) => 8,
			Self::Text(text) => text.len(),
			Self::List(items) => items.iter().map(Self::weight).sum::<usize>() + 8,
			Self::Record(fields) => {
				fields
					.iter()
					.map(|(key, value)| key.len() + value.weight())
					.sum::<usize>()
					+ 8
			}
		}
	}
}

/// Parses `text` as a number the way a person types one: `1,800`, `$42.50`,
/// `-3`.
pub(crate) fn parse_number(text: &str) -> Option<f64> {
	let trimmed = text.trim();
	let unsigned = trimmed.strip_prefix('-').unwrap_or(trimmed);
	let digits: String = unsigned
		.trim_start_matches(['$', '€', '£', '¥'])
		.chars()
		.filter(|character| *character != ',')
		.collect();

	if digits.is_empty() {
		return None;
	}

	let number: f64 = digits.parse().ok()?;
	if !number.is_finite() {
		return None;
	}

	Some(if unsigned.len() == trimmed.len() {
		number
	} else {
		-number
	})
}

/// Formats a number without trailing zeros: `1800`, `0.25`, `3.33`.
pub(crate) fn format_number(number: f64) -> String {
	if !number.is_finite() {
		return "—".to_owned();
	}

	let rounded = (number * 100.0).round() / 100.0;
	if rounded.fract() == 0.0 && rounded.abs() < 1e15 {
		return format!("{rounded:.0}");
	}

	let text = format!("{rounded:.2}");
	text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Groups an integer's digits in threes: `1234567` becomes `1,234,567`.
pub(crate) fn group_thousands(digits: &str) -> String {
	let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);

	for (index, digit) in digits.chars().enumerate() {
		if index > 0 && (digits.len() - index) % 3 == 0 {
			grouped.push(',');
		}

		grouped.push(digit);
	}

	grouped
}

impl fmt::Display for Value {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Null => Ok(()),
			Self::Bool(flag) => write!(formatter, "{flag}"),
			Self::Number(number) => formatter.write_str(&format_number(*number)),
			Self::Text(text) => formatter.write_str(text),
			Self::List(items) => {
				for (index, item) in items.iter().enumerate() {
					if index > 0 {
						formatter.write_str(", ")?;
					}

					write!(formatter, "{item}")?;
				}

				Ok(())
			}
			Self::Record(fields) => {
				for (index, (key, value)) in fields.iter().enumerate() {
					if index > 0 {
						formatter.write_str(", ")?;
					}

					write!(formatter, "{key}: {value}")?;
				}

				Ok(())
			}
		}
	}
}

impl Eq for Value {}

impl Hash for Value {
	fn hash<H: Hasher>(&self, state: &mut H) {
		std::mem::discriminant(self).hash(state);

		match self {
			Self::Null => {}
			Self::Bool(flag) => flag.hash(state),
			Self::Number(number) => number.to_bits().hash(state),
			Self::Text(text) => text.hash(state),
			Self::List(items) => items.hash(state),
			Self::Record(fields) => fields.hash(state),
		}
	}
}

impl From<f64> for Value {
	fn from(number: f64) -> Self {
		Self::Number(number)
	}
}

impl From<i64> for Value {
	fn from(number: i64) -> Self {
		Self::Number(number as f64)
	}
}

impl From<i32> for Value {
	fn from(number: i32) -> Self {
		Self::Number(f64::from(number))
	}
}

impl From<bool> for Value {
	fn from(flag: bool) -> Self {
		Self::Bool(flag)
	}
}

impl From<&str> for Value {
	fn from(text: &str) -> Self {
		Self::Text(text.to_owned())
	}
}

impl From<String> for Value {
	fn from(text: String) -> Self {
		Self::Text(text)
	}
}

impl<T: Into<Self>> From<Vec<T>> for Value {
	fn from(items: Vec<T>) -> Self {
		Self::List(items.into_iter().map(Into::into).collect())
	}
}

impl<T: Into<Self>> From<BTreeMap<String, T>> for Value {
	fn from(fields: BTreeMap<String, T>) -> Self {
		Self::Record(
			fields
				.into_iter()
				.map(|(key, value)| (key, value.into()))
				.collect(),
		)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_numbers_as_people_type_them() {
		assert_eq!(parse_number("1,800"), Some(1800.0));
		assert_eq!(parse_number("$42.50"), Some(42.5));
		assert_eq!(parse_number("-3"), Some(-3.0));
		assert_eq!(parse_number("abc"), None);
		assert_eq!(parse_number("inf"), None);
	}

	#[test]
	fn formats_without_trailing_zeros() {
		assert_eq!(format_number(1800.0), "1800");
		assert_eq!(format_number(0.25), "0.25");
		assert_eq!(format_number(10.0 / 3.0), "3.33");
		assert_eq!(group_thousands("1234567"), "1,234,567");
		assert_eq!(group_thousands("123"), "123");
	}
}
