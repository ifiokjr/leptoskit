//! Monospace text measurement and the one wrapping algorithm both measurement
//! and painting use, so a label's laid-out height is always the number of
//! rows it paints.

use std::borrow::Cow;
use std::ops::Range;

use leptoskit_native::SharedTextMeasurer;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Size;

/// One terminal cell is 8×16 logical pixels, so Tailwind's spacing scale,
/// Yoga layout, and the cell grid all agree: `p-4` is sixteen pixels, two
/// cells wide and one row tall.
pub const CELL_WIDTH: f32 = 8.0;

/// See [`CELL_WIDTH`].
pub const CELL_HEIGHT: f32 = 16.0;

/// The cells of chrome a native-drawn button adds around its label:
/// `[ label ]`.
pub const BUTTON_CHROME_CELLS: usize = 4;

/// The width a string occupies in the monospace grid.
#[must_use]
pub fn display_width(text: &str) -> usize {
	text.chars().map(char_width).sum()
}

/// The width of one glyph; zero-width joins do not advance.
fn char_width(symbol: char) -> usize {
	unicode_width::UnicodeWidthChar::width(symbol).unwrap_or(0)
}

/// Wraps `text` into lines no wider than `max_cols` cells. Every paragraph
/// yields at least one line, so empty text is one empty line.
///
/// Newlines always break. Words wrap at spaces; a word wider than a whole
/// line breaks by glyph. Every line the painter draws comes from here, so
/// measured heights and painted rows cannot disagree.
#[must_use]
pub fn wrap(text: &str, max_cols: usize) -> Vec<Cow<'_, str>> {
	wrap_ranges(text, max_cols)
		.into_iter()
		.map(|range| Cow::Borrowed(&text[range]))
		.collect()
}

/// [`wrap`], as byte ranges into `text`, so a painter can find which styled
/// run each wrapped glyph came from.
#[must_use]
pub fn wrap_ranges(text: &str, max_cols: usize) -> Vec<Range<usize>> {
	let max_cols = max_cols.max(1);
	let mut lines = Vec::new();
	let mut offset = 0;

	for paragraph in text.split('\n') {
		let mut wrap = ParagraphWrap::new(paragraph, max_cols);
		let mut word_start = 0;

		for (index, symbol) in paragraph.char_indices() {
			if symbol == ' ' {
				wrap.push_word(word_start, index);
				wrap.pending_space += 1;
				word_start = index + 1;
			}
		}

		wrap.push_word(word_start, paragraph.len());
		let rest = wrap.line_start..wrap.line_end;
		lines.extend(
			wrap.finished
				.into_iter()
				.chain([rest])
				.map(|line| offset + line.start..offset + line.end),
		);
		offset += paragraph.len() + '\n'.len_utf8();
	}

	lines
}

/// Places the words of one paragraph onto lines, breaking words that are
/// wider than a whole line by glyph.
struct ParagraphWrap<'a> {
	source: &'a str,
	max: usize,
	/// Lines already completed by wrapping, in order.
	finished: Vec<Range<usize>>,
	/// Where the line being built starts.
	line_start: usize,
	/// Just past the last glyph placed on the line being built.
	line_end: usize,
	/// The cells of glyphs on the line being built, without trailing spaces.
	line_width: usize,
	/// Spaces waiting between the placed word and the next one.
	pending_space: usize,
}

impl<'a> ParagraphWrap<'a> {
	fn new(source: &'a str, max: usize) -> Self {
		Self {
			source,
			max,
			finished: Vec::new(),
			line_start: 0,
			line_end: 0,
			line_width: 0,
			pending_space: 0,
		}
	}

	/// Appends the word `source[start..end]`, starting a new line or breaking
	/// the word when it does not fit.
	///
	/// An empty word — the gap between consecutive spaces — leaves the line
	/// untouched; the spaces wait in `pending_space` for the next word.
	fn push_word(&mut self, start: usize, end: usize) {
		if start >= end {
			return;
		}

		let width = display_width(&self.source[start..end]);

		if self.line_width > 0 && self.line_width + self.pending_space + width <= self.max {
			self.line_width += self.pending_space + width;
			self.line_end = end;
			self.pending_space = 0;
			return;
		}

		if width <= self.max {
			self.break_line();
			self.line_start = start;
			self.line_end = end;
			self.line_width = width;
			self.pending_space = 0;
			return;
		}

		// A word wider than a whole line starts on a fresh line and fills
		// whole lines with its glyphs.
		self.break_line();
		self.pending_space = 0;
		let mut chunk_start = start;
		let mut chunk_width = 0;

		for (offset, symbol) in self.source[start..end].char_indices() {
			let glyph = char_width(symbol);

			if chunk_width > 0 && chunk_width + glyph > self.max {
				let break_at = start + offset;
				self.finished.push(chunk_start..break_at);
				chunk_start = break_at;
				chunk_width = 0;
			}

			chunk_width += glyph;
		}

		self.line_start = chunk_start;
		self.line_end = end;
		self.line_width = chunk_width;
	}

	/// Completes the line being built when it holds glyphs.
	fn break_line(&mut self) {
		if self.line_width > 0 {
			self.finished.push(self.line_start..self.line_end);
		}
	}
}

/// Measures text in the terminal's monospace cell grid.
///
/// The font is the user's terminal font, so every size and weight measures
/// into the same cell: [`TextStyle::font_size`] changes nothing here, and
/// weights draw as attributes instead. Layout therefore always matches what
/// the cell grid paints.
#[derive(Debug, Clone, Copy, Default)]
pub struct TuiTextMeasurer;

impl TuiTextMeasurer {
	/// A shared measurer for
	/// [`leptoskit_native::NativeBackend::text_measurer`].
	#[must_use]
	pub fn shared() -> SharedTextMeasurer {
		std::sync::Arc::new(Self)
	}
}

impl TextMeasurer for TuiTextMeasurer {
	fn measure(&self, text: &str, _style: TextStyle, available: Size<AvailableSpace>) -> Size<f32> {
		let max_cols = match available.width {
			AvailableSpace::Definite(width) => Some((width.max(0.0) / CELL_WIDTH).floor() as usize),
			_ => None,
		};

		let lines = match max_cols {
			Some(max) => wrap(text, max),
			None => text.split('\n').map(Cow::from).collect::<Vec<_>>(),
		};

		let width = lines
			.iter()
			.map(|line| display_width(line))
			.max()
			.unwrap_or(0);

		Size::new(width as f32 * CELL_WIDTH, lines.len() as f32 * CELL_HEIGHT)
	}

	fn measure_button(
		&self,
		text: &str,
		_style: TextStyle,
		_available: Size<AvailableSpace>,
		native_chrome: bool,
	) -> Size<f32> {
		// Buttons never wrap: their label is one row, plus the bracket chrome
		// the painter draws when the app has not drawn the button itself.
		let mut cells = display_width(text);

		if native_chrome {
			cells += BUTTON_CHROME_CELLS;
		}

		Size::new(cells as f32 * CELL_WIDTH, CELL_HEIGHT)
	}
}
