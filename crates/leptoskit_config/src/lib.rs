//! Configuration for leptoskit projects, loaded from `leptoskit.toml`.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

pub mod migration;
#[cfg(feature = "schema")]
pub mod schema;

/// The file leptoskit looks for at a project root.
pub const CONFIG_FILE: &str = "leptoskit.toml";

/// The configuration contract version, independent of the crate version.
/// Version 0 is unstable; version 1 will establish the first stable contract.
pub const SCHEMA_VERSION: u32 = 0;

/// Public editor schema pinned to the current configuration contract.
pub fn schema_url() -> String {
	format!("https://ifiokjr.github.io/leptoskit/schemas/leptoskit.v{SCHEMA_VERSION}.schema.json")
}

const fn default_schema_version() -> u32 {
	SCHEMA_VERSION
}

/// Everything leptoskit needs to build, serve, and ship a project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct LeptoskitConfig {
	/// Configuration contract version. Version 0 is unstable; breaking changes
	/// are expected until version 1. Omitted versions are accepted for existing
	/// projects; `leptoskit migrate` writes the current version.
	#[serde(default = "default_schema_version")]
	#[cfg_attr(feature = "schema", schemars(schema_with = "schema::version"))]
	pub schema_version: u32,
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
	/// The published RPC: what `leptoskit rpc` describes and generates
	/// clients for.
	pub rpc: RpcConfig,
	/// File routes. The router is on when the table is present, even empty.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub router: Option<RouterConfig>,
	/// Translations. Message generation is on when the table is present,
	/// even empty.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub l10n: Option<L10nConfig>,
	/// The database. `leptoskit db` is on when the table is present, even
	/// empty.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub db: Option<DbConfig>,
	/// Sign-in. Present when the app uses `leptoskit::auth`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub auth: Option<AuthConfig>,
	/// The leptos and leptoskit plugins activated for this project.
	pub plugins: Vec<PluginSpec>,
	/// How `leptoskit format` lays out markup. Omitted from serialized
	/// configuration while it holds the defaults.
	#[serde(skip_serializing_if = "FormatConfig::is_default")]
	pub format: FormatConfig,
	/// What `leptoskit lint` checks. Omitted from serialized configuration
	/// while it holds the defaults.
	#[serde(skip_serializing_if = "LintConfig::is_default")]
	pub lint: LintConfig,
}

impl Default for LeptoskitConfig {
	fn default() -> Self {
		Self {
			schema_version: SCHEMA_VERSION,
			app: AppConfig::default(),
			serve: ServeConfig::default(),
			build: BuildConfig::default(),
			bundle: BundleConfig::default(),
			native: NativeConfig::default(),
			codepush: CodePushConfig::default(),
			rpc: RpcConfig::default(),
			router: None,
			l10n: None,
			db: None,
			auth: None,
			plugins: Vec::new(),
			format: FormatConfig::default(),
			lint: LintConfig::default(),
		}
	}
}

/// The application identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct AppConfig {
	/// The package name; defaults to the directory name.
	pub name: String,
	/// The semantic version of the app.
	pub version: semver::Version,
	/// The platform `leptoskit build` targets by default. `leptoskit dev` runs
	/// every enabled platform this machine can run.
	pub default_platform: Platform,
	/// Enabled application destinations. Each build selects one platform;
	/// `leptoskit dev` runs all of them that this machine can run.
	pub platforms: Vec<Platform>,
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
			platforms: Platform::APPLICATION.to_vec(),
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Platform {
	/// The browser build: wasm + SSR server.
	#[default]
	Web,
	/// An iOS application.
	Ios,
	/// An Android application.
	Android,
	/// A macOS application.
	Macos,
	/// A Windows application.
	Windows,
	/// A Linux application.
	Linux,
	/// A HarmonyOS application.
	Harmonyos,
	/// A terminal application: the shared tree in an interactive cell grid.
	Tui,
	/// Native apps: wasm UI bundle on platform shells.
	Native,
	/// The server binary only.
	Server,
}

impl Platform {
	/// Destinations generated by default, in their stable display order.
	pub const APPLICATION: [Self; 6] = [
		Self::Web,
		Self::Ios,
		Self::Android,
		Self::Macos,
		Self::Windows,
		Self::Linux,
	];

	/// The Cargo feature selecting this destination in a portable app crate.
	#[must_use]
	pub fn feature(self) -> Option<&'static str> {
		match self {
			Self::Web => Some("web"),
			Self::Ios => Some("ios"),
			Self::Android => Some("android"),
			Self::Macos => Some("macos"),
			Self::Windows => Some("windows"),
			Self::Linux => Some("linux"),
			Self::Harmonyos => Some("harmonyos"),
			Self::Tui => Some("tui"),
			Self::Native | Self::Server => None,
		}
	}

	/// Whether this is a concrete native application destination.
	#[must_use]
	pub fn is_native(self) -> bool {
		matches!(
			self,
			Self::Ios
				| Self::Android
				| Self::Macos
				| Self::Windows
				| Self::Linux
				| Self::Harmonyos
				| Self::Tui
		)
	}
}

impl fmt::Display for Platform {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Web => f.write_str("web"),
			Self::Ios => f.write_str("ios"),
			Self::Android => f.write_str("android"),
			Self::Macos => f.write_str("macos"),
			Self::Windows => f.write_str("windows"),
			Self::Linux => f.write_str("linux"),
			Self::Harmonyos => f.write_str("harmonyos"),
			Self::Tui => f.write_str("tui"),
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
			"ios" => Ok(Self::Ios),
			"android" => Ok(Self::Android),
			"macos" => Ok(Self::Macos),
			"windows" => Ok(Self::Windows),
			"linux" => Ok(Self::Linux),
			"harmonyos" => Ok(Self::Harmonyos),
			"tui" => Ok(Self::Tui),
			"native" => Ok(Self::Native),
			"server" => Ok(Self::Server),
			other => Err(ConfigError::UnknownPlatform(other.to_owned())),
		}
	}
}

/// How `leptoskit format` lays out the markup inside `view!` macros.
///
/// Width and indentation are not set here: they come from the project's
/// `rustfmt.toml`, so markup and the Rust around it always agree.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct FormatConfig {
	/// The macros whose markup is formatted, matched against the path written
	/// at the call site: `view` matches `view! {}` but not `leptos::view! {}`.
	pub macros: Vec<String>,
	/// Whether braces around attribute values are kept or removed.
	pub attribute_braces: AttributeBraces,
	/// How elements without children are closed.
	pub closing_tags: ClosingTags,
	/// The rustfmt toolchain that formats the Rust around and inside markup:
	/// `nightly` by default, so unstable `rustfmt.toml` options apply,
	/// `stable`, or any installed rustup toolchain name.
	pub rustfmt_toolchain: String,
}

impl Default for FormatConfig {
	fn default() -> Self {
		Self {
			macros: [
				"view",
				"leptos::view",
				"leptos::prelude::view",
				"leptoskit::view",
				"leptoskit::prelude::view",
				"leptoskit_macros::view",
			]
			.map(String::from)
			.to_vec(),
			attribute_braces: AttributeBraces::default(),
			closing_tags: ClosingTags::default(),
			rustfmt_toolchain: "nightly".to_owned(),
		}
	}
}

impl FormatConfig {
	/// Whether every setting holds its default.
	#[must_use]
	pub fn is_default(&self) -> bool {
		*self == Self::default()
	}

	/// Reads the `[format]` table of a `leptoskit.toml`, ignoring the other
	/// tables so formatting works while the rest of the file is being edited.
	///
	/// # Errors
	///
	/// Returns an error when the file cannot be read, is not valid TOML, or
	/// its `[format]` table holds an unknown key or value.
	pub fn load_from_file(path: &Path) -> Result<Self, ConfigError> {
		#[derive(Deserialize)]
		struct Document {
			#[serde(default)]
			format: FormatConfig,
		}

		let raw = std::fs::read_to_string(path).map_err(|source| {
			ConfigError::Read {
				path: path.to_owned(),
				source,
			}
		})?;
		toml::from_str::<Document>(&raw)
			.map(|document| document.format)
			.map_err(|source| {
				ConfigError::Parse {
					path: path.to_owned(),
					source,
				}
			})
	}
}

/// Whether the formatter keeps the braces around attribute values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AttributeBraces {
	/// Remove braces the parser does not need: `class={name}` becomes
	/// `class=name`, while `value={a > b}` keeps them.
	#[default]
	WhenRequired,
	/// Brace every value: `class=name` becomes `class={name}`.
	Always,
	/// Brace every value except literals: `id="a"` stays unbraced.
	AlwaysUnlessLiteral,
	/// Keep each value as written.
	Preserve,
}

/// How the formatter closes elements that have no children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ClosingTags {
	/// `<div></div>` becomes `<div />`. Void elements such as `<br />` are
	/// always self-closing.
	#[default]
	SelfClosing,
	/// `<div />` becomes `<div></div>`, except for void elements.
	Explicit,
	/// Keep each element as written.
	Preserve,
}

/// What `leptoskit lint` checks, from the `[lint]` table.
///
/// ```toml
/// [lint]
/// markup = "view"
/// exclude = ["tests/fixtures/**"]
///
/// [lint.rules]
/// component_attribute = "deny"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct LintConfig {
	/// How the project writes component trees.
	pub markup: MarkupStyle,
	/// Whether `leptoskit lint` runs `cargo clippy` beside its own rules.
	pub clippy: bool,
	/// Paths the leptoskit rules skip, as globs relative to the directory of
	/// this `leptoskit.toml`, such as `tests/fixtures/**`.
	pub exclude: Vec<String>,
	/// The level each rule reports at.
	pub rules: LintRules,
}

impl Default for LintConfig {
	fn default() -> Self {
		Self {
			markup: MarkupStyle::default(),
			clippy: true,
			exclude: Vec::new(),
			rules: LintRules::default(),
		}
	}
}

impl LintConfig {
	/// Whether every setting holds its default.
	#[must_use]
	pub fn is_default(&self) -> bool {
		*self == Self::default()
	}

	/// Reads the `[lint]` table of a `leptoskit.toml`, ignoring the other
	/// tables so linting works while the rest of the file is being edited.
	///
	/// # Errors
	///
	/// Returns an error when the file cannot be read, is not valid TOML, or
	/// its `[lint]` table holds an unknown key or value.
	pub fn load_from_file(path: &Path) -> Result<Self, ConfigError> {
		#[derive(Deserialize)]
		struct Document {
			#[serde(default)]
			lint: LintConfig,
		}

		let raw = std::fs::read_to_string(path).map_err(|source| {
			ConfigError::Read {
				path: path.to_owned(),
				source,
			}
		})?;
		toml::from_str::<Document>(&raw)
			.map(|document| document.lint)
			.map_err(|source| {
				ConfigError::Parse {
					path: path.to_owned(),
					source,
				}
			})
	}
}

/// How a project writes its component trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum MarkupStyle {
	/// Markup in `view!`: `view! { <Text class="p-4">"Hi"</Text> }`.
	#[default]
	View,
	/// Builder calls: `Text("Hi").class("p-4")`.
	Builder,
	/// Either, mixed freely.
	Any,
}

/// The level of each leptoskit lint rule.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct LintRules {
	/// A capitalized function that returns a view is a component, so it
	/// carries `#[component]`.
	pub component_attribute: LintLevel,
	/// Component trees are written the way `markup` says.
	pub markup_style: LintLevel,
	/// A `View` with no classes, props, or handlers does not wrap a single
	/// child.
	pub redundant_view: LintLevel,
	/// Space between siblings comes from `gap-*` or `Spacer()`, not an empty
	/// sized `View`.
	pub spacer_view: LintLevel,
	/// Builder trees set their properties before their children.
	pub children_last: LintLevel,
	/// Every `Image` has an accessible name or is marked decorative.
	pub image_label: LintLevel,
	/// Colors come from the theme rather than hand-written values. Off by
	/// default.
	pub color_literal: LintLevel,
}

impl Default for LintRules {
	fn default() -> Self {
		Self {
			component_attribute: LintLevel::Warn,
			markup_style: LintLevel::Warn,
			redundant_view: LintLevel::Warn,
			spacer_view: LintLevel::Warn,
			children_last: LintLevel::Warn,
			image_label: LintLevel::Warn,
			color_literal: LintLevel::Allow,
		}
	}
}

/// How a lint rule reports what it finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LintLevel {
	/// Not checked.
	Allow,
	/// Reported without failing the run, unless warnings are denied.
	Warn,
	/// Reported, and fails the run.
	Deny,
}

/// Server settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct BundleConfig {
	/// How hard release builds optimize the wasm client for size.
	pub optimize: Optimize,
	/// The cargo profile release builds compile the wasm client with, in
	/// place of the one [`optimize`](Self::optimize) picks: the escape hatch
	/// for full control from your `Cargo.toml`.
	pub cargo_profile: Option<String>,
	/// Whether the client is split with wasm-split: every `#[lazy]` function,
	/// lazy route, and `#[island(lazy)]` moves into its own wasm chunk that
	/// loads the first time it is needed, instead of with the page. Enabled
	/// by default; clients without lazy code skip the splitting phase.
	pub split: bool,
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
			split: true,
			wasm_opt: true,
			precompress: true,
			budget_kib: None,
		}
	}
}

/// How hard a release build optimizes the wasm client for size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct NativeConfig {
	/// The app-owned native capability contract. Change this when a store
	/// release changes plugin grants or native behavior incompatibly.
	pub update_contract: String,
	/// Runtime plugins explicitly granted to the generated native app.
	/// Each name must match metadata on a direct Cargo dependency.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub plugins: Vec<String>,
	/// Host-owned options for explicitly selected runtime plugins. These values
	/// are embedded in native registration, never read from guest requests.
	#[serde(rename = "plugin-options", skip_serializing_if = "BTreeMap::is_empty")]
	#[cfg_attr(feature = "schema", schemars(schema_with = "schema::plugin_options"))]
	pub plugin_options: BTreeMap<String, toml::Table>,
	/// What a build does when a selected plugin has no implementation the
	/// platform's host can run. `warn` (the default) leaves it out with a
	/// warning, so its calls fail with `Unavailable`; `error` stops the build.
	#[serde(
		rename = "missing-plugins",
		skip_serializing_if = "MissingPlugins::is_default"
	)]
	pub missing_plugins: MissingPlugins,
	/// The reverse-domain application id used by platform shells, such as
	/// `com.example.counter`.
	///
	/// It must have at least two dot-separated segments, each an ASCII letter
	/// followed by ASCII letters or digits: the only form both iOS bundle ids
	/// (no `_`) and Android package names (no `-`) accept. When unset it
	/// defaults to [`default_package_id`] of the app name.
	#[cfg_attr(
		feature = "schema",
		schemars(regex(pattern = r"^(?:$|[A-Za-z][A-Za-z0-9]*(?:\.[A-Za-z][A-Za-z0-9]*)+)$"))
	)]
	pub package_id: String,
	/// Legacy native target selection. New projects use `app.platforms`.
	#[serde(skip_serializing_if = "NativeConfig::legacy_default_targets")]
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
			plugin_options: BTreeMap::new(),
			missing_plugins: MissingPlugins::default(),
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

impl NativeConfig {
	fn legacy_default_targets(targets: &[NativeTarget]) -> bool {
		targets
			== [
				NativeTarget::Macos,
				NativeTarget::Ios,
				NativeTarget::Android,
			]
	}
}

/// What a native build does with a selected plugin that has no
/// implementation on the platform being built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum MissingPlugins {
	/// Leave the plugin out and print a warning. Its calls fail with
	/// `Unavailable`.
	#[default]
	Warn,
	/// Stop the build.
	Error,
}

impl MissingPlugins {
	#[allow(clippy::trivially_copy_pass_by_ref)]
	fn is_default(&self) -> bool {
		*self == Self::default()
	}
}

/// A native platform shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum NativeTarget {
	/// macOS.
	Macos,
	/// iOS.
	Ios,
	/// Android.
	Android,
	/// HarmonyOS (ArkUI, packaged as a HAP).
	Harmonyos,
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
			Self::Harmonyos => f.write_str("harmonyos"),
			Self::Windows => f.write_str("windows"),
			Self::Linux => f.write_str("linux"),
		}
	}
}

impl std::str::FromStr for NativeTarget {
	type Err = ConfigError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		match value {
			"macos" => Ok(Self::Macos),
			"ios" => Ok(Self::Ios),
			"android" => Ok(Self::Android),
			"harmonyos" => Ok(Self::Harmonyos),
			"windows" => Ok(Self::Windows),
			"linux" => Ok(Self::Linux),
			other => Err(ConfigError::UnknownPlatform(other.to_owned())),
		}
	}
}

/// Over-the-air update settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
	#[cfg_attr(feature = "schema", schemars(schema_with = "schema::public_keys"))]
	pub public_keys: Vec<String>,
}

/// The published RPC, as `leptoskit rpc` describes it and generates clients
/// for it.
///
/// Which functions are published is decided in code, with
/// `#[leptoskit_rpc::api]`; this section names the RPC, excludes functions by
/// pattern, and says where generated clients go.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct RpcConfig {
	/// The RPC title; defaults to `<app.name> RPC`.
	pub title: Option<String>,
	/// A longer description for the OpenAPI document.
	pub description: Option<String>,
	/// The origins the RPC is served from. The first one is the default the
	/// generated clients point at; none means the page's own origin.
	pub servers: Vec<String>,
	/// Function-name globs (`*` and `?`) to leave out of everything generated,
	/// in addition to `#[api(internal)]`.
	pub exclude: Vec<String>,
	/// Whether `#[api(internal)]` functions are generated too. Off for a
	/// public package; on for an internal one.
	pub include_internal: bool,
	/// What `leptoskit rpc generate` writes when no `--target` is given. Empty
	/// means every target, into its default directory.
	pub targets: Vec<RpcTarget>,
}

/// One client `leptoskit rpc generate` writes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct RpcTarget {
	/// The target: `typescript`, `python`, `go`, `rust`, `dart`, `cli`, or
	/// `openapi`.
	pub language: String,
	/// The output directory, relative to the project root; each target has a
	/// default under `clients/`.
	pub out: Option<String>,
	/// The package name: the npm package, the PyPI distribution, the Go
	/// module, the cargo crate, or the Dart package.
	pub package: Option<String>,
	/// The origin this client points at by default; overrides `rpc.servers`.
	pub server_url: Option<String>,
	/// How much regeneration writes: `create` (code always, scaffolding such
	/// as manifests and READMEs only when missing — the default), `full`
	/// (everything, overwriting), or `files` (code only; the package is the
	/// user's to define).
	pub scaffold: Option<String>,
}

/// The targets `rpc.targets[].language` accepts.
pub const RPC_TARGETS: &[&str] = &[
	"typescript",
	"python",
	"go",
	"rust",
	"dart",
	"cli",
	"openapi",
];

/// The modes `rpc.targets[].scaffold` accepts.
pub const RPC_SCAFFOLDS: &[&str] = &["create", "full", "files"];

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

/// File routes (legacy table; the router plugin is preferred): `leptoskit
/// plugin:router routes`, `dev` and `build` read the routes
/// directory and write the typed `Route` module beside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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

/// Translations: `leptoskit l10n`, `dev` and `build` read the ARB files in
/// the locales directory and write the typed translations module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct L10nConfig {
	/// The directory of ARB files, relative to the project root: one per
	/// locale, named after its language tag, such as `en.arb` and
	/// `pt-BR.arb`.
	pub dir: PathBuf,
	/// The generated module, relative to the project root. Declare it where
	/// it sits, such as `mod l10n;` in `src/lib.rs`.
	pub output: PathBuf,
	/// The locale whose file defines every message and the types of its
	/// placeholders; the other files translate it.
	pub template: String,
}

impl Default for L10nConfig {
	fn default() -> Self {
		Self {
			dir: PathBuf::from("locales"),
			output: PathBuf::from("src/l10n.rs"),
			template: "en".to_owned(),
		}
	}
}

/// The database: `leptoskit db` exports the tables declared in `crate`,
/// diffs them with the latest migration's snapshot, and writes new
/// migrations to `migrations`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct DbConfig {
	/// The SQL dialect every migration is written for.
	pub dialect: DbDialect,
	/// The package whose `#[db::table]` structs are the schema. Defaults to
	/// the app's name followed by `_db`, such as `my_app_db`.
	#[serde(rename = "crate", skip_serializing_if = "Option::is_none")]
	pub package: Option<String>,
	/// The package features that compile its server side, so that its tables
	/// export.
	pub features: Vec<String>,
	/// The migrations directory, relative to the project root.
	pub migrations: PathBuf,
	/// The environment variable holding the database URL for `leptoskit db
	/// migrate` and `leptoskit db status`.
	pub url_env: String,
}

impl Default for DbConfig {
	fn default() -> Self {
		Self {
			dialect: DbDialect::Postgres,
			package: None,
			features: vec!["server".to_owned()],
			migrations: PathBuf::from("migrations"),
			url_env: "DATABASE_URL".to_owned(),
		}
	}
}

impl DbConfig {
	/// The package that declares the tables.
	pub fn package(&self, app: &AppConfig) -> String {
		self.package
			.clone()
			.unwrap_or_else(|| format!("{}_db", app.name.replace('-', "_")))
	}
}

/// Sign-in, through `leptoskit::auth`. Secrets are never written here:
/// each one is named by the environment variable that holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct AuthConfig {
	/// The app's name, as emails say it. Defaults to `app.name`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub app_name: Option<String>,
	/// The site's origin, such as `https://myapp.com`. State-changing
	/// requests from anywhere else are refused. Only `localhost` may use
	/// plain `http`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub origin: Option<String>,
	/// The environment variable holding the server's keys.
	pub keys_env: String,
	/// The header a proxy in front of the server puts the client's IP
	/// address in, such as `fly-client-ip`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub client_ip_header: Option<String>,
	/// Days a session lasts unused.
	pub idle_days: u32,
	/// Days a session lasts however often it's used.
	pub absolute_days: u32,
	/// Minutes a native app's access token works before it refreshes.
	pub access_minutes: u32,
	/// How email is sent.
	#[serde(skip_serializing_if = "AuthMailConfig::is_default")]
	pub mail: AuthMailConfig,
}

impl Default for AuthConfig {
	fn default() -> Self {
		Self {
			app_name: None,
			origin: None,
			keys_env: "AUTH_ENCRYPTION_KEYS".to_owned(),
			client_ip_header: None,
			idle_days: 14,
			absolute_days: 90,
			access_minutes: 15,
			mail: AuthMailConfig::default(),
		}
	}
}

/// The longest a native app's access token may last. It's checked on every
/// request, so revoking it is immediate either way; a short life limits what
/// a copy caught in a log or a proxy is worth.
const MAX_ACCESS_MINUTES: u32 = 60;

impl AuthConfig {
	/// Why the configuration can't work, if it can't.
	fn problem(&self) -> Option<String> {
		if let Some(origin) = &self.origin {
			let trimmed = origin.trim_end_matches('/');
			let local = |authority: &&str| {
				let host = authority.split(':').next().unwrap_or_default();
				host == "localhost" || host == "127.0.0.1"
			};
			let authority = trimmed
				.strip_prefix("https://")
				.or_else(|| trimmed.strip_prefix("http://").filter(local));
			let valid = authority.is_some_and(|authority| {
				!authority.is_empty() && !authority.contains(['/', '?', '#'])
			});

			if !valid {
				return Some(format!(
					"auth.origin `{origin}` must be `https://` and a host, such as \
					 \"https://myapp.com\"; only localhost may use `http://`"
				));
			}
		}

		if self.keys_env.is_empty() {
			return Some("auth.keys_env must name an environment variable".to_owned());
		}

		if self.idle_days == 0 || self.idle_days > self.absolute_days {
			return Some(
				"auth.idle_days must be at least 1 and at most auth.absolute_days".to_owned(),
			);
		}

		if !(1..=MAX_ACCESS_MINUTES).contains(&self.access_minutes) {
			return Some(format!(
				"auth.access_minutes must be between 1 and {MAX_ACCESS_MINUTES}"
			));
		}

		self.mail.problem()
	}
}

/// How sign-in email is sent: over SMTP, or as a signed webhook to a URL
/// you control. With neither, development keeps email in a mailbox at
/// `/_leptoskit/mail`, and a release build refuses to start.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct AuthMailConfig {
	/// Who email comes from, such as `My App <hello@myapp.com>`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub from: Option<String>,
	/// The environment variable holding an SMTP URL, such as
	/// `smtps://user:password@smtp.example.com`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub smtp_url_env: Option<String>,
	/// The environment variable holding the URL that receives each email as
	/// a signed JSON webhook.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub webhook_url_env: Option<String>,
	/// The environment variable holding the webhook's signing secret.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub webhook_secret_env: Option<String>,
}

impl AuthMailConfig {
	fn is_default(&self) -> bool {
		*self == Self::default()
	}

	fn problem(&self) -> Option<String> {
		let names = [
			&self.smtp_url_env,
			&self.webhook_url_env,
			&self.webhook_secret_env,
		];

		if names.into_iter().flatten().any(String::is_empty) {
			return Some("auth.mail's `_env` settings must name environment variables".to_owned());
		}

		match (&self.smtp_url_env, &self.webhook_url_env) {
			(Some(_), Some(_)) => {
				Some("auth.mail sends over SMTP or a webhook, not both".to_owned())
			}
			(Some(_), None) if self.from.is_none() => {
				Some("auth.mail.from is needed to send over SMTP".to_owned())
			}
			(None, Some(_)) if self.webhook_secret_env.is_none() => {
				Some("auth.mail.webhook_secret_env is needed to sign webhooks".to_owned())
			}
			_ => None,
		}
	}
}

/// The SQL dialect a project's migrations are written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum DbDialect {
	/// PostgreSQL, the default.
	#[default]
	Postgres,
	/// SQLite.
	Sqlite,
}

impl DbDialect {
	/// The dialect's name, as the configuration spells it.
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Postgres => "postgres",
			Self::Sqlite => "sqlite",
		}
	}
}

impl fmt::Display for DbDialect {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

impl std::str::FromStr for DbDialect {
	type Err = ConfigError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		match value {
			"postgres" => Ok(Self::Postgres),
			"sqlite" => Ok(Self::Sqlite),
			other => Err(ConfigError::UnknownDbDialect(other.to_owned())),
		}
	}
}

/// Whether `path` is relative and stays inside the project: no root, no
/// `..`, and no `.`.
fn is_inside_project(path: &Path) -> bool {
	path.is_relative()
		&& path.components().next().is_some()
		&& path
			.components()
			.all(|component| matches!(component, std::path::Component::Normal(_)))
}

/// Whether `name` can be a Rust module's name.
fn is_module_name(name: &str) -> bool {
	let mut chars = name.chars();
	chars
		.next()
		.is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
		&& chars.all(|char| char == '_' || char.is_ascii_alphanumeric())
}

/// One activated plugin.
///
/// Names without a `path` resolve against the built-in registry; a `path`
/// points at a plugin crate compiled into the tooling. Leptos plugins —
/// activated as cargo features on the app — are declared with `leptos = true`
/// and forwarded to the app build.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct PluginSpec {
	/// The plugin name.
	pub name: String,
	/// The path to a local plugin crate, when it is not a built-in.
	pub path: Option<PathBuf>,
	/// Whether this is a leptos plugin, activated as an app cargo feature.
	pub leptos: bool,
	/// The executable answering this plugin's hooks, for plugins that are
	/// separate programs. Without it, `leptoskit-plugin-<name>` is looked up
	/// on PATH.
	pub command: Option<String>,
	/// Options passed to the plugin.
	#[cfg_attr(
		feature = "schema",
		schemars(with = "serde_json::Map<String, serde_json::Value>")
	)]
	pub options: toml::Table,
}

impl Default for PluginSpec {
	fn default() -> Self {
		Self {
			name: String::new(),
			path: None,
			leptos: false,
			command: None,
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
		Self::from_toml(&raw, dir)
	}

	/// Parses and validates `leptoskit.toml`'s contents, as if read from the
	/// project rooted at `dir`. A server embeds its configuration with this,
	/// so it needs no file at runtime:
	///
	/// ```ignore
	/// let config = LeptoskitConfig::from_toml(include_str!("../../leptoskit.toml"), Path::new("my_app"))?;
	/// ```
	///
	/// # Errors
	///
	/// As [`load_from`](Self::load_from), except for reading the file.
	pub fn from_toml(raw: &str, dir: &Path) -> Result<Self, ConfigError> {
		let path = dir.join(CONFIG_FILE);
		let config: Self = toml::from_str(raw).map_err(|source| {
			match moved_key(raw) {
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
	/// - [`ConfigError::UnknownRpcTarget`] when a `rpc.targets` entry names a
	///   target `leptoskit rpc generate` cannot write.
	/// - [`ConfigError::UnknownRpcScaffold`] when a `rpc.targets` entry names a
	///   scaffold mode other than `create`, `full`, or `files`.
	/// - [`ConfigError::CodePushPublicKeysMissing`] when code push is enabled
	///   without any `public_keys`.
	/// - [`ConfigError::InvalidCodePushPublicKey`] when a public key is not 64
	///   hex characters.
	/// - [`ConfigError::InvalidPackageId`] when `native.package_id` is set but
	///   is not a valid reverse-domain id.
	/// - [`ConfigError::InvalidRouterDir`] when `router.dir` is absolute,
	///   empty, leaves the project, or has a name that is not a Rust module
	///   name.
	/// - [`ConfigError::InvalidL10nDir`] when `l10n.dir` is absolute, empty, or
	///   leaves the project.
	/// - [`ConfigError::InvalidL10nOutput`] when `l10n.output` is not a `.rs`
	///   file inside the project with a Rust module name.
	/// - [`ConfigError::InvalidL10nTemplate`] when `l10n.template` is not
	///   shaped like a language tag.
	/// - [`ConfigError::InvalidDbMigrations`] when `db.migrations` is absolute,
	///   empty, or leaves the project.
	/// - [`ConfigError::InvalidDbPackage`] when `db.crate` isn't a package
	///   name.
	/// - [`ConfigError::InvalidDbUrlEnv`] when `db.url_env` is empty.
	/// - [`ConfigError::InvalidAuth`] when `[auth]` names no key variable, has
	///   an origin that isn't `https` (or `http` on localhost), lifetimes out
	///   of order, or both or half of a mailer.
	/// - [`ConfigError::UnselectedPluginOptions`] when native plugin options
	///   name a plugin not granted in `native.plugins`.
	pub fn validate(&self) -> Result<(), ConfigError> {
		if self.schema_version != SCHEMA_VERSION {
			return Err(ConfigError::SchemaVersion(self.schema_version));
		}

		if self.app.platforms.is_empty() {
			return Err(ConfigError::NoPlatforms);
		}

		for (index, platform) in self.app.platforms.iter().enumerate() {
			if self.app.platforms[..index].contains(platform) {
				return Err(ConfigError::DuplicatePlatform(*platform));
			}
		}

		self.ensure_platform_enabled(self.app.default_platform)?;

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
		for name in self.native.plugin_options.keys() {
			if !self.native.plugins.contains(name) {
				return Err(ConfigError::UnselectedPluginOptions(name.clone()));
			}
		}
		if !package_id.is_empty()
			&& let Some(reason) = package_id_problem(package_id)
		{
			return Err(ConfigError::InvalidPackageId {
				package_id: package_id.clone(),
				reason,
			});
		}
		for target in &self.rpc.targets {
			if !RPC_TARGETS.contains(&target.language.as_str()) {
				return Err(ConfigError::UnknownRpcTarget(target.language.clone()));
			}
			if let Some(scaffold) = &target.scaffold
				&& !RPC_SCAFFOLDS.contains(&scaffold.as_str())
			{
				return Err(ConfigError::UnknownRpcScaffold(scaffold.clone()));
			}
		}
		if let Some(router) = &self.router {
			let dir = &router.dir;
			// The directory's name is also the generated module's name.
			let module_name = dir
				.file_name()
				.and_then(|name| name.to_str())
				.is_some_and(is_module_name);
			if !is_inside_project(dir) || !module_name {
				return Err(ConfigError::InvalidRouterDir(dir.clone()));
			}
		}
		if let Some(l10n) = &self.l10n {
			if !is_inside_project(&l10n.dir) {
				return Err(ConfigError::InvalidL10nDir(l10n.dir.clone()));
			}
			let output = &l10n.output;
			let module = output
				.extension()
				.is_some_and(|extension| extension == "rs")
				&& output
					.file_stem()
					.and_then(|stem| stem.to_str())
					.is_some_and(is_module_name);
			if !is_inside_project(output) || !module {
				return Err(ConfigError::InvalidL10nOutput(output.clone()));
			}
			let template = &l10n.template;
			let tag_shaped = !template.is_empty()
				&& template.split('-').all(|subtag| {
					!subtag.is_empty() && subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
				});
			if !tag_shaped {
				return Err(ConfigError::InvalidL10nTemplate(template.clone()));
			}
		}
		if let Some(db) = &self.db {
			if !is_inside_project(&db.migrations) {
				return Err(ConfigError::InvalidDbMigrations(db.migrations.clone()));
			}
			if let Some(package) = &db.package {
				let valid = !package.is_empty()
					&& package
						.bytes()
						.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
				if !valid {
					return Err(ConfigError::InvalidDbPackage(package.clone()));
				}
			}
			if db.url_env.is_empty() {
				return Err(ConfigError::InvalidDbUrlEnv);
			}
		}
		if let Some(problem) = self.auth.as_ref().and_then(AuthConfig::problem) {
			return Err(ConfigError::InvalidAuth(problem));
		}
		Ok(())
	}

	/// Rejects a build for a destination the project has disabled.
	///
	/// # Errors
	/// Returns [`ConfigError::DisabledPlatform`] when the destination is
	/// absent.
	pub fn ensure_platform_enabled(&self, platform: Platform) -> Result<(), ConfigError> {
		// Legacy aggregate commands remain available to existing projects;
		// their concrete destination is checked by native tooling
		// before compilation.
		if matches!(platform, Platform::Native | Platform::Server)
			|| self.app.platforms.contains(&platform)
		{
			return Ok(());
		}

		Err(ConfigError::DisabledPlatform(platform))
	}

	/// Loads the configuration for the current directory or the nearest
	/// ancestor that has one, returning the project root alongside it.
	pub fn load() -> Result<(Self, PathBuf), ConfigError> {
		let current = Self::find_root()?;
		let config = Self::load_from(&current)?;
		Ok((config, current))
	}

	/// Finds the nearest project root without parsing its configuration, so
	/// migration can repair a configuration the current loader rejects.
	pub fn find_root() -> Result<PathBuf, ConfigError> {
		let mut current = std::env::current_dir().map_err(ConfigError::Io)?;
		loop {
			if current.join(CONFIG_FILE).is_file() {
				return Ok(current);
			}
			if !current.pop() {
				return Err(ConfigError::NotFound);
			}
		}
	}

	/// Serializes back to TOML.
	pub fn to_toml_string(&self) -> Result<String, ConfigError> {
		let body = toml::to_string_pretty(self).map_err(ConfigError::Serialize)?;
		Ok(format!("#:schema {}\n\n{body}", schema_url()))
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
	/// The configuration needs migration or a newer CLI.
	#[error(
		"unsupported configuration schema_version {0}; this CLI supports {SCHEMA_VERSION}; run \
		 `leptoskit migrate` for older files or upgrade leptoskit for newer files"
	)]
	SchemaVersion(u32),
	/// Native options were supplied without granting the corresponding plugin.
	#[error("native.plugin-options.{0} requires `{0}` in native.plugins")]
	UnselectedPluginOptions(String),
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
	#[error(
		"unknown platform `{0}`; expected web, ios, android, macos, windows, linux, harmonyos, \
		 native, or server"
	)]
	UnknownPlatform(String),
	/// An unknown database dialect was requested.
	#[error("unknown database `{0}`; expected postgres or sqlite")]
	UnknownDbDialect(String),
	/// Every application needs at least one destination.
	#[error("app.platforms must enable at least one platform")]
	NoPlatforms,
	/// A destination occurs more than once in the enabled list.
	#[error("app.platforms contains `{0}` more than once")]
	DuplicatePlatform(Platform),
	/// The requested destination is not enabled.
	#[error("platform `{0}` is disabled; add it to app.platforms in leptoskit.toml")]
	DisabledPlatform(Platform),
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
	/// A `rpc.targets` entry names an unknown target.
	#[error(
		"unknown fns target `{0}`; expected one of typescript, python, go, rust, dart, cli, \
		 openapi"
	)]
	UnknownRpcTarget(String),
	/// A `rpc.targets` entry names an unknown scaffold mode.
	#[error("unknown fns scaffold mode `{0}`; expected create, full, or files")]
	UnknownRpcScaffold(String),
	/// `router.dir` does not name a directory inside the project, or its name
	/// cannot be the generated module's name.
	#[error(
		"invalid router.dir `{}`: name a directory inside the project whose name is a Rust \
		 module name, such as \"src/routes\"",
		.0.display()
	)]
	InvalidRouterDir(PathBuf),
	/// `l10n.dir` does not name a directory inside the project.
	#[error(
		"invalid l10n.dir `{}`: name a directory inside the project, such as \"locales\"",
		.0.display()
	)]
	InvalidL10nDir(PathBuf),
	/// `l10n.output` does not name a `.rs` file inside the project whose name
	/// can be a module's.
	#[error(
		"invalid l10n.output `{}`: name a `.rs` file inside the project whose name is a Rust \
		 module name, such as \"src/l10n.rs\"",
		.0.display()
	)]
	InvalidL10nOutput(PathBuf),
	/// `l10n.template` is not shaped like a language tag.
	#[error(
		"invalid l10n.template `{0}`: name the locale of the template file with its language tag, \
		 such as \"en\" or \"pt-BR\""
	)]
	InvalidL10nTemplate(String),
	/// `db.migrations` does not name a directory inside the project.
	#[error(
		"invalid db.migrations `{}`: name a directory inside the project, such as \"migrations\"",
		.0.display()
	)]
	InvalidDbMigrations(PathBuf),
	/// `db.crate` is not a package name.
	#[error(
		"invalid db.crate `{0}`: name the package that declares the tables, such as \"my_app_db\""
	)]
	InvalidDbPackage(String),
	/// `db.url_env` is empty.
	#[error(
		"invalid db.url_env: name the environment variable that holds the database URL, such as \
		 \"DATABASE_URL\""
	)]
	InvalidDbUrlEnv,
	/// `[auth]` can't work as written.
	#[error("invalid [auth]: {0}")]
	InvalidAuth(String),
	/// An IO error.
	#[error("io error")]
	Io(#[from] std::io::Error),
}
