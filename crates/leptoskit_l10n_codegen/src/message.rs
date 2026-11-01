//! The subset of ICU `MessageFormat` that ARB messages are written in: text,
//! `{placeholders}`, `{count, plural, ...}`, `{kind, select, ...}`, and `#`
//! for the number inside a plural form.
//!
//! Quoting follows ICU: `''` is an apostrophe; a `'` before `{` or `}`, or
//! before `#` inside a plural, where `#` means the number, quotes literal
//! text up to the next `'`; any other `'` is itself, so `don't` needs no
//! care, and `'#'` outside a plural is three characters.
//!
//! Quoted text that reads as a placeholder, such as `'{name}'`, is an error
//! rather than literal text: Flutter quotes nothing unless `use-escaping` is
//! on, so a file written for it means the placeholder.

use std::fmt;

/// One piece of a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Part {
	/// Literal text, with quoting resolved.
	Text(String),
	/// `{name}`: the placeholder's value.
	Placeholder(String),
	/// `#` inside a plural form: the number being counted.
	Number,
	/// `{name, plural, ...}`.
	Plural(Plural),
	/// `{name, select, ...}`.
	Select(Select),
}

/// A message that changes with a number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Plural {
	/// The placeholder being counted.
	pub arg: String,
	/// The `=0 {...}` forms, in the order written.
	pub exact: Vec<(u64, Vec<Part>)>,
	/// The `one {...}`, `few {...}`, ... forms, in the order written.
	pub categories: Vec<(Category, Vec<Part>)>,
	/// The `other {...}` form, which every plural has.
	pub other: Vec<Part>,
}

/// A CLDR plural category other than `other`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Category {
	Zero,
	One,
	Two,
	Few,
	Many,
}

impl Category {
	fn parse(word: &str) -> Option<Self> {
		match word {
			"zero" => Some(Self::Zero),
			"one" => Some(Self::One),
			"two" => Some(Self::Two),
			"few" => Some(Self::Few),
			"many" => Some(Self::Many),
			_ => None,
		}
	}

	/// The `PluralCategory` variant of the runtime crate.
	pub(crate) fn variant(self) -> &'static str {
		match self {
			Self::Zero => "Zero",
			Self::One => "One",
			Self::Two => "Two",
			Self::Few => "Few",
			Self::Many => "Many",
		}
	}
}

/// A message that changes with a word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Select {
	/// The placeholder being matched.
	pub arg: String,
	/// The named forms, in the order written.
	pub cases: Vec<(String, Vec<Part>)>,
	/// The `other {...}` form, which every select has.
	pub other: Vec<Part>,
}

/// Where a message stops being well-formed, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SyntaxError {
	/// The 0-based character the problem is at.
	pub offset: usize,
	/// What is wrong and how to fix it.
	pub message: String,
}

impl fmt::Display for SyntaxError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			formatter,
			"{} (at character {})",
			self.message,
			self.offset + 1
		)
	}
}

/// Parses a message.
pub(crate) fn parse(text: &str) -> Result<Vec<Part>, SyntaxError> {
	let mut parser = Parser {
		chars: text.chars().collect(),
		position: 0,
	};
	parser.message(false, false)
}

struct Parser {
	chars: Vec<char>,
	position: usize,
}

impl Parser {
	fn peek(&self) -> Option<char> {
		self.chars.get(self.position).copied()
	}

	fn bump(&mut self) -> Option<char> {
		let next = self.peek();

		if next.is_some() {
			self.position += 1;
		}

		next
	}

	fn skip_whitespace(&mut self) {
		while self.peek().is_some_and(char::is_whitespace) {
			self.position += 1;
		}
	}

	fn error(&self, message: impl Into<String>) -> SyntaxError {
		SyntaxError {
			offset: self.position.min(self.chars.len()),
			message: message.into(),
		}
	}

	/// Reads `[A-Za-z0-9_]*`.
	fn word(&mut self) -> String {
		let mut word = String::new();

		while let Some(next) = self
			.peek()
			.filter(|next| next.is_ascii_alphanumeric() || *next == '_')
		{
			word.push(next);
			self.position += 1;
		}
		word
	}

	fn expect(&mut self, expected: char, message: &str) -> Result<(), SyntaxError> {
		if self.peek() == Some(expected) {
			self.position += 1;
			Ok(())
		} else {
			Err(self.error(message))
		}
	}

	/// Reads parts up to the end, or up to the `}` that closes a form when
	/// `nested`, which stays unread.
	fn message(&mut self, in_plural: bool, nested: bool) -> Result<Vec<Part>, SyntaxError> {
		let mut parts = Vec::new();
		let mut text = String::new();
		let flush = |text: &mut String, parts: &mut Vec<Part>| {
			if !text.is_empty() {
				parts.push(Part::Text(std::mem::take(text)));
			}
		};

		loop {
			match self.peek() {
				None if nested => {
					return Err(self.error("a form opened with `{` never closes; add `}`"));
				}

				None => break,

				Some('}') if nested => break,
				Some('}') => {
					return Err(self.error("`}` closes nothing; write `'}'` for a literal brace"));
				}
				Some('{') => {
					flush(&mut text, &mut parts);
					self.position += 1;
					parts.push(self.argument(in_plural)?);
				}

				Some('#') if in_plural => {
					flush(&mut text, &mut parts);
					self.position += 1;
					parts.push(Part::Number);
				}

				Some('\'') => {
					self.position += 1;
					self.quoted(in_plural, &mut text)?;
				}
				Some(next) => {
					text.push(next);
					self.position += 1;
				}
			}
		}

		flush(&mut text, &mut parts);
		Ok(parts)
	}

	/// Resolves what follows a `'`. `#` only starts quoted text where it is
	/// the number, inside a plural.
	fn quoted(&mut self, in_plural: bool, text: &mut String) -> Result<(), SyntaxError> {
		let start = self.position - 1;

		match self.peek() {
			Some('\'') => {
				self.position += 1;
				text.push('\'');
			}
			Some('{' | '}') => self.quoted_text(start, text)?,

			Some('#') if in_plural => self.quoted_text(start, text)?,
			_ => text.push('\''),
		}

		Ok(())
	}

	/// Reads quoted text up to the next lone `'`, which `start` opened.
	fn quoted_text(&mut self, start: usize, text: &mut String) -> Result<(), SyntaxError> {
		let mut quoted = String::new();

		loop {
			match self.bump() {
				Some('\'') if self.peek() == Some('\'') => {
					self.position += 1;
					quoted.push('\'');
				}

				None | Some('\'') => break,
				Some(next) => quoted.push(next),
			}
		}

		if let Some(name) = placeholder_in(&quoted) {
			return Err(error_at(
				start,
				format!(
					"`'{{{name}}}'` quotes `{{{name}}}`, so it shows as written rather than as \
					 the placeholder; write `''{{{name}}}''` for the placeholder between \
					 apostrophes, or `'{{'{name}'}}'` for the braces"
				),
			));
		}

		text.push_str(&quoted);
		Ok(())
	}

	/// Reads what follows a `{`, through the `}` that closes it.
	fn argument(&mut self, in_plural: bool) -> Result<Part, SyntaxError> {
		self.skip_whitespace();
		let start = self.position;
		let name = self.word();

		if name.is_empty() || name.starts_with(|first: char| first.is_ascii_digit()) {
			return Err(error_at(
				start,
				"a placeholder needs a name that starts with a letter or `_`: `{name}`",
			));
		}

		self.skip_whitespace();

		match self.bump() {
			Some('}') => Ok(Part::Placeholder(name)),
			Some(',') => {
				self.skip_whitespace();
				let kind = self.word();
				self.skip_whitespace();
				match kind.as_str() {
					"plural" => {
						self.expect(',', "`plural` needs a comma and its forms after it")?;
						self.plural(name).map(Part::Plural)
					}
					"select" => {
						self.expect(',', "`select` needs a comma and its forms after it")?;
						self.select(name, in_plural).map(Part::Select)
					}
					"" => {
						Err(self.error(format!(
							"`{{{name},` needs `plural` or `select` after the comma"
						)))
					}
					other => {
						Err(self.error(format!(
							"`{{{name}, {other}}}` is not supported: only `plural` and `select` \
							 are"
						)))
					}
				}
			}
			_ => {
				Err(self.error(format!(
					"`{{{name}` needs `}}` to close it, or `, plural,` or `, select,` and forms"
				)))
			}
		}
	}

	/// Reads a form's selector, `=N` or a word, returning it with the
	/// character it starts at.
	fn selector(&mut self) -> Result<(String, usize), SyntaxError> {
		let start = self.position;

		if self.peek() == Some('=') {
			self.position += 1;
			let digits = self.word();

			if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
				return Err(error_at(
					start,
					"`=` needs a whole number after it, such as `=0`",
				));
			}

			return Ok((format!("={digits}"), start));
		}

		let word = self.word();

		if word.is_empty() {
			return Err(self.error("expected a form such as `one {...}`, or `}` to end the forms"));
		}

		if self.peek() == Some(':') {
			return Err(error_at(
				start,
				"`offset:` is not supported; count the placeholder as it is",
			));
		}

		Ok((word, start))
	}

	/// Reads a form's text between braces.
	fn form(&mut self, selector: &str, in_plural: bool) -> Result<Vec<Part>, SyntaxError> {
		self.skip_whitespace();
		self.expect(
			'{',
			&format!("`{selector}` needs its text in braces: `{selector} {{...}}`"),
		)?;
		let body = self.message(in_plural, true)?;
		self.position += 1;
		Ok(body)
	}

	/// Whether the forms of a plural or select end here, consuming the `}`.
	fn forms_end(&mut self, arg: &str, kind: &str) -> Result<bool, SyntaxError> {
		self.skip_whitespace();

		match self.peek() {
			None => {
				Err(self.error(format!(
					"`{{{arg}, {kind},` never closes; add `}}` after its forms"
				)))
			}
			Some('}') => {
				self.position += 1;
				Ok(true)
			}
			_ => Ok(false),
		}
	}

	fn plural(&mut self, arg: String) -> Result<Plural, SyntaxError> {
		let mut exact = Vec::new();
		let mut categories: Vec<(Category, Vec<Part>)> = Vec::new();
		let mut other = None;

		while !self.forms_end(&arg, "plural")? {
			let (selector, start) = self.selector()?;
			let twice = || format!("`{selector}` is written twice; keep one form");

			if let Some(digits) = selector.strip_prefix('=') {
				let number: u64 = digits.parse().map_err(|_| {
					error_at(
						start,
						format!(
							"`{selector}` is too large a number; use a plural category instead"
						),
					)
				})?;

				let body = self.form(&selector, true)?;

				if exact.iter().any(|(seen, _)| *seen == number) {
					return Err(error_at(start, twice()));
				}

				exact.push((number, body));
			} else if selector == "other" {
				let body = self.form(&selector, true)?;

				if other.is_some() {
					return Err(error_at(start, twice()));
				}

				other = Some(body);
			} else if let Some(category) = Category::parse(&selector) {
				let body = self.form(&selector, true)?;

				if categories.iter().any(|(seen, _)| *seen == category) {
					return Err(error_at(start, twice()));
				}

				categories.push((category, body));
			} else {
				return Err(error_at(
					start,
					format!(
						"`{selector}` is not a plural form: use zero, one, two, few, many, other, \
						 or `=N`"
					),
				));
			}
		}

		let Some(other) = other else {
			return Err(error_at(
				self.position.saturating_sub(1),
				format!(
					"`{{{arg}, plural, ...}}` needs an `other` form, for every number no other \
					 form claims"
				),
			));
		};
		Ok(Plural {
			arg,
			exact,
			categories,
			other,
		})
	}

	fn select(&mut self, arg: String, in_plural: bool) -> Result<Select, SyntaxError> {
		let mut cases: Vec<(String, Vec<Part>)> = Vec::new();
		let mut other = None;

		while !self.forms_end(&arg, "select")? {
			let (selector, start) = self.selector()?;

			if selector.starts_with('=') {
				return Err(error_at(
					start,
					format!("`{selector}` is not a select form; forms are words, such as `male`"),
				));
			}

			let body = self.form(&selector, in_plural)?;
			let twice = || format!("`{selector}` is written twice; keep one form");

			if selector == "other" {
				if other.is_some() {
					return Err(error_at(start, twice()));
				}

				other = Some(body);
			} else if cases.iter().any(|(seen, _)| *seen == selector) {
				return Err(error_at(start, twice()));
			} else {
				cases.push((selector, body));
			}
		}

		let Some(other) = other else {
			return Err(error_at(
				self.position.saturating_sub(1),
				format!(
					"`{{{arg}, select, ...}}` needs an `other` form, for every value no other \
					 form names"
				),
			));
		};
		Ok(Select { arg, cases, other })
	}
}

/// The name of the first `{name}` in `text`, written the way a placeholder
/// is.
fn placeholder_in(text: &str) -> Option<&str> {
	text.match_indices('{').find_map(|(open, _)| {
		let rest = text[open + 1..].trim_start();
		let end = rest
			.find(|next: char| !(next.is_ascii_alphanumeric() || next == '_'))
			.unwrap_or(rest.len());
		let (name, after) = rest.split_at(end);
		let named = name.starts_with(|first: char| first.is_ascii_alphabetic() || first == '_');
		(named && after.trim_start().starts_with('}')).then_some(name)
	})
}

/// An error at a character other than the parser's current one.
fn error_at(offset: usize, message: impl Into<String>) -> SyntaxError {
	SyntaxError {
		offset,
		message: message.into(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn text(value: &str) -> Part {
		Part::Text(value.to_owned())
	}

	fn placeholder(name: &str) -> Part {
		Part::Placeholder(name.to_owned())
	}

	fn error(message: &str) -> SyntaxError {
		parse(message).expect_err(message)
	}

	#[test]
	fn text_and_placeholders() {
		assert_eq!(parse("").unwrap(), []);
		assert_eq!(parse("Hello").unwrap(), [text("Hello")]);
		assert_eq!(
			parse("Hello, { name }! You are {age}.").unwrap(),
			[
				text("Hello, "),
				placeholder("name"),
				text("! You are "),
				placeholder("age"),
				text("."),
			]
		);
		assert_eq!(parse("{_private_1}").unwrap(), [placeholder("_private_1")]);
	}

	#[test]
	fn quoting_follows_icu() {
		assert_eq!(parse("don't").unwrap(), [text("don't")]);
		assert_eq!(parse("it''s").unwrap(), [text("it's")]);
		assert_eq!(parse("'{'literal'}'").unwrap(), [text("{literal}")]);
		assert_eq!(
			parse("'{not a' {placeholder}").unwrap(),
			[text("{not a "), placeholder("placeholder")]
		);
		assert_eq!(parse("'{it''s quoted'").unwrap(), [text("{it's quoted")]);
		assert_eq!(
			parse("unterminated '{ to the end").unwrap(),
			[text("unterminated { to the end")]
		);
		// `#` is only syntax inside a plural, so only there does `'` quote it.
		assert_eq!(
			parse("a '#' outside a plural").unwrap(),
			[text("a '#' outside a plural")]
		);
		assert_eq!(
			parse("# outside a plural").unwrap(),
			[text("# outside a plural")]
		);

		assert_eq!(
			parse("{n, plural, other {'#' {n, select, other {'#'}}}}").unwrap(),
			[Part::Plural(Plural {
				arg: "n".to_owned(),
				exact: vec![],
				categories: vec![],
				other: vec![
					text("# "),
					Part::Select(Select {
						arg: "n".to_owned(),
						cases: vec![],
						other: vec![text("#")],
					}),
				],
			})]
		);
		assert_eq!(
			parse("{k, select, other {'#' ok}}").unwrap(),
			[Part::Select(Select {
				arg: "k".to_owned(),
				cases: vec![],
				other: vec![text("'#' ok")],
			})]
		);
		assert_eq!(parse("trailing '").unwrap(), [text("trailing '")]);
		// The braces quoted one by one are literal, and nothing like a
		// placeholder is quoted.
		assert_eq!(parse("'{'name'}'").unwrap(), [text("{name}")]);
		assert_eq!(
			parse("''{name}''").unwrap(),
			[text("'"), placeholder("name"), text("'"),]
		);
		assert_eq!(
			parse("'{0}' '{ not one }'").unwrap(),
			[text("{0} { not one }")]
		);
	}

	#[test]
	fn quoted_placeholders_are_errors() {
		// Flutter, with `use-escaping` off as it is by default, shows the name
		// here; ICU quoting would show `{name}`.
		for (message, offset) in [
			("Delete '{name}'?", 7),
			("'{ name }'", 0),
			("'{a} and {b}'", 0),
			("{n, plural, other {'#{n}'}}", 19),
		] {
			let found = error(message);
			assert_eq!(found.offset, offset, "{message}");
			assert!(
				found.message.starts_with("`'{")
					&& found.message.contains("rather than as the placeholder"),
				"{message}: {}",
				found.message
			);
		}
		assert_eq!(
			error("Delete '{name}'?").message,
			"`'{name}'` quotes `{name}`, so it shows as written rather than as the placeholder; \
			 write `''{name}''` for the placeholder between apostrophes, or `'{'name'}'` for the \
			 braces"
		);
	}

	#[test]
	fn plurals() {
		let parsed =
			parse("{count, plural, =0 {none} =1{just one} one {# item} other {# items}}").unwrap();
		assert_eq!(
			parsed,
			[Part::Plural(Plural {
				arg: "count".to_owned(),
				exact: vec![(0, vec![text("none")]), (1, vec![text("just one")])],
				categories: vec![(Category::One, vec![Part::Number, text(" item")])],
				other: vec![Part::Number, text(" items")],
			})]
		);
		let parsed = parse("{n,plural,other{{n}}}").unwrap();
		assert_eq!(
			parsed,
			[Part::Plural(Plural {
				arg: "n".to_owned(),
				exact: vec![],
				categories: vec![],
				other: vec![placeholder("n")],
			})]
		);
		assert_eq!(Category::parse("few"), Some(Category::Few));
		for (keyword, category) in [
			("zero", Category::Zero),
			("one", Category::One),
			("two", Category::Two),
			("few", Category::Few),
			("many", Category::Many),
		] {
			assert_eq!(Category::parse(keyword), Some(category));
			assert_eq!(category.variant().to_lowercase(), keyword);
		}
		assert_eq!(Category::parse("other"), None);
	}

	#[test]
	fn selects_and_nesting() {
		let parsed = parse(
			"{gender, select, male {He has {count, plural, one {# cat} other {# cats}}} other \
			 {They have #}}",
		)
		.unwrap();
		let [Part::Select(select)] = parsed.as_slice() else {
			panic!("{parsed:?}");
		};
		assert_eq!(select.arg, "gender");
		assert_eq!(select.cases.len(), 1);
		assert_eq!(select.cases[0].0, "male");
		assert!(matches!(
			select.cases[0].1.as_slice(),
			[Part::Text(_), Part::Plural(_)]
		));
		// `#` outside every plural is text, even inside a select.
		assert_eq!(select.other, [text("They have #")]);

		let parsed = parse("{count, plural, other {{kind, select, other {# of {kind}}}}}").unwrap();
		let [Part::Plural(plural)] = parsed.as_slice() else {
			panic!("{parsed:?}");
		};
		let [Part::Select(inner)] = plural.other.as_slice() else {
			panic!("{plural:?}");
		};
		// `#` inside a select inside a plural is still the plural's number.
		assert_eq!(
			inner.other,
			[Part::Number, text(" of "), placeholder("kind")]
		);
	}

	#[test]
	fn errors_say_where_and_what() {
		let cases = [
			(
				"}",
				0,
				"`}` closes nothing; write `'}'` for a literal brace",
			),
			(
				"a {",
				3,
				"a placeholder needs a name that starts with a letter or `_`: `{name}`",
			),
			(
				"{9lives}",
				1,
				"a placeholder needs a name that starts with a letter or `_`: `{name}`",
			),
			(
				"{name",
				5,
				"`{name` needs `}` to close it, or `, plural,` or `, select,` and forms",
			),
			(
				"{name,}",
				6,
				"`{name,` needs `plural` or `select` after the comma",
			),
			(
				"{when, date}",
				11,
				"`{when, date}` is not supported: only `plural` and `select` are",
			),
			(
				"{n, plural other{}}",
				11,
				"`plural` needs a comma and its forms after it",
			),
			(
				"{n, select other{}}",
				11,
				"`select` needs a comma and its forms after it",
			),
			(
				"{n, plural, one {x}",
				19,
				"`{n, plural,` never closes; add `}` after its forms",
			),
			(
				"{n, select, other {x}",
				21,
				"`{n, select,` never closes; add `}` after its forms",
			),
			(
				"{n, plural, one {x",
				18,
				"a form opened with `{` never closes; add `}`",
			),
			(
				"{n, plural, = {x} other {y}}",
				12,
				"`=` needs a whole number after it, such as `=0`",
			),
			(
				"{n, plural, offset:1 other {y}}",
				12,
				"`offset:` is not supported; count the placeholder as it is",
			),
			(
				"{n, plural, one x other {y}}",
				16,
				"`one` needs its text in braces: `one {...}`",
			),
			(
				"{n, plural, one {x} one {y} other {z}}",
				20,
				"`one` is written twice; keep one form",
			),
			(
				"{n, plural, =1 {x} =1 {y} other {z}}",
				19,
				"`=1` is written twice; keep one form",
			),
			(
				"{n, plural, other {x} other {y}}",
				22,
				"`other` is written twice; keep one form",
			),
			(
				"{n, plural, =99999999999999999999 {x} other {y}}",
				12,
				"`=99999999999999999999` is too large a number; use a plural category instead",
			),
			(
				"{n, plural, some {x} other {y}}",
				12,
				"`some` is not a plural form: use zero, one, two, few, many, other, or `=N`",
			),
			(
				"{n, plural, one {x}}",
				19,
				"`{n, plural, ...}` needs an `other` form, for every number no other form claims",
			),
			(
				"{n, plural, {x}}",
				12,
				"expected a form such as `one {...}`, or `}` to end the forms",
			),
			(
				"{n, select, =1 {x} other {y}}",
				12,
				"`=1` is not a select form; forms are words, such as `male`",
			),
			(
				"{n, select, a {x} a {y} other {z}}",
				18,
				"`a` is written twice; keep one form",
			),
			(
				"{n, select, other {x} other {y}}",
				22,
				"`other` is written twice; keep one form",
			),
			(
				"{n, select, a {x}}",
				17,
				"`{n, select, ...}` needs an `other` form, for every value no other form names",
			),
		];

		for (message, offset, expected) in cases {
			let found = error(message);
			assert_eq!(
				(found.offset, found.message.as_str()),
				(offset, expected),
				"{message}"
			);
		}

		assert_eq!(
			error("}").to_string(),
			"`}` closes nothing; write `'}'` for a literal brace (at character 1)"
		);
	}
}
