use std::fmt;
use std::io;
use std::path::PathBuf;

/// A problem in the documentation sources, at a place in one file.
///
/// Every check in this crate reports through diagnostics instead of stopping
/// at the first problem, so one build lists everything that needs fixing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
	/// The source file, relative to the docs root. `None` for a document
	/// rendered on its own, outside a [`Site`](crate::Site).
	pub path: Option<String>,
	/// The 1-indexed line, or 0 when the problem has no position.
	pub line: usize,
	/// The 1-indexed column, or 0 when the problem has no position.
	pub column: usize,
	/// What is wrong, written for the author of the document.
	pub message: String,
}

impl Diagnostic {
	pub(crate) fn new(line: usize, column: usize, message: impl Into<String>) -> Self {
		Self {
			path: None,
			line,
			column,
			message: message.into(),
		}
	}

	pub(crate) fn in_file(mut self, path: &str) -> Self {
		self.path = Some(path.to_owned());
		self
	}
}

impl fmt::Display for Diagnostic {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if let Some(path) = &self.path {
			write!(f, "{path}:")?;
		}

		if self.line > 0 {
			write!(f, "{}:{}:", self.line, self.column)?;
		}

		if self.path.is_some() || self.line > 0 {
			f.write_str(" ")?;
		}

		f.write_str(&self.message)
	}
}

/// Why building or serving a site failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// The sources have problems: broken Markdown, unknown components, bad
	/// props, or links to pages and headings that do not exist.
	#[error("{}", list(.0))]
	Docs(Vec<Diagnostic>),
	/// Reading the sources or writing the output failed.
	#[error("could not {action} `{}`: {source}", path.display())]
	Io {
		/// What was being done, such as "read" or "write".
		action: &'static str,
		/// The file or directory involved.
		path: PathBuf,
		/// The underlying failure.
		#[source]
		source: io::Error,
	},
	/// The output directory holds files this crate did not write, so
	/// cleaning it could delete something that matters.
	#[error(
		"refusing to build into `{}`: it is not empty and was not built by leptoskit_docs",
		.0.display()
	)]
	ForeignOutput(PathBuf),
	/// The development server could not listen on its address.
	#[error("could not serve the docs: {0}")]
	Listen(#[source] io::Error),
	/// The development server could not watch the book for changes.
	#[error("could not watch `{}` for changes: {source}", path.display())]
	Watch {
		/// The book's directory.
		path: PathBuf,
		/// The underlying failure.
		#[source]
		source: notify::Error,
	},
}

impl Error {
	pub(crate) fn io(
		action: &'static str,
		path: impl Into<PathBuf>,
	) -> impl FnOnce(io::Error) -> Self {
		let path = path.into();
		move |source| {
			Self::Io {
				action,
				path,
				source,
			}
		}
	}
}

fn list(diagnostics: &[Diagnostic]) -> String {
	let count = diagnostics.len();
	let noun = if count == 1 { "problem" } else { "problems" };
	let mut message = format!("the docs have {count} {noun}:");

	for diagnostic in diagnostics {
		message.push_str("\n  ");
		message.push_str(&diagnostic.to_string());
	}

	message
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn displays_whatever_position_is_known() {
		let bare = Diagnostic::new(0, 0, "no title");
		let placed = Diagnostic::new(3, 7, "unknown component");
		let filed = Diagnostic::new(3, 7, "unknown component").in_file("guide/a.md");
		let filed_only = Diagnostic::new(0, 0, "no title").in_file("guide/a.md");

		assert_eq!(bare.to_string(), "no title");
		assert_eq!(placed.to_string(), "3:7: unknown component");
		assert_eq!(filed.to_string(), "guide/a.md:3:7: unknown component");
		assert_eq!(filed_only.to_string(), "guide/a.md: no title");
	}

	#[test]
	fn lists_every_diagnostic() {
		let one = Error::Docs(vec![Diagnostic::new(1, 1, "a")]);
		let two = Error::Docs(vec![Diagnostic::new(1, 1, "a"), Diagnostic::new(2, 1, "b")]);

		assert_eq!(one.to_string(), "the docs have 1 problem:\n  1:1: a");
		assert_eq!(
			two.to_string(),
			"the docs have 2 problems:\n  1:1: a\n  2:1: b"
		);
	}

	#[test]
	fn names_the_path_of_io_failures() {
		let error = Error::io("read", "docs/SUMMARY.md")(io::Error::other("gone"));

		assert_eq!(error.to_string(), "could not read `docs/SUMMARY.md`: gone");
	}
}
