//! The updater client: check, download (full or delta), verify, stage.
//!
//! The transport is a trait so each platform shell brings its own HTTP stack
//! — `URLSession` on iOS, `OkHttp` on Android, `reqwest` on desktop — and
//! everything else (the protocol, the verification, the staging) is shared.

use std::path::Path;
use std::path::PathBuf;

use semver::Version;
use serde::Deserialize;
use serde::Serialize;

use crate::CodePushError;
use crate::manifest::hash_bytes;
use crate::manifest::read_container;
use crate::store::ReleaseStore;

/// How a client fetches bytes from the release server.
pub trait Transport {
	/// Fetches `url`, returning the body.
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError>;
}

/// A transport that reads from the filesystem, for tests and local channels.
#[derive(Debug, Default)]
pub struct FileTransport;

impl Transport for FileTransport {
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
		let path = url.strip_prefix("file://").unwrap_or(url);
		std::fs::read(path)
			.map_err(|error| CodePushError::Transport(format!("could not read {path}: {error}")))
	}
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
	pub manifest: crate::manifest::BundleManifest,
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
}

impl<T: Transport> CodePushClient<T> {
	/// Creates a client tracking `base_url` for an app and channel.
	pub fn new(
		transport: T,
		base_url: impl Into<String>,
		app: impl Into<String>,
		channel: impl Into<String>,
		current_version: Version,
		current_build: u64,
		current_container: Vec<u8>,
	) -> Self {
		Self {
			transport,
			base_url: base_url.into().trim_end_matches('/').to_owned(),
			app: app.into(),
			channel: channel.into(),
			current_version,
			current_build,
			current_container,
		}
	}

	/// Asks the server for an update plan.
	pub fn check(&mut self) -> Result<UpdatePlan, CodePushError> {
		let url = format!(
			"{}/{}/{}/check?version={}&build={}",
			self.base_url, self.app, self.channel, self.current_version, self.current_build
		);
		let body = self.transport.fetch(&url)?;
		serde_json::from_slice(&body).map_err(CodePushError::Serialize)
	}

	/// Runs one full sync: check, download, verify, stage into `staging_dir`.
	pub fn sync(&mut self, staging_dir: &Path) -> Result<SyncReport, CodePushError> {
		let plan = self.check()?;
		let UpdatePlan::Available(offer) = plan else {
			return Ok(SyncReport {
				updated: false,
				version: None,
				used_delta: false,
			});
		};

		// The offer's URLs are path-absolute (`/app/channel/...`), resolved
		// against the registry base rather than fetched on their own.
		let resolve = |url: &str| -> String {
			format!(
				"{}/{}",
				self.base_url.trim_end_matches('/'),
				url.trim_start_matches('/')
			)
		};

		let mut used_delta = false;
		let container_bytes = if let (Some(delta_url), Some(base_hash)) =
			(offer.delta_url.as_ref(), offer.delta_base_hash.as_ref())
		{
			let delta_bytes = self.transport.fetch(&resolve(delta_url))?;
			let delta: crate::delta::Delta = serde_json::from_slice(&delta_bytes)?;
			let current_hash = hash_bytes(&self.current_container);
			if delta.base_hash != *base_hash || delta.base_hash != current_hash {
				return Err(CodePushError::DeltaBaseMismatch {
					expected: base_hash.clone(),
					actual: hash_bytes(&self.current_container),
				});
			}
			let patched = crate::delta::apply_delta(&self.current_container, &delta)?;
			used_delta = true;
			patched
		} else {
			self.transport.fetch(&resolve(&offer.download_url))?
		};

		let parsed = read_container(&container_bytes)?;
		if parsed.manifest.bundle_hash() != offer.manifest.bundle_hash() {
			return Err(CodePushError::HashMismatch {
				path: "<container>".to_owned(),
				expected: offer.manifest.bundle_hash(),
				actual: parsed.manifest.bundle_hash(),
			});
		}
		stage(staging_dir, &parsed)?;
		self.current_container = container_bytes;
		self.current_version = parsed.manifest.version.clone();
		self.current_build = parsed.manifest.build;

		Ok(SyncReport {
			updated: true,
			version: Some(parsed.manifest.version),
			used_delta,
		})
	}

	/// The version the client now tracks.
	pub fn current_version(&self) -> &Version {
		&self.current_version
	}
}

/// Writes a parsed container's files into a staging directory.
fn stage(
	staging_dir: &Path,
	container: &crate::manifest::Container,
) -> Result<PathBuf, CodePushError> {
	std::fs::create_dir_all(staging_dir)?;
	for (path, content) in &container.files {
		let target = staging_dir.join(path);
		if let Some(parent) = target.parent() {
			std::fs::create_dir_all(parent)?;
		}
		std::fs::write(&target, content)?;
	}
	Ok(staging_dir.to_path_buf())
}

impl ReleaseStore {
	/// Computes the update plan for a client.
	pub fn plan_update(
		&self,
		app: &str,
		channel: &str,
		current: &Version,
		current_build: u64,
	) -> Result<UpdatePlan, crate::store::StoreError> {
		let Some(latest) = self.latest(app, channel)? else {
			return Ok(UpdatePlan::UpToDate);
		};
		let newer = latest.manifest.version > *current
			|| (latest.manifest.version == *current && latest.manifest.build > current_build);
		if !newer {
			return Ok(UpdatePlan::UpToDate);
		}
		// A delta applies only when the client runs exactly the release the
		// delta was computed against: the previous one on the channel.
		let records = self.list(app, channel)?;
		let previous = records.get(1);
		let runs_previous = previous.is_some_and(|record| {
			record.manifest.version == *current && record.manifest.build == current_build
		});
		let delta = latest
			.delta_from_previous
			.as_ref()
			.filter(|_| runs_previous);
		Ok(UpdatePlan::Available(Box::new(UpdateOffer {
			manifest: latest.manifest.clone(),
			download_url: format!(
				"/{app}/{channel}/download/{}/{}",
				latest.manifest.version, latest.manifest.build
			),
			delta_url: delta.map(|_| format!("/{app}/{channel}/delta/{current}/{current_build}")),
			delta_base_hash: delta.map(|delta| delta.base_hash.clone()),
		})))
	}
}
