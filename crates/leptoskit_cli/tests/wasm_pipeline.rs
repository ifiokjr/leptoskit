//! Wasm pipeline tests: client detection, fingerprints, and the full
//! compile-bindgen pipeline against a real fixture workspace.

use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_cli::cache::BuildCache;
use leptoskit_cli::wasm::WasmOptions;
use leptoskit_cli::wasm::{self};

struct Scratch {
	path: PathBuf,
}

impl Scratch {
	fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_wasm_test_{tag}_{}_{}",
			std::process::id(),
			unique
		));
		std::fs::create_dir_all(&path).expect("scratch");
		Self { path }
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.path);
	}
}

/// A workspace with one cdylib client and one plain server crate.
fn fixture_workspace(dir: &Path) {
	let client = dir.join("crates").join("fixture_client");
	std::fs::create_dir_all(client.join("src")).expect("client src");
	std::fs::write(
		client.join("Cargo.toml"),
		r#"
[package]
name = "fixture_client"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wasm-bindgen = "=0.2.100"
"#,
	)
	.expect("client manifest");
	std::fs::write(
		client.join("src").join("lib.rs"),
		r#"use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn greet(name: &str) -> String {
    format!("hello, {name}!")
}
"#,
	)
	.expect("client source");

	let server = dir.join("crates").join("fixture_server");
	std::fs::create_dir_all(server.join("src")).expect("server src");
	std::fs::write(
		server.join("Cargo.toml"),
		r#"
[package]
name = "fixture_server"
version = "0.1.0"
edition = "2021"
"#,
	)
	.expect("server manifest");
	std::fs::write(
		server.join("src").join("main.rs"),
		"fn main() { println!(\"server\"); }\n",
	)
	.expect("server source");

	std::fs::write(
		dir.join("Cargo.toml"),
		r#"
[workspace]
members = ["crates/*"]
resolver = "2"
"#,
	)
	.expect("workspace manifest");
}

#[test]
fn detects_the_cdylib_client_package() {
	let scratch = Scratch::new("detect");
	fixture_workspace(&scratch.path);
	let detected = wasm::detect_client_package(&scratch.path, None).expect("detect");
	assert_eq!(detected.as_deref(), Some("fixture_client"));
}

#[test]
fn configured_client_package_beats_detection() {
	let scratch = Scratch::new("override");
	fixture_workspace(&scratch.path);
	let detected =
		wasm::detect_client_package(&scratch.path, Some("some_other_crate")).expect("detect");
	assert_eq!(detected.as_deref(), Some("some_other_crate"));
}

#[test]
fn no_cdylib_yields_none() {
	let scratch = Scratch::new("none");
	let server = scratch.path.join("crates").join("plain");
	std::fs::create_dir_all(server.join("src")).expect("src");
	std::fs::write(
		server.join("Cargo.toml"),
		r#"
[package]
name = "plain"
version = "0.1.0"
edition = "2021"
"#,
	)
	.expect("manifest");
	std::fs::write(server.join("src").join("main.rs"), "fn main() {}\n").expect("main");
	std::fs::write(
		scratch.path.join("Cargo.toml"),
		"[workspace]\nmembers = [\"crates/*\"]\n",
	)
	.expect("root");
	let detected = wasm::detect_client_package(&scratch.path, None).expect("detect");
	assert_eq!(detected, None);
}

#[test]
fn fingerprints_track_their_inputs() {
	let scratch = Scratch::new("fingerprints");
	fixture_workspace(&scratch.path);
	let options = WasmOptions {
		package: "fixture_client".to_owned(),
		release: false,
		features: vec![],
		rustflags: vec![],
		wasm_opt: false,
		out_dir: scratch.path.join("dist"),
	};
	let first = wasm::compile_fingerprint(&scratch.path, &options).expect("fingerprint");
	let repeat = wasm::compile_fingerprint(&scratch.path, &options).expect("fingerprint");
	assert_eq!(first.digest(), repeat.digest());

	let mut other = options.clone();
	other.release = true;
	let release_fp = wasm::compile_fingerprint(&scratch.path, &other).expect("fingerprint");
	assert_ne!(first.digest(), release_fp.digest());

	let bindgen_a = wasm::bindgen_fingerprint("hash-a", false);
	let bindgen_b = wasm::bindgen_fingerprint("hash-b", false);
	assert_ne!(bindgen_a.digest(), bindgen_b.digest());
}

/// The full pipeline against a real wasm compile: compile, bindgen, and the
/// cache skipping both on the second pass.
///
/// Ignored by default because it needs the `wasm32-unknown-unknown` target
/// and a registry fetch for the fixture's dependencies. Run it with
/// `cargo nextest run --package leptoskit_cli --run-ignored only --
/// wasm_pipeline`.
#[test]
#[ignore = "needs the wasm32 target and a registry fetch"]
fn full_pipeline_compiles_binds_and_caches() {
	let scratch = Scratch::new("full");
	fixture_workspace(&scratch.path);
	let options = WasmOptions {
		package: "fixture_client".to_owned(),
		release: false,
		features: vec![],
		rustflags: vec![],
		wasm_opt: false,
		out_dir: scratch.path.join("dist"),
	};

	let mut cache = BuildCache::load(&scratch.path);
	let compile_fp = wasm::compile_fingerprint(&scratch.path, &options).expect("fingerprint");
	assert!(!cache.is_current(&compile_fp));

	let wasm_path = wasm::compile_wasm(&scratch.path, &options).expect("compile wasm");
	assert!(wasm_path.is_file());
	cache.record(&compile_fp);

	let hash = wasm::hash_file(&wasm_path).expect("hash");
	let bindgen_fp = wasm::bindgen_fingerprint(&hash, false);
	let pkg = scratch.path.join("dist").join("pkg");
	std::fs::create_dir_all(&pkg).expect("pkg dir");
	wasm::bindgen(&wasm_path, &pkg, false).expect("bindgen");
	cache.record(&bindgen_fp);
	cache.save().expect("save cache");

	let glue = pkg.join("fixture_client.js");
	let module = pkg.join("fixture_client_bg.wasm");
	assert!(glue.is_file(), "missing {glue:?}");
	assert!(module.is_file(), "missing {module:?}");
	assert!(
		std::fs::read_to_string(&glue)
			.expect("glue")
			.contains("greet")
	);

	// The second pass finds both phases current: this is the rebuild path a
	// one-line change does not touch.
	let reloaded = BuildCache::load(&scratch.path);
	assert!(reloaded.is_current(&compile_fp));
	assert!(reloaded.is_current(&bindgen_fp));
}

#[test]
fn parallel_join_propagates_the_first_error() {
	let error = wasm::join_parallel(
		|| Ok::<(), anyhow::Error>(()),
		|| Err::<(), anyhow::Error>(anyhow::anyhow!("client failed")),
	)
	.expect_err("the failing side must surface");
	assert!(error.to_string().contains("client failed"));
}
