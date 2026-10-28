//! The publisher-side release store: versioned releases on channels, with
//! on-ingest deltas against the previous release, kept in the static
//! [`crate::layout`] so the store directory can be hosted as is.

use std::fs;
use std::io::ErrorKind;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use semver::Version;

use crate::CodePushError;
use crate::delta::Delta;
use crate::delta::create_delta;
use crate::durable;
use crate::durable::write_atomically;
use crate::layout::ChannelIndex;
use crate::layout::DELTAS_DIR;
use crate::layout::INDEX_FORMAT_VERSION;
use crate::layout::ObjectKey;
use crate::layout::RELEASE_FILE;
use crate::layout::SIGNATURE_FILE;
use crate::layout::StaticObject;
use crate::layout::channel_path;
use crate::layout::delta_file_name;
use crate::layout::parse_delta_file_name;
use crate::layout::parse_release_name;
use crate::layout::release_path;
use crate::manifest::BundleManifest;
use crate::manifest::bounded_container_manifest;
use crate::manifest::container_bytes;
use crate::manifest::hash_bytes;
use crate::manifest::pack_directory;
use crate::manifest::read_container;
use crate::signing::ReleaseSignature;
use crate::signing::ReleaseSigningKey;

// Publisher metadata only; static clients continue to use the public layout.
const CONTAINER_HASH_FILE: &str = "release.sha256";

/// A stored release: its manifest, its container bytes, and its publisher
/// signature.
#[derive(Debug, Clone)]
pub struct ReleaseRecord {
	/// The release manifest.
	pub manifest: BundleManifest,
	/// The full container bytes.
	pub container: Vec<u8>,
	/// The publisher signature over the manifest. `None` only for a release
	/// published before signing existed, which clients reject.
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
	/// different contents and the delta is at most three quarters of the
	/// container's size; a device downloads the container otherwise.
	pub delta_from_previous: Option<Delta>,
}

/// An on-disk registry of releases, kept in the static [`crate::layout`]:
/// `<root>/<app>/<channel>/index.json`, and per release
/// `<version>+<build>/release.lkb`, `release.sig`, and
/// `deltas/<base hash>.jsondelta`. The directory is the whole registry state:
/// the axum routes in [`crate::routes`] are a thin reader over it, and any
/// static file server or bucket holding a copy serves the same thing.
///
/// Releases are immutable once ingested. A release directory is assembled
/// whole, container, signature, and delta, under a uniquely named temporary
/// directory next to it, synced, and renamed into place, so no reader ever
/// sees a release with some of its files, and an ingest that dies part way
/// leaves only a temporary directory that nothing reads. The rename is the
/// lock on the release: it never replaces a release directory, so of two
/// concurrent ingests of the same version and build exactly one wins and the
/// other gets [`CodePushError::ReleaseExists`]. Only the winner then writes
/// `index.json`, so a static reader never learns of a release before its
/// files exist.
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

	/// The directory of a release: `<root>/<app>/<channel>/<version>+<build>`.
	fn release_dir(&self, app: &str, channel: &str, version: &Version, build: u64) -> PathBuf {
		self.root.join(release_path(app, channel, version, build))
	}

	/// Ingests a directory as a new release on the channel, signed by
	/// `signer`.
	///
	/// Packs the directory into a container, signs its manifest, computes a
	/// delta against the current latest release, publishes them with the
	/// container hash in one directory rename, and then points the channel's
	/// `index.json` at the newest release. The delta is kept only when it is
	/// at most three quarters of the container's size. Fails with
	/// [`CodePushError::ReleaseExists`] when the version and build are already
	/// released on the channel.
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
		let release_exists =
			|| CodePushError::ReleaseExists(format!("{app}/{channel}/{version}+{build}"));
		let release_dir = self.release_dir(app, channel, version, build);
		if release_dir.join(RELEASE_FILE).try_exists()? {
			return Err(release_exists());
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
		bounded_container_manifest(&container)?;

		// Snapshot the previous release before the new one becomes visible,
		// so the delta is computed against it rather than against itself.
		let previous = self.latest(app, channel)?;
		let delta = match previous {
			Some(previous) if previous.manifest.bundle_hash() != manifest.bundle_hash() => {
				worthwhile_delta(&previous.container, &container)?
			}
			_ => None,
		};

		let signature_json = serde_json::to_vec_pretty(&signature)?;
		let container_hash = hash_bytes(&container);
		let delta_path = delta
			.as_ref()
			.map(|(delta, _)| format!("{DELTAS_DIR}/{}", delta_file_name(&delta.base_hash)));
		let mut files = vec![
			(RELEASE_FILE, container.as_slice()),
			(SIGNATURE_FILE, signature_json.as_slice()),
			(CONTAINER_HASH_FILE, container_hash.as_bytes()),
		];
		if let (Some(path), Some((_, bytes))) = (&delta_path, &delta) {
			files.push((path.as_str(), bytes.as_slice()));
		}
		if !publish_release_dir(&release_dir, &files)? {
			return Err(release_exists());
		}
		self.refresh_index(app, channel)?;

		Ok(IngestedRelease {
			record: ReleaseRecord {
				manifest,
				container,
				signature: Some(signature),
			},
			delta_from_previous: delta.map(|(delta, _)| delta),
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
	/// the store already held these exact bytes, in which case the container
	/// hash is refreshed and a missing signature is written. Fails with
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
		let release_dir = self.release_dir(app, channel, version, build);
		let signature_json = serde_json::to_vec_pretty(signature)?;
		let container_hash = hash_bytes(container);
		let imported = !release_dir.join(RELEASE_FILE).try_exists()?
			&& publish_release_dir(
				&release_dir,
				&[
					(RELEASE_FILE, container),
					(SIGNATURE_FILE, signature_json.as_slice()),
					(CONTAINER_HASH_FILE, container_hash.as_bytes()),
				],
			)?;
		if !imported {
			if fs::read(release_dir.join(RELEASE_FILE))? != container {
				return Err(CodePushError::ReleaseExists(format!(
					"{app}/{channel}/{version}+{build}"
				)));
			}
			write_atomically(
				&release_dir.join(CONTAINER_HASH_FILE),
				container_hash.as_bytes(),
			)?;
			// Older stores wrote the container and the signature one after the
			// other; a release left without its signature is completed by
			// importing it again.
			let signature_path = release_dir.join(SIGNATURE_FILE);
			if !signature_path.try_exists()? {
				write_atomically(&signature_path, &signature_json)?;
			}
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

	/// The persisted container hash, without reading the release body. Older
	/// stores without publisher metadata can still offer full downloads.
	pub(crate) fn container_hash(
		&self,
		app: &str,
		channel: &str,
		version: &Version,
		build: u64,
	) -> Result<Option<String>, StoreError> {
		let release = self.path_of(&ObjectKey::release(app, channel, version, build));
		if !self.object_exists(&ObjectKey::release(app, channel, version, build))? {
			return Ok(None);
		}

		let file = match fs::File::open(release.with_file_name(CONTAINER_HASH_FILE)) {
			Ok(file) => file,
			Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
			Err(error) => return Err(error.into()),
		};
		let mut hash = String::new();
		file.take(65).read_to_string(&mut hash)?;
		if hash.len() != 64
			|| !hash
				.bytes()
				.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
		{
			return Err(
				std::io::Error::new(ErrorKind::InvalidData, "invalid container hash").into(),
			);
		}

		Ok(Some(hash))
	}

	pub(crate) fn object_exists(&self, key: &ObjectKey) -> Result<bool, StoreError> {
		match fs::metadata(self.path_of(key)) {
			Ok(metadata) if metadata.is_file() => Ok(true),
			Ok(_) => {
				Err(std::io::Error::new(ErrorKind::InvalidData, "expected a release file").into())
			}
			Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
			Err(error) => Err(error.into()),
		}
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
	/// Only directories named `<version>+<build>` are releases, so the
	/// temporary directory of an interrupted ingest is skipped, and so is a
	/// release directory without a `release.lkb`, which an older store could
	/// leave behind. Releases written in a container format newer than this
	/// build understands are skipped too, so a newer publisher sharing the
	/// store cannot take a channel down for older servers. Any other failure
	/// (an unreadable or corrupt container, signature, or delta) is returned as
	/// an error rather than hidden. Signatures are read, not verified:
	/// verification is the client's job, against keys the server never holds.
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
			let is_release = entry
				.file_name()
				.to_str()
				.is_some_and(|name| parse_release_name(name).is_some());
			if !is_release || !entry.file_type()?.is_dir() {
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

/// The delta from `previous` to `container` and its serialized bytes, when
/// those are at most three quarters of the container's size.
///
/// A device that has a delta downloads it instead of the container, so a
/// delta that saves less is not worth keeping: it costs the store a file
/// forever and the device the work of patching, for little bandwidth.
fn worthwhile_delta(
	previous: &[u8],
	container: &[u8],
) -> Result<Option<(Delta, Vec<u8>)>, CodePushError> {
	let delta = create_delta(previous, container);
	let bytes = serde_json::to_vec(&delta)?;
	let worthwhile = bytes.len().saturating_mul(4) <= container.len().saturating_mul(3);
	Ok(worthwhile.then_some((delta, bytes)))
}

/// Publishes a release directory whole: assembles `files` in a temporary
/// sibling of `release_dir`, then renames it into place.
///
/// Returns `false`, writing nothing, when the release already exists. A
/// rename never replaces a directory that has entries, so of two writers of
/// the same release exactly one succeeds; the loser recognizes the winner's
/// container.
fn publish_release_dir(release_dir: &Path, files: &[(&str, &[u8])]) -> Result<bool, CodePushError> {
	let channel_dir = durable::parent_of(release_dir);
	fs::create_dir_all(channel_dir)?;
	let assembled = durable::assemble_directory(release_dir, files)?;
	if let Err(error) = fs::rename(&assembled, release_dir) {
		// The rename already failed; that error, or the release that beat it
		// into place, is what to report.
		let _ = fs::remove_dir_all(&assembled);
		return if release_dir.join(RELEASE_FILE).try_exists()? {
			Ok(false)
		} else {
			Err(error.into())
		};
	}
	durable::sync_directory(channel_dir)?;
	Ok(true)
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
