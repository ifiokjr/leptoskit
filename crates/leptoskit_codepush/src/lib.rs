//! Code push for wasm app bundles: the React Native over-the-air update
//! model, applied to the bundle that runs the native interface.
//!
//! The app shell ships with a wasm bundle through the store; every later
//! change ships as a release on a channel. A release is a
//! [`BundleManifest`] describing content-hashed, gzip-compressed files packed
//! into a single `.lkb` container. Clients check in with their current
//! version and receive either an up-to-date verdict or a download plan —
//! full bundle, or a binary [`Delta`] against the version they already have,
//! which turns a typical update into a fraction of the download.

pub mod client;
pub mod delta;
pub mod manifest;
#[cfg(feature = "server")]
pub mod routes;
pub mod store;

pub use client::CodePushClient;
pub use client::FileTransport;
pub use client::SyncReport;
pub use client::Transport;
pub use client::UpdatePlan;
pub use delta::Delta;
pub use delta::DeltaOp;
pub use delta::apply_delta;
pub use delta::create_delta;
pub use manifest::BundleFile;
pub use manifest::BundleManifest;
pub use manifest::bundle_directory;
pub use manifest::hash_bytes;
pub use manifest::read_container;
#[cfg(feature = "server")]
pub use routes::codepush_router;
pub use store::ReleaseRecord;
pub use store::ReleaseStore;
pub use store::StoreError;

/// Errors raised across the code push crates.
#[derive(Debug, thiserror::Error)]
pub enum CodePushError {
	/// An IO error.
	#[error("io error: {0}")]
	Io(#[from] std::io::Error),
	/// A manifest or container failed to parse.
	#[error("serialization error: {0}")]
	Serialize(#[from] serde_json::Error),
	/// A downloaded artifact failed its content hash.
	#[error("hash mismatch for {path}: expected {expected}, got {actual}")]
	HashMismatch {
		/// The file that failed verification.
		path: String,
		/// The hash from the manifest.
		expected: String,
		/// The hash of the downloaded bytes.
		actual: String,
	},
	/// A delta applied against the wrong base.
	#[error("delta base hash mismatch: expected {expected}, got {actual}")]
	DeltaBaseMismatch {
		/// The expected base hash.
		expected: String,
		/// The actual base hash.
		actual: String,
	},
	/// A transport-level failure.
	#[error("transport error: {0}")]
	Transport(String),
	/// The requested release does not exist.
	#[error("release {0} does not exist")]
	UnknownRelease(String),
	/// The release being ingested already exists; releases are immutable.
	#[error("release {0} already exists")]
	ReleaseExists(String),
	/// A container or delta is structurally invalid: truncated, oversized, or
	/// with lengths that do not add up.
	#[error("malformed artifact: {0}")]
	Malformed(String),
	/// A container's manifest uses a format version this crate cannot read.
	#[error("unsupported manifest format version {0}")]
	UnsupportedFormatVersion(u32),
	/// A manifest file path is empty, absolute, or escapes the bundle root.
	#[error("invalid bundle file path {0:?}")]
	InvalidPath(String),
	/// A directory walk failed.
	#[error("directory walk failed: {0}")]
	Walk(#[from] walkdir::Error),
}

impl From<StoreError> for CodePushError {
	fn from(error: StoreError) -> Self {
		match error {
			StoreError::Io(error) => Self::Io(error),
			StoreError::Serialize(error) => Self::Serialize(error),
			StoreError::CodePush(error) => error,
		}
	}
}
