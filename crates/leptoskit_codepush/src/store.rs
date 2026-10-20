//! The publisher-side release store: versioned releases on channels, with
//! on-ingest deltas against the previous release, kept in the static
//! [`crate::layout`] so the store directory can be hosted as is.

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
use crate::layout::ChannelIndex;
use crate::layout::DELTAS_DIR;
use crate::layout::INDEX_FORMAT_VERSION;
use crate::layout::ObjectKey;
use crate::layout::RELEASE_FILE;
use crate::layout::SIGNATURE_FILE;
use crate::layout::StaticObject;
use crate::layout::channel_path;
use crate::layout::parse_delta_file_name;
use crate::manifest::BundleManifest;
use crate::manifest::container_bytes;
use crate::manifest::pack_directory;
use crate::manifest::read_container;
use crate::signing::ReleaseSignature;
use crate::signing::ReleaseSigningKey;

/// A stored release: its manifest, its container bytes, and its publisher
/// signature.
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
}

impl ReleaseRecord {
	/// The `index.json` that points at this release.
	pub fn channel_index(&self) -> ChannelIndex {
		ChannelIndex {
			format_version: INDEX_FORMAT_VERSION,
			manifest: self.manifest.clone(),
			signature: self.signature.clone(),
		}
	}

	/// The version and build, which order releases on a channel.
	pub(crate) fn id(&self) -> (&Version, u64) {
		(&self.manifest.version, self.manifest.build)
	}
}

/// What an ingest published: the new release, and the delta to it from the
/// release that was the channel's newest before it.
#[derive(Debug, Clone)]
pub struct IngestedRelease {
	/// The new release.
	pub record: ReleaseRecord,
	/// The delta from the previous newest release, when there was one with
	/// different contents.
	pub delta_from_previous: Option<Delta>,
}

/// An on-disk registry of releases, kept in the static [`crate::layout`]:
/// `<root>/<app>/<channel>/index.json`, and per release
/// `<version>+<build>/release.lkb`, `release.sig`, and
/// `deltas/<base hash>.jsondelta`. The directory is the whole registry state:
/// the axum routes in [`crate::routes`] are a thin reader over it, and any
/// static file server or bucket holding a copy serves the same thing.
///
/// Releases are immutable once ingested. Each file is written to a uniquely
/// named temporary sibling and then moved into place, so readers never see a
/// partial file. `release.lkb` is published first and without replacing an
/// existing one, which makes it the lock on the release: of two concurrent
/// ingests of the same version and build, exactly one wins and the other gets
/// [`CodePushError::ReleaseExists`]. Only the winner then writes the
/// signature, the delta, and finally `index.json`, so a static reader never
/// learns of a release before its files exist. A release can briefly be
/// listed by [`Self::list`] without its signature: clients reject the unsigned
/// offer and pick the release up on their next check.
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

	fn path_of(&self, key: &ObjectKey) -> PathBuf {
		self.root.join(key.as_str())
	}

	/// Ingests a directory as a new release on the channel, signed by
	/// `signer`.
	///
	/// Packs the directory into a container, signs its manifest, computes a
	/// delta against the current latest release, writes all three under the
	/// version directory, and then points the channel's `index.json` at the
	/// newest release. Fails with [`CodePushError::ReleaseExists`] when the
	/// version and build are already released on the channel.
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
	) -> Result<IngestedRelease, CodePushError> {
		let release_path = self.path_of(&ObjectKey::release(app, channel, version, build));
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

		if let Some(parent) = release_path.parent() {
			fs::create_dir_all(parent)?;
		}
		if !publish_new(&release_path, &container)? {
			return Err(CodePushError::ReleaseExists(format!(
				"{app}/{channel}/{version}+{build}"
			)));
		}
		write_atomically(
			&self.path_of(&ObjectKey::signature(app, channel, version, build)),
			&serde_json::to_vec_pretty(&signature)?,
		)?;
		if let Some(delta) = &delta {
			let key = ObjectKey::delta(app, channel, version, build, &delta.base_hash)
				.expect("delta base hashes are SHA-256 hex");
			let path = self.path_of(&key);
			if let Some(parent) = path.parent() {
				fs::create_dir_all(parent)?;
			}
			write_atomically(&path, &serde_json::to_vec_pretty(delta)?)?;
		}
		self.refresh_index(app, channel)?;

		Ok(IngestedRelease {
			record: ReleaseRecord {
				manifest,
				container,
				signature: Some(signature),
			},
			delta_from_previous: delta,
		})
	}

	/// Imports a release published elsewhere, such as a bucket's newest
	/// release, byte for byte: its container and the publisher signature
	/// that came with it.
	///
	/// The container is parsed and every file checked against its manifest,
	/// which names the app, channel, version, and build the release is stored
	/// under. The signature is stored, not verified: callers that take a
	/// release from an untrusted source check it against their trusted keys
	/// first, as [`crate::pull_channel`] does.
	///
	/// Returns `true` when the release was new to the store and `false` when
	/// the store already held these exact bytes, in which case only a missing
	/// signature is written. Fails with
	/// [`CodePushError::ReleaseExists`] when the store holds a different
	/// release under the same version and build.
	pub fn import_release(
		&self,
		container: &[u8],
		signature: &ReleaseSignature,
	) -> Result<bool, CodePushError> {
		let manifest = read_container(container)?.manifest;
		let (app, channel) = (manifest.app.as_str(), manifest.channel.as_str());
		let (version, build) = (&manifest.version, manifest.build);
		let release_path = self.path_of(&ObjectKey::release(app, channel, version, build));
		if let Some(parent) = release_path.parent() {
			fs::create_dir_all(parent)?;
		}
		let imported = publish_new(&release_path, container)?;
		if !imported && fs::read(&release_path)? != container {
			return Err(CodePushError::ReleaseExists(format!(
				"{app}/{channel}/{version}+{build}"
			)));
		}
		// An import interrupted after its container landed is completed by
		// importing again.
		let signature_path = self.path_of(&ObjectKey::signature(app, channel, version, build));
		if imported || !signature_path.try_exists()? {
			write_atomically(&signature_path, &serde_json::to_vec_pretty(signature)?)?;
		}
		self.refresh_index(app, channel)?;
		Ok(imported)
	}

	/// Points the channel's `index.json` at its newest release.
	///
	/// Concurrent ingests can each write the index from a listing taken
	/// before another's release — or its signature — appeared, so after
	/// writing, the index is computed again and rewritten until the two agree.
	/// Every ingest writes its container and signature before calling this, so
	/// whichever writes last leaves the index naming the true newest release,
	/// signed.
	fn refresh_index(&self, app: &str, channel: &str) -> Result<(), CodePushError> {
		let path = self.path_of(&ObjectKey::index(app, channel));
		let mut written: Option<ChannelIndex> = None;
		loop {
			let Some(latest) = self.latest(app, channel)? else {
				return Ok(());
			};
			let index = latest.channel_index();
			if written.as_ref() == Some(&index) {
				return Ok(());
			}
			write_atomically(&path, &serde_json::to_vec_pretty(&index)?)?;
			written = Some(index);
		}
	}

	/// The channel's `index.json`, when the channel has a release.
	pub fn index(&self, app: &str, channel: &str) -> Result<Option<ChannelIndex>, StoreError> {
		read_optional_json(&self.path_of(&ObjectKey::index(app, channel)))
	}

	/// The delta into the release `version`+`build` from the container that
	/// hashes to `base_hash`, when the store has one.
	pub fn delta(
		&self,
		app: &str,
		channel: &str,
		version: &Version,
		build: u64,
		base_hash: &str,
	) -> Result<Option<Delta>, StoreError> {
		match ObjectKey::delta(app, channel, version, build, base_hash) {
			Some(key) => read_optional_json(&self.path_of(&key)),
			None => Ok(None),
		}
	}

	/// The bytes of a file in the layout, or `None` when it does not exist.
	pub fn read_object(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>, StoreError> {
		read_optional(&self.path_of(key)).map_err(StoreError::from)
	}

	/// Every file of a release: its container, its signature when it has one,
	/// and its deltas, in that order.
	pub fn release_objects(&self, record: &ReleaseRecord) -> Result<Vec<StaticObject>, StoreError> {
		let manifest = &record.manifest;
		let (app, channel) = (manifest.app.as_str(), manifest.channel.as_str());
		let (version, build) = (&manifest.version, manifest.build);
		let mut objects = vec![StaticObject {
			key: ObjectKey::release(app, channel, version, build),
			bytes: record.container.clone(),
		}];
		let signature = ObjectKey::signature(app, channel, version, build);
		if let Some(bytes) = self.read_object(&signature)? {
			objects.push(StaticObject {
				key: signature,
				bytes,
			});
		}
		let deltas_dir = self
			.path_of(&ObjectKey::release(app, channel, version, build))
			.with_file_name(DELTAS_DIR);
		let entries = match fs::read_dir(&deltas_dir) {
			Ok(entries) => entries,
			Err(error) if error.kind() == ErrorKind::NotFound => return Ok(objects),
			Err(error) => return Err(error.into()),
		};
		let mut deltas = Vec::new();
		for entry in entries {
			let name = entry?.file_name();
			// Temporary files of an interrupted write, and anything else that
			// is not a delta, are not part of the release.
			let Some(key) = name
				.to_str()
				.and_then(parse_delta_file_name)
				.and_then(|base_hash| ObjectKey::delta(app, channel, version, build, base_hash))
			else {
				continue;
			};
			if let Some(bytes) = self.read_object(&key)? {
				deltas.push(StaticObject { key, bytes });
			}
		}
		deltas.sort_by(|left, right| left.key.as_str().cmp(right.key.as_str()));
		objects.extend(deltas);
		Ok(objects)
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
		let channel_dir = self.root.join(channel_path(app, channel));
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
			records.push(ReleaseRecord {
				manifest: parsed.manifest,
				container,
				signature,
			});
		}
		records.sort_by(|left, right| right.id().cmp(&left.id()));
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

/// Reads a file that may not exist yet.
fn read_optional(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
	match fs::read(path) {
		Ok(bytes) => Ok(Some(bytes)),
		Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
		Err(error) => Err(error),
	}
}

/// Reads and parses a JSON file that may not exist yet.
fn read_optional_json<T: serde::de::DeserializeOwned>(
	path: &Path,
) -> Result<Option<T>, StoreError> {
	match read_optional(path)? {
		Some(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
		None => Ok(None),
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
