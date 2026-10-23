//! Compile real consumers to verify platform selection and Cargo invalidation.

use std::path::Path;
use std::process::Command;
use std::process::Output;

/// Writes a fixture source, including directories introduced during a rebuild.
fn write(root: &Path, path: &str, contents: &str) {
	let path = root.join(path);

	std::fs::create_dir_all(path.parent().unwrap()).unwrap();
	std::fs::write(path, contents).unwrap();
}

/// Compiles and executes the fixture without accessing the network.
fn run(root: &Path, features: Option<&str>) -> Output {
	let mut command = Command::new(env!("CARGO"));
	command
		.current_dir(root)
		.env("CARGO_TARGET_DIR", root.join("target"))
		.args(["run", "--offline", "--quiet", "--no-default-features"]);

	if let Some(features) = features {
		command.args(["--features", features]);
	}

	command.output().unwrap()
}

/// Checks the observed module behavior, reporting the complete compiler error.
fn assert_value(root: &Path, features: Option<&str>, expected: &str) {
	let output = run(root, features);
	let stderr = String::from_utf8_lossy(&output.stderr);
	let stdout = String::from_utf8_lossy(&output.stdout);

	assert!(output.status.success(), "features={features:?}:\n{stderr}");
	assert_eq!(stdout.trim(), expected, "features={features:?}:\n{stderr}");
}

/// A module may select a nested module using the same source-root contract.
fn profile(platform: &str) -> String {
	let filename = if platform == "shared" {
		"profile.rs".to_owned()
	} else {
		format!("profile.{platform}.rs")
	};

	format!(
		r#"//! The selected profile module retains its original module attributes.
#![deny(unsafe_code)]

leptoskit_macros::platform_module!(pub mod details = "routes/profile/details");

pub fn value() -> &'static str {{
    assert_eq!(super::application_marker(), "app");
    assert!(file!().ends_with("{filename}"));
    self::selected_value()
}}

fn selected_value() -> &'static str {{ "{platform}" }}
"#,
	)
}

/// Children keep direct access to their parent's private items and themselves.
fn details(platform: &str) -> String {
	format!(
		r#"//! The selected child module retains its module documentation.
#![deny(unsafe_code)]

pub fn value() -> &'static str {{
    let _parent = super::selected_value();
    self::selected_value()
}}

fn selected_value() -> &'static str {{ "{platform}" }}
"#,
	)
}

#[test]
fn platform_sources_compile_and_additions_and_deletions_invalidate_cargo() {
	let directory = tempfile::tempdir().unwrap();
	let root = directory.path();
	let macros = Path::new(env!("CARGO_MANIFEST_DIR"));
	let build = macros.parent().unwrap().join("leptoskit_build");
	let manifest = format!(
		r#"[package]
name = "platform_modules_probe"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
leptoskit_macros = {{ path = {macros:?} }}

[build-dependencies]
leptoskit_build = {{ path = {build:?} }}

[features]
default = []
web = []
ios = []
android = []
macos = []
windows = []
linux = []
"#,
	);

	write(root, "Cargo.toml", &manifest);
	write(
		root,
		"build.rs",
		"fn main() -> Result<(), leptoskit_build::Error> { leptoskit_build::platforms() }",
	);
	write(
		root,
		"src/main.rs",
		r#"leptoskit_macros::platform_module!(pub mod profile = "routes/profile");
fn application_marker() -> &'static str { "app" }
fn main() { println!("{}:{}", profile::value(), profile::details::value()); }
"#,
	);
	write(root, "src/routes/profile.rs", &profile("shared"));
	write(root, "src/routes/profile/details.rs", &details("shared"));
	assert_value(root, None, "shared:shared");
	assert_value(root, Some("ios"), "shared:shared");

	// Neither the shared module nor the feature selection changes: Cargo
	// must notice a new override through the build script's directory watch.
	write(root, "src/routes/profile.ios.rs", &profile("ios"));
	assert_value(root, Some("ios"), "ios:shared");
	write(root, "src/routes/profile/details.ios.rs", &details("ios"));
	assert_value(root, Some("ios"), "ios:ios");
	std::fs::remove_file(root.join("src/routes/profile.ios.rs")).unwrap();
	assert_value(root, Some("ios"), "shared:ios");
	std::fs::remove_file(root.join("src/routes/profile/details.ios.rs")).unwrap();
	assert_value(root, Some("ios"), "shared:shared");

	for platform in ["web", "ios", "android", "macos", "windows", "linux"] {
		write(
			root,
			&format!("src/routes/profile.{platform}.rs"),
			&profile(platform),
		);
		write(
			root,
			&format!("src/routes/profile/details.{platform}.rs"),
			&details(platform),
		);
		assert_value(root, Some(platform), &format!("{platform}:{platform}"));
	}

	assert_value(root, None, "shared:shared");

	// A selected file uses Rust's #[path] rules, so a plain child mod looks
	// beside profile.rs, rather than in profile/. Registering the nested
	// module explicitly preserves its logical path and platform overrides.
	for (source, feature, platform) in [
		("src/routes/profile.rs", None, "shared"),
		("src/routes/profile.ios.rs", Some("ios"), "ios"),
	] {
		write(
			root,
			source,
			"pub mod details; pub fn value() -> &'static str { \"profile\" }",
		);
		let output = run(root, feature);
		let stderr = String::from_utf8_lossy(&output.stderr);

		assert!(
			!output.status.success(),
			"ordinary children follow #[path] lookup rules"
		);
		assert!(
			stderr.contains("file not found for module `details`"),
			"{stderr}"
		);
		assert!(
			stderr.replace('\\', "/").contains("routes/details.rs"),
			"{stderr}"
		);
		write(root, source, &profile(platform));
		assert_value(root, feature, &format!("{platform}:{platform}"));
	}

	let output = run(root, Some("web,ios"));
	let stderr = String::from_utf8_lossy(&output.stderr);

	assert!(
		!output.status.success(),
		"conflicting app platform features must fail"
	);
	assert!(stderr.contains("web, ios"), "{stderr}");
	assert!(stderr.contains("--no-default-features"), "{stderr}");

	write(
		root,
		"Cargo.toml",
		&manifest.replace("edition = \"2024\"", "edition = \"2024\"\nbuild = false"),
	);
	let output = run(root, Some("ios"));
	let stderr = String::from_utf8_lossy(&output.stderr);

	assert!(
		!output.status.success(),
		"platform selection requires the build helper"
	);
	assert!(stderr.contains("leptoskit_build::platforms()"), "{stderr}");
}
