//! The gallery's palette and typography, shared across platform-independent
//! screens.
//!
//! Static layout uses the framework's built-in Tailwind classes. These helpers
//! describe gallery content and the few dimensions that depend on runtime
//! values.

use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_yoga::Dimension;

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
	Text(content).font(style.size, style.weight).color(color)
}

/// A flexible empty view that absorbs free space along its parent's axis.
pub fn spacer() -> Element {
	View().class("flex-1")
}

/// A one-point rule in the given color.
pub fn hairline(color: Color) -> Element {
	View().background(color).class("h-[1px]")
}

/// A filled circle of the given diameter.
pub fn circle(diameter: f32, color: Color) -> Element {
	View()
		.background(color)
		.corner_radius(diameter / 2.0)
		.with_style(|style| {
			style.width = Dimension::Points(diameter);
			style.height = Dimension::Points(diameter);
		})
}
