//! Local macOS development bundles. Distribution signing remains app-owned.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;

use super::Target;
use super::project::copy;
use super::project::copy_package;
use super::project::write;
use super::registry::Registry;

pub fn generate(root: &Path, _config: &LeptoskitConfig, registry: &Registry) -> Result<()> {
	let directory = root.join(".leptoskit/macos");
	let packages = directory.join("packages");
	copy_package(&registry.bridge, &packages.join("leptoskit_bridge_ffi"))?;
	let mut dependencies = String::from(".package(path: \"packages/leptoskit_bridge_ffi\"),\n");
	let mut products =
		String::from(".product(name: \"LeptosKitBridge\", package: \"leptoskit_bridge_ffi\"),\n");
	for support in &registry.support {
		if let Some(module) = &support.apple_module {
			copy_package(&support.directory, &packages.join(&support.package_name))?;
			writeln!(
				dependencies,
				".package(path: \"packages/{}\"),",
				support.package_name
			)?;
			writeln!(
				products,
				".product(name: {module:?}, package: {:?}),",
				support.package_name
			)?;
		}
	}
	let mut registrant = String::from("import CLeptosKitBridge\nimport LeptosKitBridge\n");
	let mut registrations = String::new();
	for plugin in &registry.plugins {
		if plugin.rust_type.is_some() || plugin.rust_factory.is_some() {
			continue;
		}
		let module = plugin
			.apple_module
			.as_deref()
			.context("missing Apple module")?;
		let kind = plugin
			.apple_type
			.as_deref()
			.context("missing Apple plugin type")?;
		copy_package(&plugin.directory, &packages.join(&plugin.package_name))?;
		writeln!(
			dependencies,
			".package(path: \"packages/{}\"),",
			plugin.package_name
		)?;
		writeln!(
			products,
			".product(name: {module:?}, package: {:?}),",
			plugin.package_name
		)?;
		writeln!(registrant, "import {module}")?;
		writeln!(
			registrations,
			"        try dispatcher.register({kind}({}), on: bridge)",
			if plugin.apple_context {
				"context: context"
			} else {
				""
			}
		)?;
	}
	writeln!(
		registrant,
		"@MainActor\nenum GeneratedPlugins {{\n    static func register(_ bridge: OpaquePointer, \
		 _ dispatcher: NativeDispatcher, _ context: NativePluginContext) throws \
		 {{\n{registrations}    }}\n}}"
	)?;
	write(
		&directory.join("Sources/GeneratedPlugins.swift"),
		&registrant,
	)?;
	copy(
		&super::host_dir(root, Target::Macos).join("App.swift"),
		&directory.join("Sources/App.swift"),
	)?;
	copy(
		&registry
			.bridge
			.join("desktop/LeptosKitMacViewController.swift"),
		&directory.join("Sources/LeptosKitMacViewController.swift"),
	)?;
	copy(
		&registry.bridge.join("mobile/RenderTransaction.swift"),
		&directory.join("Sources/RenderTransaction.swift"),
	)?;
	copy(
		&registry.bridge.join("mobile/NativeRustWorker.swift"),
		&directory.join("Sources/NativeRustWorker.swift"),
	)?;
	copy(
		&registry.bridge.join("mobile/NativeRustAsync.swift"),
		&directory.join("Sources/NativeRustAsync.swift"),
	)?;
	write(
		&directory.join("Package.swift"),
		&format!(
			"// swift-tools-version:6.0\nimport PackageDescription\nlet package = Package(\nname: \
			 \"App\", platforms: [.macOS(.v14)],\ndependencies: [{dependencies}],\ntargets: \
			 [.executableTarget(name: \"App\", dependencies: [{products}], path: \"Sources\", \
			 sources: [\"App.swift\", \"GeneratedPlugins.swift\", \
			 \"LeptosKitMacViewController.swift\", \"RenderTransaction.swift\", \
			 \"NativeRustWorker.swift\", \"NativeRustAsync.swift\"])]\n)\n"
		),
	)?;
	Ok(())
}

pub fn build(root: &Path, config: &LeptoskitConfig, release: bool) -> Result<()> {
	let directory = root.join(".leptoskit/macos");
	let profile = if release { "release" } else { "debug" };
	let library = super::generated_dir(root, Target::Macos).join("lib");
	super::run_tool(
		Command::new("swift")
			.current_dir(&directory)
			.args(["build", "-c", profile, "-Xlinker", "-L"])
			.arg("-Xlinker")
			.arg(&library)
			// SwiftPM does not dead-strip: without this the app carries every
			// object the Rust archive exports and nothing prunes them.
			.args(["-Xlinker", "-dead_strip"]),
	)?;
	let output = Command::new("swift")
		.current_dir(&directory)
		.args(["build", "-c", profile, "--show-bin-path"])
		.output()?;
	anyhow::ensure!(
		output.status.success(),
		"could not find the Swift executable"
	);
	let binary = String::from_utf8(output.stdout)?;
	let binary = Path::new(binary.trim()).join("App");
	if release {
		// SwiftPM release binaries keep their symbol tables; an executable
		// exports nothing, so the whole table can go before signing.
		super::run_tool(Command::new("strip").arg(&binary))?;
	}
	let bundle = assemble(root, config, &binary)?;
	super::run_tool(
		Command::new("codesign")
			.args(["--force", "--sign", "-", "--timestamp=none"])
			.arg(&bundle),
	)?;
	super::run_tool(
		Command::new("codesign")
			.args(["--verify", "--strict"])
			.arg(&bundle),
	)?;
	println!("built application at {}", bundle.display());
	Ok(())
}

fn assemble(root: &Path, config: &LeptoskitConfig, binary: &Path) -> Result<std::path::PathBuf> {
	let application = root.join(".leptoskit/macos/App.app");
	let bundle = application.join("Contents");
	copy(binary, &bundle.join("MacOS/App"))?;
	copy(
		&super::generated_dir(root, Target::Macos).join("assets/app.wasm"),
		&bundle.join("Resources/app.wasm"),
	)?;
	let info =
		match std::fs::read_to_string(super::host_dir(root, Target::Macos).join("Info.plist")) {
			Ok(info) => info,
			// Apps generated before the app-owned plist was introduced still
			// build.
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
				include_str!("templates/MacInfo.plist")
					.replace("__PACKAGE__", &config.native.package_id)
					.replace("__NAME__", &config.app.name)
					.replace(
						"__VERSION__",
						&format!(
							"{}.{}.{}",
							config.app.version.major,
							config.app.version.minor,
							config.app.version.patch
						),
					)
			}
			Err(error) => return Err(error).context("read hosts/macos/Info.plist"),
		};
	write(&bundle.join("Info.plist"), &info)?;
	Ok(application)
}

#[cfg(test)]
mod tests {
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
		let bridge = root.join("sdk/leptoskit_bridge_ffi");
		let clipboard = root.join("sdk/leptoskit_plugin_clipboard");
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
		write(
			&bridge.join("mobile/NativeRustAsync.swift"),
			"// native async",
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
		let mut registry = Registry {
			package: "demo".into(),
			plugins: vec![plugin],
			support: vec![],
			bridge,
			target: root.join("target"),
			host_features: vec![],
		};
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
		let bridge = root.join("sdk/leptoskit_bridge_ffi");
		for source in [
			"Package.swift",
			"desktop/LeptosKitMacViewController.swift",
			"mobile/RenderTransaction.swift",
			"mobile/NativeRustWorker.swift",
			"mobile/NativeRustAsync.swift",
		] {
			write(&bridge.join(source), "// bridge").unwrap();
		}
		let sensor = root.join("sdk/leptoskit_sensor");
		write(&sensor.join("Package.swift"), "// shared sensor").unwrap();
		write(&root.join("hosts/macos/App.swift"), "// app").unwrap();
		let mut registry = Registry {
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
		};
		generate(root, &LeptoskitConfig::default(), &registry).unwrap();
		let package = std::fs::read_to_string(root.join(".leptoskit/macos/Package.swift")).unwrap();
		assert!(package.contains(".package(path: \"packages/leptoskit_sensor\")"));
		assert!(
			package.contains(".product(name: \"LeptosKitSensor\", package: \"leptoskit_sensor\")")
		);
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
}
