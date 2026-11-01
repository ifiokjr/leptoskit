//! CLI tests: scaffolding, caching, configuration errors, and code push,
//! driven through the library and through the built `leptoskit` binary in
//! real temporary projects. Projects under test come from `tests/fixtures`,
//! so the same on-disk fixtures back these tests and the snapshot suite.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_cli::cache::BuildCache;
use leptoskit_cli::cache::Fingerprint;
use leptoskit_cli::codepush::SIGNING_KEY_ENV;
use leptoskit_cli::new_project;
use leptoskit_cli::s3::ACCESS_KEY_ID_ENV;
use leptoskit_cli::s3::SECRET_ACCESS_KEY_ENV;
use leptoskit_cli::s3::SESSION_TOKEN_ENV;

mod support;

use support::Scratch;
use support::fixture_project;

/// Every file every fixture is expected to carry. Fixtures are data, not
/// build output: if one of these is missing, a `.gitignore` pattern —
/// `**/dist/**` swallowed the codepush fixture's `dist/` once — is the
/// first suspect, and this fails here with that hint rather than as a
/// confusing CLI error in CI's fresh clone.
#[test]
fn every_fixture_file_is_checked_in() {
	let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
	for (fixture, files) in [
		("codepush", &["leptoskit.toml", "dist/index.html"][..]),
		(
			"feature_probe",
			&["leptoskit.toml", "Cargo.toml", "src/main.rs"][..],
		),
		("malformed", &["leptoskit.toml"][..]),
		("unknown_plugin", &["leptoskit.toml"][..]),
		(
			"wasm_workspace",
			&[
				"Cargo.toml",
				"crates/fixture_client/Cargo.toml",
				"crates/fixture_client/src/lib.rs",
				"crates/fixture_server/Cargo.toml",
				"crates/fixture_server/src/main.rs",
			][..],
		),
	] {
		for file in files {
			assert!(
				root.join(fixture).join(file).is_file(),
				"tests/fixtures/{fixture}/{file} is missing from the checkout; a .gitignore \
				 pattern may have swallowed it"
			);
		}
	}
}

/// Runs the built `leptoskit` binary in `dir`.
/// Runs the binary in `dir`, without any signing key from the environment
/// the tests happen to run in.
fn leptoskit(dir: &Path, args: &[&str]) -> Output {
	leptoskit_with_key_env(dir, args, None)
}

fn leptoskit_with_key_env(dir: &Path, args: &[&str], signing_key: Option<&str>) -> Output {
	let signing_key = signing_key.map(|key| (SIGNING_KEY_ENV, key));
	leptoskit_with_env(dir, args, signing_key.as_slice())
}

/// Runs the binary in `dir` with exactly the code push secrets in `env`, none
/// inherited from the environment the tests happen to run in.
fn leptoskit_with_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
	let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit"));
	command.args(args).current_dir(dir);
	for secret in [
		SIGNING_KEY_ENV,
		ACCESS_KEY_ID_ENV,
		SECRET_ACCESS_KEY_ENV,
		SESSION_TOKEN_ENV,
	] {
		command.env_remove(secret);
	}
	command.envs(env.iter().copied());
	command.output().expect("run leptoskit")
}

/// Runs the built `lk` alias binary in `dir`.
fn lk(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_lk"))
		.args(args)
		.current_dir(dir)
		.output()
		.expect("run lk")
}

fn stderr(output: &Output) -> String {
	String::from_utf8_lossy(&output.stderr).into_owned()
}

#[cfg(unix)]
#[test]
fn disabled_legacy_native_platforms_fail_before_invoking_cargo() {
	use std::os::unix::fs::PermissionsExt as _;

	let scratch = Scratch::new("disabled-legacy-native");
	let tools = scratch.path.join("tools");
	std::fs::create_dir_all(&tools).unwrap();
	std::fs::write(
		scratch.path.join("leptoskit.toml"),
		"[app]\nname = \"counter\"\ndefault_platform = \"native\"\nplatforms = \
		 [\"web\"]\n\n[native]\ntargets = [\"android\"]\n",
	)
	.unwrap();
	std::fs::create_dir_all(scratch.path.join("hosts/android")).unwrap();
	let cargo = tools.join("cargo");
	std::fs::write(
		&cargo,
		"#!/bin/sh\nprintf invoked > cargo-invoked\nexit 91\n",
	)
	.unwrap();
	std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();

	let path = std::env::join_paths(
		std::iter::once(tools).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
	)
	.unwrap();

	for args in [
		vec!["native", "prepare", "android"],
		vec!["native", "build", "android"],
		vec!["native", "bundle", "--target", "android"],
		vec!["build", "--platform", "native"],
	] {
		let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
			.current_dir(&scratch.path)
			.env("PATH", &path)
			.args(&args)
			.output()
			.unwrap();
		assert!(!output.status.success(), "{args:?} unexpectedly succeeded");
		assert!(
			stderr(&output).contains("platform `android` is disabled"),
			"{args:?}: {}",
			stderr(&output)
		);
		assert!(
			!scratch.path.join("cargo-invoked").exists(),
			"{args:?} invoked cargo"
		);
	}
}

fn stdout(output: &Output) -> String {
	String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn generate_app_uses_the_existing_application_scaffold() {
	let scratch = Scratch::new("generate-app");
	let output = leptoskit(
		&scratch.path,
		&["generate", "app", "app", "--platform", "web"],
	);
	assert!(output.status.success(), "{}", stderr(&output));
	new_project::verify_scaffold(&scratch.path.join("app")).unwrap();
}

#[test]
fn generated_plugin_is_a_standalone_compilable_crate_and_never_overwrites() {
	let scratch = Scratch::new("generate-plugin");
	let bridge = Path::new(env!("CARGO_MANIFEST_DIR")).join("../leptoskit_bridge");
	let bridge = std::fs::canonicalize(bridge).unwrap();
	let output = leptoskit(
		&scratch.path,
		&[
			"generate",
			"plugin",
			"echo",
			"--bridge-path",
			bridge.to_str().unwrap(),
		],
	);
	assert!(output.status.success(), "{}", stderr(&output));
	let project = scratch.path.join("echo");
	let output = Command::new("cargo")
		.args(["test", "--offline", "--quiet", "--jobs", "2"])
		.current_dir(&project)
		.output()
		.unwrap();
	assert!(output.status.success(), "{}", stderr(&output));

	let source = project.join("src/lib.rs");
	let original = std::fs::read(&source).unwrap();
	let output = leptoskit(&scratch.path, &["generate", "plugin", "echo"]);
	assert!(!output.status.success());
	assert_eq!(std::fs::read(source).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn plugin_generation_refuses_symlinked_source_directories() {
	let scratch = Scratch::new("generate-symlink");
	let project = scratch.path.join("plugin");
	let outside = scratch.path.join("outside");
	std::fs::create_dir_all(&project).unwrap();
	std::fs::create_dir_all(&outside).unwrap();
	std::os::unix::fs::symlink(&outside, project.join("src")).unwrap();
	let output = leptoskit(&scratch.path, &["generate", "plugin", "plugin"]);
	assert!(!output.status.success());

	assert!(!project.join("Cargo.toml").exists());
	assert!(!outside.join("lib.rs").exists());
}

#[test]
fn scaffolds_a_project_with_config_and_source() {
	let scratch = Scratch::new("scaffold");
	let project = scratch.path.join("counter");
	new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Web,
		None,
	)
	.expect("scaffold");
	new_project::verify_scaffold(&project).expect("verify");

	// Loading runs the config's own validation, which the scaffold must pass.
	let config = leptoskit_config::LeptoskitConfig::load_from(&project).expect("load config");
	assert_eq!(config.app.name, "counter");
	assert_eq!(config.app.default_platform, leptoskit_config::Platform::Web);
	assert_eq!(
		config.app.platforms,
		leptoskit_config::Platform::APPLICATION
	);
	assert_eq!(config.app.render, leptoskit_config::RenderMode::Ssr);
	assert_eq!(config.build.client_package.as_deref(), Some("counter_app"));
	assert_eq!(
		config.native.package_id,
		leptoskit_config::default_package_id("counter")
	);
	assert!(
		config.plugins.is_empty(),
		"the template enables no plugin whose cargo feature it lacks"
	);

	let client = std::fs::read_to_string(project.join("counter_app/Cargo.toml")).expect("app");
	assert!(client.contains("cdylib"));
	assert!(client.contains(&format!(
		"wasm-bindgen = {{ version = \"={}\"",
		leptoskit_cli::wasm::embedded_wasm_bindgen()
	)));
	let index = std::fs::read_to_string(project.join("web/index.html")).expect("index");
	assert!(index.contains("/pkg/counter_app.js"));
	assert!(index.contains("href=\"/assets/app.css\""));
	assert_eq!(config.native.plugins, [] as [String; 0]);
}

#[test]
fn native_scaffold_carries_mobile_apps_and_does_not_overwrite_sources() {
	let scratch = Scratch::new("native-scaffold");
	let project = scratch.path.join("counter_native");
	new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Native,
		None,
	)
	.expect("scaffold");

	for file in [
		"ios/App.swift",
		"ios/AppUITests.swift",
		"android/gradlew",
		"android/app/src/main/java/MainActivity.kt",
		"counter_native_app/src/lib.rs",
	] {
		let path = project.join(file);
		assert!(path.is_file(), "native scaffold is missing {file}");
	}
	std::fs::write(project.join("ios/App.swift"), "// user change").unwrap();
	assert!(
		new_project::run(
			project.to_str().unwrap(),
			leptoskit_config::Platform::Native,
			None
		)
		.is_err()
	);
	assert_eq!(
		std::fs::read_to_string(project.join("ios/App.swift")).unwrap(),
		"// user change"
	);
}

#[test]
fn native_bundle_compiles_and_optimizes_the_guest() {
	let scratch = fixture_project("native_guest");
	let output = leptoskit(
		&scratch.path,
		&["native", "bundle", "--target", "android", "--out", "bundle"],
	);
	assert!(output.status.success(), "{}", stderr(&output));
	let module = scratch.path.join("bundle/app.wasm");
	let wasm = std::fs::read(&module).expect("bundled guest");
	assert_eq!(&wasm[..4], b"\0asm", "the bundle carries the guest module");
	let runtime = std::fs::read_to_string(scratch.path.join("bundle/runtime.json"))
		.expect("runtime descriptor");
	assert!(
		runtime.contains("\"abi\""),
		"the descriptor names the ABI: {runtime}"
	);
}

#[test]
fn web_default_scaffold_includes_every_app_owned_platform_runner() {
	let scratch = Scratch::new("web-scaffold");
	let project = scratch.path.join("counter_web");
	new_project::run(
		project.to_string_lossy().as_ref(),
		leptoskit_config::Platform::Web,
		None,
	)
	.expect("scaffold");

	for platform in ["ios", "android", "macos", "windows", "linux"] {
		let manifest: toml::Table =
			std::fs::read_to_string(project.join(platform).join("Cargo.toml"))
				.expect("host manifest")
				.parse()
				.expect("TOML");
		assert_eq!(
			manifest["package"]["name"].as_str(),
			Some(format!("counter_web_{platform}").as_str())
		);
	}

	assert!(!project.join("guest").exists());

	assert!(!project.join("client").exists());
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
fn a_platform_source_conflict_leaves_no_partial_workspace() {
	let scratch = Scratch::new("platform-conflict");
	let project = scratch.path.join("counter");
	std::fs::create_dir_all(project.join("ios")).expect("platform directory");
	std::fs::write(project.join("ios/App.swift"), "// owned by the app\n")
		.expect("existing source");

	let error = new_project::run(
		project.to_str().expect("UTF-8"),
		leptoskit_config::Platform::Ios,
		None,
	)
	.expect_err("platform sources are protected before any files are written");
	assert!(
		error
			.to_string()
			.contains(project.join("ios/App.swift").to_str().expect("UTF-8"))
	);
	assert!(!project.join("Cargo.toml").exists());
	assert_eq!(
		std::fs::read_to_string(project.join("ios/App.swift")).unwrap(),
		"// owned by the app\n"
	);
}

#[test]
fn selecting_a_platform_changes_the_default_and_keeps_all_destinations_enabled() {
	let scratch = Scratch::new("platform-default");
	let output = leptoskit(&scratch.path, &["new", "counter", "--platform", "android"]);
	assert!(output.status.success(), "{}", stderr(&output));
	let config = leptoskit_config::LeptoskitConfig::load_from(&scratch.path.join("counter"))
		.expect("config");
	assert_eq!(
		config.app.default_platform,
		leptoskit_config::Platform::Android
	);
	assert_eq!(
		config.app.platforms,
		leptoskit_config::Platform::APPLICATION
	);
}

#[cfg(unix)]
#[test]
fn scaffolding_refuses_symlinked_platform_directories_before_writing_files() {
	let scratch = Scratch::new("platform-symlink");
	let project = scratch.path.join("counter");
	let outside = scratch.path.join("outside");
	std::fs::create_dir_all(&project).unwrap();
	std::fs::create_dir_all(&outside).unwrap();
	std::os::unix::fs::symlink(&outside, project.join("ios")).unwrap();

	let error = new_project::run(
		project.to_str().unwrap(),
		leptoskit_config::Platform::Web,
		None,
	)
	.expect_err("app-owned source must not redirect writes outside the workspace");
	assert!(error.to_string().contains("ordinary directory"));
	assert!(!project.join("Cargo.toml").exists());
	assert!(!outside.join("App.swift").exists());
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
	let scratch = fixture_project("malformed");
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
	let scratch = fixture_project("unknown_plugin");
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
	let scratch = fixture_project("feature_probe");
	let output = leptoskit(&scratch.path, &["build"]);
	assert!(
		output.status.success(),
		"build failed:\n{}\n{}",
		stdout(&output),
		stderr(&output)
	);
	assert!(stdout(&output).contains("built server"));
}

/// Writes the codepush fixture's config with `version` and a matching
/// `dist/index.html`, so a release has known content to pack.
fn codepush_project(dir: &Path, version: &str) {
	codepush_project_trusting(dir, version, &[]);
}

fn codepush_project_trusting(dir: &Path, version: &str, public_keys: &[&str]) {
	std::fs::write(
		dir.join(leptoskit_config::CONFIG_FILE),
		format!(
			"[app]\nname = \"pushed\"\nversion = \"{version}\"\n\n[codepush]\npublic_keys = \
			 {public_keys:?}\n"
		),
	)
	.expect("config");
	std::fs::create_dir_all(dir.join("dist")).expect("dist");
	std::fs::write(dir.join("dist/index.html"), format!("<p>{version}</p>")).expect("index");
	support::write_web_module(&dir.join("dist"));
}

#[test]
fn codepush_release_numbers_builds_and_refuses_older_versions() {
	let scratch = fixture_project("codepush");
	let keygen = leptoskit(
		&scratch.path,
		&["codepush", "keygen", "--out", "publisher.key"],
	);
	assert!(keygen.status.success(), "{}", stderr(&keygen));
	let release = ["codepush", "release", "--signing-key", "publisher.key"];
	let first = leptoskit(&scratch.path, &release);
	assert!(first.status.success(), "{}", stderr(&first));
	assert!(stdout(&first).contains("0.2.0+1"), "{}", stdout(&first));

	std::fs::write(scratch.path.join("dist/index.html"), "<p>again</p>").expect("index");
	let second = leptoskit(&scratch.path, &release);
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
	let older = leptoskit(&scratch.path, &release);
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
	assert!(stdout(&output).contains("✖ cargo"), "{}", stdout(&output));
	assert!(
		stderr(&output).contains("required checks failed; builds will not work until they pass\n"),
		"{}",
		stderr(&output)
	);
}

#[test]
fn doctor_reports_a_lockfile_that_binds_with_the_embedded_wasm_bindgen() {
	let scratch = Scratch::new("doctor-lock");
	std::fs::write(
		scratch.path.join(leptoskit_config::CONFIG_FILE),
		"[app]\nname = \"app\"\n",
	)
	.expect("config");
	let embedded = leptoskit_cli::wasm::embedded_wasm_bindgen();
	std::fs::write(
		scratch.path.join("Cargo.lock"),
		format!("version = 4\n\n[[package]]\nname = \"wasm-bindgen\"\nversion = \"{embedded}\"\n"),
	)
	.expect("lockfile");
	let output = leptoskit(&scratch.path, &["doctor"]);
	assert!(
		stdout(&output).contains(&format!("binds with the embedded wasm-bindgen {embedded}")),
		"{}",
		stdout(&output)
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
	let entries: Vec<_> = std::fs::read_dir(scratch.path.join("leptoskit/cache"))
		.expect("cache dir")
		.map(|entry| entry.expect("entry").file_name())
		.collect();

	assert_eq!(
		entries.len(),
		1,
		"one file per phase, no partial left: {entries:?}"
	);

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
			new_project::RELEASE_WORKFLOW,
			"web/index.html",
			"assets/app.css",
			"README.md",
		]
	);
}

/// The `lk` binary is the same CLI under its alias: version, help, and
/// scaffolding all answer to it, and output echoes the alias the user typed.
#[test]
fn the_lk_binary_is_an_alias_for_leptoskit() {
	let scratch = Scratch::new("lk-alias");

	let version = lk(&scratch.path, &["--version"]);
	let canonical_version = leptoskit(&scratch.path, &["--version"]);
	assert!(version.status.success());
	assert_eq!(
		stdout(&version),
		stdout(&canonical_version),
		"the alias reports the canonical version"
	);

	let help = lk(&scratch.path, &["--help"]);
	assert!(help.status.success());
	assert!(
		stdout(&help).contains("Usage: lk"),
		"help should answer to the alias, got {}",
		stdout(&help)
	);

	let created = lk(&scratch.path, &["new", "aliased"]);
	assert!(created.status.success(), "{}", stderr(&created));
	let project = scratch.path.join("aliased");
	let config = leptoskit_config::LeptoskitConfig::load_from(&project).expect("load");
	assert_eq!(config.app.name, "aliased");
	assert!(
		stdout(&created).contains("lk build"),
		"the scaffold's next step echoes the alias, got {}",
		stdout(&created)
	);
}

#[test]
fn completions_follow_the_invoked_binary() {
	let scratch = Scratch::new("lk-completions");
	let alias = lk(&scratch.path, &["completions", "zsh"]);
	assert!(alias.status.success(), "{}", stderr(&alias));
	assert!(
		stdout(&alias).starts_with("#compdef lk"),
		"zsh completions should target the alias, got {}",
		stdout(&alias)
	);
	let canonical = leptoskit(&scratch.path, &["completions", "zsh"]);
	assert!(canonical.status.success());
	assert!(
		stdout(&canonical).starts_with("#compdef leptoskit"),
		"got {}",
		stdout(&canonical)
	);
}

/// The full loop on a fresh scaffold: `new`, `build`, a cached rebuild, a
/// rebuild after the output was deleted, and a build into a second output
/// directory. The scaffold depends on this repository through `--sdk-path`,
/// so it builds against the leptoskit crates under test, not the last release.
///
/// Ignored by default because it compiles for `wasm32-unknown-unknown` and
/// fetches dependencies. The `coverage` CI job runs it, instrumented, with the
/// other ignored tests of the end-to-end targets: `cargo test --package
/// leptoskit_cli --test cli --test render --test wasm_pipeline --test size --
/// --ignored`.
#[test]
#[ignore = "builds for wasm32 and fetches dependencies; run by the `coverage` CI job"]
fn new_project_builds_caches_and_restores_outputs() {
	let scratch = Scratch::new("e2e");
	let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
	let sdk = sdk.to_str().expect("UTF-8 repository path");
	let created = leptoskit(&scratch.path, &["new", "e2e_app", "--sdk-path", sdk]);
	assert!(created.status.success(), "{}", stderr(&created));
	let project = scratch.path.join("e2e_app");
	let glue = project.join("dist/pkg/e2e_app_app.js");
	let module = project.join("dist/pkg/e2e_app_app_bg.wasm");

	let first = leptoskit(&project, &["build"]);
	assert!(first.status.success(), "first build: {}", stderr(&first));
	assert!(stdout(&first).contains("wasm client bound"));
	assert!(glue.is_file() && module.is_file());
	assert!(project.join("dist/assets/app.css").is_file());

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
	assert!(project.join("dist2/pkg/e2e_app_app.js").is_file());
}

/// `leptoskit dev` on a new project's web app. Server-rendered, it builds the
/// app and runs its server, reports a broken edit and keeps running, rebuilds
/// and restarts the server once the edit is fixed, and on `SIGTERM` stops the
/// server and exits cleanly. Client-rendered, it serves `dist/` itself and
/// skips the enabled platforms this machine cannot run. Ignored with the
/// other end-to-end builds; the `coverage` CI job runs it.
#[cfg(unix)]
#[test]
#[ignore = "builds for wasm32 and fetches dependencies; run by the `coverage` CI job"]
fn dev_serves_the_web_app_rebuilds_it_on_change_and_stops_on_sigterm() {
	use std::time::Duration;

	let scratch = Scratch::new("dev-web");
	let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
	let sdk = sdk.to_str().expect("UTF-8 repository path");
	let created = leptoskit(&scratch.path, &["new", "dev_app", "--sdk-path", sdk]);
	assert!(created.status.success(), "{}", stderr(&created));
	let project = scratch.path.join("dev_app");
	let port = std::net::TcpListener::bind("127.0.0.1:0")
		.and_then(|listener| listener.local_addr())
		.expect("a free port")
		.port()
		.to_string();

	let mut dev = DevSession::start(&project, &["dev", "--platform", "web", "--port", &port]);
	dev.expect("[web] app server at", Duration::from_mins(15));
	dev.expect("watching", Duration::from_mins(1));
	assert!(page(&port).contains("<html"), "the server renders the page");

	let main = project.join("src/main.rs");
	let source = std::fs::read_to_string(&main).expect("read src/main.rs");
	std::fs::write(&main, format!("{source}\nfn broken( {{\n")).expect("break src/main.rs");
	dev.expect("[web] rebuild failed", Duration::from_mins(10));
	std::fs::write(
		&main,
		format!("{source}\n// Changed while `leptoskit dev` watches.\n"),
	)
	.expect("fix src/main.rs");
	dev.expect("[web] rebuilt in", Duration::from_mins(10));
	assert!(
		page(&port).contains("<html"),
		"the restarted server renders the page"
	);
	dev.stop();

	assert!(
		std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_err(),
		"the app server stops with `leptoskit dev`"
	);

	let config = project.join(leptoskit_config::CONFIG_FILE);
	let text = std::fs::read_to_string(&config).expect("read leptoskit.toml");
	let platforms = text.find("platforms = [").expect("app.platforms");
	let end = platforms
		+ text[platforms..]
			.find(']')
			.expect("the end of app.platforms")
		+ 1;
	let text = format!(
		"{}platforms = [\"web\", \"windows\"]{}",
		&text[..platforms],
		&text[end..]
	)
	.replace("render = \"ssr\"", "render = \"csr\"");
	std::fs::write(&config, text).expect("write leptoskit.toml");
	let mut dev = DevSession::start(&project, &["dev", "--port", &port]);
	dev.expect("[windows] skipped", Duration::from_mins(1));

	dev.expect("[web] serving", Duration::from_mins(15));
	dev.expect("watching", Duration::from_mins(1));
	assert!(page(&port).contains("<html"), "dev serves web/index.html");
	dev.stop();
}

/// A running `leptoskit dev`, read line by line.
#[cfg(unix)]
struct DevSession {
	child: std::process::Child,
	output: std::sync::mpsc::Receiver<String>,
	seen: Vec<String>,
}

#[cfg(unix)]
impl DevSession {
	fn start(project: &Path, args: &[&str]) -> Self {
		use std::io::BufRead as _;
		use std::process::Stdio;

		let mut child = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
			.current_dir(project)
			.args(args)
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.expect("start leptoskit dev");
		// Both streams feed one channel, so a failure shows the build's
		// errors beside `dev`'s own lines.
		let (lines, output) = std::sync::mpsc::channel();
		let streams: [Box<dyn std::io::Read + Send>; 2] = [
			Box::new(child.stdout.take().expect("piped stdout")),
			Box::new(child.stderr.take().expect("piped stderr")),
		];

		for stream in streams {
			let lines = lines.clone();
			std::thread::spawn(move || {
				for line in std::io::BufReader::new(stream)
					.lines()
					.map_while(Result::ok)
				{
					let _ = lines.send(line);
				}
			});
		}

		Self {
			child,
			output,
			seen: Vec::new(),
		}
	}

	/// Waits for a line starting with `prefix`.
	fn expect(&mut self, prefix: &str, within: std::time::Duration) {
		let deadline = std::time::Instant::now() + within;

		while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
			match self.output.recv_timeout(left) {
				Ok(line) if line.starts_with(prefix) => return,
				Ok(line) => self.seen.push(line),
				Err(_) => break,
			}
		}

		panic!(
			"`leptoskit dev` never printed `{prefix}`; it printed:\n{}",
			self.seen.join("\n")
		);
	}

	/// Sends `SIGTERM` and requires a clean exit.
	fn stop(mut self) {
		let terminated = Command::new("kill")
			.args(["-TERM", &self.child.id().to_string()])
			.status()
			.expect("run kill");
		assert!(terminated.success());
		let status = self.child.wait().expect("wait for leptoskit dev");
		self.seen.extend(self.output.try_iter());
		assert!(
			status.success(),
			"`leptoskit dev` exits cleanly on SIGTERM: {status}\n{}",
			self.seen.join("\n")
		);
	}
}

/// `GET /` from the dev server on `port`, retried while the server starts.
#[cfg(unix)]
fn page(port: &str) -> String {
	use std::io::Read as _;
	use std::io::Write as _;
	use std::time::Duration;
	use std::time::Instant;

	let deadline = Instant::now() + Duration::from_mins(1);

	loop {
		let response =
			std::net::TcpStream::connect(format!("127.0.0.1:{port}")).and_then(|mut stream| {
				stream
					.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
				let mut response = String::new();
				stream.read_to_string(&mut response)?;
				Ok(response)
			});

		match response {
			Ok(response) if response.starts_with("HTTP/1.1 200") => return response,

			_ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(200)),
			other => panic!("the dev server never served the page: {other:?}"),
		}
	}
}

/// `dev` checks `serve.address` before it builds anything.
#[test]
fn dev_rejects_a_serve_address_that_is_not_an_ip_before_building() {
	let scratch = Scratch::new("dev-address");
	std::fs::write(
		scratch.path.join(leptoskit_config::CONFIG_FILE),
		"[app]\nname = \"counter\"\n\n[serve]\naddress = \"localhost\"\n",
	)
	.unwrap();
	let output = leptoskit(&scratch.path, &["dev", "--platform", "web"]);
	assert!(!output.status.success());
	assert!(
		stderr(&output).contains("invalid serve.address `localhost`"),
		"{}",
		stderr(&output)
	);
}

#[test]
fn new_with_an_sdk_path_depends_on_its_crates() {
	let scratch = Scratch::new("sdk-scaffold");
	let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
	let created = leptoskit(
		&scratch.path,
		&["new", "app", "--sdk-path", sdk.to_str().expect("UTF-8")],
	);
	assert!(created.status.success(), "{}", stderr(&created));

	let manifest: toml::Table =
		std::fs::read_to_string(scratch.path.join("app/app_app/Cargo.toml"))
			.expect("client manifest")
			.parse()
			.expect("valid TOML");
	let leptoskit = &manifest["dependencies"]["leptoskit"];
	assert_eq!(
		Path::new(leptoskit["path"].as_str().expect("a path dependency")),
		std::fs::canonicalize(sdk.join("crates/leptoskit")).expect("SDK crate")
	);
	assert_eq!(
		leptoskit["version"].as_str(),
		Some(format!("={}", env!("CARGO_PKG_VERSION")).as_str())
	);
	assert_eq!(
		manifest["features"]["web"].as_array().expect("web feature")[0].as_str(),
		Some("leptoskit/web")
	);
	assert_eq!(
		manifest["features"]["ios"].as_array().expect("ios feature")[1].as_str(),
		Some("leptoskit/ios")
	);
	assert_eq!(
		Path::new(
			manifest["build-dependencies"]["leptoskit_build"]["path"]
				.as_str()
				.expect("helper path")
		),
		std::fs::canonicalize(sdk.join("crates/leptoskit_build")).expect("SDK build helper")
	);
}

#[test]
fn new_rejects_an_sdk_path_without_leptoskit() {
	let scratch = Scratch::new("sdk-missing");
	let sdk = scratch.path.to_str().expect("UTF-8").to_owned();
	let refused = leptoskit(&scratch.path, &["new", "app", "--sdk-path", &sdk]);

	assert!(!refused.status.success());
	assert!(
		stderr(&refused).contains("has no crates/leptoskit"),
		"{}",
		stderr(&refused)
	);
	assert!(
		!scratch.path.join("app").exists(),
		"nothing is written for a bad SDK"
	);
}

/// The public key `keygen` printed for `codepush.public_keys`.
fn printed_public_key(keygen: &Output) -> String {
	let out = stdout(keygen);
	let line = out
		.lines()
		.find(|line| line.starts_with("public_keys = "))
		.unwrap_or_else(|| panic!("keygen prints the public key line:\n{out}"));
	line.trim_start_matches("public_keys = [\"")
		.trim_end_matches("\"]")
		.to_owned()
}

#[test]
fn codepush_keygen_writes_a_private_key_once() {
	let scratch = Scratch::new("keygen");
	let keygen = leptoskit(
		&scratch.path,
		&["codepush", "keygen", "--out", "publisher.key"],
	);
	assert!(keygen.status.success(), "{}", stderr(&keygen));
	let secret = std::fs::read_to_string(scratch.path.join("publisher.key")).expect("key file");
	assert_eq!(secret.trim().len(), 64);
	assert!(
		!stdout(&keygen).contains(secret.trim()),
		"keygen must never print the secret"
	);
	assert_eq!(printed_public_key(&keygen).len(), 64);
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		let mode = std::fs::metadata(scratch.path.join("publisher.key"))
			.expect("metadata")
			.permissions()
			.mode();
		assert_eq!(mode & 0o777, 0o600, "only the owner may read the key");
	}

	let again = leptoskit(
		&scratch.path,
		&["codepush", "keygen", "--out", "publisher.key"],
	);
	assert!(
		!again.status.success(),
		"an existing key must not be replaced"
	);
	assert_eq!(
		std::fs::read_to_string(scratch.path.join("publisher.key")).expect("key file"),
		secret
	);
}

#[test]
fn codepush_keygen_rejects_invalid_outputs_without_printing_a_public_key() {
	let scratch = Scratch::new("keygen-invalid-output");

	for out in ["missing/publisher.key", "."] {
		let keygen = leptoskit(&scratch.path, &["codepush", "keygen", "--out", out]);
		assert!(!keygen.status.success());
		assert!(
			stderr(&keygen).contains("could not create"),
			"{}",
			stderr(&keygen)
		);
		assert!(!stdout(&keygen).contains("public_keys ="));
	}

	assert!(!scratch.path.join("missing").exists());
}

/// Uses Windows' independent ACL reader to check the CLI's security descriptor.
#[cfg(windows)]
fn keygen_powershell(dir: &Path, script: &str) {
	// Rebuild Windows PowerShell's module path when a PowerShell 7 parent
	// launched this test process.
	let output = Command::new("powershell.exe")
		.args(["-NoProfile", "-NonInteractive", "-Command", script])
		.env_remove("PSModulePath")
		.current_dir(dir)
		.output()
		.expect("run Windows ACL check");
	assert!(output.status.success(), "{}", stderr(&output));
}

#[cfg(windows)]
#[test]
fn codepush_keygen_does_not_inherit_a_shared_directory_acl() {
	let scratch = Scratch::new("keygen-shared-parent");
	keygen_powershell(
		&scratch.path,
		r"
$ErrorActionPreference = 'Stop'
$everyone = [System.Security.Principal.SecurityIdentifier]::new('S-1-1-0')
$acl = Get-Acl .
$rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
    $everyone, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
$acl.AddAccessRule($rule)
Set-Acl . $acl
Set-Content control.txt 'public'
$inherited = (Get-Acl control.txt).GetAccessRules(
    $true, $true, [System.Security.Principal.SecurityIdentifier]) |
    Where-Object { $_.IdentityReference -eq $everyone -and $_.IsInherited }
if (-not $inherited) { throw 'control file must inherit Everyone access' }
",
	);
	let keygen = leptoskit(
		&scratch.path,
		&["codepush", "keygen", "--out", "publisher.key"],
	);
	assert!(keygen.status.success(), "{}", stderr(&keygen));
	keygen_powershell(
		&scratch.path,
		r"
$ErrorActionPreference = 'Stop'
$user = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$acl = Get-Acl publisher.key
if (-not $acl.AreAccessRulesProtected) { throw 'key inherits its parent ACL' }
if ($acl.GetOwner([System.Security.Principal.SecurityIdentifier]) -ne $user) {
    throw 'key must belong to the current user'
}
$rules = @($acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]))
if ($rules.Count -ne 1 -or $rules[0].IdentityReference -ne $user -or
    $rules[0].IsInherited -or $rules[0].AccessControlType -ne 'Allow' -or
    $rules[0].FileSystemRights -ne 'FullControl') {
    throw 'key must grant full control only to the current user'
}
",
	);
}

#[cfg(windows)]
#[test]
fn codepush_keygen_fails_without_leaving_a_key_when_creation_is_denied() {
	let scratch = Scratch::new("keygen-denied-parent");
	keygen_powershell(
		&scratch.path,
		r"
$ErrorActionPreference = 'Stop'
$user = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$acl = Get-Acl .
$rule = [System.Security.AccessControl.FileSystemAccessRule]::new($user, 'CreateFiles', 'Deny')
$acl.AddAccessRule($rule)
Set-Acl . $acl
",
	);
	let keygen = leptoskit(
		&scratch.path,
		&["codepush", "keygen", "--out", "publisher.key"],
	);
	assert!(!keygen.status.success());
	assert!(
		stderr(&keygen).contains("could not create"),
		"{}",
		stderr(&keygen)
	);
	assert!(!stdout(&keygen).contains("public_keys ="));
	assert!(!scratch.path.join("publisher.key").exists());
}

#[test]
fn codepush_release_requires_a_trusted_signing_key() {
	let scratch = Scratch::new("codepush-signing");
	let keygen = leptoskit(
		&scratch.path,
		&["codepush", "keygen", "--out", "publisher.key"],
	);
	assert!(keygen.status.success(), "{}", stderr(&keygen));
	let public_key = printed_public_key(&keygen);
	let secret = std::fs::read_to_string(scratch.path.join("publisher.key")).expect("key file");

	let other_key = "a".repeat(64);
	codepush_project_trusting(&scratch.path, "0.1.0", &[public_key.as_str()]);
	let unsigned = leptoskit(&scratch.path, &["codepush", "release"]);
	assert!(!unsigned.status.success(), "an unsigned release must fail");
	assert!(
		stderr(&unsigned).contains("releases must be signed"),
		"{}",
		stderr(&unsigned)
	);

	let malformed = "not-a-key-but-maybe-a-secret";
	let bad = leptoskit_with_key_env(&scratch.path, &["codepush", "release"], Some(malformed));
	assert!(!bad.status.success());
	assert!(
		!stderr(&bad).contains(malformed),
		"errors must not quote the key: {}",
		stderr(&bad)
	);

	codepush_project_trusting(&scratch.path, "0.1.0", &[other_key.as_str()]);
	let untrusted = leptoskit_with_key_env(&scratch.path, &["codepush", "release"], Some(&secret));
	assert!(
		!untrusted.status.success(),
		"an untrusted key must not release"
	);
	assert!(
		stderr(&untrusted).contains("is not in codepush.public_keys"),
		"{}",
		stderr(&untrusted)
	);

	codepush_project_trusting(
		&scratch.path,
		"0.1.0",
		&[other_key.as_str(), public_key.as_str()],
	);
	let signed = leptoskit_with_key_env(&scratch.path, &["codepush", "release"], Some(&secret));
	assert!(signed.status.success(), "{}", stderr(&signed));
	assert!(
		stdout(&signed).contains(&format!("signed by {public_key}")),
		"{}",
		stdout(&signed)
	);
	assert!(
		!stdout(&signed).contains(secret.trim()),
		"release must never print the secret"
	);
}

#[test]
fn native_codepush_platforms_keep_separate_indexes_and_matching_clients() {
	use leptoskit_codepush::ReleaseSigningKey;
	use leptoskit_codepush::ReleaseStore;

	let scratch = Scratch::new("codepush-platforms");
	let signer = ReleaseSigningKey::from_seed(&[23; 32]);
	let public = signer.public_key().to_hex();
	let secret = signer.to_hex();
	codepush_project_trusting(&scratch.path, "0.2.0", &[&public]);
	write_platform_bundles(&scratch.path);
	assert_bundle_release_refuses_other_platforms(&scratch.path);

	for platform in ["ios", "android"] {
		let source = format!("bundles/{platform}");
		let release = leptoskit_with_key_env(
			&scratch.path,
			&[
				"codepush",
				"release",
				"--from",
				&source,
				"--platform",
				platform,
			],
			Some(&secret),
		);
		assert!(release.status.success(), "{}", stderr(&release));
		assert!(stdout(&release).contains(&format!("on production-{platform}")));
	}

	let registry = scratch.path.join("releases");
	let store = ReleaseStore::open(&registry).unwrap();
	let mut offers = Vec::new();

	for platform in ["ios", "android"] {
		offers.push(assert_platform_release(
			&scratch.path,
			&store,
			&registry,
			&public,
			platform,
		));
	}

	let client = store_client(registry, "production-ios", &public);
	client.verify_offer(&offers[0]).unwrap();
	assert!(client.verify_offer(&offers[1]).is_err());
	assert!(store.index("pushed", "production").unwrap().is_none());
	let web = leptoskit_with_key_env(
		&scratch.path,
		&["codepush", "release", "--platform", "web"],
		Some(&secret),
	);
	assert!(web.status.success(), "{}", stderr(&web));
	assert!(store.index("pushed", "production").unwrap().is_some());

	assert_platform_channels_round_trip_through_a_bucket(&scratch.path);
}

/// Serves a release store's files at `https://releases.example/`, so a code
/// push client reads exactly what the CLI released.
struct StoreTransport(PathBuf);

impl leptoskit_codepush::Transport for StoreTransport {
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, leptoskit_codepush::CodePushError> {
		let relative = url
			.strip_prefix("https://releases.example/")
			.expect("the test release registry");
		let path = url::Url::from_directory_path(&self.0)
			.expect("the release directory")
			.join(relative)
			.expect("the release URL")
			.to_file_path()
			.expect("the local release path");
		std::fs::read(path).map_err(leptoskit_codepush::CodePushError::Io)
	}
}

/// A client running 0.1.0+0 on `channel` that trusts only `public`, reading
/// the release store at `registry`.
fn store_client(
	registry: PathBuf,
	channel: &str,
	public: &str,
) -> leptoskit_codepush::CodePushClient<StoreTransport> {
	leptoskit_codepush::CodePushClient::new(
		StoreTransport(registry),
		"https://releases.example",
		"pushed",
		channel,
		semver::Version::new(0, 1, 0),
		0,
		Vec::new(),
		leptoskit_codepush::TrustedKeys::from_hex([public]).unwrap(),
	)
}

/// Writes an app-owned runner and a native bundle for iOS and Android, each
/// bundle's runtime contract naming its own platform's shell.
fn write_platform_bundles(project: &Path) {
	for platform in ["ios", "android"] {
		let host = project.join(platform);
		let source = project.join("bundles").join(platform);
		std::fs::create_dir_all(&host).unwrap();
		std::fs::create_dir_all(&source).unwrap();
		std::fs::write(host.join("Cargo.toml"), "# app-owned platform runner").unwrap();
		std::fs::write(source.join("app.wasm"), b"\0asm\x01\0\0\0").unwrap();
		std::fs::write(
			source.join("runtime.json"),
			serde_json::to_vec(&serde_json::json!({
				"abi": leptoskit_wasm::ABI_VERSION,
				"shell": format!("mobile-v1/{platform}"),
				"plugins": [],
			}))
			.unwrap(),
		)
		.unwrap();
	}
}

/// Releasing the iOS bundle without `--platform`, or for any platform but
/// iOS, is refused before the signing key is read.
fn assert_bundle_release_refuses_other_platforms(project: &Path) {
	for extra in [
		vec![],
		vec!["--platform", "web"],
		vec!["--platform", "android"],
		vec!["--platform", "native"],
		vec!["--platform", "server"],
	] {
		let mut args = vec![
			"codepush",
			"release",
			"--from",
			"bundles/ios",
			"--signing-key",
			"must-not-read.key",
		];
		args.extend(extra);
		let refused = leptoskit(project, &args);
		assert!(!refused.status.success());
		assert!(
			stderr(&refused).contains("--platform"),
			"{}",
			stderr(&refused)
		);
		assert!(!stderr(&refused).contains("could not read the signing key"));
	}
}

/// Asserts `platform`'s own channel holds the 0.2.0+1 release, that a client
/// on that channel downloads and verifies it with the platform's runtime
/// contract, and that `codepush check` reports 0.2.0+1 current. Returns the
/// client's offer.
fn assert_platform_release(
	project: &Path,
	store: &leptoskit_codepush::ReleaseStore,
	registry: &Path,
	public: &str,
	platform: &str,
) -> Box<leptoskit_codepush::UpdateOffer> {
	let channel = format!("production-{platform}");
	let index = store.index("pushed", &channel).unwrap().unwrap();
	assert_eq!(index.manifest.version, semver::Version::new(0, 2, 0));
	assert_eq!(index.manifest.build, 1);
	let mut client = store_client(registry.to_path_buf(), &channel, public);
	let leptoskit_codepush::UpdatePlan::Available(offer) = client.check().unwrap() else {
		panic!("{platform} has no update")
	};
	assert_eq!(offer.manifest.channel, channel);
	let downloaded = client.download_verified().unwrap().unwrap();
	let parsed = leptoskit_codepush::read_container(&downloaded.container).unwrap();
	let runtime = &parsed
		.files
		.iter()
		.find(|(name, _)| name == "runtime.json")
		.unwrap()
		.1;
	let runtime: leptoskit_codepush::installed::RuntimeContract =
		serde_json::from_slice(runtime).unwrap();
	assert_eq!(runtime.shell, format!("mobile-v1/{platform}"));
	let checked = leptoskit(
		project,
		&[
			"codepush",
			"check",
			"--platform",
			platform,
			"--version",
			"0.2.0",
			"--build",
			"1",
		],
	);
	assert!(checked.status.success(), "{}", stderr(&checked));
	assert!(stdout(&checked).contains(&format!("on {channel}: up to date")));
	offer
}

/// Publishes each platform's channel to a mock bucket and pulls it back into
/// an empty store, which then holds that channel's index.
fn assert_platform_channels_round_trip_through_a_bucket(project: &Path) {
	let (endpoint, objects) = start_mock_s3();
	let credentials = [
		(ACCESS_KEY_ID_ENV, MOCK_ACCESS_KEY),
		(SECRET_ACCESS_KEY_ENV, "synthetic-platform-test-key"),
	];

	for platform in ["ios", "android"] {
		for (operation, store) in [("publish", "releases"), ("pull", "recovered")] {
			let result = leptoskit_with_env(
				project,
				&[
					"codepush",
					operation,
					"--platform",
					platform,
					"--store",
					store,
					"--endpoint",
					&endpoint,
					"--bucket",
					"registry",
					"--path-style",
				],
				&credentials,
			);
			assert!(
				result.status.success(),
				"{operation}/{platform}: {}",
				stderr(&result)
			);
		}

		let index = format!("/registry/pushed/production-{platform}/index.json");
		assert!(objects.lock().unwrap().objects.contains_key(&index));
		assert!(
			leptoskit_codepush::ReleaseStore::open(project.join("recovered"))
				.unwrap()
				.index("pushed", &format!("production-{platform}"))
				.unwrap()
				.is_some()
		);
	}
}

#[test]
fn codepush_rejects_disabled_platforms_before_reading_keys_or_bucket_credentials() {
	let scratch = Scratch::new("codepush-disabled-platform");
	codepush_project(&scratch.path, "0.1.0");
	let mut config = leptoskit_config::LeptoskitConfig::load_from(&scratch.path).unwrap();
	config.app.platforms = vec![leptoskit_config::Platform::Web];
	std::fs::write(
		scratch.path.join("leptoskit.toml"),
		config.to_toml_string().unwrap(),
	)
	.unwrap();

	for operation in ["release", "publish", "pull", "check"] {
		let mut args = vec!["codepush", operation, "--platform", "android"];

		if matches!(operation, "publish" | "pull") {
			args.extend(["--endpoint", "https://unused.example", "--bucket", "unused"]);
		}

		let refused = leptoskit(&scratch.path, &args);
		assert!(!refused.status.success());
		assert!(
			stderr(&refused).contains("platform `android` is disabled"),
			"{operation}: {}",
			stderr(&refused)
		);
	}
}

/// What the mock S3 server holds and saw.
#[derive(Default)]
struct MockS3 {
	/// Objects by decoded path (`/<bucket>/<key>`): body, content type, and
	/// cache control.
	objects: HashMap<String, (Vec<u8>, String, String)>,
	/// The decoded path of every successful PUT, in order.
	puts: Vec<String>,
	/// Every request line and header, raw, to check nothing leaks a secret.
	raw_requests: Vec<String>,
}

/// The access key id the mock S3 server accepts.
const MOCK_ACCESS_KEY: &str = "test-access-key";

/// Starts an in-process S3 stand-in on a free port: it accepts presigned
/// GETs and PUTs signed for [`MOCK_ACCESS_KEY`] and stores objects in memory.
fn start_mock_s3() -> (String, Arc<Mutex<MockS3>>) {
	let state = Arc::new(Mutex::new(MockS3::default()));
	let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
	listener.set_nonblocking(true).expect("nonblocking");
	let address = listener.local_addr().expect("address");
	let shared = Arc::clone(&state);
	std::thread::spawn(move || {
		let runtime = tokio::runtime::Runtime::new().expect("runtime");
		runtime.block_on(async move {
			let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
			let router = axum::Router::new().fallback(move |request: axum::extract::Request| {
				mock_s3_request(Arc::clone(&shared), request)
			});
			axum::serve(listener, router).await.expect("serve");
		});
	});
	(format!("http://{address}"), state)
}

async fn mock_s3_request(
	state: Arc<Mutex<MockS3>>,
	request: axum::extract::Request,
) -> axum::response::Response {
	use axum::http::StatusCode;
	use axum::response::IntoResponse;

	let (parts, body) = request.into_parts();
	let body = axum::body::to_bytes(body, usize::MAX).await.expect("body");
	let header = |name: &str| {
		parts
			.headers
			.get(name)
			.map(|value| value.to_str().expect("ascii").to_owned())
			.unwrap_or_default()
	};
	let mut state = state.lock().expect("lock");
	state.raw_requests.push(format!(
		"{} {} {:?}",
		parts.method, parts.uri, parts.headers
	));
	let query = parts.uri.query().unwrap_or_default();
	let signed = query.contains("X-Amz-Signature=")
		&& query.contains(&format!("X-Amz-Credential={MOCK_ACCESS_KEY}"));

	if !signed {
		return StatusCode::FORBIDDEN.into_response();
	}

	let path = percent_decode(parts.uri.path());

	match parts.method.as_str() {
		"GET" => {
			match state.objects.get(&path) {
				Some((bytes, ..)) => bytes.clone().into_response(),
				None => StatusCode::NOT_FOUND.into_response(),
			}
		}
		"PUT" => {
			let object = (
				body.to_vec(),
				header("content-type"),
				header("cache-control"),
			);
			state.objects.insert(path.clone(), object);
			state.puts.push(path);
			StatusCode::OK.into_response()
		}
		_ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
	}
}

fn percent_decode(path: &str) -> String {
	let mut decoded = Vec::new();
	let mut bytes = path.bytes();

	while let Some(byte) = bytes.next() {
		if byte == b'%' {
			let hex: String = bytes.by_ref().take(2).map(char::from).collect();
			decoded.push(u8::from_str_radix(&hex, 16).expect("percent escape"));
		} else {
			decoded.push(byte);
		}
	}

	String::from_utf8(decoded).expect("utf-8 path")
}

/// The bucket key every file of the store's `pushed/production` channel is
/// published under, sorted.
fn published_keys(store: &Path) -> Vec<String> {
	let mut keys: Vec<String> = walkdir::WalkDir::new(store.join("pushed/production"))
		.into_iter()
		.map(|entry| entry.expect("walk"))
		.filter(|entry| entry.file_type().is_file())
		// The publisher's hash sidecars are not part of the public layout.
		.filter(|entry| entry.file_name() != "release.sha256")
		.map(|entry| {
			let relative = entry.path().strip_prefix(store).expect("under the store");
			// Bucket keys are `/`-separated whatever the platform's separator.
			let key: Vec<_> = relative
				.components()
				.map(|component| component.as_os_str().to_string_lossy())
				.collect();
			format!("/registry/updates/{}", key.join("/"))
		})
		.collect();
	keys.sort();
	keys
}

/// The content type and cache control a published object carries: the index
/// is short-lived, while releases, signatures, and deltas are immutable.
fn expected_object_headers(key: &str) -> (&'static str, &'static str) {
	if key.ends_with("index.json") {
		("application/json", "public, max-age=60")
	} else if key.ends_with("/release.lkb") {
		(
			"application/octet-stream",
			"public, max-age=31536000, immutable",
		)
	} else {
		("application/json", "public, max-age=31536000, immutable")
	}
}

/// Asserts the bucket holds exactly the store's channel, written index last,
/// with every object's bytes and headers matching, and that no request
/// carried `secret`.
fn assert_bucket_mirrors_store(state: &MockS3, store: &Path, secret: &str) {
	let local = published_keys(store);
	assert_eq!(
		state.puts.last().map(String::as_str),
		Some("/registry/updates/pushed/production/index.json"),
		"the index is written last: {:?}",
		state.puts
	);
	let mut uploaded = state.puts.clone();
	uploaded.sort();
	assert!(uploaded.iter().all(|key| !key.ends_with("/release.sha256")));
	assert_eq!(uploaded, local, "the bucket mirrors the store's channel");
	assert_eq!(
		local.len(),
		6,
		"two releases, two signatures, a delta, the index"
	);

	for path in &local {
		let file = store.join(path.trim_start_matches("/registry/updates/"));
		let (bytes, content_type, cache_control) = &state.objects[path];
		assert_eq!(bytes, &std::fs::read(&file).expect("local file"), "{path}");
		let (expected_type, expected_cache) = expected_object_headers(path);
		assert_eq!(content_type, expected_type, "{path}");
		assert_eq!(cache_control, expected_cache, "{path}");
	}

	for request in &state.raw_requests {
		assert!(!request.contains(secret), "a request leaks the secret");
	}
}

#[test]
fn codepush_publish_uploads_the_static_layout_with_the_index_last() {
	let scratch = Scratch::new("codepush-publish");
	let keygen = leptoskit(
		&scratch.path,
		&["codepush", "keygen", "--out", "publisher.key"],
	);
	assert!(keygen.status.success(), "{}", stderr(&keygen));
	codepush_project(&scratch.path, "0.2.0");
	let release = ["codepush", "release", "--signing-key", "publisher.key"];

	for marker in ["one", "two"] {
		std::fs::write(scratch.path.join("dist/index.html"), marker).expect("index");
		let output = leptoskit(&scratch.path, &release);

		assert!(output.status.success(), "{}", stderr(&output));
	}

	let (endpoint, state) = start_mock_s3();
	let publish = [
		"codepush",
		"publish",
		"--endpoint",
		&endpoint,
		"--bucket",
		"registry",
		"--prefix",
		"/updates/",
		"--path-style",
	];
	let secret = "mock-secret-access-key-never-printed";
	let credentials = [
		(ACCESS_KEY_ID_ENV, MOCK_ACCESS_KEY),
		(SECRET_ACCESS_KEY_ENV, secret),
	];

	let missing = leptoskit_with_env(&scratch.path, &publish, &[]);
	assert!(!missing.status.success(), "publishing needs credentials");
	assert!(
		stderr(&missing).contains(ACCESS_KEY_ID_ENV),
		"{}",
		stderr(&missing)
	);

	let first = leptoskit_with_env(&scratch.path, &publish, &credentials);
	assert!(first.status.success(), "{}", stderr(&first));
	assert!(
		stdout(&first).contains("published pushed 0.2.0+2 on production"),
		"{}",
		stdout(&first)
	);

	let store = scratch.path.join("releases");
	assert_bucket_mirrors_store(&state.lock().expect("lock"), &store, secret);

	for output in [&missing, &first] {
		assert!(!stdout(output).contains(secret) && !stderr(output).contains(secret));
	}

	let again = leptoskit_with_env(&scratch.path, &publish, &credentials);
	assert!(again.status.success(), "{}", stderr(&again));
	assert!(
		stdout(&again).contains("no new releases; 0 objects written"),
		"{}",
		stdout(&again)
	);
	assert_eq!(state.lock().expect("lock").puts.len(), 6);

	let wrong_key = [
		(ACCESS_KEY_ID_ENV, "someone-else"),
		(SECRET_ACCESS_KEY_ENV, secret),
	];
	let rejected = leptoskit_with_env(&scratch.path, &publish, &wrong_key);
	assert!(!rejected.status.success());
	assert!(
		stderr(&rejected).contains("HTTP 403"),
		"{}",
		stderr(&rejected)
	);
	assert!(
		!stderr(&rejected).contains("X-Amz-Signature"),
		"{}",
		stderr(&rejected)
	);
}

#[test]
fn codepush_pull_lets_a_runner_with_an_empty_store_release_the_next_build() {
	let (endpoint, state) = start_mock_s3();
	let bucket_flags = [
		"--endpoint",
		endpoint.as_str(),
		"--bucket",
		"registry",
		"--path-style",
	];
	let command = |verb: &'static str| {
		let mut args = vec!["codepush", verb];
		args.extend(bucket_flags);
		args
	};
	let credentials = [
		(ACCESS_KEY_ID_ENV, MOCK_ACCESS_KEY),
		(SECRET_ACCESS_KEY_ENV, "mock-secret"),
	];

	// The key lives outside both runners, like a CI secret.
	let keys = Scratch::new("codepush-pull-key");
	let keygen = leptoskit(
		&keys.path,
		&["codepush", "keygen", "--out", "publisher.key"],
	);
	assert!(keygen.status.success(), "{}", stderr(&keygen));
	let public_key = printed_public_key(&keygen);
	let secret = std::fs::read_to_string(keys.path.join("publisher.key")).expect("key");
	let mut signed = credentials.to_vec();
	signed.push((SIGNING_KEY_ENV, secret.trim()));

	// Each run starts from a fresh checkout: no `releases/` on disk.
	let run_ci = |marker: &str| {
		let runner = Scratch::new("codepush-pull-runner");
		codepush_project_trusting(&runner.path, "0.3.0", &[public_key.as_str()]);
		std::fs::write(runner.path.join("dist/index.html"), marker).expect("index");
		let mut outputs = Vec::new();
		for args in [
			command("pull"),
			vec!["codepush", "release"],
			command("publish"),
		] {
			let output = leptoskit_with_env(&runner.path, &args, &signed);
			assert!(output.status.success(), "{args:?}: {}", stderr(&output));
			outputs.push(stdout(&output));
		}
		outputs
	};

	let first = run_ci("first");
	assert!(
		first[0].contains("no pushed releases on production yet"),
		"{}",
		first[0]
	);
	assert!(
		first[1].contains("0.3.0+1") && first[1].contains("full bundle"),
		"{}",
		first[1]
	);

	let second = run_ci("second, with a change");
	assert!(
		second[0].contains("pushed 0.3.0+1 on production from registry: pulled"),
		"{}",
		second[0]
	);
	assert!(
		second[1].contains("0.3.0+2") && second[1].contains("ops"),
		"the second build is numbered after, and patches, the pulled one: {}",
		second[1]
	);
	assert!(second[2].contains("uploaded 0.3.0+2"), "{}", second[2]);
	let state = state.lock().expect("lock");
	assert_eq!(
		state.puts.last().map(String::as_str),
		Some("/registry/pushed/production/index.json")
	);
	assert!(
		state
			.puts
			.iter()
			.any(|path| path.starts_with("/registry/pushed/production/0.3.0+2/deltas/")),
		"the delta from 0.3.0+1 is published: {:?}",
		state.puts
	);
	drop(state);

	// A runner that trusts no key cannot pull.
	let untrusting = Scratch::new("codepush-pull-untrusting");
	codepush_project(&untrusting.path, "0.3.0");
	let refused = leptoskit_with_env(&untrusting.path, &command("pull"), &credentials);
	assert!(!refused.status.success());
	assert!(
		stderr(&refused).contains("codepush.public_keys"),
		"{}",
		stderr(&refused)
	);
}

/// `leptoskit format` checks and writes like the formatter it embeds, and
/// fails a check that finds files to format.
#[test]
fn format_checks_then_writes() {
	let scratch = Scratch::new("format");
	let unformatted = "fn app() -> impl IntoView {\nview! {   <p>\"hi\"</p>   }\n}\n";
	std::fs::write(
		scratch.path.join("rustfmt.toml"),
		"edition = \"2024\"\nhard_tabs = true\n",
	)
	.expect("rustfmt.toml");
	std::fs::write(scratch.path.join("lib.rs"), unformatted).expect("lib.rs");
	// rustup's rustfmt, for the toolchain cargo runs the tests with.
	let env = [("LEPTOSKIT_RUSTFMT", "rustfmt")];

	let check = leptoskit_with_env(&scratch.path, &["format", "--check", "--no-cache"], &env);
	assert!(!check.status.success());
	assert!(
		stderr(&check).contains("1 file(s) would be reformatted; run `leptoskit format`"),
		"{}",
		stderr(&check)
	);

	let write = leptoskit_with_env(&scratch.path, &["format", "--no-cache"], &env);
	assert!(write.status.success(), "{}", stderr(&write));
	assert_eq!(
		std::fs::read_to_string(scratch.path.join("lib.rs")).expect("read"),
		"fn app() -> impl IntoView {\n\tview! { <p>\"hi\"</p> }\n}\n"
	);

	let failed = leptoskit_with_env(&scratch.path, &["format", "--no-cache", "missing.rs"], &env);
	assert!(
		stderr(&failed).contains("error: formatting failed"),
		"{}",
		stderr(&failed)
	);
}

#[test]
fn routes_writes_checks_and_reports_problems_by_file_and_line() {
	let scratch = Scratch::new("routes");
	let dir = &scratch.path;
	std::fs::write(dir.join("leptoskit.toml"), "[router]\n").expect("config");
	std::fs::create_dir_all(dir.join("src/routes/notes")).expect("routes dir");
	std::fs::write(dir.join("src/routes/index.rs"), "pub fn page() {}\n").expect("index");
	std::fs::write(
		dir.join("src/routes/notes/[note].rs"),
		"#[derive(Route)]\npub struct Note {\n\tpub note: u64,\n}\n\n#[component]\npub fn Page() \
		 {}\n",
	)
	.expect("note");

	let output = leptoskit(dir, &["routes", "--check"]);
	assert!(!output.status.success(), "a missing module is not current");
	assert!(
		stderr(&output).contains("out of date"),
		"{}",
		stderr(&output)
	);

	let output = leptoskit(dir, &["routes"]);
	assert!(output.status.success(), "{}", stderr(&output));
	let listed = stdout(&output);
	assert!(listed.contains("wrote src/routes.rs"), "{listed}");
	assert!(listed.contains("/notes/:note"), "{listed}");
	assert!(listed.contains("Note (component)"), "{listed}");
	let module = std::fs::read_to_string(dir.join("src/routes.rs")).expect("generated");
	assert!(module.contains("#[path = \"routes/notes\"]"), "{module}");

	let output = leptoskit(dir, &["routes", "--check"]);
	assert!(output.status.success(), "{}", stderr(&output));
	assert!(stdout(&output).contains("src/routes.rs is up to date"));

	// A not-found route is listed as one, and running again writes nothing.
	std::fs::write(dir.join("src/routes/+not-found.rs"), "pub fn page() {}\n").expect("404");
	assert!(leptoskit(dir, &["routes"]).status.success());
	let output = leptoskit(dir, &["routes"]);
	assert!(output.status.success(), "{}", stderr(&output));
	assert!(stdout(&output).contains("src/routes.rs is up to date"));
	assert!(stdout(&output).contains("NotFound (not found)"));

	// A name problem has no line; a file problem has one.
	std::fs::write(dir.join("src/routes/feed.web.rs"), "pub fn page() {}\n").expect("feed");
	let output = leptoskit(dir, &["routes"]);
	assert!(
		stderr(&output).contains("src/routes/feed.web.rs: `feed.web` is a platform override"),
		"{}",
		stderr(&output)
	);
	std::fs::remove_file(dir.join("src/routes/feed.web.rs")).expect("remove feed");
	std::fs::write(dir.join("src/routes/about.rs"), "pub fn Page() {}\n").expect("about");
	let output = leptoskit(dir, &["routes"]);
	assert!(!output.status.success(), "an unannotated component fails");
	assert!(
		stderr(&output).contains("src/routes/about.rs:1: `Page` is capitalised"),
		"{}",
		stderr(&output)
	);

	std::fs::write(dir.join("leptoskit.toml"), "").expect("config");
	let output = leptoskit(dir, &["routes"]);
	assert!(
		!output.status.success(),
		"the router is off without a router plugin"
	);
	assert!(
		stderr(&output).contains("naming `router`"),
		"{}",
		stderr(&output)
	);
}

#[test]
fn plugin_router_routes_answers_under_its_namespace() {
	let scratch = Scratch::new("plugin_router");
	let dir = &scratch.path;
	std::fs::write(
		dir.join("leptoskit.toml"),
		"[[plugins]]\nname = \"router\"\n",
	)
	.expect("config");
	std::fs::create_dir_all(dir.join("src/routes")).expect("routes dir");
	std::fs::write(dir.join("src/routes/index.rs"), "pub fn page() {}\n").expect("index");

	let output = leptoskit(dir, &["plugin:router", "routes"]);
	assert!(output.status.success(), "{}", stderr(&output));
	let listed = stdout(&output);
	assert!(listed.contains("wrote src/routes.rs"), "{listed}");
	assert!(listed.contains("/  "), "{listed}");

	let output = leptoskit(dir, &["plugin:router", "routes", "--check"]);
	assert!(output.status.success(), "{}", stderr(&output));

	let output = leptoskit(dir, &["plugin:router", "dev"]);
	assert!(!output.status.success(), "undeclared commands fail");
	assert!(
		stderr(&output).contains("its commands are routes"),
		"{}",
		stderr(&output)
	);

	let output = leptoskit(dir, &["plugin:other", "routes"]);
	assert!(!output.status.success(), "unknown namespaces fail");
	assert!(
		stderr(&output).contains("no plugin with the namespace `other`"),
		"{}",
		stderr(&output)
	);

	let output = leptoskit(dir, &["plugin:router"]);
	assert!(output.status.success(), "{}", stderr(&output));
	let listed = stdout(&output);
	assert!(listed.contains("plugin:router commands"), "{listed}");
	assert!(listed.contains("plugin:router routes"), "{listed}");
	assert!(listed.contains("--check"), "{listed}");
}

#[test]
fn builds_sync_the_routes_module_only_when_the_router_is_on() {
	let scratch = Scratch::new("routes_sync");
	let root = &scratch.path;
	std::fs::create_dir_all(root.join("src/routes")).expect("routes dir");
	std::fs::write(root.join("src/routes/index.rs"), "pub fn page() {}\n").expect("index");
	let output = root.join("src/routes.rs");

	let off = leptoskit_config::LeptoskitConfig::default();
	leptoskit_cli::routes::sync(root, &off).expect("nothing to do");
	assert!(!output.exists(), "the router is off without [router]");

	let on = leptoskit_config::LeptoskitConfig {
		router: Some(leptoskit_config::RouterConfig::default()),
		..Default::default()
	};
	leptoskit_cli::routes::sync(root, &on).expect("writes the module");
	let written = std::fs::read_to_string(&output).expect("generated");
	assert!(
		written.contains("@generated by `leptoskit routes`"),
		"{written}"
	);

	// An unchanged tree leaves the file alone, so the dev watcher sees no
	// write.
	let before = std::fs::metadata(&output)
		.expect("metadata")
		.modified()
		.expect("mtime");
	leptoskit_cli::routes::sync(root, &on).expect("up to date");
	let after = std::fs::metadata(&output)
		.expect("metadata")
		.modified()
		.expect("mtime");
	assert_eq!(before, after);

	std::fs::write(root.join("src/routes/about.rs"), "pub fn Page() {}\n").expect("about");
	let error = leptoskit_cli::routes::sync(root, &on).expect_err("a broken route stops the build");
	assert!(
		error.to_string().contains("src/routes/about.rs:1:"),
		"{error:#}"
	);
}

#[test]
fn l10n_writes_checks_and_reports_problems_by_file_and_line() {
	let scratch = Scratch::new("l10n");
	let dir = &scratch.path;
	std::fs::write(dir.join("leptoskit.toml"), "[l10n]\n").expect("config");
	std::fs::create_dir_all(dir.join("locales")).expect("locales dir");
	std::fs::create_dir_all(dir.join("src")).expect("src dir");
	std::fs::write(
		dir.join("locales/en.arb"),
		"{\n\t\"appTitle\": \"Field Notes\",\n\t\"items\": \"{count, plural, one {# item} other \
		 {# items}}\"\n}\n",
	)
	.expect("en");
	std::fs::write(
		dir.join("locales/pt-BR.arb"),
		"{\n\t\"appTitle\": \"Notas de Campo\"\n}\n",
	)
	.expect("pt");

	let output = leptoskit(dir, &["l10n", "--check"]);
	assert!(!output.status.success(), "a missing module is not current");
	assert!(
		stderr(&output).contains("out of date"),
		"{}",
		stderr(&output)
	);

	let output = leptoskit(dir, &["l10n"]);
	assert!(output.status.success(), "{}", stderr(&output));
	let listed = stdout(&output);
	assert!(listed.contains("wrote src/l10n.rs"), "{listed}");
	assert!(
		listed.contains("en     en.arb     template, 2 messages"),
		"{listed}"
	);
	assert!(
		listed.contains("pt-BR  pt-BR.arb  1 of 2 translated; missing items"),
		"{listed}"
	);
	assert!(
		listed.contains("items     items(count: i64) -> String"),
		"{listed}"
	);
	let module = std::fs::read_to_string(dir.join("src/l10n.rs")).expect("generated");

	assert!(module.contains("impl Messages for PtBr"), "{module}");

	let output = leptoskit(dir, &["l10n", "--check"]);
	assert!(output.status.success(), "{}", stderr(&output));
	assert!(stdout(&output).contains("src/l10n.rs is up to date"));

	// A complete translation is listed as one, and running again writes
	// nothing.
	std::fs::write(
		dir.join("locales/pt-BR.arb"),
		"{\n\t\"appTitle\": \"Notas de Campo\",\n\t\"items\": \"{count, plural, one {# item} \
		 other {# itens}}\"\n}\n",
	)
	.expect("pt");
	assert!(leptoskit(dir, &["l10n"]).status.success());
	let output = leptoskit(dir, &["l10n"]);
	assert!(output.status.success(), "{}", stderr(&output));
	assert!(stdout(&output).contains("src/l10n.rs is up to date"));
	assert!(
		stdout(&output).contains("pt-BR  pt-BR.arb  complete"),
		"{}",
		stdout(&output)
	);

	// A problem is reported with its file and line.
	std::fs::write(
		dir.join("locales/pt-BR.arb"),
		"{\n\t\"appTitle\": \"{surprise}\"\n}\n",
	)
	.expect("pt");
	let output = leptoskit(dir, &["l10n"]);
	assert!(
		!output.status.success(),
		"a translation with its own placeholder fails"
	);
	assert!(
		stderr(&output).contains(
			"locales cannot become translations:\n  locales/pt-BR.arb:2: `appTitle` adds \
			 `{surprise}`"
		),
		"{}",
		stderr(&output)
	);

	std::fs::write(dir.join("leptoskit.toml"), "").expect("config");
	let output = leptoskit(dir, &["l10n"]);
	assert!(
		!output.status.success(),
		"translations are off without [l10n]"
	);
	assert!(
		stderr(&output).contains("add an `[l10n]` table"),
		"{}",
		stderr(&output)
	);
}

#[test]
fn builds_sync_the_translations_module_only_when_translations_are_on() {
	let scratch = Scratch::new("l10n_sync");
	let root = &scratch.path;
	std::fs::create_dir_all(root.join("locales")).expect("locales dir");
	std::fs::create_dir_all(root.join("src")).expect("src dir");
	std::fs::write(
		root.join("locales/en.arb"),
		"{\n\t\"appTitle\": \"Field Notes\"\n}\n",
	)
	.expect("en");
	let output = root.join("src/l10n.rs");

	let off = leptoskit_config::LeptoskitConfig::default();
	leptoskit_cli::l10n::sync(root, &off).expect("nothing to do");
	assert!(!output.exists(), "translations are off without [l10n]");

	let on = leptoskit_config::LeptoskitConfig {
		l10n: Some(leptoskit_config::L10nConfig::default()),
		..Default::default()
	};
	leptoskit_cli::l10n::sync(root, &on).expect("writes the module");
	let written = std::fs::read_to_string(&output).expect("generated");
	assert!(
		written.contains("@generated by `leptoskit l10n`"),
		"{written}"
	);

	// Unchanged messages leave the file alone, so the dev watcher sees no
	// write.
	let before = std::fs::metadata(&output)
		.expect("metadata")
		.modified()
		.expect("mtime");
	leptoskit_cli::l10n::sync(root, &on).expect("up to date");
	let after = std::fs::metadata(&output)
		.expect("metadata")
		.modified()
		.expect("mtime");
	assert_eq!(before, after);

	std::fs::write(root.join("locales/en.arb"), "{\n\t\"appTitle\": \"{\"\n}\n").expect("en");
	let error = leptoskit_cli::l10n::sync(root, &on).expect_err("a broken message stops the build");
	assert!(error.to_string().contains("locales/en.arb:2:"), "{error:#}");
}

#[test]
fn scaffolds_a_native_harmonyos_counter_without_apple_or_android_plugins() {
	use leptoskit_config::LeptoskitConfig;
	use leptoskit_config::NativeTarget;
	let scratch = Scratch::new("harmonyos");
	let destination = scratch.path.join("counter");
	let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args([
			"new",
			destination.to_str().unwrap(),
			"--platform",
			"native",
			"--native-target",
			"harmonyos",
		])
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let config = LeptoskitConfig::load_from(&destination).unwrap();
	assert_eq!(config.native.targets, [NativeTarget::Harmonyos]);
	assert_eq!(config.native.plugins, [] as [String; 0]);
	let manifest: toml::Value =
		toml::from_str(&std::fs::read_to_string(destination.join("Cargo.toml")).unwrap()).unwrap();
	assert!(
		manifest["dependencies"]
			.get("leptoskit_bridge_ffi")
			.is_some()
	);
	assert!(
		manifest["dependencies"]
			.get("leptoskit_plugin_clipboard")
			.is_none()
	);
	assert!(!destination.join("hosts/android").exists());
	assert!(!destination.join("hosts/ios").exists());
	for relative in [
		"hosts/harmonyos/entry/src/main/ets/pages/Index.ets",
		"hosts/harmonyos/entry/src/main/cpp/types/libleptoskit/index.d.ts",
		"hosts/harmonyos/AppScope/app.json5",
		"hosts/harmonyos/build-profile.json5",
		"hosts/harmonyos/TESTING.md",
		"hosts/harmonyos/entry/src/ohosTest/module.json5",
		"hosts/harmonyos/entry/src/ohosTest/ets/testrunner/OpenHarmonyTestRunner.ets",
		"hosts/harmonyos/entry/src/ohosTest/ets/test/Counter.test.ets",
	] {
		assert!(destination.join(relative).is_file(), "missing {relative}");
	}
	let app: serde_json::Value = serde_json::from_str(
		&std::fs::read_to_string(destination.join("hosts/harmonyos/AppScope/app.json5")).unwrap(),
	)
	.unwrap();
	assert_eq!(app["app"]["bundleName"], "com.leptoskit.counter");
	let test = std::fs::read_to_string(
		destination.join("hosts/harmonyos/entry/src/ohosTest/ets/test/Counter.test.ets"),
	)
	.unwrap();
	assert!(test.contains("bundleName: 'com.leptoskit.counter'"));
	assert!(!test.contains("__BUNDLE_NAME__"));
}

#[test]
fn native_target_requires_a_native_project_without_creating_files() {
	let scratch = Scratch::new("harmonyos-requires-native");
	let destination = scratch.path.join("web");
	let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args([
			"new",
			destination.to_str().unwrap(),
			"--native-target",
			"harmonyos",
		])
		.output()
		.unwrap();
	assert!(!output.status.success());
	assert!(String::from_utf8_lossy(&output.stderr).contains("--platform native"));
	assert!(!destination.exists());
}

#[test]
fn native_harmonyos_build_reports_missing_sdk_before_running_cargo() {
	use leptoskit_config::NativeTarget;
	let scratch = Scratch::new("harmonyos-sdk");
	let destination = scratch.path.join("counter");
	leptoskit_cli::native::scaffold::run_for_target(
		destination.to_str().unwrap(),
		None,
		None,
		Some(NativeTarget::Harmonyos),
	)
	.unwrap();

	for target in ["harmonyos", "harmonyos-x86_64"] {
		let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
			.current_dir(&destination)
			.env_remove("OHOS_NDK_HOME")
			.args(["native", "prepare", target])
			.output()
			.unwrap();
		assert!(!output.status.success());
		assert!(String::from_utf8_lossy(&output.stderr).contains("OHOS_NDK_HOME"));
		assert!(!destination.join(".leptoskit").exists());
	}

	let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.current_dir(&destination)
		.env_remove("OHOS_NDK_HOME")
		.arg("build")
		.output()
		.unwrap();
	assert!(String::from_utf8_lossy(&output.stderr).contains("OHOS_NDK_HOME"));
}

/// Exercise the public Cargo/Hvigor process contract without requiring an SDK
/// on every CI runner. The generated guest is separately compiled for real.
#[cfg(unix)]
#[test]
fn harmonyos_packaging_selects_the_abi_and_propagates_tool_failures() {
	use std::os::unix::fs::PermissionsExt as _;

	use leptoskit_config::NativeTarget;

	let scratch = Scratch::new("harmonyos-tool-contract");
	let app = scratch.path.join("counter");
	leptoskit_cli::native::scaffold::run_for_target(
		app.to_str().unwrap(),
		None,
		None,
		Some(NativeTarget::Harmonyos),
	)
	.unwrap();
	let tools = scratch.path.join("tools");
	let sdk = scratch.path.join("SDK's native tools");
	std::fs::create_dir_all(&tools).unwrap();
	std::fs::create_dir_all(sdk.join("llvm/bin")).unwrap();
	std::fs::create_dir_all(sdk.join("sysroot/usr/include")).unwrap();

	for tool in ["clang", "clang++", "llvm-ar"] {
		std::fs::write(sdk.join("llvm/bin").join(tool), "SDK tool fixture").unwrap();
	}

	let bridge = Path::new(env!("CARGO_MANIFEST_DIR"))
		.parent()
		.unwrap()
		.join("leptoskit_bridge_ffi/Cargo.toml");
	let metadata = serde_json::json!({
		"packages": [
			{"id": "counter", "name": "counter", "manifest_path": app.join("Cargo.toml"), "metadata": {}, "targets": []},
			{"id": "guest", "name": "counter_guest", "manifest_path": app.join("guest/Cargo.toml"), "metadata": {}, "targets": [{"name": "counter_guest", "kind": ["cdylib"], "crate_types": ["cdylib"], "src_path": app.join("guest/src/lib.rs")}]},
			{"id": "bridge", "name": "leptoskit_bridge_ffi", "manifest_path": bridge, "metadata": {}, "targets": []}
		],
		"workspace_members": ["counter", "guest"], "workspace_root": app,
		"resolve": {"nodes": [{"id":"counter", "deps":[{"name":"leptoskit_bridge_ffi", "pkg":"bridge"}]}]},
		"target_directory": app.join("target")
	});
	std::fs::write(
		app.join("metadata.json"),
		serde_json::to_vec(&metadata).unwrap(),
	)
	.unwrap();
	let guest = app.join("target/wasm32-unknown-unknown/release/counter_guest.wasm");
	std::fs::create_dir_all(guest.parent().unwrap()).unwrap();
	std::fs::write(&guest, b"\0asm\x01\0\0\0").unwrap();
	let cargo = tools.join("cargo");
	std::fs::write(
		&cargo,
		r#"#!/bin/sh
set -eu
if [ "$1" = metadata ]; then cat metadata.json; exit 0; fi
shift
profile=debug
target=
target_dir=target
while [ "$#" -gt 0 ]; do
  case "$1" in
    --target) target="$2"; shift;;
    --target-dir) target_dir="$2"; shift;;
    --release) profile=release;;
  esac
  shift
done
if [ "$target" = wasm32-unknown-unknown ]; then
  # Like cargo, place the guest under the requested target directory and
  # report where it was written.
  mkdir -p "$target_dir/$target/$profile"
  fixture=target/wasm32-unknown-unknown/release/counter_guest.wasm
  output="$(cd "$target_dir/$target/$profile" && pwd)/counter_guest.wasm"
  [ "$fixture" -ef "$output" ] || cp "$fixture" "$output"
  printf '{"reason":"compiler-artifact","target":{"name":"counter_guest","kind":["cdylib"]},"filenames":["%s"]}\n' "$output"
  exit 0
fi
case "$target" in
  aarch64-unknown-linux-ohos) linker="$CARGO_TARGET_AARCH64_UNKNOWN_LINUX_OHOS_LINKER";;
  x86_64-unknown-linux-ohos) linker="$CARGO_TARGET_X86_64_UNKNOWN_LINUX_OHOS_LINKER";;
  *) exit 81;;
esac
[ "$linker" = "$OHOS_NDK_HOME/llvm/bin/clang" ]
[ "$CC_SHELL_ESCAPED_FLAGS" = 1 ]
case "$CARGO_ENCODED_RUSTFLAGS" in *link-arg=--sysroot=*) ;; *) exit 82;; esac
mkdir -p "$target_dir/$target/$profile"
printf '%s' "$target:$profile" > "$target_dir/$target/$profile/libleptoskit_app.a"
"#,
	)
	.unwrap();
	let hvigor = tools.join("hvigorw");
	std::fs::write(&hvigor, r#"#!/bin/sh
set -eu
if [ "${FAIL_HVIGOR:-}" = 1 ]; then exit 17; fi
mkdir -p entry/build/default/outputs/default
cp ../../.leptoskit/lib/libleptoskit_bridge_ffi.a entry/build/default/outputs/default/tool-contract.txt
"#).unwrap();

	for tool in [&cargo, &hvigor] {
		std::fs::set_permissions(tool, std::fs::Permissions::from_mode(0o755)).unwrap();
	}

	let path = std::env::join_paths(
		std::iter::once(tools).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
	)
	.unwrap();
	let command = || {
		let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit"));
		command
			.current_dir(&app)
			.env("PATH", &path)
			.env("OHOS_NDK_HOME", &sdk)
			.env("HVIGORW", &hvigor)
			.env_remove("CARGO_ENCODED_RUSTFLAGS")
			.env_remove("RUSTFLAGS");
		command
	};
	for (target, release, expected) in [
		("harmonyos", false, "aarch64-unknown-linux-ohos:debug"),
		(
			"harmonyos-x86_64",
			true,
			"x86_64-unknown-linux-ohos:release",
		),
	] {
		let mut invocation = command();
		invocation.args(["native", "build", target]);

		if release {
			invocation.arg("--release");
		}

		let output = invocation.output().unwrap();
		assert!(output.status.success(), "{}", stderr(&output));
		assert_eq!(
			std::fs::read_to_string(
				app.join("hosts/harmonyos/entry/build/default/outputs/default/tool-contract.txt")
			)
			.unwrap(),
			expected
		);
		assert_eq!(
			std::fs::read(app.join("hosts/harmonyos/entry/src/main/resources/rawfile/app.wasm"))
				.unwrap(),
			std::fs::read(&guest).unwrap()
		);
	}
	let failed = command()
		.env("FAIL_HVIGOR", "1")
		.args(["native", "build", "harmonyos"])
		.output()
		.unwrap();
	assert!(!failed.status.success());
	assert!(
		stderr(&failed).contains("exit status: 17"),
		"{}",
		stderr(&failed)
	);
}

#[test]
fn generates_a_tooling_plugin_program() {
	let scratch = Scratch::new("tool_plugin");
	let dir = &scratch.path;
	let output = leptoskit(
		dir,
		&["generate", "tool-plugin", "plugin-dir", "--name", "demo"],
	);
	assert!(output.status.success(), "{}", stderr(&output));
	let manifest = std::fs::read_to_string(dir.join("plugin-dir/Cargo.toml")).expect("manifest");
	assert!(manifest.contains("leptoskit-plugin-demo"), "{manifest}");
	let program = std::fs::read_to_string(dir.join("plugin-dir/src/main.rs")).expect("program");
	assert!(program.contains("describe"), "{program}");

	assert!(program.contains("cli_namespace"), "{program}");
}

#[test]
fn an_unknown_plugin_namespace_is_named_in_a_warning() {
	let scratch = Scratch::new("plugin-namespace");
	std::fs::write(
		scratch.path.join(leptoskit_config::CONFIG_FILE),
		"[app]\nname = \"app\"\n",
	)
	.expect("config");
	let output = leptoskit(&scratch.path, &["plugin:nope"]);
	assert!(output.status.success(), "{}", stderr(&output));
	assert!(
		stderr(&output).contains("no plugin with the namespace `nope` is active in this project"),
		"{}",
		stderr(&output)
	);
}

/// A web build of a crate with no client warns that it built the server
/// only, and a set `RUSTFLAGS` warns that it hides the configured flags.
#[test]
fn a_web_build_without_a_client_and_with_rustflags_set_explains_both() {
	let scratch = fixture_project("feature_probe");
	let config = scratch.path.join(leptoskit_config::CONFIG_FILE);
	let toml = std::fs::read_to_string(&config).expect("config");
	std::fs::write(
		&config,
		toml.replace(
			"features = [\"probe\"]",
			"features = [\"probe\"]\nrustflags = [\"--cfg\", \"probe_flag\"]",
		),
	)
	.expect("config");
	let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(["build", "--platform", "web"])
		.current_dir(&scratch.path)
		.env("RUSTFLAGS", "")
		.output()
		.expect("run leptoskit build");
	let warnings = stderr(&output);
	assert!(output.status.success(), "{warnings}");
	assert!(
		warnings.contains("RUSTFLAGS is set, so cargo ignores build.rustflags"),
		"{warnings}"
	);

	assert!(
		warnings.contains("no cdylib client crate found; built the server only"),
		"{warnings}"
	);
}

/// `leptoskit serve` serves a client-rendered build until `SIGTERM`, then
/// stops cleanly.
#[cfg(unix)]
#[test]
fn serve_serves_the_build_until_sigterm() {
	use std::io::BufRead as _;
	use std::io::Read as _;
	use std::io::Write as _;

	let scratch = fixture_project("codepush");
	let port = std::net::TcpListener::bind("127.0.0.1:0")
		.and_then(|listener| listener.local_addr())
		.expect("a free port")
		.port();
	let mut child = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(["serve", "--port", &port.to_string()])
		.current_dir(&scratch.path)
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped())
		.spawn()
		.expect("start leptoskit serve");
	let mut announced = String::new();
	std::io::BufReader::new(child.stdout.take().expect("stdout"))
		.read_line(&mut announced)
		.expect("read the serving line");
	assert!(
		announced.contains(&format!("serving {}/dist at", scratch.path.display()))
			&& announced.contains(&format!(":{port}")),
		"{announced}"
	);

	// The line prints just before the server binds.
	let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);

	let mut stream = loop {
		match std::net::TcpStream::connect(("127.0.0.1", port)) {
			Ok(stream) => break stream,
			Err(error) => {
				assert!(std::time::Instant::now() < deadline, "connect: {error}");
				std::thread::sleep(std::time::Duration::from_millis(50));
			}
		}
	};

	let mut page = String::new();
	stream
		.write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
		.expect("request");
	stream.read_to_string(&mut page).expect("response");
	assert!(page.contains("<p>0.2.0</p>"), "{page}");

	let terminated = Command::new("kill")
		.args(["-TERM", &child.id().to_string()])
		.status()
		.expect("send SIGTERM");
	assert!(terminated.success());
	let status = child.wait().expect("wait for serve");
	assert!(
		status.success(),
		"`leptoskit serve` exits cleanly on SIGTERM: {status}"
	);
}

/// Without `wasm-opt`, `native bundle` still ships the guest, unoptimized,
/// and says so.
#[test]
fn native_bundle_without_wasm_opt_ships_the_guest_unoptimized() {
	let path = std::env::var_os("PATH").unwrap_or_default();
	let has = |dir: &Path, program: &str| {
		dir.join(program).is_file() || dir.join(format!("{program}.exe")).is_file()
	};
	let dirs: Vec<PathBuf> = std::env::split_paths(&path)
		.filter(|dir| !has(dir, "wasm-opt"))
		.collect();
	// Where cargo shares a directory with wasm-opt, as in some Nix profiles,
	// hiding one hides both, and this case cannot be set up.
	if !dirs.iter().any(|dir| has(dir, "cargo")) {
		return;
	}

	let scratch = fixture_project("native_guest");
	let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(["native", "bundle", "--target", "android", "--out", "bundle"])
		.current_dir(&scratch.path)
		.env("PATH", std::env::join_paths(dirs).expect("PATH"))
		.output()
		.expect("run leptoskit native bundle");
	assert!(output.status.success(), "{}", stderr(&output));

	assert!(
		stderr(&output).contains("wasm-opt is not installed, so the guest wasm ships unoptimized"),
		"{}",
		stderr(&output)
	);
	assert!(scratch.path.join("bundle/app.wasm").is_file());
}
