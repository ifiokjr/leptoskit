//! Durable native bundle selection. A downloaded release is a trial until the
//! host confirms its first attached frame. Use one owner on a serialized worker
//! per directory; no UI/interpreter pointer belongs on that worker.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use semver::Version;
use serde::Deserialize;
use serde::Serialize;

use crate::BundleManifest;
use crate::CodePushClient;
use crate::CodePushError;
use crate::ReleaseSignature;
use crate::Transport;
use crate::TrustedKeys;
use crate::manifest::Container;
use crate::read_container;

/// Maximum HTTP response and decompressed native bundle size (32 MiB).
pub const MAX_BUNDLE_BYTES: u64 = 32 * 1024 * 1024;

/// Authenticated native requirements in the bundle's `runtime.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeContract {
	/// Native guest ABI revision.
	pub abi: u32,
	/// Shell compatibility identity, changed when native capabilities change.
	pub shell: String,
	/// Required plugin names; the host may support additional plugins.
	pub plugins: Vec<String>,
}

/// Store-shell configuration. Downloaded code cannot replace these trust pins.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfiguration {
	/// Whether registry access is enabled.
	pub enabled: bool,
	/// Expected app identity.
	pub app: String,
	/// Expected release channel.
	pub channel: String,
	/// Registry base URL.
	pub server: String,
	/// Trusted ed25519 public keys, hex encoded.
	pub public_keys: Vec<String>,
	/// ABI, shell identity, and plugins supported by the store shell.
	pub runtime: RuntimeContract,
	/// Explicit development-only permission to use cleartext HTTP.
	pub allow_http: bool,
}

/// Executable bytes and a generation-bound activation receipt.
#[derive(Debug)]
pub struct LaunchBundle {
	/// The manifest's verified entry module.
	pub wasm: Vec<u8>,
	/// Identity returned to the native worker for confirmation/rejection.
	pub metadata: LaunchMetadata,
}

/// Metadata is separate from Wasm bytes across native FFI boundaries.
#[derive(Debug, Clone, Serialize)]
pub struct LaunchMetadata {
	/// Opaque receipt; callbacks from older launches cannot confirm a new one.
	pub receipt: String,
	/// Selected release version.
	pub version: Version,
	/// Selected release build.
	pub build: u64,
	/// True until this downloaded release receives its first healthy frame.
	pub candidate: bool,
}

/// Result of checking and staging, without activating the guest.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateReport {
	/// Whether a compatible update is ready to launch.
	pub status: UpdateStatus,
	/// Current or staged release version.
	pub version: Version,
	/// Current or staged release build.
	pub build: u64,
}

/// Serialized as `current` or `staged` for both mobile hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
	/// No newer usable release is staged.
	Current,
	/// A verified candidate is ready for explicit activation.
	Staged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
	format: u32,
	embedded: String,
	confirmed: Option<String>,
	candidate: Option<Candidate>,
	generation: u64,
	rejected: Vec<String>,
	launch: Option<Receipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Candidate {
	Ready { release: String },
	Trying { release: String },
}

impl Candidate {
	fn release(&self) -> &str {
		match self {
			Self::Ready { release } | Self::Trying { release } => release,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
	id: String,
	release: Option<String>,
	resolution: Resolution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Resolution {
	Pending,
	Confirmed,
	Rejected,
}

/// Owns durable selection and authenticated offline releases. Methods block;
/// callers must keep this owner on their serialized background worker.
pub struct InstalledBundles {
	directory: PathBuf,
	embedded: Vec<u8>,
	config: UpdateConfiguration,
	keys: Option<TrustedKeys>,
	state: State,
	recovery: Option<String>,
	// The old native screen remains alive until the confirmation result
	// returns. Permit that one live handoff to reject even if its disk commit
	// won the race.
	just_confirmed: Option<(String, Option<String>)>,
}

impl InstalledBundles {
	/// Opens installed state, rejects interrupted trials, and revalidates
	/// stored artifacts. Corrupt/incompatible installed state recovers to the
	/// trusted store baseline and exposes a diagnostic through
	/// `recovery_diagnostic`.
	pub fn open(
		directory: &Path,
		embedded: &[u8],
		config: UpdateConfiguration,
	) -> Result<Self, CodePushError> {
		let keys = if config.enabled || !config.public_keys.is_empty() {
			Some(TrustedKeys::from_hex(&config.public_keys)?)
		} else {
			None
		};
		let baseline = validate_container(embedded, &config)?;
		let baseline_hash = baseline.manifest.bundle_hash();
		fs::create_dir_all(directory.join("releases"))?;
		let initial = || {
			State {
				format: 1,
				embedded: baseline_hash.clone(),
				confirmed: None,
				candidate: None,
				generation: 0,
				rejected: Vec::new(),
				launch: None,
			}
		};
		let mut recovery = None;
		let state = match fs::read(directory.join("state.json")) {
			Ok(bytes) => {
				match serde_json::from_slice::<State>(&bytes) {
					Ok(state) if state.format == 1 => state,
					parsed => {
						recovery =
							Some(format!("discarded invalid installed selector: {parsed:?}"));
						initial()
					}
				}
			}
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => initial(),
			Err(error) => return Err(error.into()),
		};
		let mut installed = Self {
			directory: directory.to_owned(),
			embedded: embedded.to_vec(),
			config,
			keys,
			state,
			recovery,
			just_confirmed: None,
		};
		// A new store build is the newest trusted code even when it keeps the
		// app version: downloads made for the previous build no longer apply.
		if installed.state.embedded != baseline_hash {
			installed.state.confirmed = None;
			installed.state.candidate = None;
			installed.state.launch = None;
		}
		installed.recover(&baseline.manifest);
		installed.state.embedded = baseline_hash;
		installed.persist()?;
		installed.prune()?;

		Ok(installed)
	}

	/// Last recovery reason, retained for host status/logging after
	/// open/launch.
	pub fn recovery_diagnostic(&self) -> Option<&str> {
		self.recovery.as_deref()
	}

	/// Revalidates executable identity and durably marks a trial before handing
	/// bytes to the host. Starting another launch abandons an unconfirmed
	/// trial.
	///
	/// `open` and `sync_with` already ordered the candidate above the
	/// confirmed release and that above the store baseline, so only the release
	/// that runs is read, verified, and decompressed again. If it is damaged,
	/// the next one down runs instead.
	pub fn launch(&mut self) -> Result<LaunchBundle, CodePushError> {
		self.just_confirmed = None;
		if let Some(Candidate::Trying { release }) = self.state.candidate.clone() {
			self.recovery = Some("unconfirmed trial recovered to working release".to_owned());
			self.reject_release(release);
		}
		let (release, parsed) = loop {
			let release = self
				.state
				.candidate
				.as_ref()
				.map(|candidate| candidate.release().to_owned())
				.or_else(|| self.state.confirmed.clone());
			// The store baseline was verified at `open`; only stored releases
			// can have been damaged since.
			match (self.load(release.as_deref()), release) {
				(Err(error), Some(damaged)) => self.forget_damaged(&damaged, &error),
				(loaded, release) => break (release, loaded?.1),
			}
		};
		let candidate = self.state.candidate.is_some();
		let wasm = entry_of(&parsed)?.to_vec();
		self.state.generation = self
			.state
			.generation
			.checked_add(1)
			.ok_or_else(|| activation("launch generation exhausted"))?;
		let receipt = format!(
			"{}:{}:{}",
			self.state.embedded,
			self.state.generation,
			parsed.manifest.bundle_hash()
		);
		self.state.launch = Some(Receipt {
			id: receipt.clone(),
			release: release.clone(),
			resolution: Resolution::Pending,
		});

		if candidate {
			self.state.candidate = release.map(|release| Candidate::Trying { release });
		}

		self.persist()?;

		Ok(LaunchBundle {
			wasm,
			metadata: LaunchMetadata {
				receipt,
				version: parsed.manifest.version,
				build: parsed.manifest.build,
				candidate,
			},
		})
	}

	/// Promotes only this generation after native first-frame success.
	pub fn confirm(&mut self, receipt: &str) -> Result<(), CodePushError> {
		let launch = self.receipt(receipt, Resolution::Confirmed)?;

		if launch.resolution == Resolution::Confirmed {
			return Ok(());
		}

		self.load(launch.release.as_deref())?;
		let release = launch.release.clone();
		let confirms_trial = self.state.candidate.as_ref().is_some_and(|candidate| {
			matches!(candidate, Candidate::Trying { .. })
				&& release.as_deref() == Some(candidate.release())
		});

		if confirms_trial {
			self.just_confirmed = Some((receipt.to_owned(), self.state.confirmed.clone()));
			self.state.confirmed = release;
			self.state.candidate = None;
		}
		self.state
			.launch
			.as_mut()
			.expect("receipt checked")
			.resolution = Resolution::Confirmed;
		self.persist()
	}

	/// Rejects a failed trial while preserving the last working release. The
	/// caller then invokes `launch` to obtain the verified fallback.
	pub fn reject(&mut self, receipt: &str) -> Result<(), CodePushError> {
		if self
			.just_confirmed
			.as_ref()
			.is_some_and(|(id, _)| id == receipt)
		{
			let launch = self.receipt(receipt, Resolution::Confirmed)?;
			let release = launch
				.release
				.clone()
				.ok_or_else(|| activation("confirmed trial has no release"))?;
			let (_, previous) = self
				.just_confirmed
				.take()
				.expect("matching live confirmation");
			self.state.confirmed = previous;
			self.reject_release(release);
			self.state
				.launch
				.as_mut()
				.expect("receipt checked")
				.resolution = Resolution::Rejected;

			return self.persist();
		}

		let launch = self.receipt(receipt, Resolution::Rejected)?;

		if launch.resolution == Resolution::Rejected {
			return Ok(());
		}

		let release = launch.release.clone();
		let is_trial = self
			.state
			.candidate
			.as_ref()
			.is_some_and(|candidate| release.as_deref() == Some(candidate.release()));

		if !is_trial {
			return Err(activation(
				"the working baseline failed; no earlier trial to recover",
			));
		}

		if let Some(release) = release {
			self.reject_release(release);
		}

		self.state
			.launch
			.as_mut()
			.expect("receipt checked")
			.resolution = Resolution::Rejected;
		self.persist()
	}

	/// Checks through the bounded HTTPS transport. Run exclusively off the UI
	/// thread. Disabled configurations never contact the registry.
	#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
	pub fn sync(&mut self) -> Result<UpdateReport, CodePushError> {
		let mut transport = HttpTransport::new(self.config.allow_http);
		self.sync_with(&mut transport)
	}

	/// Checks via an injected transport, sharing all trust and activation
	/// rules. Downloads remain candidates until explicitly launched and
	/// confirmed.
	pub fn sync_with(
		&mut self,
		transport: &mut impl Transport,
	) -> Result<UpdateReport, CodePushError> {
		self.just_confirmed = None;
		if let Some(Candidate::Ready { release }) = &self.state.candidate {
			let (_, parsed) = self.load(Some(release))?;
			return Ok(report(UpdateStatus::Staged, parsed.manifest));
		}

		let (current_bytes, current) = self.load(self.state.confirmed.as_deref())?;

		if !self.config.enabled {
			return Ok(report(UpdateStatus::Current, current.manifest));
		}

		if matches!(self.state.candidate, Some(Candidate::Trying { .. })) {
			return Err(activation(
				"confirm or reject the outstanding trial before checking",
			));
		}
		self.prune()?;

		let keys = self
			.keys
			.clone()
			.ok_or_else(|| activation("no update trust keys"))?;
		let mut client = CodePushClient::new(
			transport,
			&self.config.server,
			&self.config.app,
			&self.config.channel,
			current.manifest.version.clone(),
			current.manifest.build,
			current_bytes,
			keys,
		)
		.with_max_container_bytes(MAX_BUNDLE_BYTES);
		let Some(download) = client.download_verified()? else {
			return Ok(report(UpdateStatus::Current, current.manifest));
		};
		let parsed = validate_container(&download.container, &self.config)?;
		let id = parsed.manifest.bundle_hash();

		if self.state.rejected.contains(&id) {
			return Err(activation("this release previously failed activation"));
		}

		let release_dir = self.release_directory(&id)?;
		fs::create_dir_all(&release_dir)?;
		atomic_write(&release_dir.join("release.lkb"), &download.container)?;
		atomic_write(
			&release_dir.join("release.sig"),
			&serde_json::to_vec(&download.signature)?,
		)?;
		sync_directory(&self.directory.join("releases"))?;
		self.state.candidate = Some(Candidate::Ready { release: id });
		self.persist()?;

		Ok(report(UpdateStatus::Staged, parsed.manifest))
	}

	fn receipt(&self, id: &str, resolution: Resolution) -> Result<&Receipt, CodePushError> {
		let receipt = self
			.state
			.launch
			.as_ref()
			.filter(|launch| launch.id == id)
			.ok_or_else(|| activation("stale launch receipt"))?;

		if receipt.resolution != Resolution::Pending && receipt.resolution != resolution {
			return Err(activation("launch receipt already resolved differently"));
		}

		Ok(receipt)
	}

	fn reject_release(&mut self, release: String) {
		if !self.state.rejected.contains(&release) {
			self.state.rejected.push(release);
		}

		self.state.candidate = None;
	}

	/// Drops a stored release that failed verification at launch: a damaged
	/// candidate is rejected, a damaged confirmed release falls back to the
	/// store baseline.
	fn forget_damaged(&mut self, release: &str, error: &CodePushError) {
		self.recovery = Some(format!("installed release rejected: {error}"));
		let is_candidate = self
			.state
			.candidate
			.as_ref()
			.is_some_and(|candidate| candidate.release() == release);
		if is_candidate {
			self.reject_release(release.to_owned());
		} else {
			self.state.confirmed = None;
		}
	}

	fn recover(&mut self, baseline: &BundleManifest) {
		if let Some(Candidate::Trying { release }) = self.state.candidate.clone() {
			self.recovery = Some("unconfirmed trial recovered to working release".to_owned());
			self.reject_release(release);
			self.state.launch = None;
		}

		// The release a candidate must beat; each stored release is read once.
		let mut working = baseline.clone();
		if let Some(id) = self.state.confirmed.clone() {
			match self.load(Some(&id)) {
				Ok((_, parsed)) if release_order(&parsed.manifest) > release_order(baseline) => {
					working = parsed.manifest;
				}
				Ok(_) => {
					self.state.confirmed = None;
					self.state.launch = None;
				}
				Err(error) => {
					self.recovery = Some(format!("installed release rejected: {error}"));
					self.state.confirmed = None;
					self.state.launch = None;
				}
			}
		}

		if let Some(candidate) = self.state.candidate.clone() {
			match self.load(Some(candidate.release())) {
				Ok((_, parsed)) if release_order(&parsed.manifest) > release_order(&working) => {}
				Ok(_) => self.state.candidate = None,
				Err(error) => {
					self.recovery = Some(format!("staged release rejected: {error}"));
					self.reject_release(candidate.release().to_owned());
				}
			}
		}
	}

	fn release_directory(&self, id: &str) -> Result<PathBuf, CodePushError> {
		if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
			return Err(activation("invalid installed release identifier"));
		}

		Ok(self.directory.join("releases").join(id))
	}

	fn load(&self, id: Option<&str>) -> Result<(Vec<u8>, Container), CodePushError> {
		let Some(id) = id else {
			return Ok((
				self.embedded.clone(),
				validate_container(&self.embedded, &self.config)?,
			));
		};
		let directory = self.release_directory(id)?;
		let bytes = fs::read(directory.join("release.lkb"))?;
		let parsed = validate_container(&bytes, &self.config)?;
		let signature: ReleaseSignature =
			serde_json::from_slice(&fs::read(directory.join("release.sig"))?)?;
		self.keys
			.as_ref()
			.ok_or_else(|| activation("installed release has no trusted key"))?
			.verify(&parsed.manifest, Some(&signature))?;

		if parsed.manifest.bundle_hash() != id {
			return Err(activation(
				"installed release identifier does not match signed container",
			));
		}

		Ok((bytes, parsed))
	}

	/// Removes stored releases that can never run again: everything except the
	/// confirmed release and the staged or trying candidate. Callers run it
	/// only when no live rollback still needs the previously confirmed release.
	fn prune(&self) -> Result<(), CodePushError> {
		let keep = [
			self.state.confirmed.as_deref(),
			self.state.candidate.as_ref().map(Candidate::release),
		];
		for entry in fs::read_dir(self.directory.join("releases"))? {
			let entry = entry?;
			if keep.contains(&entry.file_name().to_str()) {
				continue;
			}
			if entry.file_type()?.is_dir() {
				fs::remove_dir_all(entry.path())?;
			} else {
				fs::remove_file(entry.path())?;
			}
		}
		Ok(())
	}

	fn persist(&self) -> Result<(), CodePushError> {
		atomic_write(
			&self.directory.join("state.json"),
			&serde_json::to_vec(&self.state)?,
		)
	}
}

fn release_order(manifest: &BundleManifest) -> (&Version, u64) {
	(&manifest.version, manifest.build)
}

fn report(status: UpdateStatus, manifest: BundleManifest) -> UpdateReport {
	UpdateReport {
		status,
		version: manifest.version,
		build: manifest.build,
	}
}

fn activation(message: impl Into<String>) -> CodePushError {
	CodePushError::Activation(message.into())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), CodePushError> {
	let name = path
		.file_name()
		.ok_or_else(|| activation("missing file name"))?;
	let mut temporary_name = name.to_owned();
	temporary_name.push(".pending");
	let temporary = path.with_file_name(temporary_name);
	let written = fs::File::create(&temporary).and_then(|mut file| {
		file.write_all(bytes)?;
		file.sync_all()
	});
	if let Err(error) = written.and_then(|()| fs::rename(&temporary, path)) {
		// Leave no partial file behind, or a full disk stays full.
		let _ = fs::remove_file(&temporary);
		return Err(error.into());
	}
	sync_directory(
		path.parent()
			.ok_or_else(|| activation("missing parent directory"))?,
	)
}

/// Makes a rename or new entry in `directory` durable. Unix needs an fsync of
/// the directory itself; Windows cannot open a directory as a file through
/// `std`, and NTFS journals the metadata change.
fn sync_directory(directory: &Path) -> Result<(), CodePushError> {
	if cfg!(unix) {
		fs::File::open(directory)?.sync_all()?;
	}
	Ok(())
}

/// The verified entry module of a trusted store-shipped container, for a host
/// that runs it without the update store: updates are disabled, or the store
/// cannot open. Every file hash is checked and the same size bounds apply.
pub fn entry_module(container: &[u8]) -> Result<Vec<u8>, CodePushError> {
	let parsed = parse_bounded(container)?;
	Ok(entry_of(&parsed)?.to_vec())
}

/// Parses a container after checking its size and declared expansion, so a
/// hostile manifest cannot make the parser allocate past the native bound.
fn parse_bounded(bytes: &[u8]) -> Result<Container, CodePushError> {
	if bytes.len() as u64 > MAX_BUNDLE_BYTES {
		return Err(activation("container exceeds 32 MiB"));
	}

	// Inspect declared expansion before the shared parser allocates file
	// contents.
	let (manifest, _) = crate::manifest::container_manifest(bytes)?;
	let total = manifest
		.files
		.iter()
		.try_fold(0_u64, |sum, file| sum.checked_add(file.size));

	if total.is_none_or(|size| size > MAX_BUNDLE_BYTES) {
		return Err(activation("expanded bundle exceeds 32 MiB"));
	}

	read_container(bytes)
}

/// The bundle's entry module, which must be WebAssembly version 1.
fn entry_of(parsed: &Container) -> Result<&[u8], CodePushError> {
	let entry = parsed
		.files
		.iter()
		.find(|(path, _)| path == &parsed.manifest.entry)
		.ok_or_else(|| activation("bundle entry is missing"))?;

	if !entry.1.starts_with(b"\0asm\x01\0\0\0") {
		return Err(activation(
			"bundle entry is not a WebAssembly version 1 module",
		));
	}

	Ok(&entry.1)
}

fn validate_container(
	bytes: &[u8],
	config: &UpdateConfiguration,
) -> Result<Container, CodePushError> {
	let parsed = parse_bounded(bytes)?;

	if parsed.manifest.app != config.app || parsed.manifest.channel != config.channel {
		return Err(CodePushError::WrongTarget {
			tracked: format!("{}/{}", config.app, config.channel),
			offered: format!("{}/{}", parsed.manifest.app, parsed.manifest.channel),
		});
	}

	let descriptor = parsed
		.files
		.iter()
		.find(|(path, _)| path == "runtime.json")
		.ok_or_else(|| activation("native bundle is missing runtime.json"))?;
	let runtime: RuntimeContract = serde_json::from_slice(&descriptor.1)?;

	if runtime.abi != config.runtime.abi
		|| runtime.shell != config.runtime.shell
		|| runtime
			.plugins
			.iter()
			.any(|plugin| !config.runtime.plugins.contains(plugin))
	{
		return Err(activation(
			"bundle requires an incompatible native ABI, shell, or plugin",
		));
	}

	entry_of(&parsed)?;
	Ok(parsed)
}

/// Blocking HTTP transport with no redirects and bodies capped at
/// [`MAX_BUNDLE_BYTES`]. Connecting has 15 s and the response head 30 s; a
/// whole request, body included, has 10 minutes. A 404 is
/// [`CodePushError::NotFound`], which a static registry uses for "no release
/// yet". HTTPS is required unless development opts in.
#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
pub struct HttpTransport {
	agent: ureq::Agent,
	allow_http: bool,
}

#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
impl HttpTransport {
	/// Builds a transport for a serialized background worker.
	pub fn new(allow_http: bool) -> Self {
		let config = ureq::Agent::config_builder()
			.max_redirects(0)
			// Status codes are classified below: a 404 is how a static registry
			// says a channel has no release (or a delta does not exist).
			.http_status_as_error(false)
			// Fail fast on an unreachable registry, but give a large bundle on a
			// slow link time to arrive. The overall bound keeps the worker free
			// for the next request.
			.timeout_connect(Some(std::time::Duration::from_secs(15)))
			.timeout_recv_response(Some(std::time::Duration::from_secs(30)))
			.timeout_global(Some(std::time::Duration::from_secs(10 * 60)))
			.build();
		Self {
			agent: config.into(),
			allow_http,
		}
	}
}

#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
impl Transport for HttpTransport {
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
		let secure = url.starts_with("https://");
		let development = self.allow_http && url.starts_with("http://");
		if !(secure || development) {
			return Err(CodePushError::Transport(
				"HTTPS is required; enable allow_http only for development".to_owned(),
			));
		}

		let mut response = self
			.agent
			.get(url)
			.call()
			.map_err(|error| CodePushError::Transport(error.to_string()))?;

		let status = response.status();
		if status == ureq::http::StatusCode::NOT_FOUND {
			return Err(CodePushError::NotFound(url.to_owned()));
		}
		if !status.is_success() {
			return Err(CodePushError::Transport(format!("HTTP {status}")));
		}

		response
			.body_mut()
			.with_config()
			.limit(MAX_BUNDLE_BYTES)
			.read_to_vec()
			.map_err(|error| CodePushError::Transport(error.to_string()))
	}
}
