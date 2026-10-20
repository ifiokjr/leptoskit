//! CLI tests: scaffolding, caching, configuration errors, and code push,
//! driven through the library and through the built `leptoskit` binary in
//! real temporary projects.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_cli::cache::BuildCache;
use leptoskit_cli::cache::Fingerprint;
use leptoskit_cli::new_project;

struct Scratch {
	path: PathBuf,
}

impl Scratch {
	fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_cli_test_{tag}_{}_{}",
			std::process::id(),
			unique
		));
		std::fs::create_dir_all(&path).expect("scratch");
		// Canonical, so paths compare equal to what the CLI resolves.
		let path = std::fs::canonicalize(&path).expect("canonical scratch");
		Self { path }
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.path);
	}
}

/// Runs the built `leptoskit` binary in `dir`.
fn leptoskit(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(args)
		.current_dir(dir)
		.output()
		.expect("run leptoskit")
}

fn stderr(output: &Output) -> String {
	String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
	String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn scaffolds_a_project_with_config_and_source() {
	let scratch = Scratch::new("scaffold");
	let project = scratch.path.join("counter");
	new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Native,
		None,
	)
	.expect("scaffold");
	new_project::verify_scaffold(&project).expect("verify");

	// Loading runs the config's own validation, which the scaffold must pass.
	let config = leptoskit_config::LeptoskitConfig::load_from(&project).expect("load config");
	assert_eq!(config.app.name, "counter");
	assert_eq!(
		config.app.default_platform,
		leptoskit_config::Platform::Native
	);
	assert_eq!(
		config.native.package_id,
		leptoskit_config::default_package_id("counter")
	);
	assert!(
		config.plugins.is_empty(),
		"the template enables no plugin whose cargo feature it lacks"
	);

	let client = std::fs::read_to_string(project.join("client/Cargo.toml")).expect("client");
	assert!(client.contains("cdylib"));
	assert!(client.contains(&format!(
		"wasm-bindgen = \"={}\"",
		wasm_bindgen_shared::SCHEMA_VERSION
	)));
	let index = std::fs::read_to_string(project.join("public/index.html")).expect("index");
	assert!(index.contains("./pkg/counter_client.js"));
}

#[test]
fn native_scaffold_carries_the_swift_package_host() {
	let scratch = Scratch::new("native-scaffold");
	let project = scratch.path.join("counter_native");
	new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Native,
		None,
	)
	.expect("scaffold");

	for file in new_project::native_template_files("counter_native") {
		let path = project.join(&file);
		assert!(path.is_file(), "native scaffold is missing {file}");
	}
	let manifest = std::fs::read_to_string(project.join("hosts/apple/Package.swift"))
		.expect("read Package.swift");
	assert!(manifest.contains("swift-tools-version:6.0"));
	assert!(!manifest.contains("Podfile"), "no CocoaPods anywhere");
	assert!(
		manifest.contains("leptoskit_runtime"),
		"the runtime bridge is documented in place"
	);
	assert!(
		manifest.contains("https://ifiokjr.github.io/leptoskit/platforms/host-projects.html"),
		"the host design link points at the published docs"
	);

	let entry =
		std::fs::read_to_string(project.join("hosts/apple/Sources/counter_native/main.swift"))
			.expect("read main.swift");
	assert!(entry.contains("code push"));
}

#[test]
fn web_scaffold_has_no_host_directories() {
	let scratch = Scratch::new("web-scaffold");
	let project = scratch.path.join("counter_web");
	new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Web,
		None,
	)
	.expect("scaffold");
	assert!(
		!project.join("hosts").exists(),
		"web projects carry no host"
	);
}

#[test]
fn scaffolding_twice_fails_cleanly() {
	let scratch = Scratch::new("twice");
	let project = scratch.path.join("dupe");
	new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Web,
		None,
	)
	.expect("first scaffold");
	let error = new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Web,
		None,
	)
	.expect_err("second scaffold");
	assert!(error.to_string().contains("already exists"), "got {error}");
}

#[test]
fn scaffolding_never_overwrites_existing_files() {
	let scratch = Scratch::new("no-clobber");
	let project = scratch.path.join("existing");
	std::fs::create_dir_all(&project).expect("mkdir");
	std::fs::write(project.join("Cargo.toml"), "# mine\n").expect("existing manifest");

	let error = new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Web,
		None,
	)
	.expect_err("an existing Cargo.toml blocks the scaffold");
	assert!(error.to_string().contains("Cargo.toml"), "got {error}");
	assert_eq!(
		std::fs::read_to_string(project.join("Cargo.toml")).expect("read"),
		"# mine\n"
	);
	assert!(
		!project.join(leptoskit_config::CONFIG_FILE).exists(),
		"a conflict leaves no half-written scaffold"
	);
}

#[test]
fn new_in_the_current_directory_uses_its_name() {
	let scratch = Scratch::new("dot");
	let project = scratch.path.join("dotted_app");
	std::fs::create_dir_all(&project).expect("mkdir");
	let output = leptoskit(&project, &["new", "."]);
	assert!(output.status.success(), "new . failed: {}", stderr(&output));
	let config = leptoskit_config::LeptoskitConfig::load_from(&project).expect("load");
	assert_eq!(config.app.name, "dotted_app");
	let manifest = std::fs::read_to_string(project.join("Cargo.toml")).expect("manifest");
	assert!(manifest.contains("name = \"dotted_app\""));
}

#[test]
fn new_rejects_names_cargo_would_reject() {
	let scratch = Scratch::new("bad-name");
	let error = new_project::run(
		scratch.path.join("app").to_string_lossy().as_ref(),
		leptoskit_config::Platform::Web,
		Some("1 bad name"),
	)
	.expect_err("invalid package name");
	assert!(
		error.to_string().contains("not a valid cargo package name"),
		"got {error}"
	);
}

#[test]
fn malformed_config_is_reported_as_a_parse_error() {
	let scratch = Scratch::new("malformed");
	std::fs::write(
		scratch.path.join(leptoskit_config::CONFIG_FILE),
		"[app\nname = ",
	)
	.expect("write config");
	for args in [
		&["build"][..],
		&["serve"],
		&["codepush", "release"],
		&["codepush", "check"],
	] {
		let output = leptoskit(&scratch.path, args);
		assert!(!output.status.success());
		let message = stderr(&output);
		assert!(
			message.contains("could not parse"),
			"{args:?} should report the parse error, got {message}"
		);
		assert!(
			!message.contains("no leptoskit project found"),
			"{args:?} must not claim there is no project, got {message}"
		);
	}
}

#[test]
fn unknown_plugins_fail_the_build() {
	let scratch = Scratch::new("unknown-plugin");
	std::fs::write(
		scratch.path.join(leptoskit_config::CONFIG_FILE),
		"[app]\nname = \"app\"\n\n[[plugins]]\nname = \"not_a_plugin\"\n",
	)
	.expect("write config");
	let output = leptoskit(&scratch.path, &["build"]);
	assert!(!output.status.success());
	let message = stderr(&output);
	assert!(
		message.contains("unknown plugin(s) in leptoskit.toml: not_a_plugin"),
		"got {message}"
	);
}

/// A server crate that only compiles when `build.features` and the
/// `env_manifest` plugin's environment both reach cargo.
#[test]
fn config_features_and_plugin_env_reach_cargo() {
	let scratch = Scratch::new("features");
	let root = &scratch.path;
	std::fs::write(
		root.join(leptoskit_config::CONFIG_FILE),
		r#"[app]
name = "feature_probe"
default_platform = "server"

[build]
features = ["probe"]

[[plugins]]
name = "env_manifest"
"#,
	)
	.expect("config");
	std::fs::write(
		root.join("Cargo.toml"),
		r#"[package]
name = "feature_probe"
version = "0.1.0"
edition = "2024"

[workspace]

[features]
probe = []
"#,
	)
	.expect("manifest");
	std::fs::create_dir_all(root.join("src")).expect("src");
	std::fs::write(
		root.join("src/main.rs"),
		r#"#[cfg(not(feature = "probe"))]
compile_error!("build.features did not reach cargo");

const PLATFORM: &str = env!("LEPTOSKIT_PLUGIN_PLATFORM");

fn main() {
	println!("{PLATFORM}");
}
"#,
	)
	.expect("main");

	let output = leptoskit(root, &["build"]);
	assert!(
		output.status.success(),
		"build failed:\n{}\n{}",
		stdout(&output),
		stderr(&output)
	);
	assert!(stdout(&output).contains("built server"));
}

fn codepush_project(dir: &Path, version: &str) {
	std::fs::write(
		dir.join(leptoskit_config::CONFIG_FILE),
		format!("[app]\nname = \"pushed\"\nversion = \"{version}\"\n"),
	)
	.expect("config");
	std::fs::create_dir_all(dir.join("dist")).expect("dist");
	std::fs::write(dir.join("dist/index.html"), format!("<p>{version}</p>")).expect("index");
}

#[test]
fn codepush_release_numbers_builds_and_refuses_older_versions() {
	let scratch = Scratch::new("codepush");
	codepush_project(&scratch.path, "0.2.0");
	let first = leptoskit(&scratch.path, &["codepush", "release"]);
	assert!(first.status.success(), "{}", stderr(&first));
	assert!(stdout(&first).contains("0.2.0+1"), "{}", stdout(&first));

	std::fs::write(scratch.path.join("dist/index.html"), "<p>again</p>").expect("index");
	let second = leptoskit(&scratch.path, &["codepush", "release"]);
	assert!(second.status.success(), "{}", stderr(&second));
	assert!(stdout(&second).contains("0.2.0+2"), "{}", stdout(&second));

	let current = leptoskit(
		&scratch.path,
		&["codepush", "check", "--version", "0.2.0", "--build", "2"],
	);
	assert!(current.status.success(), "{}", stderr(&current));
	assert!(stdout(&current).contains("0.2.0+2 on production: up to date"));
	let behind = leptoskit(
		&scratch.path,
		&["codepush", "check", "--version", "0.2.0", "--build", "1"],
	);
	assert!(behind.status.success(), "{}", stderr(&behind));
	assert!(
		stdout(&behind).contains("update available: 0.2.0+2"),
		"{}",
		stdout(&behind)
	);

	codepush_project(&scratch.path, "0.1.0");
	let older = leptoskit(&scratch.path, &["codepush", "release"]);
	assert!(!older.status.success(), "an older version must not release");
	assert!(
		stderr(&older).contains("older than the latest production release 0.2.0+2"),
		"{}",
		stderr(&older)
	);
}

#[test]
fn doctor_fails_when_required_tools_are_missing() {
	let scratch = Scratch::new("doctor");
	let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.arg("doctor")
		.current_dir(&scratch.path)
		.env("PATH", "")
		.output()
		.expect("run leptoskit doctor");
	assert!(
		!output.status.success(),
		"a missing cargo must fail the doctor"
	);
	assert!(stdout(&output).contains("✗ cargo"), "{}", stdout(&output));
	assert!(
		stderr(&output).contains("required check(s) failed"),
		"{}",
		stderr(&output)
	);
}

#[test]
fn fingerprints_change_with_inputs() {
	let scratch = Scratch::new("fingerprints");
	let file = scratch.path.join("input.txt");
	std::fs::write(&file, b"one").expect("write");

	let first = Fingerprint::new("phase", "tool 1").with_file("input", &file);
	let same = Fingerprint::new("phase", "tool 1").with_file("input", &file);
	assert_eq!(first.digest(), same.digest());

	std::fs::write(&file, b"two").expect("rewrite");
	let changed = Fingerprint::new("phase", "tool 1").with_file("input", &file);
	assert_ne!(first.digest(), changed.digest());

	let retargeted = Fingerprint::new("other", "tool 1").with_file("input", &file);
	assert_ne!(changed.digest(), retargeted.digest());
}

#[test]
fn build_cache_round_trips_and_skips_unchanged_phases() {
	let scratch = Scratch::new("cache");
	let mut cache = BuildCache::load(&scratch.path);
	let fingerprint = Fingerprint::new("phase", "tool");
	assert!(!cache.is_current(&fingerprint));
	cache.record(&fingerprint);
	assert!(cache.is_current(&fingerprint));
	cache.save().expect("save");
	assert!(scratch.path.join("leptoskit/cache.json").is_file());

	let reloaded = BuildCache::load(&scratch.path);
	assert!(reloaded.is_current(&fingerprint));
	assert!(!reloaded.is_current(&Fingerprint::new("other", "tool")));
}

#[test]
fn scaffold_files_are_the_promised_set() {
	assert_eq!(
		new_project::template_files(),
		&[
			leptoskit_config::CONFIG_FILE,
			".gitignore",
			"Cargo.toml",
			"src/main.rs",
			"client/Cargo.toml",
			"client/src/lib.rs",
			"public/index.html",
		]
	);
}

/// The full loop on a fresh scaffold: `new`, `build`, a cached rebuild, a
/// rebuild after the output was deleted, and a build into a second output
/// directory.
///
/// Ignored by default because it needs the `wasm32-unknown-unknown` target
/// and a registry fetch for wasm-bindgen. Run it with
/// `cargo nextest run --package leptoskit_cli --run-ignored only --
/// new_project_builds`.
#[test]
#[ignore = "needs the wasm32 target and a registry fetch"]
fn new_project_builds_caches_and_restores_outputs() {
	let scratch = Scratch::new("e2e");
	let created = leptoskit(&scratch.path, &["new", "e2e_app"]);
	assert!(created.status.success(), "{}", stderr(&created));
	let project = scratch.path.join("e2e_app");
	let glue = project.join("dist/pkg/e2e_app_client.js");
	let module = project.join("dist/pkg/e2e_app_client_bg.wasm");

	let first = leptoskit(&project, &["build"]);
	assert!(first.status.success(), "first build: {}", stderr(&first));
	assert!(stdout(&first).contains("wasm client bound"));
	assert!(glue.is_file() && module.is_file());
	assert!(project.join("dist/index.html").is_file());

	let second = leptoskit(&project, &["build"]);
	assert!(second.status.success(), "second build: {}", stderr(&second));
	assert!(
		stdout(&second).contains("skipped wasm-bindgen"),
		"{}",
		stdout(&second)
	);

	std::fs::remove_dir_all(project.join("dist")).expect("remove dist");
	let restored = leptoskit(&project, &["build"]);
	assert!(restored.status.success(), "{}", stderr(&restored));
	assert!(
		glue.is_file() && module.is_file(),
		"deleted outputs are rebuilt"
	);

	let elsewhere = leptoskit(&project, &["build", "--out", "dist2"]);
	assert!(elsewhere.status.success(), "{}", stderr(&elsewhere));
	assert!(project.join("dist2/pkg/e2e_app_client.js").is_file());
}
