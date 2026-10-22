//! What can stop a file from being formatted.

use std::path::PathBuf;

/// Why a file could not be formatted. Markup that cannot be parsed is not an
/// error: it is left as written and reported as a [`Diagnostic`].
///
/// [`Diagnostic`]: crate::Diagnostic
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
	/// A file could not be read or written.
	#[error("{}: {source}", path.display())]
	Io {
		/// The file.
		path: PathBuf,
		/// The underlying error.
		source: std::io::Error,
	},
	/// A configuration file could not be read or parsed.
	#[error("invalid configuration in {}: {message}", path.display())]
	Config {
		/// The configuration file.
		path: PathBuf,
		/// What is wrong with it.
		message: String,
	},
	/// The toolchain has no rustfmt.
	#[error(
		"no rustfmt for the `{toolchain}` toolchain: {message}\nInstall it with `rustup component \
		 add rustfmt --toolchain {toolchain}`, or pass `--stable`."
	)]
	RustfmtMissing {
		/// The toolchain asked for.
		toolchain: String,
		/// What rustup said.
		message: String,
	},
	/// rustfmt rejected the file, usually for a syntax error.
	#[error("rustfmt failed: {0}")]
	Rustfmt(String),
	/// The file is not UTF-8.
	#[error("{}: the file is not valid UTF-8", path.display())]
	NotUtf8 {
		/// The file.
		path: PathBuf,
	},
}
