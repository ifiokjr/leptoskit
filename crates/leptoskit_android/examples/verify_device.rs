//! Builds and verifies the real Android widget Activity on an explicit device.
//!
//! `--coverage` instruments the library and pulls its LLVM profile.

use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use android_apk::Apk;
use android_apk::Settings;
use android_apk::Toolchain;
use android_apk::absolute;
use android_apk::capture;
use android_apk::run;
use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;

const PACKAGE: &str = "dev.leptoskit.widgets.probe";
const VERDICT: &str = "files/verdict.txt";
/// Where `ProbeActivity` writes the LLVM profile of a `--coverage` build.
const COVERAGE_PROFILE: &str = "files/coverage.profraw";

fn main() -> Result<()> {
	let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.parent()
		.and_then(Path::parent)
		.context("Android crate must be inside the workspace crates directory")?
		.to_owned();
	let coverage = env::args().skip(1).any(|argument| argument == "--coverage");
	let artifacts = absolute(
		&root,
		env::var_os("LEPTOSKIT_ANDROID_ARTIFACTS")
			.map_or_else(|| root.join("target/android-widgets"), PathBuf::from),
	);
	let toolchain = Toolchain::new(&root, Settings::from_env())?;
	let device = toolchain.device()?;

	// `cargo rustc` passes the rustc flags to `leptoskit_android` alone, so the
	// profile covers this crate and none of its dependencies.
	let (features, rustc_flags): (_, &[_]) = if coverage {
		("probe,coverage", &["-C", "instrument-coverage"])
	} else {
		("probe", &[])
	};
	let mut cargo = toolchain.rustc("leptoskit_android", &["--features", features], rustc_flags)?;
	cargo.env(
		"CARGO_BUILD_JOBS",
		env::var_os("CARGO_BUILD_JOBS").unwrap_or_else(|| "2".into()),
	);
	run(&mut cargo)?;
	let apk = toolchain.package(&Apk {
		name: "widgets",
		manifest: "crates/leptoskit_android/android-tests/AndroidManifest.xml",
		sources: &[
			"crates/leptoskit_android/android/WidgetHost.kt",
			"crates/leptoskit_android/android-tests/ProbeActivity.kt",
		],
		library: "leptoskit_android",
		output: &artifacts,
	})?;

	device.install(&apk, PACKAGE)?;
	run(device.adb().args([
		"shell",
		"am",
		"start",
		"-W",
		"-n",
		&format!("{PACKAGE}/dev.leptoskit.widgets.ProbeActivity"),
	]))?;
	let ready = device.wait_for_app_file(PACKAGE, VERDICT, 30, Duration::from_secs(1))?;
	let logs = capture(device.adb().args([
		"logcat",
		"-d",
		"-s",
		"LeptoskitWidgets:I",
		"AndroidRuntime:E",
	]))?;
	fs::write(artifacts.join("logcat.txt"), &logs.stdout)?;
	let crashes = capture(device.adb().args(["logcat", "-d", "-b", "crash"]))?;
	fs::write(artifacts.join("crash.txt"), crashes.stdout)?;
	ensure!(
		ready,
		"Android Activity produced no verdict; inspect {}",
		artifacts.join("logcat.txt").display()
	);
	let verdict = device.read_app_file(PACKAGE, VERDICT)?;
	fs::write(artifacts.join("verdict.txt"), &verdict)?;
	let verdict = String::from_utf8(verdict)?;
	println!("{verdict}");
	ensure!(
		verdict.lines().next() == Some("VERIFIED"),
		"Android native widgets failed; inspect {}",
		artifacts.join("logcat.txt").display()
	);
	for filename in ["widgets.png", "styled-controls.png"] {
		let image = device.read_app_file(PACKAGE, &format!("files/{filename}"))?;
		ensure!(
			image.starts_with(b"\x89PNG\r\n\x1a\n"),
			"device returned no PNG for {filename}"
		);
		fs::write(artifacts.join(filename), image)?;
	}
	if coverage {
		let profile = device
			.read_app_file(PACKAGE, COVERAGE_PROFILE)
			.context("the coverage build wrote no LLVM profile")?;
		fs::write(artifacts.join("coverage.profraw"), profile)?;
	}
	println!("Android native widget evidence: {}", artifacts.display());
	Ok(())
}
