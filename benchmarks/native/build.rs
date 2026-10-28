//! Embeds the prebuilt wasm guests when `LEPTOSKIT_BENCH_GUEST_WASM` and
//! `LEPTOSKIT_BENCH_COUNTER_WASM` name them, so the one binary carries its
//! workload onto a device where no file can be shipped alongside. Without a
//! variable its scenarios are skipped at run time and say so.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

fn main() {
	let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
	let mut generated = String::new();

	for (variable, constant, file) in [
		("LEPTOSKIT_BENCH_GUEST_WASM", "GUEST_WASM", "guest.wasm"),
		(
			"LEPTOSKIT_BENCH_COUNTER_WASM",
			"COUNTER_WASM",
			"counter.wasm",
		),
	] {
		println!("cargo:rerun-if-env-changed={variable}");

		match env::var(variable) {
			Ok(path) if !path.is_empty() => {
				println!("cargo:rerun-if-changed={path}");
				fs::copy(&path, out_dir.join(file)).unwrap_or_else(|error| {
					panic!("could not copy the guest wasm from {path}: {error}")
				});
				writeln!(
					generated,
					"pub const {constant}: Option<&[u8]> = Some(include_bytes!(\"{file}\"));"
				)
				.expect("write to String");
			}
			_ => {
				writeln!(generated, "pub const {constant}: Option<&[u8]> = None;")
					.expect("write to String");
			}
		}
	}

	fs::write(out_dir.join("embedded_guest.rs"), generated).expect("write embedded_guest.rs");
}
