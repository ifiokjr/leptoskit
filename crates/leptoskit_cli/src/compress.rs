//! Precompression and the bundle size report.
//!
//! Release builds write a brotli (`.br`) and a gzip (`.gz`) copy next to
//! every compressible file in the output, so servers send compressed bytes
//! without compressing on every request; `leptoskit_axum` picks the variant
//! from `Accept-Encoding`. Brotli at its highest quality is too slow to run
//! per request but fine once per release, and it shrinks wasm well beyond
//! gzip.
//!
//! The variants are leptoskit's: every build that does not precompress
//! removes the `.br` and `.gz` siblings of compressible files, so a server
//! never sends a compressed copy of an older build.
//!
//! Like the phases after cargo, precompression only redoes work whose input
//! changed: a [`PrecompressCache`] remembers each file's content hash and
//! compressed sizes, and a file whose content is unchanged, with its
//! variants still on disk at the recorded sizes, is not compressed again.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use rayon::prelude::*;
use serde::Deserialize;
use serde::Serialize;

use crate::cache::hash_bytes;

/// The brotli quality for precompression: the maximum, since it runs once
/// per release rather than per request.
const BROTLI_QUALITY: u32 = 11;
/// The brotli window, in bits: the format's largest standard window.
const BROTLI_WINDOW: u32 = 22;

/// The encodings leptoskit precompresses into, by file suffix.
const SUFFIXES: [&str; 2] = ["br", "gz"];

/// Whether a file is worth compressing: text formats and wasm. Images,
/// fonts, and archives are already compressed.
#[must_use]
pub fn is_compressible(path: &Path) -> bool {
	matches!(
		path.extension().and_then(|extension| extension.to_str()),
		Some(
			"wasm"
				| "js"
				| "mjs"
				| "css"
				| "html"
				| "svg"
				| "json"
				| "map"
				| "txt"
				| "xml"
				| "webmanifest"
		)
	)
}

/// The bytes a server sends for a file, raw and per encoding. A compressed
/// size is the file's own size when compressing would not shrink it: no
/// variant is written then, so the server sends the file as it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sizes {
	/// The file as it is.
	pub raw: u64,
	/// Gzip at its best compression.
	pub gzip: u64,
	/// Brotli at quality 11.
	pub brotli: u64,
}

impl Sizes {
	fn served(raw: &[u8], brotli: &[u8], gzip: &[u8]) -> Self {
		let raw = raw.len() as u64;
		Self {
			raw,
			gzip: raw.min(gzip.len() as u64),
			brotli: raw.min(brotli.len() as u64),
		}
	}
}

impl std::ops::Add for Sizes {
	type Output = Self;

	fn add(self, other: Self) -> Self {
		Self {
			raw: self.raw + other.raw,
			gzip: self.gzip + other.gzip,
			brotli: self.brotli + other.brotli,
		}
	}
}

/// `bytes` compressed with brotli at the precompression settings.
pub fn brotli(bytes: &[u8]) -> Result<Vec<u8>> {
	let mut output = Vec::new();
	{
		let mut writer =
			brotli::CompressorWriter::new(&mut output, 4096, BROTLI_QUALITY, BROTLI_WINDOW);
		writer
			.write_all(bytes)
			.context("brotli compression failed")?;
	}
	Ok(output)
}

/// `bytes` compressed with gzip at its best level.
pub fn gzip(bytes: &[u8]) -> Result<Vec<u8>> {
	let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
	encoder
		.write_all(bytes)
		.context("gzip compression failed")?;
	encoder.finish().context("gzip compression failed")
}

/// Measures a file raw and compressed, without writing anything.
pub fn measure(path: &Path) -> Result<Sizes> {
	let bytes =
		std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
	Ok(Sizes::served(&bytes, &brotli(&bytes)?, &gzip(&bytes)?))
}

/// What precompression last wrote into one output directory, by file: the
/// content hash it compressed and the sizes it measured. It lives beside the
/// build cache in cargo's target directory, one file per output directory,
/// so outputs and projects that share a target directory keep their own.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrecompressCache {
	#[serde(skip)]
	file: Option<PathBuf>,
	entries: BTreeMap<PathBuf, Compressed>,
}

/// One file's precompression: the hash of the bytes compressed, and sizes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Compressed {
	hash: String,
	sizes: Sizes,
}

impl PrecompressCache {
	/// Loads `out_dir`'s cache from
	/// `<target_dir>/leptoskit/precompress-<key>.json`, or starts empty: a
	/// lost cache only costs one round of compression.
	pub fn load(target_dir: &Path, out_dir: &Path) -> Self {
		let key = hash_bytes(out_dir.as_os_str().as_encoded_bytes());
		let path = target_dir
			.join("leptoskit")
			.join(format!("precompress-{}.json", &key[..16]));
		let entries = std::fs::read(&path)
			.ok()
			.and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
			.map(|cache| cache.entries)
			.unwrap_or_default();
		Self {
			file: Some(path),
			entries,
		}
	}

	/// Persists the cache.
	///
	/// # Errors
	///
	/// Returns an error when the cache file cannot be written.
	pub fn save(&self) -> Result<()> {
		let Some(path) = &self.file else {
			return Ok(());
		};

		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)
				.with_context(|| format!("could not create {}", parent.display()))?;
		}

		let json = serde_json::to_vec_pretty(self).context("could not encode the cache")?;
		std::fs::write(path, json).with_context(|| format!("could not write {}", path.display()))
	}

	/// The recorded compression of `path`, when its content hash is still
	/// `hash` and its variants are on disk exactly as recorded.
	fn current(&self, path: &Path, hash: &str) -> Option<Sizes> {
		let compressed = self.entries.get(path)?;
		let unchanged = compressed.hash == hash
			&& SUFFIXES
				.into_iter()
				.zip([compressed.sizes.brotli, compressed.sizes.gzip])
				.all(|(suffix, size)| {
					let variant = std::fs::metadata(with_suffix(path, suffix)).ok();
					if size < compressed.sizes.raw {
						variant.is_some_and(|variant| variant.len() == size)
					} else {
						variant.is_none()
					}
				});
		unchanged.then_some(compressed.sizes)
	}
}

/// Writes `.br` and `.gz` next to every compressible file under `dir`, in
/// parallel, keeping a variant only when it is smaller than the file. Files
/// `cache` finds unchanged are skipped, and `cache` records the rest.
///
/// Returns every compressed file's [`Sizes`], so the bundle report reuses
/// them instead of compressing the same bytes again.
pub fn precompress(dir: &Path, cache: &mut PrecompressCache) -> Result<BTreeMap<PathBuf, Sizes>> {
	let files = files(dir);
	remove_stale(&files, true)?;
	let compressed: Vec<(PathBuf, Compressed)> = files
		.into_par_iter()
		.filter(|path| is_compressible(path))
		.map(|path| {
			let bytes = std::fs::read(&path)
				.with_context(|| format!("could not read {}", path.display()))?;
			let hash = hash_bytes(&bytes);
			if let Some(sizes) = cache.current(&path, &hash) {
				return Ok((path, Compressed { hash, sizes }));
			}

			let (brotli, gzip) = (brotli(&bytes)?, gzip(&bytes)?);
			let sizes = Sizes::served(&bytes, &brotli, &gzip);
			for (suffix, compressed) in SUFFIXES.into_iter().zip([brotli, gzip]) {
				let variant = with_suffix(&path, suffix);
				if compressed.len() < bytes.len() {
					std::fs::write(&variant, compressed)
						.with_context(|| format!("could not write {}", variant.display()))?;
				} else {
					remove_if_present(&variant)?;
				}
			}

			Ok((path, Compressed { hash, sizes }))
		})
		.collect::<Result<_>>()?;
	cache.entries.clear();
	let mut sizes = BTreeMap::new();

	for (path, entry) in compressed {
		sizes.insert(path.clone(), entry.sizes);
		cache.entries.insert(path, entry);
	}

	Ok(sizes)
}

/// Removes every precompressed variant leptoskit could have written under
/// `dir`: a `.br` or `.gz` beside a compressible file, or one whose file is
/// gone.
pub fn remove_precompressed(dir: &Path) -> Result<()> {
	remove_stale(&files(dir), false)
}

/// Removes, among `files`, variants whose source is gone, and, unless
/// `keep_current`, the variants of sources that still exist too.
fn remove_stale(files: &[PathBuf], keep_current: bool) -> Result<()> {
	for path in files {
		let Some(source) = variant_source(path) else {
			continue;
		};

		if !is_compressible(&source) {
			continue;
		}

		if !keep_current || !source.is_file() {
			remove_if_present(path)?;
		}
	}

	Ok(())
}

/// The file a `.br` or `.gz` variant compresses, or `None` for any other
/// file.
fn variant_source(path: &Path) -> Option<PathBuf> {
	let extension = path.extension()?.to_str()?;
	SUFFIXES
		.contains(&extension)
		.then(|| path.with_extension(""))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
	let mut name = path.as_os_str().to_owned();
	name.push(".");
	name.push(suffix);
	PathBuf::from(name)
}

fn remove_if_present(path: &Path) -> Result<()> {
	match std::fs::remove_file(path) {
		Ok(()) => Ok(()),

		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(error) => Err(error).with_context(|| format!("could not remove {}", path.display())),
	}
}

fn files(dir: &Path) -> Vec<PathBuf> {
	walkdir::WalkDir::new(dir)
		.into_iter()
		.filter_map(Result::ok)
		.filter(|entry| entry.file_type().is_file())
		.map(walkdir::DirEntry::into_path)
		.collect()
}
