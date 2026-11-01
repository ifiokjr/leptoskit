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

/// Writes a 256 KiB `pkg/app_bg.wasm` under `dist` that compresses like a
/// compiled module rather than like one repeated byte, the same on every
/// run. Code push stores a delta only when it saves most of the download,
/// so a release that changes a page around an unchanged module ships as one,
/// as a real web release does.
///
/// # Panics
///
/// Panics when the module cannot be written.
pub fn write_web_module(dist: &Path) {
	const CODE: &[u8] = b"\0asm local.get call";
	let mut state = 0x9e37_79b9_u64;
	let mut module = b"\0asm\x01\0\0\0".to_vec();
	module.extend((0..256 * 1024).map(|_| {
		state ^= state << 13;
		state ^= state >> 7;
		state ^= state << 17;
		CODE[(state % CODE.len() as u64) as usize]
	}));
	fs::create_dir_all(dist.join("pkg")).expect("dist/pkg");
	fs::write(dist.join("pkg/app_bg.wasm"), module).expect("web module");
}
