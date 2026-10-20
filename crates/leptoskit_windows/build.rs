fn main() {
	println!("cargo:rerun-if-changed=app.manifest");

	if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
		let manifest = std::path::PathBuf::from(
			std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets the manifest directory"),
		)
		.join("app.manifest");

		for target in ["tests", "examples"] {
			println!("cargo:rustc-link-arg-{target}=/MANIFEST:EMBED");
			println!(
				"cargo:rustc-link-arg-{target}=/MANIFESTINPUT:{}",
				manifest.display()
			);
		}
	}
}
