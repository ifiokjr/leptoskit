mod platform_tests {
	include!("platform_tests.rs");
}
use super::*;

fn metadata_registry(root: &Path, metadata: &[serde_json::Value]) -> Registry {
	for source in platform_sources::ANDROID {
		write(&root.join("bridge").join(source), "// bridge source").unwrap();
	}

	Registry {
		package: "demo".into(),
		plugins: metadata
			.iter()
			.map(|value| serde_json::from_value(value.clone()).unwrap())
			.collect(),
		support: vec![],
		bridge: root.join("bridge"),
		target: root.join("target"),
		host_features: vec![],
	}
}

#[test]
fn ios_requires_selected_plugin_purpose_strings_without_modifying_app_metadata() {
	let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
		"leptoskit-ios-required-purpose-{}",
		std::process::id()
	));
	std::fs::create_dir_all(&root).unwrap();
	let registry = metadata_registry(
		&root,
		&[serde_json::json!({
			"name": "tracking", "apple-module": "Tracking", "apple-type": "Tracking",
			"apple-usage-descriptions": ["NSUserTrackingUsageDescription"]
		})],
	);
	let plist = root.join("hosts/ios/Info.plist");
	assert!(validate_ios_usage_descriptions(&root, &registry).is_err());
	assert!(validate_ios_usage_descriptions(&root, &metadata_registry(&root, &[])).is_ok());
	reject_missing_purpose(&root, &registry, &plist);
	write(&plist, "<invalid").unwrap();

	assert!(validate_ios_usage_descriptions(&root, &registry).is_err());
	let mut values = plist::Dictionary::new();
	values.insert(
		"NSUserTrackingUsageDescription".into(),
		plist::Value::String("Measure which of our app's ads led you here.".into()),
	);
	let values = plist::Value::Dictionary(values);

	for binary in [false, true] {
		if binary {
			values.to_file_binary(&plist).unwrap();
		} else {
			values.to_file_xml(&plist).unwrap();
		}

		let original = std::fs::read(&plist).unwrap();
		validate_ios_usage_descriptions(&root, &registry).unwrap();
		assert_eq!(std::fs::read(&plist).unwrap(), original);
	}

	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn android_emits_selected_maven_dependencies_once_and_removes_stale_entries() {
	let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
		"leptoskit-android-dependencies-{}",
		std::process::id()
	));
	let mut registry = metadata_registry(
		&root,
		&[
			serde_json::json!({
				"name": "review", "rust-type": "Plugin",
				"android-dependencies": ["com.google.android.play:review:2.0.2", "org.example:shared:1.0"]
			}),
			serde_json::json!({
				"name": "other", "rust-type": "Plugin",
				"android-dependencies": ["org.example:shared:1.0"]
			}),
		],
	);
	for source in [
		"android/NativePlugin.kt",
		"mobile/LeptosKitView.kt",
		"android/NativePermissionHost.kt",
		"android/NativeActivityResultHost.kt",
		"mobile/NativeRustWorker.kt",
		"mobile/NativeReleases.kt",
		"mobile/LeptosKitApplicationView.kt",
	] {
		write(&registry.bridge.join(source), "// fixture").unwrap();
	}
	write(
			&root.join("hosts/android/app/src/main/AndroidManifest.xml"),
			"<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\"><application/></manifest>",
		).unwrap();
	android::generate(&root, &registry, "arm64-v8a").unwrap();
	let path = root.join(".leptoskit/android/plugin-dependencies.txt");
	assert_eq!(
		std::fs::read_to_string(&path).unwrap(),
		"com.google.android.play:review:2.0.2\norg.example:shared:1.0\n"
	);
	registry.plugins.remove(0);

	android::generate(&root, &registry, "arm64-v8a").unwrap();
	assert_eq!(
		std::fs::read_to_string(&path).unwrap(),
		"org.example:shared:1.0\n"
	);
	registry.plugins.clear();
	android::generate(&root, &registry, "arm64-v8a").unwrap();
	assert!(std::fs::read(&path).unwrap().is_empty());
	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ios_passes_presentation_context_only_to_plugins_that_request_it() {
	let root = std::env::temp_dir()
		.canonicalize()
		.unwrap()
		.join(format!("leptoskit-ios-context-{}", std::process::id()));
	let bridge = root.join("sdk/leptoskit_bridge_ffi");
	write(&bridge.join("Package.swift"), "// bridge").unwrap();
	let mut plugins = Vec::new();

	for (name, context) in [("Owned", true), ("Plain", false)] {
		let directory = root.join("sdk").join(name);
		write(&directory.join("Package.swift"), "// plugin").unwrap();
		let mut plugin: super::super::registry::Plugin =
			serde_json::from_value(serde_json::json!({
				"name": name,
				"apple-module": name,
				"apple-type": format!("{name}Plugin"),
				"apple-context": context
			}))
			.unwrap();
		plugin.directory = directory;
		plugin.package_name = name.into();
		plugins.push(plugin);
	}

	let registry = Registry {
		package: "demo".into(),
		plugins,
		support: vec![],
		bridge,
		target: root.join("target"),
		host_features: vec![],
	};
	ios::generate(
		&root,
		&leptoskit_config::LeptoskitConfig::default(),
		&registry,
	)
	.unwrap();
	let source =
		std::fs::read_to_string(root.join(".leptoskit/ios/GeneratedPlugins.swift")).unwrap();
	assert!(source.contains("_ context: NativePluginContext"));
	assert!(source.contains("OwnedPlugin(context: context)"));
	assert!(source.contains("PlainPlugin()"));
	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_package_copy_removes_stale_sources_and_rejects_output_symlinks() {
	let root = std::env::temp_dir().join(format!("leptoskit-package-copy-{}", std::process::id()));
	std::fs::create_dir_all(&root).unwrap();
	let root = root.canonicalize().unwrap();
	let source = root.join("source");
	let destination = root.join("generated");
	std::fs::create_dir_all(source.join("apple")).unwrap();
	std::fs::write(source.join("Package.swift"), "// manifest").unwrap();

	std::fs::write(source.join("apple/Plugin.swift"), "// plugin").unwrap();
	copy_package(&source, &destination).unwrap();
	assert_eq!(
		std::fs::read(destination.join("apple/Plugin.swift")).unwrap(),
		b"// plugin"
	);
	std::fs::remove_file(source.join("apple/Plugin.swift")).unwrap();
	copy_package(&source, &destination).unwrap();
	assert!(!destination.join("apple/Plugin.swift").exists());
	assert!(destination.join("Package.swift").is_file());
	#[cfg(unix)]
	{
		std::os::unix::fs::symlink(source.join("Package.swift"), destination.join("linked"))
			.unwrap();
		assert!(write(&destination.join("linked"), "replacement").is_err());
		assert_eq!(
			std::fs::read(source.join("Package.swift")).unwrap(),
			b"// manifest"
		);
		std::os::unix::fs::symlink(&source, destination.join("directory")).unwrap();
		assert!(write(&destination.join("directory/other"), "replacement").is_err());
	}
	std::fs::remove_dir_all(root).unwrap();
}

fn fixture(path: &Path, contents: &str) {
	std::fs::create_dir_all(path.parent().unwrap()).unwrap();
	std::fs::write(path, contents).unwrap();
}

fn plugin(
	metadata: serde_json::Value,
	directory: &Path,
	crate_name: &str,
) -> super::super::registry::Plugin {
	let mut plugin: super::super::registry::Plugin = serde_json::from_value(metadata).unwrap();
	plugin.directory = directory.to_owned();
	plugin.crate_name = crate_name.into();
	plugin.package_name = crate_name.into();
	plugin
}

#[test]
fn generated_hosts_install_rust_workers_and_register_only_native_plugins() {
	let root =
		std::env::temp_dir().join(format!("leptoskit-generate-hosts-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&root);
	std::fs::create_dir_all(&root).unwrap();
	let root = root.canonicalize().unwrap();
	let registry = host_registry(&root);
	let config = leptoskit_config::LeptoskitConfig::default();

	generate(&root, &config, &registry, Target::Android, false).unwrap();
	let registrant = std::fs::read_to_string(root.join(".leptoskit/registrant.rs")).unwrap();
	assert!(registrant.contains("app.install_worker(Box::new(store_crate::Store::default()))?;"));
	assert!(registrant.contains(
		"app.install_worker(Box::new(files_crate::from_app_dirs(&_context.data_dir, \
		 &_context.cache_dir)?))?;"
	));
	assert!(!registrant.contains("haptics"));
	let sources = root.join(".leptoskit/android/sources");
	let kotlin = std::fs::read_to_string(sources.join("GeneratedPlugins.kt")).unwrap();
	assert_eq!(kotlin.matches("dispatcher.register(").count(), 1);

	assert!(kotlin.contains("dispatcher.register(bridge, dev.example.HapticsPlugin(context))"));
	for name in [
		"NativeRustWorker.kt",
		"NativePermissionHost.kt",
		"plugin_2_HapticsPlugin.kt",
		"support_0_Helper.kt",
	] {
		assert!(sources.join(name).is_file(), "missing {name}");
	}
	let manifest =
		std::fs::read_to_string(root.join(".leptoskit/android/AndroidManifest.xml")).unwrap();
	assert!(manifest.contains("android.permission.VIBRATE"));
	assert_eq!(
		std::fs::read_to_string(root.join(".leptoskit/android/abi.txt")).unwrap(),
		"arm64-v8a"
	);

	generate(&root, &config, &registry, Target::IosSimulator, false).unwrap();
	let swift =
		std::fs::read_to_string(root.join(".leptoskit/ios/GeneratedPlugins.swift")).unwrap();
	assert!(swift.contains("import Haptics"));
	assert!(swift.contains("try dispatcher.register(HapticsPlugin(), on: bridge)"));
	assert_eq!(swift.matches("dispatcher.register(").count(), 1);
	let packages = root.join(".leptoskit/ios/packages");
	assert!(packages.join("haptics_crate/Package.swift").is_file());

	assert!(packages.join("support_crate/Package.swift").is_file());
	assert!(!packages.join("store_crate").exists());
	std::fs::remove_dir_all(root).unwrap();
}

fn host_registry(root: &Path) -> Registry {
	let bridge = root.join("bridge");

	for source in platform_sources::ANDROID {
		fixture(&bridge.join(source), "// bridge source");
	}
	for source in [
		"android/NativePlugin.kt",
		"android/NativePermissionHost.kt",
		"android/NativeActivityResultHost.kt",
		"mobile/LeptosKitView.kt",
		"mobile/NativeRustWorker.kt",
		"mobile/NativeReleases.kt",
		"mobile/LeptosKitApplicationView.kt",
		"Package.swift",
	] {
		fixture(&bridge.join(source), "// bridge fixture");
	}
	let haptics = root.join("plugins/haptics");

	fixture(&haptics.join("android/HapticsPlugin.kt"), "// haptics");
	fixture(&haptics.join("Package.swift"), "// haptics package");
	let support = root.join("plugins/support");
	fixture(&support.join("android/Helper.kt"), "// helper");
	fixture(&support.join("Package.swift"), "// helper package");
	fixture(
			&root.join("hosts/android/app/src/main/AndroidManifest.xml"),
			"<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\"><application/></manifest>",
		);
	fixture(&root.join("hosts/ios/App.swift"), "// app");

	Registry {
		package: "demo".into(),
		plugins: vec![
			plugin(
				serde_json::json!({"name": "store", "rust-type": "Store", "rust-worker": true}),
				root,
				"store_crate",
			),
			plugin(
				serde_json::json!({
					"name": "filesystem", "rust-factory": "from_app_dirs", "rust-worker": true
				}),
				root,
				"files_crate",
			),
			plugin(
				serde_json::json!({
					"name": "haptics", "apple-module": "Haptics", "apple-type": "HapticsPlugin",
					"android-type": "dev.example.HapticsPlugin", "android-context": true,
					"android-permissions": ["android.permission.VIBRATE"]
				}),
				&haptics,
				"haptics_crate",
			),
		],
		support: vec![super::super::registry::NativeSupport {
			apple_module: Some("Helper".into()),
			android: true,
			directory: support,
			package_name: "support_crate".into(),
		}],
		bridge,
		target: root.join("target"),
		host_features: vec![],
	}
}

fn reject_missing_purpose(root: &Path, registry: &Registry, plist: &Path) {
	for body in [
		"<dict/>",
		"<dict><key>NSUserTrackingUsageDescription</key><string> \n\t </string></dict>",
		"<dict><key>NSUserTrackingUsageDescription</key><true/></dict>",
		"<dict><key>Nested</key><dict><key>NSUserTrackingUsageDescription</key><string>Purpose</\
		 string></dict></dict>",
		"<array><string>NSUserTrackingUsageDescription</string></array>",
	] {
		write(
			plist,
			&format!("<?xml version=\"1.0\"?><plist version=\"1.0\">{body}</plist>"),
		)
		.unwrap();
		let error = generate(
			root,
			&leptoskit_config::LeptoskitConfig::default(),
			registry,
			Target::IosSimulator,
			false,
		)
		.unwrap_err();
		assert!(format!("{error:#}").contains("Info.plist"));
		assert!(
			!root.join(".leptoskit").exists(),
			"validation must precede generated writes"
		);
	}
}
