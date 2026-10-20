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

use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use rayon::prelude::*;

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

/// A file's size raw and compressed each way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sizes {
	/// The file as it is.
	pub raw: u64,
	/// Gzip at its best compression.
	pub gzip: u64,
	/// Brotli at quality 11.
	pub brotli: u64,
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
	Ok(Sizes {
		raw: bytes.len() as u64,
		gzip: gzip(&bytes)?.len() as u64,
		brotli: brotli(&bytes)?.len() as u64,
	})
}

/// Writes `.br` and `.gz` next to every compressible file under `dir`, in
/// parallel, keeping a variant only when it is smaller than the file.
pub fn precompress(dir: &Path) -> Result<()> {
	remove_stale(dir, true)?;
	files(dir)
		.into_par_iter()
		.filter(|path| is_compressible(path))
		.try_for_each(|path| {
			let bytes = std::fs::read(&path)
				.with_context(|| format!("could not read {}", path.display()))?;
			for (suffix, compressed) in SUFFIXES.into_iter().zip([brotli(&bytes)?, gzip(&bytes)?]) {
				let variant = with_suffix(&path, suffix);
				if compressed.len() < bytes.len() {
					std::fs::write(&variant, compressed)
						.with_context(|| format!("could not write {}", variant.display()))?;
				} else {
					remove_if_present(&variant)?;
				}
			}
			Ok(())
		})
}

/// Removes every precompressed variant leptoskit could have written under
/// `dir`: a `.br` or `.gz` beside a compressible file, or one whose file is
/// gone.
pub fn remove_precompressed(dir: &Path) -> Result<()> {
	remove_stale(dir, false)
}

/// Removes variants whose source is gone, and, unless `keep_current`, the
/// variants of sources that still exist too.
fn remove_stale(dir: &Path, keep_current: bool) -> Result<()> {
	for path in files(dir) {
		let Some(source) = variant_source(&path) else {
			continue;
		};
		if !is_compressible(&source) {
			continue;
		}
		if !keep_current || !source.is_file() {
			remove_if_present(&path)?;
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
