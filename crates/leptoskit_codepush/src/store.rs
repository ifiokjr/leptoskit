//! The server-side release store: versioned releases on channels, with
//! on-ingest deltas against the previous release.

use std::fs;
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use semver::Version;

use crate::CodePushError;
use crate::delta::Delta;
use crate::delta::create_delta;
use crate::manifest::BundleManifest;
use crate::manifest::container_bytes;
use crate::manifest::pack_directory;
use crate::manifest::read_container;
use crate::signing::ReleaseSignature;
use crate::signing::ReleaseSigningKey;

/// The container file inside a release directory. Its presence makes the
/// release visible; an ingest publishes it before the signature and delta.
const RELEASE_FILE: &str = "release.lkb";

/// The detached publisher signature inside a release directory.
const SIGNATURE_FILE: &str = "release.sig";

/// The delta file inside a release directory, when the release has one.
const DELTA_FILE: &str = "delta_from_previous.jsondelta";

/// A stored release: its manifest, its container bytes, its publisher
/// signature, and the delta against the previous release on the channel.
#[derive(Debug, Clone)]
pub struct ReleaseRecord {
	/// The release manifest.
	pub manifest: BundleManifest,
	/// The full container bytes.
	pub container: Vec<u8>,
	/// The publisher signature over the manifest. `None` only for a release
	/// published before signing existed, or read in the instant between its
	/// container and its signature landing; clients reject it either way.
	pub signature: Option<ReleaseSignature>,
	/// The delta from the previous release, when one exists.
	pub delta_from_previous: Option<Delta>,
}

/// An on-disk registry of releases.
///
/// Layout: `<root>/<app>/<channel>/<version>+<build>/release.lkb` plus
/// `release.sig` and `delta_from_previous.jsondelta`. The directory is the
/// whole server state; the axum routes in [`crate::routes`] are a thin reader
/// over it.
///
/// Releases are immutable once ingested. Each file is written to a uniquely
/// named temporary sibling and then moved into place, so readers never see a
/// partial file. `release.lkb` is published first and without replacing an
/// existing one, which makes it the lock on the release: of two concurrent
/// ingests of the same version and build, exactly one wins and the other gets
/// [`CodePushError::ReleaseExists`]. Only the winner then writes the signature
/// and the delta, so a release can briefly be listed without them: clients
/// reject the unsigned offer and pick the release up on their next check, and
/// download the full container while the delta is missing.
#[derive(Debug, Clone)]
pub struct ReleaseStore {
	root: PathBuf,
}

impl ReleaseStore {
	/// A store rooted at `root`, created if missing.
	pub fn open(root: impl Into<PathBuf>) -> Result<Self, CodePushError> {
		let root = root.into();
		fs::create_dir_all(&root)?;
		Ok(Self { root })
	}

	/// The store root.
	pub fn root(&self) -> &Path {
		&self.root
	}

	fn release_dir(&self, app: &str, channel: &str, version: &Version, build: u64) -> PathBuf {
		self.root
			.join(sanitize(app))
			.join(sanitize(channel))
			.join(format!("{version}+{build}"))
	}

	/// Ingests a directory as a new release on the channel, signed by
	/// `signer`.
	///
	/// Packs the directory into a container, signs its manifest, computes a
	/// delta against the current latest release, and writes all three under
	/// the version directory. Fails with [`CodePushError::ReleaseExists`] when
	/// the version and build are already released on the channel.
	#[expect(
		clippy::too_many_arguments,
		reason = "each argument is an independent part of the release identity"
	)]
	pub fn ingest_directory(
		&self,
		signer: &ReleaseSigningKey,
		app: &str,
		channel: &str,
		version: &Version,
		build: u64,
		dir: &Path,
		mandatory: bool,
	) -> Result<ReleaseRecord, CodePushError> {
		let release_dir = self.release_dir(app, channel, version, build);
		let release_path = release_dir.join(RELEASE_FILE);
		if release_path.try_exists()? {
			return Err(CodePushError::ReleaseExists(format!(
				"{app}/{channel}/{version}+{build}"
			)));
		}

		let (mut manifest, blobs) = pack_directory(dir)?;
		manifest.app = app.into();
		manifest.channel = channel.into();
		manifest.version = version.clone();
		manifest.build = build;
		manifest.mandatory = mandatory;
		let signature = signer.sign(&manifest);

		// The container bytes are built once and become the single artifact:
		// what lands on disk, what the record carries, and what the delta is
		// computed against are the same bytes, so a client patched onto them
		// verifies.
		let container = container_bytes(&manifest, &blobs)?;

		// Snapshot the previous release before the new one becomes visible,
		// so the delta is computed against it rather than against itself.
		let previous = self.latest(app, channel)?;
		let delta = previous
			.filter(|previous| previous.manifest.bundle_hash() != manifest.bundle_hash())
			.map(|previous| create_delta(&previous.container, &container));

		fs::create_dir_all(&release_dir)?;
		if !publish_new(&release_path, &container)? {
			return Err(CodePushError::ReleaseExists(format!(
				"{app}/{channel}/{version}+{build}"
			)));
		}
		write_atomically(
			&release_dir.join(SIGNATURE_FILE),
			&serde_json::to_vec_pretty(&signature)?,
		)?;
		if let Some(delta) = &delta {
			write_atomically(
				&release_dir.join(DELTA_FILE),
				&serde_json::to_vec_pretty(delta)?,
			)?;
		}

		Ok(ReleaseRecord {
			manifest,
			container,
			signature: Some(signature),
			delta_from_previous: delta,
		})
	}

	/// Every complete release for an app and channel, newest first.
	///
	/// Version directories without a `release.lkb` (an interrupted ingest) are
	/// skipped, and so are releases written in a container format newer than
	/// this build understands, so a newer publisher sharing the store cannot
	/// take a channel down for older servers. Any other failure (an unreadable
	/// or corrupt container, signature, or delta) is returned as an error
	/// rather than hidden. Signatures are read, not verified: verification is
	/// the client's job, against keys the server never holds.
	pub fn list(&self, app: &str, channel: &str) -> Result<Vec<ReleaseRecord>, StoreError> {
		let channel_dir = self.root.join(sanitize(app)).join(sanitize(channel));
		let mut records = Vec::new();
		let entries = match fs::read_dir(&channel_dir) {
			Ok(entries) => entries,
			Err(error) if error.kind() == ErrorKind::NotFound => return Ok(records),
			Err(error) => return Err(error.into()),
		};
		for entry in entries {
			let entry = entry?;
			if !entry.file_type()?.is_dir() {
				continue;
			}
			let container = match fs::read(entry.path().join(RELEASE_FILE)) {
				Ok(container) => container,
				Err(error) if error.kind() == ErrorKind::NotFound => continue,
				Err(error) => return Err(error.into()),
			};
			let parsed = match read_container(&container) {
				Ok(parsed) => parsed,
				Err(CodePushError::UnsupportedFormatVersion(_)) => continue,
				Err(error) => return Err(error.into()),
			};
			let signature = read_optional_json(&entry.path().join(SIGNATURE_FILE))?;
			let delta = read_optional_json(&entry.path().join(DELTA_FILE))?;
			records.push(ReleaseRecord {
				manifest: parsed.manifest,
				container,
				signature,
				delta_from_previous: delta,
			});
		}
		records.sort_by(|left, right| {
			(left.manifest.version.clone(), left.manifest.build)
				.cmp(&(right.manifest.version.clone(), right.manifest.build))
		});
		records.reverse();
		Ok(records)
	}

	/// The newest release for an app and channel.
	pub fn latest(&self, app: &str, channel: &str) -> Result<Option<ReleaseRecord>, StoreError> {
		Ok(self.list(app, channel)?.into_iter().next())
	}

	/// A specific release by version and build.
	pub fn find(
		&self,
		app: &str,
		channel: &str,
		version: &Version,
		build: u64,
	) -> Result<Option<ReleaseRecord>, StoreError> {
		Ok(self
			.list(app, channel)?
			.into_iter()
			.find(|record| record.manifest.version == *version && record.manifest.build == build))
	}
}

/// Reads and parses a JSON file that may not exist yet.
fn read_optional_json<T: serde::de::DeserializeOwned>(
	path: &Path,
) -> Result<Option<T>, StoreError> {
	match fs::read(path) {
		Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
		Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
		Err(error) => Err(error.into()),
	}
}

/// Writes `bytes` to a temporary sibling of `path`, syncs it, and renames it
/// into place, so readers see either the whole file or none of it.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), CodePushError> {
	let temp_path = write_temp_sibling(path, bytes)?;
	fs::rename(&temp_path, path)?;
	Ok(())
}

/// Writes `bytes` to `path` only if nothing is there yet, atomically.
///
/// Returns `false`, leaving the existing file untouched, when `path` already
/// exists. The bytes go to a temporary sibling first and are hard-linked into
/// place, so the check and the publish are one filesystem operation.
fn publish_new(path: &Path, bytes: &[u8]) -> Result<bool, CodePushError> {
	let temp_path = write_temp_sibling(path, bytes)?;
	let linked = fs::hard_link(&temp_path, path);
	fs::remove_file(&temp_path)?;
	match linked {
		Ok(()) => Ok(true),
		Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
		Err(error) => Err(error.into()),
	}
}

/// Writes and syncs `bytes` to a new, uniquely named file next to `path`, so
/// concurrent writers never share a temporary file.
fn write_temp_sibling(path: &Path, bytes: &[u8]) -> Result<PathBuf, CodePushError> {
	static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
	let file_name = path
		.file_name()
		.expect("store paths end in a file name")
		.to_string_lossy();
	let nanos = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |elapsed| elapsed.as_nanos());
	let unique = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
	let temp_path = path.with_file_name(format!(
		".{file_name}.{}.{nanos}.{unique}.tmp",
		std::process::id()
	));
	let mut file = fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(&temp_path)?;
	file.write_all(bytes)?;
	file.sync_all()?;
	Ok(temp_path)
}

fn sanitize(component: &str) -> String {
	component
		.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
				character
			} else {
				'_'
			}
		})
		.collect()
}

/// Errors raised by the release store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
	/// An IO error.
	#[error("io error: {0}")]
	Io(#[from] std::io::Error),
	/// A manifest or delta failed to parse.
	#[error("serialization error: {0}")]
	Serialize(#[from] serde_json::Error),
	/// A container was malformed.
	#[error("code push error: {0}")]
	CodePush(#[from] CodePushError),
}
