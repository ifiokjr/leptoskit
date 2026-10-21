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
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use anyhow::ensure;
use gallery::SCREENS;

fn main() -> Result<()> {
	let root = Path::new(env!("CARGO_MANIFEST_DIR"))
		.join("../..")
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

fn absolute(root: &Path, path: PathBuf) -> PathBuf {
	if path.is_absolute() {
		path
	} else {
		root.join(path)
	}
}

fn cargo_target_dir(root: &Path) -> PathBuf {
	absolute(
		root,
		env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from),
	)
}

fn run(command: &mut Command) -> Result<()> {
	let status = command
		.status()
		.with_context(|| format!("start {}", command.get_program().to_string_lossy()))?;
	ensure!(status.success(), "{command:?} failed with {status}");
	Ok(())
}

fn capture(command: &mut Command) -> Result<Output> {
	command
		.output()
		.with_context(|| format!("start {}", command.get_program().to_string_lossy()))
}

mod android {
	use sha2::Digest;
	use sha2::Sha256;

	use super::*;

	const PACKAGE: &str = "dev.leptoskit.gallery";
	/// Newer Kotlin runtimes carry metadata that D8 for API 28 rejects.
	const STDLIB: &str = "kotlin-stdlib-2.1.21.jar";
	const STDLIB_SHA256: &str = "263bdc679e1f62012db7b091796279b6d71cf36f4797a98ff1ace05835f201c8";

	struct Android {
		root: PathBuf,
		output: PathBuf,
		sdk: PathBuf,
		serial: String,
		abi: String,
		api_level: String,
		build_tools: PathBuf,
	}

	pub fn verify(root: &Path, output: &Path) -> Result<()> {
		let sdk = env::var_os("ANDROID_HOME")
			.or_else(|| env::var_os("ANDROID_SDK_ROOT"))
			.map(PathBuf::from)
			.context("set ANDROID_HOME to the Android SDK")?;
		let android = Android {
			root: root.to_owned(),
			output: output.to_owned(),
			serial: env::var("ANDROID_SERIAL")
				.context("set ANDROID_SERIAL to a dedicated device or emulator")?,
			abi: env::var("ANDROID_ABI").unwrap_or_else(|_| "arm64-v8a".into()),
			api_level: env::var("ANDROID_API_LEVEL").unwrap_or_else(|_| "35".into()),
			build_tools: sdk
				.join("build-tools")
				.join(env::var("ANDROID_BUILD_TOOLS_VERSION").unwrap_or_else(|_| "36.0.0".into())),
			sdk,
		};
		fs::create_dir_all(output)?;
		let apk = android.package()?;
		android.instrument(&apk)
	}

	impl Android {
		fn command(&self, program: impl AsRef<OsStr>) -> Command {
			let mut command = Command::new(program);
			command.current_dir(&self.root);
			command
		}

		fn adb(&self) -> Command {
			let mut command = self.command(self.sdk.join("platform-tools/adb"));
			command.args(["-s", &self.serial]);
			command
		}

		fn platform_jar(&self) -> PathBuf {
			self.sdk
				.join("platforms")
				.join(format!("android-{}", self.api_level))
				.join("android.jar")
		}

		/// Cross-compiles the gallery as `libgallery.so` for the device ABI.
		fn compile_library(&self) -> Result<PathBuf> {
			let (target, compiler) = match self.abi.as_str() {
				"arm64-v8a" => ("aarch64-linux-android", "aarch64-linux-android28-clang"),
				"x86_64" => ("x86_64-linux-android", "x86_64-linux-android28-clang"),
				abi => bail!("unsupported Android ABI {abi}; use arm64-v8a or x86_64"),
			};
			let ndk = env::var_os("ANDROID_NDK_HOME").map_or_else(
				|| {
					self.sdk.join("ndk").join(
						env::var("ANDROID_NDK_VERSION").unwrap_or_else(|_| "27.1.12297006".into()),
					)
				},
				PathBuf::from,
			);
			let host = if cfg!(target_os = "macos") {
				"darwin-x86_64"
			} else {
				"linux-x86_64"
			};
			let linker = ndk
				.join("toolchains/llvm/prebuilt")
				.join(host)
				.join("bin")
				.join(compiler);
			ensure!(
				linker.is_file(),
				"the Android NDK linker is missing at {}",
				linker.display()
			);
			run(self
				.command(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
				.env(
					format!(
						"CARGO_TARGET_{}_LINKER",
						target.to_ascii_uppercase().replace('-', "_")
					),
					linker,
				)
				.args([
					"rustc",
					"--locked",
					"--package",
					"gallery",
					"--lib",
					"--crate-type",
					"cdylib",
					"--target",
					target,
					"--",
					// Android 15 devices may use 16 KiB pages.
					"-C",
					"link-arg=-Wl,-z,max-page-size=16384",
				]))?;
			Ok(cargo_target_dir(&self.root)
				.join(target)
				.join("debug/libgallery.so"))
		}

		fn kotlin_stdlib(&self) -> Result<PathBuf> {
			let stdlib = self.output.join(STDLIB);
			if !stdlib.exists() {
				let url = format!(
					"https://repo.maven.apache.org/maven2/org/jetbrains/kotlin/kotlin-stdlib/2.1.21/{STDLIB}"
				);
				run(self
					.command("curl")
					.args([
						"--fail",
						"--silent",
						"--show-error",
						"--location",
						&url,
						"-o",
					])
					.arg(&stdlib))?;
			}
			let hash = format!("{:x}", Sha256::digest(fs::read(&stdlib)?));
			if hash != STDLIB_SHA256 {
				fs::remove_file(&stdlib)?;
				bail!("the downloaded {STDLIB} failed its checksum");
			}
			Ok(stdlib)
		}

		/// Compiles the Kotlin host into dex files and returns their paths.
		fn dex(&self) -> Result<Vec<PathBuf>> {
			let stdlib = self.kotlin_stdlib()?;
			let jar = self.output.join("gallery.jar");
			run(self
				.command("kotlinc")
				.args([
					"crates/leptoskit_android/android/WidgetHost.kt",
					"examples/gallery/android/GalleryActivity.kt",
					"examples/gallery/android/GalleryInstrumentation.kt",
					"-language-version",
					"2.1",
					"-api-version",
					"2.1",
					"-no-stdlib",
					"-classpath",
				])
				.arg(env::join_paths([self.platform_jar(), stdlib.clone()])?)
				.arg("-d")
				.arg(&jar))?;
			let dex = self.output.join("dex");
			if dex.exists() {
				fs::remove_dir_all(&dex)?;
			}
			fs::create_dir_all(&dex)?;
			run(self
				.command(self.build_tools.join("d8"))
				.arg("--lib")
				.arg(self.platform_jar())
				.args(["--min-api", "28", "--output"])
				.arg(&dex)
				.arg(jar)
				.arg(stdlib))?;
			let mut files = fs::read_dir(&dex)?
				.map(|entry| entry.map(|entry| entry.path()))
				.collect::<std::io::Result<Vec<_>>>()?;
			files.retain(|path| path.extension().is_some_and(|value| value == "dex"));
			files.sort();
			ensure!(!files.is_empty(), "D8 produced no dex files");
			Ok(files)
		}

		/// Builds, aligns, and signs `gallery.apk` with a local debug key.
		fn package(&self) -> Result<PathBuf> {
			let library = self.compile_library()?;
			let native = self.output.join("lib").join(&self.abi);
			fs::create_dir_all(&native)?;
			fs::copy(library, native.join("libgallery.so"))?;
			let dex = self.dex()?;
			let unsigned = self.output.join("unsigned.apk");
			run(self
				.command(self.build_tools.join("aapt2"))
				.args(["link", "-I"])
				.arg(self.platform_jar())
				.args([
					"--manifest",
					"examples/gallery/android/AndroidManifest.xml",
					"-o",
				])
				.arg(&unsigned))?;
			run(self
				.command("zip")
				.current_dir(&self.output)
				.args(["-q", "-u", "unsigned.apk"])
				.arg(format!("lib/{}/libgallery.so", self.abi)))?;
			run(self
				.command("zip")
				.args(["-q", "-j", "-u"])
				.arg(&unsigned)
				.args(dex))?;
			let aligned = self.output.join("aligned.apk");
			run(self
				.command(self.build_tools.join("zipalign"))
				.args(["-f", "-P", "16", "4"])
				.arg(&unsigned)
				.arg(&aligned))?;
			let key = self.output.join("debug.keystore");
			if !key.exists() {
				// Prefer the JDK that also runs kotlinc and D8.
				let keytool = env::var_os("JAVA_HOME").map_or_else(
					|| PathBuf::from("keytool"),
					|home| PathBuf::from(home).join("bin/keytool"),
				);
				run(self
					.command(keytool)
					.args(["-genkeypair", "-keystore"])
					.arg(&key)
					.args([
						"-storepass",
						"android",
						"-keypass",
						"android",
						"-alias",
						"androiddebugkey",
						"-dname",
						"CN=Android Debug,O=Android,C=US",
						"-keyalg",
						"RSA",
						"-validity",
						"10000",
					]))?;
			}
			let apk = self.output.join("gallery.apk");
			run(self
				.command(self.build_tools.join("apksigner"))
				.args(["sign", "--ks"])
				.arg(key)
				.args(["--ks-pass", "pass:android", "--out"])
				.arg(&apk)
				.arg(aligned))?;
			Ok(apk)
		}

		/// Installs the APK and runs its instrumentation journey.
		fn instrument(&self, apk: &Path) -> Result<()> {
			// A previous install signed with another debug key would refuse the
			// update. A missing package fails to uninstall, which is fine.
			capture(self.adb().args(["uninstall", PACKAGE]))?;
			run(self.adb().args(["install", "--no-incremental"]).arg(apk))?;
			run(self.adb().args(["shell", "am", "force-stop", PACKAGE]))?;
			run(self.adb().args(["logcat", "-c"]))?;
			let journey = capture(self.adb().args([
				"shell",
				"am",
				"instrument",
				"-w",
				&format!("{PACKAGE}/.GalleryInstrumentation"),
			]))?;
			let report = String::from_utf8_lossy(&journey.stdout).into_owned();
			fs::write(self.output.join("instrumentation.txt"), &report)?;
			let logs = capture(self.adb().args(["logcat", "-d", "AndroidRuntime:E", "*:W"]))?;
			fs::write(self.output.join("logcat.txt"), logs.stdout)?;
			print!("{report}");
			ensure!(
				journey.status.success() && report.contains("result=VERIFIED"),
				"the Android gallery journey failed; inspect {}",
				self.output.join("logcat.txt").display()
			);

			let screenshots = SCREENS
				.iter()
				.map(|screen| screen.slug)
				.chain(["components-scrolled"]);
			for slug in screenshots {
				let image = capture(self.adb().args([
					"exec-out",
					"run-as",
					PACKAGE,
					"cat",
					&format!("files/gallery-{slug}.png"),
				]))?;
				ensure!(
					image.stdout.starts_with(b"\x89PNG\r\n\x1a\n"),
					"the device returned no screenshot for {slug}"
				);
				fs::write(self.output.join(format!("{slug}.png")), image.stdout)?;
			}
			println!("Android gallery evidence: {}", self.output.display());
			Ok(())
		}
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
		let result = simctl(&["boot", &device]).and_then(|_| verify_on(&device, &app, output));
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
		for screen in &SCREENS {
			let report = documents.join(format!("gallery-{}.txt", screen.slug));
			if report.exists() {
				fs::remove_file(&report)?;
			}
			simctl(&[
				"launch",
				"--terminate-running-process",
				device,
				BUNDLE,
				screen.slug,
				"--verify",
			])?;
			let deadline = Instant::now() + REPORT_TIMEOUT;
			while !report.exists() {
				ensure!(
					Instant::now() < deadline,
					"`{}` produced no report within {} seconds",
					screen.slug,
					REPORT_TIMEOUT.as_secs()
				);
				thread::sleep(Duration::from_millis(200));
			}
			let text = fs::read_to_string(&report)?;
			fs::write(output.join(format!("{}.txt", screen.slug)), &text)?;
			print!("{text}");
			if !text.starts_with("VERIFIED\t") {
				failures.push(screen.slug);
			}
			thread::sleep(PRESENT_DELAY);
			let screenshot = output.join(format!("{}.png", screen.slug));
			simctl(&["io", device, "screenshot", utf8(&screenshot)?])?;
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
