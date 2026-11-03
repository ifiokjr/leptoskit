//! Wasm pipeline tests: client detection, fingerprints, and the full
//! compile-bindgen pipeline against the `tests/fixtures/wasm_workspace`
//! fixture workspace.

use std::path::Path;

use leptoskit_cli::cache::BuildCache;
use leptoskit_cli::wasm::BindgenCompatibility;
use leptoskit_cli::wasm::CargoOptions;
use leptoskit_cli::wasm::{self};

pub mod support;

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
fn output_directories_sharing_a_target_dir_keep_their_own_cache_entries() {
	let scratch = Scratch::new("shared_cache");
	let first = wasm::bindgen_fingerprint("hash-a", true, Path::new("/one/dist/pkg"), None, false);
	let second = wasm::bindgen_fingerprint("hash-b", true, Path::new("/two/dist/pkg"), None, false);
	let mut cache = BuildCache::load(&scratch.path);
	cache.record(&first);
	cache.record(&second);
	cache.save().expect("save");
	let reloaded = BuildCache::load(&scratch.path);

	assert!(
		reloaded.is_current(&first),
		"the second build must not evict the first"
	);
	assert!(reloaded.is_current(&second));
}

#[test]
fn bindgen_fingerprint_tracks_the_output_location_and_flags() {
	let pkg = Path::new("/project/dist/pkg");
	let base = wasm::bindgen_fingerprint("hash-a", false, pkg, None, false);
	assert_eq!(
		base.digest(),
		wasm::bindgen_fingerprint("hash-a", false, pkg, None, false).digest()
	);
	for other in [
		wasm::bindgen_fingerprint("hash-b", false, pkg, None, false),
		wasm::bindgen_fingerprint("hash-a", true, pkg, None, false),
		wasm::bindgen_fingerprint(
			"hash-a",
			false,
			Path::new("/project/dist2/pkg"),
			None,
			false,
		),
		wasm::bindgen_fingerprint("hash-a", false, pkg, Some("wasm-opt version 123"), false),
		wasm::bindgen_fingerprint("hash-a", false, pkg, None, true),
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
fn lockfile_wasm_bindgen_must_share_the_embedded_schema() {
	let lock = |version: &str| {
		format!(
			"version = 4\n\n[[package]]\nname = \"wasm-bindgen\"\nversion = \
			 \"{version}\"\n\n[[package]]\nname = \"other\"\nversion = \"9.9.9\"\n"
		)
	};
	let embedded = wasm::embedded_wasm_bindgen();
	for version in [
		wasm_bindgen_shared::SCHEMA_VERSION.to_owned(),
		embedded.to_string(),
	] {
		assert_eq!(
			wasm::wasm_bindgen_compatibility(&lock(&version)).expect("lock"),
			BindgenCompatibility::Compatible,
			"{version}"
		);
	}
	let newer = semver::Version::new(embedded.major, embedded.minor, embedded.patch + 1);
	assert_eq!(
		wasm::wasm_bindgen_compatibility(&lock(&newer.to_string())).expect("lock"),
		BindgenCompatibility::Mismatch {
			locked: vec![newer.to_string()]
		},
		"a newer wasm-bindgen may have changed the schema"
	);
	assert_eq!(
		wasm::wasm_bindgen_compatibility(&lock("not-a-version")).expect("lock"),
		BindgenCompatibility::Mismatch {
			locked: vec!["not-a-version".to_owned()]
		},
		"an unreadable version cannot be shown to share the schema"
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
	let bindgen_fp = wasm::bindgen_fingerprint(&hash, false, &pkg, None, false);
	assert!(!cache.is_current(&bindgen_fp));
	std::fs::create_dir_all(&pkg).expect("pkg dir");
	wasm::bindgen(&wasm_path, &pkg, &client.lib_name, false, false).expect("bindgen");

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
	assert!(reloaded.is_current(&wasm::bindgen_fingerprint(
		&rehash, false, &pkg, None, false
	)));
}

/// Packaging must use the real compiler and bindgen outputs and leave the
/// source manifest, rather than secrets, in Firefox's review archive.
#[test]
#[ignore = "builds for wasm32 and fetches dependencies; run by the `coverage` CI job"]
fn extension_build_packages_each_browser_and_replaces_stale_outputs() {
	let scratch = fixture_project("wasm_workspace");
	let manifest = scratch.path.join("crates/fixture_client/Cargo.toml");
	let source = std::fs::read_to_string(&manifest).unwrap();
	std::fs::write(manifest, source + "\n[features]\nextension = []\n").unwrap();
	std::fs::write(
		scratch.path.join("leptoskit.toml"),
		r#"
[app]
name = "extension_fixture"
version = "1.2.3"
platforms = ["extension"]
default_platform = "extension"
render = "csr"
[build]
client_package = "fixture_client"
[extension]
name = "Fixture extension"
browsers = ["chromium", "firefox", "safari"]
[extension.popup]
route = "/popup"
[extension.firefox]
id = "fixture@example.test"
[publish.chrome]
publisher_id = "fixture"
item_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
[publish.edge]
product_id = "00000000-0000-0000-0000-000000000000"
[publish.firefox]
channel = "unlisted"
"#,
	)
	.unwrap();
	std::fs::create_dir_all(scratch.path.join("assets/nested")).unwrap();
	std::fs::write(scratch.path.join("assets/app.css"), "body { color: blue; }").unwrap();
	std::fs::write(
		scratch.path.join("assets/nested/data.txt"),
		"packaged asset",
	)
	.unwrap();
	std::fs::write(scratch.path.join("monosecret.toml"), "[secrets]\n").unwrap();
	std::fs::write(scratch.path.join(".env"), "FIXTURE_SECRET=private\n").unwrap();
	let build = || {
		let output = std::process::Command::new(env!("CARGO_BIN_EXE_leptoskit"))
			.current_dir(&scratch.path)
			.args(["build", "--platform", "extension"])
			.output()
			.unwrap();
		assert!(
			output.status.success(),
			"{}\n{}",
			String::from_utf8_lossy(&output.stdout),
			String::from_utf8_lossy(&output.stderr)
		);
		String::from_utf8(output.stdout).unwrap()
	};
	let output = build();
	assert!(output.contains("firefox-source.zip"), "{output}");
	let directory = scratch.path.join(".leptoskit/extension");
	for browser in ["chromium", "firefox", "safari"] {
		let package = directory.join(browser);
		let manifest: serde_json::Value =
			serde_json::from_slice(&std::fs::read(package.join("manifest.json")).unwrap()).unwrap();
		assert_eq!(manifest["name"], "Fixture extension");
		assert_eq!(manifest["version"], "1.2.3");
		for file in [
			"popup.html",
			"popup.js",
			"pkg/fixture_client.js",
			"pkg/fixture_client_bg.wasm",
			"assets/nested/data.txt",
		] {
			assert!(package.join(file).is_file(), "{browser}: {file}");
		}
		assert_eq!(
			directory.join(format!("{browser}.zip")).is_file(),
			browser != "safari"
		);
	}
	assert!(directory.join("firefox-source.zip").is_file());
	std::fs::write(directory.join("chromium/stale.txt"), "old output").unwrap();
	build();
	assert!(!directory.join("chromium/stale.txt").exists());
	let output = std::process::Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.current_dir(&scratch.path)
		.args(["publish", "--dry-run", "--json"])
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let report: serde_json::Value = serde_json::from_slice(
		&std::fs::read(scratch.path.join(".leptoskit/publish/next-steps.json")).unwrap(),
	)
	.unwrap();
	let stores = report["stores"].as_array().unwrap();
	assert_eq!(stores.len(), 3);
	for store in stores {
		assert!(
			store["summary"]
				.as_str()
				.unwrap()
				.contains("Fixture extension 1.2.3 is built and checked")
		);
	}
}

/// Exercise the generated guest through the same interpreter the ArkUI host
/// embeds.
#[test]
#[ignore = "builds for wasm32 and fetches dependencies; run by the `coverage` CI job"]
fn harmonyos_counter_compiles_and_handles_native_presses() {
	let scratch = Scratch::new("harmonyos-counter-runtime");
	let app = scratch.path.join("counter");
	let sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
		.parent()
		.unwrap()
		.parent()
		.unwrap();
	leptoskit_cli::native::scaffold::run_for_target(
		app.to_str().unwrap(),
		None,
		Some(sdk.to_str().unwrap()),
		Some(leptoskit_config::NativeTarget::Harmonyos),
	)
	.unwrap();
	let output = std::process::Command::new("cargo")
		.current_dir(&app)
		.args([
			"build",
			"--package",
			"counter_guest",
			"--target",
			"wasm32-unknown-unknown",
		])
		.arg("--target-dir")
		.arg(app.join("target"))
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let bytes =
		std::fs::read(app.join("target/wasm32-unknown-unknown/debug/counter_guest.wasm")).unwrap();

	let mut host = leptoskit_wasm::WasmHost::headless(&bytes).unwrap();
	host.start(390.0, 844.0).unwrap();
	let first = serde_json::to_value(host.take_mutations()).unwrap();
	assert!(first.to_string().contains("Count: 0"));

	for count in 1..=3 {
		host.dispatch_wire(3, leptoskit_wasm::EventKind::Press, None)
			.unwrap();
		let mutations = serde_json::to_value(host.take_mutations()).unwrap();
		assert!(mutations.to_string().contains(&format!("Count: {count}")));
	}

	host.set_viewport(844.0, 390.0);
	host.dispatch_wire(3, leptoskit_wasm::EventKind::Press, None)
		.unwrap();
	assert!(
		serde_json::to_value(host.take_mutations())
			.unwrap()
			.to_string()
			.contains("Count: 4")
	);
	let mut restarted = leptoskit_wasm::WasmHost::headless(&bytes).unwrap();
	restarted.start(390.0, 844.0).unwrap();
	assert!(
		serde_json::to_value(restarted.take_mutations())
			.unwrap()
			.to_string()
			.contains("Count: 0")
	);
}
