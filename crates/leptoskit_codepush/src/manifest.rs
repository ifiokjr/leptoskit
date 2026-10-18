//! Release manifests and the `.lkb` bundle container.
//!
//! A container is a single file: the magic bytes `LKB1`, a little-endian
//! manifest length, the JSON manifest, then each file's gzip-compressed bytes
//! in manifest order. One file to host, one file to download, one hash to
//! verify.

use std::fs;
use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use walkdir::WalkDir;

use crate::CodePushError;

/// The container magic bytes.
pub const CONTAINER_MAGIC: &[u8; 4] = b"LKB1";

/// The manifest format version.
pub const MANIFEST_FORMAT_VERSION: u32 = 1;

/// The SHA-256 of `data`, hex encoded.
pub fn hash_bytes(data: &[u8]) -> String {
	let digest = Sha256::digest(data);
	hex::encode(digest)
}

/// One file in a bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleFile {
	/// The path relative to the bundle root, in `/`-separated form.
	pub path: String,
	/// The SHA-256 of the uncompressed bytes, hex encoded.
	pub hash: String,
	/// The uncompressed size.
	pub size: u64,
	/// The compressed size inside the container.
	pub compressed_size: u64,
}

/// Describes one versioned bundle of an app on a channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleManifest {
	/// The manifest format version.
	pub format_version: u32,
	/// The app name.
	pub app: String,
	/// The release channel.
	pub channel: String,
	/// The bundle's semantic version.
	pub version: semver::Version,
	/// A monotonically increasing build number within the version.
	pub build: u64,
	/// The entry wasm module path within the bundle.
	pub entry: String,
	/// The files in the bundle, in container order.
	pub files: Vec<BundleFile>,
	/// The unix timestamp of creation, in seconds.
	pub created_unix: u64,
	/// Whether the update must install without asking.
	pub mandatory: bool,
}

impl BundleManifest {
	/// The hash of the whole bundle: the hash of the manifest itself.
	pub fn bundle_hash(&self) -> String {
		let canonical = serde_json::to_vec(self).unwrap_or_default();
		hash_bytes(&canonical)
	}

	/// Looks up a file entry by path.
	pub fn file(&self, path: &str) -> Option<&BundleFile> {
		self.files.iter().find(|file| file.path == path)
	}
}

/// Packs `dir` into a container written to `output`, returning the manifest.
pub fn bundle_directory(dir: &Path, output: &Path) -> Result<BundleManifest, CodePushError> {
	let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
	for entry in WalkDir::new(dir).sort_by_file_name() {
		let entry = entry.map_err(CodePushError::Walk)?;
		if !entry.file_type().is_file() {
			continue;
		}
		let relative = entry
			.path()
			.strip_prefix(dir)
			.expect("walkdir entries are under the root")
			.to_string_lossy()
			.replace('\\', "/");
		let content = fs::read(entry.path())?;
		entries.push((relative, content));
	}
	entries.sort_by(|left, right| left.0.cmp(&right.0));

	let mut files = Vec::with_capacity(entries.len());
	let mut blobs = Vec::with_capacity(entries.len());
	for (path, content) in &entries {
		let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
		encoder.write_all(content)?;
		let compressed = encoder.finish()?;
		files.push(BundleFile {
			path: path.clone(),
			hash: hash_bytes(content),
			size: content.len() as u64,
			compressed_size: compressed.len() as u64,
		});
		blobs.push(compressed);
	}

	let entry_path = entries
		.iter()
		.find(|(path, _)| {
			Path::new(path.as_str())
				.extension()
				.is_some_and(|extension| extension.eq_ignore_ascii_case("wasm"))
		})
		.map_or_else(|| "app.wasm".to_owned(), |(path, _)| path.clone());

	let manifest = BundleManifest {
		format_version: MANIFEST_FORMAT_VERSION,
		app: dir
			.ancestors()
			.nth(1)
			.and_then(|parent| parent.file_name())
			.and_then(|name| name.to_str())
			.unwrap_or("app")
			.to_owned(),
		channel: "production".to_owned(),
		version: semver::Version::new(0, 1, 0),
		build: 1,
		entry: entry_path,
		files,
		created_unix: unix_now(),
		mandatory: false,
	};

	write_container(output, &manifest, &blobs)?;
	Ok(manifest)
}

/// Writes a container: magic, manifest length, manifest JSON, then blobs.
pub fn write_container(
	output: &Path,
	manifest: &BundleManifest,
	blobs: &[Vec<u8>],
) -> Result<(), CodePushError> {
	let mut file = fs::File::create(output)?;
	let manifest_bytes = serde_json::to_vec(manifest)?;
	file.write_all(CONTAINER_MAGIC)?;
	file.write_all(&(manifest_bytes.len() as u64).to_le_bytes())?;
	file.write_all(&manifest_bytes)?;
	for blob in blobs {
		file.write_all(blob)?;
	}
	Ok(())
}

/// A parsed container: the manifest plus each file's decompressed bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Container {
	/// The manifest.
	pub manifest: BundleManifest,
	/// The decompressed files, in manifest order.
	pub files: Vec<(String, Vec<u8>)>,
}

/// Parses a container from bytes.
pub fn read_container(bytes: &[u8]) -> Result<Container, CodePushError> {
	if bytes.len() < 12 || &bytes[0..4] != CONTAINER_MAGIC {
		return Err(CodePushError::Transport("not an LKB1 container".to_owned()));
	}
	let manifest_len = u64::from_le_bytes(
		bytes[4..12]
			.try_into()
			.expect("the length prefix is eight bytes"),
	) as usize;
	let manifest_end = 12 + manifest_len;
	if bytes.len() < manifest_end {
		return Err(CodePushError::Transport("truncated container".to_owned()));
	}
	let manifest: BundleManifest = serde_json::from_slice(&bytes[12..manifest_end])?;
	let mut offset = manifest_end;
	let mut files = Vec::with_capacity(manifest.files.len());
	for entry in &manifest.files {
		let end = offset + entry.compressed_size as usize;
		let compressed = bytes
			.get(offset..end)
			.ok_or_else(|| CodePushError::Transport("truncated container".to_owned()))?;
		let content = decompress(compressed, entry.size)?;
		let actual = hash_bytes(&content);
		if actual != entry.hash {
			return Err(CodePushError::HashMismatch {
				path: entry.path.clone(),
				expected: entry.hash.clone(),
				actual,
			});
		}
		files.push((entry.path.clone(), content));
		offset = end;
	}
	Ok(Container { manifest, files })
}

fn decompress(compressed: &[u8], expected_size: u64) -> Result<Vec<u8>, CodePushError> {
	use std::io::Read;

	let mut decoder = flate2::read::GzDecoder::new(compressed);
	let mut out = Vec::with_capacity(expected_size.min(64 * 1024 * 1024) as usize);
	decoder
		.read_to_end(&mut out)
		.map_err(|error| CodePushError::Transport(format!("corrupt gzip entry: {error}")))?;
	Ok(out)
}

fn unix_now() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|elapsed| elapsed.as_secs())
		.unwrap_or_default()
}
