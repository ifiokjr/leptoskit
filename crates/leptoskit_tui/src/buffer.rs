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
	/// Where the terminal's own cursor shows, as `(col, row)`, or `None` to
	/// hide it.
	cursor: Option<(u16, u16)>,
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
			cursor: None,
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

	/// Resets every cell to blank and hides the cursor.
	pub fn clear(&mut self) {
		self.cells.fill(Cell::blank());
		self.cursor = None;
	}

	/// Where the terminal's cursor shows: a focused text field's caret.
	#[must_use]
	pub const fn cursor(&self) -> Option<(u16, u16)> {
		self.cursor
	}

	/// Shows the terminal's cursor at a cell, or hides it with `None`. A
	/// position outside the buffer hides it too.
	pub fn set_cursor(&mut self, at: Option<(u16, u16)>) {
		self.cursor = at.filter(|&(col, row)| col < self.width && row < self.height);
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
	///
	/// A [`TuiColor::Default`] background is transparent: the glyph keeps the
	/// background already painted under it, the way text sits on its card.
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

		let bg = if bg == TuiColor::Default {
			self.cells[index].bg
		} else {
			bg
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

	/// Fills a region with an opaque background, clipped to the buffer.
	///
	/// The fill covers whatever was painted there before, glyphs included, the
	/// way a dialog's panel hides the text behind it.
	pub fn fill(&mut self, rect: Rect, bg: TuiColor) {
		if bg == TuiColor::Default {
			return;
		}

		self.each_cell(rect, |cell| {
			*cell = Cell::styled(' ', TuiColor::Default, bg, Attributes::EMPTY);
		});
	}

	/// Blends a translucent color over a region: every glyph and background
	/// already there mixes toward `tint` by `alpha` (out of 255), the way a
	/// scrim dims what sits behind a dialog. The terminal's own default colors
	/// cannot be mixed, so they stay as they are.
	pub fn tint(&mut self, rect: Rect, tint: (u8, u8, u8), alpha: u8) {
		let mix = |color: TuiColor| {
			match color {
				TuiColor::Default => TuiColor::Default,
				TuiColor::Rgb(red, green, blue) => {
					TuiColor::Rgb(
						blend(red, tint.0, alpha),
						blend(green, tint.1, alpha),
						blend(blue, tint.2, alpha),
					)
				}
			}
		};

		self.each_cell(rect, |cell| {
			cell.fg = mix(cell.fg);
			cell.bg = mix(cell.bg);
		});
	}

	/// Paints a background over one half of each cell in a row — the upper
	/// half (`▀`) or the lower half (`▄`) — so a box edge that falls mid-row
	/// keeps its shape. The other half keeps the background already there.
	pub fn half_fill(&mut self, rect: Rect, bg: TuiColor, upper: bool) {
		if bg == TuiColor::Default {
			return;
		}

		let symbol = if upper { '▀' } else { '▄' };
		self.each_cell(rect, |cell| {
			*cell = Cell::styled(symbol, bg, cell.bg, Attributes::EMPTY);
		});
	}

	/// Runs `paint` on every cell of a region inside the buffer.
	fn each_cell(&mut self, rect: Rect, mut paint: impl FnMut(&mut Cell)) {
		for row in rect.top.max(0)..rect.bottom() {
			for col in rect.left.max(0)..rect.right() {
				if let Some(cell) = self.cell_mut(
					u16::try_from(col).unwrap_or(u16::MAX),
					u16::try_from(row).unwrap_or(u16::MAX),
				) {
					paint(cell);
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

/// One channel of `under` mixed toward `over` by `alpha` out of 255.
fn blend(under: u8, over: u8, alpha: u8) -> u8 {
	let alpha = u32::from(alpha);
	let mixed = (u32::from(under) * (255 - alpha) + u32::from(over) * alpha + 127) / 255;
	u8::try_from(mixed).unwrap_or(u8::MAX)
}
