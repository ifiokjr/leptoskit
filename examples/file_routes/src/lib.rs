//! Field, a notes app whose screens are files.
//!
//! `src/routes/` holds one file per screen, and `leptoskit routes` writes
//! `src/routes.rs` from it: a module per file and the `Route` enum. A
//! [`Navigation`](routes::Navigation) router renders the current route inside
//! its layouts:
//!
//! ```
//! use file_routes_example::routes::Navigation;
//! use file_routes_example::routes::Route;
//!
//! let navigation = Navigation::new(Route::Notes);
//! let screen = navigation.render();
//! assert!(!screen.children.is_empty());
//! ```

pub mod routes;

/// A note's identifier in the URL.
pub type NoteId = u64;
