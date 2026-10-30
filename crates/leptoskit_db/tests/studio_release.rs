//! The studio never reaches a release build: a program with the `studio`
//! feature on, built without debug assertions, holds none of its code.
//! This test binary, built with them, holds it, which shows the search
//! would find it.

#![cfg(all(feature = "studio", feature = "sqlite", debug_assertions))]

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

/// Text only the studio's code holds: its title, and its style sheet.
const MARKERS: [&str; 2] = ["leptoskit db studio", "form.viewer { display: flex"];

fn holds(binary: &[u8], marker: &str) -> bool {
	binary
		.windows(marker.len())
		.any(|window| window == marker.as_bytes())
}

/// Builds the fixture in release, in a target directory of its own, since
/// the one running this test is locked.
fn release_fixture() -> PathBuf {
	let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
	let target = workspace.join("target").join("studio-release");
	let status = Command::new(env!("CARGO"))
		.current_dir(&workspace)
		.args([
			"build",
			"--release",
			"--locked",
			"--package",
			"leptoskit_db_studio_fixture",
			"--target-dir",
		])
		.arg(&target)
		.status()
		.expect("cargo runs");
	assert!(status.success(), "the fixture builds in release");
	target.join("release").join(format!(
		"leptoskit_db_studio_fixture{}",
		std::env::consts::EXE_SUFFIX
	))
}

#[test]
fn release_builds_leave_the_studio_out() {
	// This binary links the studio, so the markers are found in it.
	std::hint::black_box(leptoskit_db::studio::start as *const ());
	let this = std::fs::read(std::env::current_exe().unwrap()).unwrap();

	for marker in MARKERS {
		assert!(holds(&this, marker), "a debug build holds {marker:?}");
	}

	let fixture = release_fixture();
	let release = std::fs::read(&fixture).unwrap();

	for marker in MARKERS {
		assert!(!holds(&release, marker), "a release build holds {marker:?}");
	}

	// Asked to serve the studio, it says why it can't.
	let output = Command::new(&fixture)
		.env(leptoskit_db::STUDIO_ENV, "127.0.0.1:0")
		.output()
		.unwrap();
	assert!(!output.status.success());
	assert!(
		String::from_utf8_lossy(&output.stderr).contains("this build has no studio"),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
}
