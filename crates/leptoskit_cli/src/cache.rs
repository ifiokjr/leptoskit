//! The content-addressed build cache: the reason rebuilds are fast.
//!
//! Every phase of the pipeline — cargo builds, wasm-bindgen runs, wasm
//! optimization, asset copies — fingerprints its inputs: the content hash of
//! what went in plus the tool version that ran it. When the fingerprint is
//! unchanged, the phase is skipped and its output reused. That is what makes
//! `leptoskit build` after a one-line change feel instant: the unchanged
//! halves of the graph never run at all.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

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

	/// Adds an input's content hash, reading the file when its mtime moved.
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

/// A file's fingerprint: content hash when the mtime is newer than the last
/// cached hash, otherwise the cached hash — the stat-mostly, hash-rarely
/// trick that keeps large asset trees cheap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFingerprint {
	/// The content hash.
	pub hash: String,
	/// The mtime at which the hash was taken.
	pub modified_unix: u64,
	/// The file size at that time.
	pub size: u64,
}

/// Hashes a file, or returns a zero marker when it is missing.
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
	/// Loads the cache from `target/leptoskit/cache.json`, or starts empty.
	pub fn load(project_root: &Path) -> Self {
		let path = project_root
			.join("target")
			.join("leptoskit")
			.join("cache.json");
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

/// The freshest mtime under a directory, for watch-driven rebuilds.
pub fn newest_mtime(dir: &Path) -> Option<SystemTime> {
	walkdir::WalkDir::new(dir)
		.into_iter()
		.filter_map(Result::ok)
		.filter(|entry| entry.file_type().is_file())
		.filter_map(|entry| entry.metadata().ok())
		.filter_map(|metadata| metadata.modified().ok())
		.max()
}
