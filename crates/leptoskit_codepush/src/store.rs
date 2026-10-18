//! The server-side release store: versioned releases on channels, with
//! on-ingest deltas against the previous release.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use semver::Version;

use crate::CodePushError;
use crate::delta::Delta;
use crate::delta::apply_delta;
use crate::delta::create_delta;
use crate::manifest::BundleManifest;
use crate::manifest::bundle_directory;
use crate::manifest::read_container;

/// A stored release: its manifest, its container bytes, and the delta against
/// the previous release on the channel.
#[derive(Debug, Clone)]
pub struct ReleaseRecord {
	/// The release manifest.
	pub manifest: BundleManifest,
	/// The full container bytes.
	pub container: Vec<u8>,
	/// The delta from the previous release, when one exists.
	pub delta_from_previous: Option<Delta>,
}

/// An on-disk registry of releases.
///
/// Layout: `<root>/<app>/<channel>/<version>+<build>/release.lkb` plus
/// `delta_from_previous.jsondelta`. The directory is the whole server state;
/// the axum routes in [`crate::routes`] are a thin reader over it.
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

	fn release_dir(&self, app: &str, channel: &str, manifest: &BundleManifest) -> PathBuf {
		self.root
			.join(sanitize(app))
			.join(sanitize(channel))
			.join(format!("{}+{}", manifest.version, manifest.build))
	}

	/// Ingests a directory as a new release on the channel.
	///
	/// Packs the directory into a container, computes a delta against the
	/// current latest release, and writes both under the version directory.
	pub fn ingest_directory(
		&self,
		app: &str,
		channel: &str,
		version: &Version,
		build: u64,
		dir: &Path,
		mandatory: bool,
	) -> Result<ReleaseRecord, CodePushError> {
		let staging = self.root.join(".staging");
		fs::create_dir_all(&staging)?;
		let container_path = staging.join(format!("{}.{}.lkb", sanitize(app), sanitize(channel)));
		let mut manifest = bundle_directory(dir, &container_path)?;
		manifest.app = app.into();
		manifest.channel = channel.into();
		manifest.version = version.clone();
		manifest.build = build;
		manifest.mandatory = mandatory;

		// Re-pack with the corrected identity fields.
		let container = fs::read(&container_path)?;
		let parsed = read_container(&container)?;
		let mut blobs = Vec::with_capacity(parsed.files.len());
		for (_, content) in &parsed.files {
			let mut encoder =
				flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
			use std::io::Write;
			encoder.write_all(content)?;
			blobs.push(encoder.finish()?);
		}

		// Snapshot the previous release before the new one becomes visible,
		// so the delta is computed against it rather than against itself.
		let previous = self.latest(app, channel)?;
		let delta = previous
			.filter(|previous| previous.manifest.bundle_hash() != manifest.bundle_hash())
			.map(|previous| create_delta(&previous.container, &container));

		let release_dir = self.release_dir(app, channel, &manifest);
		fs::create_dir_all(&release_dir)?;
		crate::manifest::write_container(&release_dir.join("release.lkb"), &manifest, &blobs)?;
		let _ = fs::remove_file(&container_path);

		if let Some(delta) = &delta {
			let delta_bytes = serde_json::to_vec_pretty(delta)?;
			fs::write(
				release_dir.join("delta_from_previous.jsondelta"),
				delta_bytes,
			)?;
		}

		Ok(ReleaseRecord {
			manifest,
			container,
			delta_from_previous: delta,
		})
	}

	/// Every release version directory for an app and channel, newest first.
	pub fn list(&self, app: &str, channel: &str) -> Result<Vec<ReleaseRecord>, StoreError> {
		let channel_dir = self.root.join(sanitize(app)).join(sanitize(channel));
		let mut records = Vec::new();
		let entries = match fs::read_dir(&channel_dir) {
			Ok(entries) => entries,
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(records),
			Err(error) => return Err(error.into()),
		};
		for entry in entries {
			let entry = entry?;
			if !entry.file_type()?.is_dir() {
				continue;
			}
			let container = fs::read(entry.path().join("release.lkb"))?;
			let parsed = read_container(&container)?;
			let delta = match fs::read(entry.path().join("delta_from_previous.jsondelta")) {
				Ok(bytes) => Some(serde_json::from_slice(&bytes)?),
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
				Err(error) => return Err(error.into()),
			};
			records.push(ReleaseRecord {
				manifest: parsed.manifest,
				container,
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

	/// Applies a stored delta against the given base container bytes.
	pub fn apply_stored_delta(&self, delta: &Delta, base: &[u8]) -> Result<Vec<u8>, CodePushError> {
		apply_delta(base, delta)
	}
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
