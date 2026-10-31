//! Per-user login registration with an explicit app identity and command.
//!
//! Constructing an [`Autostart`] never enables login startup. Registration is
//! host-configured, so guests cannot choose arbitrary executables or arguments.
//! `is_registered` reports the entry we own, not user/administrator overrides
//! or a guarantee that the desktop will launch it. No process is launched now.

use std::path::Path;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error as BridgeError;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;

/// Empty request -> empty response, after installing the configured entry.
pub const ENABLE: MethodId = MethodId(0);
/// Empty request -> empty response, after removing the configured entry.
pub const DISABLE: MethodId = MethodId(1);
/// Empty request -> one byte (`0` or `1`) describing registration presence.
pub const IS_REGISTERED: MethodId = MethodId(2);

/// Invalid configuration, conflicting entries, and platform errors are
/// distinct.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// Identity, executable, or arguments cannot be represented safely.
	#[error("invalid autostart configuration")]
	InvalidConfiguration,
	/// An entry exists for this identity with different contents.
	#[error("autostart entry is not owned by this configuration")]
	Conflict,
	/// Login registration is desktop-only.
	#[error("autostart is unavailable on this platform")]
	Unavailable,
	/// The operating system rejected an operation.
	#[error(transparent)]
	Io(#[from] std::io::Error),
}

/// A validated, immutable login command. Disable before changing the command.
#[derive(Debug)]
pub struct Autostart {
	identity: String,
	executable: String,
	arguments: Vec<String>,
}

impl Autostart {
	/// Stable identity used for the startup entry.
	pub fn identity(&self) -> &str {
		&self.identity
	}

	/// Absolute executable configured by the host.
	pub fn executable(&self) -> &Path {
		Path::new(&self.executable)
	}

	/// Exact arguments that will be passed at login, without shell evaluation.
	pub fn arguments(&self) -> &[String] {
		&self.arguments
	}

	/// Validate the app identity and absolute executable path without
	/// registering it.
	///
	/// Identity is 1–128 ASCII letters, digits, dots, hyphens, or underscores,
	/// and starts with a letter or digit. Executable must be an existing
	/// regular file. UTF-8 arguments cannot contain control characters. On
	/// Linux, executable paths containing `=` are rejected by the desktop
	/// entry specification.
	pub fn new(identity: String, executable: &Path, arguments: Vec<String>) -> Result<Self, Error> {
		let executable_text = executable.to_str().ok_or(Error::InvalidConfiguration)?;
		if !valid_identity(&identity)
			|| !executable.is_absolute()
			|| executable_text.chars().any(char::is_control)
			|| arguments
				.iter()
				.any(|arg| arg.chars().any(char::is_control))
			|| (cfg!(all(target_os = "linux", not(target_env = "ohos")))
				&& executable_text.contains('='))
		{
			return Err(Error::InvalidConfiguration);
		}
		if !std::fs::metadata(executable)?.is_file() {
			return Err(Error::InvalidConfiguration);
		}
		Ok(Self {
			identity,
			executable: executable_text.to_owned(),
			arguments,
		})
	}

	/// Register for future logins. Repeated calls with the same command
	/// succeed.
	pub fn enable(&self) -> Result<(), Error> {
		platform::enable(self)
	}

	/// Remove only the exact entry this configuration would install.
	pub fn disable(&self) -> Result<(), Error> {
		platform::disable(self)
	}

	/// Whether our exact registration exists. Conflicting entries return an
	/// error.
	pub fn is_registered(&self) -> Result<bool, Error> {
		platform::is_registered(self)
	}
}

fn valid_identity(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 128
		&& value.as_bytes()[0].is_ascii_alphanumeric()
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

impl Plugin for Autostart {
	fn name(&self) -> &'static str {
		"autostart"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[ENABLE, DISABLE, IS_REGISTERED]
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, BridgeError> {
		let operation: fn(&Self) -> Result<Bytes, Error> = match method {
			ENABLE => |app| app.enable().map(|()| Bytes::new()),
			DISABLE => |app| app.disable().map(|()| Bytes::new()),
			IS_REGISTERED => {
				|app| {
					app.is_registered()
						.map(|value| Bytes::copy_from_slice(&[u8::from(value)]))
				}
			}
			_ => return Err(BridgeError::Unavailable),
		};
		if !payload.is_empty() {
			return Err(BridgeError::InvalidArgument);
		}
		operation(self).map_err(|error| bridge_error(&error))
	}
}

fn bridge_error(error: &Error) -> BridgeError {
	match error {
		Error::Unavailable => BridgeError::Unavailable,
		Error::InvalidConfiguration => BridgeError::InvalidArgument,
		Error::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied => {
			BridgeError::PermissionDenied
		}
		Error::Conflict | Error::Io(_) => BridgeError::Native,
	}
}

#[cfg(any(
	all(target_os = "linux", not(target_env = "ohos")),
	target_os = "macos",
	test
))]
mod file_entry;
#[cfg(any(
	all(target_os = "linux", not(target_env = "ohos")),
	target_os = "macos"
))]
mod platform {
	use super::Autostart;
	use super::Error;
	use super::file_entry;
	fn entry(app: &Autostart) -> Result<file_entry::Entry, Error> {
		#[cfg(target_os = "macos")]
		let (directory, extension, content) = (
			dirs::home_dir().map(|home| home.join("Library/LaunchAgents")),
			"plist",
			file_entry::launch_agent(app),
		);
		#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
		let (directory, extension, content) = (
			dirs::config_dir().map(|config| config.join("autostart")),
			"desktop",
			file_entry::desktop_entry(app),
		);
		let directory = directory.ok_or_else(|| {
			std::io::Error::new(
				std::io::ErrorKind::NotFound,
				"user configuration directory unavailable",
			)
		})?;
		Ok(file_entry::Entry::new(
			directory.join(format!("{}.{}", app.identity, extension)),
			content,
		))
	}
	pub fn enable(app: &Autostart) -> Result<(), Error> {
		entry(app)?.enable()
	}
	pub fn disable(app: &Autostart) -> Result<(), Error> {
		entry(app)?.disable()
	}
	pub fn is_registered(app: &Autostart) -> Result<bool, Error> {
		entry(app)?.is_registered()
	}
}
#[cfg(target_os = "windows")]
mod platform;
#[cfg(not(any(
	all(target_os = "linux", not(target_env = "ohos")),
	target_os = "macos",
	target_os = "windows"
)))]
mod platform {
	use super::Autostart;
	use super::Error;
	pub fn enable(_: &Autostart) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
	pub fn disable(_: &Autostart) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
	pub fn is_registered(_: &Autostart) -> Result<bool, Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(any(target_os = "windows", test))]
fn windows_command(app: &Autostart) -> String {
	fn quoted(value: &str) -> String {
		let mut result = String::from("\"");
		let mut slashes = 0;
		for ch in value.chars() {
			if ch == '\\' {
				slashes += 1;
				continue;
			}
			result.extend(std::iter::repeat_n(
				'\\',
				if ch == '"' { slashes * 2 + 1 } else { slashes },
			));
			result.push(ch);
			slashes = 0;
		}
		result.extend(std::iter::repeat_n('\\', slashes * 2));
		result.push('"');
		result
	}
	std::iter::once(&app.executable)
		.chain(&app.arguments)
		.map(|value| quoted(value))
		.collect::<Vec<_>>()
		.join(" ")
}

/// Typed guest access to the immutable host configuration.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub mod guest {
	use super::*;
	/// Cached bridge handle. Resolution does not alter login settings.
	#[derive(Clone, Copy)]
	pub struct AutostartClient(leptoskit_bridge::guest::Client);
	impl AutostartClient {
		pub fn new() -> Result<Self, BridgeError> {
			leptoskit_bridge::guest::Client::new("autostart").map(Self)
		}

		pub async fn enable(self) -> Result<(), BridgeError> {
			let response = self.0.call(ENABLE, &[])?.await?;
			if response.is_empty() {
				Ok(())
			} else {
				Err(BridgeError::InvalidArgument)
			}
		}

		pub async fn disable(self) -> Result<(), BridgeError> {
			let response = self.0.call(DISABLE, &[])?.await?;
			if response.is_empty() {
				Ok(())
			} else {
				Err(BridgeError::InvalidArgument)
			}
		}

		pub async fn is_registered(self) -> Result<bool, BridgeError> {
			match self.0.call(IS_REGISTERED, &[])?.await?.as_ref() {
				[0] => Ok(false),
				[1] => Ok(true),
				_ => Err(BridgeError::InvalidArgument),
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn validation_has_no_registration_side_effects() {
		for identity in ["", "../escape", ".hidden", "hello/world", "hello\nworld"] {
			assert!(
				Autostart::new(identity.into(), &std::env::current_exe().unwrap(), vec![]).is_err()
			);
		}
		let mut app = Autostart::new(
			"dev.leptoskit.test".into(),
			&std::env::current_exe().unwrap(),
			vec![],
		)
		.unwrap();
		assert!(matches!(
			app.invoke(ENABLE, Bytes::from_static(b"bad")),
			Err(BridgeError::InvalidArgument)
		));
		assert!(matches!(
			app.invoke(MethodId(99), Bytes::new()),
			Err(BridgeError::Unavailable)
		));
	}
	#[test]
	fn the_plugin_exposes_exactly_its_three_methods() {
		let app = Autostart::new(
			"dev.leptoskit.methods".into(),
			&std::env::current_exe().unwrap(),
			vec![],
		)
		.unwrap();
		assert_eq!(app.methods(), &[ENABLE, DISABLE, IS_REGISTERED]);
	}
	#[test]
	fn directories_are_not_executables_and_errors_map_to_bridge_statuses() {
		let directory = tempfile::tempdir().unwrap();
		assert!(matches!(
			Autostart::new("dev.leptoskit.test".into(), directory.path(), vec![]),
			Err(Error::InvalidConfiguration)
		));
		for (error, expected) in [
			(Error::Unavailable, BridgeError::Unavailable),
			(Error::InvalidConfiguration, BridgeError::InvalidArgument),
			(
				Error::Io(std::io::ErrorKind::PermissionDenied.into()),
				BridgeError::PermissionDenied,
			),
			(Error::Conflict, BridgeError::Native),
			(
				Error::Io(std::io::ErrorKind::NotFound.into()),
				BridgeError::Native,
			),
		] {
			assert_eq!(bridge_error(&error), expected);
		}
	}
	#[test]
	fn windows_arguments_preserve_spaces_quotes_and_trailing_slashes() {
		let app = Autostart {
			identity: "test".into(),
			executable: "C:\\Program Files\\app.exe".into(),
			arguments: vec![String::new(), "a\"b".into(), "trail\\".into()],
		};
		assert_eq!(
			windows_command(&app),
			"\"C:\\Program Files\\app.exe\" \"\" \"a\\\"b\" \"trail\\\\\""
		);
	}
}
