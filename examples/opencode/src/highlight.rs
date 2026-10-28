//! A small Rust highlighter that colors code the way opencode's theme does.
//!
//! It works one line at a time and knows only what a snippet in a chat
//! needs: keywords, bindings, calls, fields, types, literals, and operators.

use leptoskit_native::Color;
use leptoskit_native::TextSpan;

use crate::NBSP;
use crate::theme;

const KEYWORDS: &[&str] = &[
	"as", "async", "await", "break", "const", "continue", "crate", "else", "enum", "for", "if",
	"impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static",
	"struct", "super", "trait", "type", "use", "where", "while",
];

/// One line of Rust as styled runs. Spaces become no-break spaces so
/// indentation survives wrapping.
pub fn rust(line: &str) -> Vec<TextSpan> {
	let chars: Vec<char> = line.chars().collect();
	let mut spans = Vec::new();
	let mut previous_word = String::new();
	let mut previous_symbol = None;
	let mut index = 0;

	while index < chars.len() {
		let start = index;
		let symbol = chars[index];

		let color = if symbol == ' ' {
			while index < chars.len() && chars[index] == ' ' {
				index += 1;
			}
			spans.push(TextSpan::new(NBSP.to_string().repeat(index - start)));
			continue;
		} else if symbol == '"' {
			index += 1;
			while index < chars.len() && chars[index] != '"' {
				index += if chars[index] == '\\' { 2 } else { 1 };
			}
			index = (index + 1).min(chars.len());
			Some(theme::SUCCESS)
		} else if symbol.is_ascii_digit() {
			while index < chars.len()
				&& (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
			{
				index += 1;
			}
			Some(theme::WARNING)
		} else if symbol.is_alphabetic() || symbol == '_' {
			while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
				index += 1;
			}
			let word: String = chars[start..index].iter().collect();
			let next = chars[index..]
				.iter()
				.find(|symbol| **symbol != ' ')
				.copied();
			let macro_call = chars.get(index) == Some(&'!');

			if macro_call {
				index += 1;
			}

			let style = word_style(&word, &previous_word, previous_symbol, next, macro_call);
			previous_word = word;
			previous_symbol = None;
			push(&mut spans, &chars[start..index], style);
			continue;
		} else if "(){}[],".contains(symbol) {
			index += 1;
			None
		} else {
			index += 1;
			// Two-character operators stay one run.
			if matches!(
				(symbol, chars.get(index)),
				('-' | '=', Some('>')) | (':', Some(':'))
			) {
				index += 1;
			}
			Some(theme::INFO)
		};

		previous_symbol = Some(symbol);
		push(
			&mut spans,
			&chars[start..index],
			Style {
				color,
				italic: false,
			},
		);
	}

	spans
}

#[derive(Clone, Copy)]
struct Style {
	color: Option<Color>,
	italic: bool,
}

fn word_style(
	word: &str,
	previous_word: &str,
	previous_symbol: Option<char>,
	next: Option<char>,
	macro_call: bool,
) -> Style {
	let plain = |color| {
		Style {
			color: Some(color),
			italic: false,
		}
	};

	if macro_call || (KEYWORDS.contains(&word) && word != "fn") {
		return Style {
			color: Some(theme::ACCENT),
			italic: true,
		};
	}

	match word {
		"fn" => plain(theme::PRIMARY),
		"self" | "Self" => plain(theme::ERROR),
		"true" | "false" => plain(theme::WARNING),
		_ if previous_symbol == Some('.') || previous_word == "fn" || next == Some('(') => {
			plain(theme::PRIMARY)
		}
		_ if previous_symbol.is_none() && matches!(previous_word, "let" | "mut") => {
			plain(theme::WARNING)
		}
		_ if word.starts_with(char::is_uppercase) => plain(theme::TYPE),
		_ => {
			Style {
				color: None,
				italic: false,
			}
		}
	}
}

fn push(spans: &mut Vec<TextSpan>, text: &[char], style: Style) {
	let mut span = TextSpan::new(text.iter().collect::<String>());
	span.color = style.color;
	if style.italic {
		span = span.italic();
	}
	spans.push(span);
}

#[cfg(test)]
mod tests {
	use super::*;

	fn colors(line: &str) -> Vec<(String, Option<Color>)> {
		rust(line)
			.into_iter()
			.filter(|span| !span.text.chars().all(|symbol| symbol == NBSP))
			.map(|span| (span.text, span.color))
			.collect()
	}

	#[test]
	fn colors_a_binding_like_the_theme() {
		assert_eq!(
			colors("let seconds = self.seconds % 60;"),
			vec![
				("let".to_owned(), Some(theme::ACCENT)),
				("seconds".to_owned(), Some(theme::WARNING)),
				("=".to_owned(), Some(theme::INFO)),
				("self".to_owned(), Some(theme::ERROR)),
				(".".to_owned(), Some(theme::INFO)),
				("seconds".to_owned(), Some(theme::PRIMARY)),
				("%".to_owned(), Some(theme::INFO)),
				("60".to_owned(), Some(theme::WARNING)),
				(";".to_owned(), Some(theme::INFO)),
			]
		);
	}

	#[test]
	fn colors_signatures_macros_and_strings() {
		assert_eq!(
			colors("pub fn label(&self) -> String {"),
			vec![
				("pub".to_owned(), Some(theme::ACCENT)),
				("fn".to_owned(), Some(theme::PRIMARY)),
				("label".to_owned(), Some(theme::PRIMARY)),
				("(".to_owned(), None),
				("&".to_owned(), Some(theme::INFO)),
				("self".to_owned(), Some(theme::ERROR)),
				(")".to_owned(), None),
				("->".to_owned(), Some(theme::INFO)),
				("String".to_owned(), Some(theme::TYPE)),
				("{".to_owned(), None),
			]
		);
		assert_eq!(
			colors(r#"format!("{minutes}")"#),
			vec![
				("format!".to_owned(), Some(theme::ACCENT)),
				("(".to_owned(), None),
				(r#""{minutes}""#.to_owned(), Some(theme::SUCCESS)),
				(")".to_owned(), None),
			]
		);
	}

	#[test]
	fn keeps_indentation_as_no_break_spaces() {
		let spans = rust("    }");
		assert_eq!(spans[0].text, NBSP.to_string().repeat(4));
	}
}
