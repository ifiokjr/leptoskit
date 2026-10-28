//! Headless list virtualization for the shared leptoskit element tree.
//!
//! This is `TanStack Virtual`'s model, ported to leptoskit: a
//! [`Virtualizer`] holds the windowing math over a count, a size estimate
//! per item, and the scroll state, and returns exactly the items a scrolled
//! viewport intersects — never the whole list. The same core drives a
//! vertical or horizontal container, and every number it produces is a
//! layout unit the yoga engine and each native backend already understand.
//!
//! The loop is three moves, and the screen owns all of it:
//!
//! 1. feed the virtualizer the viewport size and the scroll offset the surface
//!    reports, with [`set_viewport`](Virtualizer::set_viewport) and
//!    [`scroll_to`](Virtualizer::scroll_to);
//! 2. render the window with [`VirtualList`], or read
//!    [`items`](Virtualizer::items) and [`total_size`](Virtualizer::total_size)
//!    for a container of your own;
//! 3. when the renderer measures a real size, hand it back with
//!    [`measure`](Virtualizer::measure) — a changed size shifts every item
//!    after it, and only those items resolve again.
//!
//! Sizes start from an [`Estimate`] — one fixed number, or a function of the
//! index — and resolve to real sizes as they are measured. An estimate that
//! returns anything but a finite positive size is an error the getters
//! report by index, never a silently clamped offset.
//!
//! ```
//! use leptoskit_native::components::Text;
//! use leptoskit_native::components::View;
//! use leptoskit_virtual::Axis;
//! use leptoskit_virtual::VirtualList;
//! use leptoskit_virtual::VirtualOptions;
//! use leptoskit_virtual::Virtualizer;
//!
//! fn contacts(offset: f64) -> leptoskit_native::Element {
//! 	let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(10_000, 56.0));
//! 	virtualizer.set_viewport(644.0);
//! 	virtualizer.scroll_to(offset);
//!
//! 	VirtualList(
//! 		&virtualizer,
//! 		Axis::Vertical,
//! 		|_| (),
//! 		|item| View().child(Text(format!("Contact {}", item.index))),
//! 	)
//! 	.expect("a fixed estimate is valid")
//! }
//!
//! let top = contacts(0.0);
//! assert_eq!(top.children[0].children.len(), 12);
//! let deep = contacts(280_000.0);
//! assert_eq!(deep.children[0].children.len(), 12);
//! ```

// Component builders keep their PascalCase names, like the built-ins.
#![allow(non_snake_case)]

mod list;
mod virtualizer;

pub use list::Axis;
pub use list::VirtualList;
pub use virtualizer::Align;
pub use virtualizer::Estimate;
pub use virtualizer::InvalidEstimate;
pub use virtualizer::MeasureError;
pub use virtualizer::OffsetError;
pub use virtualizer::OutOfRange;
pub use virtualizer::VirtualItem;
pub use virtualizer::VirtualOptions;
pub use virtualizer::Virtualizer;
