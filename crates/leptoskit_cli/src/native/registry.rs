use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Metadata {
	packages: Vec<Package>,
	resolve: Resolve,
	target_directory: PathBuf,
}
#[derive(Debug, Deserialize)]
struct Package {
	id: String,
	name: String,
	manifest_path: PathBuf,
	metadata: serde_json::Value,
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
pub struct Plugin {
	pub name: String,
	pub rust_type: Option<String>,
	pub apple_module: Option<String>,
	pub apple_type: Option<String>,
	pub android_type: Option<String>,
	#[serde(default)]
	pub android_context: bool,
	#[serde(default)]
	pub android_permissions: Vec<String>,
	#[serde(skip)]
	pub directory: PathBuf,
	#[serde(skip)]
	pub crate_name: String,
	#[serde(skip)]
	pub package_name: String,
}

pub struct Registry {
	pub plugins: Vec<Plugin>,
	pub bridge: PathBuf,
	pub target: PathBuf,
}

pub fn resolve(
	root: &Path,
	config: &leptoskit_config::LeptoskitConfig,
	target: &str,
) -> Result<Registry> {
	let output = Command::new("cargo")
		.current_dir(root)
		.args([
			"metadata",
			"--format-version",
			"1",
			"--filter-platform",
			target,
		])
		.output()?;
	anyhow::ensure!(
		output.status.success(),
		"cargo metadata: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	let metadata: Metadata = serde_json::from_slice(&output.stdout)?;
	resolve_metadata(metadata, config)
}

fn resolve_metadata(
	metadata: Metadata,
	config: &leptoskit_config::LeptoskitConfig,
) -> Result<Registry> {
	let app = metadata
		.packages
		.iter()
		.find(|package| package.name == config.app.name)
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
	for dependency in &node.deps {
		if !dependency.dep_kinds.is_empty()
			&& !dependency.dep_kinds.iter().any(|kind| kind.kind.is_none())
		{
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
		anyhow::ensure!(identifier(&dependency.name, "_"), "invalid crate alias");
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
		package
			.manifest_path
			.parent()
			.context("plugin directory missing")?
			.clone_into(&mut plugin.directory);
		plugin.crate_name.clone_from(&dependency.name);
		plugin.package_name.clone_from(&package.name);
		let name = plugin.name.clone();
		anyhow::ensure!(
			available.insert(name.clone(), plugin).is_none(),
			"duplicate runtime plugin {name}"
		);
	}
	let mut plugins = Vec::new();
	for name in &config.native.plugins {
		plugins.push(available.remove(name).with_context(|| {
			format!(
				"plugin `{name}` must be declared once and be a direct Cargo dependency with \
				 runtime metadata"
			)
		})?);
	}
	Ok(Registry {
		plugins,
		bridge: bridge.context("native apps need a direct leptoskit_bridge_ffi dependency")?,
		target: metadata.target_directory,
	})
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

#[cfg(test)]
mod tests {
	use leptoskit_config::LeptoskitConfig;
	use serde_json::json;

	use super::*;

	fn metadata(kind: &str) -> Metadata {
		serde_json::from_value(json!({
			"target_directory": "/target",
			"packages": [
				{"id":"app", "name":"app", "manifest_path":"/app/Cargo.toml", "metadata":{}},
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

	#[test]
	fn rejects_expressions_in_generated_type_paths() {
		let mut config = LeptoskitConfig::default();
		config.app.name = "app".into();
		config.native.plugins = vec!["echo".into()];
		assert!(resolve_metadata(metadata("Plugin::new(); panic!()"), &config).is_err());
		assert!(resolve_metadata(metadata("module::Plugin"), &config).is_ok());
	}
}
