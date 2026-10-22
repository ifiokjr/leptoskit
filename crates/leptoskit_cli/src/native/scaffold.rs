//! Native scaffold compatibility and the experimental HarmonyOS starter.

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use leptoskit_config::AppConfig;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::NativeConfig;
use leptoskit_config::NativeTarget;
use leptoskit_config::Platform;

/// Generates the shared application with the local native destination selected.
///
/// # Errors
///
/// Returns an error for an invalid name, SDK, or conflicting template file.
pub fn run(path: &str, name: Option<&str>, sdk: Option<&str>) -> Result<()> {
	run_for_target(path, name, sdk, None)
}

/// Selects a default native target while retaining every standard platform.
/// HarmonyOS keeps its experimental ArkUI scaffold.
///
/// # Errors
///
/// Returns an error for an invalid name, SDK, or conflicting destination.
pub fn run_for_target(
	path: &str,
	name: Option<&str>,
	sdk: Option<&str>,
	target: Option<NativeTarget>,
) -> Result<()> {
	let platform = match target {
		Some(NativeTarget::Harmonyos) => return harmonyos(path, name, sdk),
		Some(NativeTarget::Ios) => Platform::Ios,
		Some(NativeTarget::Android) => Platform::Android,
		Some(NativeTarget::Macos) => Platform::Macos,
		Some(NativeTarget::Windows) => Platform::Windows,
		Some(NativeTarget::Linux) => Platform::Linux,
		None => Platform::Native,
	};

	crate::new_project::run_with_sdk(path, platform, name, sdk)
}

/// Preserves the standalone ArkUI guest until HarmonyOS has the portable
/// adapter.
fn harmonyos(path: &str, name: Option<&str>, sdk: Option<&str>) -> Result<()> {
	let destination = std::path::Path::new(path);
	let name = name
		.or_else(|| destination.file_name().and_then(|name| name.to_str()))
		.context("pass --name when the destination has no package name")?;
	crate::new_project::ensure_package_name(name)?;
	anyhow::ensure!(
		name.bytes().all(|byte| {
			byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
		}),
		"native application names must be lowercase"
	);
	anyhow::ensure!(
		!destination.exists(),
		"{} already exists",
		destination.display()
	);
	let sdk = sdk.map(crate::new_project::resolve_sdk).transpose()?;
	let ffi = crate::new_project::sdk_dependency("leptoskit_bridge_ffi", sdk.as_deref(), &["app"])?;
	let bridge = crate::new_project::sdk_dependency("leptoskit_bridge", sdk.as_deref(), &[])?;
	let guest_bridge =
		crate::new_project::sdk_dependency("leptoskit_bridge", sdk.as_deref(), &["guest"])?;
	let config = LeptoskitConfig {
		app: AppConfig {
			name: name.into(),
			default_platform: Platform::Native,
			..AppConfig::default()
		},
		native: NativeConfig {
			package_id: leptoskit_config::default_package_id(name),
			targets: vec![NativeTarget::Harmonyos],
			..NativeConfig::default()
		},
		..LeptoskitConfig::default()
	};
	super::harmonyos::validate(&config)?;
	let manifest = format!(
		r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
name = "leptoskit_app"
crate-type = ["staticlib"]

[workspace]
members = ["guest"]

[profile.release]
lto = "thin"
codegen-units = 1

# Keep host proc-macros loadable on macOS 27 (rust-lang/rust#157750).
[profile.release.build-override]
strip = "none"

[dependencies]
leptoskit_bridge_ffi = {ffi}
leptoskit_bridge = {bridge}
"#
	);
	let guest = format!(
		r#"[package]
name = "{name}_guest"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
leptoskit_bridge = {guest_bridge}
serde_json = "1"
"#
	);
	let files = [
		("Cargo.toml", manifest),
		("guest/Cargo.toml", guest),
		(
			"guest/src/lib.rs",
			include_str!("templates/harmonyos/guest.rs").into(),
		),
		(
			"src/lib.rs",
			"//! Runtime registration generated from the host's direct \
			 dependencies.\ninclude!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \
			 \"/.leptoskit/registrant.rs\"));\n"
				.into(),
		),
		("leptoskit.toml", config.to_toml_string()?),
		(".gitignore", "/target\n/.leptoskit\n".into()),
		(
			"readme.md",
			include_str!("templates/harmonyos/readme.md").into(),
		),
	];
	std::fs::create_dir_all(destination)?;
	let root = Utf8PathBuf::from_path_buf(std::fs::canonicalize(destination)?)
		.map_err(|_| anyhow::anyhow!("destination path is not UTF-8"))?;

	for (path, content) in files {
		crate::new_project::write_new(&root.join(path), &content)?;
	}

	super::harmonyos::scaffold(root.as_std_path(), &config)?;
	println!(
		"created HarmonyOS native app in {root}\nnext: cd {root} && leptoskit native build \
		 harmonyos"
	);
	Ok(())
}
