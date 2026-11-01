//! Reads `src/routes/` from disk into a tree of named files and directories.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use crate::Diagnostic;
use crate::exports;
use crate::exports::Exports;
use crate::name;
use crate::name::Name;

/// A `.rs` file in the routes tree.
#[derive(Debug)]
pub(crate) struct File {
	/// The name on disk, such as `[note].rs`.
	pub(crate) disk_name: String,
	pub(crate) name: Name,
	pub(crate) module: String,
	pub(crate) path: PathBuf,
	/// `None` for private files, and for files that could not be read or
	/// parsed (those already have a diagnostic).
	pub(crate) exports: Option<Exports>,
}

/// A directory in the routes tree.
#[derive(Debug)]
pub(crate) struct Directory {
	/// The name on disk, such as `(tabs)`; empty for the root.
	pub(crate) disk_name: String,
	/// `None` for the root.
	pub(crate) name: Option<Name>,
	pub(crate) module: String,
	pub(crate) files: Vec<File>,
	pub(crate) directories: Vec<Directory>,
}

impl Directory {
	/// Whether the tree under this directory holds any file.
	fn is_empty(&self) -> bool {
		self.files.is_empty() && self.directories.iter().all(Self::is_empty)
	}
}

/// Reads the routes directory, collecting every problem rather than stopping
/// at the first.
pub(crate) fn read(root: &Path, diagnostics: &mut Vec<Diagnostic>) -> Directory {
	read_directory(root, String::new(), None, String::new(), diagnostics)
}

fn read_directory(
	path: &Path,
	disk_name: String,
	name: Option<Name>,
	module: String,
	diagnostics: &mut Vec<Diagnostic>,
) -> Directory {
	let mut directory = Directory {
		disk_name,
		name,
		module,
		files: Vec::new(),
		directories: Vec::new(),
	};

	let mut entries = match std::fs::read_dir(path) {
		Ok(entries) => {
			entries
				.filter_map(Result::ok)
				.map(|entry| entry.path())
				.collect::<Vec<_>>()
		}
		Err(error) => {
			diagnostics.push(Diagnostic::new(
				path,
				None,
				format!("could not read: {error}"),
			));
			return directory;
		}
	};

	entries.sort();

	// Module names this directory has handed out, to catch two names that
	// become the same Rust module.
	let mut modules: BTreeMap<String, String> = BTreeMap::new();

	for entry in entries {
		let Some(disk_name) = entry
			.file_name()
			.and_then(|name| name.to_str())
			.map(str::to_owned)
		else {
			diagnostics.push(Diagnostic::new(
				&entry,
				None,
				"route names must be UTF-8".to_owned(),
			));
			continue;
		};

		if disk_name.starts_with('.') {
			continue;
		}

		let is_dir = entry.is_dir();
		let stem = if is_dir {
			disk_name.as_str()
		} else if let Some(stem) = disk_name.strip_suffix(".rs") {
			stem
		} else {
			// Only Rust files are routes; anything else is left alone.
			continue;
		};

		let parsed = match name::parse(stem, is_dir) {
			Ok(parsed) => parsed,
			Err(message) => {
				diagnostics.push(Diagnostic::new(&entry, None, message));
				continue;
			}
		};

		let module = name::module_name(&parsed);

		if let Some(previous) = modules.insert(module.clone(), disk_name.clone()) {
			diagnostics.push(Diagnostic::new(
				&entry,
				None,
				format!(
					"`{disk_name}` and `{previous}` both become the Rust module `{module}`; \
					 rename one (a route file with children belongs in its directory as \
					 `index.rs`)"
				),
			));
			continue;
		}

		if is_dir {
			let child = read_directory(&entry, disk_name, Some(parsed), module, diagnostics);

			if !child.is_empty() {
				directory.directories.push(child);
			}
		} else {
			let exports = if matches!(parsed, Name::Private(_)) {
				None
			} else {
				read_exports(&entry, diagnostics)
			};

			directory.files.push(File {
				disk_name,
				name: parsed,
				module,
				path: entry,
				exports,
			});
		}
	}

	directory
}

fn read_exports(path: &Path, diagnostics: &mut Vec<Diagnostic>) -> Option<Exports> {
	let source = match std::fs::read_to_string(path) {
		Ok(source) => source,
		Err(error) => {
			diagnostics.push(Diagnostic::new(
				path,
				None,
				format!("could not read: {error}"),
			));
			return None;
		}
	};

	match exports::read(&source) {
		Ok(exports) => Some(exports),
		Err(problems) => {
			diagnostics.extend(
				problems
					.into_iter()
					.map(|(line, message)| Diagnostic::new(path, line, message)),
			);
			None
		}
	}
}
