use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;

use super::Target;
mod async_plugins;
use async_plugins::configure_plugin;
use async_plugins::host_features;
use async_plugins::validate_async;
mod implementation;
#[cfg(test)]
pub use implementation::Capability;
pub use implementation::Constructor;
pub use implementation::Flavour;
pub use implementation::HostCapabilities;
pub use implementation::Implementation;
#[cfg(test)]
pub use implementation::Missing;
pub use implementation::MissingPlugin;
pub use implementation::PlatformRegistration;
pub use implementation::RustRegistration;
use implementation::select_for_target;

#[derive(Debug, Deserialize)]
struct Metadata {
	packages: Vec<Package>,
	workspace_members: Vec<String>,
	resolve: Resolve,
	target_directory: PathBuf,
}
#[derive(Debug, Deserialize)]
struct Package {
	id: String,
	name: String,
	manifest_path: PathBuf,
	metadata: serde_json::Value,
	#[serde(default)]
	dependencies: Vec<PackageDependency>,
}
#[derive(Debug, Deserialize)]
struct PackageDependency {
	name: String,
	rename: Option<String>,
	kind: Option<String>,
}
#[derive(Debug, Deserialize)]
struct Resolve {
	nodes: Vec<Node>,
}
#[derive(Debug, Deserialize)]
struct Node {
	id: String,
	deps: Vec<Dependency>,
}
#[derive(Debug, Deserialize)]
struct Dependency {
	name: String,
	pkg: String,
	#[serde(default)]
	dep_kinds: Vec<DependencyKind>,
}
#[derive(Debug, Deserialize)]
struct DependencyKind {
	kind: Option<String>,
}

/// Validated package metadata. Types are paths, never arbitrary source code.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct Plugin {
	pub name: String,
	pub rust_type: Option<String>,
	pub rust_factory: Option<String>,
	/// Factory taking app data/cache paths and returning an inert
	/// `ServiceInstaller`.
	pub rust_service_factory: Option<String>,
	#[serde(default)]
	pub rust_worker: bool,
	#[serde(default)]
	pub rust_async: bool,
	#[serde(skip)]
	pub options_json: Option<String>,
	pub apple_module: Option<String>,
	pub apple_type: Option<String>,
	#[serde(default)]
	pub apple_context: bool,
	/// Required key names only; the app owns the corresponding consent text.
	#[serde(default)]
	pub apple_usage_descriptions: Vec<String>,
	pub android_type: Option<String>,
	#[serde(default)]
	pub android_context: bool,
	#[serde(default)]
	pub android_permissions: Vec<String>,
	/// Browser extension permissions the plugin's browser side uses, such as
	/// `notifications`. Native builds accept the key and leave it alone; the
	/// extension build reads it from `cargo metadata` and adds the
	/// permissions to the manifest.
	#[serde(default, rename = "extension-permissions")]
	_extension_permissions: Vec<String>,
	#[serde(default)]
	pub android_query_actions: Vec<String>,
	/// Scheme-constrained package visibility for an Android intent action.
	#[serde(default)]
	pub android_query_intents: Vec<AndroidIntentQuery>,
	/// Explicit Maven coordinates, never Gradle expressions or version ranges.
	#[serde(default)]
	pub android_dependencies: Vec<String>,
	/// Exported ArkTS class in `harmonyos/<type>.ts`, constructed by the
	/// generated HarmonyOS registrant.
	pub harmonyos_type: Option<String>,
	/// Passes the ability context to the ArkTS plugin's constructor.
	#[serde(default)]
	pub harmonyos_context: bool,
	/// Per-platform registration, which takes precedence over the shared Rust
	/// fields and the platform's native type.
	pub ios: Option<PlatformRegistration>,
	pub macos: Option<PlatformRegistration>,
	pub android: Option<PlatformRegistration>,
	pub harmonyos: Option<PlatformRegistration>,
	pub windows: Option<PlatformRegistration>,
	pub linux: Option<PlatformRegistration>,
	pub tui: Option<PlatformRegistration>,
	#[serde(skip)]
	pub directory: PathBuf,
	#[serde(skip)]
	pub crate_name: String,
	#[serde(skip)]
	pub package_name: String,
	#[serde(skip)]
	package_id: String,
}

/// A package-visibility query with an explicit action and URI scheme.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AndroidIntentQuery {
	pub action: String,
	pub scheme: String,
}

/// Native code needed by a selected plugin, without a registered capability.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct NativeSupport {
	pub apple_module: Option<String>,
	#[serde(default)]
	pub android: bool,
	#[serde(skip)]
	pub directory: PathBuf,
	#[serde(skip)]
	pub package_name: String,
}

pub struct Registry {
	/// App-owned host package selected for this destination.
	pub package: String,
	pub plugins: Vec<Plugin>,
	pub support: Vec<NativeSupport>,
	pub bridge: PathBuf,
	pub target: PathBuf,
	pub host_features: Vec<String>,
	/// Selected plugins this destination leaves out, and why.
	pub missing: Vec<MissingPlugin>,
}

impl Plugin {
	fn validate_service(&self) -> Result<()> {
		let Some(factory) = &self.rust_service_factory else {
			return Ok(());
		};

		anyhow::ensure!(
			rust_service_path(factory),
			"invalid Rust service factory: use a Rust path without calls or expressions"
		);
		anyhow::ensure!(
			self.rust_type.is_none()
				&& self.rust_factory.is_none()
				&& !self.rust_worker
				&& !self.rust_async,
			"rust-service-factory cannot be combined with rust-type, rust-factory, rust-worker or \
			 rust-async"
		);
		anyhow::ensure!(
			self.platform_tables().all(|(_, table)| table.unavailable),
			"rust-service-factory cannot be combined with per-platform Rust registration"
		);
		Ok(())
	}
}

pub fn resolve(
	root: &Path,
	config: &leptoskit_config::LeptoskitConfig,
	target: Target,
) -> Result<Registry> {
	if let Some((config, skipped)) = without_plugins(config, target) {
		eprintln!(
			"warning: the {} runner can't host runtime plugins yet, so the app runs there without \
			 {skipped}; the app's calls to them fail as unavailable",
			target.platform()
		);
		return resolve(root, &config, target);
	}

	let host_manifest = super::host_dir(root, target).join("Cargo.toml");
	let has_host = host_manifest.is_file();
	let client_feature = if has_host {
		let client = config
			.build
			.client_package
			.as_deref()
			.context("portable native builds require build.client_package")?;
		Some(format!("{client}/{}", target.platform()))
	} else {
		None
	};
	let metadata = cargo_metadata(root, None, target, client_feature.as_deref())?;
	let mut host_config = config.clone();

	if !has_host {
		return resolve_for_target(metadata, &host_config, Some(target));
	}

	// A host is usually a member of the app's workspace. One inside a larger
	// workspace, such as an example in a monorepo, can be a workspace of its
	// own instead, so the larger workspace never builds it without the
	// registrant the CLI writes first.
	let metadata = if metadata
		.packages
		.iter()
		.any(|package| package.manifest_path == host_manifest)
	{
		metadata
	} else {
		cargo_metadata(root, Some(&host_manifest), target, None).context(
			"the platform host must be a member of the app's workspace or a workspace of its own",
		)?
	};
	let host = metadata
		.packages
		.iter()
		.find(|package| package.manifest_path == host_manifest)
		.context("platform host Cargo.toml must be a workspace member")?;
	host_config.app.name.clone_from(&host.name);

	resolve_for_target(metadata, &host_config, Some(target))
}

/// The Rust desktop runners (Linux, Windows, and the terminal) can't host
/// runtime plugins yet. Rather than refuse to build an app that grants some
/// for its other platforms, they build it without them: its configuration
/// with no plugins, and the names left out. `None` for other targets, or
/// when nothing is granted.
fn without_plugins(
	config: &leptoskit_config::LeptoskitConfig,
	target: Target,
) -> Option<(leptoskit_config::LeptoskitConfig, String)> {
	if !matches!(target, Target::Linux | Target::Windows | Target::Tui)
		|| config.native.plugins.is_empty()
	{
		return None;
	}

	let skipped = config.native.plugins.join(", ");
	let mut config = config.clone();
	config.native.plugins.clear();
	config.native.plugin_options.clear();
	Some((config, skipped))
}

/// `cargo metadata` for the target, from the app's workspace or, with
/// `manifest`, from that manifest's own workspace.
fn cargo_metadata(
	root: &Path,
	manifest: Option<&Path>,
	target: Target,
	features: Option<&str>,
) -> Result<Metadata> {
	let mut command = Command::new("cargo");
	command.current_dir(root).args([
		"metadata",
		"--format-version",
		"1",
		"--filter-platform",
		target.rust(),
	]);

	if let Some(manifest) = manifest {
		command.arg("--manifest-path").arg(manifest);
	}

	if let Some(features) = features {
		command.args(["--no-default-features", "--features", features]);
	}

	let output = command.output()?;
	anyhow::ensure!(
		output.status.success(),
		"cargo metadata: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	Ok(serde_json::from_slice(&output.stdout)?)
}

#[cfg(test)]
fn resolve_metadata(
	metadata: Metadata,
	config: &leptoskit_config::LeptoskitConfig,
) -> Result<Registry> {
	resolve_for_target(metadata, config, None)
}

/// Resolve the selected plugins. With a target, the plugins it can't run are
/// left out and recorded in [`Registry::missing`] before native helpers and
/// host features are derived from the rest.
fn resolve_for_target(
	metadata: Metadata,
	config: &leptoskit_config::LeptoskitConfig,
	target: Option<Target>,
) -> Result<Registry> {
	let app = metadata
		.packages
		.iter()
		.find(|package| {
			package.name == config.app.name && metadata.workspace_members.contains(&package.id)
		})
		.context("app.name must name the application's Cargo package")?;
	let node = metadata
		.resolve
		.nodes
		.iter()
		.find(|node| node.id == app.id)
		.context("application dependency graph missing")?;
	let packages: BTreeMap<_, _> = metadata
		.packages
		.iter()
		.map(|package| (&package.id, package))
		.collect();
	let (available, bridge) = available_plugins(node, &packages)?;
	let plugins = selected_plugins(available, config)?;
	let (plugins, missing) = match target {
		Some(target) => select_for_target(plugins, target, config.native.missing_plugins)?,
		None => (plugins, Vec::new()),
	};
	let bridge = bridge.context("native apps need a direct leptoskit_bridge_ffi dependency")?;
	let support = resolve_support(&metadata, &plugins, &bridge)?;
	let host_features = host_features(app, node, &packages, &plugins, target)?;
	Ok(Registry {
		package: app.name.clone(),
		plugins,
		support,
		bridge,
		target: metadata.target_directory,
		host_features,
		missing,
	})
}

fn selected_plugins(
	mut available: BTreeMap<String, Plugin>,
	config: &leptoskit_config::LeptoskitConfig,
) -> Result<Vec<Plugin>> {
	let mut plugins = Vec::new();

	for name in &config.native.plugins {
		let mut plugin = available.remove(name).with_context(|| {
			format!(
				"plugin `{name}` must be declared once and be a direct Cargo dependency with \
				 runtime metadata"
			)
		})?;
		configure_plugin(&mut plugin, config, name)?;
		plugins.push(plugin);
	}

	anyhow::ensure!(
		plugins
			.iter()
			.filter(|plugin| plugin.rust_service_factory.is_some())
			.count()
			<= 1,
		"native.plugins selects multiple rust-service-factory registrations; compose them into \
		 one AppService"
	);
	Ok(plugins)
}

fn normal_dependency(dependency: &Dependency) -> bool {
	dependency.dep_kinds.is_empty() || dependency.dep_kinds.iter().any(|kind| kind.kind.is_none())
}

/// Factory paths are appended to a Cargo alias, so contextual and reserved
/// words are invalid.
fn rust_service_path(path: &str) -> bool {
	const KEYWORDS: &[&str] = &[
		"as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
		"extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
		"mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
		"true", "type", "unsafe", "use", "where", "while", "abstract", "become", "box", "do",
		"final", "gen", "macro", "override", "priv", "try", "typeof", "unsized", "virtual",
		"yield", "_",
	];
	type_path(path, "::") && path.split("::").all(|segment| !KEYWORDS.contains(&segment))
}

fn resolve_support(
	metadata: &Metadata,
	plugins: &[Plugin],
	bridge: &Path,
) -> Result<Vec<NativeSupport>> {
	let packages: BTreeMap<_, _> = metadata
		.packages
		.iter()
		.map(|package| (&package.id, package))
		.collect();
	let nodes: BTreeMap<_, _> = metadata
		.resolve
		.nodes
		.iter()
		.map(|node| (&node.id, node))
		.collect();
	let mut pending: Vec<_> = plugins.iter().map(|plugin| &plugin.package_id).collect();
	let mut seen = BTreeSet::new();
	let mut support = Vec::new();
	// Stable copy destinations and Swift module identities must remain unique,
	// including the bridge and selected runtime packages.
	let mut names = BTreeSet::from(["leptoskit_bridge_ffi".to_owned()]);
	let mut paths = BTreeSet::from([bridge.to_path_buf()]);
	let mut modules = BTreeSet::from(["LeptosKitBridge".to_owned()]);

	register_selected_packages(plugins, &mut names, &mut paths, &mut modules)?;

	while let Some(id) = pending.pop() {
		if !seen.insert(id) {
			continue;
		}

		let package = packages
			.get(id)
			.context("native support dependency package missing")?;

		if let Some(value) = package.metadata.pointer("/leptoskit/native-support") {
			let helper = parse_support(package, value, &mut names, &mut paths, &mut modules)?;
			support.push(helper);
		}

		if let Some(node) = nodes.get(id) {
			pending.extend(
				node.deps
					.iter()
					.filter(|dependency| normal_dependency(dependency))
					.map(|dependency| &dependency.pkg),
			);
		}
	}

	support.sort_by(|left, right| left.package_name.cmp(&right.package_name));
	Ok(support)
}

fn identifier(value: &str, extra: &str) -> bool {
	let mut chars = value.chars();
	chars
		.next()
		.is_some_and(|value| value.is_ascii_alphabetic() || value == '_')
		&& chars.all(|value| value.is_ascii_alphanumeric() || extra.contains(value))
}
fn type_path(value: &str, separator: &str) -> bool {
	value
		.split(separator)
		.all(|segment| identifier(segment, "_"))
}

fn maven_coordinate(value: &str) -> bool {
	let mut parts = value.split(':');
	let (Some(group), Some(artifact), Some(version), None) =
		(parts.next(), parts.next(), parts.next(), parts.next())
	else {
		return false;
	};
	let component = |value: &str| {
		value.starts_with(|character: char| character.is_ascii_alphanumeric())
			&& value.ends_with(|character: char| character.is_ascii_alphanumeric())
			&& value
				.chars()
				.all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
	};
	group.split('.').all(|segment| identifier(segment, "_-"))
		&& component(artifact)
		&& version.starts_with(|character: char| character.is_ascii_digit())
		&& component(version)
}

#[cfg(test)]
mod tests;

/// Validate each language's paths before metadata becomes generated source.
fn validate_plugin(plugin: &Plugin, alias: &str) -> Result<()> {
	anyhow::ensure!(identifier(alias, "_"), "invalid crate alias");
	plugin
		.validate_service()
		.with_context(|| format!("{} runtime metadata", plugin.name))?;
	anyhow::ensure!(
		plugin.rust_type.is_none() || plugin.rust_factory.is_none(),
		"plugin cannot declare both rust-type and rust-factory"
	);
	anyhow::ensure!(
		!plugin.rust_worker || plugin.rust_type.is_some() || plugin.rust_factory.is_some(),
		"rust-worker requires rust-type or rust-factory"
	);
	anyhow::ensure!(
		!plugin.apple_context || (plugin.apple_type.is_some() && plugin.apple_module.is_some()),
		"apple-context requires apple-type and apple-module"
	);
	anyhow::ensure!(
		plugin.apple_type.is_some() == plugin.apple_module.is_some(),
		"apple-type and apple-module must be declared together"
	);
	validate_async(plugin)?;

	for (platform, table) in plugin.platform_tables() {
		table
			.validate(platform)
			.with_context(|| format!("{} runtime metadata", plugin.name))?;
	}

	if let Some(path) = &plugin.rust_factory {
		anyhow::ensure!(type_path(path, "::"), "invalid Rust plugin factory");
	}

	if let Some(path) = &plugin.rust_type {
		anyhow::ensure!(type_path(path, "::"), "invalid Rust plugin type");
	}

	validate_apple_paths(plugin)?;

	validate_android_paths(plugin)?;

	if let Some(name) = &plugin.harmonyos_type {
		anyhow::ensure!(identifier(name, "_"), "invalid ArkTS plugin type");
	}
	anyhow::ensure!(
		!plugin.harmonyos_context || plugin.harmonyos_type.is_some(),
		"harmonyos-context requires harmonyos-type"
	);

	validate_usage_descriptions(plugin)?;

	for dependency in &plugin.android_dependencies {
		anyhow::ensure!(
			maven_coordinate(dependency),
			"invalid Android dependency in plugin {}: expected group:artifact:version with an \
			 explicit numeric-leading version",
			plugin.name
		);
	}

	Ok(())
}

fn validate_apple_paths(plugin: &Plugin) -> Result<()> {
	if let Some(path) = &plugin.apple_type {
		anyhow::ensure!(type_path(path, "."), "invalid Swift plugin type");
	}

	if let Some(path) = &plugin.apple_module {
		anyhow::ensure!(identifier(path, "_"), "invalid Swift module");
	}

	Ok(())
}

fn validate_android_paths(plugin: &Plugin) -> Result<()> {
	if let Some(path) = &plugin.android_type {
		anyhow::ensure!(type_path(path, "."), "invalid Kotlin plugin type");
	}

	for permission in &plugin.android_permissions {
		anyhow::ensure!(type_path(permission, "."), "invalid Android permission");
	}

	for action in plugin.android_query_actions.iter().chain(
		plugin
			.android_query_intents
			.iter()
			.map(|query| &query.action),
	) {
		anyhow::ensure!(
			action.contains('.') && type_path(action, "."),
			"invalid fully qualified Android query action"
		);
	}
	for query in &plugin.android_query_intents {
		let mut bytes = query.scheme.bytes();
		anyhow::ensure!(
			bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
				&& bytes.all(|byte| byte.is_ascii_alphanumeric() || b"+-.".contains(&byte)),
			"invalid Android query URI scheme"
		);
	}

	Ok(())
}

fn validate_usage_descriptions(plugin: &Plugin) -> Result<()> {
	for key in &plugin.apple_usage_descriptions {
		anyhow::ensure!(
			key.starts_with("NS")
				&& key.ends_with("UsageDescription")
				&& key.len() > "NSUsageDescription".len()
				&& identifier(key, ""),
			"invalid Apple usage-description key in plugin {}: {key}",
			plugin.name
		);
	}

	Ok(())
}

/// Direct normal dependencies alone may supply runtime registration.
fn available_plugins(
	node: &Node,
	packages: &BTreeMap<&String, &Package>,
) -> Result<(BTreeMap<String, Plugin>, Option<PathBuf>)> {
	let mut available = BTreeMap::new();
	let mut bridge = None;

	for dependency in &node.deps {
		if !normal_dependency(dependency) {
			continue;
		}

		let package = packages
			.get(&dependency.pkg)
			.context("resolved dependency missing")?;

		if package.name == "leptoskit_bridge_ffi" {
			bridge = package.manifest_path.parent().map(Path::to_path_buf);
		}

		let Some(value) = package.metadata.pointer("/leptoskit/runtime") else {
			continue;
		};
		let mut plugin: Plugin = serde_json::from_value(value.clone())
			.with_context(|| format!("{} runtime metadata", package.name))?;

		validate_plugin(&plugin, &dependency.name)?;

		package
			.manifest_path
			.parent()
			.context("plugin directory missing")?
			.clone_into(&mut plugin.directory);
		plugin.crate_name.clone_from(&dependency.name);
		plugin.package_name.clone_from(&package.name);
		plugin.package_id.clone_from(&package.id);
		let name = plugin.name.clone();
		anyhow::ensure!(
			available.insert(name.clone(), plugin).is_none(),
			"duplicate runtime plugin {name}"
		);
	}

	Ok((available, bridge))
}

fn parse_support(
	package: &Package,
	value: &serde_json::Value,
	names: &mut BTreeSet<String>,
	paths: &mut BTreeSet<PathBuf>,
	modules: &mut BTreeSet<String>,
) -> Result<NativeSupport> {
	anyhow::ensure!(
		package.metadata.pointer("/leptoskit/runtime").is_none(),
		"native support cannot also declare runtime registration"
	);
	let mut helper: NativeSupport = serde_json::from_value(value.clone())
		.with_context(|| format!("{} native-support metadata", package.name))?;
	anyhow::ensure!(
		helper.apple_module.is_some() || helper.android,
		"native support must declare a platform"
	);
	anyhow::ensure!(
		identifier(&package.name, "_-"),
		"invalid native support package name"
	);

	if let Some(module) = &helper.apple_module {
		anyhow::ensure!(
			identifier(module, "_"),
			"invalid native support Swift module"
		);
		anyhow::ensure!(
			modules.insert(module.clone()),
			"duplicate Swift module {module}"
		);
	}

	helper.directory = package
		.manifest_path
		.parent()
		.context("native support directory missing")?
		.to_path_buf();
	anyhow::ensure!(
		helper.directory.is_absolute()
			&& helper.directory.components().all(|component| {
				!matches!(
					component,
					std::path::Component::ParentDir | std::path::Component::CurDir
				)
			}),
		"native support directory must be an absolute normalized path"
	);
	anyhow::ensure!(
		paths.insert(helper.directory.clone()),
		"duplicate native package path"
	);
	anyhow::ensure!(
		names.insert(package.name.clone()),
		"duplicate native package name"
	);
	helper.package_name.clone_from(&package.name);
	Ok(helper)
}

fn register_selected_packages(
	plugins: &[Plugin],
	names: &mut BTreeSet<String>,
	paths: &mut BTreeSet<PathBuf>,
	modules: &mut BTreeSet<String>,
) -> Result<()> {
	for plugin in plugins {
		anyhow::ensure!(
			names.insert(plugin.package_name.clone()),
			"duplicate native package name"
		);
		anyhow::ensure!(
			paths.insert(plugin.directory.clone()),
			"duplicate native package path"
		);

		if let Some(module) = &plugin.apple_module {
			anyhow::ensure!(
				modules.insert(module.clone()),
				"duplicate Swift module {module}"
			);
		}
	}

	Ok(())
}
