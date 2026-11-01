//! Copies the registrant `leptoskit build --platform macos` generates into
//! this crate's build.
//!
//! The CLI builds the host into `macos/target/leptoskit-native/`, with the
//! plugin features the registrant needs. Every other build, such as the
//! workspace's lints and tests, gets an empty registrant, so it needs neither
//! a macOS build first nor those features.

use std::path::Path;
use std::path::PathBuf;

fn main() {
	let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
	let generated = manifest.join("../.leptoskit/macos/registrant.rs");
	let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets it"));
	println!("cargo::rerun-if-changed={}", generated.display());
	let registrant = if built_by_leptoskit(&out) {
		std::fs::read_to_string(&generated).unwrap_or_default()
	} else {
		String::new()
	};
	std::fs::write(out.join("registrant.rs"), registrant).expect("OUT_DIR is writable");
}

fn built_by_leptoskit(out: &Path) -> bool {
	out.components()
		.any(|component| component.as_os_str() == "leptoskit-native")
}
