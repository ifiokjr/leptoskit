//! Release manifests and the `.lkb` bundle container.
//!
//! A container is a single file: the magic bytes `LKB1`, a little-endian
//! manifest length, the JSON manifest, then each file's gzip-compressed bytes
//! in manifest order, and nothing after them. One file to host, one file to
//! download, one hash to verify.

use std::fs;
use std::io::Read;
use std::path::Component;
use std::path::Path;

use flate2::Compress;
use flate2::Compression;
use flate2::Crc;
use flate2::FlushCompress;
use flate2::Status;
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

/// The fixed header: the magic bytes, then the little-endian manifest length.
const HEADER_LEN: usize = 12;

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
	let (manifest, blobs) = pack_directory(dir)?;
	write_container(output, &manifest, &blobs)?;
	Ok(manifest)
}

/// Packs a store baseline with explicit release identity, including build zero.
pub fn bundle_directory_as(
	dir: &Path,
	output: &Path,
	app: &str,
	channel: &str,
	version: semver::Version,
	build: u64,
) -> Result<BundleManifest, CodePushError> {
	let (mut manifest, blobs) = pack_directory(dir)?;
	app.clone_into(&mut manifest.app);
	channel.clone_into(&mut manifest.channel);
	manifest.version = version;
	manifest.build = build;
	write_container(output, &manifest, &blobs)?;
	Ok(manifest)
}

/// Walks and compresses `dir`, returning the manifest and each file's
/// compressed blob in manifest order, without writing anything.
pub(crate) fn pack_directory(dir: &Path) -> Result<(BundleManifest, Vec<Vec<u8>>), CodePushError> {
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
		let compressed = gzip(content)?;
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

	Ok((manifest, blobs))
}

/// The gzip header [`gzip`] writes: no file name or timestamp, maximum
/// compression, unknown operating system. It is the header `flate2`'s
/// `GzEncoder` writes at the same level.
const GZIP_HEADER: [u8; 10] = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 2, 0xff];

/// The shortest stretch of a file [`gzip`] compresses on its own.
///
/// Chunks trade full downloads against deltas: every chunk starts without a
/// dictionary, which costs compression, and every edit costs a delta the
/// whole chunk it falls in. About 64 KiB on average makes a wasm module's
/// container under 1% larger than plain gzip, and up to 4% for one with large
/// data sections, while a local edit still ships as a few tens of KiB.
const MIN_CHUNK: usize = 32 * 1024;

/// The bits of the rolling hash that must all be clear for [`gzip`] to end a
/// chunk past [`MIN_CHUNK`]: fifteen, so a chunk runs about 32 KiB further on
/// average. The top bits are used because they depend on all of the last 64
/// bytes.
const CHUNK_MASK: u64 = !(u64::MAX >> 15);

/// Per-byte values of the gear rolling hash that cuts files into chunks:
/// `splitmix64` of a fixed seed. They decide where every chunk ends, so
/// changing them makes the next release's chunks differ from the previous
/// release's, and the delta between them as large as the file.
const GEAR: [u64; 256] = {
	let mut table = [0u64; 256];
	let mut state: u64 = 0;
	let mut index = 0;
	while index < table.len() {
		state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
		let mut mixed = state;
		mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
		mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
		table[index] = mixed ^ (mixed >> 31);
		index += 1;
	}
	table
};

/// Gzips one file so that its compressed bytes resynchronise after an edit.
///
/// Plain gzip output differs in every byte after the first change to its
/// input, so a [`crate::delta`] between two plain containers has to insert
/// everything from the first change on. Here the file is cut into chunks
/// where a rolling hash of the last 64 bytes has its top bits clear, at least
/// [`MIN_CHUNK`] bytes apart, and each chunk is compressed as if it were the
/// first, with nothing remembered from the chunks before it. A cut depends
/// only on the bytes just before it, so an edit moves the cuts near it and no
/// others, and every chunk after the edit compresses to exactly the bytes it
/// did before: the delta copies them from the old container.
///
/// The result is an ordinary single-member gzip file.
fn gzip(content: &[u8]) -> Result<Vec<u8>, CodePushError> {
	let mut compress = Compress::new(Compression::best(), false);
	let mut out = Vec::with_capacity(GZIP_HEADER.len() + content.len() / 2 + 64);
	out.extend_from_slice(&GZIP_HEADER);
	let mut start = 0;
	loop {
		let end = chunk_end(content, start);
		let last = end == content.len();
		// A sync flush ends the chunk on a byte boundary without ending the
		// stream; the reset then starts the next chunk as a fresh compressor
		// would, whatever came before it.
		let flush = if last {
			FlushCompress::Finish
		} else {
			FlushCompress::Sync
		};
		deflate(&mut compress, &content[start..end], &mut out, flush)?;
		if last {
			break;
		}
		compress.reset();
		start = end;
	}
	let mut crc = Crc::new();
	crc.update(content);
	out.extend_from_slice(&crc.sum().to_le_bytes());
	// ISIZE is the input length modulo 2^32.
	out.extend_from_slice(&(content.len() as u32).to_le_bytes());
	Ok(out)
}

/// Where the chunk of `content` that starts at `start` ends.
fn chunk_end(content: &[u8], start: usize) -> usize {
	let mut hash = 0u64;
	for (index, &byte) in content.iter().enumerate().skip(start) {
		hash = (hash << 1).wrapping_add(GEAR[usize::from(byte)]);
		if index + 1 - start >= MIN_CHUNK && hash & CHUNK_MASK == 0 {
			return index + 1;
		}
	}
	content.len()
}

/// Deflates all of `input` onto `out`, then flushes as `flush` asks.
fn deflate(
	compress: &mut Compress,
	mut input: &[u8],
	out: &mut Vec<u8>,
	flush: FlushCompress,
) -> Result<(), CodePushError> {
	loop {
		// More than deflate's worst case for the input, so a call normally
		// finishes in one go.
		out.reserve(input.len() + input.len() / 8 + 64);
		let read_before = compress.total_in();
		let status = compress
			.compress_vec(input, out, flush)
			.map_err(|error| CodePushError::Io(std::io::Error::other(error)))?;
		let read = usize::try_from(compress.total_in() - read_before)
			.expect("deflate reads at most its input");
		input = &input[read..];
		// A flush is complete once a call leaves output space unused.
		let done = match flush {
			FlushCompress::Finish => status == Status::StreamEnd,
			_ => input.is_empty() && out.len() < out.capacity(),
		};
		if done {
			return Ok(());
		}
	}
}

/// Serializes a container to bytes: magic, manifest length, manifest JSON,
/// then blobs. This is the single definition of the on-wire layout; the file
/// writer and the store's in-memory records both come from it, so a release's
/// bytes are identical everywhere they exist.
pub fn container_bytes(
	manifest: &BundleManifest,
	blobs: &[Vec<u8>],
) -> Result<Vec<u8>, CodePushError> {
	let manifest_bytes = serde_json::to_vec(manifest)?;
	let mut out = Vec::with_capacity(HEADER_LEN + manifest_bytes.len());
	out.extend_from_slice(CONTAINER_MAGIC);
	out.extend_from_slice(&(manifest_bytes.len() as u64).to_le_bytes());
	out.extend_from_slice(&manifest_bytes);
	for blob in blobs {
		out.extend_from_slice(blob);
	}
	Ok(out)
}

/// Writes a container: magic, manifest length, manifest JSON, then blobs.
pub fn write_container(
	output: &Path,
	manifest: &BundleManifest,
	blobs: &[Vec<u8>],
) -> Result<(), CodePushError> {
	fs::write(output, container_bytes(manifest, blobs)?)?;
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
///
/// Every length in the header and manifest is bounds-checked, the manifest
/// format version must be [`MANIFEST_FORMAT_VERSION`], every file path must be
/// a relative path made only of normal components (so staging can never
/// write outside its directory), every file must be exactly one gzip stream
/// that decompresses to its declared size and hash, and nothing may follow
/// the last file.
pub fn read_container(bytes: &[u8]) -> Result<Container, CodePushError> {
	let (manifest, manifest_end) = container_manifest(bytes)?;
	let mut offset = manifest_end;
	let mut files = Vec::with_capacity(manifest.files.len());
	for entry in &manifest.files {
		let compressed = usize::try_from(entry.compressed_size)
			.ok()
			.and_then(|size| offset.checked_add(size))
			.and_then(|end| bytes.get(offset..end))
			.ok_or_else(|| {
				CodePushError::Malformed(format!("truncated container entry {}", entry.path))
			})?;
		let content = decompress(compressed, entry)?;
		let actual = hash_bytes(&content);
		if actual != entry.hash {
			return Err(CodePushError::HashMismatch {
				path: entry.path.clone(),
				expected: entry.hash.clone(),
				actual,
			});
		}
		offset += compressed.len();
		files.push((entry.path.clone(), content));
	}
	if offset != bytes.len() {
		return Err(CodePushError::Malformed(format!(
			"{} bytes follow the last container entry",
			bytes.len() - offset
		)));
	}
	Ok(Container { manifest, files })
}

/// Parses metadata without allocating decompressed file contents, allowing
/// clients to authenticate expansion lengths before trusting them.
pub(crate) fn container_manifest(bytes: &[u8]) -> Result<(BundleManifest, usize), CodePushError> {
	if bytes.len() < HEADER_LEN || &bytes[0..4] != CONTAINER_MAGIC {
		return Err(CodePushError::Malformed("not an LKB1 container".to_owned()));
	}
	let manifest_len = u64::from_le_bytes(
		bytes[4..HEADER_LEN]
			.try_into()
			.expect("the length prefix is eight bytes"),
	);
	let manifest_end = usize::try_from(manifest_len)
		.ok()
		.and_then(|len| HEADER_LEN.checked_add(len))
		.filter(|&end| end <= bytes.len())
		.ok_or_else(|| CodePushError::Malformed("truncated container manifest".to_owned()))?;
	let manifest: BundleManifest = serde_json::from_slice(&bytes[HEADER_LEN..manifest_end])?;
	if manifest.format_version != MANIFEST_FORMAT_VERSION {
		return Err(CodePushError::UnsupportedFormatVersion(
			manifest.format_version,
		));
	}
	for entry in &manifest.files {
		validate_bundle_path(&entry.path)?;
	}

	Ok((manifest, manifest_end))
}

/// Checks that a manifest file path is safe to join onto a staging directory:
/// non-empty, relative, and made only of normal components — no `..`, `.`,
/// root, or drive prefix.
fn validate_bundle_path(path: &str) -> Result<(), CodePushError> {
	let parsed = Path::new(path);
	let only_normal = parsed
		.components()
		.all(|component| matches!(component, Component::Normal(_)));
	if path.is_empty() || parsed.is_absolute() || !only_normal {
		return Err(CodePushError::InvalidPath(path.to_owned()));
	}
	Ok(())
}

/// Inflates one entry, reading at most one byte past its declared size so a
/// gzip bomb cannot grow without bound, then requires the exact size and
/// that the gzip stream used every byte of the entry.
fn decompress(compressed: &[u8], entry: &BundleFile) -> Result<Vec<u8>, CodePushError> {
	let mut out = Vec::with_capacity(entry.size.min(64 * 1024 * 1024) as usize);
	let mut limited =
		flate2::bufread::GzDecoder::new(compressed).take(entry.size.saturating_add(1));
	limited
		.read_to_end(&mut out)
		.map_err(|error| CodePushError::Malformed(format!("corrupt gzip entry: {error}")))?;
	let unread = limited.into_inner().into_inner();
	if out.len() as u64 != entry.size {
		return Err(CodePushError::Malformed(format!(
			"{} declares {} bytes but inflates to {}{}",
			entry.path,
			entry.size,
			out.len(),
			if out.len() as u64 > entry.size {
				" or more"
			} else {
				""
			},
		)));
	}
	if !unread.is_empty() {
		return Err(CodePushError::Malformed(format!(
			"{} bytes follow the gzip stream of {}",
			unread.len(),
			entry.path
		)));
	}
	Ok(out)
}

fn unix_now() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|elapsed| elapsed.as_secs())
		.unwrap_or_default()
}

#[cfg(test)]
mod tests {
	use std::io::Write;

	use flate2::write::GzEncoder;

	use super::*;

	fn gzip(content: &[u8]) -> Vec<u8> {
		let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
		encoder.write_all(content).expect("gzip in memory");
		encoder.finish().expect("gzip in memory")
	}

	fn manifest(files: Vec<BundleFile>) -> BundleManifest {
		BundleManifest {
			format_version: MANIFEST_FORMAT_VERSION,
			app: "app".to_owned(),
			channel: "production".to_owned(),
			version: semver::Version::new(1, 0, 0),
			build: 1,
			entry: "app.wasm".to_owned(),
			files,
			created_unix: 0,
			mandatory: false,
		}
	}

	/// A single-file container whose entry declares `size` bytes.
	fn container(path: &str, content: &[u8], size: u64) -> (BundleManifest, Vec<u8>) {
		let blob = gzip(content);
		let manifest = manifest(vec![BundleFile {
			path: path.to_owned(),
			hash: hash_bytes(content),
			size,
			compressed_size: blob.len() as u64,
		}]);
		let bytes = container_bytes(&manifest, &[blob]).expect("container");
		(manifest, bytes)
	}

	#[test]
	fn accepts_nested_relative_paths() {
		let (_, bytes) = container("assets/logo.png", b"logo", 4);
		let parsed = read_container(&bytes).expect("valid container");
		assert_eq!(
			parsed.files,
			vec![("assets/logo.png".to_owned(), b"logo".to_vec())]
		);
	}

	#[test]
	fn rejects_paths_that_escape_the_bundle_root() {
		for path in [
			"",
			"../escape.txt",
			"assets/../../escape.txt",
			"/etc/passwd",
			"./app.wasm",
		] {
			let (_, bytes) = container(path, b"payload", 7);
			let result = read_container(&bytes);
			assert!(
				matches!(&result, Err(CodePushError::InvalidPath(rejected)) if rejected == path),
				"{path:?} must be rejected, got {result:?}"
			);
		}
	}

	#[test]
	fn rejects_entries_that_inflate_past_their_declared_size() {
		let (_, bytes) = container("app.wasm", &[0u8; 4096], 16);
		let result = read_container(&bytes);
		assert!(
			matches!(result, Err(CodePushError::Malformed(_))),
			"got {result:?}"
		);
	}

	#[test]
	fn rejects_entries_shorter_than_their_declared_size() {
		let (_, bytes) = container("app.wasm", b"short", 4096);
		let result = read_container(&bytes);
		assert!(
			matches!(result, Err(CodePushError::Malformed(_))),
			"got {result:?}"
		);
	}

	#[test]
	fn rejects_a_manifest_length_that_overflows() {
		let mut bytes = CONTAINER_MAGIC.to_vec();
		bytes.extend_from_slice(&u64::MAX.to_le_bytes());
		let result = read_container(&bytes);
		assert!(
			matches!(result, Err(CodePushError::Malformed(_))),
			"got {result:?}"
		);
	}

	#[test]
	fn rejects_a_compressed_size_that_overflows() {
		let manifest = manifest(vec![BundleFile {
			path: "app.wasm".to_owned(),
			hash: hash_bytes(b""),
			size: 0,
			compressed_size: u64::MAX,
		}]);
		let bytes = container_bytes(&manifest, &[]).expect("container");
		let result = read_container(&bytes);
		assert!(
			matches!(result, Err(CodePushError::Malformed(_))),
			"got {result:?}"
		);
	}

	#[test]
	fn rejects_unknown_format_versions() {
		let (mut manifest, _) = container("app.wasm", b"wasm", 4);
		manifest.format_version = MANIFEST_FORMAT_VERSION + 1;
		let blob = gzip(b"wasm");
		let bytes = container_bytes(&manifest, &[blob]).expect("container");
		let result = read_container(&bytes);
		assert!(
			matches!(result, Err(CodePushError::UnsupportedFormatVersion(version)) if version == MANIFEST_FORMAT_VERSION + 1),
			"got {result:?}"
		);
	}

	#[test]
	fn rejects_bytes_after_the_last_entry() {
		let (_, mut bytes) = container("app.wasm", b"wasm", 4);
		read_container(&bytes).expect("the container itself is valid");
		bytes.extend_from_slice(b"appended");
		let result = read_container(&bytes);
		assert!(
			matches!(&result, Err(CodePushError::Malformed(message)) if message.contains("follow the last")),
			"got {result:?}"
		);
	}

	#[test]
	fn rejects_data_after_an_entrys_gzip_stream() {
		let mut blob = gzip(b"wasm");
		blob.extend_from_slice(b"smuggled");
		let manifest = manifest(vec![BundleFile {
			path: "app.wasm".to_owned(),
			hash: hash_bytes(b"wasm"),
			size: 4,
			compressed_size: blob.len() as u64,
		}]);
		let bytes = container_bytes(&manifest, &[blob]).expect("container");
		let result = read_container(&bytes);
		assert!(
			matches!(&result, Err(CodePushError::Malformed(message)) if message.contains("follow the gzip stream")),
			"got {result:?}"
		);
	}

	/// Deterministic bytes that compress, but without long repeats: words of
	/// a small random vocabulary, like the identifiers and instruction runs
	/// of a wasm module.
	fn varied(len: usize, seed: u64) -> Vec<u8> {
		let mut state = seed;
		let mut next = || {
			state ^= state << 13;
			state ^= state >> 7;
			state ^= state << 17;
			state
		};
		let mut vocabulary = Vec::new();
		for _ in 0..256 {
			let word_len = next() % 12 + 1;
			let mut word = Vec::new();
			for _ in 0..word_len {
				word.push(next() as u8);
			}
			vocabulary.push(word);
		}
		let mut out = Vec::with_capacity(len + 16);
		while out.len() < len {
			out.extend_from_slice(&vocabulary[(next() % 256) as usize]);
		}
		out.truncate(len);
		out
	}

	#[test]
	fn compressed_files_are_standard_gzip() {
		for content in [Vec::new(), b"wasm".to_vec(), varied(300 * 1024, 7)] {
			let compressed = super::gzip(&content).expect("gzip");
			let mut decoded = Vec::new();
			flate2::read::GzDecoder::new(compressed.as_slice())
				.read_to_end(&mut decoded)
				.expect("a standard decoder reads it");
			assert_eq!(decoded, content);
		}
	}

	#[test]
	fn compressed_files_resynchronise_after_an_edit() {
		let original = varied(2 * 1024 * 1024, 11);
		let mut edited = original.clone();
		edited.splice(
			800 * 1024..800 * 1024 + 40,
			b"a different function body".to_vec(),
		);
		let (before, after) = (
			super::gzip(&original).expect("gzip"),
			super::gzip(&edited).expect("gzip"),
		);
		// The CRC and length trailer always differs.
		let (before, after) = (&before[..before.len() - 8], &after[..after.len() - 8]);
		let prefix = before
			.iter()
			.zip(after)
			.take_while(|(left, right)| left == right)
			.count();
		let suffix = before
			.iter()
			.rev()
			.zip(after.iter().rev())
			.take_while(|(left, right)| left == right)
			.count();
		assert!(
			prefix + suffix > before.len() * 9 / 10,
			"only {prefix} + {suffix} of {} compressed bytes survive the edit",
			before.len()
		);
	}
}
