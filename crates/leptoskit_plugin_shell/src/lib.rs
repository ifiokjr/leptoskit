//! Scoped desktop child processes with bounded capture and owned cancellation.
//!
//! Commands are host-defined capabilities, never shell command strings. Native
//! dispatch is asynchronous. The optional `async-host` adapter forwards bridge
//! cancellation and awaits direct-child reaping before completing the call.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error as BridgeError;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Named command + argument strings -> bounded encoded [`Output`].
pub const EXECUTE: MethodId = MethodId(0);
/// Maximum request size including framing.
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// Maximum requested argument count.
pub const MAX_ARGUMENTS: usize = 128;
/// Maximum combined stdout/stderr capture configured by the host.
pub const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// Request for a host-configured command. The executable cannot be overridden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
	pub command: String,
	pub arguments: Vec<String>,
}
impl Request {
	pub fn encode(&self) -> Result<Bytes, BridgeError> {
		self.validate()?;
		let mut encoder = Encoder::default();
		encoder.bytes(self.command.as_bytes())?;
		encoder.u32(self.arguments.len() as u32);
		for arg in &self.arguments {
			encoder.bytes(arg.as_bytes())?;
		}
		Ok(encoder.finish())
	}

	pub fn decode(payload: &[u8]) -> Result<Self, BridgeError> {
		if payload.len() > MAX_REQUEST_BYTES {
			return Err(BridgeError::TooLarge);
		}
		let mut decoder = Decoder::new(payload);
		let command = decoder.string()?.to_owned();
		let count = decoder.u32()? as usize;
		if count > MAX_ARGUMENTS {
			return Err(BridgeError::TooLarge);
		}
		let mut arguments = Vec::new();
		for _ in 0..count {
			arguments.push(decoder.string()?.to_owned());
		}
		decoder.finish()?;
		let request = Self { command, arguments };
		request.validate()?;
		Ok(request)
	}

	fn validate(&self) -> Result<(), BridgeError> {
		if !valid_name(&self.command) || self.arguments.iter().any(|arg| arg.contains('\0')) {
			return Err(BridgeError::InvalidArgument);
		}
		let size = self
			.arguments
			.iter()
			.fold(self.command.len().saturating_add(8), |size, arg| {
				size.saturating_add(arg.len()).saturating_add(4)
			});
		if self.arguments.len() > MAX_ARGUMENTS || size > MAX_REQUEST_BYTES {
			return Err(BridgeError::TooLarge);
		}
		Ok(())
	}
}
fn valid_name(name: &str) -> bool {
	!name.is_empty()
		&& name.len() <= 128
		&& name
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

/// Why capture stopped. Cancellation/limits return partial output explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Finish {
	Exited = 0,
	Cancelled = 1,
	TimedOut = 2,
	StdoutLimit = 3,
	StderrLimit = 4,
}

/// A reaped direct child's bounded output. No UTF-8 conversion is performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
	/// Exit code, or None when the process ended without a normal exit code.
	pub code: Option<i32>,
	/// True only for a normal successful exit with complete captured streams.
	pub success: bool,
	pub finish: Finish,
	pub stdout: Bytes,
	pub stderr: Bytes,
}
impl Output {
	/// Encode reason, success, code-presence, code bits, stdout, stderr.
	pub fn encode(&self) -> Result<Bytes, BridgeError> {
		self.validate()?;
		let mut encoder = Encoder::with_capacity(24 + self.stdout.len() + self.stderr.len());
		encoder.u32(self.finish as u32);
		encoder.u32(u32::from(self.success));
		encoder.u32(u32::from(self.code.is_some()));
		encoder.u32(u32::from_le_bytes(
			self.code.unwrap_or_default().to_le_bytes(),
		));
		encoder.bytes(&self.stdout)?;
		encoder.bytes(&self.stderr)?;
		Ok(encoder.finish())
	}

	pub fn decode(payload: &[u8]) -> Result<Self, BridgeError> {
		if payload.len() > MAX_OUTPUT_BYTES + 24 {
			return Err(BridgeError::TooLarge);
		}
		let mut decoder = Decoder::new(payload);
		let finish = match decoder.u32()? {
			0 => Finish::Exited,
			1 => Finish::Cancelled,
			2 => Finish::TimedOut,
			3 => Finish::StdoutLimit,
			4 => Finish::StderrLimit,
			_ => return Err(BridgeError::InvalidArgument),
		};
		let success = match decoder.u32()? {
			0 => false,
			1 => true,
			_ => return Err(BridgeError::InvalidArgument),
		};
		let code_present = match decoder.u32()? {
			0 => false,
			1 => true,
			_ => return Err(BridgeError::InvalidArgument),
		};
		let code_bits = decoder.u32()?;
		if !code_present && code_bits != 0 {
			return Err(BridgeError::InvalidArgument);
		}
		let code = code_present.then_some(i32::from_le_bytes(code_bits.to_le_bytes()));
		let stdout = Bytes::copy_from_slice(decoder.bytes()?);
		let stderr = Bytes::copy_from_slice(decoder.bytes()?);
		decoder.finish()?;
		let output = Self {
			code,
			success,
			finish,
			stdout,
			stderr,
		};
		output.validate()?;
		Ok(output)
	}

	fn validate(&self) -> Result<(), BridgeError> {
		if self.stdout.len().saturating_add(self.stderr.len()) > MAX_OUTPUT_BYTES {
			return Err(BridgeError::TooLarge);
		}
		if self.success != (self.finish == Finish::Exited && self.code == Some(0)) {
			return Err(BridgeError::InvalidArgument);
		}
		Ok(())
	}
}

/// Configuration, scheduling, and process errors retain their native details.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("child processes are unavailable on this platform")]
	Unavailable,
	#[error("invalid command configuration")]
	InvalidConfiguration,
	#[error("command or arguments are outside the host's scope")]
	Denied,
	#[error("child process capacity exhausted")]
	Busy,
	#[error("a running Tokio runtime is required")]
	NoRuntime,
	#[error(transparent)]
	Io(#[from] std::io::Error),
	#[error("child process worker failed: {0}")]
	Worker(String),
	#[error(transparent)]
	Protocol(#[from] BridgeError),
}
impl Error {
	pub fn bridge_error(&self) -> BridgeError {
		match self {
			Self::Unavailable => BridgeError::Unavailable,
			Self::InvalidConfiguration => BridgeError::InvalidArgument,
			Self::Denied => BridgeError::PermissionDenied,
			Self::Busy => BridgeError::Busy,
			Self::Protocol(error) => *error,
			Self::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
				BridgeError::PermissionDenied
			}
			_ => BridgeError::Native,
		}
	}
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod desktop;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub use desktop::ArgumentPolicy;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub use desktop::Cancellation;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub use desktop::CommandPolicy;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub use desktop::Limits;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub use desktop::RunningChild;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub use desktop::Shell;

#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
mod host;
#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
pub use host::ShellPlugin;
#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
pub use host::from_config;

/// Returns whether this target can run the native process backend.
pub const fn is_available() -> bool {
	cfg!(any(
		target_os = "macos",
		target_os = "linux",
		target_os = "windows"
	))
}

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub mod guest {
	use super::*;
	/// Cached scope capability. Dropping execute's future cancels the bridge
	/// call.
	#[derive(Clone, Copy)]
	pub struct ShellClient(leptoskit_bridge::guest::Client);
	impl ShellClient {
		pub fn new() -> Result<Self, BridgeError> {
			leptoskit_bridge::guest::Client::new("shell").map(Self)
		}

		pub async fn execute(self, request: &Request) -> Result<Output, BridgeError> {
			Output::decode(&self.0.call(EXECUTE, &request.encode()?)?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn binary_requests_reject_trailing_nul_and_excessive_counts() {
		let request = Request {
			command: "tool".into(),
			arguments: vec!["two words".into(), String::new()],
		};
		let encoded = request.encode().unwrap();
		assert_eq!(Request::decode(&encoded).unwrap(), request);
		for length in 0..encoded.len() {
			assert!(Request::decode(&encoded[..length]).is_err());
		}
		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert!(Request::decode(&trailing).is_err());
		assert!(
			Request {
				command: "tool".into(),
				arguments: vec!["a\0b".into()]
			}
			.encode()
			.is_err()
		);
		let mut huge = Encoder::default();
		huge.bytes(b"tool").unwrap();
		huge.u32(u32::MAX);
		assert_eq!(
			Request::decode(&huge.finish()).unwrap_err(),
			BridgeError::TooLarge
		);
	}
	#[test]
	fn binary_output_preserves_non_utf8_and_rejects_inconsistent_success() {
		let output = Output {
			code: Some(7),
			success: false,
			finish: Finish::Exited,
			stdout: Bytes::from_static(&[0xff, 0]),
			stderr: Bytes::from_static(b"error"),
		};
		assert_eq!(Output::decode(&output.encode().unwrap()).unwrap(), output);
		let mut invalid = output;
		invalid.success = true;
		assert!(invalid.encode().is_err());
	}
	#[test]
	fn requests_enforce_size_limits_before_and_after_decoding() {
		assert_eq!(
			Request::decode(&vec![0; MAX_REQUEST_BYTES + 1]).unwrap_err(),
			BridgeError::TooLarge
		);
		let too_many = Request {
			command: "tool".into(),
			arguments: vec![String::new(); MAX_ARGUMENTS + 1],
		};
		assert_eq!(too_many.encode().unwrap_err(), BridgeError::TooLarge);
		let too_long = Request {
			command: "tool".into(),
			arguments: vec!["x".repeat(MAX_REQUEST_BYTES)],
		};
		assert_eq!(too_long.encode().unwrap_err(), BridgeError::TooLarge);
		for command in ["", "rm -rf", "../tool", &"a".repeat(129)] {
			let request = Request {
				command: command.into(),
				arguments: Vec::new(),
			};
			assert_eq!(request.encode().unwrap_err(), BridgeError::InvalidArgument);
		}
	}
	#[test]
	fn every_finish_reason_and_signal_exit_roundtrips() {
		for finish in [
			Finish::Exited,
			Finish::Cancelled,
			Finish::TimedOut,
			Finish::StdoutLimit,
			Finish::StderrLimit,
		] {
			let output = Output {
				code: None,
				success: false,
				finish,
				stdout: Bytes::new(),
				stderr: Bytes::new(),
			};
			assert_eq!(Output::decode(&output.encode().unwrap()).unwrap(), output);
		}
		let success = Output {
			code: Some(0),
			success: true,
			finish: Finish::Exited,
			stdout: Bytes::from_static(b"ok"),
			stderr: Bytes::new(),
		};
		assert_eq!(Output::decode(&success.encode().unwrap()).unwrap(), success);
		let negative = Output {
			code: Some(-1),
			..success.clone()
		};
		assert_eq!(negative.encode().unwrap_err(), BridgeError::InvalidArgument);
	}
	#[test]
	fn output_decode_rejects_invalid_flags_and_oversized_payloads() {
		assert_eq!(
			Output::decode(&vec![0; MAX_OUTPUT_BYTES + 25]).unwrap_err(),
			BridgeError::TooLarge
		);
		let valid = Output {
			code: None,
			success: false,
			finish: Finish::Cancelled,
			stdout: Bytes::new(),
			stderr: Bytes::new(),
		}
		.encode()
		.unwrap()
		.to_vec();
		// finish, success, code-presence, then code bits without a code.
		for (offset, value) in [(0, 5), (4, 2), (8, 2), (12, 1)] {
			let mut invalid = valid.clone();
			invalid[offset] = value;
			assert_eq!(
				Output::decode(&invalid).unwrap_err(),
				BridgeError::InvalidArgument
			);
		}
		let oversized = Output {
			code: Some(0),
			success: true,
			finish: Finish::Exited,
			stdout: Bytes::from(vec![0; MAX_OUTPUT_BYTES]),
			stderr: Bytes::from_static(b"x"),
		};
		assert_eq!(oversized.encode().unwrap_err(), BridgeError::TooLarge);
	}
	#[test]
	fn errors_map_to_stable_bridge_errors() {
		let cases = [
			(Error::Unavailable, BridgeError::Unavailable),
			(Error::InvalidConfiguration, BridgeError::InvalidArgument),
			(Error::Denied, BridgeError::PermissionDenied),
			(Error::Busy, BridgeError::Busy),
			(Error::NoRuntime, BridgeError::Native),
			(Error::Worker("panicked".into()), BridgeError::Native),
			(
				Error::Protocol(BridgeError::TooLarge),
				BridgeError::TooLarge,
			),
			(
				Error::Io(std::io::ErrorKind::PermissionDenied.into()),
				BridgeError::PermissionDenied,
			),
			(
				Error::Io(std::io::ErrorKind::NotFound.into()),
				BridgeError::Native,
			),
		];
		for (error, expected) in cases {
			assert_eq!(error.bridge_error(), expected, "{error}");
		}
		assert_eq!(
			is_available(),
			cfg!(any(
				target_os = "macos",
				target_os = "linux",
				target_os = "windows"
			))
		);
	}
}
