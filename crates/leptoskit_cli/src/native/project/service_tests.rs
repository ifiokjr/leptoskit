// Retains the actual generated constructor for the standalone application
// fixture.

use super::*;

#[test]
fn generated_service_registration_is_inert_and_skips_platform_dispatchers() {
	let directory = tempfile::tempdir().unwrap();
	let root = directory.path().canonicalize().unwrap();
	let registry = service_registry(&root);
	generate(
		&root,
		&leptoskit_config::LeptoskitConfig::default(),
		&registry,
		Target::Android,
		false,
	)
	.unwrap();
	let registrant = std::fs::read_to_string(root.join(".leptoskit/registrant.rs")).unwrap();
	assert!(registrant.contains(
		"app.defer_service(service_crate::from_app_dirs(\n                    \
		 &_context.data_dir,\n                    &_context.cache_dir,\n                )?)?;"
	));
	assert!(!registrant.contains("app.install"));
	let kotlin =
		std::fs::read_to_string(root.join(".leptoskit/android/sources/GeneratedPlugins.kt"))
			.unwrap();
	assert!(!kotlin.contains("dispatcher.register("));

	if let Some(destination) = std::env::var_os("LEPTOSKIT_SERVICE_FIXTURE_OUTPUT") {
		let destination = std::path::PathBuf::from(destination);
		std::fs::create_dir_all(&destination).unwrap();
		std::fs::write(destination.join("registrant.rs"), registrant).unwrap();
	}
}

pub(super) fn service_registry(root: &Path) -> Registry {
	let bridge = root.join("bridge");

	for source in platform_sources::ANDROID {
		fixture(&bridge.join(source), "// bridge source");
	}

	fixture(&root.join("hosts/android/app/src/main/AndroidManifest.xml"),
		"<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\"><application/></manifest>");

	for source in [
		"android/NativePlugin.kt",
		"android/proguard-rules.pro",
		"android/NativePermissionHost.kt",
		"android/NativeActivityResultHost.kt",
		"mobile/LeptosKitView.kt",
		"mobile/NativeRustWorker.kt",
		"mobile/NativeReleases.kt",
		"mobile/LeptosKitApplicationView.kt",
		"Package.swift",
	] {
		fixture(&bridge.join(source), "// fixture");
	}

	Registry {
		package: "demo".into(),
		plugins: vec![plugin(
			serde_json::json!({
				"name": "service", "rust-service-factory": "from_app_dirs"
			}),
			root,
			"service_crate",
		)],
		support: vec![],
		bridge,
		target: root.join("target"),
		host_features: vec![],
	}
}
