//! The real CLI checks offline bundles without needing a project or Docker.

use std::path::Path;
use std::process::Command;

fn cli(directory: &Path, args: &[&std::ffi::OsStr]) -> std::process::Output {
	Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.current_dir(directory)
		.args(args)
		.env(
			"LEPTOSKIT_INVENTORY_TEST_VALUE",
			"must-not-be-captured-in-the-inventory",
		)
		.output()
		.unwrap()
}

#[test]
fn record_and_check_use_an_independent_pin_and_make_no_runtime_claim() {
	let scratch = tempfile::tempdir().unwrap();
	let bundle = scratch.path().join("bundle");
	for directory in ["source", "dependencies", "infra", "artifacts"] {
		std::fs::create_dir_all(bundle.join(directory)).unwrap();
	}
	for (path, bytes) in [
		(
			"source/Cargo.toml",
			"[package]\nname = 'backend'\nversion = '0.1.0'\n",
		),
		("source/Cargo.lock", "version = 4\n"),
		("artifacts/backend", "executable bytes"),
		("secrets.json", "[\"DATABASE_URL\"]"),
	] {
		std::fs::write(bundle.join(path), bytes).unwrap();
	}
	std::fs::write(
		bundle.join("build.json"),
		serde_json::to_vec(&serde_json::json!({
			"repository": "https://github.com/example/backend",
			"commit": "a".repeat(40),
			"builder_image": format!("rust@sha256:{}", "b".repeat(64)),
			"target": "x86_64-unknown-linux-gnu",
			"package": "backend", "binary": "backend", "features": [],
			"no_default_features": false, "rustflags": []
		}))
		.unwrap(),
	)
	.unwrap();
	let manifest = scratch.path().join("inventory.json");
	let recorded = cli(
		scratch.path(),
		&[
			"verify".as_ref(),
			"record".as_ref(),
			"--bundle".as_ref(),
			bundle.as_os_str(),
			"--out".as_ref(),
			manifest.as_os_str(),
		],
	);
	assert!(
		recorded.status.success(),
		"{}",
		String::from_utf8_lossy(&recorded.stderr)
	);
	let stdout = String::from_utf8(recorded.stdout).unwrap();
	assert!(stdout.contains("live deployment were not verified"));
	let digest = stdout
		.lines()
		.find_map(|line| line.strip_prefix("inventory SHA-256: "))
		.unwrap();
	let inventory = std::fs::read_to_string(&manifest).unwrap();
	assert!(!inventory.contains("must-not-be-captured-in-the-inventory"));
	let args = [
		"verify".as_ref(),
		"check".as_ref(),
		"--bundle".as_ref(),
		bundle.as_os_str(),
		"--manifest".as_ref(),
		manifest.as_os_str(),
		"--sha256".as_ref(),
		digest.as_ref(),
	];
	let checked = cli(scratch.path(), &args);
	assert!(
		checked.status.success(),
		"{}",
		String::from_utf8_lossy(&checked.stderr)
	);
	assert!(String::from_utf8_lossy(&checked.stdout).contains("live deployment were not verified"));
	std::fs::write(bundle.join("artifacts/backend"), "another executable").unwrap();
	let tampered = cli(scratch.path(), &args);
	assert!(!tampered.status.success());
	assert!(String::from_utf8_lossy(&tampered.stderr).contains("artifacts/backend"));
}

#[test]
fn checking_requires_a_pin_even_when_the_caller_supplies_a_manifest() {
	let scratch = tempfile::tempdir().unwrap();
	let output = cli(
		scratch.path(),
		&[
			"verify".as_ref(),
			"check".as_ref(),
			"--bundle".as_ref(),
			"bundle".as_ref(),
			"--manifest".as_ref(),
			"inventory.json".as_ref(),
		],
	);
	assert!(!output.status.success());
	assert!(String::from_utf8_lossy(&output.stderr).contains("--sha256"));
}

#[cfg(unix)]
#[test]
fn verification_never_probes_the_current_projects_plugin_programs() {
	use std::os::unix::fs::PermissionsExt as _;

	let scratch = tempfile::tempdir().unwrap();
	let plugin = scratch.path().join("plugin-probe");
	std::fs::write(&plugin, "#!/bin/sh\n: > plugin-probed\nexit 1\n").unwrap();
	std::fs::set_permissions(&plugin, std::fs::Permissions::from_mode(0o755)).unwrap();
	let command = serde_json::to_string(plugin.to_str().unwrap()).unwrap();
	std::fs::write(
		scratch.path().join("leptoskit.toml"),
		format!(
			"[app]\nname = \"backend\"\n\n[[plugins]]\nname = \"inventory-test\"\ncommand = \
			 {command}\n"
		),
	)
	.unwrap();
	let marker = scratch.path().join("plugin-probed");
	// Check the fixture is an active plugin declaration, not a config parse
	// failure that would make the verification assertion pass vacuously.
	assert!(cli(scratch.path(), &["--help".as_ref()]).status.success());
	assert!(marker.exists());
	std::fs::remove_file(&marker).unwrap();
	assert!(
		cli(scratch.path(), &["verify".as_ref(), "--help".as_ref()])
			.status
			.success()
	);
	let failed = cli(
		scratch.path(),
		&[
			"verify".as_ref(),
			"record".as_ref(),
			"--bundle".as_ref(),
			"absent".as_ref(),
			"--out".as_ref(),
			"inventory.json".as_ref(),
		],
	);
	assert!(!failed.status.success());
	assert!(
		!marker.exists(),
		"verification ran a project plugin on the host"
	);
}
