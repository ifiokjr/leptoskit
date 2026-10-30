//! A Rust lexer that is just precise enough to find macro calls, comments, and
//! literals without parsing the file.
//!
//! The formatter never needs a syntax tree of the surrounding Rust: rustfmt
//! owns that. It only needs to know where the markup macros are, which bytes
//! are comments, and which lines begin inside a string literal (and so must not
//! be re-indented). Scanning tokens for that is linear and allocation-free,
//! where `syn::parse_file` would build a tree of the whole file only to throw
//! it away.

use std::ops::Range;

/// A coarse Rust token. Whitespace is skipped; everything the formatter does
/// not distinguish is a [`Token::Punct`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Token {
	/// An identifier, keyword, or number.
	Ident,
	/// A string, byte string, C string, raw string, or character literal.
	Literal,
	/// A lifetime or loop label, such as `'a`.
	Lifetime,
	/// A `//` comment, including doc comments.
	LineComment,
	/// A `/* */` comment, which may nest.
	BlockComment,
	/// An opening `(`, `[`, or `{`.
	Open(u8),
	/// A closing `)`, `]`, or `}`.
	Close(u8),
	/// Any other single byte of punctuation.
	Punct(u8),
}

/// Yields the tokens of a Rust source text with their byte ranges.
pub(crate) struct Lexer<'a> {
	src: &'a [u8],
	pos: usize,
}

impl<'a> Lexer<'a> {
	pub(crate) fn new(src: &'a str) -> Self {
		Self {
			src: src.as_bytes(),
			pos: 0,
		}
	}

	fn peek(&self, offset: usize) -> Option<u8> {
		self.src.get(self.pos + offset).copied()
	}

	/// Consumes a quoted body up to and including the closing `"`, honouring
	/// backslash escapes.
	fn quoted(&mut self) {
		while let Some(byte) = self.peek(0) {
			self.pos += 1;
			match byte {
				b'\\' => self.pos += 1,
				b'"' => return,
				_ => {}
			}
		}
	}

	/// Consumes a raw string body after its opening quote: everything up to a
	/// `"` followed by `hashes` hash marks.
	fn raw_quoted(&mut self, hashes: usize) {
		while let Some(byte) = self.peek(0) {
			self.pos += 1;
			if byte == b'"' && (0..hashes).all(|offset| self.peek(offset) == Some(b'#')) {
				self.pos += hashes;
				return;
			}
		}
	}

	fn block_comment(&mut self) {
		let mut depth = 0usize;
		while self.pos < self.src.len() {
			if self.peek(0) == Some(b'/') && self.peek(1) == Some(b'*') {
				depth += 1;
				self.pos += 2;
			} else if self.peek(0) == Some(b'*') && self.peek(1) == Some(b'/') {
				depth -= 1;
				self.pos += 2;
				if depth == 0 {
					return;
				}
			} else {
				self.pos += 1;
			}
		}
	}

	/// After an identifier, decides whether it was the prefix of a string
	/// literal (`b"…"`, `r#"…"#`, `cr"…"`) and consumes the literal if so.
	fn literal_after_prefix(&mut self, start: usize) -> bool {
		let prefix = &self.src[start..self.pos];
		match (prefix, self.peek(0)) {
			(b"b" | b"c", Some(b'"')) => {
				self.pos += 1;
				self.quoted();
				true
			}
			(b"b", Some(b'\'')) => {
				self.pos += 1;
				self.char_body();
				true
			}
			(b"r" | b"br" | b"cr", Some(b'"' | b'#')) => {
				let hashes = self.src[self.pos..]
					.iter()
					.take_while(|byte| **byte == b'#')
					.count();
				if self.peek(hashes) != Some(b'"') {
					// `r#ident`: a raw identifier, not a string.
					return false;
				}
				self.pos += hashes + 1;
				self.raw_quoted(hashes);
				true
			}
			_ => false,
		}
	}

	/// Consumes a character literal body after its opening quote.
	fn char_body(&mut self) {
		if self.peek(0) == Some(b'\\') {
			self.pos += 2;
		} else {
			self.pos += utf8_len(self.peek(0).unwrap_or(0));
		}
		while let Some(byte) = self.peek(0) {
			self.pos += 1;
			if byte == b'\'' {
				return;
			}
		}
	}

	fn ident_tail(&mut self) {
		while self.peek(0).is_some_and(is_ident_byte) {
			self.pos += 1;
		}
	}
}

impl Iterator for Lexer<'_> {
	type Item = (Token, Range<usize>);

	fn next(&mut self) -> Option<Self::Item> {
		while self.peek(0).is_some_and(|byte| byte.is_ascii_whitespace()) {
			self.pos += 1;
		}
		let start = self.pos;
		let byte = self.peek(0)?;
		self.pos += 1;
		let token = match byte {
			b'/' if self.peek(0) == Some(b'/') => {
				while self.peek(0).is_some_and(|byte| byte != b'\n') {
					self.pos += 1;
				}
				Token::LineComment
			}
			b'/' if self.peek(0) == Some(b'*') => {
				self.pos = start;
				self.block_comment();
				Token::BlockComment
			}
			b'"' => {
				self.quoted();
				Token::Literal
			}
			b'\'' => {
				// `'a'` and `'\n'` are characters; `'a` and `'label` are not.
				let is_char = self.peek(0) == Some(b'\\') || {
					let len = utf8_len(self.peek(0).unwrap_or(0));
					self.peek(len) == Some(b'\'')
				};
				if is_char {
					self.char_body();
					Token::Literal
				} else {
					self.ident_tail();
					Token::Lifetime
				}
			}
			b'(' | b'[' | b'{' => Token::Open(byte),
			b')' | b']' | b'}' => Token::Close(byte),
			byte if is_ident_byte(byte) => {
				self.ident_tail();
				if self.literal_after_prefix(start) {
					Token::Literal
				} else {
					if self.peek(0) == Some(b'#') && &self.src[start..self.pos] == b"r" {
						// Raw identifier: `r#type`.
						self.pos += 1;
						self.ident_tail();
					}
					Token::Ident
				}
			}
			byte => Token::Punct(byte),
		};
		Some((token, start..self.pos.min(self.src.len())))
	}
}

fn is_ident_byte(byte: u8) -> bool {
	byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

fn utf8_len(first: u8) -> usize {
	match first {
		0xf0..=0xff => 4,
		0xe0..=0xef => 3,
		0xc0..=0xdf => 2,
		_ => 1,
	}
}

/// The delimiter of a macro call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Delimiter {
	/// `view! { … }`
	Brace,
	/// `view!( … )`
	Paren,
	/// `view![ … ]`
	Bracket,
}

impl Delimiter {
	fn from_open(byte: u8) -> Self {
		match byte {
			b'(' => Self::Paren,
			b'[' => Self::Bracket,
			_ => Self::Brace,
		}
	}

	pub(crate) fn close(self) -> char {
		match self {
			Self::Brace => '}',
			Self::Paren => ')',
			Self::Bracket => ']',
		}
	}
}

/// A call to one of the configured markup macros.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MacroCall {
	/// From the first byte of the path to just past the closing delimiter.
	pub range: Range<usize>,
	/// The path as written, without whitespace: `view` or `leptos::view`.
	pub path: String,
	pub delimiter: Delimiter,
	/// Between the delimiters.
	pub body: Range<usize>,
}

/// Finds the calls to the macros named in `names`, outermost first and in
/// source order. Calls nested inside a found call's body are not reported:
/// the formatter reaches them through the Rust expressions that contain them.
///
/// Items marked `#[rustfmt::skip]` (and blocks marked `#![rustfmt::skip]`) are
/// skipped, so code its author froze stays frozen.
pub(crate) fn find_macros(src: &str, names: &[String]) -> Vec<MacroCall> {
	let mut calls = Vec::new();
	// Most Rust inside markup names no macro at all; a substring search rules
	// that out without lexing.
	let mentioned = names
		.iter()
		.any(|name| src.contains(name.rsplit("::").next().unwrap_or(name)));
	if !mentioned || !src.contains('!') {
		return calls;
	}
	let tokens: Vec<(Token, Range<usize>)> = Lexer::new(src)
		.filter(|(token, _)| !matches!(token, Token::LineComment | Token::BlockComment))
		.collect();
	let mut index = 0;
	while index < tokens.len() {
		match tokens[index].0 {
			Token::Punct(b'#') => {
				if let Some(next) = skip_attribute_target(src, &tokens, index) {
					index = next;
					continue;
				}
			}
			Token::Ident => {
				let after_colons = index >= 2 && is_path_separator(&tokens, index - 2);
				let continues_path =
					after_colons && index >= 3 && tokens[index - 3].0 == Token::Ident;
				// `a::view` is tried from `a`; a leading `::` belongs to the
				// path.
				let path_start = if after_colons { index - 2 } else { index };
				if !continues_path
					&& let Some((call, next)) = macro_at(src, &tokens, path_start, index, names)
				{
					calls.push(call);
					index = next;
					continue;
				}
			}
			_ => {}
		}
		index += 1;
	}
	calls
}

/// Whether tokens `index` and `index + 1` are an adjacent `::`.
fn is_path_separator(tokens: &[(Token, Range<usize>)], index: usize) -> bool {
	matches!(
		(tokens.get(index), tokens.get(index + 1)),
		(Some((Token::Punct(b':'), first)), Some((Token::Punct(b':'), second)))
			if first.end == second.start
	)
}

/// Reads the path starting at token `first_ident` and, if a configured macro
/// call follows it, returns the call and the index of the first token after
/// it.
fn macro_at(
	src: &str,
	tokens: &[(Token, Range<usize>)],
	path_start: usize,
	first_ident: usize,
	names: &[String],
) -> Option<(MacroCall, usize)> {
	let mut path = String::new();
	if path_start < first_ident {
		path.push_str("::");
	}
	let mut index = first_ident;
	loop {
		let (Token::Ident, range) = &tokens.get(index)? else {
			return None;
		};
		path.push_str(&src[range.clone()]);
		index += 1;
		if is_path_separator(tokens, index) {
			path.push_str("::");
			index += 2;
		} else if tokens.get(index)?.0 == Token::Punct(b'!') {
			break;
		} else {
			return None;
		}
	}
	let (Token::Open(open), open_range) = tokens.get(index + 1)? else {
		return None;
	};
	if !names.contains(&path) {
		return None;
	}
	let close = matching_close(tokens, index + 1)?;
	let call = MacroCall {
		range: tokens[path_start].1.start..tokens[close].1.end,
		path,
		delimiter: Delimiter::from_open(*open),
		body: open_range.end..tokens[close].1.start,
	};
	Some((call, close + 1))
}

/// The index of the token closing the group opened at `open`.
fn matching_close(tokens: &[(Token, Range<usize>)], open: usize) -> Option<usize> {
	let mut depth = 0usize;
	for (index, (token, _)) in tokens.iter().enumerate().skip(open) {
		match token {
			Token::Open(_) => depth += 1,
			Token::Close(_) => {
				depth -= 1;
				if depth == 0 {
					return Some(index);
				}
			}
			_ => {}
		}
	}
	None
}

/// If the `#` at `hash` starts a `rustfmt::skip` attribute, returns the index
/// of the first token after whatever the attribute covers.
fn skip_attribute_target(
	src: &str,
	tokens: &[(Token, Range<usize>)],
	hash: usize,
) -> Option<usize> {
	let inner = tokens.get(hash + 1)?.0 == Token::Punct(b'!');
	let open = hash + 1 + usize::from(inner);
	if tokens.get(open)?.0 != Token::Open(b'[') {
		return None;
	}
	let close = matching_close(tokens, open)?;
	let attribute: String = tokens[open + 1..close]
		.iter()
		.map(|(_, range)| &src[range.clone()])
		.collect();
	if !attribute.starts_with("rustfmt::skip") {
		return None;
	}
	let mut depth = 0isize;
	for (index, (token, _)) in tokens.iter().enumerate().skip(close + 1) {
		match token {
			Token::Open(_) => depth += 1,
			Token::Close(close) => {
				depth -= 1;
				// An inner attribute covers the rest of its enclosing block; an
				// outer one ends with the item's body, its first braces.
				if depth < 0 || (!inner && depth == 0 && *close == b'}') {
					return Some(index + 1);
				}
			}
			Token::Punct(b';') if depth == 0 && !inner => return Some(index + 1),
			_ => {}
		}
	}
	Some(tokens.len())
}

/// A comment found between markup tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Comment {
	pub range: Range<usize>,
	pub block: bool,
}

/// The comments in `range`, which must lie between tokens (so it holds
/// nothing but whitespace and comments).
pub(crate) fn comments_in(src: &str, range: Range<usize>) -> Vec<Comment> {
	let offset = range.start;
	Lexer::new(&src[range])
		.filter_map(|(token, range)| {
			let block = match token {
				Token::LineComment => false,
				Token::BlockComment => true,
				_ => return None,
			};
			Some(Comment {
				range: range.start + offset..range.end + offset,
				block,
			})
		})
		.collect()
}

/// For each line of `text`, whether it begins inside a string literal. Those
/// lines are literal content: re-indenting them would change the string.
pub(crate) fn verbatim_lines(text: &str) -> Vec<bool> {
	let mut verbatim = vec![false; text.split('\n').count()];
	let mut line = 0;
	let mut counted = 0;
	for (token, range) in Lexer::new(text) {
		if token != Token::Literal {
			continue;
		}
		line += text[counted..range.start].matches('\n').count();
		let inner_lines = text[range.clone()].matches('\n').count();
		for flag in verbatim.iter_mut().skip(line + 1).take(inner_lines) {
			*flag = true;
		}
		line += inner_lines;
		counted = range.end;
	}
	verbatim
}

#[cfg(test)]
mod tests {
	use super::*;

	fn names() -> Vec<String> {
		vec!["view".into(), "leptos::view".into()]
	}

	fn found(src: &str) -> Vec<(String, &str)> {
		find_macros(src, &names())
			.into_iter()
			.map(|call| (call.path, &src[call.body]))
			.collect()
	}

	#[test]
	fn finds_configured_macros_by_full_path() {
		let src = r"fn a() { view! { <a/> }; leptos::view!(<b/>); other::view! { <c/> }; html! { <d/> } }";
		assert_eq!(
			found(src),
			vec![
				("view".to_owned(), " <a/> "),
				("leptos::view".to_owned(), "<b/>"),
			]
		);
	}

	#[test]
	fn ignores_macros_in_comments_and_strings() {
		let src = r##"
			// view! { <a/> }
			/* view! { /* nested */ <b/> } */
			let s = "view! { <c/> }";
			let r = r#"view! { <d/> }"#;
			let c = '"';
			let l: &'static str = "x";
			view! { <e/> }
		"##;
		assert_eq!(found(src), vec![("view".to_owned(), " <e/> ")]);
	}

	#[test]
	fn reports_only_the_outermost_call() {
		let src = "view! { <a>{view! { <b/> }}</a> }";
		assert_eq!(found(src).len(), 1);
	}

	#[test]
	fn skips_rustfmt_skip_items() {
		let src = "#[rustfmt::skip]\nfn a() { view! { <a/> } }\nfn b() { view! { <b/> } }";
		assert_eq!(found(src), vec![("view".to_owned(), " <b/> ")]);
		let inner = "mod m { #![rustfmt::skip] fn a() { view! { <a/> } } }\nview! { <b/> }";
		assert_eq!(found(inner), vec![("view".to_owned(), " <b/> ")]);
	}

	#[test]
	fn marks_lines_inside_string_literals() {
		let text = "a(\"x\n  y\n\", r#\"\nz\"#)\nb";
		assert_eq!(verbatim_lines(text), vec![false, true, true, true, false]);
	}

	#[test]
	fn collects_comments_in_a_gap() {
		let src = "  // one\n /* two */ ";
		let comments = comments_in(src, 0..src.len());
		assert_eq!(
			comments
				.iter()
				.map(|comment| &src[comment.range.clone()])
				.collect::<Vec<_>>(),
			vec!["// one", "/* two */"]
		);
	}
}
