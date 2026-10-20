//! Keep generated registrants and platform inputs stable through native builds.

use std::fs::File;
use std::fs::OpenOptions;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use fs4::FileExt;

pub(super) fn acquire(root: &Path) -> Result<File> {
	let directory = root
		.canonicalize()
		.context("canonicalize native application directory")?
		.join(".leptoskit");
	std::fs::create_dir_all(&directory).context("create native output directory")?;
	let file = OpenOptions::new()
		.create(true)
		.truncate(false)
		.read(true)
		.write(true)
		.open(directory.join("native.lock"))
		.context("open native application build lock")?;
	match FileExt::try_lock(&file) {
		Ok(()) => {}
		Err(fs4::TryLockError::WouldBlock) => {
			anyhow::bail!(
				"another native prepare or build is running for this application; wait for it to \
				 finish"
			)
		}
		Err(fs4::TryLockError::Error(error)) => {
			return Err(error).context("lock native application build");
		}
	}
	// Retain the file through platform bundling. Never unlink it: a waiter
	// could otherwise hold a different inode and bypass mutual exclusion.
	Ok(file)
}

#[cfg(test)]
mod tests {
	use std::sync::Mutex;
	use std::sync::MutexGuard;
	use std::sync::PoisonError;

	use super::*;

	/// `flock` locks belong to the open file description, and a child process
	/// forked while another test holds or drops a guard briefly shares that
	/// description. Serialize these tests so a sibling's subprocess cannot keep
	/// a released lock alive.
	static SERIAL: Mutex<()> = Mutex::new(());

	fn serial() -> MutexGuard<'static, ()> {
		SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
	}

	#[test]
	fn competing_builds_fail_and_release_allows_retry() {
		let _serial = serial();
		let app = tempfile::tempdir().unwrap();
		let other_app = tempfile::tempdir().unwrap();
		let guard = acquire(app.path()).unwrap();
		let error = acquire(app.path()).unwrap_err();
		assert!(
			error
				.to_string()
				.contains("another native prepare or build")
		);
		let _independent = acquire(other_app.path()).unwrap();
		drop(guard);
		let _retry = acquire(app.path()).unwrap();
	}

	#[test]
	fn separate_process_cannot_acquire_an_active_build_lock() {
		let _serial = serial();
		const APP_PATH: &str = "LEPTOSKIT_TEST_NATIVE_LOCK_APP";
		if let Some(path) = std::env::var_os(APP_PATH) {
			assert!(
				acquire(Path::new(&path))
					.unwrap_err()
					.to_string()
					.contains("another native prepare or build")
			);
			return;
		}
		let app = tempfile::tempdir().unwrap();
		let _guard = acquire(app.path()).unwrap();
		let output = std::process::Command::new(std::env::current_exe().unwrap())
			.args([
				"--exact",
				"native::lock::tests::separate_process_cannot_acquire_an_active_build_lock",
			])
			.env(APP_PATH, app.path())
			.output()
			.unwrap();
		assert!(
			output.status.success(),
			"{}",
			String::from_utf8_lossy(&output.stdout)
		);
	}

	#[cfg(unix)]
	#[test]
	fn symlink_aliases_contend_for_the_same_lock() {
		let _serial = serial();
		let directory = tempfile::tempdir().unwrap();
		let app = directory.path().join("app");
		let alias = directory.path().join("alias");
		std::fs::create_dir(&app).unwrap();
		std::os::unix::fs::symlink(&app, &alias).unwrap();
		let guard = acquire(&app).unwrap();
		assert!(acquire(&alias).is_err());
		drop(guard);
		let _retry = acquire(&alias).unwrap();
	}
}
