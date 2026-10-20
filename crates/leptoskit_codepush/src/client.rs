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
			self.base_url,
			encode_component(&self.app),
			encode_component(&self.channel),
			encode_component(&self.current_version.to_string()),
			self.current_build
		);
		let body = self.transport.fetch(&url)?;
		serde_json::from_slice(&body).map_err(CodePushError::Serialize)
	}

	/// Runs one full sync: check, download, verify, stage into `staging_dir`.
	///
	/// When the offer includes a delta, the delta is tried first; if fetching,
	/// parsing, or applying it fails for any reason (missing on the server,
	/// stale base, corrupt bytes), the sync falls back to the full download and
	/// reports `used_delta: false`. Failures of the full download or of
	/// container verification are returned as errors.
	pub fn sync(&mut self, staging_dir: &Path) -> Result<SyncReport, CodePushError> {
		let plan = self.check()?;
		let UpdatePlan::Available(offer) = plan else {
			return Ok(SyncReport {
				updated: false,
				version: None,
				used_delta: false,
			});
		};

		let delta_bytes = match (offer.delta_url.as_ref(), offer.delta_base_hash.as_ref()) {
			(Some(delta_url), Some(base_hash)) => self.fetch_delta(delta_url, base_hash).ok(),
			_ => None,
		};
		let used_delta = delta_bytes.is_some();
		let container_bytes = match delta_bytes {
			Some(bytes) => bytes,
			None => self.transport.fetch(&self.resolve(&offer.download_url))?,
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

	/// Fetches the offered delta and applies it to the running container,
	/// returning the patched container bytes.
	fn fetch_delta(&mut self, delta_url: &str, base_hash: &str) -> Result<Vec<u8>, CodePushError> {
		let delta_bytes = self.transport.fetch(&self.resolve(delta_url))?;
		let delta: crate::delta::Delta = serde_json::from_slice(&delta_bytes)?;
		let current_hash = hash_bytes(&self.current_container);
		if delta.base_hash != base_hash || delta.base_hash != current_hash {
			return Err(CodePushError::DeltaBaseMismatch {
				expected: base_hash.to_owned(),
				actual: current_hash,
			});
		}
		crate::delta::apply_delta(&self.current_container, &delta)
	}

	/// Resolves an offer URL: they are path-absolute (`/app/channel/...`),
	/// resolved against the registry base rather than fetched on their own.
	fn resolve(&self, url: &str) -> String {
		format!("{}/{}", self.base_url, url.trim_start_matches('/'))
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
		let records = self.list(app, channel)?;
		let Some(latest) = records.first() else {
			return Ok(UpdatePlan::UpToDate);
		};
		let newer = latest.manifest.version > *current
			|| (latest.manifest.version == *current && latest.manifest.build > current_build);
		if !newer {
			return Ok(UpdatePlan::UpToDate);
		}
		// A delta applies only when the client runs exactly the release the
		// delta was computed against, matched by container hash rather than
		// by list position: out-of-order ingests and rebuilt versions mean
		// the previous record is not necessarily the delta's base.
		let current_hash = records
			.iter()
			.find(|record| {
				record.manifest.version == *current && record.manifest.build == current_build
			})
			.map(|record| hash_bytes(&record.container));
		let delta = latest
			.delta_from_previous
			.as_ref()
			.filter(|delta| current_hash.as_deref() == Some(delta.base_hash.as_str()));
		let (app, channel) = (encode_component(app), encode_component(channel));
		Ok(UpdatePlan::Available(Box::new(UpdateOffer {
			manifest: latest.manifest.clone(),
			download_url: format!(
				"/{app}/{channel}/download/{}/{}",
				encode_component(&latest.manifest.version.to_string()),
				latest.manifest.build
			),
			delta_url: delta.map(|_| {
				format!(
					"/{app}/{channel}/delta/{}/{current_build}",
					encode_component(&current.to_string())
				)
			}),
			delta_base_hash: delta.map(|delta| delta.base_hash.clone()),
		})))
	}
}

/// Percent-encodes one URL path or query component: everything except the
/// RFC 3986 unreserved characters is escaped, so a version like `1.0.0+abc`
/// survives query decoding instead of turning into `1.0.0 abc`.
fn encode_component(component: &str) -> String {
	const HEX: &[u8; 16] = b"0123456789ABCDEF";
	let mut encoded = String::with_capacity(component.len());
	for byte in component.bytes() {
		if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
			encoded.push(char::from(byte));
		} else {
			encoded.push('%');
			encoded.push(char::from(HEX[usize::from(byte >> 4)]));
			encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
		}
	}
	encoded
}
