//! Linting files and directories: finding the Rust sources, the
//! `leptoskit.toml` that configures each one, and checking them in parallel.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use globset::Glob;
use globset::GlobSet;
use globset::GlobSetBuilder;
use leptoskit_config::LintConfig;
use leptoskit_config::LintLevel;
use rayon::prelude::*;

use crate::Diagnostic;
use crate::LintError;
use crate::check::ComponentNames;
use crate::check::check_file;

/// What a run over a set of paths found.
#[derive(Debug, Default)]
pub struct Report {
	/// Every finding, ordered by file and position.
	pub diagnostics: Vec<Diagnostic>,
	/// Files that do not parse as Rust. The compiler reports why; the rules
	/// skip them.
	pub unparsed: Vec<PathBuf>,
	/// How many files were checked.
	pub files: usize,
}

impl Report {
	/// How many findings are warnings.
	#[must_use]
	pub fn warnings(&self) -> usize {
		self.count(LintLevel::Warn)
	}

	/// How many findings are errors.
	#[must_use]
	pub fn errors(&self) -> usize {
		self.count(LintLevel::Deny)
	}

	/// Whether the run fails: any error, or any warning when warnings are
	/// denied.
	#[must_use]
	pub fn failed(&self, deny_warnings: bool) -> bool {
		self.errors() > 0 || (deny_warnings && self.warnings() > 0)
	}

	fn count(&self, level: LintLevel) -> usize {
		self.diagnostics
			.iter()
			.filter(|diagnostic| diagnostic.level == level)
			.count()
	}
}

/// The `[lint]` configuration that applies to one directory, and the globs
/// it excludes.
#[derive(Debug)]
struct Scope {
	config: LintConfig,
	/// The directory holding the `leptoskit.toml`, which `exclude` globs are
	/// relative to; `None` when no file configures the directory.
	root: Option<PathBuf>,
	exclude: GlobSet,
}

impl Scope {
	fn excludes(&self, path: &Path) -> bool {
		self.root
			.as_deref()
			.and_then(|root| path.strip_prefix(root).ok())
			.is_some_and(|relative| self.exclude.is_match(relative))
	}
}

/// Finds and caches the scope of each directory.
#[derive(Default)]
struct Scopes {
	by_config: Mutex<HashMap<Option<PathBuf>, Arc<Scope>>>,
}

impl Scopes {
	fn for_file(&self, path: &Path) -> Result<Arc<Scope>, LintError> {
		let config_path = path
			.parent()
			.into_iter()
			.flat_map(Path::ancestors)
			.map(|dir| dir.join(leptoskit_config::CONFIG_FILE))
			.find(|candidate| candidate.is_file());
		let mut scopes = self
			.by_config
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		if let Some(scope) = scopes.get(&config_path) {
			return Ok(Arc::clone(scope));
		}
		let scope = Arc::new(load_scope(config_path.as_deref())?);
		scopes.insert(config_path, Arc::clone(&scope));
		Ok(scope)
	}
}

fn load_scope(config_path: Option<&Path>) -> Result<Scope, LintError> {
	let Some(config_path) = config_path else {
		return Ok(Scope {
			config: LintConfig::default(),
			root: None,
			exclude: GlobSet::empty(),
		});
	};
	let config = LintConfig::load_from_file(config_path)?;
	let mut exclude = GlobSetBuilder::new();
	for pattern in &config.exclude {
		let glob = Glob::new(pattern).map_err(|source| {
			LintError::Exclude {
				config: config_path.to_owned(),
				pattern: pattern.clone(),
				source,
			}
		})?;
		exclude.add(glob);
	}
	let exclude = exclude.build().map_err(|source| {
		LintError::Exclude {
			config: config_path.to_owned(),
			pattern: config.exclude.join(", "),
			source,
		}
	})?;
	Ok(Scope {
		config,
		root: config_path.parent().map(Path::to_owned),
		exclude,
	})
}

/// Lints every Rust file under `paths`, which may be files or directories.
///
/// Directories are walked the way git sees them: ignored and hidden files
/// are skipped. Each file follows the `[lint]` table of the nearest
/// `leptoskit.toml` above it, or the defaults without one.
///
/// # Errors
///
/// Returns an error when a path or file cannot be read, or a
/// `leptoskit.toml` has an invalid `[lint]` table.
pub fn lint_paths(paths: &[PathBuf]) -> Result<Report, LintError> {
	let files = rust_files(paths)?;
	let scopes = Scopes::default();
	let sources = files
		.into_par_iter()
		.map(|path| {
			let path = std::path::absolute(&path).unwrap_or(path);
			let scope = scopes.for_file(&path)?;
			if scope.excludes(&path) {
				return Ok(None);
			}
			let source = std::fs::read_to_string(&path).map_err(|source| {
				LintError::Read {
					path: path.clone(),
					source,
				}
			})?;
			if is_generated(&source) {
				return Ok(None);
			}
			// Components declared in one file are called from others, so every
			// file's components are known before any file is checked. syn's
			// trees cannot cross threads, so the check parses again.
			let components = syn::parse_file(&source).ok().map(|parsed| {
				let mut components = ComponentNames::default();
				components.collect(&parsed);
				components
			});
			Ok(Some(SourceFile {
				path,
				source,
				components,
				scope,
			}))
		})
		.filter_map(Result::transpose)
		.collect::<Result<Vec<_>, LintError>>()?;

	let mut components = ComponentNames::built_in();
	for file in &sources {
		if let Some(declared) = &file.components {
			components.extend(declared);
		}
	}

	let mut report = Report {
		files: sources.len(),
		..Report::default()
	};
	let checked = sources
		.par_iter()
		.map(|file| {
			let parsed = file
				.components
				.as_ref()
				.and_then(|_| syn::parse_file(&file.source).ok());
			let Some(parsed) = parsed else {
				return Err(file.path.clone());
			};
			let rules = &file.scope.config.rules;
			let lines: Vec<&str> = file.source.lines().collect();
			Ok(
				check_file(&parsed, &file.source, &file.scope.config, &components)
					.into_iter()
					.map(|finding| {
						Diagnostic {
							level: finding.rule.level(rules),
							path: file.path.clone(),
							source_line: lines
								.get(finding.start.line.saturating_sub(1))
								.map_or_else(String::new, |&line| line.to_owned()),
							finding,
						}
					})
					.collect::<Vec<_>>(),
			)
		})
		.collect::<Vec<_>>();
	for result in checked {
		match result {
			Ok(diagnostics) => report.diagnostics.extend(diagnostics),
			Err(path) => report.unparsed.push(path),
		}
	}
	report.diagnostics.sort_by(|left, right| {
		(&left.path, left.finding.start).cmp(&(&right.path, right.finding.start))
	});
	report.unparsed.sort();
	Ok(report)
}

struct SourceFile {
	path: PathBuf,
	source: String,
	/// The components the file declares; `None` when it does not parse.
	components: Option<ComponentNames>,
	scope: Arc<Scope>,
}

/// Every `.rs` file under `paths`, sorted so runs report in the same order.
fn rust_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>, LintError> {
	let mut files = Vec::new();
	for path in paths {
		if path.is_file() {
			files.push(path.clone());
			continue;
		}
		if !path.exists() {
			return Err(LintError::Read {
				path: path.clone(),
				source: std::io::Error::from(std::io::ErrorKind::NotFound),
			});
		}
		// Build output is never source, even outside a git repository whose
		// ignore file would say so.
		let walk = ignore::WalkBuilder::new(path)
			.require_git(false)
			.filter_entry(|entry| entry.file_name() != "target")
			.build();
		for entry in walk {
			let entry = entry.map_err(|source| {
				LintError::Walk {
					path: path.clone(),
					source,
				}
			})?;
			let is_rust = entry.file_type().is_some_and(|kind| kind.is_file())
				&& entry
					.path()
					.extension()
					.is_some_and(|extension| extension == "rs");
			if is_rust {
				files.push(entry.into_path());
			}
		}
	}
	files.sort();
	files.dedup();
	Ok(files)
}

/// Whether a file says it is generated, as `@generated` near its top does by
/// convention: its generator decides how it is written.
fn is_generated(source: &str) -> bool {
	source
		.lines()
		.take(5)
		.any(|line| line.trim_start().starts_with("//") && line.contains("@generated"))
}
