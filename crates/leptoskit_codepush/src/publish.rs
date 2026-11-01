//! Publishing a channel from a [`ReleaseStore`] to a bucket, so an object
//! store behind a CDN can be the registry with no server running.
//!
//! The bucket is a mirror of the store's static [`crate::layout`]. A publish
//! uploads every release newer than the one the bucket's `index.json` names,
//! oldest first, and writes `index.json` last: a device never learns of a
//! release whose container, signature, or deltas are not there yet, and an
//! interrupted publish leaves the channel on its previous release until it
//! is run again.
//!
//! [`pull_channel`] goes the other way: it copies the bucket's newest
//! release into a store, so a CI runner that starts from nothing can number
//! its next build and compute a delta from what devices run, without keeping
//! the store between runs.
//!
//! The bucket needs no trust. Clients verify the signed manifest in
//! `index.json` before downloading anything, so whoever controls the bucket
//! can withhold updates but cannot ship code or roll devices back.

use semver::Version;

use crate::CodePushError;
use crate::layout::ChannelIndex;
use crate::layout::INDEX_FORMAT_VERSION;
use crate::layout::ObjectKey;
use crate::layout::ObjectKind;
use crate::layout::StaticObject;
use crate::layout::release_name;
use crate::manifest::container_manifest;
use crate::signing::TrustedKeys;
use crate::store::ReleaseStore;

/// An object store a channel is published to, such as an S3-compatible
/// bucket.
pub trait Bucket {
	/// Reads an object, returning `None` when it does not exist. Enforce
	/// [`crate::ObjectKind::max_bytes`] while reading the response body.
	fn get(&mut self, key: &ObjectKey) -> Result<Option<Vec<u8>>, CodePushError>;

	/// Writes an object, replacing any existing one, with the
	/// [`crate::ObjectKind::content_type`] and
	/// [`crate::ObjectKind::cache_control`] of its key.
	fn put(&mut self, object: &StaticObject) -> Result<(), CodePushError>;
}

/// What a publish did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReport {
	/// The version of the release `index.json` now names.
	pub version: Version,
	/// The build of the release `index.json` now names.
	pub build: u64,
	/// The releases uploaded, oldest first, as `version+build`.
	pub releases: Vec<String>,
	/// How many objects were written, the index included.
	pub written: usize,
	/// How many release files were already in the bucket with the same
	/// contents, from an earlier, interrupted publish.
	pub already_present: usize,
}

/// Publishes a channel's releases from `store` to `bucket`.
///
/// Uploads every release newer than the one the bucket's `index.json` names
/// and then points the index at the store's newest release. Running it again
/// after a failure resumes where it stopped; running it with nothing new
/// writes nothing.
/// Optional deltas above [`ObjectKind::max_bytes`] are omitted, so clients
/// download the full release instead.
///
/// Releases are immutable, so a release file that already exists in the
/// bucket must match the store's copy byte for byte, and the release the
/// bucket's index names must be the same release in the store. Fails with:
///
/// - [`CodePushError::NoReleases`] when the store has nothing on the channel;
/// - [`CodePushError::Unsigned`] when the newest release has no signature;
/// - [`CodePushError::RegistryAhead`] when the bucket names a newer release
///   than the store holds;
/// - [`CodePushError::RegistryConflict`] when the bucket holds different
///   contents for a release or file the store also has.
///
/// One publisher per channel at a time: the read-compare-write of each object
/// is not atomic against another publisher writing the same bucket.
pub fn publish_channel(
	store: &ReleaseStore,
	app: &str,
	channel: &str,
	bucket: &mut impl Bucket,
) -> Result<PublishReport, CodePushError> {
	let records = store.list(app, channel)?;
	let latest = records
		.first()
		.ok_or_else(|| CodePushError::NoReleases(format!("{app}/{channel}")))?;
	let latest_name = release_name(&latest.manifest.version, latest.manifest.build);

	if latest.signature.is_none() {
		return Err(CodePushError::Unsigned(format!(
			"{app}/{channel}/{latest_name}"
		)));
	}

	let index_key = ObjectKey::index(app, channel);
	let remote = bucket
		.get(&index_key)?
		.map(|bytes| parse_index(&bytes))
		.transpose()?;
	let remote_id = remote
		.as_ref()
		.map(|index| (&index.manifest.version, index.manifest.build));

	if let (Some(remote), Some(remote_id)) = (&remote, remote_id) {
		let remote_name = release_name(remote_id.0, remote_id.1);

		if remote_id > latest.id() {
			return Err(CodePushError::RegistryAhead {
				registry: remote_name,
				store: latest_name,
			});
		}

		let diverged = records.iter().any(|record| {
			record.id() == remote_id
				&& record.manifest.bundle_hash() != remote.manifest.bundle_hash()
		});

		if diverged {
			return Err(CodePushError::RegistryConflict(format!(
				"{app}/{channel}/{remote_name}"
			)));
		}
	}

	let mut report = PublishReport {
		version: latest.manifest.version.clone(),
		build: latest.manifest.build,
		releases: Vec::new(),
		written: 0,
		already_present: 0,
	};
	let pending = records
		.iter()
		.rev()
		.filter(|record| remote_id.is_none_or(|remote_id| record.id() > remote_id));

	for record in pending {
		for object in store.release_objects(record)? {
			if object.key.kind() == ObjectKind::Delta
				&& object.bytes.len() as u64 > ObjectKind::Delta.max_bytes()
			{
				continue;
			}

			match bucket.get(&object.key)? {
				Some(existing) if existing == object.bytes => report.already_present += 1,
				Some(_) => {
					return Err(CodePushError::RegistryConflict(
						object.key.as_str().to_owned(),
					));
				}
				None => {
					bucket.put(&object)?;
					report.written += 1;
				}
			}
		}

		report.releases.push(release_name(
			&record.manifest.version,
			record.manifest.build,
		));
	}

	let index = latest.channel_index();

	if remote.as_ref() != Some(&index) {
		bucket.put(&StaticObject {
			key: index_key,
			bytes: serde_json::to_vec_pretty(&index)?,
		})?;
		report.written += 1;
	}

	Ok(report)
}

/// What a pull did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullReport {
	/// The bucket has no `index.json` for the channel yet.
	Empty,
	/// The bucket's newest release is in the store.
	Pulled {
		/// The version of the bucket's newest release.
		version: Version,
		/// The build of the bucket's newest release.
		build: u64,
		/// Whether the release was new to the store, rather than already in it.
		imported: bool,
	},
}

/// Copies the newest release on a channel from `bucket` into `store`.
///
/// The release is verified before anything is written: `index.json` must be
/// in a format this build reads, name `app` and `channel`, and be signed by
/// one of `trusted`; the container must carry exactly that signed manifest,
/// which is checked before any file is decompressed; and every file must
/// match its hash in it. A bucket can therefore not plant a release in the
/// store that the next publish would build on, nor make the runner inflate
/// files the publisher never released.
/// Container size, expansion, metadata and file count are bounded as
/// described by [`crate::read_container`].
///
/// Only the newest release is pulled. It is all the next release needs: the
/// version and build to number from, and the container to compute the delta
/// against, which is the one devices on the channel run.
///
/// Fails with [`CodePushError::RegistryConflict`] when the store already
/// holds a different release under the same version and build.
pub fn pull_channel(
	store: &ReleaseStore,
	app: &str,
	channel: &str,
	bucket: &mut impl Bucket,
	trusted: &TrustedKeys,
) -> Result<PullReport, CodePushError> {
	let Some(index_bytes) = bucket.get(&ObjectKey::index(app, channel))? else {
		return Ok(PullReport::Empty);
	};
	let index = parse_index(&index_bytes)?;

	if index.format_version != INDEX_FORMAT_VERSION {
		return Err(CodePushError::UnsupportedFormatVersion(
			index.format_version,
		));
	}

	let manifest = &index.manifest;

	if manifest.app != app || manifest.channel != channel {
		return Err(CodePushError::WrongTarget {
			tracked: format!("{app}/{channel}"),
			offered: format!("{}/{}", manifest.app, manifest.channel),
		});
	}

	let signature = index.signature.as_ref().ok_or_else(|| {
		CodePushError::Unsigned(format!(
			"{app}/{channel}/{}",
			release_name(&manifest.version, manifest.build)
		))
	})?;
	trusted.verify(manifest, Some(signature))?;

	let (version, build) = (&manifest.version, manifest.build);
	let release_key = ObjectKey::release(app, channel, version, build);
	let container = bucket
		.get(&release_key)?
		.ok_or_else(|| CodePushError::NotFound(release_key.as_str().to_owned()))?;
	// The container's own manifest declares how large each file inflates, so
	// it must be the signed one before anything is decompressed.
	let (container_manifest, _) = container_manifest(&container)?;

	if container_manifest.bundle_hash() != manifest.bundle_hash() {
		return Err(CodePushError::HashMismatch {
			path: release_key.as_str().to_owned(),
			expected: manifest.bundle_hash(),
			actual: container_manifest.bundle_hash(),
		});
	}

	// Importing checks every file against the manifest.
	let imported = match store.import_release(&container, signature) {
		Ok(imported) => imported,
		Err(CodePushError::ReleaseExists(release)) => {
			return Err(CodePushError::RegistryConflict(release));
		}
		Err(error) => return Err(error),
	};

	Ok(PullReport::Pulled {
		version: version.clone(),
		build,
		imported,
	})
}

fn parse_index(bytes: &[u8]) -> Result<ChannelIndex, CodePushError> {
	if bytes.len() as u64 > ObjectKind::Index.max_bytes() {
		return Err(CodePushError::Malformed("index exceeds 2 MiB".to_owned()));
	}

	Ok(serde_json::from_slice(bytes)?)
}
