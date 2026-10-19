//! The wasm build pipeline: cargo-leptos' job, with Dioxus-CLI speed.
//!
//! Compiling a leptos app for the web is four phases — compile the client to
//! wasm, run wasm-bindgen over it, optimize with `wasm-opt`, and lay out the
//! output — and almost every rebuild repeats work whose inputs did not change.
//! This module keeps every phase pure (run it, get its artifact), so callers
//! decide from fingerprints what to run and can overlap the compiles:
//!
//! - the server and the wasm client compile **in parallel**;
//! - wasm-bindgen runs **in-process** (no CLI subprocess) and is **skipped
//!   entirely** when the compiled wasm hash is unchanged, which removes the
//!   most expensive fixed cost of a typical rebuild;
//! - `wasm-opt` only ever runs in release, and only when its input moved.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

use anyhow::Context;
use anyhow::Result;

use crate::cache::Fingerprint;
use crate::cache::hash_bytes;

/// The wasm target every client build uses.
pub const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// Everything one wasm client build needs.
#[derive(Debug, Clone)]
pub struct WasmOptions {
	/// The client crate's package name.
	pub package: String,
	/// Whether this is a release build.
	pub release: bool,
	/// Extra cargo features.
	pub features: Vec<String>,
	/// Extra rustc flags.
	pub rustflags: Vec<String>,
	/// Whether release builds run `wasm-opt`.
	pub wasm_opt: bool,
	/// Where the processed output lands.
	pub out_dir: PathBuf,
}

impl WasmOptions {
	/// The artifact subdirectory inside `out_dir`.
	#[must_use]
	pub fn pkg_dir(&self) -> PathBuf {
		self.out_dir.join("pkg")
	}
}

/// The detected client crate: its package name and manifest path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
	/// The client crate's package name.
	pub name: String,
	/// The client crate's `Cargo.toml`, whose directory holds its `src`.
	pub manifest_path: PathBuf,
}

/// Finds the workspace's client crate: the one producing a `cdylib` target,
/// which is how leptos apps mark their wasm entry point.
///
/// `configured` (from `build.client_package`) overrides detection; explicit
/// beats inferred.
pub fn detect_client_package(root: &Path, configured: Option<&str>) -> Result<Option<String>> {
	Ok(detect_client(root, configured)?.map(|client| client.name))
}

/// The full detection result, with the manifest path the source fingerprint
/// needs.
pub fn detect_client(root: &Path, configured: Option<&str>) -> Result<Option<ClientInfo>> {
	detect_client_with(root, configured, &cargo_metadata(root)?)
}

/// The detection core over a shared `cargo metadata` result, so one run
/// answers both client detection and server manifest lookup.
pub fn detect_client_with(
	root: &Path,
	configured: Option<&str>,
	metadata: &serde_json::Value,
) -> Result<Option<ClientInfo>> {
	if let Some(name) = configured {
		return Ok(Some(ClientInfo {
			name: name.to_owned(),
			manifest_path: root.join("Cargo.toml"),
		}));
	}
	let empty_packages = Vec::new();
	let packages = metadata
		.get("packages")
		.and_then(|packages| packages.as_array())
		.unwrap_or(&empty_packages);
	for package in packages {
		let name = package
			.get("name")
			.and_then(|name| name.as_str())
			.unwrap_or_default();
		let manifest_path = package
			.get("manifest_path")
			.and_then(|path| path.as_str())
			.map(PathBuf::from);
		let empty_targets = Vec::new();
		let targets = package
			.get("targets")
			.and_then(|targets| targets.as_array())
			.unwrap_or(&empty_targets);
		let is_cdylib = targets.iter().any(|target| {
			target
				.get("kind")
				.and_then(|kind| kind.as_array())
				.is_some_and(|kind| kind.iter().any(|kind| kind.as_str() == Some("cdylib")))
		});
		if is_cdylib {
			return Ok(Some(ClientInfo {
				name: name.to_owned(),
				manifest_path: manifest_path.unwrap_or_else(|| root.join("Cargo.toml")),
			}));
		}
	}
	Ok(None)
}

/// The compile phase's fingerprint: everything that determines the wasm
/// cargo produces — manifest, sources, lockfile, flags, and tool.
///
/// The source digest is what makes the skip decision trustworthy: hashing
/// only the manifest would leave a source-only edit looking unchanged, and
/// the pipeline would serve stale output rather than skip honestly.
pub fn compile_fingerprint(
	root: &Path,
	options: &WasmOptions,
	client: Option<&ClientInfo>,
) -> Result<Fingerprint> {
	let mut fingerprint = Fingerprint::new("wasm-compile", cargo_version()?)
		.with_input("package", options.package.clone())
		.with_input("target", WASM_TARGET.to_owned())
		.with_input(
			"release",
			if options.release { "1" } else { "0" }.to_owned(),
		)
		.with_input("features", options.features.join(","))
		.with_input("rustflags", options.rustflags.join("\u{1f}"))
		.with_input(
			"lockfile",
			hash_bytes(&std::fs::read(root.join("Cargo.lock")).unwrap_or_default()),
		);
	if let Some(client) = client {
		let package_root = client.manifest_path.parent().unwrap_or(root).to_path_buf();
		fingerprint = fingerprint.with_file("manifest", &client.manifest_path);
		fingerprint = fingerprint.with_input(
			"sources",
			crate::build::directory_digest_parallel(&package_root.join("src")),
		);
	} else {
		fingerprint = fingerprint.with_file("manifest", &root.join("Cargo.toml"));
	}
	Ok(fingerprint)
}

/// The bindgen phase's fingerprint: keyed on the compiled wasm's *content*, so
/// an unchanged compile skips bindgen entirely.
pub fn bindgen_fingerprint(wasm_hash: &str, release: bool) -> Fingerprint {
	Fingerprint::new("wasm-bindgen", wasm_bindgen_version())
		.with_input("wasm", wasm_hash.to_owned())
		.with_input("release", if release { "1" } else { "0" }.to_owned())
}

/// The optimization phase's fingerprint.
pub fn optimize_fingerprint(wasm_hash: &str) -> Fingerprint {
	Fingerprint::new("wasm-opt", wasm_opt_version().unwrap_or_default())
		.with_input("wasm", wasm_hash.to_owned())
}

/// Where cargo puts the compiled wasm for a package and profile.
#[must_use]
pub fn compiled_wasm_path(root: &Path, package: &str, release: bool) -> PathBuf {
	let profile = if release { "release" } else { "debug" };
	root.join("target")
		.join(WASM_TARGET)
		.join(profile)
		.join(format!("{}.wasm", package.replace('-', "_")))
}

/// Compiles the client crate to wasm. Pure: runs cargo, returns the artifact.
pub fn compile_wasm(root: &Path, options: &WasmOptions) -> Result<PathBuf> {
	let mut command = Command::new("cargo");
	command
		.arg("build")
		.arg("--package")
		.arg(&options.package)
		.arg("--target")
		.arg(WASM_TARGET)
		.current_dir(root);
	if options.release {
		command.arg("--release");
	}
	if !options.features.is_empty() {
		command.arg("--features").arg(options.features.join(","));
	}
	if !options.rustflags.is_empty() {
		command.env("RUSTFLAGS", options.rustflags.join(" "));
	}
	let status = command
		.status()
		.context("could not run cargo for the wasm build")?;
	anyhow::ensure!(status.success(), "wasm build failed");
	let path = compiled_wasm_path(root, &options.package, options.release);
	anyhow::ensure!(
		path.is_file(),
		"cargo reported success but {} is missing; is the client crate a cdylib?",
		path.display()
	);
	Ok(path)
}

/// Runs wasm-bindgen in-process over the compiled wasm. Pure: writes the glue
/// into `out_dir`.
pub fn bindgen(wasm_path: &Path, out_dir: &Path, release: bool) -> Result<()> {
	let mut bindgen = wasm_bindgen_cli_support::Bindgen::new();
	bindgen
		.input_path(wasm_path)
		.web(true)
		.context("the web output target is unavailable")?
		.debug(!release)
		.generate(out_dir)
		.with_context(|| format!("wasm-bindgen failed on {}", wasm_path.display()))?;
	Ok(())
}

/// Optimizes the processed wasm in place with `wasm-opt`. Pure.
pub fn optimize(wasm_path: &Path) -> Result<()> {
	let status = Command::new("wasm-opt")
		.arg("-O")
		.arg("--enable-bulk-memory")
		.arg(wasm_path)
		.arg("-o")
		.arg(wasm_path)
		.status()
		.context("could not run wasm-opt; is binaryen installed?")?;
	anyhow::ensure!(status.success(), "wasm-opt failed");
	Ok(())
}

/// The SHA-256 of a file's bytes, hex encoded.
pub fn hash_file(path: &Path) -> Result<String> {
	let bytes =
		std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
	Ok(hash_bytes(&bytes))
}

/// Runs two build steps in parallel and joins their results, preserving the
/// first error if both fail.
pub fn join_parallel<A, B, RA, RB>(first: A, second: B) -> Result<(RA, RB)>
where
	A: FnOnce() -> Result<RA> + Send + 'static,
	B: FnOnce() -> Result<RB> + Send + 'static,
	RA: Send + 'static,
	RB: Send + 'static,
{
	let (first_tx, first_rx) = mpsc::channel();
	let (second_tx, second_rx) = mpsc::channel();
	let first_handle = thread::spawn(move || {
		let _ = first_tx.send(first());
	});
	let second_handle = thread::spawn(move || {
		let _ = second_tx.send(second());
	});
	let _ = first_handle.join();
	let _ = second_handle.join();
	let first_result = first_rx
		.recv()
		.expect("the first thread always sends a result");
	let second_result = second_rx
		.recv()
		.expect("the second thread always sends a result");
	Ok((first_result?, second_result?))
}

/// One `cargo metadata --no-deps` call, shared by every consumer in a build
/// so the fixed cost lands once.
pub fn cargo_metadata(root: &Path) -> Result<serde_json::Value> {
	let output = Command::new("cargo")
		.args(["metadata", "--no-deps", "--format-version", "1"])
		.current_dir(root)
		.output()
		.context("could not run cargo metadata")?;
	anyhow::ensure!(
		output.status.success(),
		"cargo metadata failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	serde_json::from_slice(&output.stdout).context("could not parse cargo metadata")
}

fn cargo_version() -> Result<String> {
	let output = Command::new("cargo")
		.arg("--version")
		.output()
		.context("could not run cargo")?;
	Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn wasm_bindgen_version() -> String {
	// `wasm-bindgen-shared` is versioned in lockstep with
	// `wasm-bindgen-cli-support` (both pin `=0.2.x` against each other), so
	// its runtime version is the in-process bindgen's identity for the phase
	// fingerprint.
	wasm_bindgen_shared::version()
}

fn wasm_opt_version() -> Option<String> {
	let output = Command::new("wasm-opt").arg("--version").output().ok()?;
	Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
