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
tag!(TextInput => TextInput());
