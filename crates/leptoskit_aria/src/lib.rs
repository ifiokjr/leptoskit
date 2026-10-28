//! Headless accessible components for the shared leptoskit element tree.
//!
//! The [declarative ARIA model](leptoskit_native::aria) gives every element
//! roles, states, relationships, and live regions, and every backend maps them
//! onto its platform's accessibility API. What it leaves to each app is the
//! widget behavior: the expanded state of a disclosure, the selection of a tab
//! list, the label, help, and error wiring of a field. This crate is that
//! layer, in the spirit of react-aria and `leptos_aria`: the component owns the
//! semantics, the app owns the styling through the same Tailwind classes every
//! leptoskit component takes.
//!
//! Components are plain functions, or small values like [`Field`], built from
//! current state plus callbacks. A screen owns its state — signals, atomics, or
//! otherwise — and rebuilds the tree when it changes; each component renders
//! the state it is given, the way the gallery screens do. The parts of one
//! widget are separate elements, so layout stays the app's choice, and compound
//! parts share a caller-chosen id prefix: [`DisclosureButton`](fn@
//! DisclosureButton) and [`DisclosurePanel`](fn@DisclosurePanel) wired by
//! `{id}-button` and `{id}-panel` agree because both derive from the same
//! `"faq"` prefix.
//!
//! The renderer's validation enforces the wiring these components declare. A
//! [`Field`] whose input is labelled by an id no element carries, or a
//! [`Dialog`] container without its title element, is a tree that fails to
//! commit — wrong accessibility is an error a test catches, not a
//! screen-reader-only surprise.
//!
//! ```
//! use leptoskit_aria::DisclosureButton;
//! use leptoskit_aria::DisclosurePanel;
//! use leptoskit_native::components::Text;
//! use leptoskit_native::components::View;
//!
//! let open = true;
//! let screen = View()
//! 	.child(DisclosureButton(
//! 		"faq",
//! 		"Frequently asked questions",
//! 		open,
//! 		|_| (),
//! 	))
//! 	.child(DisclosurePanel("faq", open).child(Text(
//! 		"An open panel keeps its id, so the button's aria-controls always resolves.",
//! 	)));
//!
//! let (button, panel) = (&screen.children[0], &screen.children[1]);
//! assert_eq!(button.props.id.as_deref(), Some("faq-button"));
//! assert_eq!(button.props.aria().expanded, Some(true));
//! assert_eq!(button.props.aria().controls, ["faq-panel"]);
//! assert_eq!(panel.props.id.as_deref(), Some("faq-panel"));
//! assert_eq!(panel.props.aria().labelled_by, ["faq-button"]);
//! ```
//!
//! The renderer has no key or focus events yet, so these components keep to
//! what press dispatch can operate: every [`Tabs`] tab stays in the tab
//! order instead of roving, and a [`Dialog`] is not marked modal because
//! focus cannot be contained inside it. The accessibility guide tracks that
//! work.

#![allow(non_snake_case)]

mod dialog;
mod disclosure;
mod field;
mod tabs;
mod toggle;

pub use dialog::Dialog;
pub use disclosure::DisclosureButton;
pub use disclosure::DisclosurePanel;
pub use field::Field;
pub use tabs::Tabs;
pub use toggle::ToggleButton;
