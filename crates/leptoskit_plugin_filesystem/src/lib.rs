//! Application-private file storage with bounded binary messages.
//!
//! Native hosts supply an application-owned data directory. Every native
//! operation resolves relative to an open directory capability; symlinks cannot
//! escape it. The browser backend uses the origin-private file system (OPFS).

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
#[cfg(any(test, not(target_arch = "wasm32"), feature = "guest"))]
use leptoskit_bridge::codec::Encoder;

/// Stable plugin name.
pub const NAME: &str = "filesystem";
/// Read a file: length-prefixed path -> raw bytes.
pub const READ: MethodId = MethodId(0);
/// Atomically replace a file: length-prefixed path and bytes -> empty.
pub const WRITE: MethodId = MethodId(1);
/// List one directory: path -> count followed by length-prefixed UTF-8 names.
pub const LIST: MethodId = MethodId(2);
/// Create a directory, including missing parents: path -> empty.
pub const MKDIR: MethodId = MethodId(3);
/// Remove a file or empty directory: path -> empty. Never recursive.
pub const REMOVE: MethodId = MethodId(4);
/// Query metadata: path -> kind u32 (1=file, 2=directory), byte size u64 LE.
pub const STAT: MethodId = MethodId(5);
/// Rename within the root: source path, destination path -> empty.
pub const RENAME: MethodId = MethodId(6);
/// Supported methods.
pub const METHODS: &[MethodId] = &[READ, WRITE, LIST, MKDIR, REMOVE, STAT, RENAME];
/// Maximum single-file payload, deliberately below the default bridge limit.
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
/// Maximum encoded path length in bytes.
pub const MAX_PATH_BYTES: usize = 4096;
/// Maximum entries returned by one list operation.
pub const MAX_ENTRIES: usize = 1024;
/// Maximum listing response, including names and framing.
pub const MAX_LIST_BYTES: usize = 64 * 1024;
const TEMP_PREFIX: &str = ".leptoskit-tmp-";

/// Only regular files and directories are exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
	/// A regular file.
	File,
	/// A directory.
	Directory,
}

/// File metadata. Directory sizes are always zero for portability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metadata {
	/// Node type.
	pub kind: Kind,
	/// File length in bytes.
	pub len: u64,
}

impl Metadata {
	/// Encodes the fixed-width metadata response.
	pub fn encode(self) -> Bytes {
		let mut bytes = Vec::with_capacity(12);
		bytes.extend_from_slice(&(if self.kind == Kind::File { 1_u32 } else { 2 }).to_le_bytes());
		bytes.extend_from_slice(&self.len.to_le_bytes());
		bytes.into()
	}

	/// Validates the fixed-width metadata response.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut decoder = Decoder::new(bytes);
		let kind = match decoder.u32()? {
			1 => Kind::File,
			2 => Kind::Directory,
			_ => return Err(Error::InvalidArgument),
		};
		let len = u64::from(decoder.u32()?) | (u64::from(decoder.u32()?) << 32);
		decoder.finish()?;
		if kind == Kind::Directory && len != 0 {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { kind, len })
	}
}

/// Validates portable relative paths. Empty paths denote the root only for
/// `list` and `stat`. Dot segments, alternate separators, drive prefixes and
/// Windows device names and reserved temporary names are rejected before
/// platform APIs see them.
pub fn validate_path(path: &str, allow_root: bool) -> Result<(), Error> {
	if path.len() > MAX_PATH_BYTES {
		return Err(Error::TooLarge);
	}
	if path.is_empty() {
		return if allow_root {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		};
	}
	if path
		.chars()
		.any(|ch| ch.is_control() || ['\\', ':', '<', '>', '"', '|', '?', '*'].contains(&ch))
		|| path.split('/').any(|part| {
			let basename = part
				.split('.')
				.next()
				.unwrap_or_default()
				.to_ascii_uppercase();
			let device = matches!(basename.as_str(), "CON" | "PRN" | "AUX" | "NUL")
				|| (basename.len() == 4
					&& (basename.starts_with("COM") || basename.starts_with("LPT"))
					&& matches!(basename.as_bytes()[3], b'1'..=b'9'));
			part.is_empty()
				|| part == "."
				|| part == ".."
				|| part.ends_with(['.', ' '])
				|| part.starts_with(TEMP_PREFIX)
				|| device
		}) {
		return Err(Error::InvalidArgument);
	}
	Ok(())
}

#[cfg(any(
	all(test, not(target_arch = "wasm32")),
	all(target_arch = "wasm32", feature = "guest")
))]
fn request(path: &str, allow_root: bool) -> Result<Encoder, Error> {
	validate_path(path, allow_root)?;
	let mut encoder = Encoder::with_capacity(path.len() + 4);
	encoder.bytes(path.as_bytes())?;
	Ok(encoder)
}

#[cfg(any(test, not(target_arch = "wasm32")))]
fn encode_names(names: &[String]) -> Result<Bytes, Error> {
	if names.len() > MAX_ENTRIES {
		return Err(Error::TooLarge);
	}
	let size = names.iter().try_fold(4_usize, |size, name| {
		size.checked_add(4 + name.len()).ok_or(Error::TooLarge)
	})?;
	if size > MAX_LIST_BYTES {
		return Err(Error::TooLarge);
	}
	let mut encoder = Encoder::with_capacity(size);
	encoder.u32(u32::try_from(names.len()).map_err(|_| Error::TooLarge)?);
	for name in names {
		encoder.bytes(name.as_bytes())?;
	}
	Ok(encoder.finish())
}

#[cfg(any(test, all(target_arch = "wasm32", feature = "guest")))]
fn decode_names(bytes: &[u8]) -> Result<Vec<String>, Error> {
	if bytes.len() > MAX_LIST_BYTES {
		return Err(Error::TooLarge);
	}
	let mut decoder = Decoder::new(bytes);
	let count = decoder.u32()? as usize;
	if count > MAX_ENTRIES {
		return Err(Error::TooLarge);
	}
	let mut names = Vec::with_capacity(count);
	for _ in 0..count {
		let name = decoder.string()?;
		validate_path(name, false)?;
		if name.contains('/') {
			return Err(Error::InvalidArgument);
		}
		names.push(name.to_owned());
	}
	decoder.finish()?;
	Ok(names)
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::Filesystem;
#[cfg(not(target_arch = "wasm32"))]
pub use native::Quota;
#[cfg(not(target_arch = "wasm32"))]
pub use native::from_app_dirs;

/// Browser OPFS adapter, available only in secure contexts supporting OPFS.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

/// Cached numeric-ID client for native-hosted applications.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use leptoskit_bridge::guest::Client;

	use super::*;

	/// Resolves the filesystem plugin once.
	#[derive(Debug, Clone, Copy)]
	pub struct Filesystem(Client);

	impl Filesystem {
		/// Connects to the host's application-private filesystem.
		pub fn new() -> Result<Self, Error> {
			Client::new(NAME).map(Self)
		}

		/// Reads a regular file of at most [`MAX_FILE_BYTES`].
		pub async fn read(self, path: &str) -> Result<Bytes, Error> {
			let response = self.0.call(READ, &request(path, false)?.finish())?.await?;
			if response.len() > MAX_FILE_BYTES {
				return Err(Error::TooLarge);
			}
			Ok(response)
		}

		/// Atomically replaces file contents. Parent directories must exist.
		pub async fn write(self, path: &str, bytes: &[u8]) -> Result<(), Error> {
			if bytes.len() > MAX_FILE_BYTES {
				return Err(Error::TooLarge);
			}
			let mut input = request(path, false)?;
			input.bytes(bytes)?;
			Decoder::new(&self.0.call(WRITE, &input.finish())?.await?).finish()
		}

		/// Returns sorted immediate names. Empty path lists the root.
		pub async fn list(self, path: &str) -> Result<Vec<String>, Error> {
			decode_names(&self.0.call(LIST, &request(path, true)?.finish())?.await?)
		}

		/// Creates directories recursively, succeeding if already present.
		pub async fn mkdir(self, path: &str) -> Result<(), Error> {
			Decoder::new(&self.0.call(MKDIR, &request(path, false)?.finish())?.await?).finish()
		}

		/// Removes a file or empty directory. Missing paths fail.
		pub async fn remove(self, path: &str) -> Result<(), Error> {
			Decoder::new(
				&self
					.0
					.call(REMOVE, &request(path, false)?.finish())?
					.await?,
			)
			.finish()
		}

		/// Reads file type and length. Empty path queries the root.
		pub async fn stat(self, path: &str) -> Result<Metadata, Error> {
			Metadata::decode(&self.0.call(STAT, &request(path, true)?.finish())?.await?)
		}

		/// Renames a path within the same application root.
		pub async fn rename(self, source: &str, destination: &str) -> Result<(), Error> {
			validate_path(destination, false)?;
			let mut input = request(source, false)?;
			input.bytes(destination.as_bytes())?;
			Decoder::new(&self.0.call(RENAME, &input.finish())?.await?).finish()
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn rejects_ambiguous_paths_and_root_mutation() {
		for path in [
			"",
			"/etc/passwd",
			"a/../b",
			"a/./b",
			"a//b",
			"a/",
			"C:hello",
			"a\\b",
			"\0",
			"con",
			"NUL.txt",
			"a/LPT9",
			"name.",
			"name ",
			"a?",
		] {
			assert_eq!(
				validate_path(path, false),
				Err(Error::InvalidArgument),
				"{path:?}"
			);
		}
		assert!(validate_path("", true).is_ok());
		assert!(validate_path("notes/hello.txt", false).is_ok());
	}
	#[test]
	fn metadata_and_listing_reject_malformed_messages() {
		let meta = Metadata {
			kind: Kind::File,
			len: u64::MAX,
		};
		assert_eq!(Metadata::decode(&meta.encode()), Ok(meta));
		assert!(Metadata::decode(&[0; 12]).is_err());
		assert!(Metadata::decode(&meta.encode()[..11]).is_err());
		assert_eq!(
			decode_names(&encode_names(&["a".into(), "b".into()]).unwrap()).unwrap(),
			["a", "b"]
		);
		assert!(decode_names(&encode_names(&["../escape".into()]).unwrap()).is_err());
	}
}
