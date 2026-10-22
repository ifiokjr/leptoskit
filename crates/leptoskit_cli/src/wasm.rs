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
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;

use crate::cache::Fingerprint;
use crate::cache::hash_bytes;

/// The wasm target every client build uses.
pub const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// The cargo profile `bundle.optimize = "smallest"` compiles release clients
/// with: defined by leptoskit, through [`smallest_profile_config`], so no
/// manifest needs editing.
pub const SMALLEST_PROFILE: &str = "leptoskit-smallest";

/// The `--config` entries that define [`SMALLEST_PROFILE`]: `release`, tuned
/// for the smallest wasm. Fat LTO and one codegen unit let the optimizer see
/// (and drop) the whole program, which is where nearly all of the saving
/// comes from and why rebuilds are slower; `opt-level = "z"` optimizes for
/// size, and `panic = "abort"` leaves out unwinding.
#[must_use]
pub fn smallest_profile_config() -> Vec<String> {
	[
		"inherits = \"release\"",
		"opt-level = \"z\"",
		"lto = true",
		"codegen-units = 1",
		"panic = \"abort\"",
	]
	.into_iter()
	.map(|setting| format!("profile.{SMALLEST_PROFILE}.{setting}"))
	.collect()
}

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
	/// The profile a release build compiles with instead of `release`.
	pub profile: Option<String>,
	/// Extra `--config` values, such as the definition of `profile`.
	pub config: Vec<String>,
}

impl CargoOptions {
	/// These options compiling release builds with `profile` instead of
	/// `release`, defined by the `config` entries (empty for a profile the
	/// manifest defines).
	#[must_use]
	pub fn with_profile(&self, profile: &str, config: Vec<String>) -> Self {
		let mut options = self.clone();
		options.profile = Some(profile.to_owned());
		options.config.extend(config);
		options
	}

	/// These options with `features` added after the shared ones, skipping
	/// any already present.
	#[must_use]
	pub fn with_features(&self, features: &[String]) -> Self {
		let mut options = self.clone();
		for feature in features {
			if !options.features.contains(feature) {
				options.features.push(feature.clone());
			}
		}
		options
	}
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
	/// The features the package declares, by name.
	#[serde(default)]
	pub features: BTreeMap<String, Vec<String>>,
	/// The package's direct dependencies.
	#[serde(default)]
	pub dependencies: Vec<MetadataDependency>,
}

impl MetadataPackage {
	/// Whether the package declares the cargo feature `name`.
	#[must_use]
	pub fn declares_feature(&self, name: &str) -> bool {
		self.features.contains_key(name)
	}

	/// The name the package uses for its direct dependency on the crate
	/// `name`: the `package = ...` rename when there is one. This is the
	/// prefix for that dependency's features in `--features dep/feature`.
	#[must_use]
	pub fn dependency_key(&self, name: &str) -> Option<&str> {
		self.dependencies
			.iter()
			.find(|dependency| dependency.name == name)
			.map(|dependency| dependency.rename.as_deref().unwrap_or(&dependency.name))
	}
}

/// One direct dependency of a package.
#[derive(Debug, Clone, Deserialize)]
pub struct MetadataDependency {
	/// The depended-on crate's package name.
	pub name: String,
	/// The local name, when the dependency is renamed.
	pub rename: Option<String>,
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
	detect_client_with(configured, None, &cargo_metadata(root)?)
}

/// The detection core over a shared `cargo metadata` result.
///
/// Without a configured package, the app package itself wins when it has a
/// `cdylib` target (the single-crate layout, where one package is both the
/// server binary and the wasm client); otherwise the first workspace member
/// with one does.
///
/// # Errors
///
/// Returns an error when `configured` names a package that is not a
/// workspace member, or one without a `cdylib` target.
pub fn detect_client_with(
	configured: Option<&str>,
	app: Option<&str>,
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
	if let Some(client) = app
		.and_then(|app| metadata.package(app))
		.and_then(ClientInfo::from_package)
	{
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
	split: bool,
) -> Fingerprint {
	// The cache keeps one entry per phase, and the target directory holding
	// it can be shared by projects or output directories (a workspace, a
	// global CARGO_TARGET_DIR): naming the phase after its output directory
	// keeps each one's entry instead of evicting the others'.
	let phase = format!("wasm-bindgen {}", pkg_dir.display());
	Fingerprint::new(phase, wasm_bindgen_shared::version())
		.with_input("wasm", wasm_hash.to_owned())
		.with_input("release", if release { "1" } else { "0" }.to_owned())
		.with_input("pkg_dir", pkg_dir.display().to_string())
		.with_input("wasm_opt", wasm_opt_version.unwrap_or("off").to_owned())
		// Bumped when the glue or optimizer flags change, so caches holding
		// output from the old flags rebind: 1 — emit the default module path;
		// 2 — `wasm-opt -Oz`, and release output without name or producers
		// sections.
		.with_input("glue_flags", "2".to_owned())
		.with_input("split", if split { "1" } else { "0" }.to_owned())
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
		match &options.profile {
			Some(profile) => command.arg("--profile").arg(profile),
			None => command.arg("--release"),
		};
	}
	for config in &options.config {
		command.arg("--config").arg(config);
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

/// The cargo messages `--message-format=json` prints that [`compile_wasm`]
/// reads: compiled artifacts and the files they wrote.
#[derive(Debug, Deserialize)]
struct ArtifactMessage {
	reason: String,
	target: Option<ArtifactTarget>,
	#[serde(default)]
	filenames: Vec<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct ArtifactTarget {
	kind: Vec<String>,
}

/// The `.wasm` a cdylib artifact in cargo's JSON message stream wrote: the
/// compiled client, wherever the profile and target directory put it.
pub fn wasm_in_messages(messages: impl BufRead) -> Result<Option<PathBuf>> {
	let mut wasm = None;
	for line in messages.lines() {
		let line = line.context("could not read cargo's output")?;
		let Ok(message) = serde_json::from_str::<ArtifactMessage>(&line) else {
			continue;
		};
		let is_cdylib = message
			.target
			.is_some_and(|target| target.kind.iter().any(|kind| kind == "cdylib"));
		if message.reason == "compiler-artifact" && is_cdylib {
			let file = message.filenames.into_iter().find(|file| {
				file.extension()
					.is_some_and(|extension| extension == "wasm")
			});
			wasm = file.or(wasm);
		}
	}
	Ok(wasm)
}

/// Compiles the client crate to wasm. Pure: runs cargo, returns the artifact.
///
/// Only the library is built (`--lib`), so a package that is both the server
/// binary and the wasm client never tries to compile its server for wasm.
///
/// The artifact's path comes from cargo's JSON messages rather than from
/// cargo's directory naming, so any profile (`bundle.cargo_profile` included)
/// and any target directory resolve exactly. Diagnostics still render to the
/// terminal as usual.
pub fn compile_wasm(root: &Path, client: &ClientInfo, options: &CargoOptions) -> Result<PathBuf> {
	let mut child = cargo_build_command(root, &client.name, options)
		.arg("--lib")
		.arg("--target")
		.arg(WASM_TARGET)
		.arg("--message-format=json-render-diagnostics")
		.stdout(Stdio::piped())
		.spawn()
		.context("could not run cargo for the wasm build")?;
	let messages = child
		.stdout
		.take()
		.context("cargo's output was not captured")?;
	let wasm = wasm_in_messages(BufReader::new(messages))?;
	let status = child.wait().context("could not wait for cargo")?;
	anyhow::ensure!(status.success(), "wasm build failed");
	let path = wasm.with_context(|| {
		format!(
			"cargo built `{}` but reported no .wasm artifact",
			client.name
		)
	})?;
	anyhow::ensure!(
		path.is_file(),
		"cargo reported {} but it is missing",
		path.display()
	);
	Ok(path)
}

/// Runs wasm-bindgen in-process over the compiled wasm. Pure: writes the glue
/// into `out_dir`, named after `out_name`. Release output drops the name and
/// producers sections, which only debuggers and telemetry read.
///
/// A `split` main module keeps the linker exports its chunks import, and
/// keeps symbol names mangled so they still match the chunks'.
pub fn bindgen(
	wasm_path: &Path,
	out_dir: &Path,
	out_name: &str,
	release: bool,
	split: bool,
) -> Result<()> {
	let mut bindgen = wasm_bindgen_cli_support::Bindgen::new();
	bindgen
		.input_path(wasm_path)
		.out_name(out_name)
		.web(true)
		.context("the web output target is unavailable")?
		// The library defaults to omitting the default module path, unlike the
		// `wasm-bindgen` CLI; without it the glue's `init()` requires the
		// caller to pass the `.wasm` URL, and a plain `await init()` — as the
		// scaffold's `index.html` writes it — fails in the browser.
		.omit_default_module_path(false)
		.keep_lld_exports(split)
		.demangle(!split)
		.debug(!release)
		.remove_name_section(release)
		.remove_producers_section(release)
		.generate(out_dir)
		.with_context(|| format!("wasm-bindgen failed on {}", wasm_path.display()))?;
	Ok(())
}

/// The wasm features `wasm-opt` must accept: the ones rustc emits for
/// `wasm32-unknown-unknown` by default since Rust 1.87.
const WASM_OPT_FEATURES: [&str; 6] = [
	"--enable-bulk-memory",
	"--enable-nontrapping-float-to-int",
	"--enable-sign-ext",
	"--enable-mutable-globals",
	"--enable-reference-types",
	"--enable-multivalue",
];

/// Optimizes the processed wasm in place with `wasm-opt -Oz`, its most
/// aggressive size optimization. Pure.
pub fn optimize(wasm_path: &Path) -> Result<()> {
	let status = Command::new("wasm-opt")
		.arg("-Oz")
		.args(WASM_OPT_FEATURES)
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
		"could not run wasm-opt; install binaryen or set `bundle.wasm_opt = false` in \
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

/// The version of the wasm-bindgen crate this CLI embeds, such as `0.2.129`.
#[must_use]
pub fn embedded_wasm_bindgen() -> semver::Version {
	// `version()` is the crate version, followed by a git revision when
	// wasm-bindgen was built from a checkout.
	let version = wasm_bindgen_shared::version();
	let number = version.split_whitespace().next().unwrap_or_default();
	semver::Version::parse(number).expect("wasm-bindgen reports a semver version")
}

/// Whether wasm built against wasm-bindgen `version` binds with the embedded
/// wasm-bindgen. Bindgen requires the two *schema* versions to match, and
/// wasm-bindgen sets its schema version to the release that last changed the
/// schema, so every release from that one up to the embedded one shares it.
fn binds_with_embedded(version: &str) -> bool {
	let (Ok(version), Ok(schema)) = (
		semver::Version::parse(version),
		semver::Version::parse(wasm_bindgen_shared::SCHEMA_VERSION),
	) else {
		return false;
	};
	schema <= version && version <= embedded_wasm_bindgen()
}

/// Compares the `wasm-bindgen` versions in a `Cargo.lock` with the
/// wasm-bindgen embedded in this CLI: each must share its schema, which
/// wasm-bindgen refuses to bind across.
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
		.filter(|version| !binds_with_embedded(version))
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
	let embedded = embedded_wasm_bindgen();
	let schema = wasm_bindgen_shared::SCHEMA_VERSION;
	format!(
		"the project locks wasm-bindgen {}, but leptoskit embeds wasm-bindgen {embedded}, which \
		 binds wasm from wasm-bindgen {schema} through {embedded} only; run `cargo update \
		 --package wasm-bindgen --precise {embedded}`",
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
