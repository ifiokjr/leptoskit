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
	let mut painter = Painter {
		out,
		depth,
		cursor: None,
		style: None,
		written: false,
	};

	if previous.width() != next.width() || previous.height() != next.height() {
		painter.clear()?;
	}

	for row in 0..next.height() {
		painter.row(previous, next, row)?;
	}

	painter.finish()
}

/// Writes the cells that differ, remembering where the cursor and the style
/// were left so a run of cells costs one move.
struct Painter<'a, W: Write> {
	out: &'a mut W,
	depth: ColorDepth,
	cursor: Option<(u16, u16)>,
	style: Option<Style>,
	written: bool,
}

impl<W: Write> Painter<'_, W> {
	/// Clears the screen, after a resize.
	fn clear(&mut self) -> io::Result<()> {
		self.out.write_all(b"\x1b[2J\x1b[H")?;
		self.written = true;
		self.cursor = Some((0, 0));

		Ok(())
	}

	/// Writes the runs of changed cells on one row.
	fn row(&mut self, previous: &Buffer, next: &Buffer, row: u16) -> io::Result<()> {
		let mut col = 0;

		while col < next.width() {
			let Some(cell) = next.cell(col, row) else {
				break;
			};

			if same_cell(previous, next, col, row) {
				col += 1;
				continue;
			}

			col = self.run(previous, next, row, col, cell)?;
		}

		Ok(())
	}

	/// Writes one run of changed cells, from `start`, in one style. Returns
	/// the column after it.
	fn run(
		&mut self,
		previous: &Buffer,
		next: &Buffer,
		row: u16,
		start: u16,
		cell: &Cell,
	) -> io::Result<u16> {
		let run_style = Style::of(cell, self.depth);
		let mut col = start;

		while col < next.width()
			&& !same_cell(previous, next, col, row)
			&& next
				.cell(col, row)
				.is_some_and(|cell| Style::of(cell, self.depth) == run_style)
		{
			col += 1;
		}

		if self.cursor != Some((start, row)) {
			write!(self.out, "\x1b[{};{}H", row + 1, start + 1)?;
			self.cursor = Some((start, row));
		}

		if self.style != Some(run_style) {
			self.out.write_all(run_style.sgr().as_bytes())?;
			self.style = Some(run_style);
		}

		for column in start..col {
			self.cell(next, column, row)?;
		}

		Ok(col)
	}

	/// Writes one cell, moving the cursor over the glyph's width.
	fn cell(&mut self, next: &Buffer, column: u16, row: u16) -> io::Result<()> {
		let Some(cell) = next.cell(column, row) else {
			return Ok(());
		};

		if cell.wide {
			// The wide glyph before this cell already advanced the cursor
			// over it.
			if let Some(position) = self.cursor.as_mut() {
				position.0 += 1;
			}

			return Ok(());
		}

		let mut symbol = [0u8; 4];
		self.out
			.write_all(cell.symbol.encode_utf8(&mut symbol).as_bytes())?;
		self.written = true;

		let width = width_of(cell.symbol);

		if let Some(position) = self.cursor.as_mut() {
			position.0 += width;
		}

		Ok(())
	}

	/// Resets the style, and says whether anything was written.
	fn finish(&mut self) -> io::Result<bool> {
		if self.written {
			self.out.write_all(b"\x1b[0m")?;
			self.out.flush()?;
		}

		Ok(self.written)
	}
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
