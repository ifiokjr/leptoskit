//! Plugin resolution and hook tests.

use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;
use leptoskit_config::PluginSpec;
use leptoskit_plugin::Builtins;
use leptoskit_plugin::PluginBuildContext;
use leptoskit_plugin::PluginRegistry;

fn spec(name: &str, leptos: bool) -> PluginSpec {
	PluginSpec {
		name: name.to_owned(),
		leptos,
		..PluginSpec::default()
	}
}

fn config_with(plugins: Vec<PluginSpec>) -> LeptoskitConfig {
	LeptoskitConfig {
		plugins,
		..LeptoskitConfig::default()
	}
}

#[test]
fn empty_config_resolves_to_an_empty_registry() {
	let (registry, unresolved) =
		PluginRegistry::resolve(&LeptoskitConfig::default()).expect("resolve");
	assert!(registry.is_empty());
	assert!(unresolved.is_empty());
}

#[test]
fn builtins_resolve_in_declaration_order() {
	let config = config_with(vec![
		spec("size_report", false),
		spec("env_manifest", false),
	]);
	let (registry, unresolved) = PluginRegistry::resolve(&config).expect("resolve");
	assert_eq!(registry.names(), vec!["size_report", "env_manifest"]);
	assert_eq!(registry.len(), 2);
	assert!(unresolved.is_empty());
}

#[test]
fn unknown_names_are_reported_unresolved() {
	let config = config_with(vec![spec("not_a_builtin", false)]);
	let (registry, unresolved) = PluginRegistry::resolve(&config).expect("resolve");
	assert!(registry.is_empty());
	assert_eq!(unresolved.len(), 1);
	assert_eq!(unresolved[0].spec.name, "not_a_builtin");
}

#[test]
fn leptos_plugins_are_skipped() {
	// Even a name that matches a built-in is not loaded when it is declared as
	// a leptos plugin, and an unknown leptos plugin is not reported.
	let config = config_with(vec![spec("size_report", true), spec("leptos_meta", true)]);
	let (registry, unresolved) = PluginRegistry::resolve(&config).expect("resolve");
	assert!(registry.is_empty());
	assert!(unresolved.is_empty());
}

#[test]
fn every_builtin_name_resolves() {
	for name in Builtins::NAMES {
		let plugin = Builtins::by_name(name).expect("builtin");
		assert_eq!(plugin.name(), *name);
	}
	assert!(Builtins::by_name("nope").is_none());
}

#[test]
fn env_manifest_sets_build_variables() {
	let config = config_with(vec![spec("env_manifest", false)]);
	let (registry, _) = PluginRegistry::resolve(&config).expect("resolve");
	let mut context = PluginBuildContext::new(config, Platform::Native, true);
	registry.before_build(&mut context).expect("before_build");
	assert_eq!(
		context
			.env
			.get("LEPTOSKIT_PLUGIN_PLATFORM")
			.map(String::as_str),
		Some("native")
	);
	assert_eq!(
		context
			.env
			.get("LEPTOSKIT_PLUGIN_RELEASE")
			.map(String::as_str),
		Some("1")
	);
}

#[test]
fn size_report_reads_the_bundle_after_a_build() {
	let dir = std::env::temp_dir().join(format!("leptoskit_size_report_{}", std::process::id()));
	std::fs::create_dir_all(&dir).expect("dir");
	let bundle = dir.join("app_bg.wasm");
	std::fs::write(&bundle, vec![0_u8; 2048]).expect("bundle");
	let registry = PluginRegistry::resolve(&config_with(vec![spec("size_report", false)]))
		.expect("resolve")
		.0;
	for wasm_bundle in [Some(bundle), Some(dir.join("missing.wasm")), None] {
		let mut context = leptoskit_plugin::PluginPostBuildContext {
			config: LeptoskitConfig::default(),
			platform: Platform::Native,
			release: true,
			output_dir: dir.clone(),
			wasm_bundle,
		};
		registry
			.after_build(&mut context)
			.expect("a missing or absent bundle is reported, not an error");
	}
	let _ = std::fs::remove_dir_all(&dir);
}
