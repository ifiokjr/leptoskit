//! The content-addressed build cache for the phases cargo does not cache.
//!
//! Cargo is incremental on its own and always runs. The phases after it —
//! wasm-bindgen and, in release, `wasm-opt` — fingerprint their inputs: the
//! content hash of the compiled wasm, the flags and output location that
//! shape the result, and the tool version that runs them. When the
//! fingerprint is unchanged (and the outputs still exist), the phase is
//! skipped and its output reused.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;

/// A phase fingerprint: everything that determines the phase's output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
	/// The phase name.
	pub phase: String,
	/// The tool version that runs the phase.
	pub tool: String,
	/// Hashed inputs, keyed by a stable name.
	pub inputs: BTreeMap<String, String>,
}

impl Fingerprint {
	/// Builds a fingerprint from a phase name, tool version, and input hashes.
	pub fn new(phase: impl Into<String>, tool: impl Into<String>) -> Self {
		Self {
			phase: phase.into(),
			tool: tool.into(),
			inputs: BTreeMap::new(),
		}
	}

	/// Adds an input hash.
	#[must_use]
	pub fn with_input(mut self, name: impl Into<String>, hash: impl Into<String>) -> Self {
		self.inputs.insert(name.into(), hash.into());
		self
	}

	/// Adds a file's content hash as an input; a missing file hashes as the
	/// marker `missing`. The file is read in full on every call.
	#[must_use]
	pub fn with_file(self, name: impl Into<String>, path: &Path) -> Self {
		self.with_input(name, file_fingerprint(path))
	}

	/// The digest of the whole fingerprint.
	pub fn digest(&self) -> String {
		let canonical = serde_json::to_vec(self).unwrap_or_default();
		hash_bytes(&canonical)
	}
}

/// The SHA-256 of `bytes`, hex encoded.
pub fn hash_bytes(bytes: &[u8]) -> String {
	hex::encode(Sha256::digest(bytes))
}

/// Hashes a file, or returns the marker `missing` when it cannot be read.
pub fn file_fingerprint(path: &Path) -> String {
	match std::fs::read(path) {
		Ok(bytes) => hash_bytes(&bytes),
		Err(_) => "missing".to_owned(),
	}
}

/// A persisted set of phase digests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildCache {
	#[serde(skip)]
	file: Option<PathBuf>,
	entries: BTreeMap<String, String>,
}

impl BuildCache {
	/// Loads the cache from `<target_dir>/leptoskit/cache.json`, or starts
	/// empty. `target_dir` is cargo's target directory, so the cache lives
	/// and dies with the artifacts it describes.
	pub fn load(target_dir: &Path) -> Self {
		let path = target_dir.join("leptoskit").join("cache.json");
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

	/// Whether the phase's fingerprint matches the cached digest, meaning its
	/// output is already current.
	#[must_use]
	pub fn is_current(&self, fingerprint: &Fingerprint) -> bool {
		self.entries
			.get(&fingerprint.phase)
			.is_some_and(|cached| *cached == fingerprint.digest())
	}

	/// Records a phase as run with this fingerprint.
	pub fn record(&mut self, fingerprint: &Fingerprint) {
		self.entries
			.insert(fingerprint.phase.clone(), fingerprint.digest());
	}

	/// Persists the cache to disk.
	///
	/// # Errors
	///
	/// Returns an error when the cache file cannot be written.
	pub fn save(&self) -> std::io::Result<()> {
		let Some(path) = &self.file else {
			return Ok(());
		};
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)?;
		}
		std::fs::write(path, serde_json::to_vec_pretty(self).unwrap_or_default())
	}
}
