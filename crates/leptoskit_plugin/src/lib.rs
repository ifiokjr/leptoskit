//! The plugin system: one trait, a registry, and the build hooks plugins can
//! intercept.
//!
//! Two kinds of plugins ride the same declaration in `leptoskit.toml`:
//!
//! - **leptoskit plugins** implement [`LeptoskitPlugin`] and are compiled into
//!   the tooling (built-ins, or local crates declared with `path = ...`). They
//!   intercept build hooks: add cargo features, set environment variables,
//!   register extra assets, and post-process the wasm bundle.
//! - **leptos plugins** are declared with `leptos = true` and forwarded to the
//!   app build as cargo features, which is how the leptos plugin ecosystem
//!   activates. Full support means the declaration, the forwarding, and the
//!   options table all survive into the build.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;
use leptoskit_config::PluginSpec;
use serde_json::Value;

/// The hooks a leptoskit plugin can intercept.
///
/// Every method has a default no-op implementation, so a plugin implements
/// only what it needs.
pub trait LeptoskitPlugin: Send + Sync {
	/// The plugin name, matching its `leptoskit.toml` declaration.
	fn name(&self) -> &str;

	/// The options from `leptoskit.toml`, if the plugin wants typed access.
	fn configure(&self, options: Value) -> Result<(), PluginError> {
		let _ = options;
		Ok(())
	}

	/// Runs before a build starts.
	fn before_build(&self, context: &mut PluginBuildContext) -> Result<(), PluginError> {
		let _ = context;
		Ok(())
	}

	/// Runs after a build finishes, with the produced artifacts.
	fn after_build(&self, context: &mut PluginPostBuildContext) -> Result<(), PluginError> {
		let _ = context;
		Ok(())
	}

	/// Runs before the dev or production server starts.
	fn before_serve(&self, context: &PluginBuildContext) -> Result<(), PluginError> {
		let _ = context;
		Ok(())
	}
}

/// The mutable build context plugins write into before a build or serve.
#[derive(Debug, Clone)]
pub struct PluginBuildContext {
	/// The project configuration.
	pub config: LeptoskitConfig,
	/// The platform being built.
	pub platform: Platform,
	/// Whether this is a release build.
	pub release: bool,
	/// Extra cargo features for the app build.
	pub cargo_features: Vec<String>,
	/// Extra environment variables for the build and the app.
	pub env: BTreeMap<String, String>,
	/// Extra rustc flags.
	pub rustflags: Vec<String>,
	/// Extra asset directories copied into the bundle.
	pub asset_dirs: Vec<PathBuf>,
}

impl PluginBuildContext {
	/// Creates a context from the project configuration.
	pub fn new(config: LeptoskitConfig, platform: Platform, release: bool) -> Self {
		Self {
			config,
			platform,
			release,
			cargo_features: Vec::new(),
			env: BTreeMap::new(),
			rustflags: Vec::new(),
			asset_dirs: Vec::new(),
		}
	}

	/// Adds a cargo feature unless it is already present.
	pub fn add_feature(&mut self, feature: impl Into<String>) {
		let feature = feature.into();
		if !self.cargo_features.contains(&feature) {
			self.cargo_features.push(feature);
		}
	}

	/// Sets an environment variable.
	pub fn set_env(&mut self, key: impl Into<String>, value: impl Into<String>) {
		self.env.insert(key.into(), value.into());
	}
}

/// The read-only post-build context.
#[derive(Debug, Clone)]
pub struct PluginPostBuildContext {
	/// The project configuration.
	pub config: LeptoskitConfig,
	/// The platform that was built.
	pub platform: Platform,
	/// Whether this was a release build.
	pub release: bool,
	/// The build output directory.
	pub output_dir: PathBuf,
	/// The wasm bundle path, when the platform has one.
	pub wasm_bundle: Option<PathBuf>,
}

/// The set of active plugins.
#[derive(Clone, Default)]
pub struct PluginRegistry {
	plugins: Vec<Arc<dyn LeptoskitPlugin>>,
}

impl PluginRegistry {
	/// An empty registry.
	pub fn new() -> Self {
		Self::default()
	}

	/// Registers a plugin instance.
	pub fn register(&mut self, plugin: impl LeptoskitPlugin + 'static) {
		self.plugins.push(Arc::new(plugin));
	}

	/// The registered plugin names.
	pub fn names(&self) -> Vec<&str> {
		self.plugins.iter().map(|plugin| plugin.name()).collect()
	}

	/// The number of registered plugins.
	pub fn len(&self) -> usize {
		self.plugins.len()
	}

	/// Whether no plugins are registered.
	pub fn is_empty(&self) -> bool {
		self.plugins.is_empty()
	}

	/// Resolves the plugin declarations in a configuration.
	///
	/// Built-in names resolve against [`Builtins`] and are configured with
	/// their options; any other name is returned as an [`UnresolvedPlugin`]
	/// so the caller can compile it in. Leptos plugin declarations
	/// (`leptos = true`) are skipped entirely: they are neither loaded nor
	/// checked here, because the build forwards them to the app as cargo
	/// features.
	///
	/// # Errors
	///
	/// Returns the first error a built-in raises while configuring.
	pub fn resolve(config: &LeptoskitConfig) -> Result<(Self, Vec<UnresolvedPlugin>), PluginError> {
		let mut registry = Self::new();
		let mut unresolved = Vec::new();
		for spec in &config.plugins {
			if spec.leptos {
				// Leptos plugins activate through the app's cargo features;
				// nothing to load into the tooling.
				continue;
			}
			match Builtins::by_name(&spec.name) {
				Some(builtin) => {
					let options = toml_options(&spec.options);
					builtin.configure(options)?;
					registry.plugins.push(builtin);
				}
				None => unresolved.push(UnresolvedPlugin { spec: spec.clone() }),
			}
		}
		Ok((registry, unresolved))
	}

	/// Runs `before_build` across all plugins in registration order.
	pub fn before_build(&self, context: &mut PluginBuildContext) -> Result<(), PluginError> {
		for plugin in &self.plugins {
			plugin.before_build(context)?;
		}
		Ok(())
	}

	/// Runs `after_build` across all plugins in registration order.
	pub fn after_build(&self, context: &mut PluginPostBuildContext) -> Result<(), PluginError> {
		for plugin in &self.plugins {
			plugin.after_build(context)?;
		}
		Ok(())
	}

	/// Runs `before_serve` across all plugins in registration order.
	pub fn before_serve(&self, context: &PluginBuildContext) -> Result<(), PluginError> {
		for plugin in &self.plugins {
			plugin.before_serve(context)?;
		}
		Ok(())
	}
}

impl std::fmt::Debug for PluginRegistry {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("PluginRegistry")
			.field("names", &self.names())
			.finish()
	}
}

/// A plugin declaration that must be compiled into the tooling to run.
#[derive(Debug, Clone, PartialEq)]
pub struct UnresolvedPlugin {
	/// The declaration from `leptoskit.toml`.
	pub spec: PluginSpec,
}

/// Converts a TOML options table to JSON for [`LeptoskitPlugin::configure`].
pub fn toml_options(table: &toml::Table) -> Value {
	serde_json::to_value(table).unwrap_or(Value::Null)
}

/// The built-in plugin set.
pub struct Builtins;

impl Builtins {
	/// The names of every built-in plugin.
	pub const NAMES: &'static [&'static str] = &["env_manifest", "size_report"];

	/// Resolves a built-in plugin by name.
	#[must_use]
	pub fn by_name(name: &str) -> Option<Arc<dyn LeptoskitPlugin>> {
		match name {
			"env_manifest" => Some(Arc::new(EnvManifestPlugin)),
			"size_report" => Some(Arc::new(SizeReportPlugin)),
			_ => None,
		}
	}
}

/// Emits `LEPTOSKIT_PLUGIN_*` environment variables describing the build, so
/// apps and downstream plugins can branch on them at compile time.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvManifestPlugin;

impl LeptoskitPlugin for EnvManifestPlugin {
	fn name(&self) -> &'static str {
		"env_manifest"
	}

	fn before_build(&self, context: &mut PluginBuildContext) -> Result<(), PluginError> {
		context.set_env("LEPTOSKIT_PLUGIN_PLATFORM", context.platform.to_string());
		context.set_env(
			"LEPTOSKIT_PLUGIN_RELEASE",
			if context.release { "1" } else { "0" },
		);
		Ok(())
	}
}

/// Prints the wasm bundle size after native builds, where every kilobyte
/// travels over the air.
#[derive(Debug, Clone, Copy, Default)]
pub struct SizeReportPlugin;

impl LeptoskitPlugin for SizeReportPlugin {
	fn name(&self) -> &'static str {
		"size_report"
	}

	fn after_build(&self, context: &mut PluginPostBuildContext) -> Result<(), PluginError> {
		if let Some(bundle) = &context.wasm_bundle {
			let size = std::fs::metadata(bundle).map_or(0, |metadata| metadata.len());
			println!(
				"[size_report] {} bundle: {:.1} KiB",
				bundle.display(),
				size as f64 / 1024.0
			);
		}
		Ok(())
	}
}

/// Errors raised by plugins.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
	/// The plugin rejected its options.
	#[error("plugin `{plugin}` rejected its options: {message}")]
	Configure {
		/// The plugin name.
		plugin: String,
		/// What was wrong.
		message: String,
	},
	/// A hook failed.
	#[error("plugin `{plugin}` failed: {message}")]
	Hook {
		/// The plugin name.
		plugin: String,
		/// What failed.
		message: String,
	},
	/// An IO error.
	#[error("io error")]
	Io(#[from] std::io::Error),
}
