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
		leptoskit_cli::wasm::embedded_wasm_bindgen()
	)));
	let index = std::fs::read_to_string(project.join("public/index.html")).expect("index");
	assert!(index.contains("./pkg/counter_client.js"));
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
		"hosts/ios/App.swift",
		"hosts/ios/AppUITests.swift",
		"hosts/android/gradlew",
		"hosts/android/app/src/main/java/MainActivity.kt",
		"guest/src/lib.rs",
	] {
		let path = project.join(file);
		assert!(path.is_file(), "native scaffold is missing {file}");
	}
	std::fs::write(project.join("hosts/ios/App.swift"), "// user change").unwrap();
	assert!(
		new_project::run(
			project.to_str().unwrap(),
			leptoskit_config::Platform::Native,
			None
		)
		.is_err()
	);
	assert_eq!(
		std::fs::read_to_string(project.join("hosts/ios/App.swift")).unwrap(),
		"// user change"
	);
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

#[test]
fn new_with_an_sdk_path_depends_on_its_crates() {
	let scratch = Scratch::new("sdk-scaffold");
	let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
	let created = leptoskit(
		&scratch.path,
		&["new", "app", "--sdk-path", sdk.to_str().expect("UTF-8")],
	);
	assert!(created.status.success(), "{}", stderr(&created));

	let manifest: toml::Table = std::fs::read_to_string(scratch.path.join("app/client/Cargo.toml"))
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
		leptoskit["features"].as_array().expect("features"),
		&[toml::Value::from("web")]
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
	let channel = store.join("pushed/production");
	let mut local: Vec<String> = walkdir::WalkDir::new(&channel)
		.into_iter()
		.map(|entry| entry.expect("walk"))
		.filter(|entry| entry.file_type().is_file())
		.map(|entry| {
			let relative = entry.path().strip_prefix(&store).expect("under the store");
			// Bucket keys are `/`-separated whatever the platform's separator.
			let key: Vec<_> = relative
				.components()
				.map(|component| component.as_os_str().to_string_lossy())
				.collect();
			format!("/registry/updates/{}", key.join("/"))
		})
		.collect();
	local.sort();
	{
		let state = state.lock().expect("lock");
		assert_eq!(
			state.puts.last().map(String::as_str),
			Some("/registry/updates/pushed/production/index.json"),
			"the index is written last: {:?}",
			state.puts
		);
		let mut uploaded = state.puts.clone();
		uploaded.sort();
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
			let (expected_type, expected_cache) = if path.ends_with("index.json") {
				("application/json", "public, max-age=60")
			} else if path.ends_with("/release.lkb") {
				(
					"application/octet-stream",
					"public, max-age=31536000, immutable",
				)
			} else {
				("application/json", "public, max-age=31536000, immutable")
			};
			assert_eq!(content_type, expected_type, "{path}");
			assert_eq!(cache_control, expected_cache, "{path}");
		}
		for request in &state.raw_requests {
			assert!(!request.contains(secret), "a request leaks the secret");
		}
	}
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
		"the router is off without [router]"
	);
	assert!(
		stderr(&output).contains("add a `[router]` table"),
		"{}",
		stderr(&output)
	);
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
	assert!(config.native.plugins.is_empty());
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
	] {
		assert!(destination.join(relative).is_file(), "missing {relative}");
	}
	let app: serde_json::Value = serde_json::from_str(
		&std::fs::read_to_string(destination.join("hosts/harmonyos/AppScope/app.json5")).unwrap(),
	)
	.unwrap();
	assert_eq!(app["app"]["bundleName"], "com.leptoskit.counter");
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
