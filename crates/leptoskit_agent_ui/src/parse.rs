//! Parsing a document, including one an agent is still writing.

use markdown::Constructs;
use markdown::ParseOptions;
use markdown::mdast::Node;
use markdown::message::Place;

use crate::Diagnostic;

/// How many shorter prefixes a streaming document tries before giving up.
const MAX_REPAIRS: usize = 8;

/// A parsed document.
pub(crate) struct Parsed {
	pub(crate) root: Node,
	/// Whether the end of the source was left out because it did not parse
	/// yet, as while an agent is still writing it.
	pub(crate) partial: bool,
}

/// The dialect: GitHub-flavoured Markdown, with components written as tags
/// and formulas written in braces. Raw HTML is not Markdown here: a tag is a
/// component or nothing.
fn options() -> ParseOptions {
	ParseOptions {
		constructs: Constructs {
			code_indented: false,
			html_flow: false,
			html_text: false,
			mdx_jsx_flow: true,
			mdx_jsx_text: true,
			mdx_expression_flow: true,
			mdx_expression_text: true,
			..Constructs::gfm()
		},
		..ParseOptions::gfm()
	}
}

fn diagnostic(message: &markdown::message::Message) -> Diagnostic {
	let (line, column) = match message.place.as_deref() {
		Some(Place::Point(point)) => (point.line, point.column),
		Some(Place::Position(position)) => (position.start.line, position.start.column),
		None => (0, 0),
	};

	Diagnostic {
		line,
		column,
		message: message.reason.clone(),
	}
}

/// Parses `source`. When it does not parse, as while an agent is still
/// streaming it, renders the longest leading run of whole blocks that does,
/// with the tags it left open closed.
pub(crate) fn parse(source: &str) -> Result<Parsed, Diagnostic> {
	let options = options();
	let first_error = match markdown::to_mdast(source, &options) {
		Ok(root) => {
			return Ok(Parsed {
				root,
				partial: false,
			});
		}
		Err(message) => diagnostic(&message),
	};

	let mut end = source.len();
	for _ in 0..MAX_REPAIRS {
		let repaired = close_open_tags(&source[..end]);
		if let Ok(root) = markdown::to_mdast(&repaired, &options) {
			return Ok(Parsed {
				root,
				partial: true,
			});
		}

		// Drop the block being written, back to the last blank line.
		let Some(cut) = source[..end].trim_end().rfind("\n\n") else {
			break;
		};
		end = cut;
	}

	Err(first_error)
}

/// `prefix` with a closing tag added for each block-level component it opens
/// and does not close yet, innermost first.
fn close_open_tags(prefix: &str) -> String {
	let mut open: Vec<&str> = Vec::new();

	for line in prefix.lines() {
		let line = line.trim();
		if let Some(name) = line.strip_prefix("</") {
			let name = tag_name(name);
			if let Some(position) = open.iter().rposition(|candidate| *candidate == name) {
				open.truncate(position);
			}
		} else if let Some(rest) = line.strip_prefix('<') {
			let name = tag_name(rest);
			let opens_block = !name.is_empty()
				&& line.ends_with('>')
				&& !line.ends_with("/>")
				&& !line.contains(&format!("</{name}>"));
			if opens_block {
				open.push(name);
			}
		}
	}

	let mut repaired = prefix.to_owned();
	for name in open.iter().rev() {
		repaired.push_str("\n\n</");
		repaired.push_str(name);
		repaired.push('>');
	}
	repaired.push('\n');
	repaired
}

fn tag_name(text: &str) -> &str {
	let end = text
		.find(|character: char| {
			!(character.is_alphanumeric() || character == '.' || character == '_')
		})
		.unwrap_or(text.len());
	&text[..end]
}

#[cfg(test)]
mod tests {
	use super::*;

	fn top_level(source: &str) -> (usize, bool) {
		let parsed = parse(source).unwrap();
		(parsed.root.children().map_or(0, Vec::len), parsed.partial)
	}

	#[test]
	fn parses_a_whole_document() {
		assert_eq!(
			top_level("Hello\n\n<Card>\n\nInside\n\n</Card>\n"),
			(2, false)
		);
	}

	#[test]
	fn renders_what_has_streamed_so_far() {
		// An attribute still being written.
		assert_eq!(top_level("Hello\n\nWorld\n\n<Card title=\"x"), (2, true));
		// A formula still being written.
		assert_eq!(top_level("Hello\n\nRent is {rent *"), (1, true));
	}

	#[test]
	fn closes_components_that_are_still_open() {
		let parsed = parse("Intro\n\n<Card title=\"Plan\">\n\nFirst line\n\nSecond li").unwrap();
		assert!(parsed.partial);
		let children = parsed.root.children().unwrap();
		assert_eq!(children.len(), 2);
		let Node::MdxJsxFlowElement(card) = &children[1] else {
			panic!("expected the card");
		};
		assert_eq!(card.name.as_deref(), Some("Card"));
		// The line still being written shows too.
		assert_eq!(card.children.len(), 2);
	}

	#[test]
	fn reports_a_document_that_never_parses() {
		assert!(parse("<Card title=\"x").is_err());
	}
}
