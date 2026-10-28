//! Reads a leptoskit `locales/` directory of ARB files and writes its typed
//! translations module.
//!
//! The `leptoskit l10n` command and the build pipeline call this crate; apps
//! never depend on it. Each `<tag>.arb` file holds one locale's messages,
//! the way Flutter's `gen_l10n` reads them:
//!
//! | In `locales/en.arb`                                                    | Meaning                                                |
//! | ---------------------------------------------------------------------- | ------------------------------------------------------ |
//! | `"appTitle": "Field Notes"`                                            | a message, read with `app_title()`                     |
//! | `"greeting": "Hello, {name}!"`                                         | a placeholder, so `greeting(name: &str)`               |
//! | `"@greeting": { "description": "…", "placeholders": { "name": {…} } }` | what the message and its placeholders are              |
//! | `"items": "{count, plural, =0 {none} one {# item} other {# items}}"`   | a plural, so `items(count: i64)`                       |
//! | `"pronoun": "{who, select, female {she} male {he} other {they}}"`      | a select, so `pronoun(who: &str)`                      |
//! | `"@@locale": "en"`                                                     | the file's locale, which must match the file's name    |
//!
//! The template file, `en.arb` unless configured, defines every message and
//! the types of its placeholders; the other files translate them, and a
//! message a file leaves out falls back to the template's text. The generator
//! never runs app code, and it reports every problem with its file and line.
#![deny(missing_docs)]
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

mod arb;
mod catalog;
mod emit;
mod message;

use std::fmt;
use std::path::Path;
use std::path::PathBuf;

/// A generated translations module and what it holds.
#[derive(Clone, Debug)]
pub struct Generated {
	/// The source of the module.
	pub source: String,
	/// Every locale, the template first.
	pub locales: Vec<LocaleEntry>,
	/// Every message, by key.
	pub messages: Vec<MessageEntry>,
}

/// One locale's file and how complete it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocaleEntry {
	/// The canonical tag, such as `pt-BR`.
	pub tag: String,
	/// The file, relative to the locales directory.
	pub file: String,
	/// Whether this is the template every other file translates.
	pub template: bool,
	/// The keys the file leaves out, which fall back to the template.
	pub untranslated: Vec<String>,
}

/// One message and the method it became.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageEntry {
	/// The key in the ARB files.
	pub key: String,
	/// The method's signature, such as `greeting(name: &str) -> String`.
	pub signature: String,
}

/// A problem in the locales directory, with the file and, when it applies,
/// the line.
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

	/// The diagnostic with its file shown relative to `base`, with `/`
	/// between names on every platform: `locales/pt-BR.arb:4: ...`. A file
	/// outside `base` is shown as it is.
	#[must_use]
	pub fn relative_to(&self, base: &Path) -> String {
		match self.file.strip_prefix(base) {
			Ok(relative) => self.render(&slash_path(relative)),
			Err(_) => self.to_string(),
		}
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

/// Every problem found in a locales directory.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}", self.0.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))]
pub struct Diagnostics(pub Vec<Diagnostic>);

/// Generates the translations module for the ARB files in `dir`, with the
/// locale tagged `template` as the template.
///
/// # Errors
///
/// Returns every problem in the directory, by file and then by line: files
/// that are not named after a locale, JSON that does not parse, messages
/// that are not well-formed, keys and placeholders that cannot become Rust
/// names, `=N` forms too large for their placeholder's type, placeholders
/// whose types do not fit how they are used, and translations that do not
/// match the template's placeholders.
pub fn generate(dir: &Path, template: &str) -> Result<Generated, Diagnostics> {
	let mut diagnostics = Vec::new();
	let Some(dir_name) = dir.file_name().and_then(|name| name.to_str()) else {
		return Err(Diagnostics(vec![Diagnostic::new(
			dir,
			None,
			"the locales directory needs a name that is UTF-8".to_owned(),
		)]));
	};
	if !dir.is_dir() {
		return Err(Diagnostics(vec![Diagnostic::new(
			dir,
			None,
			format!("the locales directory does not exist; create it with `{template}.arb` inside"),
		)]));
	}
	let files = arb::read_dir(dir, &mut diagnostics);
	let catalog = catalog::build(files, dir, template, &mut diagnostics);
	let Some(catalog) = catalog.filter(|_| diagnostics.is_empty()) else {
		// By file, then by line, the way an editor lists them; problems at one
		// line stay in the order they were found.
		diagnostics.sort_by(|left, right| (&left.file, left.line).cmp(&(&right.file, right.line)));
		return Err(Diagnostics(diagnostics));
	};
	let source = emit::source(&catalog, dir_name);
	let locales = catalog
		.locales
		.iter()
		.map(|locale| {
			LocaleEntry {
				tag: locale.tag.clone(),
				file: locale.file.clone(),
				template: locale.tag == catalog.template().tag,
				untranslated: locale.untranslated.clone(),
			}
		})
		.collect();
	let messages = catalog
		.messages
		.iter()
		.map(|message| {
			MessageEntry {
				key: message.key.clone(),
				signature: message.signature(),
			}
		})
		.collect();
	Ok(Generated {
		source,
		locales,
		messages,
	})
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
		"{0} exists and was not generated by leptoskit; move it, or point `[l10n] output` at \
		 another file"
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

/// The source re-printed in one canonical form, with its items in a
/// canonical order, so whatever rustfmt changes (indentation, line breaks,
/// trailing commas, braces around a closure body or match arm, rewrapped
/// documentation, split string literals) does not count as a change.
/// Comments are dropped; doc comments are attributes and stay.
fn normalized(source: &str) -> Option<String> {
	let mut file = syn::parse_file(source).ok()?;
	syn::visit_mut::VisitMut::visit_file_mut(&mut Canonical, &mut file);
	file.items.sort_by_cached_key(|item| {
		let mut single = syn::parse_file("").expect("an empty file parses");
		single.items.push(item.clone());
		prettyplease::unparse(&single)
	});
	Some(prettyplease::unparse(&file))
}

/// Rewrites what rustfmt and prettyplease each write their own way, and what
/// rustfmt's options rewrite, into one form that means the same thing.
struct Canonical;

impl Canonical {
	/// Replaces a closure body or match arm written as `{ expression }` with
	/// the expression: rustfmt and prettyplease each add or drop those braces
	/// by line length.
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

	/// The text of a `#[doc = "..."]` attribute.
	fn doc(attribute: &syn::Attribute) -> Option<String> {
		let syn::Meta::NameValue(meta) = &attribute.meta else {
			return None;
		};
		let syn::Expr::Lit(syn::ExprLit {
			lit: syn::Lit::Str(text),
			..
		}) = &meta.value
		else {
			return None;
		};
		meta.path.is_ident("doc").then(|| text.value())
	}

	/// A string literal written as its value: rustfmt's `format_strings`
	/// splits long literals with `\` and a line break.
	fn literal(literal: proc_macro2::Literal) -> proc_macro2::Literal {
		let token = proc_macro2::TokenTree::Literal(literal.clone());
		match syn::parse2::<syn::LitStr>(token.into()) {
			Ok(text) => proc_macro2::Literal::string(&text.value()),
			Err(_) => literal,
		}
	}

	/// A macro's tokens with their string literals written as their values.
	fn tokens(tokens: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
		tokens
			.into_iter()
			.map(|token| {
				match token {
					proc_macro2::TokenTree::Literal(literal) => {
						proc_macro2::TokenTree::Literal(Self::literal(literal))
					}
					proc_macro2::TokenTree::Group(group) => {
						let mut canonical = proc_macro2::Group::new(
							group.delimiter(),
							Self::tokens(group.stream()),
						);
						canonical.set_span(group.span());
						proc_macro2::TokenTree::Group(canonical)
					}
					other => other,
				}
			})
			.collect()
	}
}

impl syn::visit_mut::VisitMut for Canonical {
	fn visit_expr_closure_mut(&mut self, closure: &mut syn::ExprClosure) {
		Self::unbrace(&mut closure.body);
		syn::visit_mut::visit_expr_closure_mut(self, closure);
	}

	fn visit_arm_mut(&mut self, arm: &mut syn::Arm) {
		Self::unbrace(&mut arm.body);
		syn::visit_mut::visit_arm_mut(self, arm);
	}

	/// Joins an item's documentation into one attribute, with every run of
	/// whitespace one space: rustfmt's `wrap_comments` rewraps it, and the
	/// words are what count.
	fn visit_attributes_mut(&mut self, attributes: &mut Vec<syn::Attribute>) {
		let mut words = Vec::new();
		attributes.retain(|attribute| {
			let Some(text) = Self::doc(attribute) else {
				return true;
			};
			words.extend(text.split_whitespace().map(str::to_owned));
			false
		});
		if !words.is_empty() {
			let text = words.join(" ");
			attributes.insert(0, syn::parse_quote!(#[doc = #text]));
		}
		for attribute in attributes {
			self.visit_attribute_mut(attribute);
		}
	}

	fn visit_lit_str_mut(&mut self, literal: &mut syn::LitStr) {
		*literal = syn::LitStr::new(&literal.value(), literal.span());
	}

	fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
		mac.tokens = Self::tokens(std::mem::take(&mut mac.tokens));
		syn::visit_mut::visit_macro_mut(self, mac);
	}
}

/// A relative path with `/` between its names on every platform, the way
/// locale files are written in messages.
pub(crate) fn slash_path(path: &Path) -> String {
	path.components()
		.map(|component| component.as_os_str().to_string_lossy())
		.collect::<Vec<_>>()
		.join("/")
}
