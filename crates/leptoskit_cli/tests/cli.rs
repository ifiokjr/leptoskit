//! CLI tests: scaffolding, caching, configuration errors, and code push,
//! driven through the library and through the built `leptoskit` binary in
//! real temporary projects. Projects under test come from `tests/fixtures`,
//! so the same on-disk fixtures back these tests and the snapshot suite.

use std::path::Path;
use std::process::Command;
use std::process::Output;

use leptoskit_cli::cache::BuildCache;
use leptoskit_cli::cache::Fingerprint;
use leptoskit_cli::codepush::SIGNING_KEY_ENV;
use leptoskit_cli::new_project;

mod support;

use support::Scratch;
use support::fixture_project;

/// Runs the built `leptoskit` binary in `dir`.
/// Runs the binary in `dir`, without any signing key from the environment
/// the tests happen to run in.
fn leptoskit(dir: &Path, args: &[&str]) -> Output {
	leptoskit_with_key_env(dir, args, None)
}

fn leptoskit_with_key_env(dir: &Path, args: &[&str], signing_key: Option<&str>) -> Output {
	let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit"));
	command
		.args(args)
		.current_dir(dir)
		.env_remove(SIGNING_KEY_ENV);
	if let Some(key) = signing_key {
		command.env(SIGNING_KEY_ENV, key);
	}
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
