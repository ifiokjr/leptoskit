use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;

use super::Target;

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
// Each flag mirrors one `[package.metadata.leptoskit]` key; `load` rejects the
// invalid combinations (`rust-async` with `rust-worker`).
#[allow(clippy::struct_excessive_bools)]
pub struct Plugin {
	pub name: String,
	pub rust_type: Option<String>,
	pub rust_factory: Option<String>,
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
	#[serde(default)]
	pub android_query_actions: Vec<String>,
	/// Explicit Maven coordinates, never Gradle expressions or version ranges.
	#[serde(default)]
	pub android_dependencies: Vec<String>,
	#[serde(skip)]
	pub directory: PathBuf,
	#[serde(skip)]
	pub crate_name: String,
	#[serde(skip)]
	pub package_name: String,
	#[serde(skip)]
	package_id: String,
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
}

pub fn resolve(
	root: &Path,
	config: &leptoskit_config::LeptoskitConfig,
	target: Target,
) -> Result<Registry> {
	let host_manifest = super::host_dir(root, target).join("Cargo.toml");
	let mut command = Command::new("cargo");
	command.current_dir(root).args([
		"metadata",
		"--format-version",
		"1",
		"--filter-platform",
		target.rust(),
	]);

	if host_manifest.is_file() {
		let client = config
			.build
			.client_package
			.as_deref()
			.context("portable native builds require build.client_package")?;
		command.args([
			"--no-default-features",
			"--features",
			&format!("{client}/{}", target.platform()),
		]);
	}

	let output = command.output()?;
	anyhow::ensure!(
		output.status.success(),
		"cargo metadata: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	let metadata: Metadata = serde_json::from_slice(&output.stdout)?;
	let mut host_config = config.clone();

	if host_manifest.is_file() {
		let host = metadata
			.packages
			.iter()
			.find(|package| package.manifest_path == host_manifest)
			.context("platform host Cargo.toml must be a workspace member")?;
		host_config.app.name.clone_from(&host.name);
	}

	resolve_metadata(metadata, &host_config)
}

fn resolve_metadata(
	metadata: Metadata,
	config: &leptoskit_config::LeptoskitConfig,
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
	let mut available = BTreeMap::new();
	let mut bridge = None;
	let mut bridge_alias = None;
	for dependency in &node.deps {
		if !normal_dependency(dependency) {
			continue;
		}
		let package = packages
			.get(&dependency.pkg)
			.context("resolved dependency missing")?;
		if package.name == "leptoskit_bridge_ffi" {
			bridge = package.manifest_path.parent().map(Path::to_path_buf);
			bridge_alias = Some(dependency.name.clone());
		}
		let Some(value) = package.metadata.pointer("/leptoskit/runtime") else {
			continue;
		};
		let mut plugin: Plugin = serde_json::from_value(value.clone())
			.with_context(|| format!("{} runtime metadata", package.name))?;
		anyhow::ensure!(identifier(&dependency.name, "_"), "invalid crate alias");
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
			!plugin.rust_async || (plugin.rust_factory.is_some() && !plugin.rust_worker),
			"rust-async requires rust-factory and cannot be combined with rust-worker"
		);
		if let Some(path) = &plugin.rust_factory {
			anyhow::ensure!(type_path(path, "::"), "invalid Rust plugin factory");
		}
		if let Some(path) = &plugin.rust_type {
			anyhow::ensure!(type_path(path, "::"), "invalid Rust plugin type");
		}
		if let Some(path) = &plugin.apple_type {
			anyhow::ensure!(type_path(path, "."), "invalid Swift plugin type");
		}
		if let Some(path) = &plugin.apple_module {
			anyhow::ensure!(identifier(path, "_"), "invalid Swift module");
		}
		if let Some(path) = &plugin.android_type {
			anyhow::ensure!(type_path(path, "."), "invalid Kotlin plugin type");
		}
		for permission in &plugin.android_permissions {
			anyhow::ensure!(type_path(permission, "."), "invalid Android permission");
		}
		for action in &plugin.android_query_actions {
			anyhow::ensure!(
				action.contains('.') && type_path(action, "."),
				"invalid fully qualified Android query action"
			);
		}
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
		for dependency in &plugin.android_dependencies {
			anyhow::ensure!(
				maven_coordinate(dependency),
				"invalid Android dependency in plugin {}: expected group:artifact:version with an \
				 explicit numeric-leading version",
				plugin.name
			);
		}
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
	let mut plugins = Vec::new();
	for name in &config.native.plugins {
		let mut plugin = available.remove(name).with_context(|| {
			format!(
				"plugin `{name}` must be declared once and be a direct Cargo dependency with \
				 runtime metadata"
			)
		})?;
		if plugin.rust_async {
			let options = config.native.plugin_options.get(name).with_context(|| {
				format!(
					"async plugin `{name}` requires [native.plugin-options.{name}] host \
					 configuration"
				)
			})?;
			let json = serde_json::to_string(options)
				.with_context(|| format!("serializing native.plugin-options.{name}"))?;
			anyhow::ensure!(
				json.len() <= 65_536,
				"native.plugin-options.{name} exceeds 64 KiB"
			);
			plugin.options_json = Some(json);
		} else {
			anyhow::ensure!(
				!config.native.plugin_options.contains_key(name),
				"plugin `{name}` does not accept native.plugin-options"
			);
		}
		plugins.push(plugin);
	}
	let bridge = bridge.context("native apps need a direct leptoskit_bridge_ffi dependency")?;
	let mut host_features: Vec<_> = plugins
		.iter()
		.filter(|plugin| plugin.rust_async)
		.map(|plugin| {
			cargo_dependency_key(app, &plugin.package_name, &plugin.crate_name)
				.map(|key| format!("{key}/async-host"))
		})
		.collect::<Result<_>>()?;
	if !host_features.is_empty() {
		host_features.push(format!(
			"{}/async-host",
			cargo_dependency_key(
				app,
				"leptoskit_bridge_ffi",
				&bridge_alias.context("bridge alias missing")?
			)?
		));
	}
	let support = resolve_support(&metadata, &plugins, &bridge)?;
	Ok(Registry {
		package: app.name.clone(),
		plugins,
		support,
		bridge,
		target: metadata.target_directory,
		host_features,
	})
}

fn cargo_dependency_key<'a>(
	app: &'a Package,
	package_name: &str,
	rust_alias: &str,
) -> Result<&'a str> {
	let dependency = app
		.dependencies
		.iter()
		.find(|dependency| {
			dependency.kind.is_none()
				&& dependency.name == package_name
				&& dependency
					.rename
					.as_ref()
					.is_none_or(|rename| rename.replace('-', "_") == rust_alias)
		})
		.with_context(|| format!("Cargo dependency key missing for `{rust_alias}`"))?;
	Ok(dependency.rename.as_deref().unwrap_or(&dependency.name))
}

fn normal_dependency(dependency: &Dependency) -> bool {
	dependency.dep_kinds.is_empty() || dependency.dep_kinds.iter().any(|kind| kind.kind.is_none())
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
	while let Some(id) = pending.pop() {
		if !seen.insert(id) {
			continue;
		}
		let package = packages
			.get(id)
			.context("native support dependency package missing")?;
		if let Some(value) = package.metadata.pointer("/leptoskit/native-support") {
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
mod tests {
	use leptoskit_config::LeptoskitConfig;
	use serde_json::json;

	use super::*;

	fn metadata(kind: &str) -> Metadata {
		serde_json::from_value(json!({
			"target_directory": "/target",
			"workspace_members": ["app"],
			"packages": [
				{"id":"app", "name":"app", "manifest_path":"/app/Cargo.toml", "metadata":{},
				 "dependencies":[{"name":"leptoskit_bridge_ffi","rename":"ffi","kind":null},{"name":"echo","rename":"renamed_echo","kind":null}]},
				{"id":"ffi", "name":"leptoskit_bridge_ffi", "manifest_path":"/sdk/ffi/Cargo.toml", "metadata":{}},
				{"id":"echo", "name":"echo", "manifest_path":"/sdk/echo/Cargo.toml",
					"metadata":{"leptoskit":{"runtime":{"name":"echo", "rust-type":kind}}}},
				{"id":"transitive", "name":"hidden", "manifest_path":"/sdk/hidden/Cargo.toml",
					"metadata":{"leptoskit":{"runtime":{"name":"hidden", "rust-type":"Plugin"}}}}
			],
			"resolve":{"nodes":[{"id":"app","deps":[{"name":"ffi","pkg":"ffi"},{"name":"renamed_echo","pkg":"echo"}]}]}
		})).unwrap()
	}

	#[test]
	fn registers_selected_direct_dependencies_using_their_cargo_alias() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		let registry = resolve_metadata(metadata("Plugin"), &config).unwrap();
		assert!(registry.host_features.is_empty());
		assert_eq!(registry.plugins[0].crate_name, "renamed_echo");
		assert_eq!(registry.bridge, Path::new("/sdk/ffi"));
		config.native.plugins.clear();
		assert!(
			resolve_metadata(metadata("Plugin"), &config)
				.unwrap()
				.plugins
				.is_empty()
		);
		config.native.plugins = vec!["hidden".into()];
		assert!(resolve_metadata(metadata("Plugin"), &config).is_err());
		config.native.plugins = vec!["echo".into(), "echo".into()];
		assert!(resolve_metadata(metadata("Plugin"), &config).is_err());
	}

	fn async_metadata() -> Metadata {
		let mut value = metadata("Plugin");
		value.packages[2].metadata = json!({"leptoskit":{"runtime":{
			"name":"echo", "rust-factory":"from_config", "rust-async":true
		}}});
		value
	}

	#[test]
	fn async_factories_require_host_options_and_select_host_features() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		let error = resolve_metadata(async_metadata(), &config)
			.err()
			.unwrap()
			.to_string();
		assert!(error.contains("[native.plugin-options.echo]"));
		config.native.plugin_options.insert(
			"echo".into(),
			toml::from_str("origins = [\"https://example.com\"]\nlabel = 'quote\" and newline\\n'")
				.unwrap(),
		);
		let registry = resolve_metadata(async_metadata(), &config).unwrap();
		assert_eq!(
			registry.host_features,
			["renamed_echo/async-host", "ffi/async-host"]
		);
		let options: serde_json::Value =
			serde_json::from_str(registry.plugins[0].options_json.as_ref().unwrap()).unwrap();
		assert_eq!(options["origins"][0], "https://example.com");
		let source = super::super::project::rust_registrant(&registry, Target::Android).unwrap();
		assert!(
			source.contains("app.install_async(std::sync::Arc::new(renamed_echo::from_config(")
		);
		assert!(!source.contains("&_context.data_dir"));
		assert!(source.contains("\\\"origins\\\""));
		assert!(resolve_metadata(metadata("Plugin"), &config).is_err());
		config
			.native
			.plugin_options
			.get_mut("echo")
			.unwrap()
			.insert("oversized".into(), toml::Value::String("x".repeat(65_536)));
		assert!(resolve_metadata(async_metadata(), &config).is_err());
	}

	#[test]
	fn async_metadata_rejects_worker_mode_and_missing_factory() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		config
			.native
			.plugin_options
			.insert("echo".into(), toml::Table::new());
		for runtime in [
			json!({"name":"echo", "rust-async":true}),
			json!({"name":"echo", "rust-async":true, "rust-type":"Plugin"}),
			json!({"name":"echo", "rust-async":true, "rust-factory":"from_config", "rust-worker":true}),
			json!({"name":"echo", "rust-async":true, "rust-factory":"from_config();panic!()"}),
		] {
			let mut value = metadata("Plugin");
			value.packages[2].metadata = json!({"leptoskit":{"runtime":runtime}});
			assert!(resolve_metadata(value, &config).is_err());
		}
	}

	#[test]
	fn host_features_use_manifest_keys_while_rust_uses_normalized_aliases() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		config
			.native
			.plugin_options
			.insert("echo".into(), toml::Table::new());
		let mut metadata = async_metadata();
		metadata.packages[0].dependencies[1].rename = Some("renamed-echo".into());
		let registry = resolve_metadata(metadata, &config).unwrap();
		assert_eq!(registry.plugins[0].crate_name, "renamed_echo");
		assert_eq!(
			registry.host_features,
			["renamed-echo/async-host", "ffi/async-host"]
		);
	}

	#[test]
	fn rejects_expressions_in_generated_type_paths() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		assert!(resolve_metadata(metadata("Plugin::new(); panic!()"), &config).is_err());
		assert!(resolve_metadata(metadata("module::Plugin"), &config).is_ok());
	}

	#[test]
	fn validates_factory_paths_and_rejects_ambiguous_constructors() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		let mut factory = metadata("Plugin");
		let runtime = factory.packages[2]
			.metadata
			.pointer_mut("/leptoskit/runtime")
			.unwrap()
			.as_object_mut()
			.unwrap();
		runtime.remove("rust-type");
		runtime.insert("rust-factory".into(), json!("storage::from_app_dirs"));
		assert!(resolve_metadata(factory, &config).is_ok());
		for path in ["factory(); panic!()", "factory()", "::factory"] {
			let mut factory = metadata("Plugin");
			let runtime = factory.packages[2]
				.metadata
				.pointer_mut("/leptoskit/runtime")
				.unwrap()
				.as_object_mut()
				.unwrap();
			runtime.remove("rust-type");
			runtime.insert("rust-factory".into(), json!(path));
			assert!(resolve_metadata(factory, &config).is_err());
		}
		let mut ambiguous = metadata("Plugin");
		ambiguous.packages[2]
			.metadata
			.pointer_mut("/leptoskit/runtime")
			.unwrap()
			.as_object_mut()
			.unwrap()
			.insert("rust-factory".into(), json!("from_app_dirs"));
		assert!(resolve_metadata(ambiguous, &config).is_err());
	}

	fn support_metadata() -> Metadata {
		let mut metadata = metadata("Plugin");
		metadata.resolve.nodes[0]
			.deps
			.push(serde_json::from_value(json!({"name":"hidden","pkg":"transitive"})).unwrap());
		for (id, module) in [
			("shared", "SharedNative"),
			("dev", "DevNative"),
			("unselected", "OtherNative"),
			("wrapper", "WrapperNative"),
		] {
			metadata.packages.push(
				serde_json::from_value(json!({
					"id": id, "name": id, "manifest_path": std::env::temp_dir().join("leptoskit-native-support").join(id).join("Cargo.toml"),
					"metadata": {"leptoskit": {"native-support": {"apple-module": module, "android": true}}}
				}))
				.unwrap(),
			);
		}
		for value in [
			json!({"id":"echo","deps":[
				{"name":"shared","pkg":"shared","dep_kinds":[{"kind":null}]},
				{"name":"wrapper","pkg":"wrapper","dep_kinds":[{"kind":null}]},
				{"name":"dev","pkg":"dev","dep_kinds":[{"kind":"dev"},{"kind":"build"}]}
			]}),
			json!({"id":"wrapper","deps":[{"name":"shared","pkg":"shared"}]}),
			json!({"id":"transitive","deps":[{"name":"unselected","pkg":"unselected"}]}),
		] {
			metadata
				.resolve
				.nodes
				.push(serde_json::from_value(value).unwrap());
		}
		metadata
	}

	#[test]
	fn support_packages_follow_only_selected_normal_dependency_closures() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		let registry = resolve_metadata(support_metadata(), &config).unwrap();
		assert_eq!(
			registry
				.support
				.iter()
				.map(|helper| helper.package_name.as_str())
				.collect::<Vec<_>>(),
			["shared", "wrapper"]
		);
		assert_eq!(registry.plugins.len(), 1);
		config.native.plugins.clear();
		assert!(
			resolve_metadata(support_metadata(), &config)
				.unwrap()
				.support
				.is_empty()
		);
	}

	#[test]
	fn support_rejects_duplicate_paths_modules_and_invalid_metadata() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		let mut duplicate_path = support_metadata();
		duplicate_path
			.packages
			.iter_mut()
			.find(|package| package.id == "wrapper")
			.unwrap()
			.manifest_path =
			std::env::temp_dir().join("leptoskit-native-support/shared/Cargo.toml");
		assert!(resolve_metadata(duplicate_path, &config).is_err());
		for module in ["SharedNative", "../Outside", "LeptosKitBridge"] {
			let mut metadata = support_metadata();
			metadata
				.packages
				.iter_mut()
				.find(|package| package.id == "wrapper")
				.unwrap()
				.metadata = json!({"leptoskit": {"native-support": {"apple-module": module}}});
			assert!(resolve_metadata(metadata, &config).is_err());
		}
		let mut invalid_path = support_metadata();
		invalid_path
			.packages
			.iter_mut()
			.find(|package| package.id == "wrapper")
			.unwrap()
			.manifest_path = "/sdk/../other/Cargo.toml".into();
		assert!(resolve_metadata(invalid_path, &config).is_err());
	}

	#[test]
	fn apple_context_requires_a_complete_native_implementation() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		for (module, kind, valid) in [
			(None, None, false),
			(Some("Echo"), None, false),
			(None, Some("EchoPlugin"), false),
			(Some("Echo"), Some("EchoPlugin"), true),
		] {
			let mut metadata = metadata("Plugin");
			metadata.packages[2].metadata = json!({
				"leptoskit": {"runtime": {
					"name": "echo", "apple-context": true,
					"apple-module": module, "apple-type": kind
				}}
			});
			assert_eq!(resolve_metadata(metadata, &config).is_ok(), valid);
		}
	}

	#[test]
	fn worker_metadata_requires_a_rust_implementation() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		let mut metadata = metadata("Plugin");
		metadata.packages[2].metadata = json!({"leptoskit":{"runtime":{"name":"echo","rust-worker":true,"apple-module":"Echo","apple-type":"Echo"}}});
		assert!(resolve_metadata(metadata, &config).is_err());
	}

	#[test]
	fn validates_android_query_actions_before_generating_xml() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		for (action, valid) in [
			("android.intent.action.TTS_SERVICE", true),
			("single", false),
			("android.action\"><invalid", false),
		] {
			let mut metadata = metadata("Plugin");
			metadata.packages[2]
				.metadata
				.pointer_mut("/leptoskit/runtime")
				.unwrap()
				.as_object_mut()
				.unwrap()
				.insert("android-query-actions".into(), json!([action]));
			assert_eq!(resolve_metadata(metadata, &config).is_ok(), valid);
		}
	}

	#[test]
	fn validates_required_apple_keys_and_maven_coordinates_before_generation() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		for (field, value, valid) in [
			(
				"apple-usage-descriptions",
				"NSUserTrackingUsageDescription",
				true,
			),
			(
				"apple-usage-descriptions",
				"NSMicrophoneUsageDescription",
				true,
			),
			("apple-usage-descriptions", "NSUsageDescription", false),
			("apple-usage-descriptions", "CFBundleIdentifier", false),
			(
				"apple-usage-descriptions",
				"NSUserTrackingUsageDescription\n",
				false,
			),
			("apple-usage-descriptions", "NS<key>UsageDescription", false),
			(
				"android-dependencies",
				"com.google.android.play:review:2.0.2",
				true,
			),
			(
				"android-dependencies",
				"org.example:library-ktx:1.2.0-RC1",
				true,
			),
			(
				"android-dependencies",
				"org.example:library:1.0-SNAPSHOT",
				true,
			),
			("android-dependencies", "org..example:library:1.0", false),
			("android-dependencies", ":library:1.0", false),
			("android-dependencies", "org.example::1.0", false),
			("android-dependencies", "org.example:library:", false),
			(
				"android-dependencies",
				"org.example:library:1.0:classifier",
				false,
			),
			("android-dependencies", "org.example:library:1.0@aar", false),
			("android-dependencies", "org.example:library:+", false),
			("android-dependencies", "org.example:library:1.+", false),
			(
				"android-dependencies",
				"org.example:library:latest.release",
				false,
			),
			("android-dependencies", "org.example:library:[1,2)", false),
			(
				"android-dependencies",
				"org.example:library:1.0\");exec(\"bad",
				false,
			),
			(
				"android-dependencies",
				"org.example:library:${version}",
				false,
			),
			(
				"android-dependencies",
				"org.example:library:1.0\nbad",
				false,
			),
			(
				"android-dependencies",
				"org.example:library:1.0\\bad",
				false,
			),
		] {
			let mut metadata = metadata("Plugin");
			metadata.packages[2]
				.metadata
				.pointer_mut("/leptoskit/runtime")
				.unwrap()
				.as_object_mut()
				.unwrap()
				.insert(field.into(), json!([value]));
			assert_eq!(
				resolve_metadata(metadata, &config).is_ok(),
				valid,
				"{field}: {value}"
			);
		}
	}

	#[test]
	fn selects_the_workspace_application_instead_of_a_same_named_dependency() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		let mut metadata = metadata("Plugin");
		metadata.workspace_members = vec!["app".into()];
		metadata.packages.insert(0, serde_json::from_value(json!({
			"id":"another-app-version", "name":"app", "manifest_path":"/registry/app/Cargo.toml", "metadata":{}
		})).unwrap());
		metadata.resolve.nodes.insert(
			0,
			serde_json::from_value(json!({
				"id":"another-app-version", "deps":[]
			}))
			.unwrap(),
		);
		assert_eq!(
			resolve_metadata(metadata, &config).unwrap().plugins[0].name,
			"echo"
		);
	}
}
