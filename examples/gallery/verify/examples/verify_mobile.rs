//! Builds the mobile gallery and verifies every shared screen on a device.
//!
//! - `verify_mobile android [--output <dir>]` packages an APK, installs it on
//!   `$ANDROID_SERIAL`, and runs its instrumentation journey.
//! - `verify_mobile ios [--output <dir>] [--device <udid> | --runtime <id>
//!   --device-type <id>]` packages a simulator app and launches every screen on
//!   the given simulator, or on one created for this run.
//!
//! Both fail unless the device reports `VERIFIED`, and both leave the package
//! and one screenshot per screen in the output directory.

use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use android_apk::absolute;
use android_apk::capture;
use android_apk::run;
use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use anyhow::ensure;
use gallery_verify::SCREENS;
use gallery_verify::check_rendered;

fn main() -> Result<()> {
	let root = Path::new(env!("CARGO_MANIFEST_DIR"))
		.join("../../..")
		.canonicalize()?;
	let platform = env::args().nth(1).unwrap_or_default();
	let output = absolute(
		&root,
		option("--output")?.map_or_else(
			|| root.join("target/gallery").join(&platform),
			PathBuf::from,
		),
	);
	match platform.as_str() {
		"android" => android::verify(&root, &output),
		"ios" => ios::verify(&root, &output),
		_ => {
			bail!(
				"usage: verify_mobile android [--output <dir>]\n       verify_mobile ios \
				 [--output <dir>] [--device <udid> | --runtime <id> --device-type <id>]"
			)
		}
	}
}

/// The value after `--name`, if it was given.
fn option(name: &str) -> Result<Option<String>> {
	let args: Vec<_> = env::args().skip(2).collect();
	let Some(index) = args.iter().position(|arg| arg == name) else {
		return Ok(None);
	};
	args.get(index + 1)
		.cloned()
		.map(Some)
		.with_context(|| format!("{name} requires a value"))
}

fn cargo_target_dir(root: &Path) -> PathBuf {
	absolute(
		root,
		env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from),
	)
}

mod android {
	use android_apk::Apk;
	use android_apk::Settings;
	use android_apk::Toolchain;

	use super::*;

	const PACKAGE: &str = "dev.leptoskit.gallery";

	/// Packages `gallery.apk`, installs it, and runs its instrumentation
	/// journey.
	pub fn verify(root: &Path, output: &Path) -> Result<()> {
		let toolchain = Toolchain::new(root, Settings::from_env())?;
		let device = toolchain.device()?;
		run(&mut toolchain.rustc("gallery", &["--crate-type", "cdylib"], &[])?)?;
		let apk = toolchain.package(&Apk {
			name: "gallery",
			manifest: "examples/gallery/android/AndroidManifest.xml",
			sources: &[
				"crates/leptoskit_android/android/WidgetHost.kt",
				"examples/gallery/android/GalleryActivity.kt",
				"examples/gallery/android/GalleryInstrumentation.kt",
			],
			library: "gallery",
			output,
		})?;

		device.install(&apk, PACKAGE)?;
		let journey = capture(device.adb().args([
			"shell",
			"am",
			"instrument",
			"-w",
			&format!("{PACKAGE}/.GalleryInstrumentation"),
		]))?;
		let report = String::from_utf8_lossy(&journey.stdout).into_owned();
		fs::write(output.join("instrumentation.txt"), &report)?;
		let logs = capture(
			device
				.adb()
				.args(["logcat", "-d", "AndroidRuntime:E", "*:W"]),
		)?;
		fs::write(output.join("logcat.txt"), logs.stdout)?;
		print!("{report}");
		ensure!(
			journey.status.success() && report.contains("result=VERIFIED"),
			"the Android gallery journey failed; inspect {}",
			output.join("logcat.txt").display()
		);

		let screenshots = SCREENS.iter().copied().chain(["components-scrolled"]);
		for slug in screenshots {
			let image = device.read_app_file(PACKAGE, &format!("files/gallery-{slug}.png"))?;
			if let Err(error) = check_rendered(&image) {
				bail!("the `{slug}` screenshot is not a rendered screen: {error}");
			}
			fs::write(output.join(format!("{slug}.png")), image)?;
		}
		println!("Android gallery evidence: {}", output.display());
		Ok(())
	}
}

mod ios {
	use super::*;

	const BUNDLE: &str = "dev.leptoskit.gallery";
	/// How long a launched screen may take to report.
	const REPORT_TIMEOUT: Duration = Duration::from_secs(60);
	/// Time for the window to appear on screen after its views report.
	const PRESENT_DELAY: Duration = Duration::from_millis(1500);

	fn simctl(arguments: &[&str]) -> Result<String> {
		let output = capture(Command::new("xcrun").arg("simctl").args(arguments))?;
		ensure!(
			output.status.success(),
			"simctl {} failed ({})\n{}",
			arguments.join(" "),
			output.status,
			String::from_utf8_lossy(&output.stderr)
		);
		Ok(String::from_utf8(output.stdout)?.trim().to_owned())
	}

	fn utf8(path: &Path) -> Result<&str> {
		path.to_str()
			.with_context(|| format!("{} is not Unicode", path.display()))
	}

	pub fn verify(root: &Path, output: &Path) -> Result<()> {
		let app = package(root, output)?;
		if let Some(device) = option("--device")? {
			return verify_on(&device, &app, output);
		}
		let runtime =
			option("--runtime")?.context("pass --device, or --runtime and --device-type")?;
		let device_type = option("--device-type")?.context("pass --device-type with --runtime")?;
		let device = simctl(&[
			"create",
			"Leptoskit Gallery Verification",
			&device_type,
			&runtime,
		])?;
		println!("Task-owned simulator {device}");
		// `bootstatus -b` waits until the simulator can install and launch
		// apps.
		let result = simctl(&["boot", &device])
			.and_then(|_| simctl(&["bootstatus", &device, "-b"]))
			.and_then(|_| verify_on(&device, &app, output));
		let shutdown = simctl(&["shutdown", &device]);
		let deletion = simctl(&["delete", &device]);
		result?;
		shutdown?;
		deletion?;
		Ok(())
	}

	/// Builds `Gallery.app` for the simulator matching this Mac.
	fn package(root: &Path, output: &Path) -> Result<PathBuf> {
		let target = if cfg!(target_arch = "aarch64") {
			"aarch64-apple-ios-sim"
		} else {
			"x86_64-apple-ios"
		};
		run(
			Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
				.current_dir(root)
				.env("IPHONEOS_DEPLOYMENT_TARGET", "15.0")
				.args([
					"build",
					"--locked",
					"--package",
					"gallery",
					"--bin",
					"gallery-ios",
					"--target",
					target,
				]),
		)?;
		let app = output.join("Gallery.app");
		fs::create_dir_all(&app)?;
		fs::copy(
			cargo_target_dir(root)
				.join(target)
				.join("debug/gallery-ios"),
			app.join("gallery-ios"),
		)?;
		fs::copy(
			root.join("examples/gallery/ios/Info.plist"),
			app.join("Info.plist"),
		)?;
		run(Command::new("codesign")
			.args(["--force", "--sign", "-"])
			.arg(&app))?;
		Ok(app)
	}

	fn verify_on(device: &str, app: &Path, output: &Path) -> Result<()> {
		simctl(&["install", device, utf8(app)?])?;
		let documents =
			Path::new(&simctl(&["get_app_container", device, BUNDLE, "data"])?).join("Documents");
		let mut failures = Vec::new();
		for slug in SCREENS {
			let report = documents.join(format!("gallery-{slug}.txt"));
			if report.exists() {
				fs::remove_file(&report)?;
			}
			simctl(&[
				"launch",
				"--terminate-running-process",
				device,
				BUNDLE,
				slug,
				"--verify",
			])?;
			let deadline = Instant::now() + REPORT_TIMEOUT;
			while !report.exists() {
				ensure!(
					Instant::now() < deadline,
					"`{}` produced no report within {} seconds",
					slug,
					REPORT_TIMEOUT.as_secs()
				);
				thread::sleep(Duration::from_millis(200));
			}
			let text = fs::read_to_string(&report)?;
			fs::write(output.join(format!("{slug}.txt")), &text)?;
			print!("{text}");
			if !text.starts_with("VERIFIED\t") {
				failures.push(slug);
			}
			thread::sleep(PRESENT_DELAY);
			let screenshot = output.join(format!("{slug}.png"));
			simctl(&["io", device, "screenshot", utf8(&screenshot)?])?;
			if let Err(error) = check_rendered(&fs::read(&screenshot)?) {
				bail!("the `{slug}` screenshot is not a rendered screen: {error}");
			}
		}
		simctl(&["terminate", device, BUNDLE])?;
		ensure!(
			failures.is_empty(),
			"UIKit gallery screens failed: {}",
			failures.join(", ")
		);
		println!("iOS gallery evidence: {}", output.display());
		Ok(())
	}
}
