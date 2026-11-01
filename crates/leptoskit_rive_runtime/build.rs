//! Verifies and compiles pinned third-party sources without build-time
//! downloads.

use std::env;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use sha2::Digest;

fn sources(directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
	for entry in fs::read_dir(directory)? {
		let path = entry?.path();

		if path.is_dir() {
			sources(&path, files)?;
		} else if path.extension().is_some_and(|extension| extension == "cpp") {
			files.push(path);
		}
	}

	Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	println!("cargo:rerun-if-changed=vendor/runtime.tar.gz");

	if env::var("CARGO_CFG_TARGET_ARCH")? == "wasm32" {
		return Ok(());
	}

	let compressed = fs::read("vendor/runtime.tar.gz")?;
	let digest = format!("{:x}", sha2::Sha256::digest(&compressed));

	if digest != "5d2106870e8caeb9a69ebd702e3874560ca6f628ec0eaed1b0c1006f59521872" {
		return Err("pinned Rive source archive checksum mismatch".into());
	}

	let output = PathBuf::from(env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?);
	// Keep MSVC include paths short. Replace the extraction as a whole so a
	// pinned revision update cannot retain sources removed upstream.
	let root = output.join("rive");

	if root.exists() {
		fs::remove_dir_all(&root)?;
	}

	let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(compressed.as_slice()));
	archive.unpack(&root)?;

	let mut files = Vec::new();
	sources(&root.join("src"), &mut files)?;
	files.sort();
	let optimized = env::var("PROFILE")? == "release";
	let mut runtime = cc::Build::new();
	runtime
		.cpp(true)
		.std("c++17")
		.debug(false)
		.opt_level(if optimized { 2 } else { 1 })
		.include(root.join("include"))
		.define("_RIVE_INTERNAL_", None)
		.define("_USE_MATH_DEFINES", None)
		.files(files)
		.flag_if_supported("-Wno-deprecated-declarations")
		.warnings(false);

	if runtime.get_compiler().is_like_msvc() {
		// Preserve destructors when an upstream allocation throws into the
		// adapter's exception boundary. MSVC also needs the complete context
		// type before instantiating text.cpp's reference-counted return value.
		runtime
			.flag("/EHsc")
			.flag("/FIrive/data_bind/data_context.hpp");
	}

	runtime.compile("leptoskit_rive_runtime");
	println!("cargo:include={}", root.join("include").display());
	Ok(())
}
