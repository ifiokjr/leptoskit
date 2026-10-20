//! The wasm build pipeline: compile the client, bind it, optionally optimize.
//!
//! Compiling a leptos app for the web is three phases — compile the client to
//! wasm, run wasm-bindgen over it, optimize with `wasm-opt` — plus laying out
//! the output. This module keeps every phase pure (run it, get its artifact),
//! so the caller decides what to run:
//!
//! - cargo **always runs**: it is incremental, a no-op build costs a fraction
//!   of a second, and it is the only thing that knows when a dependency crate,
//!   a build script, or an environment variable changed the output;
//! - wasm-bindgen runs **in-process** (no CLI subprocess) and is **skipped**
//!   when the compiled wasm's content hash, the output directory, and the flags
//!   are unchanged and the outputs are still on disk;
//! - `wasm-opt` only ever runs in release, as part of the same cached phase.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;

use crate::cache::Fingerprint;
use crate::cache::hash_bytes;

/// The wasm target every client build uses.
pub const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// The cargo inputs shared by the server and the wasm client builds.
#[derive(Debug, Clone, Default)]
pub struct CargoOptions {
	/// Whether this is a release build.
	pub release: bool,
	/// Cargo features, from `build.features`, plugins, and leptos plugins.
	pub features: Vec<String>,
	/// Extra rustc flags for the wasm target, from `build.rustflags` and
	/// plugins. They are passed as `target.wasm32-unknown-unknown.rustflags`
	/// through `--config`, so they merge with the same key in
	/// `.cargo/config.toml` and never reach host builds. Cargo then ignores a
	/// `[build] rustflags` table, so projects that set wasm flags there should
	/// move them under `[target.wasm32-unknown-unknown]`.
	pub rustflags: Vec<String>,
	/// Environment variables set for cargo, from plugins.
	pub env: BTreeMap<String, String>,
}

/// The subset of `cargo metadata --no-deps` the pipeline reads.
#[derive(Debug, Clone, Deserialize)]
pub struct CargoMetadata {
	/// The workspace members.
	pub packages: Vec<MetadataPackage>,
	/// Where cargo writes artifacts, honoring `CARGO_TARGET_DIR` and
	/// `build.target-dir`.
	pub target_directory: PathBuf,
	/// The workspace root, which holds `Cargo.lock`.
	pub workspace_root: PathBuf,
}

/// One workspace member.
#[derive(Debug, Clone, Deserialize)]
pub struct MetadataPackage {
	/// The package name.
	pub name: String,
	/// The package's `Cargo.toml`.
	pub manifest_path: PathBuf,
	/// The package's build targets.
	pub targets: Vec<MetadataTarget>,
}

/// One build target of a package.
#[derive(Debug, Clone, Deserialize)]
pub struct MetadataTarget {
	/// The target name; for a library, the crate name cargo names artifacts
	/// after (with `-` replaced by `_`).
	pub name: String,
	/// The target kinds, such as `bin`, `lib`, or `cdylib`.
	pub kind: Vec<String>,
}

impl MetadataTarget {
	fn is_cdylib(&self) -> bool {
		self.kind.iter().any(|kind| kind == "cdylib")
	}
}

impl CargoMetadata {
	/// The workspace member named `name`.
	#[must_use]
	pub fn package(&self, name: &str) -> Option<&MetadataPackage> {
		self.packages.iter().find(|package| package.name == name)
	}
}

/// The detected client crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
	/// The client crate's package name, passed to `cargo build --package`.
	pub name: String,
	/// The client crate's `Cargo.toml`.
	pub manifest_path: PathBuf,
	/// The cdylib target's crate name with `-` replaced by `_`: the stem of
	/// the compiled `.wasm` and of the wasm-bindgen outputs.
	pub lib_name: String,
}

impl ClientInfo {
	fn from_package(package: &MetadataPackage) -> Option<Self> {
		let target = package.targets.iter().find(|target| target.is_cdylib())?;
		Some(Self {
			name: package.name.clone(),
			manifest_path: package.manifest_path.clone(),
			lib_name: target.name.replace('-', "_"),
		})
	}
}

/// Finds the workspace's client crate: the one producing a `cdylib` target,
/// which is how leptos apps mark their wasm entry point.
///
/// `configured` (from `build.client_package`) overrides detection; explicit
/// beats inferred.
pub fn detect_client_package(root: &Path, configured: Option<&str>) -> Result<Option<String>> {
	Ok(detect_client(root, configured)?.map(|client| client.name))
}

/// The full detection result, running `cargo metadata` in `root`.
pub fn detect_client(root: &Path, configured: Option<&str>) -> Result<Option<ClientInfo>> {
	detect_client_with(configured, &cargo_metadata(root)?)
}

/// The detection core over a shared `cargo metadata` result.
///
/// # Errors
///
/// Returns an error when `configured` names a package that is not a
/// workspace member, or one without a `cdylib` target.
pub fn detect_client_with(
	configured: Option<&str>,
	metadata: &CargoMetadata,
) -> Result<Option<ClientInfo>> {
	if let Some(name) = configured {
		let package = metadata.package(name).with_context(|| {
			format!("build.client_package `{name}` is not a member of this cargo workspace")
		})?;
		let client = ClientInfo::from_package(package).with_context(|| {
			format!(
				"build.client_package `{name}` has no cdylib target; add `crate-type = \
				 [\"cdylib\"]` to its [lib] section"
			)
		})?;
		return Ok(Some(client));
	}
	Ok(metadata.packages.iter().find_map(ClientInfo::from_package))
}

/// The bindgen phase's fingerprint: keyed on the compiled wasm's *content*
/// plus everything else that shapes the `pkg/` output, so an unchanged
/// compile into the same place skips bindgen (and `wasm-opt`) entirely.
///
/// Features, environment, and rustflags are not inputs: they only reach the
/// output through the compiled wasm, whose hash is already here.
pub fn bindgen_fingerprint(
	wasm_hash: &str,
	release: bool,
	pkg_dir: &Path,
	wasm_opt_version: Option<&str>,
) -> Fingerprint {
	Fingerprint::new("wasm-bindgen", wasm_bindgen_shared::version())
		.with_input("wasm", wasm_hash.to_owned())
		.with_input("release", if release { "1" } else { "0" }.to_owned())
		.with_input("pkg_dir", pkg_dir.display().to_string())
		.with_input("wasm_opt", wasm_opt_version.unwrap_or("off").to_owned())
		// Bumped when the glue flags change, so caches holding output from
		// the old flags rebind: 1 — emit the default module path.
		.with_input("glue_flags", "1".to_owned())
}

/// The files wasm-bindgen writes for a client: the JS glue and the processed
/// wasm module.
#[must_use]
pub fn bindgen_outputs(pkg_dir: &Path, lib_name: &str) -> [PathBuf; 2] {
	[
		pkg_dir.join(format!("{lib_name}.js")),
		pkg_dir.join(format!("{lib_name}_bg.wasm")),
	]
}

/// Where cargo puts the compiled wasm for a client and profile.
#[must_use]
pub fn compiled_wasm_path(target_dir: &Path, lib_name: &str, release: bool) -> PathBuf {
	let profile = if release { "release" } else { "debug" };
	target_dir
		.join(WASM_TARGET)
		.join(profile)
		.join(format!("{lib_name}.wasm"))
}

/// A `cargo build --package <package>` command carrying the shared options.
#[must_use]
pub fn cargo_build_command(root: &Path, package: &str, options: &CargoOptions) -> Command {
	let mut command = Command::new("cargo");
	command
		.arg("build")
		.arg("--package")
		.arg(package)
		.current_dir(root)
		.envs(&options.env);
	if options.release {
		command.arg("--release");
	}
	if !options.features.is_empty() {
		command.arg("--features").arg(options.features.join(","));
	}
	if !options.rustflags.is_empty() {
		command
			.arg("--config")
			.arg(rustflags_config_arg(&options.rustflags));
	}
	command
}

/// The `--config` value that adds `rustflags` to the wasm target: a TOML
/// array, so flags containing spaces or quotes survive intact.
#[must_use]
pub fn rustflags_config_arg(rustflags: &[String]) -> String {
	let array = toml::Value::Array(rustflags.iter().cloned().map(toml::Value::String).collect());
	format!("target.{WASM_TARGET}.rustflags={array}")
}

/// Compiles the client crate to wasm. Pure: runs cargo, returns the artifact.
pub fn compile_wasm(
	root: &Path,
	target_dir: &Path,
	client: &ClientInfo,
	options: &CargoOptions,
) -> Result<PathBuf> {
	let mut command = cargo_build_command(root, &client.name, options);
	command.arg("--target").arg(WASM_TARGET);
	let status = command
		.status()
		.context("could not run cargo for the wasm build")?;
	anyhow::ensure!(status.success(), "wasm build failed");
	let path = compiled_wasm_path(target_dir, &client.lib_name, options.release);
	anyhow::ensure!(
		path.is_file(),
		"cargo reported success but {} is missing",
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
		// The library defaults to omitting the default module path, unlike the
		// `wasm-bindgen` CLI; without it the glue's `init()` requires the
		// caller to pass the `.wasm` URL, and a plain `await init()` — as the
		// scaffold's `index.html` writes it — fails in the browser.
		.omit_default_module_path(false)
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

/// One `cargo metadata --no-deps` call, shared by every consumer in a build
/// so the fixed cost lands once.
pub fn cargo_metadata(root: &Path) -> Result<CargoMetadata> {
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

/// The `wasm-opt --version` line, part of the bindgen fingerprint in release.
pub fn wasm_opt_version() -> Result<String> {
	let output = Command::new("wasm-opt").arg("--version").output().context(
		"could not run wasm-opt; install binaryen or set `build.wasm_opt = false` in \
		 leptoskit.toml",
	)?;
	anyhow::ensure!(output.status.success(), "`wasm-opt --version` failed");
	Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// How a project's locked wasm-bindgen relates to the in-process bindgen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindgenCompatibility {
	/// The lockfile resolves no wasm-bindgen crate.
	NotLocked,
	/// Every locked wasm-bindgen version matches the embedded schema.
	Compatible,
	/// The lockfile resolves versions the embedded bindgen cannot process.
	Mismatch {
		/// The locked versions that differ from the embedded schema.
		locked: Vec<String>,
	},
}

#[derive(Deserialize)]
struct Lockfile {
	#[serde(default)]
	package: Vec<LockedPackage>,
}

#[derive(Deserialize)]
struct LockedPackage {
	name: String,
	version: String,
}

/// Compares the `wasm-bindgen` versions in a `Cargo.lock` with the schema
/// version of the wasm-bindgen embedded in this CLI. The two must match
/// exactly: wasm-bindgen refuses wasm built against any other schema.
pub fn wasm_bindgen_compatibility(lockfile: &str) -> Result<BindgenCompatibility> {
	let lockfile: Lockfile = toml::from_str(lockfile).context("could not parse Cargo.lock")?;
	let versions: Vec<String> = lockfile
		.package
		.into_iter()
		.filter(|package| package.name == "wasm-bindgen")
		.map(|package| package.version)
		.collect();
	if versions.is_empty() {
		return Ok(BindgenCompatibility::NotLocked);
	}
	let locked: Vec<String> = versions
		.into_iter()
		.filter(|version| version != wasm_bindgen_shared::SCHEMA_VERSION)
		.collect();
	if locked.is_empty() {
		Ok(BindgenCompatibility::Compatible)
	} else {
		Ok(BindgenCompatibility::Mismatch { locked })
	}
}

/// The error message for a mismatched lockfile.
#[must_use]
pub fn mismatch_message(locked: &[String]) -> String {
	let schema = wasm_bindgen_shared::SCHEMA_VERSION;
	format!(
		"the project locks wasm-bindgen {}, but leptoskit embeds wasm-bindgen {schema} and the \
		 versions must match exactly; run `cargo update --package wasm-bindgen --precise {schema}`",
		locked.join(", ")
	)
}

/// Fails with a clear message when the lockfile at `lockfile` resolves a
/// wasm-bindgen the embedded bindgen cannot process. A missing lockfile or
/// one without wasm-bindgen passes: there is nothing to disagree with.
pub fn ensure_wasm_bindgen_compatible(lockfile: &Path) -> Result<()> {
	let contents = match std::fs::read_to_string(lockfile) {
		Ok(contents) => contents,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
		Err(error) => {
			return Err(error).with_context(|| format!("could not read {}", lockfile.display()));
		}
	};
	match wasm_bindgen_compatibility(&contents)
		.with_context(|| format!("could not check {}", lockfile.display()))?
	{
		BindgenCompatibility::NotLocked | BindgenCompatibility::Compatible => Ok(()),
		BindgenCompatibility::Mismatch { locked } => anyhow::bail!(mismatch_message(&locked)),
	}
}
