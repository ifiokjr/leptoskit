use markdown::ParseOptions;
use markdown::mdast::Node;

use crate::Diagnostic;

/// A book's table of contents, read from an mdBook `SUMMARY.md`.
///
/// ```markdown
/// # Summary
///
/// [Introduction](readme.md)
///
/// # Guide
///
/// - [Installation](guide/installation.md)
///   - [On macOS](guide/macos.md)
/// - [A draft chapter]()
///
/// ---
///
/// [Contributing](contributing.md)
/// ```
///
/// The first `#` heading is the summary's own title and is skipped. Later
/// headings start parts, links in paragraphs are chapters outside any list,
/// nested lists are sub-chapters, a link without a target is a draft, and
/// `---` draws a separator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
	/// The parts, chapters, and separators, in order.
	pub items: Vec<SummaryItem>,
}

/// One entry of a [`Summary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryItem {
	/// A part title, such as `# Guide`, heading the chapters after it.
	Part(String),
	/// A chapter and its sub-chapters.
	Chapter(Chapter),
	/// A `---` line.
	Separator,
}

/// A chapter of a [`Summary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
	/// The link text.
	pub title: String,
	/// The Markdown source, relative to the summary, or `None` for a draft.
	/// Parsed paths use `/` separators without empty, `.` or `..` components,
	/// backslashes, or colons, so they stay relative on every supported host.
	pub path: Option<String>,
	/// The nested chapters.
	pub children: Vec<Chapter>,
}

impl Summary {
	/// Parses the text of a `SUMMARY.md`, or lists what does not fit the
	/// format.
	pub fn parse(source: &str) -> Result<Self, Vec<Diagnostic>> {
		let root = markdown::to_mdast(source, &ParseOptions::default())
			.expect("CommonMark without MDX always parses");
		let mut items = Vec::new();
		let mut diagnostics = Vec::new();
		let mut titled = false;

		for node in root.children().into_iter().flatten() {
			match node {
				Node::Heading(heading) if heading.depth == 1 => {
					if titled {
						items.push(SummaryItem::Part(node.to_string()));
					}
					titled = true;
				}
				Node::Paragraph(paragraph) => {
					let links = paragraph
						.children
						.iter()
						.filter(|child| matches!(child, Node::Link(_)))
						.count();

					if links == 0 {
						diagnostics.push(at(
							node,
							"expected a link to a chapter, such as `[Title](page.md)`",
						));
					}

					items.extend(paragraph.children.iter().filter_map(|child| {
						chapter_link(child, &mut diagnostics).map(|(title, path)| {
							SummaryItem::Chapter(Chapter {
								title,
								path,
								children: Vec::new(),
							})
						})
					}));
				}
				Node::List(_) => {
					items.extend(
						chapters(node, &mut diagnostics)
							.into_iter()
							.map(SummaryItem::Chapter),
					);
				}
				Node::ThematicBreak(_) => items.push(SummaryItem::Separator),
				_ => {
					diagnostics.push(at(
						node,
						"expected a part heading, a chapter link, a list of chapters, or `---`",
					));
				}
			}
		}

		if diagnostics.is_empty() {
			Ok(Self { items })
		} else {
			Err(diagnostics)
		}
	}

	/// Every chapter with a page, depth first, in reading order.
	pub fn pages(&self) -> Vec<&Chapter> {
		fn walk<'a>(chapter: &'a Chapter, pages: &mut Vec<&'a Chapter>) {
			if chapter.path.is_some() {
				pages.push(chapter);
			}

			for child in &chapter.children {
				walk(child, pages);
			}
		}

		let mut pages = Vec::new();
		for item in &self.items {
			if let SummaryItem::Chapter(chapter) = item {
				walk(chapter, &mut pages);
			}
		}

		pages
	}
}

fn chapters(list: &Node, diagnostics: &mut Vec<Diagnostic>) -> Vec<Chapter> {
	let mut found = Vec::new();

	for item in list.children().into_iter().flatten() {
		let mut parts = item.children().into_iter().flatten();
		let Some((title, path)) = parts.next().and_then(|first| {
			match first {
				Node::Paragraph(paragraph) if paragraph.children.len() == 1 => {
					chapter_link(&paragraph.children[0], diagnostics)
				}
				_ => None,
			}
		}) else {
			diagnostics.push(at(
				item,
				"a list item must hold one chapter link, such as `- [Title](page.md)`",
			));
			continue;
		};

		let mut children = Vec::new();
		for rest in parts {
			if matches!(rest, Node::List(_)) {
				children.extend(chapters(rest, diagnostics));
			} else {
				diagnostics.push(at(rest, "only a nested list can follow a chapter link"));
			}
		}

		found.push(Chapter {
			title,
			path,
			children,
		});
	}

	found
}

fn chapter_link(
	node: &Node,
	diagnostics: &mut Vec<Diagnostic>,
) -> Option<(String, Option<String>)> {
	let Node::Link(link) = node else {
		return None;
	};
	let path = (!link.url.is_empty()).then(|| link.url.clone());

	// Check both Unix and Windows forms even when parsing on a different host.
	// Windows also treats components made of dots and trailing spaces
	// specially.
	if !link.url.is_empty()
		&& (link.url.contains(['\\', ':'])
			|| link
				.url
				.split('/')
				.any(|part| part.trim_end_matches([' ', '.']).is_empty()))
	{
		diagnostics.push(at(
			node,
			&format!(
				"`{}` must be a relative chapter path with `/` separators and no empty, `.` or \
				 `..` components, backslashes, or colons",
				link.url
			),
		));
	}

	Some((node.to_string(), path))
}

fn at(node: &Node, message: &str) -> Diagnostic {
	let (line, column) = node.position().map_or((0, 0), |position| {
		(position.start.line, position.start.column)
	});
	Diagnostic::new(line, column, message)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn chapter(title: &str, path: Option<&str>, children: Vec<Chapter>) -> Chapter {
		Chapter {
			title: title.to_owned(),
			path: path.map(str::to_owned),
			children,
		}
	}

	#[test]
	fn reads_parts_chapters_drafts_and_separators() {
		let summary =
			Summary::parse(
				"# Summary\n\n[Introduction](readme.md)\n\n# Guide\n\n- \
				 [Install](guide/install.md)\n  - [On `macOS`](guide/macos.md)\n- \
				 [Draft]()\n\n---\n\n[Contributing](contributing.md)\n",
			)
			.unwrap();

		assert_eq!(
			summary.items,
			[
				SummaryItem::Chapter(chapter("Introduction", Some("readme.md"), vec![])),
				SummaryItem::Part("Guide".to_owned()),
				SummaryItem::Chapter(chapter(
					"Install",
					Some("guide/install.md"),
					vec![chapter("On macOS", Some("guide/macos.md"), vec![])]
				)),
				SummaryItem::Chapter(chapter("Draft", None, vec![])),
				SummaryItem::Separator,
				SummaryItem::Chapter(chapter("Contributing", Some("contributing.md"), vec![])),
			]
		);

		let pages: Vec<_> = summary
			.pages()
			.iter()
			.map(|page| page.path.as_deref().unwrap())
			.collect();
		assert_eq!(
			pages,
			[
				"readme.md",
				"guide/install.md",
				"guide/macos.md",
				"contributing.md"
			]
		);
	}

	#[test]
	fn rejects_unsafe_chapter_paths_at_every_depth() {
		for path in [
			"../outside.md",
			"guide/../../outside.md",
			"/outside.md",
			"//server/share/outside.md",
			"C:/outside.md",
			"C:outside.md",
			r"C:\outside.md",
			r"guide\..\outside.md",
			r"\\server\share\outside.md",
			r"guide/..\outside.md",
			"./readme.md",
			"guide/./page.md",
			"guide//page.md",
			"guide/",
			".. /outside.md",
			"guide/. /page.md",
		] {
			for source in [
				format!("[Unsafe](<{path}>)\n"),
				format!("- [Home](readme.md)\n  - [Unsafe](<{path}>)\n"),
			] {
				let diagnostics = Summary::parse(&source).expect_err(&source);

				assert_eq!(diagnostics.len(), 1, "{source}: {diagnostics:?}");
				assert!(diagnostics[0].message.contains("relative chapter path"));
				assert!(diagnostics[0].line > 0);
				assert!(diagnostics[0].column > 0);
			}
		}
	}

	#[test]
	fn needs_no_title() {
		let summary = Summary::parse("[Only](only.md)\n").unwrap();

		assert_eq!(
			summary.items,
			[SummaryItem::Chapter(chapter(
				"Only",
				Some("only.md"),
				vec![]
			))]
		);
	}

	#[test]
	fn lists_everything_out_of_place() {
		let diagnostics = Summary::parse(
			"# Summary\n\nJust prose.\n\n- Not a link\n- [Fine](fine.md)\n\n  Stray \
			 paragraph.\n\n## Subheading\n",
		)
		.unwrap_err();
		let lines: Vec<_> = diagnostics
			.iter()
			.map(|diagnostic| diagnostic.line)
			.collect();

		assert_eq!(lines, [3, 5, 8, 10]);
	}
}
