//! `leptoskit new`: scaffolds a project from the embedded template.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use leptoskit_config::AppConfig;
use leptoskit_config::BuildConfig;
use leptoskit_config::CodePushConfig;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::NativeConfig;
use leptoskit_config::Platform;
use leptoskit_config::PluginSpec;
use leptoskit_config::ServeConfig;

/// Writes a new project with a `leptoskit.toml` and a starter main.
pub fn run(path: &str, platform: Platform, name: Option<&str>) -> Result<()> {
	let dir = Utf8PathBuf::from(path);
	std::fs::create_dir_all(&dir).with_context(|| format!("could not create {dir}"))?;
	let app_name = name.unwrap_or_else(|| {
		dir.file_name()
			.unwrap_or("leptoskit_app")
			.trim_end_matches('/')
	});

	let mut config = LeptoskitConfig {
		app: AppConfig {
			name: app_name.to_owned(),
			..AppConfig::default()
		},
		serve: ServeConfig::default(),
		build: BuildConfig::default(),
		native: NativeConfig {
			package_id: format!("com.leptoskit.{}", app_name.replace(['-', '.'], "_")),
			..NativeConfig::default()
		},
		codepush: CodePushConfig::default(),
		plugins: vec![PluginSpec {
			name: "leptos_meta".to_owned(),
			leptos: true,
			..PluginSpec::default()
		}],
	};
	config.app.default_platform = platform;

	let config_path = dir.join(leptoskit_config::CONFIG_FILE);
	if config_path.exists() {
		anyhow::bail!("{config_path} already exists");
	}
	std::fs::write(
		&config_path,
		config.to_toml_string().context("serialize config")?,
	)
	.with_context(|| format!("could not write {config_path}"))?;

	let src = dir.join("src");
	std::fs::create_dir_all(&src).with_context(|| format!("could not create {src}"))?;
	std::fs::write(src.join("main.rs"), MAIN_RS)
		.with_context(|| format!("could not write {}", src.join("main.rs")))?;

	// Native projects carry their Apple host as a Swift Package: no Podfile,
	// no generated Xcode project — `open hosts/apple` and Xcode understands
	// the folder as-is. See docs/platforms/host-projects.md for the design.
	if platform == Platform::Native {
		let apple = dir.join("hosts").join("apple");
		let sources = apple.join("Sources").join(&config.app.name);
		std::fs::create_dir_all(&sources).with_context(|| format!("could not create {sources}"))?;
		std::fs::write(
			apple.join("Package.swift"),
			apple_package_swift(&config.app.name, &config.native.package_id),
		)
		.with_context(|| format!("could not write {}", apple.join("Package.swift")))?;
		std::fs::write(sources.join("main.swift"), APPLE_MAIN_SWIFT)
			.with_context(|| format!("could not write {}", sources.join("main.swift")))?;
	}

	println!("created {dir} ({platform} project)");
	Ok(())
}

/// The Apple host package manifest. The executable is real and builds with
/// `swift build` today; the runtime bridge lines are where the wasm host's
/// C ABI lands (tracked with the native backend work).
fn apple_package_swift(name: &str, package_id: &str) -> String {
	format!(
		r#"// swift-tools-version:6.0
// The leptoskit Apple host: a Swift Package, not an Xcode project or a
// CocoaPods integration. `swift build` compiles this skeleton as-is, and
// Xcode opens the folder directly (`open hosts/apple`).
//
// The app identity lives in leptoskit.toml: package id `{package_id}`.
import PackageDescription

let package = Package(
    name: "{name}",
    platforms: [.macOS(.v14), .iOS(.v17)],
    targets: [
        .executableTarget(
            name: "{name}",
            path: "Sources/{name}"
            // The leptoskit runtime links here as a static library target
            // with a C header, once the native backend ABI lands:
            //
            //   .linkedLibrary("leptoskit_runtime")
            //
            // The host then boots the wasm bundle, pumps events, and checks
            // for code push updates — see docs/platforms/host-projects.md.
        )
    ]
)
"#
	)
}

/// The host entry point skeleton.
const APPLE_MAIN_SWIFT: &str = r#"// The leptoskit host entry point.
//
// Three responsibilities, and only three: boot the wasm runtime over the
// app bundle, forward platform events, and check for code push updates on
// launch. The interface itself is a wasm module downloaded over the air —
// this file changes only when the host's native capabilities change.
//
// Runtime bridge lands with the native backend ABI; this skeleton builds
// with `swift build` today so the package shape is continuously verified.
print("leptoskit host skeleton: the runtime bridge attaches here")
"#;

/// The starter main: a headless native layout smoke test that doubles as the
/// project's first test.
const MAIN_RS: &str = r#"fn main() {
	println!("hello from leptoskit");
}
"#;

/// The files a scaffold always writes.
pub fn template_files() -> &'static [&'static str] {
	&[leptoskit_config::CONFIG_FILE, "src/main.rs"]
}

/// The files only a native scaffold writes: the Swift Package host.
pub fn native_template_files(name: &str) -> Vec<String> {
	vec![
		"hosts/apple/Package.swift".to_owned(),
		format!("hosts/apple/Sources/{name}/main.swift"),
	]
}

/// Ensures the scaffold writes every file it promises.
pub fn verify_scaffold(dir: &Path) -> Result<()> {
	for file in template_files() {
		let path = dir.join(file);
		anyhow::ensure!(path.is_file(), "scaffold is missing {file}");
	}
	Ok(())
}
