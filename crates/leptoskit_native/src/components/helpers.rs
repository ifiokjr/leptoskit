//! Portable compositions over the shared element and box-layout model.

use std::fmt;
use std::num::NonZeroU8;

use leptoskit_yoga::Align as BoxAlign;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Display;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::FlexWrap;
use leptoskit_yoga::GridPlacement;
use leptoskit_yoga::GridStyle;
use leptoskit_yoga::JustifyContent;
use leptoskit_yoga::MinMax;
use leptoskit_yoga::Overflow;
use leptoskit_yoga::PositionType;
use leptoskit_yoga::Rect;
use leptoskit_yoga::TrackSizing;

use super::ScrollView;
use super::Text;
use super::View;
use crate::Color;
use crate::Element;
use crate::FontWeight;
use crate::TextStyle;

/// Invalid numeric input to a layout or presentation component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
	/// A constraint must be finite and nonnegative.
	InvalidConstraint(&'static str),
	/// A maximum constraint is smaller than the minimum on this axis.
	InvertedConstraints(&'static str),
	/// An aspect ratio must be finite and between `1e-6` and `1e6`.
	InvalidAspectRatio,
	/// An opacity must be finite and between zero and one, inclusive.
	InvalidOpacity,
	/// A corner radius must be finite and nonnegative.
	InvalidRadius,
}

impl fmt::Display for LayoutError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::InvalidConstraint(name) => {
				write!(formatter, "{name} must be finite and nonnegative")
			}
			Self::InvertedConstraints(axis) => {
				write!(
					formatter,
					"maximum {axis} must not be smaller than minimum {axis}"
				)
			}
			Self::InvalidAspectRatio => {
				formatter.write_str("aspect ratio must be finite and between 0.000001 and 1000000")
			}
			Self::InvalidOpacity => {
				formatter.write_str("opacity must be finite and between zero and one")
			}
			Self::InvalidRadius => {
				formatter.write_str("corner radius must be finite and nonnegative")
			}
		}
	}
}

impl std::error::Error for LayoutError {}

/// Validated minimum and optional maximum border-box dimensions.
///
/// These are CSS-style min/max constraints on the wrapper, in logical pixels.
/// They do not implement Flutter's parent-to-child constraint protocol. `None`
/// means no maximum; finite maxima cannot be smaller than their minima.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BoxConstraints {
	min_width: f32,
	max_width: Option<f32>,
	min_height: f32,
	max_height: Option<f32>,
}

impl BoxConstraints {
	/// Validates explicit minimum and optional maximum dimensions.
	pub fn new(
		min_width: f32,
		max_width: Option<f32>,
		min_height: f32,
		max_height: Option<f32>,
	) -> Result<Self, LayoutError> {
		for (name, value) in [
			("minimum width", Some(min_width)),
			("maximum width", max_width),
			("minimum height", Some(min_height)),
			("maximum height", max_height),
		] {
			if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
				return Err(LayoutError::InvalidConstraint(name));
			}
		}

		if max_width.is_some_and(|maximum| maximum < min_width) {
			return Err(LayoutError::InvertedConstraints("width"));
		}

		if max_height.is_some_and(|maximum| maximum < min_height) {
			return Err(LayoutError::InvertedConstraints("height"));
		}

		Ok(Self {
			min_width,
			max_width,
			min_height,
			max_height,
		})
	}

	/// Constrains both dimensions to exactly the supplied sizes.
	pub fn tight(width: f32, height: f32) -> Result<Self, LayoutError> {
		Self::new(width, Some(width), height, Some(height))
	}

	/// The smallest permitted width.
	pub const fn min_width(self) -> f32 {
		self.min_width
	}

	/// The largest permitted width, or no maximum.
	pub const fn max_width(self) -> Option<f32> {
		self.max_width
	}

	/// The smallest permitted height.
	pub const fn min_height(self) -> f32 {
		self.min_height
	}

	/// The largest permitted height, or no maximum.
	pub const fn max_height(self) -> Option<f32> {
		self.max_height
	}
}

/// Logical alignment of children inside an [`Align`] container.
///
/// Start and end follow the wrapper's writing direction. The container
/// centers or packs its children within its own allocated box; give it a size
/// or a flex grow factor when it should occupy additional available space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Alignment {
	/// Top edge, inline start.
	TopStart,
	/// Top edge, horizontal center.
	TopCenter,
	/// Top edge, inline end.
	TopEnd,
	/// Vertical center, inline start.
	CenterStart,
	/// Center on both axes.
	#[default]
	Center,
	/// Vertical center, inline end.
	CenterEnd,
	/// Bottom edge, inline start.
	BottomStart,
	/// Bottom edge, horizontal center.
	BottomCenter,
	/// Bottom edge, inline end.
	BottomEnd,
}

impl Alignment {
	fn axes(self) -> (JustifyContent, BoxAlign) {
		let vertical = match self {
			Self::TopStart | Self::TopCenter | Self::TopEnd => JustifyContent::FlexStart,
			Self::CenterStart | Self::Center | Self::CenterEnd => JustifyContent::Center,
			Self::BottomStart | Self::BottomCenter | Self::BottomEnd => JustifyContent::FlexEnd,
		};
		let horizontal = match self {
			Self::TopStart | Self::CenterStart | Self::BottomStart => BoxAlign::FlexStart,
			Self::TopCenter | Self::Center | Self::BottomCenter => BoxAlign::Center,
			Self::TopEnd | Self::CenterEnd | Self::BottomEnd => BoxAlign::FlexEnd,
		};
		(vertical, horizontal)
	}
}

/// Places children horizontally, without wrapping or shrinking by default.
pub fn Row() -> Element {
	Flex(FlexDirection::Row)
}

/// Places children vertically, without wrapping or shrinking by default.
pub fn Column() -> Element {
	Flex(FlexDirection::Column)
}

/// Places children along an explicit flex direction.
pub fn Flex(direction: FlexDirection) -> Element {
	View().with_style(|style| style.flex_direction = direction)
}

/// Places children in horizontal runs that wrap at the available width.
///
/// Item sizes follow flexbox rather than Flutter's `Wrap` constraint protocol.
pub fn Wrap() -> Element {
	Row().with_style(|style| {
		style.flex_wrap = FlexWrap::Wrap;
		style.align_content = BoxAlign::FlexStart;
	})
}

/// Places eager children in equal-width columns and auto-sized rows.
///
/// Columns use `minmax(0, 1fr)`, allowing content to shrink to the available
/// width. The nonzero count is limited to 255 columns by its input type.
pub fn Grid(columns: NonZeroU8) -> Element {
	View().with_style(|style| {
		style.display = Display::Grid;
		style.grid = Some(equal_columns(columns));
	})
}

/// Adds the supplied box-model padding around its children.
///
/// Percentages resolve against the containing block width. `Auto` means zero.
pub fn Padding(insets: Rect<Dimension>) -> Element {
	View().with_style(|style| style.padding = insets)
}

/// A box with explicit width and height, optionally containing children.
///
/// `Auto` leaves an axis sized by content and its parent layout. This uses the
/// shared CSS box model rather than Flutter's tight constraints on a child.
pub fn SizedBox(width: Dimension, height: Dimension) -> Element {
	View().with_style(|style| {
		style.width = width;
		style.height = height;
	})
}

/// Constrains the wrapper's border box to validated pixel bounds.
pub fn ConstrainedBox(constraints: BoxConstraints) -> Element {
	View().with_style(|style| {
		style.min_width = Dimension::Points(constraints.min_width);
		style.max_width = constraints
			.max_width
			.map_or(Dimension::Auto, Dimension::Points);
		style.min_height = Dimension::Points(constraints.min_height);
		style.max_height = constraints
			.max_height
			.map_or(Dimension::Auto, Dimension::Points);
	})
}

/// Gives a box a preferred width/height ratio.
///
/// At least one axis must be definite to derive the other. Explicit dimensions
/// and min/max bounds take precedence, as they do for CSS `aspect-ratio`.
/// Supported ratios range from `1e-6` through `1e6`, matching Tailwind
/// utilities.
pub fn AspectRatio(ratio: f32) -> Result<Element, LayoutError> {
	if !ratio.is_finite() || !(1e-6..=1e6).contains(&ratio) {
		return Err(LayoutError::InvalidAspectRatio);
	}

	Ok(View().with_style(|style| style.aspect_ratio = Some(ratio)))
}

/// Packs children at one of nine positions inside its allocated box.
///
/// Multiple children form a column. Start and end follow this wrapper's
/// `direction` style, which defaults to LTR in the shared element model.
pub fn Align(alignment: Alignment) -> Element {
	let (vertical, horizontal) = alignment.axes();
	View().with_style(|style| {
		style.justify_content = vertical;
		style.align_items = horizontal;
	})
}

/// Centers children as a column inside its allocated box.
pub fn Center() -> Element {
	Align(Alignment::Center)
}

/// A flex item that receives an equal share of remaining main-axis space.
///
/// Edits the supplied child's parent data without adding a wrapper. It uses
/// `flex: 1 1 0` and zero minimum dimensions, and must be a direct child of a
/// flex container. Set `flex_grow` to change its share.
pub fn Expanded(child: Element) -> Element {
	child.with_style(|style| {
		style.flex_grow = 1.0;
		style.flex_shrink = 1.0;
		style.flex_basis = Dimension::Points(0.0);
		style.min_width = Dimension::Points(0.0);
		style.min_height = Dimension::Points(0.0);
	})
}

/// Empty flex space that shares the main-axis remainder with expanded items.
pub fn Spacer() -> Element {
	Expanded(View()).with_style(|style| {
		style.width = Dimension::Points(0.0);
		style.height = Dimension::Points(0.0);
	})
}

/// Overlaps supplied children in a single grid cell, in paint order.
///
/// Non-absolute children contribute to intrinsic sizing and stretch within
/// the shared cell, which fills any definite allocation. [`Positioned`]
/// children do not contribute to sizing. Pass
/// every overlapping child here: later `.child` calls use normal grid
/// placement.
pub fn Stack(children: impl IntoIterator<Item = Element>) -> Element {
	View()
		.with_style(|style| {
			style.display = Display::Grid;
			style.grid = Some(GridStyle {
				template_columns: vec![fraction_track()],
				template_rows: vec![fraction_track()],
				justify_items: BoxAlign::Stretch,
				..GridStyle::default()
			});
		})
		.children(children.into_iter().map(|child| {
			child.with_style(|style| {
				let grid = style.grid.get_or_insert_with(GridStyle::default);
				grid.column = GridPlacement::line(1, 2);
				grid.row = GridPlacement::line(1, 2);
			})
		}))
}

/// Positions a child outside normal flow, relative to its containing block.
///
/// Use `Auto` for unspecified insets. Opposing insets size an automatic axis;
/// explicit dimensions retain their size and use the starting inset. Edits the
/// supplied child directly, without inserting another element. In a [`Stack`],
/// the assigned grid cell defines the containing block; in a flex container,
/// the parent's padding box does.
pub fn Positioned(insets: Rect<Dimension>, child: Element) -> Element {
	child.with_style(|style| {
		style.position = PositionType::Absolute;
		style.inset = insets;
	})
}

/// Paints a solid background behind its children.
pub fn ColoredBox(color: Color) -> Element {
	View().background(color)
}

/// Clips overflowing descendants to the wrapper's rectangular bounds.
pub fn ClipRect() -> Element {
	View().with_style(|style| style.overflow = Overflow::Hidden)
}

/// Clips descendants to a rectangle with a uniform corner radius.
pub fn ClipRRect(radius: f32) -> Result<Element, LayoutError> {
	if !radius.is_finite() || radius < 0.0 {
		return Err(LayoutError::InvalidRadius);
	}

	Ok(ClipRect().corner_radius(radius))
}

/// Composites a wrapper and its descendants with a uniform opacity.
///
/// Zero opacity preserves layout, hit testing, and accessibility. Use
/// [`Visibility`] when the subtree should be removed from layout and input.
pub fn Opacity(opacity: f32) -> Result<Element, LayoutError> {
	if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
		return Err(LayoutError::InvalidOpacity);
	}

	Ok(View().with_props(|props| props.opacity = Some(opacity)))
}

/// Supplies inherited system-font size, weight, and alignment to descendants.
///
/// A child's explicit text properties or Tailwind utilities override defaults.
/// Set `.color(...)` on this wrapper to supply an inherited foreground as well.
pub fn DefaultTextStyle(text_style: TextStyle) -> Element {
	View()
		.font(text_style.font_size, text_style.font_weight)
		.with_props(|props| props.text_align = Some(text_style.text_align))
}

/// Collapses a subtree when false and restores its normal layout when true.
///
/// Hidden descendants retain their reconciled nodes but receive no input and
/// occupy no space. This does not preserve hidden layout as Flutter `Offstage`
/// does.
pub fn Visibility(visible: bool) -> Element {
	View().with_style(|style| {
		style.display = if visible {
			Display::Flex
		} else {
			Display::None
		};
	})
}

/// A one-pixel horizontal separator that stretches across a column.
pub fn Divider() -> Element {
	ColoredBox(separator_color()).with_style(|style| {
		style.height = Dimension::Points(1.0);
		style.align_self = BoxAlign::Stretch;
	})
}

/// A one-pixel vertical separator that stretches across a row's height.
pub fn VerticalDivider() -> Element {
	ColoredBox(separator_color()).with_style(|style| {
		style.width = Dimension::Points(1.0);
		style.align_self = BoxAlign::Stretch;
	})
}

/// A neutral padded, rounded surface, with no elevation or implicit
/// interaction.
pub fn Card() -> Element {
	View()
		.background(Color::rgba(255, 255, 255, 255))
		.corner_radius(12.0)
		.with_style(|style| style.padding = all_points(16.0))
}

/// A compact, rounded label; positioning beside an icon is the caller's choice.
pub fn Badge(label: impl Into<String>) -> Element {
	Center()
		.background(Color::rgba(30, 64, 175, 255))
		.color(Color::rgba(255, 255, 255, 255))
		.font(11.0, FontWeight::Semibold)
		.corner_radius(10.0)
		.with_style(|style| {
			style.min_width = Dimension::Points(20.0);
			style.min_height = Dimension::Points(20.0);
			style.padding = Rect::all_edges(
				Dimension::Points(2.0),
				Dimension::Points(6.0),
				Dimension::Points(2.0),
				Dimension::Points(6.0),
			);
			style.align_self = BoxAlign::FlexStart;
		})
		.child(Text(label))
}

/// A decorative label capsule, with no selection, deletion, or press behavior.
pub fn Chip(label: impl Into<String>) -> Element {
	Row()
		.background(Color::rgba(241, 245, 249, 255))
		.color(Color::rgba(15, 23, 42, 255))
		.font(13.0, FontWeight::Medium)
		.corner_radius(16.0)
		.with_style(|style| {
			style.align_items = BoxAlign::Center;
			style.align_self = BoxAlign::FlexStart;
			style.min_height = Dimension::Points(32.0);
			style.padding = Rect::all_edges(
				Dimension::Points(6.0),
				Dimension::Points(12.0),
				Dimension::Points(6.0),
				Dimension::Points(12.0),
			);
			style.gap = all_points(8.0);
		})
		.child(Text(label))
}

/// A row with optional leading/trailing slots and an expanding content slot.
///
/// This is a noninteractive layout. Supply actual controls in slots when
/// needed.
pub fn ListTile(leading: Option<Element>, content: Element, trailing: Option<Element>) -> Element {
	let mut tile = Row().with_style(|style| {
		style.align_items = BoxAlign::Center;
		style.padding = all_points(12.0);
		style.gap = all_points(12.0);
		style.min_height = Dimension::Points(48.0);
	});

	if let Some(leading) = leading {
		tile = tile.child(leading);
	}

	tile = tile.child(Expanded(content));

	if let Some(trailing) = trailing {
		tile = tile.child(trailing);
	}

	tile
}

/// A 56-pixel-minimum header with an expanding title and appended action slots.
///
/// System window chrome and safe-area insets remain the host's responsibility.
pub fn AppBar(title: Element) -> Element {
	Row()
		.background(Color::rgba(248, 250, 252, 255))
		.with_style(|style| {
			style.min_height = Dimension::Points(56.0);
			style.align_items = BoxAlign::Center;
			style.padding = all_points(12.0);
			style.gap = all_points(12.0);
		})
		.child(Expanded(title))
}

/// A column containing an optional header, growing body, and optional footer.
///
/// Give this box a definite height or let its parent size it. It provides no
/// navigation, overlays, safe areas, or platform window behavior.
pub fn Scaffold(app_bar: Option<Element>, body: Element, bottom_bar: Option<Element>) -> Element {
	let mut scaffold = Column();

	if let Some(app_bar) = app_bar {
		scaffold = scaffold.child(app_bar);
	}

	scaffold = scaffold.child(Expanded(body));

	if let Some(bottom_bar) = bottom_bar {
		scaffold = scaffold.child(bottom_bar);
	}

	scaffold
}

/// An eager vertical scrolling collection with stable normal child order.
///
/// Every item is constructed and laid out; this is not a virtualized list.
/// Inside a bounded column, this flex item grows or shrinks to the available
/// height, with zero automatic minimum dimensions so its content can overflow.
pub fn ListView() -> Element {
	ScrollView().with_style(|style| {
		style.flex_direction = FlexDirection::Column;
		style.overflow = Overflow::Scroll;
		style.flex_grow = 1.0;
		style.flex_shrink = 1.0;
		style.min_width = Dimension::Points(0.0);
		style.min_height = Dimension::Points(0.0);
	})
}

/// An eager scrolling grid with equal-width columns and auto-sized rows.
///
/// Every item is constructed and laid out. Like [`ListView`], this flex item
/// fills the available height of a bounded column and permits content overflow.
pub fn GridView(columns: NonZeroU8) -> Element {
	ScrollView().with_style(|style| {
		style.display = Display::Grid;
		style.overflow = Overflow::Scroll;
		style.flex_grow = 1.0;
		style.flex_shrink = 1.0;
		style.min_width = Dimension::Points(0.0);
		style.min_height = Dimension::Points(0.0);
		style.grid = Some(equal_columns(columns));
	})
}

fn equal_columns(columns: NonZeroU8) -> GridStyle {
	GridStyle {
		template_columns: vec![fraction_track(); usize::from(columns.get())],
		justify_items: BoxAlign::Stretch,
		..GridStyle::default()
	}
}

fn fraction_track() -> TrackSizing {
	TrackSizing::MinMax(Box::new(MinMax {
		min: TrackSizing::Points(0.0),
		max: TrackSizing::Fraction(1.0),
	}))
}

fn all_points(value: f32) -> Rect<Dimension> {
	Rect::all_edges(
		Dimension::Points(value),
		Dimension::Points(value),
		Dimension::Points(value),
		Dimension::Points(value),
	)
}

fn separator_color() -> Color {
	Color::rgba(203, 213, 225, 255)
}

tag!(Row => Row());
tag!(Column => Column());
tag!(Wrap => Wrap());
tag!(Center => Center());
tag!(Spacer => Spacer());
tag!(ClipRect => ClipRect());
tag!(Divider => Divider());
tag!(VerticalDivider => VerticalDivider());
tag!(Card => Card());
tag!(ListView => ListView());
