//! Embeds the Win32 backend's application manifest in the desktop gallery
//! and in the tests that open its windows.
//!
//! `leptoskit_windows` needs common controls version 6 and per-monitor DPI
//! awareness, which only an executable's manifest can request.

fn main() {
	let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
		.join("../../crates/leptoskit_windows/app.manifest");
	println!("cargo:rerun-if-changed={}", manifest.display());

	if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
		for target in ["bin=gallery", "tests"] {
			println!("cargo:rustc-link-arg-{target}=/MANIFEST:EMBED");
			println!(
				"cargo:rustc-link-arg-{target}=/MANIFESTINPUT:{}",
				manifest.display()
			);
		}
	}
}
