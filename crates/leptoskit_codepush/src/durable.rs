//! Crash-safe filesystem writes, shared by the release store, the client's
//! staging, and the installed bundles.
//!
//! Everything is written under a uniquely named temporary sibling of its
//! destination, synced, and only then renamed into place, so a reader, or
//! the next launch after a power cut, sees the old contents or the new ones
//! and never a mix. Temporary names start with `.` and end in `.tmp`, and
//! are unique per process and call, so concurrent writers never share one.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// A new, uniquely named temporary path next to `path`:
/// `.<name>.<pid>.<nanos>.<counter>.tmp`.
pub(crate) fn temporary_sibling(path: &Path) -> io::Result<PathBuf> {
	static NEXT: AtomicU64 = AtomicU64::new(0);
	let name = path.file_name().ok_or_else(|| {
		io::Error::new(
			io::ErrorKind::InvalidInput,
			format!("{} does not name a file or directory", path.display()),
		)
	})?;
	let nanos = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |elapsed| elapsed.as_nanos());
	let unique = NEXT.fetch_add(1, Ordering::Relaxed);
	Ok(path.with_file_name(format!(
		".{}.{}.{nanos}.{unique}.tmp",
		name.to_string_lossy(),
		std::process::id()
	)))
}

/// Whether a directory entry is a temporary file or directory of an
/// interrupted write, as named by [`temporary_sibling`].
pub(crate) fn is_temporary(name: &str) -> bool {
	name.starts_with('.') && Path::new(name).extension() == Some("tmp".as_ref())
}

/// Writes `bytes` to `path` atomically and durably, replacing any file
/// there.
///
/// The bytes go to a temporary sibling, which is synced and renamed over
/// `path`, and then the directory is synced so the rename survives a power
/// cut. A failed write removes its temporary file, so a full disk does not
/// stay full.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
	let temporary = temporary_sibling(path)?;
	let written = write_new_file(&temporary, bytes).and_then(|()| fs::rename(&temporary, path));
	if let Err(error) = written {
		// The write already failed; that error is the one to report.
		let _ = fs::remove_file(&temporary);
		return Err(error);
	}
	sync_directory(parent_of(path))
}

/// Writes `files`, as `/`-separated paths relative to the directory, into a
/// new temporary sibling of `target` and syncs every file and directory in
/// it, returning the temporary directory for the caller to rename into
/// place. A failure removes everything written.
///
/// Every file must be new: two entries with the same path, or a file where
/// another entry needs a directory, are an error.
pub(crate) fn assemble_directory(target: &Path, files: &[(&str, &[u8])]) -> io::Result<PathBuf> {
	let temporary = temporary_sibling(target)?;
	fs::create_dir(&temporary)?;
	let written = write_tree(&temporary, files);
	if let Err(error) = written {
		// The write already failed; that error is the one to report.
		let _ = fs::remove_dir_all(&temporary);
		return Err(error);
	}
	Ok(temporary)
}

fn write_tree(root: &Path, files: &[(&str, &[u8])]) -> io::Result<()> {
	let mut directories = BTreeSet::from([root.to_path_buf()]);
	for (path, bytes) in files {
		let file = root.join(path);
		if let Some(parent) = file.parent() {
			fs::create_dir_all(parent)?;
			directories.extend(
				parent
					.ancestors()
					.take_while(|ancestor| ancestor.starts_with(root))
					.map(Path::to_path_buf),
			);
		}
		write_new_file(&file, bytes)?;
	}
	directories
		.iter()
		.try_for_each(|directory| sync_directory(directory))
}

/// Creates `path`, which must not exist, and writes and syncs `bytes`.
fn write_new_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
	let mut file = fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(path)?;
	file.write_all(bytes)?;
	file.sync_all()
}

/// The directory holding `path`; `.` for a bare file name.
pub(crate) fn parent_of(path: &Path) -> &Path {
	path.parent()
		.filter(|parent| !parent.as_os_str().is_empty())
		.unwrap_or(Path::new("."))
}

/// Makes a rename or new entry in `directory` durable. Unix needs an fsync of
/// the directory itself; Windows cannot open a directory as a file through
/// `std`, and NTFS journals the metadata change.
pub(crate) fn sync_directory(directory: &Path) -> io::Result<()> {
	if cfg!(unix) {
		fs::File::open(directory)?.sync_all()?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Every entry of `directory`, sorted.
	fn entries(directory: &Path) -> Vec<String> {
		let mut names: Vec<_> = fs::read_dir(directory)
			.expect("read directory")
			.map(|entry| {
				entry
					.expect("entry")
					.file_name()
					.to_string_lossy()
					.into_owned()
			})
			.collect();
		names.sort();
		names
	}

	#[test]
	fn atomic_writes_replace_the_file_and_leave_nothing_else() {
		let directory = tempfile::tempdir().expect("directory");
		let path = directory.path().join("state.json");
		write_atomically(&path, b"first").expect("write");
		write_atomically(&path, b"second").expect("replace");
		assert_eq!(fs::read(&path).expect("read"), b"second");
		assert_eq!(entries(directory.path()), ["state.json"]);
	}

	#[test]
	fn a_failed_atomic_write_removes_its_temporary_file() {
		let directory = tempfile::tempdir().expect("directory");
		// A non-empty directory cannot be replaced by a file.
		let path = directory.path().join("state.json");
		fs::create_dir(&path).expect("blocking directory");
		fs::write(path.join("keep"), b"").expect("non-empty");
		assert!(write_atomically(&path, b"bytes").is_err());
		assert_eq!(entries(directory.path()), ["state.json"]);
	}

	#[test]
	fn assembled_directories_hold_every_file_and_failures_leave_nothing() {
		let directory = tempfile::tempdir().expect("directory");
		let target = directory.path().join("release");
		let assembled = assemble_directory(
			&target,
			&[
				("release.lkb", b"container"),
				("deltas/a.jsondelta", b"delta"),
			],
		)
		.expect("assemble");
		assert!(is_temporary(
			&assembled.file_name().expect("name").to_string_lossy()
		));
		assert_eq!(
			fs::read(assembled.join("deltas/a.jsondelta")).expect("read"),
			b"delta"
		);
		fs::remove_dir_all(assembled).expect("clean up");

		for conflicting in [
			[("app.wasm", b"one".as_slice()), ("app.wasm", b"two")],
			[
				("assets", b"a file"),
				("assets/logo.png", b"needs a directory"),
			],
		] {
			assert!(assemble_directory(&target, &conflicting).is_err());
			assert!(
				entries(directory.path()).is_empty(),
				"{:?}",
				entries(directory.path())
			);
		}
	}
}
