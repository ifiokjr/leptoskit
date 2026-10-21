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
	/// What release builds ship: optimization, compression, and the size
	/// budget.
	pub bundle: BundleConfig,
	/// Native app settings.
	pub native: NativeConfig,
	/// Over-the-air update settings.
	pub codepush: CodePushConfig,
	/// File routes. The router is on when the table is present, even empty.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub router: Option<RouterConfig>,
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
	/// How the web build renders: in the browser only, on the server with
	/// full hydration, or on the server with interactive islands.
	pub render: RenderMode,
}

impl Default for AppConfig {
	fn default() -> Self {
		Self {
			name: String::new(),
			version: semver::Version::new(0, 1, 0),
			default_platform: Platform::Web,
			render: RenderMode::Csr,
		}
	}
}

/// How a web build renders its interface.
///
/// The mode decides which cargo features each side of the build gets, which
/// follows the leptos convention: the server package is built with its `ssr`
/// feature and the wasm client with its `hydrate` feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RenderMode {
	/// Client-side rendering: the server only serves files, and the wasm
	/// client renders the whole page. Neither side gets a mode feature. The
	/// default, so a project opts into server rendering explicitly.
	#[default]
	Csr,
	/// Server-side rendering with full hydration: the server renders every
	/// page, and the client hydrates all of it.
	Ssr,
	/// Server-side rendering with islands: the server renders every page,
	/// and the client hydrates only the `#[island]` components. Both sides
	/// also get `leptos/islands`, so the wasm bundle carries nothing but the
	/// islands.
	Islands,
}

impl RenderMode {
	/// Whether this mode renders HTML on the server, which makes the server
	/// binary the thing that serves the app.
	#[must_use]
	pub fn renders_on_server(self) -> bool {
		matches!(self, Self::Ssr | Self::Islands)
	}
}

impl fmt::Display for RenderMode {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Csr => f.write_str("csr"),
			Self::Ssr => f.write_str("ssr"),
			Self::Islands => f.write_str("islands"),
		}
	}
}

/// The platforms leptoskit builds for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
	/// The browser build: wasm + SSR server.
	#[default]
	Web,
	/// Native apps: wasm UI bundle on platform shells.
	Native,
	/// The server binary only.
	Server,
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
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BuildConfig {
	/// The wasm client crate's package name. When unset, leptoskit detects
	/// the workspace member producing a `cdylib` target.
	pub client_package: Option<String>,
	/// Cargo features activated for app builds.
	pub features: Vec<String>,
	/// Extra rustc flags for the wasm client build. They merge with any
	/// `target.wasm32-unknown-unknown.rustflags` in `.cargo/config.toml`. When
	/// set, cargo ignores a `[build] rustflags` table for the wasm build, and
	/// it ignores all of them when the `RUSTFLAGS` environment variable is set.
	pub rustflags: Vec<String>,
}

/// What a release build ships to the browser: how hard the client is
/// optimized, how its files are compressed, and how much it may weigh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BundleConfig {
	/// How hard release builds optimize the wasm client for size.
	pub optimize: Optimize,
	/// The cargo profile release builds compile the wasm client with, in
	/// place of the one [`optimize`](Self::optimize) picks: the escape hatch
	/// for full control from your `Cargo.toml`.
	pub cargo_profile: Option<String>,
	/// Whether release builds run `wasm-opt -Oz` over the wasm bundle.
	/// Release builds fail with an install hint when this is on and
	/// `wasm-opt` is not on the `PATH`.
	pub wasm_opt: bool,
	/// Whether release builds write brotli (`.br`) and gzip (`.gz`) copies
	/// next to every compressible file in the output, for servers to send
	/// as-is.
	pub precompress: bool,
	/// The most the initial load may weigh, in KiB, brotli-compressed: every
	/// script and wasm module the page fetches before any lazy chunk. Release
	/// builds over it fail.
	pub budget_kib: Option<u32>,
}

impl Default for BundleConfig {
	fn default() -> Self {
		Self {
			optimize: Optimize::Fast,
			cargo_profile: None,
			wasm_opt: true,
			precompress: true,
			budget_kib: None,
		}
	}
}

/// How hard a release build optimizes the wasm client for size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Optimize {
	/// Your `release` profile, then `wasm-opt -Oz`: small, and quick to
	/// rebuild.
	#[default]
	Fast,
	/// Also fat LTO, one codegen unit, `opt-level = "z"`, and
	/// `panic = "abort"`, so the optimizer sees and prunes the whole program.
	/// On leptoskit's benchmark app: 14% less wasm, brotli-compressed, for
	/// release rebuilds about three times slower.
	Smallest,
}

impl fmt::Display for Optimize {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Fast => f.write_str("fast"),
			Self::Smallest => f.write_str("smallest"),
		}
	}
}

impl std::str::FromStr for Optimize {
	type Err = ConfigError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		match value {
			"fast" => Ok(Self::Fast),
			"smallest" => Ok(Self::Smallest),
			other => Err(ConfigError::UnknownOptimize(other.to_owned())),
		}
	}
}

/// Native app settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct NativeConfig {
	/// The app-owned native capability contract. Change this when a store
	/// release changes plugin grants or native behavior incompatibly.
	pub update_contract: String,
	/// Runtime plugins explicitly granted to the generated native app.
	/// Each name must match metadata on a direct Cargo dependency.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub plugins: Vec<String>,
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
			update_contract: "mobile-v1".into(),
			plugins: Vec::new(),
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
	/// Allows plaintext HTTP for an explicitly configured local development
	/// registry. Native release builds always require HTTPS.
	pub allow_http: bool,
	/// The release registry base URL.
	pub server: String,
	/// The release channel to track.
	pub channel: String,
	/// Whether updates install without asking.
	pub mandatory: bool,
	/// The local staging directory, relative to the app data directory.
	pub staging_dir: String,
	/// The ed25519 public keys, hex encoded, whose signed releases the app
	/// installs. List the next key alongside the current one to rotate.
	pub public_keys: Vec<String>,
}

impl Default for CodePushConfig {
	fn default() -> Self {
		Self {
			enabled: false,
			allow_http: false,
			server: String::new(),
			channel: "production".to_owned(),
			mandatory: false,
			staging_dir: "codepush".to_owned(),
			public_keys: Vec::new(),
		}
	}
}

/// File routes: `leptoskit routes`, `dev` and `build` read the routes
/// directory and write the typed `Route` module beside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RouterConfig {
	/// The routes directory, relative to the project root. The generated
	/// module sits beside it with the same name, so `src/routes/` gets
	/// `src/routes.rs`, which `mod routes;` finds.
	pub dir: PathBuf,
}

impl Default for RouterConfig {
	fn default() -> Self {
		Self {
			dir: PathBuf::from("src/routes"),
		}
	}
}

impl RouterConfig {
	/// The generated module's path for this routes directory, relative to
	/// the project root.
	pub fn output(&self) -> PathBuf {
		self.dir.with_extension("rs")
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

/// Keys that moved to another table, as `(table, key, new table)`. A
/// configuration still using the old place gets an error naming the new one
/// instead of a bare "unknown field".
const MOVED_KEYS: &[(&str, &str, &str)] = &[("build", "wasm_opt", "bundle")];

/// The first moved key `raw` still sets in its old table, as
/// `("table.key", "new_table.key")`.
fn moved_key(raw: &str) -> Option<(String, String)> {
	let table = raw.parse::<toml::Table>().ok()?;
	MOVED_KEYS.iter().find_map(|(old, key, new)| {
		table
			.get(*old)
			.and_then(toml::Value::as_table)
			.is_some_and(|old| old.contains_key(*key))
			.then(|| (format!("{old}.{key}"), format!("{new}.{key}")))
	})
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
			match moved_key(&raw) {
				Some((from, to)) => {
					ConfigError::MovedKey {
						path: path.clone(),
						from,
						to,
					}
				}
				None => {
					ConfigError::Parse {
						path: path.clone(),
						source,
					}
				}
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
	/// - [`ConfigError::CodePushPublicKeysMissing`] when code push is enabled
	///   without any `public_keys`.
	/// - [`ConfigError::InvalidCodePushPublicKey`] when a public key is not 64
	///   hex characters.
	/// - [`ConfigError::InvalidPackageId`] when `native.package_id` is set but
	///   is not a valid reverse-domain id.
	/// - [`ConfigError::InvalidRouterDir`] when `router.dir` is absolute,
	///   empty, leaves the project, or has a name that is not a Rust module
	///   name.
	pub fn validate(&self) -> Result<(), ConfigError> {
		if self.codepush.enabled && self.codepush.server.trim().is_empty() {
			return Err(ConfigError::CodePushServerMissing);
		}
		if self.codepush.enabled && self.codepush.public_keys.is_empty() {
			return Err(ConfigError::CodePushPublicKeysMissing);
		}
		if let Some(key) = self
			.codepush
			.public_keys
			.iter()
			.find(|key| key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()))
		{
			return Err(ConfigError::InvalidCodePushPublicKey(key.clone()));
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
		if let Some(router) = &self.router {
			let dir = &router.dir;
			let inside_project = dir.is_relative()
				&& dir
					.components()
					.all(|component| matches!(component, std::path::Component::Normal(_)));
			// The directory's name is also the generated module's name.
			let module_name = dir
				.file_name()
				.and_then(|name| name.to_str())
				.is_some_and(|name| {
					let mut chars = name.chars();
					chars
						.next()
						.is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
						&& chars.all(|char| char == '_' || char.is_ascii_alphanumeric())
				});
			if !inside_project || !module_name {
				return Err(ConfigError::InvalidRouterDir(dir.clone()));
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
	/// The file sets a key in the table it moved out of.
	#[error("{from} in {path} has moved to {to}; move it under [{table}]", table = to.split('.').next().unwrap_or_default())]
	MovedKey {
		/// The file that sets the old key.
		path: PathBuf,
		/// The key's old place, such as `build.wasm_opt`.
		from: String,
		/// The key's new place, such as `bundle.wasm_opt`.
		to: String,
	},
	/// The configuration could not be serialized.
	#[error("could not serialize configuration")]
	Serialize(#[source] toml::ser::Error),
	/// An unknown platform was requested.
	#[error("unknown platform `{0}`; expected web, native, or server")]
	UnknownPlatform(String),
	/// An unknown optimization level was requested.
	#[error("unknown optimize level `{0}`; expected fast or smallest")]
	UnknownOptimize(String),
	/// Code push is enabled but has no release registry to talk to.
	#[error("codepush.enabled is true but codepush.server is empty; set the release registry URL")]
	CodePushServerMissing,
	/// Code push is enabled but trusts no publisher key, so no release could
	/// ever install.
	#[error(
		"codepush.enabled is true but codepush.public_keys is empty; add the key printed by \
		 `leptoskit codepush keygen`"
	)]
	CodePushPublicKeysMissing,
	/// A `codepush.public_keys` entry is not a hex-encoded ed25519 key.
	#[error("invalid codepush.public_keys entry `{0}`: expected 64 hex characters")]
	InvalidCodePushPublicKey(String),
	/// `native.package_id` is not a valid reverse-domain id.
	#[error("invalid native.package_id `{package_id}`: {reason}")]
	InvalidPackageId {
		/// The configured id.
		package_id: String,
		/// What is wrong with it.
		reason: &'static str,
	},
	/// `router.dir` does not name a directory inside the project, or its name
	/// cannot be the generated module's name.
	#[error(
		"invalid router.dir `{}`: name a directory inside the project whose name is a Rust \
		 module name, such as \"src/routes\"",
		.0.display()
	)]
	InvalidRouterDir(PathBuf),
	/// An IO error.
	#[error("io error")]
	Io(#[from] std::io::Error),
}
