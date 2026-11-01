//! Typed file routes for leptoskit.
//!
//! Routes are files. `leptoskit routes` reads `src/routes/` and writes
//! `src/routes.rs`: a module for every route file, and one `Route` enum with a
//! variant per route. This crate is the runtime that enum stands on: URL
//! parsing, pattern matching, typed parameters and hrefs.
//!
//! A route with parameters declares a struct with `#[derive(Route)]`. A field
//! named after a `[segment]` in the file path comes from the path, and every
//! other field comes from the query string:
//!
//! ```
//! use leptoskit_router::Route;
//! use leptoskit_router::RouteParams;
//!
//! /// `src/routes/notes/[note].rs`, matching `/notes/:note`.
//! #[derive(Route, Clone, Debug, PartialEq)]
//! pub struct Note {
//! 	pub note: u64,
//! 	pub highlight: Option<u32>,
//! }
//!
//! assert_eq!(Note::FIELDS, ["note", "highlight"]);
//! ```
//!
//! The generated enum implements [`Routable`], so the app parses and writes
//! URLs through types: `Route::parse("/notes/42")` and
//! `Route::from(Note { note: 42, highlight: None }).href()`.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

mod params;
mod pattern;
mod router;
mod url;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod web;

pub use leptoskit_router_macros::Route;
pub use params::NotFound;
pub use params::ParamError;
pub use params::ParamSink;
pub use params::Params;
pub use params::RouteParams;
pub use pattern::Pattern;
pub use pattern::Segment;
pub use router::Render;
pub use router::Router;
pub use url::Href;
pub use url::RouteUrl;

/// An app's routes: implemented by the `Route` enum that `leptoskit routes`
/// generates.
pub trait Routable: Sized {
	/// The route for a parsed URL. Routes are tried from most to least
	/// specific, and a route whose path values fail to parse is skipped.
	///
	/// # Errors
	///
	/// Returns [`NoMatch`] when no route matches, which only happens without a
	/// root `+not-found.rs`.
	fn from_url(url: &RouteUrl) -> Result<Self, NoMatch>;

	/// The route's path and query string.
	fn href(&self) -> Href;

	/// Parses a path and query string the way a browser reads it, then
	/// matches it.
	///
	/// # Errors
	///
	/// Returns [`NoMatch`] when no route matches, which only happens without
	/// a root `+not-found.rs`.
	fn parse(url: &str) -> Result<Self, NoMatch> {
		Self::from_url(&RouteUrl::parse(url))
	}
}

/// No route matches the URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("no route matches the URL")]
pub struct NoMatch;

/// Support for generated code. Not part of the public API.
#[doc(hidden)]
pub mod __private {
	pub use leptoskit_native::Element;
	pub use leptoskit_native::view::Fragment;
	pub use leptoskit_native::view::IntoView;

	pub use crate::pattern::Captures;
	pub use crate::pattern::Shape;

	/// Reads a route from a shape's captures and the URL's query, or `None`
	/// when its path values do not parse.
	pub type ReadRoute<R> = fn(&Captures, &crate::RouteUrl) -> Option<R>;

	/// Reads a route struct from a matched shape's captures and the URL's
	/// query.
	pub fn decode<P: crate::RouteParams>(
		captures: &Captures,
		url: &crate::RouteUrl,
	) -> Result<P, crate::ParamError> {
		P::decode(&crate::Params::new(captures, url.query()))
	}

	/// Fails compilation with `message` when `fields` lacks `name`: the
	/// generated `routes.rs` checks each path parameter this way, so a route
	/// struct that drifts from its file name is a compile error even before
	/// the generator runs again.
	pub const fn require_field(fields: &[&str], name: &str, message: &str) {
		assert!(has_field(fields, name), "{}", message);
	}

	/// Whether `fields` contains `name`, in a `const` context.
	pub const fn has_field(fields: &[&str], name: &str) -> bool {
		let mut index = 0;

		while index < fields.len() {
			if str_eq(fields[index], name) {
				return true;
			}

			index += 1;
		}

		false
	}

	const fn str_eq(left: &str, right: &str) -> bool {
		let (left, right) = (left.as_bytes(), right.as_bytes());

		if left.len() != right.len() {
			return false;
		}

		let mut index = 0;

		while index < left.len() {
			if left[index] != right[index] {
				return false;
			}

			index += 1;
		}

		true
	}

	#[cfg(test)]
	mod tests {
		use super::has_field;

		#[test]
		fn finds_fields_in_const_context() {
			const { assert!(has_field(&["note", "highlight"], "highlight")) };
			assert!(!has_field(&["note"], "not"));
			assert!(!has_field(&[], "note"));
		}

		#[test]
		fn compares_names_byte_by_byte() {
			// Same length, different bytes: "note" is not "nope", but the
			// second field is.
			assert!(has_field(&["note", "nope"], "nope"));
			assert!(!has_field(&["note"], "nope"));
		}

		#[test]
		fn require_field_accepts_a_present_field() {
			super::require_field(&["note"], "note", "unreachable");
		}

		#[test]
		#[should_panic(expected = "`Note` needs a field named `note`")]
		fn require_field_fails_with_the_message() {
			super::require_field(&["id"], "note", "`Note` needs a field named `note`");
		}
	}
}
