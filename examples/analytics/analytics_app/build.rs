//! Links the server without a PDB on MSVC.
//!
//! leptos view types give this binary symbol names over 100,000 characters
//! long, more than an MSVC PDB can hold (`LNK4319`). The example needs no
//! debugger symbols, so its binaries link with `/DEBUG:NONE`, which follows
//! and overrides the `/DEBUG` rustc passes.

fn main() {
	println!("cargo:rerun-if-changed=build.rs");
	if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
		println!("cargo:rustc-link-arg-bins=/DEBUG:NONE");
	}
}
