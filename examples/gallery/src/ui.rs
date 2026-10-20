//! Layout shorthands and the design scale every screen shares.
//!
//! Screens are plain [`Element`] trees; these helpers only make the trees
//! read like the interface they describe.

use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_yoga::Align;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::JustifyContent;
use leptoskit_yoga::PositionType;
use leptoskit_yoga::Rect;

/// The spacing scale, in points. Every gap, padding, and margin is one of
/// these.
pub mod space {
	/// Hairline gaps between tightly related items.
	pub const XXS: f32 = 2.0;
	/// Icon-to-label gaps.
	pub const XS: f32 = 4.0;
	/// Gaps inside a component.
	pub const S: f32 = 8.0;
	/// Gaps between a component's groups.
	pub const M: f32 = 12.0;
	/// Standard content inset.
	pub const L: f32 = 16.0;
	/// Section spacing.
	pub const XL: f32 = 24.0;
	/// Spacing between major regions.
	pub const XXL: f32 = 32.0;
}

/// A step of the type scale.
#[derive(Debug, Clone, Copy)]
pub struct TypeStyle {
	/// The font size, in points.
	pub size: f32,
	/// The font weight.
	pub weight: FontWeight,
}

/// The type scale, shared by every screen.
pub mod type_scale {
	use leptoskit_native::FontWeight;

	use super::TypeStyle;

	/// Captions, timestamps, and fine print.
	pub const CAPTION: TypeStyle = TypeStyle {
		size: 12.0,
		weight: FontWeight::Regular,
	};
	/// Small uppercase labels above titles.
	pub const EYEBROW: TypeStyle = TypeStyle {
		size: 12.0,
		weight: FontWeight::Semibold,
	};
	/// Secondary text.
	pub const FOOTNOTE: TypeStyle = TypeStyle {
		size: 13.0,
		weight: FontWeight::Regular,
	};
	/// Body text.
	pub const BODY: TypeStyle = TypeStyle {
		size: 15.0,
		weight: FontWeight::Regular,
	};
	/// Emphasized body text and control labels.
	pub const BODY_STRONG: TypeStyle = TypeStyle {
		size: 15.0,
		weight: FontWeight::Semibold,
	};
	/// Row titles and headlines.
	pub const HEADLINE: TypeStyle = TypeStyle {
		size: 17.0,
		weight: FontWeight::Semibold,
	};
	/// Section titles.
	pub const TITLE: TypeStyle = TypeStyle {
		size: 22.0,
		weight: FontWeight::Bold,
	};
	/// Screen titles.
	pub const LARGE_TITLE: TypeStyle = TypeStyle {
		size: 32.0,
		weight: FontWeight::Bold,
	};
}

/// A color from a `#rrggbb` literal.
///
/// # Panics
///
/// Panics on a malformed literal, which is a typo in a screen definition.
pub fn hex(value: &str) -> Color {
	Color::hex(value).unwrap_or_else(|| panic!("`{value}` is not a #rrggbb color"))
}

/// The same color at a different opacity, from `0.0` to `1.0`.
pub fn alpha(color: Color, opacity: f32) -> Color {
	Color::rgba(
		color.red(),
		color.green(),
		color.blue(),
		(opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
	)
}

/// A text label set in a type style and color.
pub fn label(content: impl Into<String>, style: TypeStyle, color: Color) -> Element {
	Element::text(content)
		.font(style.size, style.weight)
		.color(color)
}

/// A flexible empty view that absorbs free space along its parent's axis.
pub fn spacer() -> Element {
	Element::view().grow()
}

/// A one-point rule in the given color.
pub fn hairline(color: Color) -> Element {
	Element::view()
		.background(color)
		.with_style(|style| style.height = Dimension::Points(1.0))
}

/// A filled circle of the given diameter.
pub fn circle(diameter: f32, color: Color) -> Element {
	Element::view()
		.background(color)
		.corner_radius(diameter / 2.0)
		.size(diameter, diameter)
}

/// The same value on every edge.
pub fn edges(value: f32) -> Rect<Dimension> {
	Rect::all_edges(
		Dimension::Points(value),
		Dimension::Points(value),
		Dimension::Points(value),
		Dimension::Points(value),
	)
}

/// Layout shorthands on [`Element`].
pub trait Layout: Sized {
	/// Lays the children out left to right, vertically centered.
	#[must_use]
	fn row(self) -> Self;
	/// Lays the children out top to bottom, stretched to the full width.
	#[must_use]
	fn column(self) -> Self;
	/// Spaces the children apart on both axes.
	#[must_use]
	fn gap(self, gap: f32) -> Self;
	/// Insets the content: `vertical` top and bottom, `horizontal` left and
	/// right.
	#[must_use]
	fn padding(self, vertical: f32, horizontal: f32) -> Self;
	/// Insets the content on each edge separately.
	#[must_use]
	fn padding_edges(self, top: f32, right: f32, bottom: f32, left: f32) -> Self;
	/// Fixes the width.
	#[must_use]
	fn width(self, width: f32) -> Self;
	/// Fixes the height.
	#[must_use]
	fn height(self, height: f32) -> Self;
	/// Fixes both dimensions.
	#[must_use]
	fn size(self, width: f32, height: f32) -> Self;
	/// Fills the parent on both axes.
	#[must_use]
	fn fill(self) -> Self;
	/// Caps the width at a fraction of the parent's width.
	#[must_use]
	fn max_width_fraction(self, fraction: f32) -> Self;
	/// Keeps `width / height` at `ratio`.
	#[must_use]
	fn aspect_ratio(self, ratio: f32) -> Self;
	/// Offsets this element from the previous sibling.
	#[must_use]
	fn margin_top(self, margin: f32) -> Self;
	/// Takes the element out of the flow and pins its top-left corner to
	/// the parent's padding box.
	#[must_use]
	fn pinned(self, top: f32, left: f32) -> Self;
	/// Takes an equal share of the free space along the parent's axis.
	#[must_use]
	fn grow(self) -> Self;
	/// Aligns the children on the cross axis.
	#[must_use]
	fn align(self, align: Align) -> Self;
	/// Aligns this element on its parent's cross axis.
	#[must_use]
	fn align_self(self, align: Align) -> Self;
	/// Distributes the children along the main axis.
	#[must_use]
	fn justify(self, justify: JustifyContent) -> Self;
}

impl Layout for Element {
	fn row(self) -> Self {
		self.with_style(|style| {
			style.flex_direction = FlexDirection::Row;
			style.align_items = Align::Center;
		})
	}

	fn column(self) -> Self {
		self.with_style(|style| {
			style.flex_direction = FlexDirection::Column;
			style.align_items = Align::Stretch;
		})
	}

	fn gap(self, gap: f32) -> Self {
		self.with_style(|style| style.gap = edges(gap))
	}

	fn padding(self, vertical: f32, horizontal: f32) -> Self {
		self.padding_edges(vertical, horizontal, vertical, horizontal)
	}

	fn padding_edges(self, top: f32, right: f32, bottom: f32, left: f32) -> Self {
		self.with_style(|style| {
			style.padding = Rect::all_edges(
				Dimension::Points(top),
				Dimension::Points(right),
				Dimension::Points(bottom),
				Dimension::Points(left),
			);
		})
	}

	fn width(self, width: f32) -> Self {
		self.with_style(|style| style.width = Dimension::Points(width))
	}

	fn height(self, height: f32) -> Self {
		self.with_style(|style| style.height = Dimension::Points(height))
	}

	fn size(self, width: f32, height: f32) -> Self {
		self.width(width).height(height)
	}

	fn fill(self) -> Self {
		self.with_style(|style| {
			style.width = Dimension::Percent(1.0);
			style.height = Dimension::Percent(1.0);
		})
	}

	fn max_width_fraction(self, fraction: f32) -> Self {
		self.with_style(|style| style.max_width = Dimension::Percent(fraction))
	}

	fn aspect_ratio(self, ratio: f32) -> Self {
		self.with_style(|style| style.aspect_ratio = Some(ratio))
	}

	fn margin_top(self, margin: f32) -> Self {
		self.with_style(|style| style.margin.top = Dimension::Points(margin))
	}

	fn pinned(self, top: f32, left: f32) -> Self {
		self.with_style(|style| {
			style.position = PositionType::Absolute;
			style.inset.top = Dimension::Points(top);
			style.inset.left = Dimension::Points(left);
		})
	}

	fn grow(self) -> Self {
		self.with_style(|style| {
			style.flex_grow = 1.0;
			style.flex_shrink = 1.0;
			style.flex_basis = Dimension::Points(0.0);
		})
	}

	fn align(self, align: Align) -> Self {
		self.with_style(|style| style.align_items = align)
	}

	fn align_self(self, align: Align) -> Self {
		self.with_style(|style| style.align_self = align)
	}

	fn justify(self, justify: JustifyContent) -> Self {
		self.with_style(|style| style.justify_content = justify)
	}
}
