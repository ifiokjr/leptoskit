use super::*;

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
	let source = crate::native::project::rust_registrant(&registry, Target::Android).unwrap();
	assert!(source.contains("app.install_async(std::sync::Arc::new(renamed_echo::from_config("));
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
