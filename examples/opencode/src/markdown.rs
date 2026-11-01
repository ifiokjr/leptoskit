//! The markdown an assistant writes, rendered as opencode renders it:
//! paragraphs with inline code and bold, bullet lists, and highlighted code
//! fences, each block a `Text` of styled runs.

use leptoskit::prelude::*;
use leptoskit_native::FontWeight;
use leptoskit_native::TextSpan;

use crate::highlight;
use crate::theme;

/// One block per paragraph, list item, and code line, in order. Callers
/// space paragraphs and lists apart; consecutive list items and code lines
/// arrive grouped so they stack without gaps.
pub fn blocks(source: &str) -> Vec<Vec<Element>> {
	let mut blocks = Vec::new();
	let mut lines = source.lines().peekable();

	while let Some(line) = lines.next() {
		if line.trim().is_empty() {
			continue;
		}

		if let Some(language) = line.strip_prefix("```") {
			let mut code = Vec::new();
			for line in lines.by_ref() {
				if line.starts_with("```") {
					break;
				}
				code.push(view! { <CodeLine language=language line=line /> });
			}
			blocks.push(code);
		} else if line.starts_with("- ") {
			let mut items = vec![view! { <Bullet line=line /> }];
			while let Some(next) = lines.next_if(|next| next.starts_with("- ")) {
				items.push(view! { <Bullet line=next /> });
			}
			blocks.push(items);
		} else {
			let mut paragraph = line.to_owned();
			while let Some(next) = lines.next_if(|next| {
				!next.trim().is_empty() && !next.starts_with("- ") && !next.starts_with("```")
			}) {
				paragraph.push(' ');
				paragraph.push_str(next.trim());
			}
			blocks.push(vec![view! { <Text spans=inline(&paragraph) /> }]);
		}
	}

	blocks
}

#[component]
fn Bullet(line: &str) -> Element {
	let mut spans = vec![TextSpan::new("- ").color(theme::PRIMARY)];
	spans.extend(inline(&line[2..]));
	view! { <Text spans=spans /> }
}

#[component]
fn CodeLine(language: &str, line: &str) -> Element {
	let spans = if language == "rust" {
		highlight::rust(line)
	} else {
		vec![TextSpan::new(line.replace(' ', &crate::NBSP.to_string()))]
	};
	view! { <Text spans=spans /> }
}

/// Inline markup: `code` and **bold**; everything else is body text.
pub fn inline(text: &str) -> Vec<TextSpan> {
	let mut spans = Vec::new();
	let mut rest = text;

	while !rest.is_empty() {
		let code = rest.find('`');
		let bold = rest.find("**");
		let (start, marker) = match (code, bold) {
			(Some(code), Some(bold)) if bold < code => (bold, "**"),
			(Some(code), _) => (code, "`"),
			(None, Some(bold)) => (bold, "**"),
			(None, None) => {
				spans.push(TextSpan::new(rest).color(theme::TEXT));
				break;
			}
		};

		let after = &rest[start + marker.len()..];
		let Some(end) = after.find(marker) else {
			spans.push(TextSpan::new(rest).color(theme::TEXT));
			break;
		};

		if start > 0 {
			spans.push(TextSpan::new(&rest[..start]).color(theme::TEXT));
		}

		let content = &after[..end];
		spans.push(if marker == "`" {
			TextSpan::new(content).color(theme::SUCCESS)
		} else {
			TextSpan::new(content)
				.color(theme::WARNING)
				.weight(FontWeight::Bold)
		});
		rest = &after[end + marker.len()..];
	}

	spans
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn splits_code_and_bold_runs() {
		let spans = inline("Fixed **src/timer.rs**: use `%` here.");
		let runs: Vec<_> = spans
			.iter()
			.map(|span| (span.text.as_str(), span.color))
			.collect();
		assert_eq!(
			runs,
			vec![
				("Fixed ", Some(theme::TEXT)),
				("src/timer.rs", Some(theme::WARNING)),
				(": use ", Some(theme::TEXT)),
				("%", Some(theme::SUCCESS)),
				(" here.", Some(theme::TEXT)),
			]
		);
	}

	#[test]
	fn leaves_unclosed_markers_as_text() {
		let spans = inline("a `b");
		assert_eq!(spans.len(), 1);
		assert_eq!(spans[0].text, "a `b");
	}

	#[test]
	fn groups_lists_and_code() {
		let blocks =
			blocks("Intro\nline two\n\n- one\n- two\n\n```rust\nlet a = 1;\nlet b = 2;\n```");
		let sizes: Vec<_> = blocks.iter().map(Vec::len).collect();
		assert_eq!(sizes, vec![1, 2, 2]);
		assert_eq!(blocks[0][0].props.text.as_deref(), Some("Intro line two"));
	}
}
