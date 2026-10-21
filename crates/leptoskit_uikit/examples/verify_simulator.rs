//! Builds and runs the UIKit native probe on a simulator created for this run.

#[cfg(not(target_os = "macos"))]
fn main() {
	eprintln!("UIKit simulator verification requires macOS and Xcode.");
	std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
	host::run()
}

#[cfg(target_os = "macos")]
mod host {
	use std::error::Error;
	use std::path::Path;
	use std::path::PathBuf;
	use std::process::Command;
	use std::time::Duration;
	use std::time::Instant;

	type Result<T> = std::result::Result<T, Box<dyn Error>>;
	const BUNDLE: &str = "dev.leptoskit.uikit-probe";

	fn command(program: &str, arguments: &[&str]) -> Result<String> {
		let output = Command::new(program).args(arguments).output()?;
		if !output.status.success() {
			return Err(format!(
				"{program} {} failed ({})\n{}\n{}",
				arguments.join(" "),
				output.status,
				String::from_utf8_lossy(&output.stdout),
				String::from_utf8_lossy(&output.stderr)
			)
			.into());
		}
		Ok(String::from_utf8(output.stdout)?.trim().to_owned())
	}

	fn sim(arguments: &[&str]) -> Result<String> {
		let mut full = vec!["simctl"];
		full.extend_from_slice(arguments);
		command("xcrun", &full)
	}

	fn option(name: &str, default: &str) -> Result<String> {
		let args: Vec<_> = std::env::args().skip(1).collect();
		if let Some(index) = args.iter().position(|value| value == name) {
			return args
				.get(index + 1)
				.cloned()
				.ok_or_else(|| format!("{name} requires a value").into());
		}
		Ok(default.to_owned())
	}

	fn flag(name: &str) -> bool {
		std::env::args().skip(1).any(|value| value == name)
	}

	fn absolute(path: impl AsRef<Path>) -> Result<PathBuf> {
		let path = path.as_ref();
		Ok(if path.is_absolute() {
			path.to_owned()
		} else {
			std::env::current_dir()?.join(path)
		})
	}

	pub fn run() -> Result<()> {
		let root = Path::new(env!("CARGO_MANIFEST_DIR"));
		let output = absolute(option("--output", "/tmp/leptoskit-uikit-verification")?)?;
		let target_dir = absolute(
			std::env::var_os("CARGO_TARGET_DIR")
				.map_or_else(|| root.join("../../target"), PathBuf::from),
		)?;
		let runtime = option("--runtime", "com.apple.CoreSimulator.SimRuntime.iOS-26-5")?;
		let device_type = option(
			"--device-type",
			"com.apple.CoreSimulator.SimDeviceType.iPhone-17",
		)?;
		// `--coverage` instruments the probe, which flushes its LLVM profile
		// into the output directory before reporting.
		let coverage = flag("--coverage");
		let target = if std::env::consts::ARCH == "aarch64" {
			"aarch64-apple-ios-sim"
		} else {
			"x86_64-apple-ios"
		};
		let mut build = Command::new("cargo");
		build.args([
			"build",
			"-p",
			"leptoskit_uikit",
			"--target",
			target,
			"--example",
			"native_probe",
		]);
		if coverage {
			build
				.args(["--features", "coverage"])
				.env("RUSTFLAGS", "-C instrument-coverage");
		}
		let status = build
			.env("CARGO_TARGET_DIR", &target_dir)
			.env("CARGO_BUILD_JOBS", "2")
			.env("IPHONEOS_DEPLOYMENT_TARGET", "15.0")
			.status()?;
		if !status.success() {
			return Err("UIKit probe build failed".into());
		}
		std::fs::create_dir_all(&output)?;
		let bundle = output.join("LeptoskitUIKitProbe.app");
		std::fs::create_dir_all(&bundle)?;
		std::fs::copy(
			target_dir.join(target).join("debug/examples/native_probe"),
			bundle.join("native_probe"),
		)?;
		for file in ["Info.plist", "probe-image.png", "probe-image.bmp"] {
			std::fs::copy(root.join("probe").join(file), bundle.join(file))?;
		}
		command(
			"codesign",
			&[
				"--force",
				"--sign",
				"-",
				bundle.to_str().ok_or("bundle path must be Unicode")?,
			],
		)?;
		let device = sim(&[
			"create",
			"Leptoskit UIKit Verification",
			&device_type,
			&runtime,
		])?;
		println!("Task-owned simulator {device}");
		let boot = sim(&["boot", &device]);
		let result = match boot {
			Ok(_) => verify(&device, &bundle, &output, coverage),
			Err(error) => Err(error),
		};
		let shutdown = sim(&["shutdown", &device]);
		let deletion = sim(&["delete", &device]);
		result?;
		shutdown?;
		deletion?;
		println!(
			"Verified UIKit native renderer. Artifacts: {}",
			output.display()
		);
		Ok(())
	}

	fn verify(device: &str, bundle: &Path, output: &Path, coverage: bool) -> Result<()> {
		// Booted devices can keep reporting unrelated location-service
		// migrations. Installing and launching UIApplication, followed
		// by its native assertion report, verifies the readiness this
		// renderer actually requires.
		sim(&[
			"install",
			device,
			bundle.to_str().ok_or("bundle path must be Unicode")?,
		])?;
		let mut launch = Command::new("xcrun");
		launch.args(["simctl", "launch", device, BUNDLE]);
		if coverage {
			// simctl forwards SIMCTL_CHILD_-prefixed variables to the app.
			launch.env(
				"SIMCTL_CHILD_LLVM_PROFILE_FILE",
				// The runtime replaces this profile on every run.
				output.join("uikit.profraw"),
			);
		}
		let launched = launch.output()?;
		if !launched.status.success() {
			return Err(format!(
				"simctl launch failed\n{}",
				String::from_utf8_lossy(&launched.stderr)
			)
			.into());
		}
		let container = sim(&["get_app_container", device, BUNDLE, "data"])?;
		let report = Path::new(&container).join("Documents/native-report.tsv");
		let deadline = Instant::now() + Duration::from_secs(60);
		while !report.exists() {
			if Instant::now() >= deadline {
				return Err("native UIKit probe did not produce a report within 60 seconds".into());
			}
			std::thread::sleep(Duration::from_millis(200));
		}
		let report_text = std::fs::read_to_string(&report)?;
		std::fs::copy(&report, output.join("native-report.tsv"))?;
		sim(&[
			"io",
			device,
			"screenshot",
			output
				.join("native-widgets.png")
				.to_str()
				.ok_or("screenshot path must be Unicode")?,
		])?;
		print!("{report_text}");
		if !report_text.starts_with("VERIFIED\t") {
			return Err("native UIKit assertions failed".into());
		}
		Ok(())
	}
}
