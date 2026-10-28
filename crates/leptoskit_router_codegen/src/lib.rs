//! Reads a leptoskit routes directory and writes its typed `routes.rs`.
//!
//! The `leptoskit routes` command and the build pipeline call this crate; apps
//! never depend on it. Names in the directory are URLs, and items in each file
//! are behaviour:
//!
//! | Name                         | Meaning                                         |
//! | ---------------------------- | ----------------------------------------------- |
//! | `index.rs`                   | the directory's own route                       |
//! | `sign-in.rs`, `notes/`       | a static segment                                |
//! | `[note].rs`, `[note]/`       | one dynamic segment                             |
//! | `[[lang]]/`                  | an optional segment                             |
//! | `[...topic].rs`              | one or more segments                            |
//! | `[[...topic]].rs`            | any number of segments                          |
//! | `(tabs)/`                    | a group: shapes the tree, not the URL           |
//! | `_layout.rs`                 | the directory's layout                          |
//! | `_note_card.rs`              | a private module beside the routes, not a route |
//! | `+not-found.rs`              | unmatched URLs at or below the directory        |
//! | `+html.rs`                   | the SSR document shell, at the top only         |
//!
//! The generator reads file names and the names of top-level items. It never
//! runs route code, and it reports every problem with its file and line.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

mod emit;
mod exports;
mod name;
mod table;
mod tree;

use std::fmt;
use std::path::Path;
use std::path::PathBuf;

pub use exports::ExportStyle;

/// A generated `routes.rs` and the route table it encodes.
#[derive(Clone, Debug)]
pub struct Generated {
	/// The source of `routes.rs`.
	pub source: String,
	/// Every route, in the order of its most specific URL: a route with
	/// optional segments is tried once for each combination of them present
	/// and absent, each at its own place in the order.
	pub routes: Vec<RouteEntry>,
}

/// One route in the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteEntry {
	/// The `Route` variant.
	pub name: String,
	/// The URL pattern, such as `/notes/:note`.
	pub pattern: String,
	/// The route file, relative to the routes directory, with `/` between
	/// names on every platform.
	pub file: String,
	/// A page, or a not-found route.
	pub kind: RouteKind,
	/// How the file writes its page: `page` or `#[component] Page`.
	pub page: ExportStyle,
}

/// What a route renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteKind {
	/// A route file's page.
	Page,
	/// A `+not-found.rs` page, for unmatched URLs.
	NotFound,
}

/// A problem in the routes directory, with the file and, when it applies, the
/// line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
	/// The file or directory with the problem.
	pub file: PathBuf,
	/// The 1-based line, when the problem is at one.
	pub line: Option<usize>,
	/// What is wrong and how to fix it.
	pub message: String,
}

impl Diagnostic {
	pub(crate) fn new(file: &Path, line: Option<usize>, message: String) -> Self {
		Self {
			file: file.to_path_buf(),
			line,
			message,
		}
	}
}

impl Diagnostic {
	/// The diagnostic with its file shown relative to `base`, with `/`
	/// between names on every platform: `src/routes/about.rs:1: ...`.
	pub fn relative_to(&self, base: &Path) -> String {
		self.render(&slash_path(
			self.file.strip_prefix(base).unwrap_or(&self.file),
		))
	}

	fn render(&self, file: &str) -> String {
		match self.line {
			Some(line) => format!("{file}:{line}: {}", self.message),
			None => format!("{file}: {}", self.message),
		}
	}
}

impl fmt::Display for Diagnostic {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.render(&self.file.display().to_string()))
	}
}

/// Every problem found in a routes directory.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.0.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))]
pub struct Diagnostics(pub Vec<Diagnostic>);

/// Generates `routes.rs` for the routes directory at `dir`.
///
/// # Errors
///
/// Returns every problem in the directory: names that are not routes, route
/// files without a page, route structs that do not match their path, and
/// routes that compete for the same URLs.
pub fn generate(dir: &Path) -> Result<Generated, Diagnostics> {
	let mut diagnostics = Vec::new();
	let Some(dir_name) = dir.file_name().and_then(|name| name.to_str()) else {
		return Err(Diagnostics(vec![Diagnostic::new(
			dir,
			None,
			"the routes directory needs a name that is UTF-8".to_owned(),
		)]));
	};
	if !dir.is_dir() {
		return Err(Diagnostics(vec![Diagnostic::new(
			dir,
			None,
			"the routes directory does not exist".to_owned(),
		)]));
	}
	let root = tree::read(dir, &mut diagnostics);
	let table = table::build(&root, dir, &mut diagnostics);
	if table.routes.is_empty() && diagnostics.is_empty() {
		diagnostics.push(Diagnostic::new(
			dir,
			None,
			"there are no route files; add `index.rs` with a `page`".to_owned(),
		));
	}
	if !diagnostics.is_empty() {
		return Err(Diagnostics(diagnostics));
	}
	let source = emit::source(&root, dir_name, &table).map_err(|message| {
		Diagnostics(vec![Diagnostic::new(
			dir,
			None,
			format!("{message}; this is a bug in leptoskit, please report it"),
		)])
	})?;
	let routes = table
		.routes
		.iter()
		.map(|route| {
			RouteEntry {
				name: route.variant.clone(),
				pattern: pattern_display(&route.segments),
				file: slash_path(route.file.strip_prefix(dir).unwrap_or(&route.file)),
				kind: route.kind,
				page: route.page,
			}
		})
		.collect();
	Ok(Generated { source, routes })
}

/// Where the generated module goes: beside the routes directory, named after
/// it, so `src/routes/` gets `src/routes.rs` and `mod routes;` finds it.
pub fn output_path(dir: &Path) -> PathBuf {
	dir.with_extension("rs")
}

/// Whether the file on disk matches what the generator writes now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
	/// The file matches. Formatting differences don't count, so running
	/// rustfmt over it keeps it current.
	Current,
	/// The file is out of date.
	Stale,
	/// There is no file yet.
	Missing,
}

/// Compares the generated module with the file at `output`.
///
/// # Errors
///
/// Returns [`WriteError::NotGenerated`] when the file exists but was not
/// written by leptoskit, and [`WriteError::Io`] when it cannot be read.
pub fn status(output: &Path, generated: &Generated) -> Result<Status, WriteError> {
	let existing = match std::fs::read_to_string(output) {
		Ok(existing) => existing,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Status::Missing),
		Err(source) => {
			return Err(WriteError::Io {
				path: output.to_path_buf(),
				source,
			});
		}
	};
	if !existing.contains(emit::MARKER) {
		return Err(WriteError::NotGenerated(output.to_path_buf()));
	}
	let current = normalized(&existing).is_some_and(|existing| {
		normalized(&generated.source).is_some_and(|generated| generated == existing)
	});
	Ok(if current {
		Status::Current
	} else {
		Status::Stale
	})
}

/// Writes the generated module to `output` unless the file there is already
/// current. Returns whether it wrote.
///
/// # Errors
///
/// Returns [`WriteError::NotGenerated`] rather than overwrite a file leptoskit
/// did not write, and [`WriteError::Io`] when the file cannot be read or
/// written.
pub fn write(output: &Path, generated: &Generated) -> Result<bool, WriteError> {
	if status(output, generated)? == Status::Current {
		return Ok(false);
	}
	std::fs::write(output, &generated.source).map_err(|source| {
		WriteError::Io {
			path: output.to_path_buf(),
			source,
		}
	})?;
	Ok(true)
}

/// Why the generated module could not be compared or written.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
	/// A file is in the way that leptoskit did not write.
	#[error(
		"{0} exists and was not generated by leptoskit; move it, or point `[router] dir` at \
		 another directory"
	)]
	NotGenerated(PathBuf),
	/// The file could not be read or written.
	#[error("could not access {path}")]
	Io {
		/// The generated module's path.
		path: PathBuf,
		/// The underlying error.
		source: std::io::Error,
	},
}

/// The source re-printed in one canonical form, with each module's items in
/// a canonical order, so whatever rustfmt changes (indentation, line breaks,
/// trailing commas, import order, braces around a closure body or match arm)
/// does not count as a change. Comments are dropped; doc comments are
/// attributes and stay.
fn normalized(source: &str) -> Option<String> {
	let mut file = syn::parse_file(source).ok()?;
	syn::visit_mut::VisitMut::visit_file_mut(&mut Unbrace, &mut file);
	sort_items(&mut file.items);
	Some(prettyplease::unparse(&file))
}

/// Replaces a closure body or match arm written as `{ expression }` with the
/// expression: rustfmt and prettyplease each add or drop those braces by line
/// length, and they mean the same thing.
struct Unbrace;

impl Unbrace {
	fn unbrace(expr: &mut syn::Expr) {
		while let syn::Expr::Block(block) = expr {
			let [syn::Stmt::Expr(inner, None)] = block.block.stmts.as_slice() else {
				return;
			};
			if !block.attrs.is_empty() || block.label.is_some() {
				return;
			}
			*expr = inner.clone();
		}
	}
}

impl syn::visit_mut::VisitMut for Unbrace {
	fn visit_expr_closure_mut(&mut self, closure: &mut syn::ExprClosure) {
		Self::unbrace(&mut closure.body);
		syn::visit_mut::visit_expr_closure_mut(self, closure);
	}

	fn visit_arm_mut(&mut self, arm: &mut syn::Arm) {
		Self::unbrace(&mut arm.body);
		syn::visit_mut::visit_arm_mut(self, arm);
	}
}

fn sort_items(items: &mut [syn::Item]) {
	for item in items.iter_mut() {
		if let syn::Item::Mod(module) = item
			&& let Some((_, inner)) = &mut module.content
		{
			sort_items(inner);
		}
	}
	items.sort_by_cached_key(|item| {
		// A file holding just this item, built from an empty parse so it
		// does not depend on which fields `syn::File` has.
		let mut single = syn::parse_file("").expect("an empty file parses");
		single.items.push(item.clone());
		prettyplease::unparse(&single)
	});
}

/// A relative path with `/` between its names on every platform, the way
/// route files are written in messages and in `leptoskit.toml`.
pub(crate) fn slash_path(path: &Path) -> String {
	path.components()
		.map(|component| component.as_os_str().to_string_lossy())
		.collect::<Vec<_>>()
		.join("/")
}

/// A pattern the way people write routes: `/notes/:note`, `/help/*topic`.
pub(crate) fn pattern_display(segments: &[name::Segment]) -> String {
	if segments.is_empty() {
		return "/".to_owned();
	}
	segments
		.iter()
		.map(|segment| {
			match segment {
				name::Segment::Static(text) => format!("/{text}"),
				name::Segment::Param(name) => format!("/:{name}"),
				name::Segment::OptionalParam(name) => format!("/:{name}?"),
				name::Segment::Rest(name) => format!("/*{name}"),
				name::Segment::OptionalRest(name) => format!("/*{name}?"),
			}
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use leptoskit_router::Pattern;

	use super::*;

	/// The generator writes patterns into variant docs, messages and the
	/// `leptoskit routes` table, and the runtime's `Pattern` displays them at
	/// run time; they must read the same.
	#[test]
	fn writes_patterns_the_way_the_runtime_displays_them() {
		use leptoskit_router::Segment as Runtime;
		use name::Segment;

		let generated = [
			Segment::OptionalParam("lang".to_owned()),
			Segment::Static("docs".to_owned()),
			Segment::Param("id".to_owned()),
			Segment::Rest("rest".to_owned()),
			Segment::OptionalRest("more".to_owned()),
		];
		let runtime = Pattern::new(&[
			Runtime::OptionalParam("lang"),
			Runtime::Static("docs"),
			Runtime::Param("id"),
			Runtime::Rest("rest"),
			Runtime::OptionalRest("more"),
		]);
		assert_eq!(pattern_display(&generated), runtime.to_string());
		assert_eq!(pattern_display(&[]), Pattern::new(&[]).to_string());
	}
}
