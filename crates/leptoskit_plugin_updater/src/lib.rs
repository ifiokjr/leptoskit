//! <!-- {=p_updaterSummary|trim|linePrefix:"//! ":true} -->
//!
//! Native hosts construct [`Updater`] with an explicit [`Backend`] that knows
//! how to check, download, and install a new native executable. There is no
//! automatic `native.plugins` registration and no built-in update server. The
//! default [`UnavailableBackend`] keeps unsupported targets compiling.
//! Guest bundle / Wasm over-the-air updates remain `leptoskit_codepush`; this
//! crate is the installed-application updater surface, matching Electron's
//! `autoUpdater` responsibility without shipping a specific feed format.
//!
//! <!-- {/p_updaterSummary} -->
//!
//! First-wave hooks only: check, download, install-and-restart, and status.
//! Signing policy, delta feeds, and silent background downloads belong to the
//! host's chosen backend.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error as BridgeError;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Empty request -> current installed version UTF-8.
pub const CURRENT_VERSION: MethodId = MethodId(0);
/// Empty request -> versioned [`Status`] after a check.
pub const CHECK: MethodId = MethodId(1);
/// Empty request -> versioned [`Status`] after starting/finishing a download.
pub const DOWNLOAD: MethodId = MethodId(2);
/// Empty request -> empty success after requesting install and restart.
pub const INSTALL: MethodId = MethodId(3);
/// Empty request -> last known versioned [`Status`].
pub const STATUS: MethodId = MethodId(4);
/// Maximum UTF-8 length for version and notes fields.
pub const MAX_TEXT_BYTES: usize = 4096;

/// Progress and availability reported by an updater backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
	/// No newer release is available.
	UpToDate,
	/// A newer release can be downloaded.
	Available {
		/// Feed version string.
		version: String,
		/// Optional release notes.
		notes: Option<String>,
	},
	/// A download is in progress.
	Downloading {
		/// Bytes received so far.
		received: u64,
		/// Total bytes when the feed reports a size.
		total: Option<u64>,
	},
	/// A downloaded update is ready to install.
	Ready {
		/// Version that will be installed.
		version: String,
	},
	/// The backend failed; message is for logs, not raw UI.
	Failed {
		/// Bounded error summary.
		message: String,
	},
}

impl Status {
	/// Wire version 1 status encoding.
	pub fn encode(&self) -> Result<Bytes, BridgeError> {
		let mut output = Encoder::with_capacity(64);
		output.u32(1);
		match self {
			Self::UpToDate => output.u32(1),
			Self::Available { version, notes } => {
				validate_text(version)?;
				output.u32(2);
				output.bytes(version.as_bytes())?;
				match notes {
					None => output.u32(0),
					Some(text) => {
						validate_text(text)?;
						output.u32(1);
						output.bytes(text.as_bytes())?;
					}
				}
			}
			Self::Downloading { received, total } => {
				output.u32(3);
				output.u32((received >> 32) as u32);
				output.u32(*received as u32);
				match total {
					None => {
						output.u32(0);
						output.u32(0);
						output.u32(0);
					}
					Some(value) => {
						output.u32(1);
						output.u32((value >> 32) as u32);
						output.u32(*value as u32);
					}
				}
			}
			Self::Ready { version } => {
				validate_text(version)?;
				output.u32(4);
				output.bytes(version.as_bytes())?;
			}
			Self::Failed { message } => {
				validate_text(message)?;
				output.u32(5);
				output.bytes(message.as_bytes())?;
			}
		}
		Ok(output.finish())
	}

	/// Decodes a versioned status and rejects trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, BridgeError> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != 1 {
			return Err(BridgeError::InvalidArgument);
		}
		let status = match input.u32()? {
			1 => Self::UpToDate,
			2 => {
				let version = owned_text(input.string()?)?;
				let notes = match input.u32()? {
					0 => None,
					1 => Some(owned_text(input.string()?)?),
					_ => return Err(BridgeError::InvalidArgument),
				};
				Self::Available { version, notes }
			}
			3 => {
				let received = (u64::from(input.u32()?) << 32) | u64::from(input.u32()?);
				let total = match input.u32()? {
					0 => {
						let _ = input.u32()?;
						let _ = input.u32()?;
						None
					}
					1 => Some((u64::from(input.u32()?) << 32) | u64::from(input.u32()?)),
					_ => return Err(BridgeError::InvalidArgument),
				};
				Self::Downloading { received, total }
			}
			4 => {
				Self::Ready {
					version: owned_text(input.string()?)?,
				}
			}
			5 => {
				Self::Failed {
					message: owned_text(input.string()?)?,
				}
			}
			_ => return Err(BridgeError::InvalidArgument),
		};
		input.finish()?;
		Ok(status)
	}
}

/// Host-supplied native update implementation.
pub trait Backend: Send {
	/// Currently installed application version.
	fn current_version(&self) -> &str;
	/// Asks the feed whether an update exists.
	fn check(&mut self) -> Result<Status, Error>;
	/// Downloads the available update, or continues an in-progress download.
	fn download(&mut self) -> Result<Status, Error>;
	/// Installs a ready update and requests process restart.
	fn install_and_restart(&mut self) -> Result<(), Error>;
}

/// Errors from backends and argument validation.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// No updater backend is configured for this target.
	#[error("native updater is unavailable")]
	Unavailable,
	/// Invalid version text or bridge payload.
	#[error("invalid updater argument")]
	InvalidArgument,
	/// Backend refused or failed an operation.
	#[error("updater operation failed: {0}")]
	Native(String),
}

impl From<Error> for BridgeError {
	fn from(error: Error) -> Self {
		match error {
			Error::Unavailable => Self::Unavailable,
			Error::InvalidArgument => Self::InvalidArgument,
			Error::Native(_) => Self::Native,
		}
	}
}

/// Placeholder backend that keeps unsupported hosts compiling.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableBackend;

impl Backend for UnavailableBackend {
	#[allow(clippy::unnecessary_literal_bound)]
	fn current_version(&self) -> &str {
		"0.0.0"
	}

	fn check(&mut self) -> Result<Status, Error> {
		Err(Error::Unavailable)
	}

	fn download(&mut self) -> Result<Status, Error> {
		Err(Error::Unavailable)
	}

	fn install_and_restart(&mut self) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}

/// In-memory backend for tests and host scaffolding before a real feed exists.
#[derive(Debug, Clone)]
pub struct MemoryBackend {
	current: String,
	available: Option<(String, Option<String>)>,
	downloaded: bool,
	status: Status,
}

impl MemoryBackend {
	/// Creates a backend reporting `current` with an optional available update.
	pub fn new(
		current: impl Into<String>,
		available: Option<(String, Option<String>)>,
	) -> Result<Self, Error> {
		let current = current.into();
		validate_text(&current).map_err(|_| Error::InvalidArgument)?;
		if let Some((version, notes)) = &available {
			validate_text(version).map_err(|_| Error::InvalidArgument)?;
			if let Some(notes) = notes {
				validate_text(notes).map_err(|_| Error::InvalidArgument)?;
			}
		}
		Ok(Self {
			current,
			available,
			downloaded: false,
			status: Status::UpToDate,
		})
	}
}

impl Backend for MemoryBackend {
	fn current_version(&self) -> &str {
		&self.current
	}

	fn check(&mut self) -> Result<Status, Error> {
		self.status = match &self.available {
			None => Status::UpToDate,
			Some((version, notes)) => {
				Status::Available {
					version: version.clone(),
					notes: notes.clone(),
				}
			}
		};
		Ok(self.status.clone())
	}

	fn download(&mut self) -> Result<Status, Error> {
		let Some((version, _)) = &self.available else {
			self.status = Status::UpToDate;
			return Ok(self.status.clone());
		};
		self.downloaded = true;
		self.status = Status::Ready {
			version: version.clone(),
		};
		Ok(self.status.clone())
	}

	fn install_and_restart(&mut self) -> Result<(), Error> {
		if !self.downloaded {
			return Err(Error::Native("no downloaded update is ready".into()));
		}
		Ok(())
	}
}

/// Bridge-facing updater that owns a host backend.
pub struct Updater {
	backend: Box<dyn Backend>,
	status: Status,
}

impl std::fmt::Debug for Updater {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Updater")
			.field("current_version", &self.backend.current_version())
			.field("status", &self.status)
			.finish_non_exhaustive()
	}
}

impl Updater {
	/// Wraps a backend. Prefer a real installer backend in production hosts.
	#[must_use]
	pub fn new(backend: impl Backend + 'static) -> Self {
		Self {
			backend: Box::new(backend),
			status: Status::UpToDate,
		}
	}

	/// Currently installed version from the backend.
	#[must_use]
	pub fn current_version(&self) -> &str {
		self.backend.current_version()
	}

	/// Last status produced by check/download.
	#[must_use]
	pub fn status(&self) -> &Status {
		&self.status
	}

	/// Asks the backend for update availability.
	pub fn check(&mut self) -> Result<&Status, Error> {
		self.status = self.backend.check()?;
		Ok(&self.status)
	}

	/// Downloads through the backend.
	pub fn download(&mut self) -> Result<&Status, Error> {
		self.status = self.backend.download()?;
		Ok(&self.status)
	}

	/// Installs and requests restart through the backend.
	pub fn install_and_restart(&mut self) -> Result<(), Error> {
		self.backend.install_and_restart()
	}
}

impl Plugin for Updater {
	fn name(&self) -> &'static str {
		"updater"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[CURRENT_VERSION, CHECK, DOWNLOAD, INSTALL, STATUS]
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, BridgeError> {
		if !payload.is_empty() {
			return Err(BridgeError::InvalidArgument);
		}
		match method {
			CURRENT_VERSION => {
				let version = self.current_version();
				validate_text(version)?;
				let mut output = Encoder::with_capacity(version.len() + 4);
				output.bytes(version.as_bytes())?;
				Ok(output.finish())
			}
			CHECK => {
				self.check()?;
				self.status.encode()
			}
			DOWNLOAD => {
				self.download()?;
				self.status.encode()
			}
			INSTALL => {
				self.install_and_restart()?;
				Ok(Bytes::new())
			}
			STATUS => self.status.encode(),
			_ => Err(BridgeError::Unavailable),
		}
	}
}

fn validate_text(value: &str) -> Result<(), BridgeError> {
	if value.is_empty() || value.len() > MAX_TEXT_BYTES || value.chars().any(char::is_control) {
		Err(BridgeError::InvalidArgument)
	} else {
		Ok(())
	}
}

fn owned_text(value: &str) -> Result<String, BridgeError> {
	validate_text(value)?;
	Ok(value.to_owned())
}

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub mod guest {
	use super::*;

	/// Cached guest client for the host updater.
	#[derive(Debug, Clone, Copy)]
	pub struct UpdaterClient(leptoskit_bridge::guest::Client);

	impl UpdaterClient {
		/// Resolves the registered updater plugin once.
		pub fn new() -> Result<Self, BridgeError> {
			leptoskit_bridge::guest::Client::new("updater").map(Self)
		}

		/// Reads the installed version.
		pub async fn current_version(self) -> Result<String, BridgeError> {
			let bytes = self.0.call(CURRENT_VERSION, &[])?.await?;
			let mut input = Decoder::new(&bytes);
			let version = owned_text(input.string()?)?;
			input.finish()?;
			Ok(version)
		}

		/// Runs a check through the host backend.
		pub async fn check(self) -> Result<Status, BridgeError> {
			Status::decode(&self.0.call(CHECK, &[])?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn memory_backend_check_download_install() {
		let mut updater = Updater::new(
			MemoryBackend::new("1.0.0", Some(("1.1.0".into(), Some("fixes".into())))).unwrap(),
		);
		assert_eq!(updater.current_version(), "1.0.0");
		assert!(matches!(
			updater.check().unwrap(),
			Status::Available { version, .. } if version == "1.1.0"
		));
		assert!(matches!(
			updater.download().unwrap(),
			Status::Ready { version } if version == "1.1.0"
		));
		updater.install_and_restart().unwrap();
		let encoded = updater.status().encode().unwrap();
		assert_eq!(Status::decode(&encoded).unwrap(), updater.status().clone());
	}

	#[test]
	fn unavailable_backend_keeps_targets_compiling() {
		let mut updater = Updater::new(UnavailableBackend);
		assert_eq!(
			updater.invoke(CHECK, Bytes::new()),
			Err(BridgeError::Unavailable)
		);
		assert_eq!(
			updater.invoke(CURRENT_VERSION, Bytes::from_static(b"x")),
			Err(BridgeError::InvalidArgument)
		);
	}

	#[test]
	fn status_wire_rejects_trailing_bytes() {
		let status = Status::UpToDate.encode().unwrap();
		let mut bad = status.to_vec();
		bad.push(0);
		assert!(Status::decode(&bad).is_err());
	}
}
