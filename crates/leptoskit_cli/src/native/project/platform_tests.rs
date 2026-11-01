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
		"NativeRive.swift",
		"WidgetProps.swift",
		"NativeAppFactory.swift",
		"NativeAppRuntime.swift",
		"NativeRequestExecutor.swift",
		"MobileWidgetRenderer.swift",
		"MobileApplicationChrome.swift",
		"MobileApplicationTeardown.swift",
		"MobileApplicationDevelopment.swift",
		"MobileApplicationGuests.swift",
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
	write(
		&registry.bridge.join("android/proguard-rules.pro"),
		"# bridge keep rules",
	)
	.unwrap();

	write(&root.join("hosts/android/app/src/main/AndroidManifest.xml"),
		"<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\"><application/></manifest>").unwrap();
	android(
		root,
		&leptoskit_config::LeptoskitConfig::default(),
		registry,
		"arm64-v8a",
	)
	.unwrap();

	for name in [
		"NativeRiveAnimator.kt",
		"NativeApp.kt",
		"NativeServiceOwner.kt",
		"NativePluginJobs.kt",
		"NativeWidgetRenderer.kt",
		"NativeWidgetProps.kt",
		"RenderBatch.kt",
		"NativeTextMeasurer.kt",
		"NativeApplicationTeardown.kt",
		"NativeRustAsync.kt",
	] {
		assert!(
			root.join(".leptoskit/android/sources").join(name).is_file(),
			"missing Android helper {name}"
		);
	}
	// R8 reads the rules from beside the sources, not as a compiled source.
	assert_eq!(
		std::fs::read_to_string(root.join(".leptoskit/android/proguard-rules.pro")).unwrap(),
		"# bridge keep rules"
	);
	assert!(
		!root
			.join(".leptoskit/android/sources/proguard-rules.pro")
			.exists()
	);
}

#[test]
fn an_app_with_a_safari_extension_embeds_it_as_an_app_extension_target() {
	let directory = tempfile::tempdir().unwrap();
	let root = directory.path().canonicalize().unwrap();
	let registry = metadata_registry(&root, &[]);
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		"[app]\nplatforms = [\"web\", \"ios\", \"extension\"]\nicon = \
		 \"icon.svg\"\n[native]\npackage_id = \"com.example.notes\"\n[extension]\nbrowsers = \
		 [\"safari\"]\n[extension.popup]\nroute = \"/\"\n",
		&root,
	)
	.unwrap();
	write(
		&root.join("icon.svg"),
		r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8"/></svg>"#,
	)
	.unwrap();
	write(&registry.bridge.join("Package.swift"), "// bridge package").unwrap();
	for source in platform_sources::IOS {
		write(&registry.bridge.join(source), source).unwrap();
	}
	let package = root.join(".leptoskit/extension/safari");
	write(&package.join("manifest.json"), "{}").unwrap();
	write(&package.join("pkg/app_bg.wasm"), "").unwrap();

	ios::generate(&root, &config, &registry).unwrap();
	let project =
		std::fs::read_to_string(root.join(".leptoskit/ios/Host.xcodeproj/project.pbxproj"))
			.unwrap();
	let mut ids = BTreeSet::new();
	for line in project.lines() {
		if let Some((key, _)) = line.trim().split_once(" = {isa = ") {
			assert!(ids.insert(key), "duplicate Xcode object {key}");
		}
	}
	assert!(project.contains("productType = \"com.apple.product-type.app-extension\""));
	assert!(project.contains("PRODUCT_BUNDLE_IDENTIFIER = com.example.notes.SafariExtension;"));
	assert!(project.contains("name = \"Embed Foundation Extensions\""));
	assert!(project.contains("dstSubfolderSpec = 13;"));
	// The package's files and folders go to the extension's top level.
	assert_eq!(project.matches("lastKnownFileType = folder;").count(), 1);
	assert!(project.contains("manifest.json"));
	// app.icon is the app's asset catalog, and its icon.
	assert!(project.contains("lastKnownFileType = folder.assetcatalog;"));
	assert_eq!(
		project
			.matches("ASSETCATALOG_COMPILER_APPICON_NAME = AppIcon;")
			.count(),
		2
	);
	assert!(
		root.join(".leptoskit/ios/Assets.xcassets/AppIcon.appiconset/icon-1024.png")
			.is_file()
	);
	let extension = root.join(".leptoskit/ios/SafariExtension");
	assert!(
		std::fs::read_to_string(extension.join("Info.plist"))
			.unwrap()
			.contains("com.apple.Safari.web-extension")
	);
	assert!(
		std::fs::read_to_string(extension.join("SafariWebExtensionHandler.swift"))
			.unwrap()
			.contains("NSExtensionRequestHandling")
	);
}
