//! Embeds the prebuilt wasm guest when `LEPTOSKIT_BENCH_GUEST_WASM` names
//! it, so the one binary carries its workload onto a device where no file
//! can be shipped alongside. Without the variable the wasm scenarios are
//! skipped at run time and say so.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
	println!("cargo:rerun-if-env-changed=LEPTOSKIT_BENCH_GUEST_WASM");
	let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
	let generated = out_dir.join("embedded_guest.rs");
	match env::var("LEPTOSKIT_BENCH_GUEST_WASM") {
		Ok(path) if !path.is_empty() => {
			println!("cargo:rerun-if-changed={path}");
			let destination = out_dir.join("guest.wasm");
			fs::copy(&path, &destination).unwrap_or_else(|error| {
				panic!("could not copy the guest wasm from {path}: {error}")
			});
			fs::write(
				&generated,
				"pub const GUEST_WASM: Option<&[u8]> = Some(include_bytes!(\"guest.wasm\"));\n",
			)
			.expect("write embedded_guest.rs");
		}
		_ => {
			fs::write(&generated, "pub const GUEST_WASM: Option<&[u8]> = None;\n")
				.expect("write embedded_guest.rs");
		}
	}
}
