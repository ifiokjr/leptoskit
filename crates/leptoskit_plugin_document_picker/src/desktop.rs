//! Direct desktop adapter for apps with an explicit live parent window.
//!
//! Uses the maintained native backend already shared by the dialog crate.
//! Windows requires COM and a message loop; Linux requires an available XDG
//! desktop portal. Start on the owning UI thread, keep the parent alive until
//! the native dialog closes, including after dropping this future. The backend
//! cannot distinguish dismissal from every native presentation failure: either
//! becomes `Error::Cancelled`, never a successful empty document.
use std::fs::OpenOptions;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use futures::channel::oneshot;
use leptoskit_plugin_dialog::desktop::FileDialog;
use leptoskit_plugin_dialog::desktop::FileOptions;
pub use leptoskit_plugin_dialog::desktop::HasDisplayHandle;
pub use leptoskit_plugin_dialog::desktop::HasWindowHandle;

use crate::Document;
use crate::Error;
use crate::MAX_METADATA_BYTES;
use crate::Options;

struct CancelRead(Arc<AtomicBool>);
static READERS: AtomicUsize = AtomicUsize::new(0);

struct ReadPermit;
impl ReadPermit {
	fn acquire() -> Result<Self, Error> {
		READERS
			.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
				(count < 4).then_some(count + 1)
			})
			.map_err(|_| Error::Busy)?;
		Ok(Self)
	}
}
impl Drop for ReadPermit {
	fn drop(&mut self) {
		READERS.fetch_sub(1, Ordering::AcqRel);
	}
}
impl Drop for CancelRead {
	fn drop(&mut self) {
		self.0.store(true, Ordering::Release);
	}
}

/// Selects one file and copies at most the requested content limit off-thread.
/// This host API is deliberately not a synchronous guest bridge handler.
/// Selection grants no persistent guest path access. MIME type is unknown.
pub async fn pick<W: HasWindowHandle + HasDisplayHandle + ?Sized>(
	parent: &W,
	options: Options,
) -> Result<Document, Error> {
	let dialog = FileDialog::new(parent, FileOptions::default()).map_err(|_| Error::Unavailable)?;
	let path = dialog.open_file().await.ok_or(Error::Cancelled)?;
	import(path, options).await
}

/// Copies a selected file on a worker thread that holds a reader permit until
/// it finishes. Dropping the future cancels the read at its next chunk.
async fn import(path: PathBuf, options: Options) -> Result<Document, Error> {
	let permit = ReadPermit::acquire()?;
	let cancel = CancelRead(Arc::new(AtomicBool::new(false)));
	let worker_cancel = cancel.0.clone();
	let (sender, receiver) = oneshot::channel();
	std::thread::Builder::new()
		.name("document-import".into())
		.spawn(move || {
			let _permit = permit;
			let result = read_file(&path, options, &worker_cancel);
			let _ = sender.send(result);
		})
		.map_err(|_| Error::Native)?;
	receiver.await.map_err(|_| Error::Native)?
}

fn read_file(path: &Path, options: Options, cancelled: &AtomicBool) -> Result<Document, Error> {
	if cancelled.load(Ordering::Acquire) {
		return Err(Error::Cancelled);
	}
	let name = path
		.file_name()
		.and_then(|name| name.to_str())
		.ok_or(Error::InvalidArgument)?;
	if name.is_empty() || name.len() > MAX_METADATA_BYTES {
		return Err(Error::TooLarge);
	}
	let mut open = OpenOptions::new();
	open.read(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt as _;
		open.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
	}
	#[cfg(windows)]
	{
		use std::os::windows::fs::OpenOptionsExt as _;
		// Open a link itself rather than following a swapped reparse target.
		open.custom_flags(0x0020_0000);
	}
	let mut file = open.open(path).map_err(|error| io_error(&error))?;
	let metadata = file.metadata().map_err(|error| io_error(&error))?;
	if !metadata.is_file() {
		return Err(Error::InvalidArgument);
	}
	if metadata.len() > u64::from(options.max_bytes()) {
		return Err(Error::TooLarge);
	}
	let mut content = Vec::new();
	let mut chunk = vec![0_u8; 65536].into_boxed_slice();
	loop {
		if cancelled.load(Ordering::Acquire) {
			return Err(Error::Cancelled);
		}
		let count = chunk
			.len()
			.min(options.max_bytes() as usize - content.len() + 1);
		let count = file
			.read(&mut chunk[..count])
			.map_err(|error| io_error(&error))?;
		if count == 0 {
			break;
		}
		if count > options.max_bytes() as usize - content.len() {
			return Err(Error::TooLarge);
		}
		content.extend_from_slice(&chunk[..count]);
	}
	Ok(Document {
		name: name.to_owned(),
		mime_type: None,
		bytes: content.into(),
	})
}

fn io_error(error: &std::io::Error) -> Error {
	if error.kind() == std::io::ErrorKind::PermissionDenied {
		Error::PermissionDenied
	} else {
		Error::Native
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn owned_fixture_is_bounded_and_cancelled_before_opening() {
		let directory =
			std::env::temp_dir().join(format!("leptoskit-document-{}", std::process::id()));
		std::fs::create_dir(&directory).unwrap();
		let path = directory.join("fixture.txt");
		std::fs::write(&path, b"abc").unwrap();
		let cancelled = AtomicBool::new(false);
		assert_eq!(
			read_file(&path, Options::new(3).unwrap(), &cancelled)
				.unwrap()
				.bytes,
			b"abc"[..]
		);
		assert_eq!(
			read_file(&path, Options::new(2).unwrap(), &cancelled),
			Err(Error::TooLarge)
		);
		assert!(read_file(&directory, Options::default(), &cancelled).is_err());
		#[cfg(unix)]
		{
			let link = directory.join("link.txt");
			std::os::unix::fs::symlink(&path, &link).unwrap();
			assert!(read_file(&link, Options::default(), &cancelled).is_err());
		}
		std::fs::remove_dir_all(&directory).unwrap();
		cancelled.store(true, Ordering::Release);
		assert_eq!(
			read_file(&path, Options::default(), &cancelled),
			Err(Error::Cancelled)
		);
	}

	// Windows has no owner mode bits to clear; its unreadable file is one that
	// another process holds open without sharing, tested below.
	#[cfg(unix)]
	#[test]
	fn unreadable_files_report_permission_denied() {
		use std::os::unix::fs::PermissionsExt as _;
		let directory =
			std::env::temp_dir().join(format!("leptoskit-document-private-{}", std::process::id()));
		std::fs::create_dir(&directory).unwrap();
		let path = directory.join("private.txt");
		std::fs::write(&path, b"secret").unwrap();
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
		let result = read_file(&path, Options::default(), &AtomicBool::new(false));
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
		std::fs::remove_dir_all(&directory).unwrap();
		// Root can read any file, so only assert the mapping when access fails.
		if result.is_err() {
			assert_eq!(result, Err(Error::PermissionDenied));
		}
	}

	#[cfg(windows)]
	#[test]
	fn files_held_open_without_sharing_cannot_be_imported() {
		use std::os::windows::fs::OpenOptionsExt as _;
		let directory =
			std::env::temp_dir().join(format!("leptoskit-document-held-{}", std::process::id()));
		std::fs::create_dir(&directory).unwrap();
		let path = directory.join("held.txt");
		std::fs::write(&path, b"secret").unwrap();
		let held = OpenOptions::new()
			.read(true)
			.share_mode(0)
			.open(&path)
			.unwrap();
		let result = read_file(&path, Options::default(), &AtomicBool::new(false));
		drop(held);
		std::fs::remove_dir_all(&directory).unwrap();
		assert!(result.is_err());
	}

	/// Tests that take reader permits share one global admission count.
	static PERMITS: std::sync::Mutex<()> = std::sync::Mutex::new(());

	/// Waits for every reader permit to be released.
	///
	/// An import's result reaches its caller before the worker that sent it
	/// has dropped its permit: the permit is held until the worker's
	/// ownership ends, which
	/// `reader_admission_stays_reserved_until_worker_ownership_ends` asserts.
	/// A test that counts permits after an import has to wait for that
	/// worker, not assume it is gone.
	fn wait_for_idle_readers() {
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
		while READERS.load(Ordering::Acquire) != 0 {
			assert!(
				std::time::Instant::now() < deadline,
				"{} reader permits were still held 5s after their imports returned",
				READERS.load(Ordering::Acquire)
			);
			std::thread::yield_now();
		}
	}

	#[test]
	fn names_longer_than_the_metadata_limit_are_too_large() {
		let path = std::env::temp_dir().join("n".repeat(MAX_METADATA_BYTES + 1));
		assert_eq!(
			read_file(&path, Options::default(), &AtomicBool::new(false)),
			Err(Error::TooLarge)
		);
	}

	#[test]
	fn imports_copy_off_thread_and_refuse_beyond_reader_admission() {
		let _serial = PERMITS.lock().unwrap();
		// Under `cargo test`, the previous permit test shares this process.
		wait_for_idle_readers();
		let directory =
			std::env::temp_dir().join(format!("leptoskit-document-import-{}", std::process::id()));
		std::fs::create_dir(&directory).unwrap();
		let path = directory.join("import.txt");
		std::fs::write(&path, b"imported").unwrap();
		let document =
			futures::executor::block_on(import(path.clone(), Options::default())).unwrap();
		assert_eq!(document.name, "import.txt");
		assert_eq!(document.bytes, b"imported"[..]);
		// The import's worker may still hold its permit; the four below must
		// be the only ones held.
		wait_for_idle_readers();
		let permits = (0..4)
			.map(|_| ReadPermit::acquire().unwrap())
			.collect::<Vec<_>>();
		assert_eq!(
			futures::executor::block_on(import(path, Options::default())),
			Err(Error::Busy)
		);
		drop(permits);
		std::fs::remove_dir_all(&directory).unwrap();
	}

	#[test]
	fn reader_admission_stays_reserved_until_worker_ownership_ends() {
		let _serial = PERMITS.lock().unwrap();
		wait_for_idle_readers();
		let mut permits = (0..4)
			.map(|_| ReadPermit::acquire().unwrap())
			.collect::<Vec<_>>();
		let permit = permits.pop().unwrap();
		let cancellation = Arc::new(AtomicBool::new(false));
		let caller = CancelRead(cancellation.clone());
		let (release, finish) = std::sync::mpsc::channel();
		let worker = std::thread::spawn(move || {
			let _permit = permit;
			finish.recv().unwrap();
		});
		drop(caller);
		assert!(cancellation.load(Ordering::Acquire));
		assert!(matches!(ReadPermit::acquire(), Err(Error::Busy)));
		release.send(()).unwrap();
		worker.join().unwrap();
		assert!(ReadPermit::acquire().is_ok());
		drop(permits);
	}
}
