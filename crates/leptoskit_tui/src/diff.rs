//! Streaming the difference between two cell buffers as terminal escapes.

use std::fmt::Write as _;
use std::io;
use std::io::Write;

use crate::buffer::Buffer;
use crate::cell::Attributes;
use crate::cell::Cell;
use crate::color::ColorDepth;
use crate::color::TerminalColor;
use crate::color::encode;

/// The style state one run of cells shares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Style {
	fg: TerminalColor,
	bg: TerminalColor,
	attrs: Attributes,
}

impl Style {
	fn of(cell: &Cell, depth: ColorDepth) -> Self {
		let colorless = depth == ColorDepth::Mono;

		Self {
			fg: if colorless {
				TerminalColor::Default
			} else {
				encode(cell.fg, depth)
			},
			bg: if colorless {
				TerminalColor::Default
			} else {
				encode(cell.bg, depth)
			},
			attrs: cell.attrs,
		}
	}

	fn sgr(&self) -> String {
		let mut sequence = String::from("\x1b[0");

		if self.attrs.bold() {
			sequence.push_str(";1");
		}
		if self.attrs.dim() {
			sequence.push_str(";2");
		}
		if self.attrs.italic() {
			sequence.push_str(";3");
		}
		if self.attrs.underline() {
			sequence.push_str(";4");
		}
		if self.attrs.reverse() {
			sequence.push_str(";7");
		}

		match self.fg {
			TerminalColor::Default => sequence.push_str(";39"),
			TerminalColor::TrueColor(red, green, blue) => {
				let _ = write!(sequence, ";38;2;{red};{green};{blue}");
			}
			TerminalColor::Indexed(index) => {
				let _ = write!(sequence, ";38;5;{index}");
			}
		}

		match self.bg {
			TerminalColor::Default => sequence.push_str(";49"),
			TerminalColor::TrueColor(red, green, blue) => {
				let _ = write!(sequence, ";48;2;{red};{green};{blue}");
			}
			TerminalColor::Indexed(index) => {
				let _ = write!(sequence, ";48;5;{index}");
			}
		}

		sequence.push('m');
		sequence
	}
}

/// Writes the escapes that turn `previous` into `next`, reporting whether
/// anything was written. A size change repaints everything.
///
/// Only changed runs are written, each with one cursor move and one style
/// sequence, and a trailing reset leaves the terminal clean.
///
/// # Errors
///
/// Returns the writer's errors unchanged.
pub fn diff(
	previous: &Buffer,
	next: &Buffer,
	depth: ColorDepth,
	out: &mut impl Write,
) -> io::Result<bool> {
	let mut written = false;
	let mut cursor: Option<(u16, u16)> = None;
	let mut style: Option<Style> = None;

	if previous.width() != next.width() || previous.height() != next.height() {
		out.write_all(b"\x1b[2J\x1b[H")?;
		written = true;
		cursor = Some((0, 0));
	}

	for row in 0..next.height() {
		let mut col = 0;

		while col < next.width() {
			let Some(cell) = next.cell(col, row) else {
				break;
			};

			if same_cell(previous, next, col, row) {
				col += 1;
				continue;
			}

			let run_style = Style::of(cell, depth);
			let start = col;

			while col < next.width()
				&& !same_cell(previous, next, col, row)
				&& next
					.cell(col, row)
					.is_some_and(|cell| Style::of(cell, depth) == run_style)
			{
				col += 1;
			}

			if cursor != Some((start, row)) {
				write!(out, "\x1b[{};{}H", row + 1, start + 1)?;
				cursor = Some((start, row));
			}

			if style != Some(run_style) {
				out.write_all(run_style.sgr().as_bytes())?;
				style = Some(run_style);
			}

			for column in start..col {
				let Some(cell) = next.cell(column, row) else {
					break;
				};

				if cell.wide {
					// The wide glyph before this cell already advanced the
					// cursor over it.
					if let Some(position) = cursor.as_mut() {
						position.0 += 1;
					}
					continue;
				}

				let mut symbol = [0u8; 4];
				out.write_all(cell.symbol.encode_utf8(&mut symbol).as_bytes())?;
				written = true;

				let width = width_of(cell.symbol);

				if let Some(position) = cursor.as_mut() {
					position.0 += width;
				}
			}
		}
	}

	if written {
		out.write_all(b"\x1b[0m")?;
		out.flush()?;
	}

	Ok(written)
}

/// Whether both buffers show the same cell at a position; a missing previous
/// cell, after a resize, always differs.
fn same_cell(previous: &Buffer, next: &Buffer, col: u16, row: u16) -> bool {
	match previous.cell(col, row) {
		Some(before) => before == next.cell(col, row).expect("walked inside the next buffer"),
		None => false,
	}
}

/// The cells one glyph advances the cursor.
fn width_of(symbol: char) -> u16 {
	unicode_width::UnicodeWidthChar::width(symbol)
		.map_or(1, |width| u16::try_from(width).unwrap_or(1))
		.max(1)
}
