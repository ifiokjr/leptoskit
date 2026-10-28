// Hybrid metadata keeps package selection separate from capability identities.

use super::*;

#[test]
fn selects_one_service_with_both_native_implementations() {
	let registry = resolve_metadata(
		service_metadata(&json!({
			"name": "echo", "rust-service-factory": "native::from_app_dirs",
			"apple-module": "Echo", "apple-type": "EchoPlatform", "apple-context": true,
			"android-type": "dev.example.EchoPlatform", "android-context": true
		})),
		&service_config(),
	)
	.unwrap();
	let plugin = &registry.plugins[0];
	assert!(plugin.has_rust_registration());
	assert!(plugin.uses_platform_dispatcher(Target::Macos));
	assert!(plugin.uses_platform_dispatcher(Target::Android));
	assert_eq!(plugin.crate_name, "renamed_echo");
	assert_eq!(plugin.apple_type.as_deref(), Some("EchoPlatform"));
	assert_eq!(
		plugin.android_type.as_deref(),
		Some("dev.example.EchoPlatform")
	);
}

#[test]
fn rejects_invalid_native_paths_even_alongside_a_service() {
	for (key, value) in [
		("apple-type", "Platform(); fatalError()"),
		("apple-module", "../Outside"),
		("android-type", "dev.Platform(); exit()"),
	] {
		let mut fields = json!({"name": "echo", "rust-service-factory": "new"});
		fields
			.as_object_mut()
			.unwrap()
			.insert(key.into(), json!(value));
		assert!(resolve_metadata(service_metadata(&fields), &service_config()).is_err());
	}
}

#[test]
fn hybrid_selection_still_rejects_duplicate_runtime_packages() {
	let mut config = service_config();
	config.native.plugins.push("echo".into());
	let metadata = service_metadata(&json!({
		"name": "echo", "rust-service-factory": "new",
		"apple-module": "Echo", "apple-type": "EchoPlatform"
	}));
	assert!(resolve_metadata(metadata, &config).is_err());
}

#[test]
fn hybrid_package_does_not_bypass_the_one_service_limit() {
	let mut metadata = service_metadata(&json!({
		"name": "echo", "rust-service-factory": "new", "android-type": "dev.Platform"
	}));
	metadata.packages[3].metadata = json!({"leptoskit":{"runtime":{
		"name":"second", "rust-service-factory":"new"
	}}});
	metadata.resolve.nodes[0]
		.deps
		.push(serde_json::from_value(json!({"name":"second", "pkg":"transitive"})).unwrap());
	let mut config = service_config();
	config.native.plugins.push("second".into());
	let error = resolve_metadata(metadata, &config).err().unwrap();
	assert!(error.to_string().contains("multiple rust-service-factory"));
}
