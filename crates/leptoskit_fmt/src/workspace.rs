//! Formatting many files at once: in parallel, and skipping the files a
//! previous run already left formatted.
//!
//! rustfmt dominates the cost of formatting a file, so a run over an
//! unchanged workspace should not start it at all. Every file a run leaves
//! formatted is recorded in a cache, keyed by a hash of its contents and of
//! everything that decides its formatting: the settings, the rustfmt
//! configuration's contents, the rustfmt binary, and this crate's version.
//! A later run skips a file whose key is unchanged.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use rayon::prelude::*;

use crate::Diagnostic;
use crate::config::Settings;
use crate::engine::Stats;
use crate::engine::format_source;
use crate::error::FormatError;
use crate::rustfmt::Rustfmt;

/// How a run treats the files it formats.
#[derive(Debug, Clone, Default)]
pub struct Options {
	/// Report what would change instead of writing it.
	pub check: bool,
	/// The rustfmt toolchain for every file, overriding the configuration's
	/// `rustfmt_toolchain`.
	pub toolchain: Option<String>,
	/// Where the cache of formatted files lives; `None` formats every file.
	pub cache: Option<PathBuf>,
	/// Run each file's markup expressions through a second rustfmt process
	/// alongside the file's: see [`Rustfmt::concurrent`].
	pub concurrent: bool,
}

/// What happened to one file.
#[derive(Debug)]
pub enum Status {
	/// Formatted as it was.
	Unchanged,
	/// Skipped: the cache says it is formatted.
	Cached,
	/// Rewritten, or in check mode, would be.
	Changed {
		/// The text before formatting.
		original: String,
		/// The formatted text.
		formatted: String,
	},
	/// Could not be formatted.
	Failed(FormatError),
}

/// One file's result.
#[derive(Debug)]
pub struct FileReport {
	/// The file.
	pub path: PathBuf,
	/// What happened to it.
	pub status: Status,
	/// Markup left as written.
	pub diagnostics: Vec<Diagnostic>,
	/// How long it took.
	pub elapsed: Duration,
	/// How much of that rustfmt took.
	pub stats: Stats,
}

/// The results of a run, in path order.
#[derive(Debug, Default)]
pub struct Report {
	/// Every file the run looked at.
	pub files: Vec<FileReport>,
}

impl Report {
	/// The files that changed, or would.
	pub fn changed(&self) -> impl Iterator<Item = &FileReport> {
		self.files
			.iter()
			.filter(|file| matches!(file.status, Status::Changed { .. }))
	}

	/// The files that could not be formatted.
	pub fn failed(&self) -> impl Iterator<Item = &FileReport> {
		self.files
			.iter()
			.filter(|file| matches!(file.status, Status::Failed(_)))
	}
}

/// The Rust files under `paths`: files as given, and directories walked
/// with `.gitignore` respected and files matching an `exclude` glob skipped.
///
/// # Errors
///
/// Returns [`FormatError::Io`] when a path does not exist, and
/// [`FormatError::Config`] for an invalid glob.
pub fn collect_files(paths: &[PathBuf], exclude: &[String]) -> Result<Vec<PathBuf>, FormatError> {
	let mut files = Vec::new();
	for path in paths {
		let metadata = std::fs::metadata(path).map_err(|source| {
			FormatError::Io {
				path: path.clone(),
				source,
			}
		})?;
		if metadata.is_file() {
			files.push(path.clone());
			continue;
		}
		let mut overrides = ignore::overrides::OverrideBuilder::new(path);
		for glob in exclude {
			overrides.add(&format!("!{glob}")).map_err(|error| {
				FormatError::Config {
					path: PathBuf::from(glob),
					message: error.to_string(),
				}
			})?;
		}
		let overrides = overrides.build().map_err(|error| {
			FormatError::Config {
				path: path.clone(),
				message: error.to_string(),
			}
		})?;
		let walker = ignore::WalkBuilder::new(path).overrides(overrides).build();
		for entry in walker.filter_map(Result::ok) {
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

/// Formats `files` in parallel.
#[must_use]
pub fn format_files(files: &[PathBuf], options: &Options) -> Report {
	let run = Run::new(options);
	let mut reports: Vec<FileReport> = files.par_iter().map(|path| run.file(path)).collect();
	reports.sort_by(|left, right| left.path.cmp(&right.path));
	run.save_cache();
	Report { files: reports }
}

/// The cache directory for a run from `dir`: `leptoskit/format` in the
/// cargo target directory of the workspace around it.
#[must_use]
pub fn default_cache_dir(dir: &Path) -> Option<PathBuf> {
	if let Some(target) = std::env::var_os("CARGO_TARGET_DIR") {
		return Some(PathBuf::from(target).join("leptoskit").join("format"));
	}
	let dir = std::path::absolute(dir).ok()?;
	dir.ancestors()
		.find(|ancestor| ancestor.join("Cargo.lock").is_file())
		.map(|root| root.join("target").join("leptoskit").join("format"))
}

/// Shared state of one run.
struct Run<'a> {
	options: &'a Options,
	/// Settings by directory: most files share a handful.
	settings: Mutex<HashMap<PathBuf, Result<Arc<Settings>, String>>>,
	/// rustfmt by toolchain.
	rustfmt: Mutex<HashMap<String, Result<Rustfmt, String>>>,
	/// The cache read at the start: path to key.
	known: HashMap<PathBuf, u64>,
	/// Keys of the files this run left formatted.
	formatted: Mutex<HashMap<PathBuf, u64>>,
}

impl<'a> Run<'a> {
	fn new(options: &'a Options) -> Self {
		let known = options
			.cache
			.as_deref()
			.map(|dir| read_cache(&dir.join(CACHE_FILE)))
			.unwrap_or_default();
		Self {
			options,
			settings: Mutex::default(),
			rustfmt: Mutex::default(),
			known,
			formatted: Mutex::default(),
		}
	}

	fn file(&self, path: &Path) -> FileReport {
		let started = Instant::now();
		let mut diagnostics = Vec::new();
		let mut timing = Stats::default();
		let status = self
			.format(path, &mut diagnostics, &mut timing)
			.unwrap_or_else(Status::Failed);
		FileReport {
			path: path.to_owned(),
			status,
			diagnostics,
			elapsed: started.elapsed(),
			stats: timing,
		}
	}

	fn format(
		&self,
		path: &Path,
		diagnostics: &mut Vec<Diagnostic>,
		stats: &mut Stats,
	) -> Result<Status, FormatError> {
		let bytes = std::fs::read(path).map_err(|source| {
			FormatError::Io {
				path: path.to_owned(),
				source,
			}
		})?;
		let source = String::from_utf8(bytes).map_err(|_| {
			FormatError::NotUtf8 {
				path: path.to_owned(),
			}
		})?;
		let settings = self.settings_for(path)?;
		let rustfmt = self.rustfmt_for(&settings)?;
		let key = cache_key(&source, &settings, &rustfmt);
		if self.known.get(path) == Some(&key) {
			self.remember(path, key);
			return Ok(Status::Cached);
		}
		let output = format_source(&source, &settings, &rustfmt)?;
		*diagnostics = output.diagnostics;
		*stats = output.stats;
		if output.text == source {
			self.remember(path, key);
			return Ok(Status::Unchanged);
		}
		if !self.options.check {
			std::fs::write(path, &output.text).map_err(|source| {
				FormatError::Io {
					path: path.to_owned(),
					source,
				}
			})?;
			self.remember(path, cache_key(&output.text, &settings, &rustfmt));
		}
		Ok(Status::Changed {
			original: source,
			formatted: output.text,
		})
	}

	fn remember(&self, path: &Path, key: u64) {
		if let Ok(mut formatted) = self.formatted.lock() {
			formatted.insert(path.to_owned(), key);
		}
	}

	fn settings_for(&self, path: &Path) -> Result<Arc<Settings>, FormatError> {
		let dir = path.parent().unwrap_or(path).to_owned();
		let mut cache = self.settings.lock().map_err(|_| poisoned(path))?;
		let entry = cache.entry(dir).or_insert_with(|| {
			Settings::for_path(path)
				.map(Arc::new)
				.map_err(|error| error.to_string())
		});
		entry.clone().map_err(|message| {
			FormatError::Config {
				path: path.to_owned(),
				message,
			}
		})
	}

	fn rustfmt_for(&self, settings: &Settings) -> Result<Rustfmt, FormatError> {
		let toolchain = self
			.options
			.toolchain
			.clone()
			.unwrap_or_else(|| settings.markup.rustfmt_toolchain.clone());
		let mut cache = self
			.rustfmt
			.lock()
			.map_err(|_| FormatError::Rustfmt("a formatting thread panicked".to_owned()))?;
		let entry = cache.entry(toolchain.clone()).or_insert_with(|| {
			Rustfmt::locate(&toolchain)
				.map(|rustfmt| rustfmt.concurrent(self.options.concurrent))
				.map_err(|error| error.to_string())
		});
		entry
			.clone()
			.map_err(|message| FormatError::RustfmtMissing { toolchain, message })
	}

	fn save_cache(&self) {
		let Some(dir) = &self.options.cache else {
			return;
		};
		let Ok(formatted) = self.formatted.lock() else {
			return;
		};
		let mut entries = self.known.clone();
		entries.extend(formatted.iter().map(|(path, key)| (path.clone(), *key)));
		let mut lines: Vec<String> = entries
			.iter()
			.filter(|(path, _)| path.exists())
			.map(|(path, key)| format!("{key:016x}\t{}", path.display()))
			.collect();
		lines.sort();
		// The cache only saves time: a run that cannot write it still
		// formatted every file correctly.
		if std::fs::create_dir_all(dir).is_ok() {
			let _ = std::fs::write(dir.join(CACHE_FILE), lines.join("\n"));
		}
	}
}

const CACHE_FILE: &str = "formatted-files";

fn poisoned(path: &Path) -> FormatError {
	FormatError::Config {
		path: path.to_owned(),
		message: "a formatting thread panicked".to_owned(),
	}
}

fn read_cache(path: &Path) -> HashMap<PathBuf, u64> {
	let Ok(raw) = std::fs::read_to_string(path) else {
		return HashMap::new();
	};
	raw.lines()
		.filter_map(|line| {
			let (key, path) = line.split_once('\t')?;
			Some((PathBuf::from(path), u64::from_str_radix(key, 16).ok()?))
		})
		.collect()
}

/// Everything that decides how `source` formats, hashed.
fn cache_key(source: &str, settings: &Settings, rustfmt: &Rustfmt) -> u64 {
	let mut hasher = DefaultHasher::new();
	env!("CARGO_PKG_VERSION").hash(&mut hasher);
	source.hash(&mut hasher);
	settings.hash(&mut hasher);
	if let Some(config) = &settings.rustfmt_config {
		std::fs::read(config).ok().hash(&mut hasher);
	}
	rustfmt.identity().hash(&mut hasher);
	if let Ok(metadata) = std::fs::metadata(rustfmt.program()) {
		metadata.len().hash(&mut hasher);
		metadata.modified().ok().hash(&mut hasher);
	}
	hasher.finish()
}
