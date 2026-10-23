// Expanded shipping helpers remain copied and registered exactly once.

use std::collections::BTreeSet;

use super::*;

#[test]
fn expanded_platform_sources_are_registered_and_xcode_objects_remain_unique() {
	let directory = tempfile::tempdir().unwrap();
	let root = directory.path().canonicalize().unwrap();
	let registry = metadata_registry(&root, &[]);
	let config = leptoskit_config::LeptoskitConfig::default();
	write(&registry.bridge.join("Package.swift"), "// bridge package").unwrap();
	write(&root.join("hosts/macos/App.swift"), "// app").unwrap();

	for source in platform_sources::MACOS.iter().chain(platform_sources::IOS) {
		write(&registry.bridge.join(source), source).unwrap();
	}

	assert_macos_sources(&root, &registry, &config);
	assert_ios_sources(&root, &registry, &config);
	assert_android_sources(&root, &registry);
}

fn assert_macos_sources(
	root: &Path,
	registry: &Registry,
	config: &leptoskit_config::LeptoskitConfig,
) {
	crate::native::desktop::generate(root, config, registry).unwrap();
	let package = std::fs::read_to_string(root.join(".leptoskit/macos/Package.swift")).unwrap();

	for source in platform_sources::MACOS {
		let filename = Path::new(source).file_name().unwrap().to_str().unwrap();
		let copied = root.join(".leptoskit/macos/Sources").join(filename);
		assert_eq!(std::fs::read_to_string(copied).unwrap(), *source);
		assert_eq!(package.matches(&format!("\"{filename}\"")).count(), 1);
	}
}

fn assert_ios_sources(
	root: &Path,
	registry: &Registry,
	config: &leptoskit_config::LeptoskitConfig,
) {
	ios::generate(root, config, registry).unwrap();
	let project =
		std::fs::read_to_string(root.join(".leptoskit/ios/Host.xcodeproj/project.pbxproj"))
			.unwrap();
	let mut ids = BTreeSet::new();

	for line in project.lines() {
		let Some((key, _)) = line.trim().split_once(" = {isa = ") else {
			continue;
		};

		assert!(ids.insert(key), "duplicate Xcode object {key}");
	}

	for name in [
		"NativeAppFactory.swift",
		"NativeAppRuntime.swift",
		"NativeRequestExecutor.swift",
		"MobileWidgetRenderer.swift",
		"MobileApplicationChrome.swift",
		"MobileApplicationTeardown.swift",
		"NativeRustAsync.swift",
	] {
		assert_eq!(
			project.matches(name).count(),
			1,
			"missing or duplicate iOS helper {name}"
		);
	}
}

fn assert_android_sources(root: &Path, registry: &Registry) {
	write(
		&registry.bridge.join("android/NativePlugin.kt"),
		"// bridge plugin",
	)
	.unwrap();
	write(
		&registry.bridge.join("mobile/LeptosKitView.kt"),
		"// bridge view",
	)
	.unwrap();

	write(&root.join("hosts/android/app/src/main/AndroidManifest.xml"),
		"<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\"><application/></manifest>").unwrap();
	android::generate(root, registry, "arm64-v8a").unwrap();

	for name in [
		"NativeApp.kt",
		"NativeServiceOwner.kt",
		"NativePluginJobs.kt",
		"NativeWidgetRenderer.kt",
		"NativeWidgetProps.kt",
		"NativeApplicationTeardown.kt",
		"NativeRustAsync.kt",
	] {
		assert!(
			root.join(".leptoskit/android/sources").join(name).is_file(),
			"missing Android helper {name}"
		);
	}
}
