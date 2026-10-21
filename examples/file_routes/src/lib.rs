//! Field, a notes app whose screens are files.
//!
//! `src/routes/` holds one file per screen, and `leptoskit routes` writes
//! `src/routes.rs` from it: a module per file and the `Route` enum. The
//! routes module is `pub` so every page stays reachable until the router
//! renders them.

pub mod routes;

/// A note's identifier in the URL.
pub type NoteId = u64;
