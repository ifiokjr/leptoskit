use std::borrow::Cow;

use markdown::Constructs;
use markdown::ParseOptions;
use markdown::mdast::Node;
use markdown::message::Place;

use crate::Diagnostic;

/// A parsed Markdown document, ready to [`render`](crate::render).
///
/// The dialect is GitHub-flavoured Markdown with MDX's JSX: tables, task
/// lists, strikethrough, footnotes, and autolink literals, plus elements
/// written as tags. A tag whose name starts with a capital letter, such as
/// `<Callout kind="tip">`, is a component looked up in
/// [`Components`](crate::Components); a lowercase tag such as `<figure>` is a
/// plain HTML element.
///
/// Like MDX, raw HTML is parsed as JSX, so every tag must be closed (`<br />`,
/// not `<br>`). HTML comments are allowed, as in mdBook, and dropped; tools
/// use them to mark generated regions. Unlike MDX, there is no
/// JavaScript: `import`/`export` lines are ordinary text, and braces in prose
/// are literal characters. Braces only mean something in an attribute value,
/// where `count={3}` passes the text `3` to the component.
pub struct Document {
	pub(crate) root: Node,
}

impl Document {
	/// Parses `source`.
	///
	/// Fails with the position of the first syntax error, such as a tag that
	/// is never closed.
	pub fn parse(source: &str) -> Result<Self, Diagnostic> {
		markdown::to_mdast(&without_comments(source), &options())
			.map(|root| Self { root })
			.map_err(|message| {
				let (line, column) = match message.place.as_deref() {
					Some(Place::Point(point)) => (point.line, point.column),
					Some(Place::Position(position)) => (position.start.line, position.start.column),
					None => (0, 0),
				};

				Diagnostic::new(line, column, message.reason)
			})
	}

	/// The document as plain text, without markup.
	pub fn plain_text(&self) -> String {
		self.root.to_string()
	}
}

/// `source` with every HTML comment outside code blanked to spaces.
///
/// MDX has no comments, so they are found first by parsing as `CommonMark`,
/// where they are raw HTML and never part of a code span or block. Blanking
/// keeps every line and every byte offset, so positions in later diagnostics
/// still point at the original source. An unterminated comment is left for
/// the MDX parser to report.
fn without_comments(source: &str) -> Cow<'_, str> {
	if !source.contains("<!--") {
		return Cow::Borrowed(source);
	}

	fn raw_html(node: &Node, ranges: &mut Vec<(usize, usize)>) {
		if let (Node::Html(_), Some(position)) = (node, node.position()) {
			ranges.push((position.start.offset, position.end.offset));
		}

		for child in node.children().into_iter().flatten() {
			raw_html(child, ranges);
		}
	}

	let commonmark = ParseOptions {
		constructs: Constructs::gfm(),
		..ParseOptions::gfm()
	};
	let root = markdown::to_mdast(source, &commonmark).expect("Markdown without MDX always parses");
	let mut ranges = Vec::new();
	raw_html(&root, &mut ranges);

	let mut bytes = source.as_bytes().to_vec();

	for (start, end) in ranges {
		let mut cursor = start;

		while let Some(open) = source[cursor..end].find("<!--") {
			let open = cursor + open;
			let Some(close) = source[open + 4..end].find("-->") else {
				break;
			};
			let close = open + 4 + close + 3;

			for byte in &mut bytes[open..close] {
				if *byte != b'\n' {
					*byte = b' ';
				}
			}

			cursor = close;
		}
	}

	Cow::Owned(
		String::from_utf8(bytes)
			.expect("only whole comments, which start and end in ASCII, were blanked"),
	)
}

/// GFM with MDX's JSX in place of raw HTML.
///
/// Indented code is off as in MDX: component children are indented for
/// readability, and fenced code is the only way to write a code block.
fn options() -> ParseOptions {
	ParseOptions {
		constructs: Constructs {
			code_indented: false,
			html_flow: false,
			html_text: false,
			mdx_jsx_flow: true,
			mdx_jsx_text: true,
			..Constructs::gfm()
		},
		..ParseOptions::gfm()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_components_and_html_as_jsx() {
		let document =
			Document::parse("<Callout kind=\"tip\">\n\nHello <br /> world\n\n</Callout>\n")
				.unwrap();

		let Node::Root(root) = &document.root else {
			panic!("expected a root");
		};
		assert!(
			matches!(&root.children[0], Node::MdxJsxFlowElement(element) if element.name.as_deref() == Some("Callout"))
		);
		assert_eq!(document.plain_text(), "Hello  world");
	}

	#[test]
	fn keeps_braces_in_prose_as_text() {
		let document = Document::parse("Use `{}` or {braces} freely.").unwrap();

		assert_eq!(document.plain_text(), "Use {} or {braces} freely.");
	}

	#[test]
	fn reports_where_a_tag_is_left_open() {
		let error = Document::parse("Intro\n\n<Callout>\n\nNever closed.\n")
			.err()
			.unwrap();

		assert_eq!(error.line, 6);
		assert!(error.message.contains("Callout"), "{}", error.message);
	}

	#[test]
	fn drops_html_comments_outside_code() {
		let document = Document::parse(
			"<!-- BEGIN GENERATED -->\n\n| a |\n| - |\n| 1 |\n\n<!-- END -->\n\nInline <!-- gone \
			 --> text and `<!-- kept -->`.\n\n```html\n<!-- kept too -->\n```\n",
		)
		.unwrap();

		let text = document.plain_text();
		assert_eq!(
			text.split_whitespace().collect::<Vec<_>>().join(" "),
			"a1Inline text and <!-- kept -->.<!-- kept too -->"
		);
	}

	#[test]
	fn keeps_positions_after_a_comment() {
		let error = Document::parse("<!-- a\ncomment -->\n\n<Open>\n")
			.err()
			.unwrap();

		assert_eq!(error.line, 5);
	}

	#[test]
	fn reports_unterminated_comments() {
		let error = Document::parse("<!-- never closed").err().unwrap();

		assert_eq!((error.line, error.column), (1, 2));
	}
}
