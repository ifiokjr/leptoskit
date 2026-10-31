//! Platform information about the executing host.
//!
//! Register this on the native side to report the actual OS to an interpreted
//! guest. Direct calls in a browser correctly report `web` and `wasm32`;
//! they do not infer device identity from a spoofable user-agent string.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_bridge::codec::Encoder;

/// No arguments -> length-prefixed platform and architecture UTF-8 strings.
pub const INFO: MethodId = MethodId(0);

/// The compilation target of the host implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
	/// `ios`, `android`, `macos`, `windows`, `linux`, `harmonyos`, or `web`.
	pub platform: &'static str,
	/// Rust target architecture.
	pub architecture: &'static str,
}

/// Reads static host information without allocation or a native call.
pub fn info() -> Info {
	Info {
		platform: platform(
			std::env::consts::OS,
			cfg!(target_env = "ohos"),
			cfg!(all(target_arch = "wasm32", target_os = "unknown")),
		),
		architecture: std::env::consts::ARCH,
	}
}

/// Names the platform for a Rust `target_os`. HarmonyOS targets report
/// `target_os = "linux"` with `target_env = "ohos"`, but they aren't Linux to
/// an app.
fn platform(os: &'static str, ohos: bool, web: bool) -> &'static str {
	if web {
		"web"
	} else if ohos {
		"harmonyos"
	} else {
		os
	}
}

/// Host information exposed through the binary plugin bridge.
#[derive(Debug, Default)]
pub struct Os;

impl Plugin for Os {
	fn name(&self) -> &'static str {
		"os"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[INFO]
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		if method != INFO {
			return Err(Error::Unavailable);
		}
		if !payload.is_empty() {
			return Err(Error::InvalidArgument);
		}
		let info = info();
		let mut output = Encoder::with_capacity(8 + info.platform.len() + info.architecture.len());
		output.bytes(info.platform.as_bytes())?;
		output.bytes(info.architecture.as_bytes())?;
		Ok(output.finish())
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::codec::Decoder;

	use super::*;

	#[test]
	fn reports_the_host_and_rejects_unexpected_arguments() {
		let payload = Os.invoke(INFO, Bytes::new()).unwrap();
		let mut decoder = Decoder::new(&payload);
		assert_eq!(decoder.string().unwrap(), info().platform);
		assert_eq!(decoder.string().unwrap(), info().architecture);
		decoder.finish().unwrap();
		assert_eq!(
			Os.invoke(INFO, Bytes::from_static(b"x")),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn registers_only_the_info_method() {
		let mut os = Os;
		assert_eq!(os.name(), "os");
		assert_eq!(os.methods(), [INFO]);
		assert_eq!(
			os.invoke(MethodId(1), Bytes::new()),
			Err(Error::Unavailable)
		);
	}

	#[test]
	fn host_information_names_a_known_platform_and_this_architecture() {
		let info = info();
		assert!(
			[
				"ios",
				"android",
				"macos",
				"windows",
				"linux",
				"harmonyos",
				"web"
			]
			.contains(&info.platform),
			"unexpected platform {:?}",
			info.platform
		);
		assert_eq!(info.architecture, std::env::consts::ARCH);
		assert_ne!(info.architecture, "");
	}

	#[test]
	fn harmonyos_targets_report_harmonyos_instead_of_linux() {
		assert_eq!(platform("linux", true, false), "harmonyos");
		assert_eq!(platform("linux", false, false), "linux");
		assert_eq!(platform("unknown", false, true), "web");
		assert_eq!(platform("macos", false, false), "macos");
	}
}
