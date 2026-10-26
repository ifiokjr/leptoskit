//! The plugin system: one trait, a registry, and the build hooks plugins can
//! intercept.
//!
//! Two kinds of plugins ride the same declaration in `leptoskit.toml`:
//!
//! - **leptoskit plugins** implement [`LeptoskitPlugin`] and are compiled into
//!   the tooling (built-ins, or local crates declared with `path = ...`). They
//!   intercept build hooks: generate code before a build, add cargo features,
//!   set environment variables, register extra assets, post-process the wasm
//!   bundle, and answer their own `plugin:<name>` commands.
//! - **leptos plugins** are declared with `leptos = true` and forwarded to the
//!   app build as cargo features, which is how the leptos plugin ecosystem
//!   activates. Full support means the declaration, the forwarding, and the
//!   options table all survive into the build.
//!
//! The hook order across one `leptoskit build`:
//!
//! 1. [`LeptoskitPlugin::configure`] once per resolved plugin;
//! 2. [`LeptoskitPlugin::generate`] before cargo runs, so generated modules
//!    exist when the app compiles — also on every dev-server rebuild;
//! 3. [`LeptoskitPlugin::before_build`], then cargo, then
//!    [`LeptoskitPlugin::after_build`];
//! 4. [`LeptoskitPlugin::before_serve`] before the dev or production server
//!    starts.
//!
//! [`LeptoskitPlugin::run_command`] backs `plugin:<name> <command>` outside
//! the build pipeline, and [`LeptoskitPlugin::native_integration`] declares
//! the native dependencies a plugin needs installed into a generated host.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

use std::collections::BTreeMap;
use std::path::Path;
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

	/// The plugin's declared surface: its CLI namespace, if any, and the
	/// platforms it supports.
	fn metadata(&self) -> PluginMetadata {
		PluginMetadata::default()
	}

	/// The subcommands this plugin answers under `plugin:<namespace>`.
	fn commands(&self) -> Vec<PluginCommand> {
		Vec::new()
	}

	/// Runs one of [`Self::commands`]. The exit code becomes the CLI's.
	fn run_command(
		&self,
		context: &PluginCommandContext,
		command: &str,
		args: &[String],
	) -> Result<i32, PluginError> {
		let _ = (context, command, args);
		Err(PluginError::Hook {
			plugin: self.name().to_owned(),
			message: "this plugin declares no commands".to_owned(),
		})
	}

	/// Generates code before cargo runs, recording every file written so the
	/// dev watcher can exclude it from re-triggering builds.
	fn generate(&self, context: &mut PluginGenerateContext) -> Result<(), PluginError> {
		let _ = context;
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

	/// The native dependencies a plugin needs installed into a generated
	/// host for `platform`. Returning [`None`] — the default — declares none.
	fn native_integration(&self, platform: Platform) -> Option<NativeIntegration> {
		let _ = platform;
		None
	}
}

/// The plugin surface a project can address: its CLI namespace, if any, and
/// the platforms it supports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginMetadata {
	/// The namespace this plugin's commands live under: `plugin:<namespace>`.
	/// Plugins without one declare no commands.
	pub cli_namespace: Option<String>,
	/// The platforms this plugin supports. Empty means every platform.
	pub platforms: Vec<Platform>,
}

impl PluginMetadata {
	/// Whether the plugin runs on `platform`.
	pub fn supports(&self, platform: Platform) -> bool {
		self.platforms.is_empty() || self.platforms.contains(&platform)
	}
}

/// One subcommand a plugin answers under its namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCommand {
	/// The command name: `plugin:<namespace> <name>`.
	pub name: String,
	/// One line of help, shown by `leptoskit --help` and `plugin:<ns> --help`.
	pub about: String,
	/// The boolean flags the command accepts.
	pub flags: Vec<PluginFlag>,
}

/// One boolean flag of a plugin command, like `routes --check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginFlag {
	/// The flag's long name, without `--`.
	pub name: String,
	/// One line of help for the flag.
	pub help: String,
}

/// The read-only context a plugin command runs in.
#[derive(Debug, Clone)]
pub struct PluginCommandContext {
	/// The project root.
	pub root: PathBuf,
	/// The project configuration.
	pub config: LeptoskitConfig,
}

/// The context a plugin generates code in.
#[derive(Debug, Clone)]
pub struct PluginGenerateContext {
	/// The project root.
	pub root: PathBuf,
	/// The platform being built.
	pub platform: Platform,
	/// Whether this is a release build.
	pub release: bool,
	generated: Vec<PathBuf>,
}

impl PluginGenerateContext {
	/// A context generating under `root`.
	pub fn new(root: impl Into<PathBuf>, platform: Platform, release: bool) -> Self {
		Self {
			root: root.into(),
			platform,
			release,
			generated: Vec::new(),
		}
	}

	/// Records one file the plugin wrote, so the dev watcher can exclude it
	/// from re-triggering builds.
	pub fn generated(&mut self, path: impl Into<PathBuf>) {
		let path = path.into();
		if !self.generated.contains(&path) {
			self.generated.push(path);
		}
	}

	/// Every file recorded through [`Self::generated`].
	pub fn outputs(&self) -> &[PathBuf] {
		&self.generated
	}
}

/// The native dependencies a plugin declares for one platform, merged into a
/// generated host the way runtime-plugin metadata is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeIntegration {
	/// Android permissions requested in the generated manifest.
	pub android_permissions: Vec<String>,
	/// Android dependencies, as Maven coordinates.
	pub android_dependencies: Vec<String>,
	/// Apple usage descriptions keyed by their `NS*UsageDescription` key.
	pub apple_usage_descriptions: BTreeMap<String, String>,
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

	/// The plugin whose CLI namespace is `namespace`, if one is registered.
	pub fn by_namespace(&self, namespace: &str) -> Option<&dyn LeptoskitPlugin> {
		self.plugins
			.iter()
			.find(|plugin| plugin.metadata().cli_namespace.as_deref() == Some(namespace))
			.map(AsRef::as_ref)
	}

	/// Runs `generate` across all plugins in registration order, returning
	/// every file they recorded writing.
	pub fn generate(
		&self,
		root: &Path,
		platform: Platform,
		release: bool,
	) -> Result<Vec<PathBuf>, PluginError> {
		let mut outputs = Vec::new();
		for plugin in &self.plugins {
			let mut context = PluginGenerateContext::new(root, platform, release);
			plugin.generate(&mut context)?;
			outputs.extend(context.outputs().iter().cloned());
		}
		Ok(outputs)
	}

	/// Runs one plugin command. The command must be one the plugin declares
	/// through [`LeptoskitPlugin::commands`]; an undeclared name names the
	/// declared ones in the error.
	pub fn run_command(
		&self,
		root: &Path,
		config: &LeptoskitConfig,
		namespace: &str,
		command: &str,
		args: &[String],
	) -> Result<i32, PluginError> {
		let plugin = self.by_namespace(namespace).ok_or_else(|| {
			PluginError::Hook {
				plugin: namespace.to_owned(),
				message: format!("no plugin with the namespace `{namespace}` is active"),
			}
		})?;
		let declared = plugin.commands();
		if !declared.iter().any(|declared| declared.name == command) {
			let known = if declared.is_empty() {
				"none".to_owned()
			} else {
				declared
					.iter()
					.map(|command| command.name.as_str())
					.collect::<Vec<_>>()
					.join(", ")
			};
			return Err(PluginError::Hook {
				plugin: plugin.name().to_owned(),
				message: format!("no command `{command}`; its commands are {known}"),
			});
		}
		let context = PluginCommandContext {
			root: root.to_owned(),
			config: config.clone(),
		};
		plugin.run_command(&context, command, args)
	}

	/// Fails naming every plugin that does not support `platform`.
	pub fn ensure_supported(&self, platform: Platform) -> Result<(), PluginError> {
		let unsupported = self
			.plugins
			.iter()
			.filter(|plugin| !plugin.metadata().supports(platform))
			.map(|plugin| plugin.name().to_owned())
			.collect::<Vec<_>>();
		if unsupported.is_empty() {
			Ok(())
		} else {
			Err(PluginError::Hook {
				plugin: unsupported.join(", "),
				message: format!("not supported on {platform}"),
			})
		}
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

#[cfg(test)]
mod tests {
	use super::*;

	/// A plugin recording every hook call it receives.
	#[derive(Default)]
	struct Recorder {
		namespace: Option<&'static str>,
		platforms: Vec<Platform>,
		generated: Vec<&'static str>,
	}

	impl LeptoskitPlugin for Recorder {
		fn name(&self) -> &'static str {
			"recorder"
		}

		fn metadata(&self) -> PluginMetadata {
			PluginMetadata {
				cli_namespace: self.namespace.map(str::to_owned),
				platforms: self.platforms.clone(),
			}
		}

		fn commands(&self) -> Vec<PluginCommand> {
			vec![PluginCommand {
				name: "routes".to_owned(),
				about: "writes the typed route module".to_owned(),
				flags: vec![PluginFlag {
					name: "check".to_owned(),
					help: "fail instead of writing".to_owned(),
				}],
			}]
		}

		fn run_command(
			&self,
			context: &PluginCommandContext,
			command: &str,
			args: &[String],
		) -> Result<i32, PluginError> {
			Ok(
				i32::try_from(context.config.plugins.len() + args.len()).unwrap_or(1)
					+ i32::from(command == "routes"),
			)
		}

		fn generate(&self, context: &mut PluginGenerateContext) -> Result<(), PluginError> {
			for file in &self.generated {
				context.generated(format!("{}/{file}", context.root.display()));
			}
			Ok(())
		}
	}

	fn registry_with(plugin: Recorder) -> PluginRegistry {
		let mut registry = PluginRegistry::new();
		registry.register(plugin);
		registry
	}

	#[test]
	fn commands_route_through_the_declared_namespace() {
		let mut registry = PluginRegistry::new();
		registry.register(Recorder {
			namespace: Some("router"),
			..Recorder::default()
		});
		let config = LeptoskitConfig::default();
		let code = registry
			.run_command(Path::new("/tmp/project"), &config, "router", "routes", &[])
			.expect("the declared command runs");
		assert_eq!(code, 1);
	}

	#[test]
	fn undeclared_namespaces_and_commands_fail_with_the_known_names() {
		let mut registry = PluginRegistry::new();
		registry.register(Recorder {
			namespace: Some("router"),
			..Recorder::default()
		});
		let config = LeptoskitConfig::default();

		let error = registry
			.run_command(Path::new("/tmp/project"), &config, "other", "routes", &[])
			.expect_err("no plugin holds the namespace");
		assert!(
			error
				.to_string()
				.contains("no plugin with the namespace `other`")
		);

		let error = registry
			.run_command(Path::new("/tmp/project"), &config, "router", "dev", &[])
			.expect_err("the command is undeclared");
		assert!(error.to_string().contains("its commands are routes"));
	}

	#[test]
	fn generate_collects_the_recorded_outputs_in_registration_order() {
		let mut registry = PluginRegistry::new();
		registry.register(Recorder {
			generated: vec!["src/routes.rs"],
			..Recorder::default()
		});
		registry.register(Recorder {
			namespace: Some("second"),
			generated: vec!["src/other.rs"],
			..Recorder::default()
		});

		let outputs = registry
			.generate(Path::new("/tmp/project"), Platform::Web, false)
			.expect("both plugins generate");
		assert_eq!(outputs.len(), 2, "{outputs:?}");
		assert!(outputs[0].ends_with("src/routes.rs"));
		assert!(outputs[1].ends_with("src/other.rs"));
	}

	#[test]
	fn platform_support_defaults_to_every_platform_and_can_be_limited() {
		let registry = registry_with(Recorder::default());
		for platform in Platform::APPLICATION {
			registry
				.ensure_supported(platform)
				.expect("empty means all");
		}

		let registry = registry_with(Recorder {
			platforms: vec![Platform::Web],
			..Recorder::default()
		});
		registry.ensure_supported(Platform::Web).expect("declared");
		let error = registry
			.ensure_supported(Platform::Ios)
			.expect_err("undeclared platform");
		assert!(error.to_string().contains("recorder"), "{error}");
	}
}
