//! Publishing dry runs must work without SDKs or credentials and must keep
//! their terminal and saved reports consistent. Every command runs in a
//! throwaway project and is explicitly prevented from inheriting store keys.

use std::path::Path;
use std::process::Command;
use std::process::Output;

const CONFIG: &str = r#"
[app]
name = "publish_fixture"
version = "1.2.3-beta.1"
[native]
package_id = "com.example.fixture"
[publish.app_store]
app_id = "1234567890"
team_id = "ABCDE12345"
"#;

fn run(root: &Path, arguments: &[&str]) -> Output {
	let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit"));
	command.current_dir(root).arg("publish").args(arguments);
	for (key, _) in std::env::vars_os() {
		if key.to_string_lossy().starts_with("LEPTOSKIT_PUBLISH_") {
			command.env_remove(key);
		}
	}
	command.output().unwrap()
}

fn project() -> tempfile::TempDir {
	let root = tempfile::tempdir().unwrap();
	std::fs::write(root.path().join("leptoskit.toml"), CONFIG).unwrap();
	root
}

fn success(output: Output) -> String {
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	String::from_utf8(output.stdout).unwrap()
}

#[test]
fn dry_runs_save_the_same_report_they_print_and_never_require_store_keys() {
	let root = project();
	let json = success(run(
		root.path(),
		&[
			"app_store",
			"--dry-run",
			"--submit",
			"--yes",
			"--json",
			"--build-number",
			"42",
		],
	));
	let report: serde_json::Value = serde_json::from_str(&json).unwrap();
	assert_eq!(report["stores"][0]["store"], "app_store");
	let summary = report["stores"][0]["summary"].as_str().unwrap();
	assert!(
		summary.contains("publish_fixture 1.2.3")
			&& summary.contains("42")
			&& summary.contains("Nothing was built or uploaded"),
		"{summary}"
	);
	assert!(
		report["steps"]
			.as_array()
			.unwrap()
			.iter()
			.any(|step| step["by"] == "person")
	);
	let saved: serde_json::Value = serde_json::from_slice(
		&std::fs::read(root.path().join(".leptoskit/publish/next-steps.json")).unwrap(),
	)
	.unwrap();
	assert_eq!(saved, report);
	let export = std::fs::read_to_string(
		root.path()
			.join(".leptoskit/publish/ios/ExportOptions.plist"),
	)
	.unwrap();
	assert!(export.contains("ABCDE12345") && export.contains("app-store-connect"));
	let text = success(run(root.path(), &["--dry-run"]));
	assert!(text.contains("one more than App Store Connect's highest"));
	assert!(text.contains("What to do next") && text.contains("next-steps.json"));
	for invalid in ["0", "-1", "later"] {
		assert!(
			!run(root.path(), &["--dry-run", "--build-number", invalid])
				.status
				.success()
		);
	}
}

#[test]
fn publishing_reports_configuration_and_credential_errors_before_building() {
	let root = project();
	let listing = success(run(root.path(), &["credentials"]));
	assert!(listing.contains("LEPTOSKIT_PUBLISH_APPLE_KEY_ID"));
	let error = run(root.path(), &["app_store"]);
	assert!(!error.status.success());
	assert!(String::from_utf8_lossy(&error.stderr).contains("isn't set"));
	assert!(!root.path().join(".leptoskit/publish/ios").exists());
	let error = run(root.path(), &["firefox", "--dry-run"]);
	assert!(!error.status.success());
	assert!(String::from_utf8_lossy(&error.stderr).contains("isn't configured"));
	std::fs::write(
		root.path().join("leptoskit.toml"),
		"[app]\nname = 'empty_publish'\n",
	)
	.unwrap();
	let error = run(root.path(), &["--dry-run"]);
	assert!(!error.status.success());
	assert!(String::from_utf8_lossy(&error.stderr).contains("no store is configured"));
}

#[test]
fn workflow_generation_and_screenshot_plans_need_no_sdk() {
	let root = project();
	success(run(root.path(), &["workflow"]));
	let workflow = std::fs::read_to_string(
		root.path()
			.join(leptoskit_cli::new_project::RELEASE_WORKFLOW),
	)
	.unwrap();
	assert!(
		workflow.contains("leptoskit publish") && workflow.contains("LEPTOSKIT_PUBLISH_APPLE_KEY")
	);
	let screenshots = root.path().join("store/screenshots");
	std::fs::create_dir_all(&screenshots).unwrap();
	std::fs::write(
		screenshots.join("screenshots.toml"),
		r#"
[devices.chrome-1280x800]
viewport = [1280, 800]
[devices."iphone-6.3"]
simulator = "Fixture iPhone"
[[screenshot]]
name = "library"
route = "/library"
[[screenshot]]
name = "settings"
route = "/settings"
devices = ["chrome-1280x800"]
"#,
	)
	.unwrap();
	let list = success(run(root.path(), &["screenshots", "--dry-run"]));
	assert!(list.contains("chrome-1280x800/01-library.png"));
	assert!(list.contains("chrome-1280x800/02-settings.png"));
	assert!(list.contains("iphone-6.3/01-library.png"));
	assert!(list.contains("3 screenshots on 2 sizes; nothing was captured"));
	assert!(!screenshots.join("chrome-1280x800").exists());
	std::fs::write(
		screenshots.join("screenshots.toml"),
		"[devices.unknown_size]\nviewport = [100, 100]\n",
	)
	.unwrap();
	let error = run(root.path(), &["screenshots", "--dry-run"]);
	assert!(!error.status.success());
	assert!(String::from_utf8_lossy(&error.stderr).contains("devices.unknown_size"));
}
