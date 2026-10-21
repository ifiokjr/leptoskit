use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;

use cap_std::fs::Dir;
use leptoskit_bridge::Plugin;

use super::*;

/// Directory capability retained for the lifetime of the plugin.
///
/// This is a synchronous native API. Schedule bulk filesystem work away from UI
/// event handling; each bridge operation is bounded but disk latency is not.
pub struct Filesystem {
	root: Dir,
}

/// Opens the dedicated files subdirectory of the host's application data root.
/// Cache storage is intentionally not mixed with persistent application files.
pub fn from_app_dirs(data_dir: &Path, _cache_dir: &Path) -> Result<Filesystem, Error> {
	let parent = Dir::open_ambient_dir(data_dir, cap_std::ambient_authority()).map_err(failure)?;
	let mut builder = cap_std::fs::DirBuilder::new();
	#[cfg(unix)]
	{
		use cap_std::fs::DirBuilderExt as _;
		builder.mode(0o700);
	}
	builder.recursive(true);
	parent.create_dir_with("files", &builder).map_err(failure)?;
	let root = parent.open_dir("files").map_err(failure)?;
	#[cfg(unix)]
	{
		use cap_std::fs::PermissionsExt as _;
		root.set_permissions(".", cap_std::fs::Permissions::from_mode(0o700))
			.map_err(failure)?;
	}
	Ok(Filesystem { root })
}

// Consumes the platform error at the intentionally small bridge error boundary.
#[allow(clippy::needless_pass_by_value)]
fn failure(error: std::io::Error) -> Error {
	match error.kind() {
		std::io::ErrorKind::PermissionDenied => Error::PermissionDenied,
		std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData => {
			Error::InvalidArgument
		}
		_ => Error::Native,
	}
}

impl Filesystem {
	/// Consumes a directory capability selected by trusted host code.
	/// The caller owns the policy for choosing and securing this directory.
	pub fn from_dir(root: Dir) -> Self {
		Self { root }
	}

	/// Grants access to an existing directory selected by trusted host code.
	/// The guest can never replace or expand this root capability.
	pub fn open(root: &Path) -> Result<Self, Error> {
		Ok(Self {
			root: Dir::open_ambient_dir(root, cap_std::ambient_authority()).map_err(failure)?,
		})
	}

	fn metadata(&self, path: &str) -> Result<Metadata, Error> {
		validate_path(path, true)?;
		let metadata = if path.is_empty() {
			self.root.dir_metadata()
		} else {
			self.root.symlink_metadata(path)
		}
		.map_err(failure)?;
		if metadata.is_file() {
			Ok(Metadata {
				kind: Kind::File,
				len: metadata.len(),
			})
		} else if metadata.is_dir() {
			Ok(Metadata {
				kind: Kind::Directory,
				len: 0,
			})
		} else {
			Err(Error::PermissionDenied)
		}
	}

	/// Reads a regular file. Reads are capped even if the file grows
	/// concurrently.
	pub fn read(&self, path: &str) -> Result<Bytes, Error> {
		validate_path(path, false)?;
		let metadata = self.metadata(path)?;
		if metadata.kind != Kind::File {
			return Err(Error::InvalidArgument);
		}
		if metadata.len > MAX_FILE_BYTES as u64 {
			return Err(Error::TooLarge);
		}
		let mut options = cap_std::fs::OpenOptions::new();
		options.read(true);
		#[cfg(unix)]
		{
			use cap_std::fs::OpenOptionsExt as _;
			// Opening a concurrently substituted FIFO must not block the host.
			options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
		}
		let file = self.root.open_with(path, &options).map_err(failure)?;
		if !file.metadata().map_err(failure)?.is_file() {
			return Err(Error::PermissionDenied);
		}
		let mut bytes = Vec::new();
		file.take((MAX_FILE_BYTES + 1) as u64)
			.read_to_end(&mut bytes)
			.map_err(failure)?;
		if bytes.len() > MAX_FILE_BYTES {
			return Err(Error::TooLarge);
		}
		Ok(bytes.into())
	}

	/// Replaces contents by renaming a fully written temporary file in the same
	/// directory. Readers see old or new contents. Successful return means file
	/// data was synced; power-loss durability of the directory entry is not
	/// promised.
	pub fn write(&self, path: &str, bytes: &[u8]) -> Result<(), Error> {
		validate_path(path, false)?;
		if bytes.len() > MAX_FILE_BYTES {
			return Err(Error::TooLarge);
		}
		let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
		let parent = if parent.is_empty() {
			self.root.try_clone()
		} else {
			self.root.open_dir(parent)
		}
		.map_err(failure)?;
		match parent.symlink_metadata(name) {
			Ok(meta) if !meta.is_file() => return Err(Error::PermissionDenied),
			Ok(_) => {}
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
			Err(error) => return Err(failure(error)),
		}
		let mut nonce = [0_u8; 16];
		getrandom::fill(&mut nonce).map_err(|_| Error::Native)?;
		let temporary = format!("{TEMP_PREFIX}{:032x}", u128::from_le_bytes(nonce));
		let mut options = cap_std::fs::OpenOptions::new();
		options.write(true).create_new(true);
		#[cfg(unix)]
		{
			use cap_std::fs::OpenOptionsExt as _;
			options.mode(0o600);
		}
		let mut file = parent.open_with(&temporary, &options).map_err(failure)?;
		let result = file.write_all(bytes).and_then(|()| file.sync_all());
		drop(file);
		let result = result.and_then(|()| parent.rename(&temporary, &parent, name));
		if let Err(error) = result {
			// Report the write failure itself. A temporary that cannot be
			// removed stays hidden from list() and is never
			// addressable by the guest.
			let _ = parent.remove_file(&temporary);
			return Err(failure(error));
		}
		Ok(())
	}

	/// Returns sorted immediate names. Unsupported special files and symlinks
	/// are not listed; excessive results fail instead of truncating silently.
	pub fn list(&self, path: &str) -> Result<Vec<String>, Error> {
		validate_path(path, true)?;
		let directory = if path.is_empty() {
			self.root.try_clone()
		} else {
			self.root.open_dir(path)
		}
		.map_err(failure)?;
		let mut names = Vec::new();
		let mut size = 4;
		for entry in directory.entries().map_err(failure)? {
			let entry = entry.map_err(failure)?;
			let kind = entry.file_type().map_err(failure)?;
			if !kind.is_file() && !kind.is_dir() {
				continue;
			}
			// Names the guest could not address (non-UTF-8, reserved, or
			// in-flight temporaries) are skipped like special
			// files, not reported as errors.
			let Ok(name) = entry.file_name().into_string() else {
				continue;
			};
			if validate_path(&name, false).is_err() {
				continue;
			}
			size += 4 + name.len();
			if names.len() >= MAX_ENTRIES || size > MAX_LIST_BYTES {
				return Err(Error::TooLarge);
			}
			names.push(name);
		}
		names.sort_unstable();
		Ok(names)
	}

	/// Creates parent directories as needed; existing directories succeed.
	pub fn mkdir(&self, path: &str) -> Result<(), Error> {
		validate_path(path, false)?;
		self.root.create_dir_all(path).map_err(failure)
	}

	/// Removes one file or empty directory, never a subtree or the root.
	pub fn remove(&self, path: &str) -> Result<(), Error> {
		validate_path(path, false)?;
		match self.metadata(path)?.kind {
			Kind::File => self.root.remove_file(path),
			Kind::Directory => self.root.remove_dir(path),
		}
		.map_err(failure)
	}

	/// Reads file type and byte length.
	pub fn stat(&self, path: &str) -> Result<Metadata, Error> {
		self.metadata(path)
	}

	/// Renames within the root. Existing destinations follow native rename
	/// semantics; use a fresh destination for portable behavior across
	/// platforms.
	pub fn rename(&self, source: &str, destination: &str) -> Result<(), Error> {
		validate_path(source, false)?;
		validate_path(destination, false)?;
		self.metadata(source)?;
		self.root
			.rename(source, &self.root, destination)
			.map_err(failure)
	}
}

impl Plugin for Filesystem {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		if !METHODS.contains(&method) {
			return Err(Error::Unavailable);
		}
		if payload.len() > MAX_FILE_BYTES + MAX_PATH_BYTES + 8 {
			return Err(Error::TooLarge);
		}
		let mut input = Decoder::new(&payload);
		let path = input.string()?;
		let second = if method == WRITE || method == RENAME {
			Some(input.bytes()?)
		} else {
			None
		};
		input.finish()?;
		match method {
			READ => self.read(path),
			WRITE => {
				self.write(path, second.ok_or(Error::InvalidArgument)?)?;
				Ok(Bytes::new())
			}
			LIST => encode_names(&self.list(path)?),
			MKDIR => {
				self.mkdir(path)?;
				Ok(Bytes::new())
			}
			REMOVE => {
				self.remove(path)?;
				Ok(Bytes::new())
			}
			STAT => Ok(self.stat(path)?.encode()),
			RENAME => {
				self.rename(
					path,
					std::str::from_utf8(second.ok_or(Error::InvalidArgument)?)
						.map_err(|_| Error::InvalidArgument)?,
				)?;
				Ok(Bytes::new())
			}
			_ => Err(Error::Unavailable),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn atomic_replacements_never_expose_partial_contents() {
		use std::sync::Arc;
		use std::sync::atomic::AtomicBool;
		use std::sync::atomic::Ordering;
		let root = tempfile::tempdir().unwrap();
		let fs = Arc::new(Filesystem::open(root.path()).unwrap());
		let stop = Arc::new(AtomicBool::new(false));
		let old = vec![1; 4096];
		let new = vec![2; 8192];
		fs.write("item", &old).unwrap();
		let reader = {
			let fs = Arc::clone(&fs);
			let stop = Arc::clone(&stop);
			let old = old.clone();
			let new = new.clone();
			std::thread::spawn(move || {
				while !stop.load(Ordering::Acquire) {
					let contents = fs.read("item").unwrap();
					assert!(contents.as_ref() == old || contents.as_ref() == new);
				}
			})
		};
		for _ in 0..20 {
			fs.write("item", &new).unwrap();
			fs.write("item", &old).unwrap();
		}
		stop.store(true, Ordering::Release);
		reader.join().unwrap();
		assert_eq!(fs.list("").unwrap(), ["item"]);
	}

	#[test]
	fn application_factory_uses_a_dedicated_private_subdirectory() {
		let root = tempfile::tempdir().unwrap();
		let cache = tempfile::tempdir().unwrap();
		let fs = from_app_dirs(root.path(), cache.path()).unwrap();
		fs.write("owned", b"private").unwrap();
		assert!(root.path().join("files/owned").is_file());
		assert!(!cache.path().join("owned").exists());
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			assert_eq!(
				std::fs::metadata(root.path().join("files"))
					.unwrap()
					.permissions()
					.mode()
					& 0o777,
				0o700
			);
		}
	}
	#[test]
	fn lifecycle_and_failed_atomic_write_preserve_existing_contents() {
		let root = tempfile::tempdir().unwrap();
		let fs = Filesystem::open(root.path()).unwrap();
		fs.mkdir("notes/drafts").unwrap();
		fs.write("notes/item", b"old").unwrap();
		assert_eq!(
			fs.write("notes/item", &vec![1; MAX_FILE_BYTES + 1]),
			Err(Error::TooLarge)
		);
		assert_eq!(fs.read("notes/item").unwrap().as_ref(), b"old");
		fs.write("notes/item", b"new\0binary").unwrap();
		fs.rename("notes/item", "notes/drafts/item").unwrap();
		assert_eq!(
			fs.stat("notes/drafts/item").unwrap(),
			Metadata {
				kind: Kind::File,
				len: 10
			}
		);
		assert_eq!(fs.list("notes/drafts").unwrap(), ["item"]);
		assert!(fs.remove("notes").is_err());
		fs.remove("notes/drafts/item").unwrap();
		fs.remove("notes/drafts").unwrap();
		fs.remove("notes").unwrap();
		assert!(fs.list("").unwrap().is_empty());
		assert!(fs.read("missing").is_err());
	}
	// Unix symlinks; the Windows equivalent is the junction test below.
	#[cfg(unix)]
	#[test]
	fn symlinks_cannot_read_write_or_rename_outside_root() {
		let root = tempfile::tempdir().unwrap();
		let outside = tempfile::tempdir().unwrap();
		std::fs::write(outside.path().join("secret"), b"private").unwrap();
		std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
		std::os::unix::fs::symlink(outside.path().join("secret"), root.path().join("link"))
			.unwrap();
		let fs = Filesystem::open(root.path()).unwrap();
		assert!(fs.read("escape/secret").is_err());
		assert!(fs.read("link").is_err());
		assert!(fs.write("escape/secret", b"bad").is_err());
		assert!(fs.write("link", b"bad").is_err());
		fs.write("local", b"data").unwrap();
		assert!(fs.rename("local", "escape/new").is_err());
		assert_eq!(
			std::fs::read(outside.path().join("secret")).unwrap(),
			b"private"
		);
		assert_eq!(fs.list("").unwrap(), ["local"]);
	}
	// Windows creates directory junctions without the symlink privilege, so
	// they are the escape an unprivileged process can plant there.
	#[cfg(windows)]
	#[test]
	fn junctions_cannot_read_write_or_rename_outside_root() {
		let root = tempfile::tempdir().unwrap();
		let outside = tempfile::tempdir().unwrap();
		std::fs::write(outside.path().join("secret"), b"private").unwrap();
		let status = std::process::Command::new("cmd")
			.args(["/C", "mklink", "/J"])
			.arg(root.path().join("escape"))
			.arg(outside.path())
			.status()
			.unwrap();
		assert!(status.success(), "create a directory junction");
		let fs = Filesystem::open(root.path()).unwrap();
		assert!(fs.read("escape/secret").is_err());
		assert!(fs.write("escape/secret", b"bad").is_err());
		fs.write("local", b"data").unwrap();
		assert!(fs.rename("local", "escape/new").is_err());
		assert_eq!(
			std::fs::read(outside.path().join("secret")).unwrap(),
			b"private"
		);
		assert!(!outside.path().join("new").exists());
		assert_eq!(fs.list("").unwrap(), ["local"]);
	}
	#[test]
	fn list_skips_in_flight_temporaries_on_every_platform() {
		let root = tempfile::tempdir().unwrap();
		let fs = Filesystem::open(root.path()).unwrap();
		fs.write("kept", b"data").unwrap();
		std::fs::write(
			root.path().join(format!("{TEMP_PREFIX}pending")),
			b"partial",
		)
		.unwrap();
		assert_eq!(fs.list("").unwrap(), ["kept"]);
	}
	// These names exist only on Unix: Windows strips trailing spaces, treats
	// `:` as an alternate data stream and maps `CON` to a device, so it cannot
	// create them. The Windows equivalent is the test below.
	#[cfg(unix)]
	#[test]
	fn list_skips_names_the_guest_cannot_address() {
		let root = tempfile::tempdir().unwrap();
		let fs = Filesystem::open(root.path()).unwrap();
		fs.write("kept", b"data").unwrap();
		for name in ["trailing ", "CON.log", "report:1.txt"] {
			std::fs::write(root.path().join(name), b"host").unwrap();
		}
		// Linux filesystems accept arbitrary bytes; APFS rejects non-UTF-8.
		#[cfg(target_os = "linux")]
		{
			use std::os::unix::ffi::OsStringExt as _;
			let name = std::ffi::OsString::from_vec(vec![b'a', 0xff]);
			std::fs::write(root.path().join(name), b"host").unwrap();
		}
		assert_eq!(fs.list("").unwrap(), ["kept"]);
	}
	// NTFS stores UTF-16 code units without validating them, so a host can
	// create a name with an unpaired surrogate that no guest string can name.
	#[cfg(windows)]
	#[test]
	fn list_skips_names_that_are_not_unicode_on_windows() {
		use std::os::windows::ffi::OsStringExt as _;
		let root = tempfile::tempdir().unwrap();
		let fs = Filesystem::open(root.path()).unwrap();
		fs.write("kept", b"data").unwrap();
		let name = std::ffi::OsString::from_wide(&[u16::from(b'a'), 0xD800]);
		std::fs::write(root.path().join(name), b"host").unwrap();
		assert_eq!(fs.list("").unwrap(), ["kept"]);
	}

	#[test]
	fn dispatch_rejects_trailing_bytes_before_mutation() {
		let root = tempfile::tempdir().unwrap();
		let mut fs = Filesystem::open(root.path()).unwrap();
		let mut input = request("item", false).unwrap();
		input.bytes(b"contents").unwrap();
		input.u32(123);
		assert_eq!(
			fs.invoke(WRITE, input.finish()),
			Err(Error::InvalidArgument)
		);
		assert!(fs.list("").unwrap().is_empty());
		std::fs::write(root.path().join("large"), vec![0; MAX_FILE_BYTES + 1]).unwrap();
		assert_eq!(fs.read("large"), Err(Error::TooLarge));
	}

	#[test]
	fn every_method_round_trips_through_the_bridge_adapter() {
		let root = tempfile::tempdir().unwrap();
		let mut fs = Filesystem::open(root.path()).unwrap();
		assert_eq!(fs.name(), NAME);
		assert_eq!(fs.methods(), METHODS);
		let path = |path: &str| request(path, true).unwrap().finish();
		let with_second = |path: &str, second: &[u8]| {
			let mut input = request(path, false).unwrap();
			input.bytes(second).unwrap();
			input.finish()
		};

		assert_eq!(fs.invoke(MKDIR, path("notes")), Ok(Bytes::new()));
		assert_eq!(
			fs.invoke(WRITE, with_second("notes/a", b"alpha")),
			Ok(Bytes::new())
		);
		assert_eq!(fs.invoke(READ, path("notes/a")).unwrap().as_ref(), b"alpha");
		assert_eq!(
			Metadata::decode(&fs.invoke(STAT, path("notes/a")).unwrap()),
			Ok(Metadata {
				kind: Kind::File,
				len: 5
			})
		);
		assert_eq!(
			fs.invoke(RENAME, with_second("notes/a", b"notes/b")),
			Ok(Bytes::new())
		);
		assert_eq!(
			decode_names(&fs.invoke(LIST, path("notes")).unwrap()),
			Ok(vec!["b".to_owned()])
		);
		assert_eq!(fs.invoke(REMOVE, path("notes/b")), Ok(Bytes::new()));

		assert_eq!(fs.invoke(MethodId(99), path("")), Err(Error::Unavailable));
		assert_eq!(
			fs.invoke(RENAME, with_second("notes", &[0xff])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			fs.invoke(
				READ,
				Bytes::from(vec![0; MAX_FILE_BYTES + MAX_PATH_BYTES + 9])
			),
			Err(Error::TooLarge)
		);
		assert_eq!(fs.read("notes"), Err(Error::InvalidArgument));
	}

	#[test]
	fn platform_errors_map_to_bridge_errors() {
		use std::io::ErrorKind;
		for (kind, expected) in [
			(ErrorKind::PermissionDenied, Error::PermissionDenied),
			(ErrorKind::InvalidInput, Error::InvalidArgument),
			(ErrorKind::InvalidData, Error::InvalidArgument),
			(ErrorKind::NotFound, Error::Native),
		] {
			assert_eq!(failure(std::io::Error::from(kind)), expected);
		}
	}
}
