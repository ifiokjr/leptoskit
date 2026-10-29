//! Enables native common controls and per-monitor DPI awareness.

fn main() {
	println!("cargo:rerun-if-changed=app.manifest");

	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
		&& std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
	{
		let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("app.manifest");
		println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
		println!(
			"cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
			manifest.display()
		);
	}
}
