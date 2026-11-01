//! Embeds the SHA-256 of the `DevTools` interface archive released with this
//! version of the CLI, so `leptoskit devtools fetch` downloads exactly that
//! archive and nothing else.
//!
//! The release workflow passes the hash as `LEPTOSKIT_DEVTOOLS_UI_SHA256` to
//! the binary builds, and writes it to `devtools-ui.sha256` beside this file
//! before the crate is packaged, so `cargo install` embeds it too. A CLI
//! built from a checkout has neither, and serves a local build of the
//! interface instead (`LEPTOSKIT_DEVTOOLS_UI`).

use std::path::Path;

const VARIABLE: &str = "LEPTOSKIT_DEVTOOLS_UI_SHA256";
const FILE: &str = "devtools-ui.sha256";

fn main() {
	println!("cargo::rerun-if-env-changed={VARIABLE}");
	println!("cargo::rerun-if-changed={FILE}");

	let from_env = std::env::var(VARIABLE)
		.ok()
		.map(|hash| hash.trim().to_owned())
		.filter(|hash| !hash.is_empty());
	let from_file = || {
		let manifest = std::env::var("CARGO_MANIFEST_DIR").ok()?;
		let contents = std::fs::read_to_string(Path::new(&manifest).join(FILE)).ok()?;
		let hash = contents.split_whitespace().next()?.to_owned();
		Some(hash)
	};

	let Some(hash) = from_env.or_else(from_file) else {
		return;
	};
	let hash = hash.to_ascii_lowercase();
	assert!(
		hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
		"{VARIABLE} (or {FILE}) must be a SHA-256 in hex, not {hash:?}"
	);
	println!("cargo::rustc-env=LEPTOSKIT_DEVTOOLS_UI_HASH={hash}");
}
