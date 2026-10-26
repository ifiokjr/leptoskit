//! The cell buffer: the terminal's framebuffer.

use crate::cell::Attributes;
use crate::cell::Cell;
use crate::cell::TuiColor;

/// A rectangular region of the cell grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
	/// The left column, inclusive.
	pub left: i16,
	/// The top row, inclusive.
	pub top: i16,
	/// The width in cells.
	pub width: u16,
	/// The height in cells.
	pub height: u16,
}

impl Rect {
	/// A region from its edges.
	pub const fn new(left: i16, top: i16, width: u16, height: u16) -> Self {
		Self {
			left,
			top,
			width,
			height,
		}
	}

	/// The column after the last drawn column.
	#[must_use]
	pub const fn right(self) -> i16 {
		self.left + self.width as i16
	}

	/// The row after the last drawn row.
	#[must_use]
	pub const fn bottom(self) -> i16 {
		self.top + self.height as i16
	}

	/// Whether the region covers at least one cell.
	#[must_use]
	pub const fn is_empty(self) -> bool {
		self.width == 0 || self.height == 0
	}

	/// The largest region that lies inside both.
	#[must_use]
	pub fn intersect(self, other: Self) -> Self {
		let left = self.left.max(other.left);
		let top = self.top.max(other.top);
		let right = self.right().min(other.right());
		let bottom = self.bottom().min(other.bottom());

		if right <= left || bottom <= top {
			return Self::new(left, top, 0, 0);
		}

		Self::new(left, top, (right - left) as u16, (bottom - top) as u16)
	}

	/// Whether a cell lies inside the region.
	#[must_use]
	pub const fn contains(self, col: i16, row: i16) -> bool {
		col >= self.left && col < self.right() && row >= self.top && row < self.bottom()
	}
}

/// A grid of terminal cells, addressed by column and row.
///
/// The painter fills it; the differ streams it to the terminal; tests read it
/// back as text. Nothing here touches terminal I/O.
#[derive(Debug, Clone, PartialEq)]
pub struct Buffer {
	width: u16,
	height: u16,
	cells: Vec<Cell>,
}

impl Buffer {
	/// A blank buffer covering `width` columns and `height` rows.
	///
	/// # Panics
	///
	/// Panics when either dimension is zero; a terminal always has at least
	/// one cell of each.
	pub fn new(width: u16, height: u16) -> Self {
		assert!(
			width > 0 && height > 0,
			"a cell buffer needs a positive size"
		);
		Self {
			width,
			height,
			cells: vec![Cell::blank(); usize::from(width) * usize::from(height)],
		}
	}

	/// The width in columns.
	pub const fn width(&self) -> u16 {
		self.width
	}

	/// The height in rows.
	pub const fn height(&self) -> u16 {
		self.height
	}

	/// Resets every cell to blank.
	pub fn clear(&mut self) {
		self.cells.fill(Cell::blank());
	}

	/// The cell at a column and row, or `None` outside the buffer.
	#[must_use]
	pub fn cell(&self, col: u16, row: u16) -> Option<&Cell> {
		if col < self.width && row < self.height {
			Some(&self.cells[usize::from(row) * usize::from(self.width) + usize::from(col)])
		} else {
			None
		}
	}

	/// Mutable access to one cell, or `None` outside the buffer.
	pub fn cell_mut(&mut self, col: u16, row: u16) -> Option<&mut Cell> {
		if col < self.width && row < self.height {
			let width = self.width;
			Some(&mut self.cells[usize::from(row) * usize::from(width) + usize::from(col)])
		} else {
			None
		}
	}

	/// Writes one glyph with its style, clipped to the buffer. Wide glyphs
	/// claim the following cell as their continuation half.
	pub fn set_glyph(
		&mut self,
		col: i16,
		row: i16,
		symbol: char,
		fg: TuiColor,
		bg: TuiColor,
		attrs: Attributes,
	) {
		let index = match (u16::try_from(col), u16::try_from(row)) {
			(Ok(col), Ok(row)) if col < self.width && row < self.height => {
				usize::from(row) * usize::from(self.width) + usize::from(col)
			}
			_ => return,
		};

		self.cells[index] = Cell::styled(symbol, fg, bg, attrs);

		if unicode_width(symbol) > 1 {
			let continuation = u16::try_from(col + 1)
				.ok()
				.filter(|next| *next < self.width);
			if let (Some(next_col), Ok(row)) = (continuation, u16::try_from(row)) {
				let next = usize::from(row) * usize::from(self.width) + usize::from(next_col);
				self.cells[next] = Cell::continuation();
			}
		}
	}

	/// Fills a region with a background color, clipped to the buffer.
	pub fn fill(&mut self, rect: Rect, bg: TuiColor) {
		if bg == TuiColor::Default || rect.is_empty() {
			return;
		}

		for row in rect.top..rect.bottom() {
			for col in rect.left..rect.right() {
				if let Some(cell) = self.cell_mut(
					u16::try_from(col).unwrap_or(u16::MAX),
					u16::try_from(row).unwrap_or(u16::MAX),
				) {
					cell.bg = bg;
				}
			}
		}
	}

	/// Every row as text, the picture a terminal shows: wide glyphs print
	/// once and already advance two columns, so their continuation cells add
	/// nothing.
	#[must_use]
	pub fn to_plain_lines(&self) -> Vec<String> {
		(0..self.height)
			.map(|row| {
				let mut line = String::new();
				let mut col = 0;

				while col < self.width {
					match self.cell(col, row) {
						Some(cell) if cell.wide => {}
						Some(cell) => line.push(cell.symbol),
						None => line.push(' '),
					}
					col += 1;
				}

				line
			})
			.collect()
	}

	/// The buffer as one text block, rows joined by newlines.
	#[must_use]
	pub fn to_plain_text(&self) -> String {
		let mut text = self.to_plain_lines().join("\n");
		text.push('\n');
		text
	}

	/// Whether two buffers show the same picture, ignoring how they encode
	/// wide-glyph halves.
	#[must_use]
	pub fn same_picture(&self, other: &Self) -> bool {
		self.to_plain_lines() == other.to_plain_lines()
	}
}

/// The number of cells a glyph advances the cursor.
fn unicode_width(symbol: char) -> usize {
	unicode_width::UnicodeWidthChar::width(symbol).unwrap_or(0)
}
