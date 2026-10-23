use super::*;
use crate::native::registry::Plugin;

#[test]
fn assembly_preserves_app_owned_usage_descriptions() {
	let root = std::env::temp_dir()
		.canonicalize()
		.unwrap()
		.join(format!("leptoskit-macos-assembly-{}", std::process::id()));
	let plist = "<plist><dict><key>NSMicrophoneUsageDescription</key><string>Record a voice \
	             note</string></dict></plist>";
	write(&root.join("hosts/macos/Info.plist"), plist).unwrap();
	write(&root.join("binary"), "executable").unwrap();
	write(&root.join(".leptoskit/assets/app.wasm"), "guest").unwrap();
	let bundle = assemble(&root, &LeptoskitConfig::default(), &root.join("binary")).unwrap();
	assert_eq!(
		std::fs::read_to_string(bundle.join("Contents/Info.plist")).unwrap(),
		plist
	);
	assert_eq!(
		std::fs::read_to_string(bundle.join("Contents/Resources/app.wasm")).unwrap(),
		"guest"
	);

	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn registers_only_selected_native_plugins_and_preserves_user_host() {
	let temporary = std::env::temp_dir()
		.canonicalize()
		.unwrap()
		.join(format!("leptoskit-macos-generation-{}", std::process::id()));
	let root = temporary.as_path();

	if root.exists() {
		std::fs::remove_dir_all(root).unwrap();
	}

	let mut registry = clipboard_registry(root);
	generate(root, &LeptoskitConfig::default(), &registry).unwrap();
	let generated = root.join(".leptoskit/macos");
	assert_eq!(
		std::fs::read_to_string(generated.join("Sources/App.swift")).unwrap(),
		"// customized window"
	);
	assert!(
		std::fs::read_to_string(generated.join("Sources/GeneratedPlugins.swift"))
			.unwrap()
			.contains("dispatcher.register(ClipboardPlugin(), on: bridge)")
	);
	assert!(
		std::fs::read_to_string(generated.join("Package.swift"))
			.unwrap()
			.contains(".package(path: \"packages/leptoskit_plugin_clipboard\")")
	);
	assert!(
		generated
			.join("packages/leptoskit_bridge_ffi/Package.swift")
			.is_file()
	);
	assert_eq!(
		std::fs::read_to_string(root.join("hosts/macos/App.swift")).unwrap(),
		"// customized window"
	);

	registry.plugins[0].apple_context = true;
	generate(root, &LeptoskitConfig::default(), &registry).unwrap();
	let context_registrant =
		std::fs::read_to_string(generated.join("Sources/GeneratedPlugins.swift")).unwrap();
	assert!(context_registrant.contains("_ context: NativePluginContext"));
	assert!(context_registrant.contains("ClipboardPlugin(context: context)"));
	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn assembly_falls_back_to_the_template_plist_for_older_apps() {
	let root = std::env::temp_dir()
		.canonicalize()
		.unwrap()
		.join(format!("leptoskit-macos-plist-{}", std::process::id()));
	write(&root.join("binary"), "executable").unwrap();
	write(&root.join(".leptoskit/assets/app.wasm"), "guest").unwrap();
	let mut config = LeptoskitConfig::default();
	config.native.package_id = "dev.example.legacy".into();
	let bundle = assemble(&root, &config, &root.join("binary")).unwrap();
	let info = std::fs::read_to_string(bundle.join("Contents/Info.plist")).unwrap();
	assert!(info.contains("dev.example.legacy"));

	assert!(!info.contains("__PACKAGE__") && !info.contains("__NAME__"));
	assert!(!info.contains("__VERSION__"));
	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn support_packages_link_and_incomplete_apple_metadata_fails() {
	let temporary = std::env::temp_dir()
		.canonicalize()
		.unwrap()
		.join(format!("leptoskit-macos-support-{}", std::process::id()));
	let root = temporary.as_path();

	if root.exists() {
		std::fs::remove_dir_all(root).unwrap();
	}

	let mut registry = support_registry(root);
	generate(root, &LeptoskitConfig::default(), &registry).unwrap();
	let package = std::fs::read_to_string(root.join(".leptoskit/macos/Package.swift")).unwrap();
	assert!(package.contains(".package(path: \"packages/leptoskit_sensor\")"));
	assert!(package.contains(".product(name: \"LeptosKitSensor\", package: \"leptoskit_sensor\")"));
	assert!(
		root.join(".leptoskit/macos/packages/leptoskit_sensor/Package.swift")
			.is_file()
	);

	// A native plugin without its Apple type cannot be registered.
	registry.plugins.push(
		serde_json::from_value(serde_json::json!({
			"name": "broken", "apple-module": "Broken"
		}))
		.unwrap(),
	);
	assert!(generate(root, &LeptoskitConfig::default(), &registry).is_err());
	std::fs::remove_dir_all(root).unwrap();
}

fn clipboard_registry(root: &Path) -> Registry {
	let bridge = root.join("sdk/leptoskit_bridge_ffi");
	let clipboard = root.join("sdk/leptoskit_plugin_clipboard");

	for source in crate::native::project::platform_sources::MACOS {
		write(&bridge.join(source), "// bridge source").unwrap();
	}

	write(&bridge.join("Package.swift"), "// bridge").unwrap();
	write(
		&bridge.join("desktop/LeptosKitMacViewController.swift"),
		"// renderer",
	)
	.unwrap();

	write(
		&bridge.join("mobile/RenderTransaction.swift"),
		"// transactions",
	)
	.unwrap();
	write(
		&bridge.join("mobile/NativeRustWorker.swift"),
		"// native worker",
	)
	.unwrap();
	write(&clipboard.join("Package.swift"), "// clipboard").unwrap();
	write(&root.join("hosts/macos/App.swift"), "// customized window").unwrap();
	let mut plugin: Plugin = serde_json::from_value(serde_json::json!({
		"name": "clipboard",
		"apple-module": "LeptosKitClipboard",
		"apple-type": "ClipboardPlugin"
	}))
	.unwrap();
	plugin.directory = clipboard;
	plugin.package_name = "leptoskit_plugin_clipboard".into();
	plugin.crate_name = "clipboard".into();

	Registry {
		package: "demo".into(),
		plugins: vec![plugin],
		support: vec![],
		bridge,
		target: root.join("target"),
		host_features: vec![],
	}
}

fn support_registry(root: &Path) -> Registry {
	let bridge = root.join("sdk/leptoskit_bridge_ffi");

	for source in crate::native::project::platform_sources::MACOS {
		write(&bridge.join(source), "// bridge source").unwrap();
	}
	for source in [
		"Package.swift",
		"desktop/LeptosKitMacViewController.swift",
		"mobile/RenderTransaction.swift",
		"mobile/NativeRustWorker.swift",
	] {
		write(&bridge.join(source), "// bridge").unwrap();
	}
	let sensor = root.join("sdk/leptoskit_sensor");
	write(&sensor.join("Package.swift"), "// shared sensor").unwrap();

	write(&root.join("hosts/macos/App.swift"), "// app").unwrap();

	Registry {
		package: "demo".into(),
		plugins: vec![],
		support: vec![crate::native::registry::NativeSupport {
			apple_module: Some("LeptosKitSensor".into()),
			android: false,
			directory: sensor,
			package_name: "leptoskit_sensor".into(),
		}],
		bridge,
		target: root.join("target"),
		host_features: vec![],
	}
}
