//! The gallery's palette and typography, shared across platform-independent
//! screens.
//!
//! Static layout uses the framework's built-in Tailwind classes. These
//! components describe gallery content and the few dimensions that depend on
//! runtime values.

use leptoskit::prelude::*;
use leptoskit_native::Color;
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

/// A text label set in a type style and color: `<Label style=type_scale::BODY
/// color=ink>"Hello"</Label>`.
#[component]
pub fn Label(
	/// The step of the type scale.
	style: TypeStyle,
	/// The text color.
	color: Color,
	/// Extra Tailwind classes, such as `flex-1`.
	#[prop(optional, into)]
	class: Option<String>,
	/// The text.
	children: Children,
) -> Element {
	let label = Text("")
		.font(style.size, style.weight)
		.color(color)
		.children(children);
	match class {
		Some(class) => label.class(class),
		None => label,
	}
}

/// A filled circle of the given diameter.
#[component]
pub fn Circle(
	/// The diameter, in points.
	diameter: f32,
	/// The fill color.
	color: Color,
	/// Extra Tailwind classes, such as an opacity.
	#[prop(optional, into)]
	class: Option<String>,
) -> Element {
	let circle = View()
		.background(color)
		.corner_radius(diameter / 2.0)
		.with_style(|style| {
			style.width = Dimension::Points(diameter);
			style.height = Dimension::Points(diameter);
		});
	match class {
		Some(class) => circle.class(class),
		None => circle,
	}
}
