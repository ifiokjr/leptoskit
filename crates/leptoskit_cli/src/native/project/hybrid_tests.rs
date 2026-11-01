// The actual generated hosts must retain both halves of a hybrid package.

use super::service_tests::service_registry;
use super::*;

#[test]
fn generated_hybrid_hosts_copy_and_register_native_code_beside_the_service() {
	let directory = tempfile::tempdir().unwrap();
	let root = directory.path().canonicalize().unwrap();
	let registry = hybrid_registry(&root);

	for target in [Target::Macos, Target::IosSimulator, Target::AndroidX86_64] {
		generate(
			&root,
			&leptoskit_config::LeptoskitConfig::default(),
			&registry,
			target,
			false,
		)
		.unwrap();
		let rust = std::fs::read_to_string(root.join(".leptoskit/registrant.rs")).unwrap();
		assert_eq!(rust.matches("app.defer_service(").count(), 1);
		assert!(rust.contains("service_crate::from_app_dirs("));
	}

	assert_apple_output(&root, "macos/Sources/GeneratedPlugins.swift");
	assert_apple_output(&root, "ios/GeneratedPlugins.swift");
	let kotlin = read_generated(&root, "android/sources/GeneratedPlugins.kt");
	assert_eq!(
		kotlin,
		include_str!("../../../tests/fixtures/app-service/generated/GeneratedPlugins.kt")
	);
	assert_eq!(kotlin.matches("dispatcher.register(").count(), 1);
	assert!(kotlin.contains("dev.example.PlatformPlugin(context)"));
	assert_eq!(
		read_generated(&root, "android/sources/plugin_0_PlatformPlugin.kt"),
		KOTLIN_PLUGIN
	);

	for platform in ["macos", "ios"] {
		assert!(
			root.join(format!(
				".leptoskit/{platform}/packages/service_crate/Package.swift"
			))
			.is_file()
		);
	}

	export_registrants(&root);
}

#[test]
fn native_only_package_is_skipped_on_an_unsupported_destination() {
	let directory = tempfile::tempdir().unwrap();
	let root = directory.path().canonicalize().unwrap();
	let mut registry = hybrid_registry(&root);
	registry.plugins[0].rust_service_factory = None;
	registry.plugins[0].android_type = None;
	let config = leptoskit_config::LeptoskitConfig::default();

	// Unresolved plugins never reach the generators.
	let error = generate(&root, &config, &registry, Target::Android, false).unwrap_err();
	assert!(error.to_string().contains("has no Android implementation"));
	assert!(!root.join(".leptoskit/registrant.rs").exists());

	registry
		.select(Target::Android, MissingPlugins::Warn)
		.unwrap();
	assert!(registry.plugins.is_empty());
	assert_eq!(registry.warnings().count(), 1);
	generate(&root, &config, &registry, Target::Android, false).unwrap();
	assert!(
		!read_generated(&root, "android/sources/GeneratedPlugins.kt")
			.contains("dispatcher.register(")
	);
}

#[test]
fn android_hybrid_retains_its_rust_service_on_apple_destinations() {
	let directory = tempfile::tempdir().unwrap();
	let root = directory.path().canonicalize().unwrap();
	let mut registry = hybrid_registry(&root);
	registry.plugins[0].apple_type = None;
	registry.plugins[0].apple_module = None;
	registry.plugins[0].apple_context = false;

	for target in [Target::Macos, Target::IosSimulator] {
		generate(
			&root,
			&leptoskit_config::LeptoskitConfig::default(),
			&registry,
			target,
			false,
		)
		.unwrap();
		assert!(read_generated(&root, "registrant.rs").contains("app.defer_service("));
	}

	assert!(!read_generated(&root, "ios/GeneratedPlugins.swift").contains("PlatformPlugin"));
	assert!(
		!read_generated(&root, "macos/Sources/GeneratedPlugins.swift").contains("PlatformPlugin")
	);
}

fn hybrid_registry(root: &Path) -> Registry {
	let mut registry = service_registry(root);
	let package = root.join("platform");
	fixture(&package.join("Package.swift"), "// platform package");
	fixture(&package.join("android/PlatformPlugin.kt"), KOTLIN_PLUGIN);
	fixture(
		&package.join("Sources/ServicePlatform/PlatformPlugin.swift"),
		SWIFT_PLUGIN,
	);
	fixture(&root.join("hosts/macos/App.swift"), "// app");
	let plugin = &mut registry.plugins[0];

	plugin.directory = package;
	plugin.apple_module = Some("ServicePlatform".into());
	plugin.apple_type = Some("PlatformPlugin".into());
	plugin.apple_context = true;

	plugin.android_type = Some("dev.example.PlatformPlugin".into());
	plugin.android_context = true;

	for source in platform_sources::MACOS.iter().chain(platform_sources::IOS) {
		fixture(&registry.bridge.join(source), "// bridge source");
	}

	registry
}

fn export_registrants(root: &Path) {
	let Some(destination) = std::env::var_os("LEPTOSKIT_HYBRID_FIXTURE_OUTPUT") else {
		return;
	};
	let destination = std::path::PathBuf::from(destination);

	for (source, output) in [
		("registrant.rs", "registrant.rs"),
		("android/sources/GeneratedPlugins.kt", "GeneratedPlugins.kt"),
		("ios/GeneratedPlugins.swift", "IosGeneratedPlugins.swift"),
		(
			"macos/Sources/GeneratedPlugins.swift",
			"MacGeneratedPlugins.swift",
		),
	] {
		fixture(&destination.join(output), &read_generated(root, source));
	}
}

const KOTLIN_PLUGIN: &str =
	include_str!("../../../tests/fixtures/app-service/android/PlatformPlugin.kt");
const SWIFT_PLUGIN: &str =
	include_str!("../../../tests/fixtures/app-service/apple/PlatformPlugin.swift");

fn assert_apple_output(root: &Path, path: &str) {
	let swift = read_generated(root, path);
	let expected = if path.starts_with("macos/") {
		include_str!("../../../tests/fixtures/app-service/generated/MacGeneratedPlugins.swift")
	} else {
		include_str!("../../../tests/fixtures/app-service/generated/IosGeneratedPlugins.swift")
	};

	assert_eq!(swift, expected);
	assert!(swift.contains("import ServicePlatform"));
	assert_eq!(swift.matches("dispatcher.register(").count(), 1);

	assert!(swift.contains("PlatformPlugin(context: context)"));
}

fn read_generated(root: &Path, path: &str) -> String {
	std::fs::read_to_string(root.join(".leptoskit").join(path)).unwrap()
}
