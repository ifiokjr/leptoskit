//! The content-addressed build cache for the phases cargo does not cache.
//!
//! Cargo is incremental on its own and always runs. The phases after it —
//! wasm-bindgen and, in release, `wasm-opt` — fingerprint their inputs: the
//! content hash of the compiled wasm, the flags and output location that
//! shape the result, and the tool version that runs them. When the
//! fingerprint is unchanged (and the outputs still exist), the phase is
//! skipped and its output reused. Each phase's digest is its own file under
//! `<target dir>/leptoskit/cache/` (see [`BuildCache`]).

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

/// The persisted phase digests, one file per phase under
/// `<target_dir>/leptoskit/cache/`.
///
/// A target directory can be shared by projects, output directories, and
/// builds running at the same time (`leptoskit dev` beside a release build).
/// With a file per phase, written whole and renamed into place, no build
/// ever rewrites another's entry: a shared single file would lose entries
/// to concurrent read-modify-write cycles and rerun their phases for
/// nothing.
#[derive(Debug, Clone, Default)]
pub struct BuildCache {
	dir: Option<PathBuf>,
	recorded: BTreeMap<String, String>,
}

impl BuildCache {
	/// The cache under `target_dir`, cargo's target directory, so the cache
	/// lives and dies with the artifacts it describes. Nothing is read until
	/// a phase is checked.
	pub fn load(target_dir: &Path) -> Self {
		Self {
			dir: Some(target_dir.join("leptoskit").join("cache")),
			recorded: BTreeMap::new(),
		}
	}

	/// The file holding `phase`'s digest.
	fn entry(&self, phase: &str) -> Option<PathBuf> {
		let key = hash_bytes(phase.as_bytes());
		self.dir.as_ref().map(|dir| dir.join(&key[..32]))
	}

	/// Whether the phase's fingerprint matches the cached digest, meaning its
	/// output is already current.
	#[must_use]
	pub fn is_current(&self, fingerprint: &Fingerprint) -> bool {
		let digest = fingerprint.digest();
		if let Some(recorded) = self.recorded.get(&fingerprint.phase) {
			return *recorded == digest;
		}
		self.entry(&fingerprint.phase)
			.and_then(|path| std::fs::read_to_string(path).ok())
			.is_some_and(|cached| cached == digest)
	}

	/// Records a phase as run with this fingerprint.
	pub fn record(&mut self, fingerprint: &Fingerprint) {
		self.recorded
			.insert(fingerprint.phase.clone(), fingerprint.digest());
	}

	/// Persists the recorded phases, each to its own file: written beside it
	/// and renamed into place, so a reader never sees half an entry.
	///
	/// # Errors
	///
	/// Returns an error when an entry cannot be written.
	pub fn save(&self) -> std::io::Result<()> {
		let Some(dir) = &self.dir else {
			return Ok(());
		};
		std::fs::create_dir_all(dir)?;
		for (phase, digest) in &self.recorded {
			let Some(path) = self.entry(phase) else {
				continue;
			};
			let partial = path.with_extension(format!("{}.partial", std::process::id()));
			std::fs::write(&partial, digest)?;
			std::fs::rename(&partial, &path)?;
		}
		Ok(())
	}
}
