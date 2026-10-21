//! Style properties understood by the layout engine.
//!
//! The types mirror the CSS box model closely enough that a stylesheet or a
//! React-Native-style `StyleSheet.create` call maps onto them one-to-one, while
//! staying `Copy` where possible so a whole [`Style`] is cheap to move around.

use core::fmt;

/// A single edge rectangle: insets, margins, padding, or borders.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rect<T> {
	/// The top edge.
	pub top: T,
	/// The right edge.
	pub right: T,
	/// The bottom edge.
	pub bottom: T,
	/// The left edge.
	pub left: T,
}

impl<T> Rect<T> {
	/// Builds a rect from a function of the edge name.
	pub fn new(f: impl Fn() -> T) -> Self
	where
		T: Clone,
	{
		let all = f();
		Self {
			top: all.clone(),
			right: all.clone(),
			bottom: all.clone(),
			left: all,
		}
	}

	/// Builds a rect with explicit values for each edge.
	pub const fn all_edges(top: T, right: T, bottom: T, left: T) -> Self {
		Self {
			top,
			right,
			bottom,
			left,
		}
	}
}

impl<T: Default> Default for Rect<T> {
	fn default() -> Self {
		Self {
			top: T::default(),
			right: T::default(),
			bottom: T::default(),
			left: T::default(),
		}
	}
}

impl<T: fmt::Display> fmt::Display for Rect<T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"(top: {}, right: {}, bottom: {}, left: {})",
			self.top, self.right, self.bottom, self.left
		)
	}
}

/// A width or height value.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Dimension {
	/// No constraint; the size comes from content or the layout algorithm.
	#[default]
	Auto,
	/// A fixed length in layout units (logical pixels).
	Points(f32),
	/// A percentage of the containing block's matching dimension.
	Percent(f32),
}

impl Dimension {
	/// Resolves the dimension against a definite containing block size.
	///
	/// Returns `None` when the dimension is `Auto` or the containing block is
	/// indefinite, which callers treat as "not specified".
	pub fn resolve(self, containing: Option<f32>) -> Option<f32> {
		match (self, containing) {
			(Self::Points(points), _) => Some(points),
			(Self::Percent(fraction), Some(total)) => Some(fraction * total),
			_ => None,
		}
	}

	/// Returns `true` when the dimension carries no information.
	pub fn is_auto(self) -> bool {
		matches!(self, Self::Auto)
	}
}

/// How a node's children flow inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Display {
	/// Standard flow, laid out with the flexbox algorithm.
	#[default]
	Flex,
	/// Standard flow, laid out with the CSS grid algorithm.
	Grid,
	/// The node generates no box; children participate in the parent's layout.
	Contents,
	/// The node and its subtree are skipped entirely.
	None,
}

/// The positioning scheme used for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PositionType {
	/// In-flow, participating normally in flex or grid layout.
	#[default]
	Relative,
	/// Out-of-flow, positioned against the containing block's padding box.
	Absolute,
	/// Out-of-flow. Currently laid out exactly like [`PositionType::Absolute`],
	/// against the parent's padding box rather than the viewport.
	Fixed,
}

/// The writing direction of the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Direction {
	/// Left to right.
	#[default]
	Ltr,
	/// Right to left.
	Rtl,
}

/// The main axis of a flex container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FlexDirection {
	/// Main axis runs left to right (or right to left under `Rtl`).
	#[default]
	Row,
	/// Main axis runs right to left (or left to right under `Rtl`).
	RowReverse,
	/// Main axis runs top to bottom.
	Column,
	/// Main axis runs bottom to top.
	ColumnReverse,
}

impl FlexDirection {
	/// Returns `true` when the main axis is horizontal.
	pub fn is_row(self) -> bool {
		matches!(self, Self::Row | Self::RowReverse)
	}

	/// Returns `true` when the main axis runs against the writing direction.
	pub fn is_reverse(self) -> bool {
		matches!(self, Self::RowReverse | Self::ColumnReverse)
	}
}

/// Distribution of items along the main axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum JustifyContent {
	/// Items pack toward the start of the main axis.
	#[default]
	FlexStart,
	/// Items pack toward the end of the main axis.
	FlexEnd,
	/// Items pack toward the center of the main axis.
	Center,
	/// Free space is distributed between items.
	SpaceBetween,
	/// Free space is distributed around items.
	SpaceAround,
	/// Free space is distributed evenly, including before the first and after
	/// the last item.
	SpaceEvenly,
}

/// Alignment along the cross axis, for items and lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Align {
	/// Defers to the parent's `align_items`; only valid for `align_self`.
	#[default]
	Auto,
	/// Pack toward the start of the cross axis.
	FlexStart,
	/// Pack toward the end of the cross axis.
	FlexEnd,
	/// Pack toward the center of the cross axis.
	Center,
	/// Fill the available cross space.
	Stretch,
	/// Align the first baselines; falls back to `FlexStart` in this engine.
	Baseline,
	/// Free space between items; only meaningful for `align_content`.
	SpaceBetween,
	/// Free space around items; only meaningful for `align_content`.
	SpaceAround,
	/// Free space evenly; only meaningful for `align_content`.
	SpaceEvenly,
}

impl Align {
	/// Resolves an `align_self` value against the container's `align_items`.
	#[must_use]
	pub fn resolve_self(self, parent_items: Align) -> Self {
		if self == Self::Auto {
			parent_items
		} else {
			self
		}
	}
}

/// Whether flex items wrap onto multiple lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FlexWrap {
	/// All items stay on one line.
	#[default]
	NoWrap,
	/// Items wrap onto new lines in the cross direction.
	Wrap,
	/// Items wrap onto new lines in the reverse cross direction.
	WrapReverse,
}

/// How content that overflows the content box is treated.
///
/// The engine always lays out overflow content; the value is carried through so
/// the platform backend can clip or add scroll affordances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Overflow {
	/// Overflow is painted outside the box.
	#[default]
	Visible,
	/// Overflow is clipped to the box.
	Hidden,
	/// Overflow is clipped and scrollable.
	Scroll,
}

/// Which box the `width` and `height` properties size.
///
/// The selected box also applies to min/max constraints, flex basis, and the
/// preferred aspect ratio. Layout results always report border-box sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BoxSizing {
	/// `width` sizes the border box; padding and border eat into the content.
	#[default]
	BorderBox,
	/// `width` sizes the content box; padding and border add onto it.
	ContentBox,
}

/// A two-dimensional size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Size<T> {
	/// The inline size.
	pub width: T,
	/// The block size.
	pub height: T,
}

impl<T> Size<T> {
	/// Builds a size from a width and a height.
	pub const fn new(width: T, height: T) -> Self {
		Self { width, height }
	}

	/// Maps both dimensions through a function.
	#[must_use]
	pub fn map<U>(self, f: impl Fn(T) -> U) -> Size<U> {
		Size::new(f(self.width), f(self.height))
	}
}

/// What the layout algorithm knows about one axis of available space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AvailableSpace {
	/// A definite size, in layout units.
	Definite(f32),
	/// Only the min-content contribution is available.
	MinContent,
	/// The max-content contribution is available.
	#[default]
	MaxContent,
}

impl AvailableSpace {
	/// The definite size, if known.
	pub fn definite(self) -> Option<f32> {
		match self {
			Self::Definite(value) => Some(value),
			_ => None,
		}
	}
}

/// A track sizing function for grid rows and columns.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TrackSizing {
	/// Sized by the largest min-content contribution of items in the track.
	#[default]
	Auto,
	/// A fixed length.
	Points(f32),
	/// A percentage of the grid container's matching dimension.
	Percent(f32),
	/// A share of the leftover space; `1fr` divides the free space evenly.
	Fraction(f32),
	/// A floor and ceiling on an inner sizing function.
	MinMax(Box<MinMax>),
}

/// The bounds of a `minmax()` track sizing function.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MinMax {
	/// The smallest the track may be.
	pub min: TrackSizing,
	/// The largest the track may be.
	pub max: TrackSizing,
}

/// A minmax track between fixed bounds.
pub fn minmax(min: f32, max: f32) -> TrackSizing {
	TrackSizing::MinMax(Box::new(MinMax {
		min: TrackSizing::Points(min),
		max: TrackSizing::Points(max),
	}))
}

impl TrackSizing {
	/// A `Points` or `Percent` track resolves immediately against a definite
	/// container size.
	pub fn resolve_base(&self, container: Option<f32>) -> Option<f32> {
		match *self {
			Self::Points(points) => Some(points),
			Self::Percent(fraction) => container.map(|total| fraction * total),
			Self::MinMax(ref inner) => inner.max.resolve_base(container),
			_ => None,
		}
	}
}

/// A grid line reference: automatic, an explicit line number, or a span.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GridLine {
	/// The placement algorithm chooses the line.
	#[default]
	Auto,
	/// A 1-based, grid-relative line number; negative numbers count back from
	/// the last explicit line (`-1`). Values beyond ±10 000 are clamped.
	Line(i32),
	/// The item spans this many tracks, at most 10 000.
	Span(u16),
}

/// Placement of an item along one grid axis.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GridPlacement {
	/// Where the item starts.
	pub start: GridLine,
	/// Where the item ends.
	pub end: GridLine,
}

impl GridPlacement {
	/// Fully automatic placement on this axis.
	pub const AUTO: Self = Self {
		start: GridLine::Auto,
		end: GridLine::Auto,
	};

	/// Placement spanning an explicit range of lines.
	pub const fn line(start: i32, end: i32) -> Self {
		Self {
			start: GridLine::Line(start),
			end: GridLine::Line(end),
		}
	}

	/// Placement spanning a number of tracks.
	pub const fn span(tracks: u16) -> Self {
		Self {
			start: GridLine::Auto,
			end: GridLine::Span(tracks),
		}
	}
}

/// How the auto-placement algorithm walks the implicit grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GridAutoFlow {
	/// Fill rows first, moving the cursor forward only.
	#[default]
	Row,
	/// Fill columns first, moving the cursor forward only.
	Column,
	/// Fill rows first, backtracking to fill earlier holes.
	RowDense,
	/// Fill columns first, backtracking to fill earlier holes.
	ColumnDense,
}

impl GridAutoFlow {
	/// Returns `true` when the algorithm backtracks to fill holes.
	pub fn is_dense(self) -> bool {
		matches!(self, Self::RowDense | Self::ColumnDense)
	}

	/// Returns `true` when rows are filled first.
	pub fn is_row(self) -> bool {
		matches!(self, Self::Row | Self::RowDense)
	}
}

/// Grid-specific style properties.
///
/// Applied when [`Display::Grid`] is set; ignored otherwise.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GridStyle {
	/// The explicit column tracks.
	pub template_columns: Vec<TrackSizing>,
	/// The explicit row tracks.
	pub template_rows: Vec<TrackSizing>,
	/// The sizing function for implicit columns.
	pub auto_columns: TrackSizing,
	/// The sizing function for implicit rows.
	pub auto_rows: TrackSizing,
	/// The auto-placement direction and density.
	pub auto_flow: GridAutoFlow,
	/// Placement along the inline axis.
	pub column: GridPlacement,
	/// Placement along the block axis.
	pub row: GridPlacement,
	/// Default inline-axis alignment for items in their area.
	pub justify_items: Align,
	/// This item's inline-axis alignment, overriding the container default.
	pub justify_self: Align,
}

/// The full style of a layout node.
///
/// Defaults follow the React Native convention: `flex_direction: Column`,
/// `align_items: Stretch`, and `flex_shrink: 0`, which is what a native app
/// expects out of the box.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
	feature = "serde",
	derive(serde::Serialize, serde::Deserialize),
	serde(default)
)]
pub struct Style {
	/// Box generation mode.
	pub display: Display,
	/// Positioning scheme.
	pub position: PositionType,
	/// Writing direction.
	pub direction: Direction,
	/// Main axis of the flex container.
	pub flex_direction: FlexDirection,
	/// Main-axis distribution.
	pub justify_content: JustifyContent,
	/// Default cross-axis alignment for children.
	pub align_items: Align,
	/// This node's cross-axis alignment, overriding the parent default.
	pub align_self: Align,
	/// Cross-axis distribution of wrapped lines.
	pub align_content: Align,
	/// Whether items wrap.
	pub flex_wrap: FlexWrap,
	/// Overflow behavior, carried through for the backend.
	pub overflow: Overflow,
	/// Which box dimensions, min/max constraints, and flex basis size.
	pub box_sizing: BoxSizing,
	/// Grow factor in the flex algorithm.
	pub flex_grow: f32,
	/// Shrink factor in the flex algorithm.
	pub flex_shrink: f32,
	/// Initial main size in the flex algorithm.
	pub flex_basis: Dimension,
	/// Fixed gaps between lines and between items.
	pub gap: Rect<Dimension>,
	/// The width of the box selected by `box_sizing`.
	pub width: Dimension,
	/// The height of the box selected by `box_sizing`.
	pub height: Dimension,
	/// The smallest width.
	pub min_width: Dimension,
	/// The largest width.
	pub max_width: Dimension,
	/// The smallest height.
	pub min_height: Dimension,
	/// The largest height.
	pub max_height: Dimension,
	/// The preferred width/height ratio (`width / height`).
	pub aspect_ratio: Option<f32>,
	/// Insets from the containing block's edges.
	pub inset: Rect<Dimension>,
	/// Margins around the border box; `Auto` margins absorb free space.
	pub margin: Rect<Dimension>,
	/// Padding inside the border box.
	pub padding: Rect<Dimension>,
	/// Borders, which take space but do not collapse.
	pub border: Rect<f32>,
	/// Grid properties, used when `display` is `Grid`.
	pub grid: Option<GridStyle>,
}

impl Default for Style {
	fn default() -> Self {
		Self {
			display: Display::default(),
			position: PositionType::default(),
			direction: Direction::default(),
			flex_direction: FlexDirection::Column,
			justify_content: JustifyContent::default(),
			align_items: Align::Stretch,
			align_self: Align::default(),
			align_content: Align::default(),
			flex_wrap: FlexWrap::default(),
			overflow: Overflow::default(),
			box_sizing: BoxSizing::default(),
			flex_grow: 0.0,
			flex_shrink: 0.0,
			flex_basis: Dimension::default(),
			gap: Rect::default(),
			width: Dimension::default(),
			height: Dimension::default(),
			min_width: Dimension::default(),
			max_width: Dimension::default(),
			min_height: Dimension::default(),
			max_height: Dimension::default(),
			aspect_ratio: None,
			inset: Rect::default(),
			// CSS margins default to zero, not `auto`, so unset margins never
			// absorb free space.
			margin: Rect {
				top: Dimension::Points(0.0),
				right: Dimension::Points(0.0),
				bottom: Dimension::Points(0.0),
				left: Dimension::Points(0.0),
			},
			padding: Rect::default(),
			border: Rect::default(),
			grid: None,
		}
	}
}

impl Style {
	/// A style with `flex_direction: Row`, matching the CSS default.
	pub fn row() -> Self {
		Self {
			flex_direction: FlexDirection::Row,
			..Self::default()
		}
	}
}
