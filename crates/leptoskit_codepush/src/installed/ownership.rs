//! Hold one installed-state snapshot per directory, including across processes.

use std::fs;
use std::path::Path;

use crate::CodePushError;

/// Retains an advisory lock until the installed-bundle owner is dropped.
#[cfg(any(unix, windows))]
pub(super) fn acquire(directory: &Path) -> Result<fs::File, CodePushError> {
	fs::create_dir_all(directory)?;
	let file = fs::OpenOptions::new()
		.create(true)
		.truncate(false)
		.read(true)
		.write(true)
		.open(directory.join("installed.lock"))?;

	match fs4::FileExt::try_lock(&file) {
		Ok(()) => {}
		Err(fs4::TryLockError::WouldBlock) => {
			return Err(CodePushError::Activation(
				"installed update store is already owned; close the previous owner before retrying"
					.into(),
			));
		}
		Err(fs4::TryLockError::Error(error)) => return Err(error.into()),
	}

	// Never unlink the lock: overlapping opens must lock the same inode even
	// after a previous owner exits. Closing the file releases ownership.
	Ok(file)
}

/// Unsupported targets must never open an unprotected installed-state snapshot.
#[cfg(not(any(unix, windows)))]
pub(super) fn acquire(_directory: &Path) -> Result<fs::File, CodePushError> {
	Err(std::io::Error::new(
		std::io::ErrorKind::Unsupported,
		"installed update stores require native file locking",
	)
	.into())
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
	use super::*;

	#[test]
	fn another_process_cannot_acquire_an_owned_directory() {
		const STORE: &str = "LEPTOSKIT_TEST_INSTALLED_LOCK_STORE";

		if let Some(path) = std::env::var_os(STORE) {
			assert!(matches!(
				acquire(Path::new(&path)),
				Err(CodePushError::Activation(message)) if message.contains("already owned")
			));
			return;
		}

		let directory = tempfile::tempdir().expect("installed");
		let owner = acquire(directory.path()).expect("owner");
		let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
			.args([
				"--exact",
				"installed::ownership::tests::another_process_cannot_acquire_an_owned_directory",
			])
			.env(STORE, directory.path())
			.output()
			.expect("child process");
		assert!(
			output.status.success(),
			"{}",
			String::from_utf8_lossy(&output.stdout)
		);
		drop(owner);
		let _replacement = acquire(directory.path()).expect("replacement");
	}
}
