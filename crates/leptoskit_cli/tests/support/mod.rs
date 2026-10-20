//! Shared integration-test support: scratch directories and fixture copies.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

/// A temporary directory that removes itself on drop, so tests leave
/// nothing behind.
pub struct Scratch {
	/// The scratch directory, canonicalized so paths compare equal to what
	/// the CLI resolves.
	pub path: PathBuf,
}

impl Scratch {
	/// A fresh, canonicalized directory under the system temp dir.
	pub fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_cli_test_{tag}_{}_{}",
			std::process::id(),
			unique
		));
		fs::create_dir_all(&path).expect("scratch");
		let path = fs::canonicalize(&path).expect("canonical scratch");
		Self { path }
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = fs::remove_dir_all(&self.path);
	}
}

/// Copies the `tests/fixtures/<name>` fixture into a fresh scratch
/// directory and returns it, so tests run the CLI against a real on-disk
/// project without mutating the fixture.
///
/// # Panics
///
/// Panics when the named fixture does not exist under `tests/fixtures`.
pub fn fixture_project(name: &str) -> Scratch {
	let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("tests/fixtures")
		.join(name);
	assert!(source.is_dir(), "missing fixture {}", source.display());
	let scratch = Scratch::new(name);
	copy_dir(&source, &scratch.path);
	scratch
}

fn copy_dir(from: &Path, to: &Path) {
	fs::create_dir_all(to).expect("create target");
	for entry in fs::read_dir(from).expect("read fixture") {
		let entry = entry.expect("fixture entry");
		let target = to.join(entry.file_name());
		if entry.file_type().expect("fixture entry type").is_dir() {
			copy_dir(&entry.path(), &target);
		} else {
			fs::copy(entry.path(), target).expect("copy fixture file");
		}
	}
}
