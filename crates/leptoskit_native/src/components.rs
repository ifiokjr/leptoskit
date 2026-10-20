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

mod layout;

pub use layout::*;

/// A container whose children use column layout by default.
pub fn View() -> Element {
	Element::view()
}

/// A text label with inherited font and foreground styling.
pub fn Text(content: impl Into<String>) -> Element {
	Element::text(content)
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
pub fn TextInput() -> Element {
	Element::text_input()
}
