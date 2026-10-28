//! The updater client: check, verify, download (full or delta), verify,
//! stage.
//!
//! Nothing from the registry is trusted. An offer is accepted only when its
//! manifest is signed by a key in the client's [`TrustedKeys`], names the
//! client's own app and channel, and is newer than the running release; the
//! downloaded container must then carry exactly that signed manifest, which
//! is checked before any file is decompressed, and every file must match its
//! hash in it. A registry or proxy can therefore withhold updates, but not
//! ship its own code or roll a device back to an older release.
//!
//! The client speaks the static [`crate::layout`]: it reads the channel's
//! `index.json`, asks for the delta from its exact container by hash, and
//! falls back to the full container on a miss. Every request is a plain GET
//! of a file that never changes except the index, so the registry can be
//! `leptoskit codepush serve`, any static file server, or an object store
//! behind a CDN.
//!
//! The transport is a trait so each platform shell brings its own HTTP stack
//! — `URLSession` on iOS, `OkHttp` on Android, `reqwest` on desktop — and
//! everything else (the protocol, the verification, the staging) is shared.

use std::path::Path;

use semver::Version;
use serde::Deserialize;
use serde::Serialize;

use crate::CodePushError;
use crate::durable;
use crate::layout::ChannelIndex;
use crate::layout::INDEX_FORMAT_VERSION;
use crate::layout::ObjectKey;
use crate::layout::encode_component;
use crate::manifest::BundleManifest;
use crate::manifest::hash_bytes;
use crate::manifest::read_container;
use crate::signing::ReleaseSignature;
use crate::signing::TrustedKeys;
use crate::store::ReleaseStore;

/// How a client fetches bytes from the release registry.
pub trait Transport {
	/// Fetches `url`, returning the body of a successful response.
	///
	/// A missing file, an HTTP 404, must be reported as
	/// [`CodePushError::NotFound`]: that is how the client learns a channel
	/// has no releases yet or that no delta exists from its container. Any
	/// other failure, a 403 included, is a [`CodePushError::Transport`], so a
	/// misconfigured registry is an error rather than a silent "up to date";
	/// serve the registry publicly, so that missing files are 404s.
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError>;
}

impl<T: Transport + ?Sized> Transport for &mut T {
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
		(**self).fetch(url)
	}
}

/// An authenticated container, downloaded without changing running identity.
pub struct VerifiedDownload {
	/// Complete original container, retained for offline signature
	/// verification.
	pub container: Vec<u8>,
	/// Manifest authenticated before downloading.
	pub manifest: BundleManifest,
	/// Detached publisher signature.
	pub signature: ReleaseSignature,
	/// Whether the container was reconstructed from a delta.
	pub used_delta: bool,
}

// Containers run to megabytes; a debug print names them by size.
impl std::fmt::Debug for VerifiedDownload {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("VerifiedDownload")
			.field("container_len", &self.container.len())
			.field("manifest", &self.manifest)
			.field("signature", &self.signature)
			.field("used_delta", &self.used_delta)
			.finish()
	}
}

/// A transport that reads from the filesystem, for tests and local channels:
/// `file:///srv/releases` as the registry base reads a store directory or a
/// copy of a bucket.
#[derive(Debug, Default)]
pub struct FileTransport;

impl Transport for FileTransport {
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
		let path = decode_path(url.strip_prefix("file://").unwrap_or(url))?;
		std::fs::read(&path).map_err(|error| {
			if error.kind() == std::io::ErrorKind::NotFound {
				CodePushError::NotFound(path)
			} else {
				CodePushError::Transport(format!("could not read {path}: {error}"))
			}
		})
	}
}

/// Decodes the percent escapes of a URL path.
fn decode_path(path: &str) -> Result<String, CodePushError> {
	let malformed = || CodePushError::Transport(format!("malformed path {path:?}"));
	let mut decoded = Vec::with_capacity(path.len());
	let mut bytes = path.bytes();
	while let Some(byte) = bytes.next() {
		if byte != b'%' {
			decoded.push(byte);
			continue;
		}
		let high = bytes
			.next()
			.and_then(|digit| char::from(digit).to_digit(16));
		let low = bytes
			.next()
			.and_then(|digit| char::from(digit).to_digit(16));
		let (Some(high), Some(low)) = (high, low) else {
			return Err(malformed());
		};
		decoded.push((high * 16 + low) as u8);
	}
	String::from_utf8(decoded).map_err(|_| malformed())
}

/// The answer to an update check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum UpdatePlan {
	/// The client already runs the newest release.
	UpToDate,
	/// A newer release is available.
	Available(Box<UpdateOffer>),
}

/// A downloadable update.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateOffer {
	/// The manifest of the new release.
	pub manifest: BundleManifest,
	/// The publisher signature over the manifest. Clients reject offers
	/// without one.
	#[serde(default)]
	pub signature: Option<ReleaseSignature>,
	/// The full container download URL.
	pub download_url: String,
	/// The delta download URL, when the server can serve one against the
	/// client's version.
	pub delta_url: Option<String>,
	/// The hash of the base the delta expects, verified before applying.
	pub delta_base_hash: Option<String>,
}

/// What the sync pass did.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncReport {
	/// Whether an update was downloaded and staged.
	pub updated: bool,
	/// The version now staged, when one is.
	pub version: Option<Version>,
	/// Whether the update arrived as a delta.
	pub used_delta: bool,
}

/// The updater.
pub struct CodePushClient<T: Transport> {
	transport: T,
	base_url: String,
	app: String,
	channel: String,
	current_version: Version,
	current_build: u64,
	/// The bytes of the currently running container.
	current_container: Vec<u8>,
	/// The publisher keys releases must be signed by.
	trusted_keys: TrustedKeys,
	/// The largest container the client accepts, downloaded or rebuilt.
	max_container_bytes: u64,
}

impl<T: Transport> CodePushClient<T> {
	/// Creates a client tracking `base_url` for an app and channel, running
	/// `current_version`+`current_build`, that installs only releases signed
	/// by one of `trusted_keys`.
	///
	/// The shell must pass the release it actually runs — the store build or
	/// the last staged update — because it is the floor every offer has to
	/// beat: an older release, however validly signed, is never installed.
	#[expect(
		clippy::too_many_arguments,
		reason = "each argument is required, independent client state"
	)]
	pub fn new(
		transport: T,
		base_url: impl Into<String>,
		app: impl Into<String>,
		channel: impl Into<String>,
		current_version: Version,
		current_build: u64,
		current_container: Vec<u8>,
		trusted_keys: TrustedKeys,
	) -> Self {
		Self {
			transport,
			base_url: base_url.into().trim_end_matches('/').to_owned(),
			app: app.into(),
			channel: channel.into(),
			current_version,
			current_build,
			current_container,
			trusted_keys,
			max_container_bytes: crate::delta::MAX_DELTA_OUTPUT,
		}
	}

	/// Caps the size of a container the client accepts, whether downloaded
	/// in full or rebuilt from a delta. Defaults to
	/// [`MAX_DELTA_OUTPUT`](crate::delta::MAX_DELTA_OUTPUT).
	///
	/// A delta whose output would exceed the cap, or whose own body does, is
	/// refused before anything is allocated for it, and the client falls back
	/// to the full download. A full download over the cap is an error before
	/// it is parsed or decompressed. The transport has already received that
	/// body, so the [`Transport`] must bound what it reads as well, as the
	/// built-in `HttpTransport` does.
	#[must_use]
	pub fn with_max_container_bytes(mut self, limit: u64) -> Self {
		self.max_container_bytes = limit;
		self
	}

	/// Reads the channel's `index.json` and turns it into an update plan.
	///
	/// The index is verified — signed by a trusted key, for this client's app
	/// and channel — before anything else; a rejected index is an error, never
	/// a silent up-to-date. A channel without an index yet, or one whose
	/// newest release is not newer than the running one (a CDN still serving
	/// the previous index, say), is up to date.
	///
	/// An offer always names the delta from the running container by that
	/// container's hash; whether the registry has it is only known by asking,
	/// and [`Self::sync`] falls back to the full container when it does not.
	pub fn check(&mut self) -> Result<UpdatePlan, CodePushError> {
		let index_url = self.url(&ObjectKey::index(&self.app, &self.channel));
		let body = match self.transport.fetch(&index_url) {
			Ok(body) => body,
			Err(CodePushError::NotFound(_)) => return Ok(UpdatePlan::UpToDate),
			Err(error) => return Err(error),
		};
		let index: ChannelIndex = serde_json::from_slice(&body)?;
		if index.format_version != INDEX_FORMAT_VERSION {
			return Err(CodePushError::UnsupportedFormatVersion(
				index.format_version,
			));
		}
		self.verify_release(&index.manifest, index.signature.as_ref())?;
		if !self.is_newer(&index.manifest) {
			return Ok(UpdatePlan::UpToDate);
		}

		let manifest = index.manifest;
		let (app, channel) = (manifest.app.as_str(), manifest.channel.as_str());
		let (version, build) = (&manifest.version, manifest.build);
		let release = ObjectKey::release(app, channel, version, build);
		// Without a running container there is nothing to patch, so there is
		// no point asking for a delta.
		let delta = (!self.current_container.is_empty())
			.then(|| hash_bytes(&self.current_container))
			.and_then(|base_hash| {
				let key = ObjectKey::delta(app, channel, version, build, &base_hash)?;
				Some((key, base_hash))
			});
		Ok(UpdatePlan::Available(Box::new(UpdateOffer {
			download_url: format!("/{}", release.url_path()),
			delta_url: delta
				.as_ref()
				.map(|(key, _)| format!("/{}", key.url_path())),
			delta_base_hash: delta.map(|(_, base_hash)| base_hash),
			manifest,
			signature: index.signature,
		})))
	}

	/// Checks that an offer may be installed: signed by a trusted key, for
	/// this client's app and channel, and newer than the running release.
	///
	/// The signature covers the whole manifest — app, channel, version, build,
	/// mandatory flag, and every file hash — so once it verifies, the other
	/// checks are checks of what the publisher actually released.
	pub fn verify_offer(&self, offer: &UpdateOffer) -> Result<(), CodePushError> {
		let manifest = &offer.manifest;
		self.verify_release(manifest, offer.signature.as_ref())?;
		if !self.is_newer(manifest) {
			return Err(CodePushError::NotNewer {
				current: format!("{}+{}", self.current_version, self.current_build),
				offered: format!("{}+{}", manifest.version, manifest.build),
			});
		}
		Ok(())
	}

	/// Checks that a release is signed by a trusted key and is for this
	/// client's app and channel.
	fn verify_release(
		&self,
		manifest: &BundleManifest,
		signature: Option<&ReleaseSignature>,
	) -> Result<(), CodePushError> {
		self.trusted_keys.verify(manifest, signature)?;
		if manifest.app != self.app || manifest.channel != self.channel {
			return Err(CodePushError::WrongTarget {
				tracked: format!("{}/{}", self.app, self.channel),
				offered: format!("{}/{}", manifest.app, manifest.channel),
			});
		}
		Ok(())
	}

	/// Whether a release is newer than the running one, comparing version and
	/// then build.
	fn is_newer(&self, manifest: &BundleManifest) -> bool {
		(&manifest.version, manifest.build) > (&self.current_version, self.current_build)
	}

	/// Runs one full sync: check, download, verify, stage into `staging_dir`.
	///
	/// The offer is verified by [`Self::check`] before anything is downloaded,
	/// and the container, whether downloaded or rebuilt from a delta, must
	/// carry the signed manifest and match every file hash in it before
	/// anything is staged.
	///
	/// When the offer includes a delta, the delta is tried first; if fetching,
	/// parsing, or applying it fails for any reason (not on the registry,
	/// stale base, corrupt bytes, over the size cap), the sync falls back to
	/// the full download and reports `used_delta: false`. Failures of the full
	/// download or of container verification are returned as errors.
	///
	/// The release replaces `staging_dir` whole: it is written to a temporary
	/// directory next to it and renamed into place, so files of the previous
	/// release do not survive, and a sync that fails leaves the previous
	/// staging as it was.
	pub fn sync(&mut self, staging_dir: &Path) -> Result<SyncReport, CodePushError> {
		let Some(download) = self.download_verified()? else {
			return Ok(SyncReport {
				updated: false,
				version: None,
				used_delta: false,
			});
		};
		let parsed = read_container(&download.container)?;
		stage(staging_dir, &parsed)?;
		self.current_container = download.container;
		self.current_version = download.manifest.version.clone();
		self.current_build = download.manifest.build;

		Ok(SyncReport {
			updated: true,
			version: Some(download.manifest.version),
			used_delta: download.used_delta,
		})
	}

	/// Downloads and authenticates an update without staging or advancing the
	/// running release. Native hosts confirm identity only after a healthy
	/// frame.
	pub fn download_verified(&mut self) -> Result<Option<VerifiedDownload>, CodePushError> {
		match self.check()? {
			UpdatePlan::UpToDate => Ok(None),
			UpdatePlan::Available(offer) => self.download_offer(*offer).map(Some),
		}
	}

	/// Downloads and authenticates the container of an offer [`Self::check`]
	/// returned, so already verified.
	pub(crate) fn download_offer(
		&mut self,
		offer: UpdateOffer,
	) -> Result<VerifiedDownload, CodePushError> {
		let delta_bytes = match (offer.delta_url.as_ref(), offer.delta_base_hash.as_ref()) {
			(Some(delta_url), Some(base_hash)) => self.fetch_delta(delta_url, base_hash).ok(),
			_ => None,
		};
		let used_delta = delta_bytes.is_some();
		let container_bytes = if let Some(bytes) = delta_bytes {
			bytes
		} else {
			let bytes = self.transport.fetch(&self.resolve(&offer.download_url))?;
			self.within_limit("container", bytes.len())?;
			bytes
		};

		let (manifest, _) = crate::manifest::container_manifest(&container_bytes)?;
		if manifest.bundle_hash() != offer.manifest.bundle_hash() {
			return Err(CodePushError::HashMismatch {
				path: "<container>".to_owned(),
				expected: offer.manifest.bundle_hash(),
				actual: manifest.bundle_hash(),
			});
		}
		let parsed = read_container(&container_bytes)?;
		let signature = offer
			.signature
			.ok_or_else(|| CodePushError::Unsigned(parsed.manifest.bundle_hash()))?;

		Ok(VerifiedDownload {
			container: container_bytes,
			manifest: parsed.manifest,
			signature,
			used_delta,
		})
	}

	/// Fails when `len` bytes are over [`Self::with_max_container_bytes`].
	fn within_limit(&self, what: &str, len: usize) -> Result<(), CodePushError> {
		if len as u64 > self.max_container_bytes {
			return Err(CodePushError::Malformed(format!(
				"the {what} is {len} bytes, over the {} byte limit",
				self.max_container_bytes
			)));
		}
		Ok(())
	}

	/// Fetches the offered delta and applies it to the running container,
	/// returning the patched container bytes.
	fn fetch_delta(&mut self, delta_url: &str, base_hash: &str) -> Result<Vec<u8>, CodePushError> {
		let delta_bytes = self.transport.fetch(&self.resolve(delta_url))?;
		self.within_limit("delta", delta_bytes.len())?;
		let delta: crate::delta::Delta = serde_json::from_slice(&delta_bytes)?;
		let current_hash = hash_bytes(&self.current_container);
		if delta.base_hash != base_hash || delta.base_hash != current_hash {
			return Err(CodePushError::DeltaBaseMismatch {
				expected: base_hash.to_owned(),
				actual: current_hash,
			});
		}
		crate::delta::apply_delta_within(&self.current_container, &delta, self.max_container_bytes)
	}

	/// Resolves an offer URL: they are path-absolute (`/app/channel/...`),
	/// resolved against the registry base rather than fetched on their own.
	fn resolve(&self, url: &str) -> String {
		format!("{}/{}", self.base_url, url.trim_start_matches('/'))
	}

	/// The URL of a file in the registry.
	fn url(&self, key: &ObjectKey) -> String {
		format!("{}/{}", self.base_url, key.url_path())
	}

	/// The version the client now tracks.
	pub fn current_version(&self) -> &Version {
		&self.current_version
	}
}

/// Replaces `staging_dir` with a parsed container's files.
///
/// The files are written and synced in a temporary sibling, which then takes
/// the place of the previous staging. Until that rename, the previous staging
/// is untouched; if the rename fails, the previous staging is moved back.
fn stage(staging_dir: &Path, container: &crate::manifest::Container) -> Result<(), CodePushError> {
	let parent = durable::parent_of(staging_dir);
	std::fs::create_dir_all(parent)?;
	let files: Vec<(&str, &[u8])> = container
		.files
		.iter()
		.map(|(path, content)| (path.as_str(), content.as_slice()))
		.collect();
	let staged = durable::assemble_directory(staging_dir, &files)?;
	let previous = if staging_dir.try_exists()? {
		let previous = durable::temporary_sibling(staging_dir)?;
		if let Err(error) = std::fs::rename(staging_dir, &previous) {
			// The rename already failed; that error is the one to report.
			let _ = std::fs::remove_dir_all(&staged);
			return Err(error.into());
		}
		Some(previous)
	} else {
		None
	};
	if let Err(error) = std::fs::rename(&staged, staging_dir) {
		// Put the previous staging back; the failed rename is what to report.
		if let Some(previous) = &previous {
			let _ = std::fs::rename(previous, staging_dir);
		}
		let _ = std::fs::remove_dir_all(&staged);
		return Err(error.into());
	}
	durable::sync_directory(parent)?;
	if let Some(previous) = previous {
		std::fs::remove_dir_all(previous)?;
	}
	Ok(())
}

impl ReleaseStore {
	/// Computes the update plan for a client, for the dynamic `check` route.
	///
	/// Reads only the channel index and publisher metadata. Release and delta
	/// bodies are never read while computing an offer.
	pub fn plan_update(
		&self,
		app: &str,
		channel: &str,
		current: &Version,
		current_build: u64,
	) -> Result<UpdatePlan, crate::store::StoreError> {
		let Some(index) = self.index(app, channel)? else {
			return Ok(UpdatePlan::UpToDate);
		};
		let latest = index.manifest;
		let newer = latest.version > *current
			|| (latest.version == *current && latest.build > current_build);
		if !newer {
			return Ok(UpdatePlan::UpToDate);
		}
		// A delta applies only when the client runs exactly the release the
		// delta was computed against, matched by container hash rather than
		// by version: out-of-order ingests and rebuilt versions mean the
		// previous release is not necessarily the delta's base.
		let current_hash = self.container_hash(app, channel, current, current_build)?;
		let delta_key = current_hash
			.as_ref()
			.and_then(|hash| ObjectKey::delta(app, channel, &latest.version, latest.build, hash));
		let has_delta = match delta_key {
			Some(key) => self.object_exists(&key)?,
			None => false,
		};
		let (app, channel) = (encode_component(app), encode_component(channel));
		Ok(UpdatePlan::Available(Box::new(UpdateOffer {
			download_url: format!(
				"/{app}/{channel}/download/{}/{}",
				encode_component(&latest.version.to_string()),
				latest.build
			),
			delta_url: has_delta.then(|| {
				format!(
					"/{app}/{channel}/delta/{}/{current_build}",
					encode_component(&current.to_string())
				)
			}),
			delta_base_hash: current_hash.filter(|_| has_delta),
			manifest: latest,
			signature: index.signature,
		})))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::manifest::Container;
	use crate::manifest::MANIFEST_FORMAT_VERSION;

	/// A parsed container holding `files`; staging reads nothing else.
	fn container(files: &[(&str, &[u8])]) -> Container {
		Container {
			manifest: BundleManifest {
				format_version: MANIFEST_FORMAT_VERSION,
				app: "app".to_owned(),
				channel: "production".to_owned(),
				version: Version::new(1, 0, 0),
				build: 1,
				entry: "app.wasm".to_owned(),
				files: Vec::new(),
				created_unix: 0,
				mandatory: false,
			},
			files: files
				.iter()
				.map(|(path, content)| ((*path).to_owned(), content.to_vec()))
				.collect(),
		}
	}

	/// Every file under `directory`, as a `/`-separated relative path and its
	/// contents, sorted.
	fn tree(directory: &Path) -> Vec<(String, Vec<u8>)> {
		walkdir::WalkDir::new(directory)
			.sort_by_file_name()
			.into_iter()
			.map(|entry| entry.expect("walk"))
			.filter(|entry| entry.file_type().is_file())
			.map(|entry| {
				let relative = entry
					.path()
					.strip_prefix(directory)
					.expect("under the directory")
					.to_string_lossy()
					.replace('\\', "/");
				(relative, std::fs::read(entry.path()).expect("read"))
			})
			.collect()
	}

	/// The names in `directory`, sorted.
	fn names(directory: &Path) -> Vec<String> {
		let mut names: Vec<_> = std::fs::read_dir(directory)
			.expect("read directory")
			.map(|entry| {
				entry
					.expect("entry")
					.file_name()
					.to_string_lossy()
					.into_owned()
			})
			.collect();
		names.sort();
		names
	}

	#[test]
	fn staging_replaces_the_previous_release_whole() {
		let parent = tempfile::tempdir().expect("parent");
		let staging = parent.path().join("staged");
		stage(
			&staging,
			&container(&[("app.wasm", b"v1"), ("assets/old.txt", b"not in v2")]),
		)
		.expect("stage v1");
		stage(&staging, &container(&[("app.wasm", b"v2")])).expect("stage v2");
		assert_eq!(tree(&staging), [("app.wasm".to_owned(), b"v2".to_vec())]);
		assert_eq!(
			names(parent.path()),
			["staged"],
			"nothing temporary survives"
		);
	}

	#[test]
	fn a_failed_stage_leaves_the_previous_staging_intact() {
		let parent = tempfile::tempdir().expect("parent");
		let staging = parent.path().join("staged");
		let v1 = [("app.wasm", b"v1".as_slice()), ("assets/logo.png", b"logo")];
		stage(&staging, &container(&v1)).expect("stage v1");
		// `assets` cannot be a file and a directory at once.
		let broken = container(&[
			("app.wasm", b"v2"),
			("assets", b"a file"),
			("assets/logo.png", b"new"),
		]);
		assert!(stage(&staging, &broken).is_err());
		assert_eq!(
			tree(&staging),
			v1.map(|(path, content)| (path.to_owned(), content.to_vec()))
		);
		assert_eq!(
			names(parent.path()),
			["staged"],
			"nothing temporary survives"
		);
	}
}
