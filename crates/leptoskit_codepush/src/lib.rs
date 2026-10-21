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
//!
//! Releases live in a static [`layout`]: immutable, content-addressed files
//! plus one `index.json` per channel naming the newest release. Any static
//! file server, object store, or CDN can host a channel, and
//! [`publish::publish_channel`] uploads one to a [`publish::Bucket`] with the
//! index written last; [`publish::pull_channel`] seeds a store from one.
//! `leptoskit codepush serve` serves the same files.
//!
//! Every release is signed by its publisher, and a client stages only
//! releases signed by a key in its [`TrustedKeys`] that are newer than what it
//! runs, so a compromised registry can neither ship its own code nor roll a
//! device back to an older release.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

pub mod client;
pub mod delta;
pub mod installed;
pub mod layout;
pub mod manifest;
pub mod publish;
#[cfg(feature = "server")]
pub mod routes;
pub mod signing;
pub mod store;

pub use client::CodePushClient;
pub use client::FileTransport;
pub use client::SyncReport;
pub use client::Transport;
pub use client::UpdateOffer;
pub use client::UpdatePlan;
pub use delta::Delta;
pub use delta::DeltaOp;
pub use delta::apply_delta;
pub use delta::create_delta;
pub use layout::ChannelIndex;
pub use layout::ObjectKey;
pub use layout::ObjectKind;
pub use layout::StaticObject;
pub use manifest::BundleFile;
pub use manifest::BundleManifest;
pub use manifest::bundle_directory;
pub use manifest::bundle_directory_as;
pub use manifest::hash_bytes;
pub use manifest::read_container;
pub use publish::Bucket;
pub use publish::PublishReport;
pub use publish::PullReport;
pub use publish::publish_channel;
pub use publish::pull_channel;
#[cfg(feature = "server")]
pub use routes::codepush_router;
pub use signing::ReleasePublicKey;
pub use signing::ReleaseSignature;
pub use signing::ReleaseSigningKey;
pub use signing::TrustedKeys;
pub use store::IngestedRelease;
pub use store::ReleaseRecord;
pub use store::ReleaseStore;
pub use store::StoreError;

/// Errors raised across the code push crates.
#[derive(Debug, thiserror::Error)]
pub enum CodePushError {
	/// Native release configuration, compatibility, or activation failed.
	#[error("native update error: {0}")]
	Activation(String),
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
	/// The registry has no file at the requested location.
	#[error("{0} was not found")]
	NotFound(String),
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
	/// A signing or public key is malformed, or a trust set is empty.
	#[error("invalid key: {0}")]
	InvalidKey(String),
	/// A release carries no signature.
	#[error("release {0} is not signed")]
	Unsigned(String),
	/// A release is signed by a key the client does not trust.
	#[error("release is signed by an untrusted key {0}")]
	UntrustedKey(String),
	/// A release signature does not verify against its manifest.
	#[error("release {0} has an invalid signature")]
	BadSignature(String),
	/// An offer is for a different app or channel than the client tracks.
	#[error("offer is for {offered}, but the client tracks {tracked}")]
	WrongTarget {
		/// The `app/channel` the client tracks.
		tracked: String,
		/// The `app/channel` the offer names.
		offered: String,
	},
	/// An offer is not newer than the release the client runs: a downgrade or
	/// a replay of an old release.
	#[error("offered release {offered} is not newer than the running {current}")]
	NotNewer {
		/// The `version+build` the client runs.
		current: String,
		/// The `version+build` offered.
		offered: String,
	},
	/// A channel being published has no releases in the store.
	#[error("channel {0} has no releases to publish")]
	NoReleases(String),
	/// The registry already serves a newer release than the store holds, so
	/// publishing would roll the channel back.
	#[error(
		"the registry's newest release {registry} is newer than the store's {store}; publish from \
		 the store that released it"
	)]
	RegistryAhead {
		/// The `version+build` the registry's index names.
		registry: String,
		/// The newest `version+build` in the store.
		store: String,
	},
	/// A release or release file differs between the store and the registry:
	/// another store published the same version and build.
	#[error("{0} differs between the store and the registry")]
	RegistryConflict(String),
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
