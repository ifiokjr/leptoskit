use std::io::Read as _;
use std::io::Seek as _;
use std::io::SeekFrom;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use cap_std::fs::Dir;
use cap_std::fs::File;
use leptoskit_bridge::Plugin;

use super::*;

const ACTIVE: &str = "current.jsonl";
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FILES: u8 = 16;

/// Trusted host configuration. Guests cannot select sinks, paths, or retention.
#[derive(Debug, Clone, Copy)]
pub struct Config {
	/// Records below this level are acknowledged as filtered.
	pub minimum: Level,
	/// Include app-owned rotating files.
	pub files: bool,
	/// Also write JSON lines to process stderr. Disabled by default.
	pub stderr: bool,
	/// Maximum bytes per file, from 128 bytes to 16 MiB.
	pub max_file_bytes: u64,
	/// Total retained files, including current, from 1 to 16.
	pub max_files: u8,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			minimum: Level::Info,
			files: true,
			stderr: false,
			max_file_bytes: 1024 * 1024,
			max_files: 4,
		}
	}
}
impl Config {
	fn validate(self) -> Result<(), Error> {
		if (!self.files && !self.stderr)
			|| !(128..=MAX_FILE_BYTES).contains(&self.max_file_bytes)
			|| !(1..=MAX_FILES).contains(&self.max_files)
		{
			return Err(Error::InvalidArgument);
		}
		Ok(())
	}
}

fn failure(error: &std::io::Error) -> Error {
	match error.kind() {
		std::io::ErrorKind::PermissionDenied => Error::PermissionDenied,
		std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData => {
			Error::InvalidArgument
		}
		_ => Error::Native,
	}
}

fn filename(index: u8) -> String {
	if index == 0 {
		ACTIVE.into()
	} else {
		format!("{index}.jsonl")
	}
}

// Rust 1.90's Unix File::try_lock implementation excludes Android even though
// Bionic supports flock. Keep the same nonblocking, file-description-owned lock
// there instead of treating Unsupported as permission to write without a lock.
#[cfg(any(
	target_os = "android",
	all(test, any(target_os = "linux", target_vendor = "apple"))
))]
fn flock_exclusive(file: &std::fs::File) -> Result<(), Error> {
	use std::os::fd::AsRawFd as _;

	// SAFETY: The borrowed File owns a live descriptor for this entire call.
	// flock neither takes ownership nor retains a pointer into Rust memory.
	// LOCK_NB prevents blocking the worker on another logger's lifetime.
	#[expect(
		unsafe_code,
		reason = "Bionic file locking requires this audited FFI call"
	)]
	let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
	if result == 0 {
		return Ok(());
	}
	let error = std::io::Error::last_os_error();
	if error.kind() == std::io::ErrorKind::WouldBlock {
		Err(Error::Busy)
	} else {
		Err(failure(&error))
	}
}

struct FileSink {
	root: Dir,
	current: Option<File>,
	size: u64,
	_lock: std::fs::File,
}

impl FileSink {
	fn open(data_dir: &Path, config: Config) -> Result<Self, Error> {
		let parent = Dir::open_ambient_dir(data_dir, cap_std::ambient_authority())
			.map_err(|error| failure(&error))?;
		let mut builder = cap_std::fs::DirBuilder::new();
		builder.recursive(true);
		#[cfg(unix)]
		{
			use cap_std::fs::DirBuilderExt as _;
			builder.mode(0o700);
		}
		parent
			.create_dir_with("logs", &builder)
			.map_err(|error| failure(&error))?;
		let root = parent.open_dir("logs").map_err(|error| failure(&error))?;
		#[cfg(unix)]
		{
			use cap_std::fs::PermissionsExt as _;
			root.set_permissions(".", cap_std::fs::Permissions::from_mode(0o700))
				.map_err(|error| failure(&error))?;
		}
		let lock = Self::lock(&root)?;
		let mut sink = Self {
			root,
			_lock: lock,
			current: None,
			size: 0,
		};
		sink.preflight(config)?;
		for index in config.max_files..MAX_FILES {
			match sink.root.remove_file(filename(index)) {
				Ok(()) => {}
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
				Err(error) => return Err(failure(&error)),
			}
		}
		sink.open_current(config)?;
		Ok(sink)
	}

	fn lock(root: &Dir) -> Result<std::fs::File, Error> {
		match root.symlink_metadata(".writer.lock") {
			Ok(metadata) if !metadata.is_file() => return Err(Error::PermissionDenied),
			Ok(_) => {}
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
			Err(error) => return Err(failure(&error)),
		}
		let mut options = cap_std::fs::OpenOptions::new();
		options.read(true).write(true).create(true);
		#[cfg(unix)]
		{
			use cap_std::fs::OpenOptionsExt as _;
			options
				.mode(0o600)
				.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
		}
		let lock = root
			.open_with(".writer.lock", &options)
			.map_err(|error| failure(&error))?;
		let metadata = lock.metadata().map_err(|error| failure(&error))?;
		if !metadata.is_file() {
			return Err(Error::PermissionDenied);
		}
		if metadata.len() != 0 {
			return Err(Error::InvalidArgument);
		}
		let lock = lock.into_std();
		#[cfg(target_os = "android")]
		flock_exclusive(&lock)?;
		#[cfg(not(target_os = "android"))]
		lock.try_lock().map_err(|error| {
			match error {
				std::fs::TryLockError::WouldBlock => Error::Busy,
				std::fs::TryLockError::Error(error) => failure(&error),
			}
		})?;
		Ok(lock)
	}

	fn preflight(&self, config: Config) -> Result<(), Error> {
		for index in 0..MAX_FILES {
			match self.root.symlink_metadata(filename(index)) {
				Ok(metadata) if !metadata.is_file() => return Err(Error::PermissionDenied),
				Ok(metadata) if metadata.len() > config.max_file_bytes => {
					return Err(Error::TooLarge);
				}
				Ok(_) => {}
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
				Err(error) => return Err(failure(&error)),
			}
		}
		Ok(())
	}

	fn open_current(&mut self, config: Config) -> Result<(), Error> {
		let mut options = cap_std::fs::OpenOptions::new();
		options.read(true).append(true).create(true);
		#[cfg(unix)]
		{
			use cap_std::fs::OpenOptionsExt as _;
			options
				.mode(0o600)
				.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
		}
		let mut file = self
			.root
			.open_with(ACTIVE, &options)
			.map_err(|error| failure(&error))?;
		let metadata = file.metadata().map_err(|error| failure(&error))?;
		if !metadata.is_file() {
			return Err(Error::PermissionDenied);
		}
		if metadata.len() > config.max_file_bytes {
			return Err(Error::TooLarge);
		}
		if metadata.len() != 0 {
			file.seek(SeekFrom::End(-1))
				.map_err(|error| failure(&error))?;
			let mut tail = [0];
			file.read_exact(&mut tail)
				.map_err(|error| failure(&error))?;
			// Never append a new event onto a partial record from an earlier
			// I/O failure.
			if tail[0] != b'\n' {
				return Err(Error::InvalidArgument);
			}
		}
		self.size = metadata.len();
		self.current = Some(file);
		Ok(())
	}

	fn rotate(&mut self, config: Config) -> Result<(), Error> {
		self.preflight(config)?;
		self.flush()?;
		self.current = None;
		for index in (0..config.max_files).rev() {
			let source = filename(index);
			match self.root.symlink_metadata(&source) {
				Ok(metadata) if !metadata.is_file() => return Err(Error::PermissionDenied),
				Ok(_) => {}
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
				Err(error) => return Err(failure(&error)),
			}
			if index == config.max_files - 1 {
				self.root
					.remove_file(source)
					.map_err(|error| failure(&error))?;
			} else {
				self.root
					.rename(source, &self.root, filename(index + 1))
					.map_err(|error| failure(&error))?;
			}
		}
		self.open_current(config)
	}

	fn write(&mut self, bytes: &[u8], config: Config) -> Result<(), Error> {
		if self.size + bytes.len() as u64 > config.max_file_bytes {
			self.rotate(config)?;
		}
		self.current
			.as_mut()
			.ok_or(Error::Native)?
			.write_all(bytes)
			.map_err(|error| failure(&error))?;
		self.size += bytes.len() as u64;
		Ok(())
	}

	fn flush(&self) -> Result<(), Error> {
		self.current
			.as_ref()
			.ok_or(Error::Native)?
			.sync_all()
			.map_err(|error| failure(&error))
	}
}

/// A worker-owned logger with no global registration or unbounded background
/// queue. First accepted record lazily opens the trusted application log root.
pub struct Logger {
	data_dir: PathBuf,
	config: Config,
	file: Option<FileSink>,
	failed: bool,
	closed: bool,
}

/// Automatic registration factory; performs no filesystem I/O.
pub fn from_app_dirs(data_dir: &Path, _cache_dir: &Path) -> Result<Logger, Error> {
	Logger::new(data_dir, Config::default())
}

impl Logger {
	/// Captures a trusted root and validates retention settings. Run operations
	/// off the UI thread; the generated host uses the Rust worker adapter.
	pub fn new(data_dir: impl Into<PathBuf>, config: Config) -> Result<Self, Error> {
		config.validate()?;
		Ok(Self {
			data_dir: data_dir.into(),
			config,
			file: None,
			failed: false,
			closed: false,
		})
	}

	fn ready(&self) -> Result<(), Error> {
		if self.closed {
			Err(Error::Cancelled)
		} else if self.failed {
			Err(Error::Native)
		} else {
			Ok(())
		}
	}

	/// Writes one event or returns false if filtered. Input validation errors
	/// do not disable the logger. Any sink I/O failure does: a partial line
	/// or an already-written sibling sink must not be silently retried.
	pub fn write(&mut self, record: &Record) -> Result<bool, Error> {
		self.ready()?;
		record.validate()?;
		if record.level < self.config.minimum {
			return Ok(false);
		}
		let timestamp = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.map_err(|_| Error::Native)?;
		let timestamp = u64::try_from(timestamp.as_millis()).map_err(|_| Error::Native)?;
		let bytes = json(record, timestamp)?;
		if self.config.files && bytes.len() as u64 > self.config.max_file_bytes {
			return Err(Error::TooLarge);
		}
		let result = self.write_sinks(&bytes);
		if result.is_err() && result != Err(Error::Busy) {
			self.failed = true;
		}
		result.map(|()| true)
	}

	fn write_sinks(&mut self, bytes: &[u8]) -> Result<(), Error> {
		if self.config.files {
			if self.file.is_none() {
				self.file = Some(FileSink::open(&self.data_dir, self.config)?);
			}
			self.file
				.as_mut()
				.ok_or(Error::Native)?
				.write(bytes, self.config)?;
		}
		if self.config.stderr {
			std::io::stderr()
				.lock()
				.write_all(bytes)
				.map_err(|error| failure(&error))?;
		}
		Ok(())
	}

	/// Syncs the active file and flushes stderr. Does not create files if no
	/// record was accepted. Filesystem directory metadata durability is not
	/// promised.
	pub fn flush(&mut self) -> Result<(), Error> {
		self.ready()?;
		let result = self.flush_sinks();
		if result.is_err() {
			self.failed = true;
		}
		result
	}

	fn flush_sinks(&self) -> Result<(), Error> {
		if let Some(file) = &self.file {
			file.flush()?;
		}
		if self.config.stderr {
			std::io::stderr()
				.lock()
				.flush()
				.map_err(|error| failure(&error))?;
		}
		Ok(())
	}

	/// Attempts flush, releases handles even on failure, and permanently
	/// closes. Call explicitly when errors matter; Drop cannot report flush
	/// failures and therefore performs no implicit sync. Repeated close
	/// calls succeed.
	pub fn close(&mut self) -> Result<(), Error> {
		if self.closed {
			return Ok(());
		}
		let result = self.flush();
		self.file = None;
		self.closed = true;
		result
	}
}

impl Plugin for Logger {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		match method {
			WRITE => {
				Ok(u32::from(self.write(&Record::decode(&payload)?)?)
					.to_le_bytes()
					.to_vec()
					.into())
			}
			FLUSH | CLOSE => {
				if !payload.is_empty() {
					return Err(Error::InvalidArgument);
				}
				if method == FLUSH {
					self.flush()?;
				} else {
					self.close()?;
				}
				Ok(Bytes::new())
			}
			_ => Err(Error::Unavailable),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn record(index: usize) -> Record {
		Record {
			level: Level::Info,
			target: "tests".into(),
			message: format!("synthetic {index}"),
			fields: BTreeMap::new(),
		}
	}
	#[test]
	fn rotation_is_bounded_and_retains_newest_complete_records() {
		let root = tempfile::tempdir().unwrap();
		let config = Config {
			max_file_bytes: 128,
			max_files: 3,
			..Config::default()
		};
		let mut logger = Logger::new(root.path(), config).unwrap();
		assert!(!root.path().join("logs").exists());
		for index in 0..8 {
			logger.write(&record(index)).unwrap();
		}
		logger.close().unwrap();
		let mut retained = Vec::new();
		for index in (0..3).rev() {
			let data = std::fs::read(root.path().join("logs").join(filename(index))).unwrap();
			assert!(data.len() <= 128);
			let entry: serde_json::Value = serde_json::from_slice(&data).unwrap();
			retained.push(entry["message"].as_str().unwrap().to_owned());
		}
		assert_eq!(retained, ["synthetic 5", "synthetic 6", "synthetic 7"]);
		assert_eq!(
			std::fs::read_dir(root.path().join("logs")).unwrap().count(),
			4,
			"three logs and one lock"
		);
		assert_eq!(logger.write(&record(9)), Err(Error::Cancelled));
		logger.close().unwrap();
		let mut reopened = Logger::new(root.path(), config).unwrap();
		assert_eq!(reopened.write(&record(10)), Ok(true));
		reopened.flush().unwrap();
	}
	#[test]
	fn concurrent_writers_are_rejected_until_the_owner_closes() {
		let root = tempfile::tempdir().unwrap();
		let mut first = Logger::new(root.path(), Config::default()).unwrap();
		first.write(&record(0)).unwrap();
		let mut second = Logger::new(root.path(), Config::default()).unwrap();
		assert_eq!(second.write(&record(1)), Err(Error::Busy));
		first.close().unwrap();
		assert_eq!(
			second.write(&record(2)),
			Ok(true),
			"lock contention is retryable"
		);
		second.close().unwrap();
	}
	#[cfg(any(target_os = "android", target_os = "linux", target_vendor = "apple"))]
	#[test]
	fn android_flock_path_excludes_independent_descriptors_until_final_owner_drops() {
		let root = tempfile::tempdir().unwrap();
		let path = root.path().join("synthetic-writer.lock");
		let first = std::fs::OpenOptions::new()
			.read(true)
			.write(true)
			.create_new(true)
			.open(&path)
			.unwrap();
		let second = std::fs::OpenOptions::new()
			.read(true)
			.write(true)
			.open(&path)
			.unwrap();
		assert_eq!(flock_exclusive(&first), Ok(()));
		assert_eq!(flock_exclusive(&second), Err(Error::Busy));
		let duplicate = first.try_clone().unwrap();
		drop(first);
		assert_eq!(
			flock_exclusive(&second),
			Err(Error::Busy),
			"a duplicated descriptor still owns the same locked file description"
		);
		drop(duplicate);
		assert_eq!(flock_exclusive(&second), Ok(()));
	}
	#[test]
	fn adapter_checks_binary_replies_and_lifecycle_payloads() {
		let root = tempfile::tempdir().unwrap();
		let mut logger = Logger::new(root.path(), Config::default()).unwrap();
		assert_eq!(
			logger
				.invoke(WRITE, record(0).encode().unwrap())
				.unwrap()
				.as_ref(),
			&1_u32.to_le_bytes()
		);
		let mut filtered = record(1);
		filtered.level = Level::Debug;
		assert_eq!(
			logger
				.invoke(WRITE, filtered.encode().unwrap())
				.unwrap()
				.as_ref(),
			&0_u32.to_le_bytes()
		);
		for method in [FLUSH, CLOSE] {
			assert_eq!(
				logger.invoke(method, Bytes::from_static(b"bad")),
				Err(Error::InvalidArgument)
			);
		}
		assert!(logger.invoke(FLUSH, Bytes::new()).unwrap().is_empty());
		assert!(logger.invoke(CLOSE, Bytes::new()).unwrap().is_empty());
		assert_eq!(
			logger.invoke(WRITE, record(2).encode().unwrap()),
			Err(Error::Cancelled)
		);
	}
	#[test]
	fn unexpected_archive_entry_stops_rotation_without_appending() {
		let root = tempfile::tempdir().unwrap();
		let mut logger = Logger::new(
			root.path(),
			Config {
				max_file_bytes: 128,
				..Config::default()
			},
		)
		.unwrap();
		logger.write(&record(0)).unwrap();
		let directory = root.path().join("logs");
		let before = std::fs::read(directory.join(ACTIVE)).unwrap();
		// A directory where an archive belongs fails rotation on every
		// platform.
		std::fs::create_dir(directory.join(filename(1))).unwrap();
		assert_eq!(logger.write(&record(1)), Err(Error::PermissionDenied));
		assert_eq!(std::fs::read(directory.join(ACTIVE)).unwrap(), before);
		assert_eq!(logger.write(&record(2)), Err(Error::Native));
	}
	// Directory permissions make the rename itself fail for the owning user
	// only on Unix; Windows ignores the read-only attribute on directories.
	#[cfg(unix)]
	#[test]
	fn rotation_io_failure_is_reported_without_appending_the_new_record() {
		use std::os::unix::fs::PermissionsExt as _;
		let root = tempfile::tempdir().unwrap();
		let mut logger = Logger::new(
			root.path(),
			Config {
				max_file_bytes: 128,
				..Config::default()
			},
		)
		.unwrap();
		logger.write(&record(0)).unwrap();
		let directory = root.path().join("logs");
		let before = std::fs::read(directory.join(ACTIVE)).unwrap();
		std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500)).unwrap();
		let result = logger.write(&record(1));
		std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
		assert_eq!(result, Err(Error::PermissionDenied));
		assert_eq!(std::fs::read(directory.join(ACTIVE)).unwrap(), before);
		assert_eq!(logger.write(&record(2)), Err(Error::Native));
		assert_eq!(logger.close(), Err(Error::Native));
	}
	#[test]
	fn reducing_retention_prunes_old_managed_archives() {
		let root = tempfile::tempdir().unwrap();
		let mut logger = Logger::new(
			root.path(),
			Config {
				max_file_bytes: 128,
				max_files: 4,
				..Config::default()
			},
		)
		.unwrap();
		for index in 0..6 {
			logger.write(&record(index)).unwrap();
		}
		logger.close().unwrap();
		let mut reduced = Logger::new(
			root.path(),
			Config {
				max_file_bytes: 128,
				max_files: 1,
				..Config::default()
			},
		)
		.unwrap();
		reduced.write(&record(7)).unwrap();
		reduced.close().unwrap();
		assert_eq!(
			std::fs::read_dir(root.path().join("logs")).unwrap().count(),
			2
		);
		assert!(!root.path().join("logs/1.jsonl").exists());
	}
	#[test]
	fn filtering_limits_and_message_escaping_do_not_create_extra_lines() {
		let root = tempfile::tempdir().unwrap();
		let mut logger = Logger::new(root.path(), Config::default()).unwrap();
		let mut input = record(0);
		input.level = Level::Debug;
		assert_eq!(logger.write(&input), Ok(false));
		logger.flush().unwrap();
		assert!(!root.path().join("logs").exists());
		input.level = Level::Error;
		input.target = "../../outside".into();
		input.message = "\n{\"spoof\":true}\n".into();
		logger.write(&input).unwrap();
		logger.close().unwrap();
		let data = std::fs::read_to_string(root.path().join("logs/current.jsonl")).unwrap();
		assert_eq!(data.lines().count(), 1);
		let entry: serde_json::Value = serde_json::from_str(&data).unwrap();
		assert_eq!(entry["target"], "../../outside");
		assert_eq!(entry["message"], input.message);
		assert!(
			Logger::new(
				root.path(),
				Config {
					max_files: 0,
					..Config::default()
				}
			)
			.is_err()
		);
		assert!(
			Logger::new(
				root.path(),
				Config {
					files: false,
					stderr: false,
					..Config::default()
				}
			)
			.is_err()
		);
	}
	#[test]
	fn io_failure_is_reported_and_failed_logger_does_not_retry() {
		let root = tempfile::tempdir().unwrap();
		std::fs::write(root.path().join("logs"), b"not a directory").unwrap();
		let mut logger = Logger::new(root.path(), Config::default()).unwrap();
		assert!(logger.write(&record(0)).is_err());
		std::fs::remove_file(root.path().join("logs")).unwrap();
		assert_eq!(logger.write(&record(1)), Err(Error::Native));
		assert_eq!(logger.close(), Err(Error::Native));
		assert!(!root.path().join("logs").exists());
	}
	#[test]
	fn incomplete_existing_record_and_oversized_event_are_rejected() {
		let root = tempfile::tempdir().unwrap();
		std::fs::create_dir(root.path().join("logs")).unwrap();
		std::fs::write(root.path().join("logs/current.jsonl"), b"{partial").unwrap();
		let mut logger = Logger::new(root.path(), Config::default()).unwrap();
		assert_eq!(logger.write(&record(0)), Err(Error::InvalidArgument));
		std::fs::remove_file(root.path().join("logs/current.jsonl")).unwrap();
		let mut logger = Logger::new(
			root.path(),
			Config {
				max_file_bytes: 128,
				..Config::default()
			},
		)
		.unwrap();
		let mut long = record(0);
		long.message = "x".repeat(128);
		assert_eq!(logger.write(&long), Err(Error::TooLarge));
		assert_eq!(
			logger.write(&record(1)),
			Ok(true),
			"input failures must not poison the logger"
		);
	}
	// Unix symlinks; the Windows equivalent is the junction test below.
	#[cfg(unix)]
	#[test]
	fn symlink_paths_cannot_escape_the_application_root() {
		let root = tempfile::tempdir().unwrap();
		let outside = tempfile::tempdir().unwrap();
		let secret = outside.path().join("private");
		std::fs::write(&secret, b"do not touch\n").unwrap();
		std::fs::create_dir(root.path().join("logs")).unwrap();
		std::os::unix::fs::symlink(&secret, root.path().join("logs/current.jsonl")).unwrap();
		let mut logger = Logger::new(root.path(), Config::default()).unwrap();
		assert_eq!(logger.write(&record(0)), Err(Error::PermissionDenied));
		assert_eq!(std::fs::read(&secret).unwrap(), b"do not touch\n");
	}
	// Windows creates directory junctions without the symlink privilege, so
	// they are the escape an unprivileged process can plant there.
	#[cfg(windows)]
	#[test]
	fn junctioned_log_directory_cannot_escape_the_application_root() {
		let root = tempfile::tempdir().unwrap();
		let outside = tempfile::tempdir().unwrap();
		let secret = outside.path().join(ACTIVE);
		std::fs::write(&secret, b"do not touch\n").unwrap();
		let status = std::process::Command::new("cmd")
			.args(["/C", "mklink", "/J"])
			.arg(root.path().join("logs"))
			.arg(outside.path())
			.status()
			.unwrap();
		assert!(status.success(), "create a directory junction");
		if let Ok(mut logger) = Logger::new(root.path(), Config::default()) {
			assert!(logger.write(&record(0)).is_err());
		}
		assert_eq!(std::fs::read(&secret).unwrap(), b"do not touch\n");
	}
}
