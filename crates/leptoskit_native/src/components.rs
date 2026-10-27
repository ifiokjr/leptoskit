//! Built-in components shared by native apps and the web renderer.
//!
//! A screen returns one [`Element`] tree on every platform. These Rust builder
//! components accept Tailwind through [`Element::class`] by default; HTML and
//! platform widget types stay inside the renderers.
//!
//! ```
//! use leptoskit_native::components::Button;
//! use leptoskit_native::components::Text;
//! use leptoskit_native::components::View;
//!
//! let screen = View()
//! 	.class("p-6 gap-4 md:flex-row")
//! 	.child(Text("Hello, leptoskit").class("text-2xl font-bold"))
//! 	.child(Button("Continue").class("rounded-lg px-4 py-2"));
//! assert_eq!(screen.children.len(), 2);
//! ```
#![allow(non_snake_case)]

use crate::Element;
use crate::Role;

/// Declares the `view!` tag of a built-in component: a type with the
/// function's name, which has no values, whose `builder()` starts the element.
/// The tag's attributes call the [`Element`] builder method of the same name.
macro_rules! tag {
	($name:ident => $start:expr) => {
		#[doc = concat!(
			"The `view!` tag for [`", stringify!($name), "()`](fn@", stringify!($name), ").\n\n",
			"`<", stringify!($name), ">` starts from [`", stringify!($name), "::builder`], and each ",
			"attribute calls the [`Element`](crate::Element) method of the same name."
		)]
		#[allow(clippy::empty_enums)]
		pub enum $name {}

		impl $name {
			/// Starts this component's element for `view!`.
			pub fn builder() -> crate::Element {
				$start
			}
		}
	};
}

mod controls;

/// Optional layout and presentation compositions built from the core
/// primitives.
///
/// Import helpers explicitly when useful. The same layouts can be expressed
/// with [`View`] and Tailwind classes; helpers introduce no native widget
/// kinds.
pub mod helpers;

pub use controls::*;

pub use crate::vector::Affine;
pub use crate::vector::Drawing;
pub use crate::vector::FillRule;
pub use crate::vector::LineCap;
pub use crate::vector::LineJoin;
pub use crate::vector::Paint;
pub use crate::vector::PathData;
pub use crate::vector::Stroke;
pub use crate::vector::StrokeScaling;
#[cfg(feature = "svg")]
pub use crate::vector::SvgError;
pub use crate::vector::VectorAlign;
pub use crate::vector::VectorAlignment;
pub use crate::vector::VectorError;
pub use crate::vector::VectorPath;
pub use crate::vector::VectorScaling;
pub use crate::vector::ViewBox;

/// A container whose children use column layout by default.
pub fn View() -> Element {
	Element::view()
}

/// A text label with inherited font and foreground styling.
pub fn Text(content: impl Into<String>) -> Element {
	Element::text(content)
}

/// A section title at the given depth, starting at one for the page's prime
/// heading.
///
/// The web renderer emits a real heading element (`<h1>` to `<h6>` within
/// its range); native backends expose the heading trait, flag, or level.
pub fn Heading(level: u8, content: impl Into<String>) -> Element {
	Element::text(content)
		.role(Role::Heading)
		.aria_level(u32::from(level.clamp(1, 9)))
}

/// A polite live region: screen readers announce changes to its text once
/// they are idle. The usual home of status messages.
pub fn Status(content: impl Into<String>) -> Element {
	View().role(Role::Status).child(Text(content))
}

/// An assertive live region for urgent messages, announced immediately.
pub fn Alert(content: impl Into<String>) -> Element {
	View().role(Role::Alert).child(Text(content))
}

/// An accessible pressable control; register activation with `on_press`.
pub fn Button(label: impl Into<String>) -> Element {
	Element::button(label)
}

/// An image from a source understood by the platform backend.
pub fn Image(source: impl Into<String>) -> Element {
	Element::image(source)
}

/// Filled and stroked vector paths, drawn natively on every platform.
///
/// Paths are in `view_box` coordinates. The drawing scales uniformly to fit
/// the element's frame and is centred in it; pass
/// [`VectorScaling::Stretch`](crate::VectorScaling::Stretch) to
/// [`Element::scaling`] to fill the frame on both axes instead. Stroke widths
/// are in layout units and never scale. The element has no size of its own:
/// size it with classes such as `w-6 h-6`, or `absolute inset-0` to cover its
/// parent.
///
/// A drawing with an `aria_label` is an image to assistive technology; one
/// without is decorative and hidden from it.
///
/// ```
/// use leptoskit_native::Color;
/// use leptoskit_native::PathData;
/// use leptoskit_native::Stroke;
/// use leptoskit_native::VectorPath;
/// use leptoskit_native::ViewBox;
/// use leptoskit_native::components::Vector;
///
/// let ink = Color::rgba(17, 17, 17, 255);
/// let tick = Vector(ViewBox::new(0.0, 0.0, 24.0, 24.0)?)
/// 	.class("w-6 h-6")
/// 	.aria_label("Done")
/// 	.path(
/// 		VectorPath::new(PathData::parse("M4 13 L9 18 L20 6")?)
/// 			.stroke(Stroke::new(ink, 2.0)?.round()),
/// 	);
/// # Ok::<(), leptoskit_native::VectorError>(())
/// ```
pub fn Vector(view_box: ViewBox) -> Element {
	Element::vector(view_box)
}

/// A Rive scene rendered by the native host, initially paused. Set an explicit
/// size, label or decorative state, and `running(true)` when it should animate.
pub fn Rive(playback: leptoskit_rive::Playback) -> Element {
	Element::rive(playback)
}

tag!(Rive => Element::new(crate::NativeKind::Rive));

/// A container whose overflowing children can scroll.
pub fn ScrollView() -> Element {
	Element::scroll_view()
}

/// An editable single-line text field.
///
/// It sizes itself from its displayed text, plus 12 layout units of height
/// for the native text-field chrome, so an unstyled field at the default font
/// size is 28 tall. A background, border, or corner radius replaces that
/// chrome, and the field is then as tall as its text plus its padding. A
/// height class such as `h-10` replaces the measured height.
pub fn TextInput() -> Element {
	Element::text_input()
}

tag!(View => View());
tag!(Text => Text(""));
tag!(Heading => Heading(2, ""));
tag!(Status => Status(""));
tag!(Alert => Alert(""));
tag!(Button => Button(""));
tag!(Image => Image(""));
tag!(ScrollView => ScrollView());
tag!(Vector => Vector(ViewBox::UNIT));
tag!(TextInput => TextInput());
