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

	support_packages(registry, &packages, &mut dependencies, &mut products)?;
	let mut registrant = String::from("import CLeptosKitBridge\nimport LeptosKitBridge\n");
	let mut registrations = String::new();

	plugin_packages(
		registry,
		&packages,
		&mut dependencies,
		&mut products,
		&mut registrant,
		&mut registrations,
	)?;
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
	let mut sources = vec!["App.swift".to_owned(), "GeneratedPlugins.swift".to_owned()];

	for source in super::project::platform_sources::MACOS {
		let filename = Path::new(source)
			.file_name()
			.context("native source filename missing")?;
		copy(
			&registry.bridge.join(source),
			&directory.join("Sources").join(filename),
		)?;
		sources.push(
			filename
				.to_str()
				.context("native source filename is not UTF-8")?
				.to_owned(),
		);
	}

	let sources = sources
		.iter()
		.map(|source| format!("{source:?}"))
		.collect::<Vec<_>>()
		.join(", ");
	write(
		&directory.join("Package.swift"),
		&format!(
			"// swift-tools-version:6.0\nimport PackageDescription\nlet package = Package(\nname: \
			 \"App\", platforms: [.macOS(.v14)],\ndependencies: [{dependencies}],\ntargets: \
			 [.executableTarget(name: \"App\", dependencies: [{products}], path: \"Sources\", \
			 sources: [{sources}])])\n"
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
			// Older apps lack an app-owned plist.
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
mod tests;

fn support_packages(
	registry: &Registry,
	packages: &Path,
	dependencies: &mut String,
	products: &mut String,
) -> Result<()> {
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

	Ok(())
}

fn plugin_packages(
	registry: &Registry,
	packages: &Path,
	dependencies: &mut String,
	products: &mut String,
	registrant: &mut String,
	registrations: &mut String,
) -> Result<()> {
	for plugin in &registry.plugins {
		if plugin.has_rust_registration() {
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

	Ok(())
}
