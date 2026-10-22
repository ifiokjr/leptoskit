//! A Wadler-style pretty printer, after prettier's document algebra.
//!
//! The layout code describes markup as a [`Doc`]: text, places a line may
//! break, and groups whose breaks are taken all together or not at all. The
//! printer then picks, left to right, the flattest layout that fits the line
//! width. That keeps every layout decision in one place, so the rules in
//! `layout.rs` only say *what may break*, never *whether it does*.

use std::ops::Range;
use std::rc::Rc;

use crate::config::Layout;

/// A place a line may break.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Line {
	/// Nothing when flat, a newline when broken.
	Soft,
	/// A space when flat, a newline when broken.
	Space,
	/// Always a newline; breaks every group around it.
	Hard,
	/// Always a newline without indentation: the next line is literal
	/// content, such as the inside of a multi-line string.
	Literal,
}

#[derive(Debug)]
pub(crate) enum Doc {
	/// Text without newlines, with its display width.
	Text(Box<str>, usize),
	Concat(Vec<Doc>),
	/// Breaks all of its lines or none of them. `broken` is set when it must
	/// break, which [`propagate_breaks`] works out from hard lines.
	Group {
		broken: bool,
		contents: Box<Doc>,
	},
	/// Alternating contents and separators: each separator breaks only when
	/// the content after it would not fit, so items pack onto lines like
	/// words in a paragraph.
	Fill(Vec<Doc>),
	/// One more level of indentation for the lines inside.
	Indent(Box<Doc>),
	/// Extra literal indentation for the lines inside.
	IndentBy(Box<str>, Box<Doc>),
	Line(Line),
	/// Records the output range the contents print to.
	Track(usize, Box<Doc>),
}

/// Builds documents for one layout, numbering tracked ranges.
pub(crate) struct Builder<'a> {
	pub layout: &'a Layout,
	tracks: usize,
}

impl<'a> Builder<'a> {
	pub(crate) fn new(layout: &'a Layout) -> Self {
		Self { layout, tracks: 0 }
	}

	pub(crate) fn text(&self, text: impl Into<Box<str>>) -> Doc {
		let text = text.into();
		debug_assert!(!text.contains('\n'), "text holds no newlines: {text:?}");
		let width = self.layout.width(&text);
		Doc::Text(text, width)
	}

	pub(crate) fn track_id(&mut self) -> usize {
		self.tracks += 1;
		self.tracks - 1
	}

	/// Prints `doc` starting at `start_column` on a line indented by
	/// `base_indent`.
	pub(crate) fn print(&self, mut doc: Doc, start_column: usize, base_indent: &str) -> Printed {
		propagate_breaks(&mut doc);
		Printer {
			layout: self.layout,
			unit: self.layout.indent_unit(),
			out: String::new(),
			column: start_column,
			pending_indent: None,
			tracks: vec![0..0; self.tracks],
		}
		.print(&doc, base_indent)
	}
}

pub(crate) fn group(contents: Doc) -> Doc {
	Doc::Group {
		broken: false,
		contents: Box::new(contents),
	}
}

/// A group that always breaks.
pub(crate) fn broken_group(contents: Doc) -> Doc {
	Doc::Group {
		broken: true,
		contents: Box::new(contents),
	}
}

pub(crate) fn indent(contents: Doc) -> Doc {
	Doc::Indent(Box::new(contents))
}

pub(crate) const fn line(line: Line) -> Doc {
	Doc::Line(line)
}

/// Marks every group holding a hard line, directly or through a broken
/// group, as broken. Returns whether `doc` forces a break.
fn propagate_breaks(doc: &mut Doc) -> bool {
	match doc {
		Doc::Text(..) => false,
		Doc::Line(line) => matches!(line, Line::Hard | Line::Literal),
		Doc::Concat(parts) | Doc::Fill(parts) => {
			parts
				.iter_mut()
				.fold(false, |forced, part| propagate_breaks(part) | forced)
		}
		Doc::Group { broken, contents } => {
			*broken |= propagate_breaks(contents);
			*broken
		}
		Doc::Indent(contents) | Doc::IndentBy(_, contents) | Doc::Track(_, contents) => {
			propagate_breaks(contents)
		}
	}
}

/// The printed text and the ranges of every tracked document.
pub(crate) struct Printed {
	pub text: String,
	pub tracks: Vec<Range<usize>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
	Flat,
	Break,
}

struct Indentation {
	text: String,
	width: usize,
}

#[derive(Clone, Copy)]
enum Item<'d> {
	Doc(&'d Doc),
	/// The rest of a fill, from some separator on.
	Fill(&'d [Doc]),
	EndTrack(usize),
}

#[derive(Clone)]
struct Command<'d> {
	indent: Rc<Indentation>,
	mode: Mode,
	item: Item<'d>,
}

struct Printer<'a> {
	layout: &'a Layout,
	unit: String,
	out: String,
	column: usize,
	pending_indent: Option<Rc<Indentation>>,
	tracks: Vec<Range<usize>>,
}

impl Printer<'_> {
	fn print(mut self, doc: &Doc, base_indent: &str) -> Printed {
		let root = Rc::new(Indentation {
			text: base_indent.to_owned(),
			width: self.layout.width(base_indent),
		});
		let mut commands = vec![Command {
			indent: root,
			mode: Mode::Break,
			item: Item::Doc(doc),
		}];
		while let Some(command) = commands.pop() {
			self.run(command, &mut commands);
		}
		Printed {
			text: self.out,
			tracks: self.tracks,
		}
	}

	fn run<'d>(&mut self, command: Command<'d>, commands: &mut Vec<Command<'d>>) {
		let Command { indent, mode, item } = command;
		let push = |commands: &mut Vec<Command<'d>>, indent: &Rc<Indentation>, mode, item| {
			commands.push(Command {
				indent: Rc::clone(indent),
				mode,
				item,
			});
		};
		let doc = match item {
			Item::Doc(doc) => doc,
			Item::Fill(parts) => return self.fill(parts, &indent, mode, commands),
			Item::EndTrack(id) => {
				self.tracks[id].end = self.out.len();
				return;
			}
		};
		match doc {
			Doc::Text(text, width) => {
				if let Some(pending) = self.pending_indent.take() {
					self.out.push_str(&pending.text);
				}
				self.out.push_str(text);
				self.column += width;
			}
			Doc::Concat(parts) => {
				for part in parts.iter().rev() {
					push(commands, &indent, mode, Item::Doc(part));
				}
			}
			Doc::Indent(contents) => {
				let indent = self.nested(&indent, &self.unit.clone());
				push(commands, &indent, mode, Item::Doc(contents));
			}
			Doc::IndentBy(extra, contents) => {
				let indent = self.nested(&indent, extra);
				push(commands, &indent, mode, Item::Doc(contents));
			}
			Doc::Group { broken, contents } => {
				let group_mode = if *broken {
					Mode::Break
				} else if mode == Mode::Flat {
					Mode::Flat
				} else {
					let remaining = self.remaining();
					let flat = vec![(Mode::Flat, Item::Doc(contents.as_ref()))];
					if Self::fits(flat, commands, remaining, false) {
						Mode::Flat
					} else {
						Mode::Break
					}
				};
				push(commands, &indent, group_mode, Item::Doc(contents));
			}
			Doc::Fill(parts) => self.fill(parts, &indent, mode, commands),
			Doc::Line(line) => self.line(*line, &indent, mode),
			Doc::Track(id, contents) => {
				self.tracks[*id].start = self.out.len();
				push(commands, &indent, mode, Item::EndTrack(*id));
				push(commands, &indent, mode, Item::Doc(contents));
			}
		}
	}

	fn nested(&self, indent: &Indentation, extra: &str) -> Rc<Indentation> {
		Rc::new(Indentation {
			text: format!("{}{extra}", indent.text),
			width: indent.width + self.layout.width(extra),
		})
	}

	fn remaining(&self) -> isize {
		self.layout.max_width as isize - self.column as isize
	}

	fn line(&mut self, line: Line, indent: &Rc<Indentation>, mode: Mode) {
		match (line, mode) {
			(Line::Soft, Mode::Flat) => {}
			(Line::Space, Mode::Flat) => {
				self.out.push(' ');
				self.column += 1;
			}
			(Line::Literal, _) => {
				self.out.push('\n');
				self.pending_indent = None;
				self.column = 0;
			}
			_ => {
				self.out.push('\n');
				// Indentation is written with the next text, so blank lines
				// carry no trailing whitespace.
				self.pending_indent = Some(Rc::clone(indent));
				self.column = indent.width;
			}
		}
	}

	/// Lays out a fill: each separator breaks only if the content after it
	/// does not fit on the current line.
	fn fill<'d>(
		&mut self,
		parts: &'d [Doc],
		indent: &Rc<Indentation>,
		mode: Mode,
		commands: &mut Vec<Command<'d>>,
	) {
		let Some(content) = parts.first() else {
			return;
		};
		let remaining = self.remaining();
		let command = |mode, item| {
			Command {
				indent: Rc::clone(indent),
				mode,
				item,
			}
		};
		let content_fits = Self::fits(vec![(Mode::Flat, Item::Doc(content))], &[], remaining, true);
		let content_mode = if content_fits {
			Mode::Flat
		} else {
			Mode::Break
		};
		let Some(separator) = parts.get(1) else {
			commands.push(command(content_mode, Item::Doc(content)));
			return;
		};
		if parts.len() == 2 {
			commands.push(command(content_mode, Item::Doc(separator)));
			commands.push(command(content_mode, Item::Doc(content)));
			return;
		}
		let pair_fits = Self::fits(
			vec![
				(Mode::Flat, Item::Doc(&parts[2])),
				(Mode::Flat, Item::Doc(separator)),
				(Mode::Flat, Item::Doc(content)),
			],
			&[],
			remaining,
			true,
		);
		let separator_mode = if pair_fits { Mode::Flat } else { Mode::Break };
		commands.push(command(mode, Item::Fill(&parts[2..])));
		commands.push(command(separator_mode, Item::Doc(separator)));
		commands.push(command(content_mode, Item::Doc(content)));
	}

	/// Whether `stack` (top last) fits in `width` columns before its first
	/// line break, continuing into `rest` when it runs out. With
	/// `must_be_flat`, a broken group does not fit.
	fn fits<'d>(
		mut stack: Vec<(Mode, Item<'d>)>,
		rest: &[Command<'d>],
		mut width: isize,
		must_be_flat: bool,
	) -> bool {
		let mut rest_index = rest.len();
		loop {
			if width < 0 {
				return false;
			}
			let Some((mode, item)) = stack.pop() else {
				if must_be_flat || rest_index == 0 {
					return true;
				}
				rest_index -= 1;
				stack.push((rest[rest_index].mode, rest[rest_index].item));
				continue;
			};
			let doc = match item {
				Item::Doc(doc) => doc,
				Item::Fill(parts) => {
					stack.extend(parts.iter().rev().map(|part| (mode, Item::Doc(part))));
					continue;
				}
				Item::EndTrack(_) => continue,
			};
			match doc {
				Doc::Text(_, text_width) => width -= *text_width as isize,
				Doc::Concat(parts) | Doc::Fill(parts) => {
					stack.extend(parts.iter().rev().map(|part| (mode, Item::Doc(part))));
				}
				Doc::Indent(contents) | Doc::IndentBy(_, contents) | Doc::Track(_, contents) => {
					stack.push((mode, Item::Doc(contents)));
				}
				Doc::Group { broken, contents } => {
					if must_be_flat && *broken {
						return false;
					}
					let group_mode = if *broken { Mode::Break } else { mode };
					stack.push((group_mode, Item::Doc(contents)));
				}
				Doc::Line(line) => {
					if mode == Mode::Break || matches!(line, Line::Hard | Line::Literal) {
						return true;
					}
					if *line == Line::Space {
						width -= 1;
					}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn layout(max_width: usize) -> Layout {
		Layout {
			max_width,
			hard_tabs: true,
			..Layout::default()
		}
	}

	fn words(builder: &Builder<'_>, words: &[&str]) -> Doc {
		let mut parts = Vec::new();
		for (index, word) in words.iter().enumerate() {
			if index > 0 {
				parts.push(line(Line::Space));
			}
			parts.push(builder.text(*word));
		}
		Doc::Fill(parts)
	}

	#[test]
	fn groups_break_together_when_they_do_not_fit() {
		let layout = layout(12);
		let builder = Builder::new(&layout);
		let doc = || {
			group(Doc::Concat(vec![
				builder.text("<a"),
				indent(Doc::Concat(vec![
					line(Line::Space),
					builder.text("b=1"),
					line(Line::Space),
					builder.text("c=2"),
				])),
				line(Line::Soft),
				builder.text(">"),
			]))
		};
		assert_eq!(builder.print(doc(), 0, "").text, "<a b=1 c=2>");
		assert_eq!(builder.print(doc(), 4, "").text, "<a\n\tb=1\n\tc=2\n>");
	}

	#[test]
	fn fills_pack_items_onto_lines() {
		let layout = layout(9);
		let builder = Builder::new(&layout);
		let doc = words(&builder, &["aa", "bb", "cc", "dd", "ee"]);
		assert_eq!(builder.print(doc, 0, "").text, "aa bb cc\ndd ee");
	}

	#[test]
	fn hard_lines_break_enclosing_groups_without_trailing_whitespace() {
		let layout = layout(80);
		let builder = Builder::new(&layout);
		let doc = group(Doc::Concat(vec![
			builder.text("{"),
			indent(Doc::Concat(vec![
				line(Line::Space),
				builder.text("a"),
				line(Line::Hard),
				line(Line::Hard),
				builder.text("b"),
			])),
			line(Line::Space),
			builder.text("}"),
		]));
		assert_eq!(builder.print(doc, 0, "\t").text, "{\n\t\ta\n\n\t\tb\n\t}");
	}

	#[test]
	fn literal_lines_skip_indentation_and_tracks_record_ranges() {
		let layout = layout(80);
		let mut builder = Builder::new(&layout);
		let track = builder.track_id();
		let doc = indent(Doc::Concat(vec![
			builder.text("x"),
			Doc::Track(
				track,
				Box::new(Doc::Concat(vec![
					line(Line::Hard),
					builder.text("\"a"),
					line(Line::Literal),
					builder.text("b\""),
				])),
			),
		]));
		let printed = builder.print(doc, 0, "");
		assert_eq!(printed.text, "x\n\t\"a\nb\"");
		assert_eq!(&printed.text[printed.tracks[0].clone()], "\n\t\"a\nb\"");
	}
}
