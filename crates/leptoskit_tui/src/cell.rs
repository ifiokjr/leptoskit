//! The cell grid the terminal backend draws into.

/// One terminal cell: a glyph, its colors, and its text attributes.
///
/// A wide glyph (a CJK character, an em dash) occupies its cell plus the
/// following one, which [`Cell::continuation`] marks so diffing and text
/// dumps skip it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
	/// The drawn glyph; a space when the cell is empty.
	pub symbol: char,
	/// The foreground color.
	pub fg: TuiColor,
	/// The background color.
	pub bg: TuiColor,
	/// The text attributes.
	pub attrs: Attributes,
	/// Marks the trailing half of a wide glyph.
	pub wide: bool,
}

impl Default for Cell {
	fn default() -> Self {
		Self {
			symbol: ' ',
			fg: TuiColor::Default,
			bg: TuiColor::Default,
			attrs: Attributes::EMPTY,
			wide: false,
		}
	}
}

impl Cell {
	/// A blank cell with default colors.
	pub const fn blank() -> Self {
		Self {
			symbol: ' ',
			fg: TuiColor::Default,
			bg: TuiColor::Default,
			attrs: Attributes::EMPTY,
			wide: false,
		}
	}

	/// Marks this cell as the trailing half of the previous wide glyph.
	pub const fn continuation() -> Self {
		Self {
			wide: true,
			..Self::blank()
		}
	}

	/// A cell with the given glyph, colors, and attributes.
	pub const fn styled(symbol: char, fg: TuiColor, bg: TuiColor, attrs: Attributes) -> Self {
		Self {
			symbol,
			fg,
			bg,
			attrs,
			wide: false,
		}
	}
}

/// A color a terminal can show, before quantization to its color depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TuiColor {
	/// The terminal's default foreground or background.
	#[default]
	Default,
	/// A 24-bit color.
	Rgb(u8, u8, u8),
}

impl TuiColor {
	/// A mid gray, for placeholders and placeholders' kin.
	pub const PLACEHOLDER: Self = Self::Rgb(128, 128, 128);

	/// The shared packed color as a terminal color, treating colors the
	/// terminal cannot layer (low alpha) as transparent.
	#[must_use]
	pub fn from_packed(color: leptoskit_native::Color) -> Self {
		if color.alpha() < MIN_ALPHA {
			return Self::Default;
		}

		Self::Rgb(color.red(), color.green(), color.blue())
	}
}

/// Below this alpha a color paints nothing over the terminal's own colors.
pub const MIN_ALPHA: u8 = 128;

/// Text attributes a cell can combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Attributes(u8);

impl Attributes {
	/// Heavier glyphs, the terminal's bold.
	pub const BOLD: Self = Self(Self::BOLD_BIT);
	const BOLD_BIT: u8 = 1 << 0;
	/// Fainter glyphs, the terminal's dim: disabled and translucent content.
	pub const DIM: Self = Self(Self::DIM_BIT);
	const DIM_BIT: u8 = 1 << 1;
	/// No attributes.
	pub const EMPTY: Self = Self(0);
	/// Italic glyphs.
	pub const ITALIC: Self = Self(Self::ITALIC_BIT);
	const ITALIC_BIT: u8 = 1 << 4;
	/// Swap foreground and background: focus rings and the block caret.
	pub const REVERSE: Self = Self(Self::REVERSE_BIT);
	const REVERSE_BIT: u8 = 1 << 3;
	/// An underline, the focused `Pressable` ring.
	pub const UNDERLINE: Self = Self(Self::UNDERLINE_BIT);
	const UNDERLINE_BIT: u8 = 1 << 2;

	/// The attributes bold text renders with for the given weight.
	#[must_use]
	pub const fn for_weight(weight: leptoskit_native::FontWeight) -> Self {
		match weight {
			leptoskit_native::FontWeight::Semibold | leptoskit_native::FontWeight::Bold => {
				Self::BOLD
			}
			_ => Self::EMPTY,
		}
	}

	/// Both sets of attributes at once.
	#[must_use]
	pub const fn union(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}

	/// These attributes without any of `other`'s.
	#[must_use]
	pub const fn without(self, other: Self) -> Self {
		Self(self.0 & !other.0)
	}

	/// Whether the bold bit is set.
	#[must_use]
	pub const fn bold(self) -> bool {
		self.0 & Self::BOLD_BIT != 0
	}

	/// Whether the dim bit is set.
	#[must_use]
	pub const fn dim(self) -> bool {
		self.0 & Self::DIM_BIT != 0
	}

	/// Whether the underline bit is set.
	#[must_use]
	pub const fn underline(self) -> bool {
		self.0 & Self::UNDERLINE_BIT != 0
	}

	/// Whether the reverse bit is set.
	#[must_use]
	pub const fn reverse(self) -> bool {
		self.0 & Self::REVERSE_BIT != 0
	}

	/// Whether glyphs draw italic.
	#[must_use]
	pub const fn italic(self) -> bool {
		self.0 & Self::ITALIC_BIT != 0
	}
}
