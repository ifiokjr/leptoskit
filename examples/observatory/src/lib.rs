//! Observatory, a desktop app for a year of stargazing: the worked example
//! for `leptoskit`'s `table` and `virtual` features.
//!
//! - [`catalogue`] is a [`DataTable`](leptoskit::table::DataTable) of the
//!   forty-four brightest stars: sortable by name, magnitude, and distance,
//!   searchable with a custom `mag<1` rule, paged into photographic plates in
//!   the observatory's own words, and a hand-built grid of the brightest three
//!   from the same row pipeline.
//! - [`log`] is a year of observations — about twenty thousand rows — under a
//!   ribbon of 365 nights. Both are
//!   [`VirtualList`](leptoskit::virtual_list::VirtualList)s, so only what is on
//!   screen exists; log rows differ in height, open when pressed, and arrive
//!   live without disturbing the rows already measured.
//!
//! [`Observatory`] holds the state and renders the screen; the tests drive it
//! through the real renderer on the headless backend, and on macOS
//! `cargo run -p observatory_example` opens it in a window.

pub mod catalogue;
pub mod log;
pub mod sky;

mod app;

pub use app::Observatory;
pub use app::VIEWPORT;
pub use app::grouped;
