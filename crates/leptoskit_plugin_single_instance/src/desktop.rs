use std::fs::File;
use std::fs::{self};
use std::io;
use std::path::Path;

use fs4::FileExt;

/// An exclusive lock released by dropping the guard or exiting the process.
///
/// The lock file is deliberately retained after release. Deleting it would let
/// processes lock separate inodes and both believe they own the same identity.
#[derive(Debug)]
pub struct InstanceGuard {
	_file: File,
}

impl InstanceGuard {
	/// Try to acquire an app identity inside an existing private directory.
	///
	/// The directory must be absolute, local, and owned by the current user.
	/// Unix permissions must exclude all group/other access. On Windows the
	/// caller must provide a directory protected by a per-user ACL (for
	/// example an app directory below `LocalAppData`). This function never
	/// changes permissions. Every instance must use the same directory and
	/// identity. Do not delete or replace lock files while any application
	/// instance may be running.
	///
	/// Returns `Ok(None)` only when another instance currently holds the lock.
	pub fn acquire(directory: &Path, identity: &str) -> io::Result<Option<Self>> {
		if identity.is_empty()
			|| identity.len() > 128
			|| !identity.as_bytes()[0].is_ascii_alphanumeric()
			|| !identity
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
			|| !directory.is_absolute()
		{
			return Err(io::Error::new(
				io::ErrorKind::InvalidInput,
				"invalid instance directory or identity",
			));
		}
		let metadata = fs::symlink_metadata(directory)?;
		if !metadata.file_type().is_dir() {
			return Err(io::Error::new(
				io::ErrorKind::InvalidInput,
				"instance directory must not be a symlink",
			));
		}
		#[cfg(unix)]
		{
			use std::os::unix::fs::MetadataExt;
			if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0
			{
				return Err(io::Error::new(
					io::ErrorKind::PermissionDenied,
					"instance directory must be private to the current user",
				));
			}
		}
		let file = open_lock(&directory.join(format!("{identity}.lock")))?;
		let metadata = file.metadata()?;
		if !metadata.is_file() {
			return Err(io::Error::new(
				io::ErrorKind::InvalidInput,
				"instance lock must be a regular file",
			));
		}
		#[cfg(unix)]
		{
			use std::os::unix::fs::MetadataExt;
			if metadata.uid() != rustix::process::geteuid().as_raw()
				|| metadata.nlink() != 1
				|| metadata.mode() & 0o077 != 0
			{
				return Err(io::Error::new(
					io::ErrorKind::PermissionDenied,
					"instance lock must be a private file with one link",
				));
			}
		}
		match FileExt::try_lock(&file) {
			Ok(()) => Ok(Some(Self { _file: file })),
			Err(fs4::TryLockError::WouldBlock) => Ok(None),
			Err(fs4::TryLockError::Error(error)) => Err(error),
		}
	}
}

#[cfg(unix)]
fn open_lock(path: &Path) -> io::Result<File> {
	use rustix::fs::Mode;
	use rustix::fs::OFlags;
	use rustix::fs::open;
	Ok(open(
		path,
		OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
		Mode::RUSR | Mode::WUSR,
	)?
	.into())
}

#[cfg(windows)]
fn open_lock(path: &Path) -> io::Result<File> {
	use std::os::windows::fs::OpenOptionsExt;
	// Open the reparse point itself; the regular-file check rejects it.
	fs::OpenOptions::new()
		.read(true)
		.write(true)
		.create(true)
		.truncate(false)
		.custom_flags(0x0020_0000)
		.open(path)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn private_directory() -> tempfile::TempDir {
		let directory = tempfile::tempdir().unwrap();
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
		}
		directory
	}
	#[test]
	fn lock_lives_until_guard_is_dropped_and_file_is_retained() {
		let dir = private_directory();
		let first = InstanceGuard::acquire(dir.path(), "dev.test.app")
			.unwrap()
			.unwrap();
		assert!(
			InstanceGuard::acquire(dir.path(), "dev.test.app")
				.unwrap()
				.is_none()
		);
		assert!(
			InstanceGuard::acquire(dir.path(), "dev.other.app")
				.unwrap()
				.is_some()
		);
		drop(first);
		assert!(dir.path().join("dev.test.app.lock").exists());
		assert!(
			InstanceGuard::acquire(dir.path(), "dev.test.app")
				.unwrap()
				.is_some()
		);
	}
	#[test]
	fn invalid_identity_cannot_escape_directory() {
		let dir = private_directory();
		for identity in ["", "../escape", "foo/bar", ".hidden"] {
			assert_eq!(
				InstanceGuard::acquire(dir.path(), identity)
					.unwrap_err()
					.kind(),
				io::ErrorKind::InvalidInput
			);
		}
	}
	#[cfg(unix)]
	#[test]
	fn symlinks_and_shared_directories_are_rejected() {
		use std::os::unix::fs::PermissionsExt;
		use std::os::unix::fs::symlink;
		let dir = private_directory();
		let target = dir.path().join("target");
		fs::write(&target, "untouched").unwrap();
		symlink(&target, dir.path().join("test.lock")).unwrap();
		assert!(InstanceGuard::acquire(dir.path(), "test").is_err());
		assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
		fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
		assert_eq!(
			InstanceGuard::acquire(dir.path(), "another")
				.unwrap_err()
				.kind(),
			io::ErrorKind::PermissionDenied
		);
	}
}
