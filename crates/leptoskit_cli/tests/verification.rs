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

/// The Rust 1.99.1 slim image index, for amd64 and arm64 builders.
const BUILDER_IMAGE: &str = "docker.io/library/rust@sha256:\
                             ed2c95ca377d6571b124b05e75e01207973ded639dce7d5542eb7bcd797e32aa";

fn git(directory: &Path, args: &[&str]) -> String {
	let output = Command::new("git")
		.current_dir(directory)
		.env("GIT_CONFIG_GLOBAL", "/dev/null")
		.env("GIT_CONFIG_NOSYSTEM", "1")
		.args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
		.args(args)
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn copy_tree(from: &Path, to: &Path) {
	std::fs::create_dir_all(to).unwrap();
	for entry in std::fs::read_dir(from).unwrap() {
		let entry = entry.unwrap();
		let target = to.join(entry.file_name());
		if entry.file_type().unwrap().is_dir() {
			copy_tree(&entry.path(), &target);
		} else {
			std::fs::copy(entry.path(), &target).unwrap();
		}
	}
}

fn run(directory: &Path, args: &[&std::ffi::OsStr]) -> (bool, String) {
	let output = cli(directory, args);
	let text = format!(
		"{}{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
	(output.status.success(), text)
}

/// Records `bundle` again, as someone republishing a tampered bundle would,
/// and returns the new inventory and its digest.
fn republish(scratch: &Path, bundle: &Path, name: &str) -> (std::path::PathBuf, String) {
	let manifest = scratch.join(format!("{name}.json"));
	let (ok, text) = run(
		scratch,
		&[
			"verify".as_ref(),
			"record".as_ref(),
			"--bundle".as_ref(),
			bundle.as_os_str(),
			"--out".as_ref(),
			manifest.as_os_str(),
		],
	);
	assert!(ok, "{text}");
	let digest = text
		.lines()
		.find_map(|line| line.strip_prefix("inventory SHA-256: "))
		.unwrap()
		.to_owned();
	(manifest, digest)
}

fn reproduce(
	scratch: &Path,
	bundle: &Path,
	manifest: &Path,
	digest: &str,
	repository: &Path,
) -> (bool, String) {
	run(
		scratch,
		&[
			"verify".as_ref(),
			"reproduce".as_ref(),
			"--bundle".as_ref(),
			bundle.as_os_str(),
			"--manifest".as_ref(),
			manifest.as_os_str(),
			"--sha256".as_ref(),
			digest.as_ref(),
			"--checkout".as_ref(),
			repository.as_os_str(),
		],
	)
}

/// Needs docker or podman and network access to crates.io and Docker Hub:
/// `cargo nextest run -p leptoskit_cli --test verification --run-ignored only`.
#[test]
#[ignore = "needs a container engine and network access"]
fn a_commit_builds_reproducibly_and_tampering_fails_the_rebuild() {
	let scratch = tempfile::tempdir().unwrap();
	let scratch = scratch.path().canonicalize().unwrap();
	let repository = scratch.join("repository");
	std::fs::create_dir_all(repository.join("src")).unwrap();
	std::fs::create_dir_all(repository.join("deploy")).unwrap();
	std::fs::write(
		repository.join("Cargo.toml"),
		"[package]\nname = \"backend\"\nversion = \"0.1.0\"\nedition = \
		 \"2024\"\n\n[dependencies]\nitoa = \"=1.0.15\"\n",
	)
	.unwrap();
	// The build fails if the compile step can reach the network.
	std::fs::write(
		repository.join("build.rs"),
		"fn main() {\n\tlet address = \"1.1.1.1:80\".parse().unwrap();\n\tlet timeout = \
		 std::time::Duration::from_secs(3);\n\tif std::net::TcpStream::connect_timeout(&address, \
		 timeout).is_ok() {\n\t\tpanic!(\"the network was reachable while compiling\");\n\t}\n}\n",
	)
	.unwrap();
	std::fs::write(
		repository.join("src/main.rs"),
		"fn main() {\n\tprintln!(\"{}\", itoa::Buffer::new().format(42));\n}\n",
	)
	.unwrap();
	std::fs::write(repository.join("deploy/service.toml"), "port = 8080\n").unwrap();
	git(&repository, &["init", "--quiet"]);
	let lock = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
		.current_dir(&repository)
		.args(["generate-lockfile", "--quiet"])
		.status()
		.unwrap();
	assert!(lock.success());
	git(&repository, &["add", "--all"]);
	git(&repository, &["commit", "--quiet", "--message", "backend"]);
	let commit = git(&repository, &["rev-parse", "HEAD"]);
	let target = if cfg!(target_arch = "aarch64") {
		"aarch64-unknown-linux-gnu"
	} else {
		"x86_64-unknown-linux-gnu"
	};
	let recipe = scratch.join("recipe.json");
	std::fs::write(
		&recipe,
		serde_json::to_vec(&serde_json::json!({
			"repository": "https://github.com/example/backend",
			"commit": commit,
			"builder_image": BUILDER_IMAGE,
			"target": target,
			"package": "backend",
			"binary": "backend",
			"features": [],
			"no_default_features": false,
			"rustflags": [],
			"infra": "deploy",
		}))
		.unwrap(),
	)
	.unwrap();

	let release = scratch.join("release");
	let (ok, text) = run(
		&scratch,
		&[
			"verify".as_ref(),
			"build".as_ref(),
			"--recipe".as_ref(),
			recipe.as_os_str(),
			"--checkout".as_ref(),
			repository.as_os_str(),
			"--secret".as_ref(),
			"DATABASE_URL".as_ref(),
			"--out".as_ref(),
			release.as_os_str(),
		],
	);
	assert!(ok, "{text}");
	assert!(text.contains("twice with identical bytes"), "{text}");
	let digest = text
		.lines()
		.find_map(|line| line.strip_prefix("inventory SHA-256: "))
		.unwrap()
		.to_owned();
	let bundle = release.join("bundle");
	let manifest = release.join("inventory.json");
	assert_eq!(
		std::fs::read_to_string(bundle.join("infra/service.toml")).unwrap(),
		"port = 8080\n"
	);
	assert!(bundle.join("dependencies/itoa-1.0.15/Cargo.toml").is_file());
	let provenance: serde_json::Value =
		serde_json::from_slice(&std::fs::read(release.join("provenance.json")).unwrap()).unwrap();
	assert_eq!(
		provenance["predicate"]["buildDefinition"]["resolvedDependencies"][0]["digest"]
			["gitCommit"],
		commit
	);

	let (ok, text) = reproduce(&scratch, &bundle, &manifest, &digest, &repository);
	assert!(ok, "{text}");
	for claim in [
		"source/ and infra/ are exactly commit",
		"dependencies/ is exactly what source/Cargo.lock resolves to",
		"artifacts/backend was rebuilt byte for byte",
		"Not verified: which executable a live server runs",
	] {
		assert!(text.contains(claim), "{claim:?} missing from {text}");
	}

	// Each attack republishes a consistent inventory, so only the rebuild,
	// the commit, or the lockfile can catch it.
	type Attack = (&'static str, fn(&Path), &'static str);
	let attacks: [Attack; 3] = [
		(
			"source",
			|bundle| std::fs::write(bundle.join("source/src/main.rs"), "fn main() {}\n").unwrap(),
			"source/ is not commit",
		),
		(
			"dependency",
			|bundle| {
				let library = bundle.join("dependencies/itoa-1.0.15/src/lib.rs");
				let mut text = std::fs::read_to_string(&library).unwrap();
				text.push_str("\n// changed\n");
				std::fs::write(library, text).unwrap();
			},
			"dependencies/ is not what source/Cargo.lock resolves to",
		),
		(
			"executable",
			|bundle| {
				let executable = bundle.join("artifacts/backend");
				let mut bytes = std::fs::read(&executable).unwrap();
				bytes.extend_from_slice(b"appended");
				std::fs::write(executable, bytes).unwrap();
			},
			"was not reproduced",
		),
	];

	for (name, tamper, expected) in attacks {
		let copy = scratch.join(format!("tampered-{name}"));
		copy_tree(&bundle, &copy);
		tamper(&copy);
		let (manifest, digest) = republish(&scratch, &copy, name);
		let (ok, text) = reproduce(&scratch, &copy, &manifest, &digest, &repository);
		assert!(!ok, "{name} tampering was accepted: {text}");
		assert!(
			text.contains(expected),
			"{name}: {expected:?} missing from {text}"
		);
	}
}
