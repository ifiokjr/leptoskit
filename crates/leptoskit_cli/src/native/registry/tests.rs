mod service_tests {
	include!("service_tests.rs");
}
mod async_tests {
	include!("async_tests.rs");
}
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
		.manifest_path = std::env::temp_dir().join("leptoskit-native-support/shared/Cargo.toml");
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
fn validates_android_intent_schemes_and_complete_metadata_before_generating_xml() {
	let mut config = LeptoskitConfig::default();
	config.app.name = "app".into();
	config.native.plugins = vec!["echo".into()];

	for (query, valid) in [
		(
			json!({"action":"android.intent.action.VIEW", "scheme":"solana-wallet"}),
			true,
		),
		(
			json!({"action":"android.intent.action.VIEW", "scheme":"wallet+v1.test"}),
			true,
		),
		(json!({"action":"single", "scheme":"solana-wallet"}), false),
		(
			json!({"action":"android.intent.action.VIEW", "scheme":""}),
			false,
		),
		(
			json!({"action":"android.intent.action.VIEW", "scheme":"1wallet"}),
			false,
		),
		(
			json!({"action":"android.intent.action.VIEW", "scheme":"wallet:/"}),
			false,
		),
		(
			json!({"action":"android.intent.action.VIEW", "scheme":"wallet\"><invalid"}),
			false,
		),
		(json!({"action":"android.intent.action.VIEW"}), false),
		(
			json!({"action":"android.intent.action.VIEW", "scheme":"wallet", "host":"example"}),
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
			.insert("android-query-intents".into(), json!([query]));
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
