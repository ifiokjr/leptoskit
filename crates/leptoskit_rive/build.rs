//! Compiles only the adapter; Cargo retains the separate native runtime build.

use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
	println!("cargo:rerun-if-changed=src/native/runtime.cpp");
	println!("cargo:rerun-if-changed=src/native/runtime.h");

	if env::var_os("CARGO_FEATURE_NATIVE").is_none()
		|| env::var("CARGO_CFG_TARGET_ARCH")? == "wasm32"
	{
		return Ok(());
	}
	let include = PathBuf::from(
		env::var_os("DEP_LEPTOSKIT_RIVE_RUNTIME_INCLUDE")
			.ok_or("missing pinned native runtime headers")?,
	);
	let mut adapter = cc::Build::new();
	adapter
		.cpp(true)
		.std("c++17")
		.file("src/native/runtime.cpp")
		.warnings_into_errors(true);

	// Ignore warnings only inside unmodified upstream headers; our adapter
	// retains the compiler's full warning set with warnings treated as errors.
	if adapter.get_compiler().is_like_msvc() {
		adapter
			.flag("/EHsc")
			.flag(format!("/external:I{}", include.display()))
			.flag("/external:W0");
	} else {
		adapter.flag("-isystem").flag(include);
	}
	adapter.compile("leptoskit_rive_adapter");
	Ok(())
}
