//! Rust expressions inside markup: what is sent to rustfmt, and what comes
//! back.

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

use crate::lexer::Lexer;
use crate::lexer::MacroCall;
use crate::lexer::Token;
use crate::lexer::verbatim_lines;

/// One expression for rustfmt, with where it will sit: rustfmt sees the
/// same indentation and line prefix so its width decisions hold in place.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Request {
	/// The expression with its markup macros masked and its common
	/// indentation removed.
	pub text: String,
	/// The width of the indentation of the line the expression starts on.
	pub indent: usize,
	/// How many columns precede the expression on its first line.
	pub prefix: usize,
}

/// A formatted expression: lines relative to the indentation it is placed
/// at.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Formatted {
	pub text: String,
	/// For each line, whether it starts inside a string literal and must be
	/// printed without indentation.
	pub verbatim: Vec<bool>,
	/// rustfmt could not parse the expression, so `text` is as written.
	pub unparsed: bool,
}

impl Formatted {
	pub(crate) fn new(text: String) -> Self {
		let verbatim = verbatim_lines(&text);
		Self {
			text,
			verbatim,
			unparsed: false,
		}
	}

	/// An expression rustfmt rejected, kept as written.
	pub(crate) fn unparsed(text: String) -> Self {
		Self {
			unparsed: true,
			..Self::new(text)
		}
	}
}

/// Answers requests for formatted expressions.
pub(crate) trait Resolve {
	/// The formatted expression, or `None` when it is not known yet; the
	/// request is then remembered so a later rustfmt run can answer it.
	fn resolve(&mut self, request: Request) -> Option<Arc<Formatted>>;
}

/// Answers every request with its own text and records it. Laying out the
/// markup with this resolver predicts the expressions the real layout will
/// ask for, so they can go to rustfmt in the same run as the file.
#[derive(Default)]
pub(crate) struct Predict {
	pub requests: HashSet<Request>,
}

impl Resolve for Predict {
	fn resolve(&mut self, request: Request) -> Option<Arc<Formatted>> {
		let formatted = Arc::new(Formatted::new(request.text.clone()));
		self.requests.insert(request);
		Some(formatted)
	}
}

/// rustfmt's answers so far, and the requests still waiting for one.
#[derive(Default)]
pub(crate) struct Memo {
	pub answers: HashMap<Request, Arc<Formatted>>,
	pub missing: HashSet<Request>,
}

impl Resolve for Memo {
	fn resolve(&mut self, request: Request) -> Option<Arc<Formatted>> {
		if let Some(formatted) = self.answers.get(&request) {
			return Some(Arc::clone(formatted));
		}

		self.missing.insert(request);
		None
	}
}

/// The stand-in rustfmt sees for a markup macro's body.
///
/// rustfmt re-indents the lines of macro bodies it cannot parse, including
/// lines inside multi-line string literals, which changes those strings. So
/// rustfmt never sees markup: a single-line body becomes underscores of the
/// same width, so rustfmt plans the surrounding code around a macro of the
/// right size, and a multi-line body becomes `_ _` on a line of its own,
/// indented like the macro's line so the text around it keeps its shared
/// indentation. Two tokens keep a parenthesized mask from parsing as one
/// argument that rustfmt would pull onto one line.
fn mask_body(body: &str, indentation: &str) -> String {
	match body_shape(body) {
		Shape::Line(mask) => mask,
		Shape::Lines => format!("\n{indentation}_ _\n{indentation}"),
	}
}

/// What rustfmt learns about a markup macro from its mask: everything that
/// can change rustfmt's layout of the code around it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Shape {
	/// One line, as wide as these underscores and spaces.
	Line(String),
	/// More than one line.
	Lines,
}

pub(crate) fn body_shape(body: &str) -> Shape {
	if body.contains('\n') {
		return Shape::Lines;
	}

	Shape::Line(
		body.chars()
			.map(|character| {
				if character.is_whitespace() {
					character
				} else {
					'_'
				}
			})
			.collect(),
	)
}

/// `text` with the bodies of `calls` masked.
pub(crate) fn mask_calls(text: &str, calls: &[MacroCall]) -> String {
	let mut masked = String::with_capacity(text.len());
	let mut cursor = 0;

	for call in calls {
		masked.push_str(&text[cursor..call.body.start]);
		let line_start = text[..call.range.start]
			.rfind('\n')
			.map_or(0, |index| index + 1);
		let indentation = leading_whitespace(&text[line_start..]);
		masked.push_str(&mask_body(&text[call.body.clone()], indentation));
		cursor = call.body.end;
	}

	masked.push_str(&text[cursor..]);
	masked
}

/// Removes the indentation shared by the lines after the first, which keeps
/// a request the same wherever its expression was indented.
pub(crate) fn dedent(text: &str) -> String {
	if !text.contains('\n') {
		return text.to_owned();
	}

	let verbatim = verbatim_lines(text);
	let indentable = || {
		text.split('\n')
			.zip(&verbatim)
			.skip(1)
			.filter(|(line, verbatim)| !**verbatim && !line.trim().is_empty())
			.map(|(line, _)| line)
	};
	let common = indentable()
		.map(leading_whitespace)
		.reduce(|common, indentation| common_prefix(common, indentation))
		.unwrap_or("");

	if common.is_empty() {
		return text.to_owned();
	}

	text.split('\n')
		.zip(&verbatim)
		.enumerate()
		.map(|(index, (line, verbatim))| {
			if index == 0 || *verbatim {
				line
			} else {
				line.strip_prefix(common)
					.unwrap_or_else(|| line.trim_start())
			}
		})
		.collect::<Vec<_>>()
		.join("\n")
}

pub(crate) fn leading_whitespace(line: &str) -> &str {
	&line[..line.len() - line.trim_start().len()]
}

fn common_prefix<'a>(left: &'a str, right: &str) -> &'a str {
	let length = left
		.bytes()
		.zip(right.bytes())
		.take_while(|(left, right)| left == right)
		.count();
	&left[..length]
}

/// An expression rustfmt would print exactly as written: a literal, a path,
/// or a short chain such as `count.get()`. Formatting these needs no rustfmt
/// run.
pub(crate) fn is_trivial(text: &str) -> bool {
	if text.is_empty() || text.contains(char::is_whitespace) {
		return false;
	}

	let mut previous = None;
	Lexer::new(text).all(|(token, _)| {
		let allowed = match token {
			Token::Ident
			| Token::Literal
			| Token::Lifetime
			| Token::Open(b'(' | b'[')
			| Token::Close(b']') => true,
			Token::Punct(byte) => matches!(byte, b'.' | b':' | b'&' | b'*' | b'!' | b'?'),
			Token::Close(b')') => previous == Some(Token::Open(b'(')),
			_ => false,
		};

		previous = Some(token);
		allowed
	})
}

/// Whether `text` is a single literal, which `always_unless_literal` leaves
/// unbraced.
pub(crate) fn is_literal(text: &str) -> bool {
	let mut tokens = Lexer::new(text);
	matches!(tokens.next(), Some((Token::Literal, _))) && tokens.next().is_none()
		|| matches!(text, "true" | "false")
}

/// Whether `text` starts with a parenthesis, as a tuple or a parenthesized
/// expression does.
pub(crate) fn starts_parenthesized(text: &str) -> bool {
	matches!(Lexer::new(text).next(), Some((Token::Open(b'('), _)))
}

/// Whether an attribute value parses back the same without braces.
///
/// rstml ends a tag at the first `>` outside a group, and parses an unbraced
/// value as one greedy Rust expression. So the braces stay when the value
/// holds a `<` or `>` outside a group, begins with a block, ends in an
/// operator (`x..` would swallow the next attribute), or, before a braced
/// attribute such as `{..}`, ends in a path (`a {..}` reads as a struct
/// literal).
pub(crate) fn unbraced_is_safe(text: &str, before_block: bool) -> bool {
	let mut depth = 0usize;
	let mut first = None;
	let mut last = None;
	let mut before_last = None;

	for (token, _) in Lexer::new(text) {
		match token {
			Token::LineComment | Token::BlockComment => return false,
			Token::Open(_) => {
				if depth == 0 {
					first.get_or_insert(token);
					before_last = last.replace(token);
				}

				depth += 1;
			}
			Token::Close(_) => {
				depth = depth.saturating_sub(1);
				if depth == 0 {
					before_last = last.replace(token);
				}
			}

			_ if depth > 0 => {}
			Token::Punct(b'<' | b'>') => return false,
			_ => {
				first.get_or_insert(token);
				before_last = last.replace(token);
			}
		}
	}

	if first == Some(Token::Open(b'{')) {
		return false;
	}

	match last {
		None => false,
		Some(Token::Punct(byte)) => byte == b'?',

		Some(Token::Ident) if before_block => before_last == Some(Token::Punct(b'.')),
		Some(_) => true,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::lexer::find_macros;

	#[test]
	fn masks_markup_bodies_by_shape() {
		let text = "move || view! { <a b=\"c\"/> }";
		let calls = find_macros(text, &["view".to_owned()]);
		assert_eq!(mask_calls(text, &calls), "move || view! { __ _______ }");
		let text = "\t\tview! {\n\t\t\t<a />\n\t\t}";
		let calls = find_macros(text, &["view".to_owned()]);
		assert_eq!(mask_calls(text, &calls), "\t\tview! {\n\t\t_ _\n\t\t}");
		assert_eq!(body_shape("\n<a />\n"), Shape::Lines);
		assert_eq!(body_shape(" <a /> "), Shape::Line(" __ __ ".to_owned()));
	}

	#[test]
	fn dedents_all_but_literal_lines() {
		let text = "{\n\t\t\tlet a = \"x\n  y\";\n\t\t\ta\n\t\t}";
		assert_eq!(dedent(text), "{\n\tlet a = \"x\n  y\";\n\ta\n}");
		assert_eq!(dedent("one line"), "one line");
	}

	#[test]
	fn recognizes_trivial_expressions() {
		for text in [
			"count",
			"a::b",
			"self.value",
			"count.get()",
			"\"text\"",
			"&x",
			"items[0]",
		] {
			assert!(is_trivial(text), "{text}");
		}

		for text in ["a+b", "f(a,b)", "move || x", "x.get( )", "f(x)", "a - b"] {
			assert!(!is_trivial(text), "{text}");
		}
	}

	#[test]
	fn keeps_braces_the_parser_needs() {
		assert!(unbraced_is_safe("count", false));
		assert!(unbraced_is_safe("move |_| set(x > 1)", false));
		assert!(unbraced_is_safe("value.get()?", false));
		assert!(unbraced_is_safe("a.b", true));
		assert!(!unbraced_is_safe("a > b", false));
		assert!(!unbraced_is_safe("Vec::<u8>::new()", false));
		assert!(!unbraced_is_safe("|| -> u8 { 1 }", false));
		assert!(!unbraced_is_safe("0..", false));

		assert!(!unbraced_is_safe("name", true));
		assert!(!unbraced_is_safe("{ x }.y()", false));
		assert!(!unbraced_is_safe("x /* why */", false));
	}

	#[test]
	fn recognizes_parenthesized_values() {
		assert!(starts_parenthesized("(a, b)"));
		assert!(starts_parenthesized("(value)"));
		assert!(!starts_parenthesized("f(a, b)"));
		assert!(!starts_parenthesized("[a, b]"));
	}

	#[test]
	fn recognizes_literals() {
		assert!(is_literal("\"a\""));
		assert!(is_literal("r#\"a\"#"));
		assert!(is_literal("true"));
		assert!(!is_literal("a"));
		assert!(!is_literal("\"a\".to_owned()"));
	}
}
