// Metadata rejection controls for asynchronous application services.

use super::*;

mod hybrid_tests {
	include!("hybrid_tests.rs");
}

fn service_metadata(fields: &serde_json::Value) -> Metadata {
	let mut value = metadata("Plugin");
	value.packages[2].metadata = json!({"leptoskit":{"runtime": fields}});
	value
}

fn service_config() -> LeptoskitConfig {
	let mut config = LeptoskitConfig::default();
	config.app.name = "app".into();
	config.native.plugins = vec!["echo".into()];
	config
}

#[test]
fn selects_a_service_factory_using_the_direct_dependency_alias() {
	let registry = resolve_metadata(
		service_metadata(&json!({
			"name": "echo", "rust-service-factory": "native::from_app_dirs"
		})),
		&service_config(),
	)
	.unwrap();
	assert!(registry.plugins[0].has_rust_registration());
	assert_eq!(registry.plugins[0].crate_name, "renamed_echo");
	assert_eq!(
		registry.plugins[0].rust_service_factory.as_deref(),
		Some("native::from_app_dirs")
	);
}

#[test]
fn rejects_executable_or_malformed_service_factory_metadata() {
	for factory in [
		"",
		"::factory",
		"factory()",
		"a::",
		"a::b();panic!()",
		"with space",
		"match",
		"native::type",
		"Self::new",
	] {
		let error = resolve_metadata(
			service_metadata(&json!({
				"name": "echo", "rust-service-factory": factory
			})),
			&service_config(),
		)
		.err()
		.unwrap();
		assert!(format!("{error:#}").contains("invalid Rust service factory"));
	}
}

#[test]
fn rejects_ambiguous_service_and_plugin_registration() {
	for (key, value) in [
		("rust-type", json!("Plugin")),
		("rust-factory", json!("new")),
		("rust-worker", json!(true)),
		("rust-async", json!(true)),
	] {
		let mut fields = json!({"name": "echo", "rust-service-factory": "new"});
		fields.as_object_mut().unwrap().insert(key.into(), value);
		let error = resolve_metadata(service_metadata(&fields), &service_config())
			.err()
			.unwrap();
		assert!(
			format!("{error:#}").contains("rust-service-factory"),
			"{key}"
		);
	}
}

#[test]
fn rejects_multiple_selected_service_owners() {
	let mut metadata = service_metadata(&json!({"name": "echo", "rust-service-factory": "new"}));
	metadata.packages[3].metadata = json!({"leptoskit":{"runtime":{
		"name":"second", "rust-service-factory":"new"
	}}});
	metadata.resolve.nodes[0].deps.push(
		serde_json::from_value(json!({
			"name":"second", "pkg":"transitive"
		}))
		.unwrap(),
	);
	let mut config = service_config();
	config.native.plugins.push("second".into());
	let error = resolve_metadata(metadata, &config).err().unwrap();
	assert!(error.to_string().contains("multiple rust-service-factory"));
}
