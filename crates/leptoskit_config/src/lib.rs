//! Configuration for leptoskit projects, loaded from `leptoskit.toml`.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

/// The file leptoskit looks for at a project root.
pub const CONFIG_FILE: &str = "leptoskit.toml";

/// Everything leptoskit needs to build, serve, and ship a project.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LeptoskitConfig {
	/// The application identity and platform defaults.
	pub app: AppConfig,
	/// Development and production server settings.
	pub serve: ServeConfig,
	/// Build pipeline settings.
	pub build: BuildConfig,
	/// Native app settings.
	pub native: NativeConfig,
	/// Over-the-air update settings.
	pub codepush: CodePushConfig,
	/// The leptos and leptoskit plugins activated for this project.
	pub plugins: Vec<PluginSpec>,
}

/// The application identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AppConfig {
	/// The package name; defaults to the directory name.
	pub name: String,
	/// The semantic version of the app.
	pub version: semver::Version,
	/// The platform `leptoskit dev` and `build` target by default.
	pub default_platform: Platform,
	/// Whether the web build renders HTML on the server.
	pub ssr: bool,
	/// Whether the client attaches to server-rendered HTML.
	pub hydration: bool,
}

impl Default for AppConfig {
	fn default() -> Self {
		Self {
			name: String::new(),
			version: semver::Version::new(0, 1, 0),
			default_platform: Platform::Web,
			ssr: true,
			hydration: true,
		}
	}
}

/// The platforms leptoskit builds for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
	/// The browser build: wasm + SSR server.
	Web,
	/// Native apps: wasm UI bundle on platform shells.
	Native,
	/// The server binary only.
	Server,
}

impl Default for Platform {
	fn default() -> Self {
		Self::Web
	}
}

impl fmt::Display for Platform {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Web => f.write_str("web"),
			Self::Native => f.write_str("native"),
			Self::Server => f.write_str("server"),
		}
	}
}

impl std::str::FromStr for Platform {
	type Err = ConfigError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		match value {
			"web" => Ok(Self::Web),
			"native" => Ok(Self::Native),
			"server" => Ok(Self::Server),
			other => Err(ConfigError::UnknownPlatform(other.to_owned())),
		}
	}
}

/// Server settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ServeConfig {
	/// The bind address.
	pub address: String,
	/// The bind port.
	pub port: u16,
}

impl Default for ServeConfig {
	fn default() -> Self {
		Self {
			address: "127.0.0.1".to_owned(),
			port: 3000,
		}
	}
}

/// Build pipeline settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BuildConfig {
	/// The wasm client crate's package name. When unset, leptoskit detects
	/// the workspace member producing a `cdylib` target.
	pub client_package: Option<String>,
	/// Cargo features activated for app builds.
	pub features: Vec<String>,
	/// Whether release builds run `wasm-opt` over the wasm bundle. Release
	/// builds fail with an install hint when this is on and `wasm-opt` is not
	/// on the `PATH`.
	pub wasm_opt: bool,
	/// Extra rustc flags for the wasm client build. They merge with any
	/// `target.wasm32-unknown-unknown.rustflags` in `.cargo/config.toml`. When
	/// set, cargo ignores a `[build] rustflags` table for the wasm build, and
	/// it ignores all of them when the `RUSTFLAGS` environment variable is set.
	pub rustflags: Vec<String>,
}

impl Default for BuildConfig {
	fn default() -> Self {
		Self {
			client_package: None,
			features: Vec::new(),
			wasm_opt: true,
			rustflags: Vec::new(),
		}
	}
}

/// Native app settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct NativeConfig {
	/// The reverse-domain application id used by platform shells, such as
	/// `com.example.counter`.
	///
	/// It must have at least two dot-separated segments, each an ASCII letter
	/// followed by ASCII letters or digits: the only form both iOS bundle ids
	/// (no `_`) and Android package names (no `-`) accept. When unset it
	/// defaults to [`default_package_id`] of the app name.
	pub package_id: String,
	/// The native platforms to build for.
	pub targets: Vec<NativeTarget>,
	/// The initial viewport width for the headless layout pass.
	pub viewport_width: f32,
	/// The initial viewport height for the headless layout pass.
	pub viewport_height: f32,
}

impl Default for NativeConfig {
	fn default() -> Self {
		Self {
			package_id: String::new(),
			targets: vec![
				NativeTarget::Macos,
				NativeTarget::Ios,
				NativeTarget::Android,
			],
			viewport_width: 390.0,
			viewport_height: 844.0,
		}
	}
}

/// A native platform shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NativeTarget {
	/// macOS.
	Macos,
	/// iOS.
	Ios,
	/// Android.
	Android,
	/// Windows.
	Windows,
	/// Linux.
	Linux,
}

impl fmt::Display for NativeTarget {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Macos => f.write_str("macos"),
			Self::Ios => f.write_str("ios"),
			Self::Android => f.write_str("android"),
			Self::Windows => f.write_str("windows"),
			Self::Linux => f.write_str("linux"),
		}
	}
}

/// Over-the-air update settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CodePushConfig {
	/// Whether code push is enabled for this project.
	pub enabled: bool,
	/// The release registry base URL.
	pub server: String,
	/// The release channel to track.
	pub channel: String,
	/// Whether updates install without asking.
	pub mandatory: bool,
	/// The local staging directory, relative to the app data directory.
	pub staging_dir: String,
}

impl Default for CodePushConfig {
	fn default() -> Self {
		Self {
			enabled: false,
			server: String::new(),
			channel: "production".to_owned(),
			mandatory: false,
			staging_dir: "codepush".to_owned(),
		}
	}
}

/// One activated plugin.
///
/// Names without a `path` resolve against the built-in registry; a `path`
/// points at a plugin crate compiled into the tooling. Leptos plugins —
/// activated as cargo features on the app — are declared with `leptos = true`
/// and forwarded to the app build.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PluginSpec {
	/// The plugin name.
	pub name: String,
	/// The path to a local plugin crate, when it is not a built-in.
	pub path: Option<PathBuf>,
	/// Whether this is a leptos plugin, activated as an app cargo feature.
	pub leptos: bool,
	/// Options passed to the plugin.
	pub options: toml::Table,
}

impl Default for PluginSpec {
	fn default() -> Self {
		Self {
			name: String::new(),
			path: None,
			leptos: false,
			options: toml::Table::new(),
		}
	}
}

/// The package id leptoskit derives for an app named `app_name`:
/// `com.leptoskit.<segment>`.
///
/// The segment is the name lowercased with every character outside `a-z` and
/// `0-9` removed, prefixed with `app` when that leaves it empty or starting
/// with a digit, so the id is valid as both an iOS bundle id and an Android
/// package name.
///
/// ```
/// use leptoskit_config::default_package_id;
///
/// assert_eq!(default_package_id("my-app"), "com.leptoskit.myapp");
/// assert_eq!(default_package_id("1app"), "com.leptoskit.app1app");
/// assert_eq!(default_package_id(""), "com.leptoskit.app");
/// ```
pub fn default_package_id(app_name: &str) -> String {
	let mut segment: String = app_name
		.chars()
		.map(|character| character.to_ascii_lowercase())
		.filter(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
		.collect();
	if !segment.starts_with(|character: char| character.is_ascii_lowercase()) {
		segment.insert_str(0, "app");
	}
	format!("com.leptoskit.{segment}")
}

/// Why `package_id` is not a valid reverse-domain id, if it is not.
fn package_id_problem(package_id: &str) -> Option<&'static str> {
	let segments: Vec<&str> = package_id.split('.').collect();
	if segments.len() < 2 {
		return Some("it needs at least two dot-separated segments");
	}
	for segment in segments {
		let mut characters = segment.chars();
		if !characters
			.next()
			.is_some_and(|first| first.is_ascii_alphabetic())
		{
			return Some("every segment must start with an ASCII letter");
		}
		if !characters.all(|character| character.is_ascii_alphanumeric()) {
			return Some("segments may only contain ASCII letters and digits");
		}
	}
	None
}

impl LeptoskitConfig {
	/// Loads and validates the configuration for the project rooted at `dir`.
	///
	/// # Errors
	///
	/// Returns an error when the file cannot be read or parsed, or when
	/// [`validate`](Self::validate) rejects it.
	pub fn load_from(dir: &Path) -> Result<Self, ConfigError> {
		let path = dir.join(CONFIG_FILE);
		let raw = std::fs::read_to_string(&path).map_err(|source| {
			ConfigError::Read {
				path: path.clone(),
				source,
			}
		})?;
		let config: Self = toml::from_str(&raw).map_err(|source| {
			ConfigError::Parse {
				path: path.clone(),
				source,
			}
		})?;
		let config = config.with_defaults_from(dir);
		config.validate()?;
		Ok(config)
	}

	/// Checks settings that parse but cannot work together.
	///
	/// # Errors
	///
	/// - [`ConfigError::CodePushServerMissing`] when code push is enabled
	///   without a `server`.
	/// - [`ConfigError::InvalidPackageId`] when `native.package_id` is set but
	///   is not a valid reverse-domain id.
	pub fn validate(&self) -> Result<(), ConfigError> {
		if self.codepush.enabled && self.codepush.server.trim().is_empty() {
			return Err(ConfigError::CodePushServerMissing);
		}
		let package_id = &self.native.package_id;
		if !package_id.is_empty() {
			if let Some(reason) = package_id_problem(package_id) {
				return Err(ConfigError::InvalidPackageId {
					package_id: package_id.clone(),
					reason,
				});
			}
		}
		Ok(())
	}

	/// Loads the configuration for the current directory or the nearest
	/// ancestor that has one, returning the project root alongside it.
	pub fn load() -> Result<(Self, PathBuf), ConfigError> {
		let mut current = std::env::current_dir().map_err(ConfigError::Io)?;
		loop {
			if current.join(CONFIG_FILE).is_file() {
				let config = Self::load_from(&current)?;
				return Ok((config, current));
			}
			if !current.pop() {
				return Err(ConfigError::NotFound);
			}
		}
	}

	/// Serializes back to TOML.
	pub fn to_toml_string(&self) -> Result<String, ConfigError> {
		toml::to_string_pretty(self).map_err(ConfigError::Serialize)
	}

	/// Fills defaults that depend on the project directory, like the app name.
	fn with_defaults_from(mut self, dir: &Path) -> Self {
		if self.app.name.is_empty() {
			self.app.name = dir
				.file_name()
				.and_then(|name| name.to_str())
				.unwrap_or("leptoskit_app")
				.into();
		}
		if self.native.package_id.is_empty() {
			self.native.package_id = default_package_id(&self.app.name);
		}
		self
	}
}

/// Errors raised while loading configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
	/// No `leptoskit.toml` in this directory or any ancestor.
	#[error("no {CONFIG_FILE} found in this directory or any ancestor")]
	NotFound,
	/// The file could not be read.
	#[error("could not read {path}")]
	Read {
		/// The file that failed to read.
		path: PathBuf,
		/// The underlying IO error.
		source: std::io::Error,
	},
	/// The file could not be parsed.
	#[error("could not parse {path}")]
	Parse {
		/// The file that failed to parse.
		path: PathBuf,
		/// The underlying TOML error.
		source: toml::de::Error,
	},
	/// The configuration could not be serialized.
	#[error("could not serialize configuration")]
	Serialize(#[source] toml::ser::Error),
	/// An unknown platform was requested.
	#[error("unknown platform `{0}`; expected web, native, or server")]
	UnknownPlatform(String),
	/// Code push is enabled but has no release registry to talk to.
	#[error("codepush.enabled is true but codepush.server is empty; set the release registry URL")]
	CodePushServerMissing,
	/// `native.package_id` is not a valid reverse-domain id.
	#[error("invalid native.package_id `{package_id}`: {reason}")]
	InvalidPackageId {
		/// The configured id.
		package_id: String,
		/// What is wrong with it.
		reason: &'static str,
	},
	/// An IO error.
	#[error("io error")]
	Io(#[from] std::io::Error),
}
