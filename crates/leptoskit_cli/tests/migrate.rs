//! Configuration migration through the installed command surface.

use std::path::Path;
use std::process::Command;
use std::process::Output;

use leptoskit_config::LeptoskitConfig;

fn run(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.current_dir(dir)
		.args(args)
		.output()
		.expect("run CLI")
}

#[test]
fn previews_checks_and_migrates_from_a_nested_directory() {
	let dir = tempfile::tempdir().expect("scratch");
	let path = dir.path().join("leptoskit.toml");
	let nested = dir.path().join("src");
	std::fs::create_dir(&nested).expect("nested");
	let raw = "[app]\nname = 'legacy'\n[build]\nwasm_opt = false # keep\n";
	std::fs::write(&path, raw).expect("write config");

	let preview = run(&nested, &["migrate", "--dry-run"]);
	assert!(preview.status.success(), "{:?}", preview.stderr);
	let proposed = String::from_utf8(preview.stdout).expect("preview TOML");
	let config: LeptoskitConfig = toml::from_str(&proposed).expect("valid preview");
	assert!(!config.bundle.wasm_opt);
	assert!(proposed.contains("# keep"));
	assert_eq!(std::fs::read_to_string(&path).expect("unchanged"), raw);
	assert!(!run(&nested, &["migrate", "--check"]).status.success());
	assert_eq!(std::fs::read_to_string(&path).expect("unchanged"), raw);

	let applied = run(&nested, &["migrate"]);
	assert!(applied.status.success(), "{:?}", applied.stderr);
	assert_eq!(std::fs::read_to_string(&path).expect("applied"), proposed);
	assert!(LeptoskitConfig::load_from(dir.path()).is_ok());
	assert!(run(&nested, &["migrate", "--check"]).status.success());
	assert!(run(&nested, &["migrate"]).status.success());
	assert_eq!(
		std::fs::read_to_string(&path).expect("idempotent"),
		proposed
	);
}

#[test]
fn failures_never_change_the_original_file() {
	let dir = tempfile::tempdir().expect("scratch");
	let path = dir.path().join("custom.toml");

	for raw in [
		"schema_version = 2\n",
		"unknown = true\n",
		"[build]\nwasm_opt = false\n[bundle]\nwasm_opt = true\n",
		"[codepush]\nenabled = true\n",
		"[invalid",
	] {
		std::fs::write(&path, raw).expect("write config");
		let output = run(dir.path(), &["migrate", "--file", "custom.toml"]);
		assert!(!output.status.success(), "accepted {raw}");
		assert_eq!(std::fs::read_to_string(&path).expect("unchanged"), raw);
	}

	assert!(!run(dir.path(), &["migrate"]).status.success());
	assert!(
		!run(dir.path(), &["migrate", "--file", "missing.toml"])
			.status
			.success()
	);
}

#[cfg(unix)]
#[test]
fn keeps_file_permissions() {
	use std::os::unix::fs::PermissionsExt;
	let dir = tempfile::tempdir().expect("scratch");
	let path = dir.path().join("leptoskit.toml");
	std::fs::write(&path, "").expect("write config");
	std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).expect("permissions");
	assert!(run(dir.path(), &["migrate"]).status.success());
	assert_eq!(
		std::fs::metadata(&path)
			.expect("metadata")
			.permissions()
			.mode()
			& 0o777,
		0o640
	);
}

#[cfg(unix)]
#[test]
fn refuses_to_replace_symlinks() {
	let dir = tempfile::tempdir().expect("scratch");
	let target = dir.path().join("shared.toml");
	let path = dir.path().join("leptoskit.toml");
	std::fs::write(&target, "").expect("target");
	std::os::unix::fs::symlink(&target, &path).expect("symlink");
	assert!(!run(dir.path(), &["migrate"]).status.success());
	assert!(
		std::fs::symlink_metadata(&path)
			.expect("metadata")
			.file_type()
			.is_symlink()
	);
	assert_eq!(
		std::fs::read_to_string(&target).expect("unchanged target"),
		""
	);
	assert!(
		run(dir.path(), &["migrate", "--file", "shared.toml"])
			.status
			.success()
	);
}
