//! The formatter behind `leptoskit format`.
//!
//! Rust is formatted by rustfmt, nightly by default, with the project's own
//! `rustfmt.toml`. The markup inside `view!` macros, which rustfmt leaves
//! alone, is parsed with rstml and laid out by this crate at the width and
//! indentation rustfmt uses, and the Rust expressions inside that markup go
//! back through rustfmt at the columns they sit in.
//!
//! ```no_run
//! use leptoskit_fmt::Rustfmt;
//! use leptoskit_fmt::Settings;
//!
//! let rustfmt = Rustfmt::locate("nightly")?;
//! let source = "fn app() -> impl IntoView { view! { <p>\"hi\"</p> } }";
//! let output = leptoskit_fmt::format_source(source, &Settings::default(), &rustfmt)?;
//! print!("{}", output.text);
//! # Ok::<(), leptoskit_fmt::FormatError>(())
//! ```
//!
//! Formatting is idempotent, keeps every comment, and never changes what the
//! markup means: text and literals are copied exactly, braces are only
//! removed where the parser reads the value the same without them, and markup
//! that does not parse is left exactly as written with a [`Diagnostic`].
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

mod config;
mod doc;
mod engine;
mod error;
mod expr;
mod layout;
mod lexer;
mod markup;

mod rustfmt;
pub mod workspace;

#[cfg(feature = "cli")]
pub mod cli;

pub use config::AttributeBraces;
pub use config::ClosingTags;
pub use config::FormatConfig;
pub use config::Layout;
pub use config::NewlineStyle;
pub use config::Settings;
pub use config::find_rustfmt_config;
pub use engine::Output;
pub use engine::Stats;
pub use engine::format_source;
pub use error::FormatError;
pub use rustfmt::RUSTFMT_ENV;
pub use rustfmt::Rustfmt;

/// Markup the formatter left as written, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
	/// The 1-based line in the formatted text's input.
	pub line: usize,
	/// The 1-based column, in characters.
	pub column: usize,
	/// What went wrong.
	pub message: String,
}

impl std::fmt::Display for Diagnostic {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(formatter, "{}:{}: {}", self.line, self.column, self.message)
	}
}
