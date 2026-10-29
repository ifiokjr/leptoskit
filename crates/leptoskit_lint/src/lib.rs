//! The project rules behind `leptoskit lint`.
//!
//! rustc and clippy check Rust; these rules check the conventions a
//! leptoskit project adds on top, reading the `[lint]` table of the nearest
//! `leptoskit.toml`:
//!
//! - `component_attribute`: a capitalized function that returns a view is a
//!   component, so it carries `#[component]`.
//! - `markup_style`: component trees are written the way `[lint] markup` says —
//!   markup in `view!` by default, or builder calls.
//!
//! The rules read source, so they run in milliseconds and need no build. A
//! `// leptoskit-lint: allow(rule)` comment on the line above a finding
//! silences it there.
//!
//! ```
//! use leptoskit_lint::lint_paths;
//!
//! let report = lint_paths(&["src".into()])?;
//! for diagnostic in &report.diagnostics {
//! 	eprint!(
//! 		"{}",
//! 		leptoskit_lint::render_diagnostic(diagnostic, ".".as_ref())
//! 	);
//! }
//! # Ok::<(), leptoskit_lint::LintError>(())
//! ```
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

use std::path::PathBuf;

pub use check::BUILT_IN_COMPONENTS;
pub use check::Finding;
pub use check::Location;
pub use leptoskit_config::LintConfig;
pub use leptoskit_config::LintLevel;
pub use leptoskit_config::MarkupStyle;
pub use render::render_diagnostic;
pub use rules::Rule;
pub use walk::Report;
pub use walk::lint_paths;

mod check;
mod render;
mod rules;
mod walk;

/// A finding placed in its file, at the level the project gives its rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
	/// What was found, and where.
	pub finding: Finding,
	/// `Warn` or `Deny`; rules at `Allow` do not run.
	pub level: LintLevel,
	/// The file.
	pub path: PathBuf,
	/// The line the finding starts on, for display.
	pub source_line: String,
}

/// Why a lint run could not finish.
#[derive(Debug, thiserror::Error)]
pub enum LintError {
	/// A file or directory could not be read.
	#[error("could not read {}: {source}", path.display())]
	Read {
		/// The path.
		path: PathBuf,
		/// The underlying error.
		source: std::io::Error,
	},
	/// A directory could not be walked.
	#[error("could not walk {}: {source}", path.display())]
	Walk {
		/// The directory.
		path: PathBuf,
		/// The underlying error.
		source: ignore::Error,
	},
	/// A `leptoskit.toml` could not be loaded.
	#[error(transparent)]
	Config(#[from] leptoskit_config::ConfigError),
	/// An `exclude` glob in a `leptoskit.toml` is invalid.
	#[error("invalid `[lint] exclude` pattern `{pattern}` in {}: {source}", config.display())]
	Exclude {
		/// The `leptoskit.toml`.
		config: PathBuf,
		/// The pattern.
		pattern: String,
		/// The underlying error.
		source: globset::Error,
	},
}
