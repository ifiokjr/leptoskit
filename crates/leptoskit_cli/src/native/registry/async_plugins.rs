//! Host configuration and Cargo feature ownership for asynchronous plugins.

use super::*;
const MAX_OPTIONS_BYTES: usize = 65_536;

pub(super) fn validate_async(plugin: &Plugin) -> Result<()> {
	anyhow::ensure!(
		!plugin.rust_async || (plugin.rust_factory.is_some() && !plugin.rust_worker),
		"rust-async requires rust-factory and cannot be combined with rust-worker"
	);
	Ok(())
}

pub(super) fn configure_plugin(
	plugin: &mut Plugin,
	config: &leptoskit_config::LeptoskitConfig,
	name: &str,
) -> Result<()> {
	if plugin.declares_async() {
		let options = config.native.plugin_options.get(name).with_context(|| {
			format!(
				"async plugin `{name}` requires [native.plugin-options.{name}] host configuration"
			)
		})?;
		let json = serde_json::to_string(options)
			.with_context(|| format!("serializing native.plugin-options.{name}"))?;
		anyhow::ensure!(
			json.len() <= MAX_OPTIONS_BYTES,
			"native.plugin-options.{name} exceeds 64 KiB"
		);
		plugin.options_json = Some(json);
	} else {
		anyhow::ensure!(
			!config.native.plugin_options.contains_key(name),
			"plugin `{name}` does not accept native.plugin-options"
		);
	}

	Ok(())
}

pub(super) fn host_features(
	app: &Package,
	node: &Node,
	packages: &BTreeMap<&String, &Package>,
	plugins: &[Plugin],
	target: Option<Target>,
) -> Result<Vec<String>> {
	// Without a target, every async declaration counts.
	let runs_async = |plugin: &Plugin| {
		match target {
			Some(target) => {
				plugin
					.implementation(target, HostCapabilities::of(target))
					.is_ok_and(|implementation| {
						implementation
							.rust()
							.is_some_and(|rust| rust.flavour == Flavour::Async)
					})
			}
			None => plugin.declares_async(),
		}
	};
	let mut features: Vec<_> = plugins
		.iter()
		.filter(|plugin| runs_async(plugin))
		.map(|plugin| {
			cargo_dependency_key(app, &plugin.package_name, &plugin.crate_name)
				.map(|key| format!("{key}/async-host"))
		})
		.collect::<Result<_>>()?;

	if !features.is_empty() {
		let bridge_alias = node
			.deps
			.iter()
			.filter(|dependency| normal_dependency(dependency))
			.find(|dependency| {
				packages
					.get(&dependency.pkg)
					.is_some_and(|package| package.name == "leptoskit_bridge_ffi")
			})
			.context("bridge alias missing")?;
		features.push(format!(
			"{}/async-host",
			cargo_dependency_key(app, "leptoskit_bridge_ffi", &bridge_alias.name)?
		));
	}

	Ok(features)
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
