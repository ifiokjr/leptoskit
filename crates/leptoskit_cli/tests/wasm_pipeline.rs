//! Wasm pipeline tests: client detection, fingerprints, and the full
//! compile-bindgen pipeline against the `tests/fixtures/wasm_workspace`
//! fixture workspace.

use std::path::Path;

use leptoskit_cli::cache::BuildCache;
use leptoskit_cli::wasm::BindgenCompatibility;
use leptoskit_cli::wasm::CargoOptions;
use leptoskit_cli::wasm::{self};

mod support;

use support::Scratch;
use support::fixture_project;

#[test]
fn detects_the_cdylib_client_package() {
	let scratch = fixture_project("wasm_workspace");
	let detected = wasm::detect_client_package(&scratch.path, None).expect("detect");
	assert_eq!(detected.as_deref(), Some("fixture_client"));
}

#[test]
fn configured_client_package_resolves_its_own_manifest() {
	let scratch = fixture_project("wasm_workspace");
	let client = wasm::detect_client(&scratch.path, Some("fixture_client"))
		.expect("detect")
		.expect("configured client");
	// Compare canonical forms: on Windows the scratch path carries the `\\?\`
	// verbatim prefix from `canonicalize`, while cargo metadata reports the
	// plain path to the same file.
	assert_eq!(
		std::fs::canonicalize(&client.manifest_path).expect("canonical client manifest"),
		scratch.path.join("crates/fixture_client/Cargo.toml"),
		"the configured package's own manifest, not the workspace root's"
	);
	assert_eq!(client.lib_name, "fixture_client");
}

#[test]
fn configured_client_package_must_exist() {
	let scratch = fixture_project("wasm_workspace");
	let error = wasm::detect_client_package(&scratch.path, Some("some_other_crate"))
		.expect_err("an unknown configured package is an error");
	assert!(error.to_string().contains("not a member"), "got {error}");
	let error = wasm::detect_client_package(&scratch.path, Some("fixture_server"))
		.expect_err("a configured package without a cdylib is an error");
	assert!(
		error.to_string().contains("no cdylib target"),
		"got {error}"
	);
}

#[test]
fn client_lib_name_follows_the_cdylib_target() {
	let scratch = Scratch::new("lib-name");
	let client = scratch.path.join("web-client");
	std::fs::create_dir_all(client.join("src")).expect("src");
	std::fs::write(
		client.join("Cargo.toml"),
		r#"
[package]
name = "web-client"
version = "0.1.0"
edition = "2021"

[lib]
name = "renamed_client"
crate-type = ["cdylib"]
"#,
	)
	.expect("manifest");
	std::fs::write(client.join("src").join("lib.rs"), "").expect("lib");
	std::fs::write(
		scratch.path.join("Cargo.toml"),
		"[workspace]\nmembers = [\"web-client\"]\nresolver = \"2\"\n",
	)
	.expect("root");
	let metadata = wasm::cargo_metadata(&scratch.path).expect("metadata");
	let client = wasm::detect_client_with(None, None, &metadata)
		.expect("detect")
		.expect("client");
	assert_eq!(client.name, "web-client");
	assert_eq!(client.lib_name, "renamed_client");
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
fn bindgen_fingerprint_tracks_the_output_location_and_flags() {
	let pkg = Path::new("/project/dist/pkg");
	let base = wasm::bindgen_fingerprint("hash-a", false, pkg, None);
	assert_eq!(
		base.digest(),
		wasm::bindgen_fingerprint("hash-a", false, pkg, None).digest()
	);
	for other in [
		wasm::bindgen_fingerprint("hash-b", false, pkg, None),
		wasm::bindgen_fingerprint("hash-a", true, pkg, None),
		wasm::bindgen_fingerprint("hash-a", false, Path::new("/project/dist2/pkg"), None),
		wasm::bindgen_fingerprint("hash-a", false, pkg, Some("wasm-opt version 123")),
	] {
		assert_ne!(base.digest(), other.digest(), "{other:?}");
	}
}

#[test]
fn rustflags_become_a_toml_array_for_the_wasm_target() {
	let flags = vec![
		"--cfg".to_owned(),
		r#"getrandom_backend="wasm_js""#.to_owned(),
		"-Clink-arg=--export-table with spaces".to_owned(),
	];
	let argument = wasm::rustflags_config_arg(&flags);
	let parsed: toml::Table = toml::from_str(&argument).expect("valid TOML");
	let parsed_flags: Vec<&str> = parsed["target"]["wasm32-unknown-unknown"]["rustflags"]
		.as_array()
		.expect("array")
		.iter()
		.map(|flag| flag.as_str().expect("string"))
		.collect();
	assert_eq!(parsed_flags, flags);
}

#[test]
fn lockfile_wasm_bindgen_must_match_the_embedded_schema() {
	let lock = |version: &str| {
		format!(
			"version = 4\n\n[[package]]\nname = \"wasm-bindgen\"\nversion = \
			 \"{version}\"\n\n[[package]]\nname = \"other\"\nversion = \"9.9.9\"\n"
		)
	};
	assert_eq!(
		wasm::wasm_bindgen_compatibility(&lock(wasm_bindgen_shared::SCHEMA_VERSION)).expect("lock"),
		BindgenCompatibility::Compatible
	);
	assert_eq!(
		wasm::wasm_bindgen_compatibility(&lock("0.2.1")).expect("lock"),
		BindgenCompatibility::Mismatch {
			locked: vec!["0.2.1".to_owned()]
		}
	);
	assert_eq!(
		wasm::wasm_bindgen_compatibility("version = 4\n").expect("lock"),
		BindgenCompatibility::NotLocked
	);

	let scratch = Scratch::new("lock");
	let lockfile = scratch.path.join("Cargo.lock");
	wasm::ensure_wasm_bindgen_compatible(&lockfile).expect("a missing lockfile passes");
	std::fs::write(&lockfile, lock("0.2.1")).expect("write lock");
	let error = wasm::ensure_wasm_bindgen_compatible(&lockfile).expect_err("mismatch");
	assert!(
		error
			.to_string()
			.contains("cargo update --package wasm-bindgen --precise"),
		"got {error}"
	);
}

/// The full pipeline against a real wasm compile: compile, bindgen, and the
/// cache finding the bindgen phase current on the second pass.
///
/// Ignored by default because it compiles for `wasm32-unknown-unknown` and
/// fetches the fixture's dependencies; the `coverage` CI job runs it,
/// instrumented.
#[test]
#[ignore = "builds for wasm32 and fetches dependencies; run by the `coverage` CI job"]
fn full_pipeline_compiles_binds_and_caches() {
	let scratch = fixture_project("wasm_workspace");
	let options = CargoOptions::default();
	let metadata = wasm::cargo_metadata(&scratch.path).expect("metadata");
	let client = wasm::detect_client_with(None, None, &metadata)
		.expect("detect")
		.expect("the fixture has a cdylib client");

	let wasm_path = wasm::compile_wasm(&scratch.path, &client, &options).expect("compile wasm");
	assert!(wasm_path.is_file());
	wasm::ensure_wasm_bindgen_compatible(&metadata.workspace_root.join("Cargo.lock"))
		.expect("the fixture pins the embedded wasm-bindgen");

	let cache_dir = scratch.path.join("cache");
	let mut cache = BuildCache::load(&cache_dir);
	let pkg = scratch.path.join("dist").join("pkg");
	let hash = wasm::hash_file(&wasm_path).expect("hash");
	let bindgen_fp = wasm::bindgen_fingerprint(&hash, false, &pkg, None);
	assert!(!cache.is_current(&bindgen_fp));
	std::fs::create_dir_all(&pkg).expect("pkg dir");
	wasm::bindgen(&wasm_path, &pkg, false).expect("bindgen");
	cache.record(&bindgen_fp);
	cache.save().expect("save cache");

	let [glue, module] = wasm::bindgen_outputs(&pkg, &client.lib_name);
	assert!(glue.is_file(), "missing {glue:?}");
	assert!(module.is_file(), "missing {module:?}");
	let glue_source = std::fs::read_to_string(&glue).expect("glue");
	assert!(glue_source.contains("greet"));
	// `await init()` with no arguments must resolve the module next to the
	// glue, as the scaffold's `index.html` relies on.
	assert!(
		glue_source.contains("import.meta.url"),
		"the glue must carry the default module path so `init()` works without arguments"
	);

	// A no-op recompile produces the same wasm, so the bindgen phase is
	// current on the second pass.
	let recompiled = wasm::compile_wasm(&scratch.path, &client, &options).expect("recompile wasm");
	let rehash = wasm::hash_file(&recompiled).expect("hash");
	let reloaded = BuildCache::load(&cache_dir);
	assert!(reloaded.is_current(&wasm::bindgen_fingerprint(&rehash, false, &pkg, None)));
}
