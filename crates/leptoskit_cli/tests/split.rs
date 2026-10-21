//! Code splitting tests: spotting `#[lazy]` code in a compiled client, and
//! reading which chunks load on demand.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_cli::build;
use leptoskit_cli::split;
use leptoskit_cli::wasm::ClientInfo;
use leptoskit_config::BundleConfig;

struct Scratch {
	path: PathBuf,
}

impl Scratch {
	fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_split_test_{tag}_{}_{}",
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

/// A wasm section: its id, then its length-prefixed body. Every body here is
/// shorter than 128 bytes, so each length is one LEB128 byte.
fn section(id: u8, body: &[u8]) -> Vec<u8> {
	let mut bytes = vec![id, u8::try_from(body.len()).expect("short section")];
	bytes.extend_from_slice(body);
	bytes
}

fn name(text: &str) -> Vec<u8> {
	let mut bytes = vec![u8::try_from(text.len()).expect("short name")];
	bytes.extend_from_slice(text.as_bytes());
	bytes
}

/// A module importing one `() -> ()` function from each of `modules`.
fn module_importing(modules: &[&str]) -> Vec<u8> {
	let mut wasm = b"\0asm\x01\0\0\0".to_vec();
	wasm.extend(section(1, &[1, 0x60, 0, 0]));
	if !modules.is_empty() {
		let mut imports = vec![u8::try_from(modules.len()).expect("few imports")];
		for module in modules {
			imports.extend(name(module));
			imports.extend(name("f"));
			imports.extend([0x00, 0x00]);
		}
		wasm.extend(section(2, &imports));
	}
	wasm
}

#[test]
fn lazy_code_shows_as_an_import_from_the_split_placeholder() {
	assert!(
		split::uses_lazy_loading(&module_importing(&["wbg", split::PLACEHOLDER_MODULE]))
			.expect("parse")
	);
	assert!(!split::uses_lazy_loading(&module_importing(&["wbg", "env"])).expect("parse"));
	assert!(!split::uses_lazy_loading(&module_importing(&[])).expect("parse"));
	assert!(split::uses_lazy_loading(b"not wasm").is_err());
}

#[test]
fn a_client_linked_without_relocations_cannot_be_split() {
	let scratch = Scratch::new("no-relocs");
	let compiled = scratch.path.join("app.wasm");
	let wasm = module_importing(&[split::PLACEHOLDER_MODULE]);
	std::fs::write(&compiled, &wasm).expect("wasm");
	let error = split::split(&wasm, &compiled, &scratch.path, "app")
		.unwrap_err()
		.to_string();
	assert!(error.contains("could not split"), "{error}");
	assert!(error.contains(split::EMIT_RELOCS), "{error}");
}

#[test]
fn lazy_files_come_from_the_split_manifest() {
	let scratch = Scratch::new("manifest");
	let pkg = &scratch.path;
	assert!(
		split::lazy_files(pkg).expect("no manifest").is_empty(),
		"an unsplit build has no lazy files"
	);

	std::fs::write(
		pkg.join(split::MANIFEST),
		r#"{"mixer_loader_1": ["chunk_0", "split_mixer_loader_1"], "chart_2": ["chunk_0"]}"#,
	)
	.expect("manifest");
	let lazy: Vec<PathBuf> = split::lazy_files(pkg)
		.expect("manifest")
		.into_iter()
		.collect();
	assert_eq!(
		lazy,
		[
			pkg.join("chunk_0.wasm"),
			pkg.join("split_mixer_loader_1.wasm")
		]
	);

	std::fs::write(pkg.join(split::MANIFEST), "not json").expect("manifest");
	assert!(split::lazy_files(pkg).is_err());

	std::fs::remove_file(pkg.join(split::MANIFEST)).expect("remove");
	std::fs::create_dir(pkg.join(split::MANIFEST)).expect("directory");
	let error = split::lazy_files(pkg).unwrap_err().to_string();
	assert!(error.contains("could not read"), "{error}");
}

#[test]
fn environment_rustflags_must_carry_the_relocations_flag() {
	let unset = |_: &str| None;
	split::ensure_relocs_reach_cargo(&unset).expect("cargo receives leptoskit's flags");

	let encoded = |name: &str| (name == "CARGO_ENCODED_RUSTFLAGS").then(|| "-Dwarnings".into());
	let error = split::ensure_relocs_reach_cargo(&encoded)
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("while CARGO_ENCODED_RUSTFLAGS is set"),
		"{error}"
	);
	assert!(error.contains(split::EMIT_RELOCS), "{error}");

	let plain = |name: &str| (name == "RUSTFLAGS").then(|| "-Dwarnings".into());
	let error = split::ensure_relocs_reach_cargo(&plain)
		.unwrap_err()
		.to_string();
	assert!(error.contains("while RUSTFLAGS is set"), "{error}");

	// The encoded variable wins, as it does in cargo, so a plain RUSTFLAGS
	// without the flag is never read once the encoded one carries it.
	let both = |name: &str| {
		match name {
			"CARGO_ENCODED_RUSTFLAGS" => {
				Some(format!("-Dwarnings\x1f{}", split::EMIT_RELOCS).into())
			}
			_ => Some("-Dwarnings".into()),
		}
	};
	split::ensure_relocs_reach_cargo(&both).expect("the encoded flags carry relocations");
}

/// A client whose crate is `app`, compiled to `<dir>/app.wasm`.
fn compiled_client(dir: &Path, wasm: &[u8]) -> (PathBuf, ClientInfo) {
	let compiled = dir.join("app.wasm");
	std::fs::write(&compiled, wasm).expect("wasm");
	let client = ClientInfo {
		name: "app".to_owned(),
		manifest_path: dir.join("Cargo.toml"),
		lib_name: "app".to_owned(),
	};
	(compiled, client)
}

#[test]
fn binding_writes_a_fresh_pkg_and_skips_while_it_is_current() {
	let scratch = Scratch::new("bind");
	let (compiled, client) = compiled_client(&scratch.path, &module_importing(&[]));
	let pkg = scratch.path.join("dist/pkg");
	let target = scratch.path.join("target");
	let bundle = BundleConfig::default();
	std::fs::create_dir_all(&pkg).expect("pkg");
	std::fs::write(pkg.join("stale_chunk.wasm"), b"old").expect("stale");

	let (bound, module) =
		build::bind_client(&compiled, &pkg, &client, false, &bundle, &target).expect("bind");
	assert!(bound, "a first build binds");
	assert_eq!(module, pkg.join("app_bg.wasm"));
	assert!(module.is_file());
	assert!(pkg.join("app.js").is_file());
	assert!(
		!pkg.join("stale_chunk.wasm").exists(),
		"binding starts from an empty pkg/"
	);

	let (bound, _) =
		build::bind_client(&compiled, &pkg, &client, false, &bundle, &target).expect("cached");
	assert!(!bound, "unchanged input and outputs skip binding");

	std::fs::remove_file(pkg.join("app.js")).expect("remove glue");
	let (bound, _) =
		build::bind_client(&compiled, &pkg, &client, false, &bundle, &target).expect("rebind");
	assert!(bound, "a missing output rebinds");

	// Release strips the name and producers sections, a different output, so
	// it never reuses the dev bind.
	let release = BundleConfig {
		wasm_opt: false,
		..BundleConfig::default()
	};
	let (bound, _) =
		build::bind_client(&compiled, &pkg, &client, true, &release, &target).expect("release");
	assert!(bound, "a release build rebinds a dev bind");
}

#[test]
fn turning_splitting_on_never_reuses_an_unsplit_bind() {
	let scratch = Scratch::new("bind-split");
	let (compiled, client) = compiled_client(&scratch.path, &module_importing(&[]));
	let pkg = scratch.path.join("pkg");
	let target = scratch.path.join("target");
	build::bind_client(
		&compiled,
		&pkg,
		&client,
		false,
		&BundleConfig::default(),
		&target,
	)
	.expect("bind");

	// `split` is part of the fingerprint, so the same input splits afresh even
	// with a manifest and its chunks on disk, which this module, linked
	// without relocations, cannot.
	std::fs::write(pkg.join(split::MANIFEST), r#"{"heavy": ["chunk_0"]}"#).expect("manifest");
	std::fs::write(pkg.join("chunk_0.wasm"), b"chunk").expect("chunk");
	let split_bundle = BundleConfig {
		split: true,
		..BundleConfig::default()
	};
	let error = build::bind_client(&compiled, &pkg, &client, false, &split_bundle, &target)
		.unwrap_err()
		.to_string();
	assert!(error.contains("could not split"), "{error}");
}

#[test]
fn a_pkg_path_that_is_not_a_directory_is_reported() {
	let scratch = Scratch::new("bind-pkg");
	let (compiled, client) = compiled_client(&scratch.path, &module_importing(&[]));
	let target = scratch.path.join("target");
	let bundle = BundleConfig::default();

	let file = scratch.path.join("pkg");
	std::fs::write(&file, b"not a directory").expect("file");
	let error = build::bind_client(&compiled, &file, &client, false, &bundle, &target)
		.unwrap_err()
		.to_string();
	assert!(error.contains("could not clear"), "{error}");

	let nested = file.join("pkg");
	let error = build::bind_client(&compiled, &nested, &client, false, &bundle, &target)
		.unwrap_err()
		.to_string();
	assert!(error.contains("could not create"), "{error}");
}

#[test]
fn unsplit_lazy_code_is_refused_before_binding() {
	let scratch = Scratch::new("bind-lazy");
	let (compiled, client) = compiled_client(
		&scratch.path,
		&module_importing(&[split::PLACEHOLDER_MODULE]),
	);
	let pkg = scratch.path.join("pkg");
	let error = build::bind_client(
		&compiled,
		&pkg,
		&client,
		false,
		&BundleConfig::default(),
		&scratch.path.join("target"),
	)
	.unwrap_err()
	.to_string();
	assert!(error.contains("`app` has #[lazy] code"), "{error}");
	assert!(error.contains("split = true"), "{error}");
	assert!(!pkg.exists(), "nothing is written for a refused client");

	let missing = build::bind_client(
		&scratch.path.join("missing.wasm"),
		&pkg,
		&client,
		false,
		&BundleConfig::default(),
		&scratch.path.join("target"),
	)
	.unwrap_err()
	.to_string();
	assert!(missing.contains("could not read"), "{missing}");
}

/// Runs the built CLI in `dir`, without a coverage run's RUSTFLAGS, which
/// cannot build for wasm.
fn leptoskit(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(args)
		.current_dir(dir)
		.env_remove("RUSTFLAGS")
		.output()
		.expect("run leptoskit")
}

/// Runs the built CLI with an environment rustflags variable set.
fn flagged(dir: &Path, variable: &str, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(args)
		.current_dir(dir)
		.env_remove("RUSTFLAGS")
		.env(variable, "-Dwarnings")
		.output()
		.expect("run leptoskit")
}

/// A client with one split point: `heavy` moves into a chunk of its own,
/// which `run` loads the first time it is called.
fn lazy_app(dir: &Path, split: bool) {
	std::fs::write(
		dir.join(leptoskit_config::CONFIG_FILE),
		format!("[app]\nname = \"lazy\"\n\n[bundle]\nsplit = {split}\n"),
	)
	.expect("config");
	std::fs::write(
		dir.join("Cargo.toml"),
		r#"[package]
name = "lazy"
version = "0.1.0"
edition = "2024"

[workspace]

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
wasm-bindgen = "=0.2.129"
wasm-bindgen-futures = "0.4"
wasm_split_helpers = "0.2"
"#,
	)
	.expect("manifest");
	std::fs::create_dir_all(dir.join("src")).expect("src");
	std::fs::write(
		dir.join("src/lib.rs"),
		"use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_split_helpers::wasm_split(heavy)]
fn heavy(input: u32) -> u32 {
	(0..input).fold(7_u32, |hash, n| hash.wrapping_mul(31).wrapping_add(n))
}

#[wasm_bindgen]
pub async fn run(input: u32) -> u32 {
	heavy(input).await
}
",
	)
	.expect("lib");
	std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").expect("main");
}

/// Run with `cargo nextest run --package leptoskit_cli --run-ignored only`;
/// release builds run `wasm-opt` over the chunks, so binaryen must be on the
/// `PATH`.
#[test]
#[ignore = "needs the wasm32 target, binaryen, and a registry fetch"]
fn split_builds_chunks_and_unsplit_lazy_code_is_refused() {
	let scratch = Scratch::new("e2e");
	let project = &scratch.path;
	lazy_app(project, true);

	let build = leptoskit(project, &["build", "--release"]);
	let out = String::from_utf8_lossy(&build.stdout);
	assert!(
		build.status.success(),
		"split build failed:\n{out}\n{}",
		String::from_utf8_lossy(&build.stderr)
	);
	let pkg = project.join("dist/pkg");
	assert!(pkg.join(split::LINK_MODULE).is_file(), "the chunk loader");
	let lazy = split::lazy_files(&pkg).expect("manifest");
	assert_eq!(lazy.len(), 1, "one chunk for one split point: {lazy:?}");
	let chunk = lazy.iter().next().expect("chunk");
	assert!(chunk.is_file(), "{chunk:?} is written");
	let chunk_name = chunk.file_name().expect("name").to_string_lossy();
	assert!(out.contains(&format!("pkg/{chunk_name} (lazy)")), "{out}");

	let again = leptoskit(project, &["build", "--release"]);
	assert!(again.status.success());
	assert!(
		String::from_utf8_lossy(&again.stdout).contains("skipped wasm-bindgen"),
		"an unchanged split client is not split again"
	);

	// A chunk that went missing is a reason to bind again, not a cache hit.
	std::fs::remove_file(chunk).expect("remove chunk");
	let rebound = leptoskit(project, &["build", "--release"]);
	assert!(
		String::from_utf8_lossy(&rebound.stdout).contains("wasm client bound"),
		"a missing chunk is rebuilt"
	);
	assert!(chunk.is_file());

	// Environment rustflags hide leptoskit's --emit-relocs from cargo, so
	// wasm-split would have nothing to split: refused up front, but only for
	// builds that compile the client.
	for variable in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"] {
		let refused = flagged(project, variable, &["build"]);
		assert!(!refused.status.success(), "{variable}");
		let message = String::from_utf8_lossy(&refused.stderr);
		assert!(message.contains(&format!("unset {variable}")), "{message}");
	}
	let server = flagged(project, "RUSTFLAGS", &["build", "--platform", "server"]);
	assert!(
		server.status.success(),
		"a server-only build splits nothing: {}",
		String::from_utf8_lossy(&server.stderr)
	);

	// The same client unsplit would import a loader that does not exist.
	lazy_app(project, false);
	let refused = leptoskit(project, &["build"]);
	assert!(
		!refused.status.success(),
		"unsplit lazy code must not build"
	);
	let message = String::from_utf8_lossy(&refused.stderr);
	assert!(
		message.contains("set `split = true` under [bundle]"),
		"{message}"
	);
}
