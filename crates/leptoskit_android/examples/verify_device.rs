//! Builds and verifies the real Android widget Activity on an explicit device.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::thread;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use anyhow::ensure;
use sha2::Digest;
use sha2::Sha256;

const PACKAGE: &str = "dev.leptoskit.widgets.probe";
const STDLIB_SHA256: &str = "263bdc679e1f62012db7b091796279b6d71cf36f4797a98ff1ace05835f201c8";

struct Tools {
	root: PathBuf,
	sdk: PathBuf,
	java_home: PathBuf,
	artifacts: PathBuf,
	serial: String,
}

impl Tools {
	fn load() -> Result<Self> {
		let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.parent()
			.and_then(Path::parent)
			.context("Android crate must be inside the workspace crates directory")?
			.to_owned();
		let sdk = env::var_os("ANDROID_HOME")
			.or_else(|| env::var_os("ANDROID_SDK_ROOT"))
			.map(PathBuf::from)
			.context("set ANDROID_HOME to the Android SDK")?;
		let serial = env::var("ANDROID_SERIAL")
			.context("set ANDROID_SERIAL to a dedicated device or emulator")?;
		let java_home = if let Some(path) = env::var_os("JAVA_HOME") {
			PathBuf::from(path)
		} else {
			let output = Command::new("java")
				.args(["-XshowSettings:properties", "-version"])
				.output()
				.context("discover Java home")?;
			ensure!(
				output.status.success(),
				"java failed while discovering JAVA_HOME"
			);
			let properties = String::from_utf8(output.stderr)?;
			PathBuf::from(
				properties
					.lines()
					.find_map(|line| line.trim().strip_prefix("java.home = "))
					.context("java did not report java.home")?,
			)
		};
		let artifacts = absolute(
			&root,
			env::var_os("LEPTOSKIT_ANDROID_ARTIFACTS")
				.map_or_else(|| root.join("target/android-widgets"), PathBuf::from),
		);
		fs::create_dir_all(&artifacts)?;
		Ok(Self {
			root,
			sdk,
			java_home,
			artifacts,
			serial,
		})
	}

	fn command(&self, program: impl AsRef<OsStr>) -> Command {
		let mut command = Command::new(program);
		command
			.current_dir(&self.root)
			.env("JAVA_HOME", &self.java_home);
		command
	}

	fn adb(&self) -> Command {
		let mut command = self.command(self.sdk.join("platform-tools/adb"));
		command.args(["-s", &self.serial]);
		command
	}

	fn build(&self) -> Result<()> {
		let abi = env::var("ANDROID_ABI").unwrap_or_else(|_| "arm64-v8a".into());
		let (target, compiler) = match abi.as_str() {
			"arm64-v8a" => ("aarch64-linux-android", "aarch64-linux-android28-clang"),
			"x86_64" => ("x86_64-linux-android", "x86_64-linux-android28-clang"),
			_ => bail!("unsupported Android test ABI {abi}"),
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
			"Android NDK linker is missing at {}",
			linker.display()
		);
		let cargo_target = absolute(
			&self.root,
			env::var_os("CARGO_TARGET_DIR").map_or_else(|| self.root.join("target"), PathBuf::from),
		);
		let mut cargo = self.command(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
		cargo
			.env(
				format!(
					"CARGO_TARGET_{}_LINKER",
					target.to_ascii_uppercase().replace('-', "_")
				),
				linker,
			)
			.env(
				"CARGO_BUILD_JOBS",
				env::var_os("CARGO_BUILD_JOBS").unwrap_or_else(|| "2".into()),
			)
			.args([
				"rustc",
				"--locked",
				"--package",
				"leptoskit_android",
				"--lib",
				"--features",
				"probe",
				"--target",
				target,
				"--",
				"-C",
				"link-arg=-Wl,-z,max-page-size=16384",
			]);
		run(&mut cargo)?;
		let native_dir = self.artifacts.join("lib").join(&abi);
		fs::create_dir_all(&native_dir)?;
		fs::copy(
			cargo_target
				.join(target)
				.join("debug/libleptoskit_android.so"),
			native_dir.join("libleptoskit_android.so"),
		)?;
		let stdlib = self.artifacts.join("kotlin-stdlib-2.1.21.jar");
		if !stdlib.exists() {
			run(self.command("curl").args(["--fail", "--silent", "--show-error", "--location", "https://repo.maven.apache.org/maven2/org/jetbrains/kotlin/kotlin-stdlib/2.1.21/kotlin-stdlib-2.1.21.jar", "-o"]).arg(&stdlib))?;
		}
		let hash = format!("{:x}", Sha256::digest(fs::read(&stdlib)?));
		ensure!(
			hash == STDLIB_SHA256,
			"Kotlin stdlib checksum mismatch at {}",
			stdlib.display()
		);
		let platform = self
			.sdk
			.join("platforms")
			.join(format!(
				"android-{}",
				env::var("ANDROID_API_LEVEL").unwrap_or_else(|_| "35".into())
			))
			.join("android.jar");
		let tools = self
			.sdk
			.join("build-tools")
			.join(env::var("ANDROID_BUILD_TOOLS_VERSION").unwrap_or_else(|_| "36.0.0".into()));
		let jar = self.artifacts.join("widgets.jar");
		let classpath = env::join_paths([&platform, &stdlib])?;
		run(self
			.command("kotlinc")
			.args([
				"crates/leptoskit_android/android/WidgetHost.kt",
				"crates/leptoskit_android/android-tests/ProbeActivity.kt",
				"-language-version",
				"2.1",
				"-api-version",
				"2.1",
				"-no-stdlib",
				"-classpath",
			])
			.arg(classpath)
			.arg("-d")
			.arg(&jar))?;
		let dex = self.artifacts.join("dex");
		fs::create_dir_all(&dex)?;
		for entry in fs::read_dir(&dex)? {
			let path = entry?.path();
			if path.extension().is_some_and(|value| value == "dex") {
				fs::remove_file(path)?;
			}
		}
		run(self
			.command(tools.join("d8"))
			.arg("--lib")
			.arg(platform)
			.args(["--min-api", "28", "--output"])
			.arg(&dex)
			.arg(jar)
			.arg(stdlib))?;
		let unsigned = self.artifacts.join("unsigned.apk");
		run(self
			.command(tools.join("aapt2"))
			.args(["link", "-I"])
			.arg(
				self.sdk
					.join("platforms")
					.join(format!(
						"android-{}",
						env::var("ANDROID_API_LEVEL").unwrap_or_else(|_| "35".into())
					))
					.join("android.jar"),
			)
			.args([
				"--manifest",
				"crates/leptoskit_android/android-tests/AndroidManifest.xml",
				"-o",
			])
			.arg(&unsigned))?;
		run(self
			.command("zip")
			.current_dir(&self.artifacts)
			.args(["-q", "-u", "unsigned.apk"])
			.arg(format!("lib/{abi}/libleptoskit_android.so")))?;
		let mut dex_files = fs::read_dir(&dex)?
			.map(|entry| entry.map(|entry| entry.path()))
			.collect::<std::io::Result<Vec<_>>>()?;
		dex_files.retain(|path| path.extension().is_some_and(|value| value == "dex"));
		dex_files.sort();
		ensure!(!dex_files.is_empty(), "D8 produced no dex files");
		let mut zip = self.command("zip");
		zip.current_dir(&dex).args(["-q", "-u"]).arg(&unsigned);
		for path in dex_files {
			zip.arg(path.file_name().context("dex filename")?);
		}
		run(&mut zip)?;
		let aligned = self.artifacts.join("aligned.apk");
		run(self
			.command(tools.join("zipalign"))
			.args(["-f", "-P", "16", "4"])
			.arg(unsigned)
			.arg(&aligned))?;
		let key = self.artifacts.join("debug.keystore");
		if !key.exists() {
			run(self
				.command(self.java_home.join("bin/keytool"))
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
		run(self
			.command(tools.join("apksigner"))
			.args(["sign", "--ks"])
			.arg(key)
			.args(["--ks-pass", "pass:android", "--out"])
			.arg(self.artifacts.join("widgets.apk"))
			.arg(aligned))?;
		Ok(())
	}

	fn verify(&self) -> Result<()> {
		run(self
			.adb()
			.args(["install", "--no-incremental", "-r"])
			.arg(self.artifacts.join("widgets.apk")))?;
		run(self.adb().args(["shell", "am", "force-stop", PACKAGE]))?;
		run(self.adb().args(["logcat", "-c"]))?;
		run(self.adb().args([
			"shell",
			"run-as",
			PACKAGE,
			"rm",
			"-f",
			"files/verdict.txt",
			"files/widgets.png",
			"files/styled-controls.png",
		]))?;
		run(self.adb().args([
			"shell",
			"am",
			"start",
			"-W",
			"-n",
			"dev.leptoskit.widgets.probe/dev.leptoskit.widgets.ProbeActivity",
		]))?;
		let mut ready = false;
		for _ in 0..30 {
			let output = capture(self.adb().args([
				"shell",
				"run-as",
				PACKAGE,
				"test",
				"-f",
				"files/verdict.txt",
			]))?;
			if output.status.success() {
				ready = true;
				break;
			}
			thread::sleep(Duration::from_secs(1));
		}
		let logs = capture(self.adb().args([
			"logcat",
			"-d",
			"-s",
			"LeptoskitWidgets:I",
			"AndroidRuntime:E",
		]))?;
		fs::write(self.artifacts.join("logcat.txt"), &logs.stdout)?;
		let crashes = capture(self.adb().args(["logcat", "-d", "-b", "crash"]))?;
		fs::write(self.artifacts.join("crash.txt"), crashes.stdout)?;
		ensure!(
			ready,
			"Android Activity produced no verdict; inspect {}",
			self.artifacts.join("logcat.txt").display()
		);
		let verdict =
			capture(
				self.adb()
					.args(["exec-out", "run-as", PACKAGE, "cat", "files/verdict.txt"]),
			)?;
		fs::write(self.artifacts.join("verdict.txt"), &verdict.stdout)?;
		let verdict = String::from_utf8(verdict.stdout)?;
		println!("{verdict}");
		ensure!(
			verdict.lines().next() == Some("VERIFIED"),
			"Android native widgets failed; inspect {}",
			self.artifacts.join("logcat.txt").display()
		);
		for filename in ["widgets.png", "styled-controls.png"] {
			let image = capture(self.adb().args([
				"exec-out",
				"run-as",
				PACKAGE,
				"cat",
				&format!("files/{filename}"),
			]))?;
			ensure!(
				image.stdout.starts_with(b"\x89PNG\r\n\x1a\n"),
				"device returned no PNG for {filename}"
			);
			fs::write(self.artifacts.join(filename), image.stdout)?;
		}
		println!(
			"Android native widget evidence: {}",
			self.artifacts.display()
		);
		Ok(())
	}
}

fn absolute(root: &Path, path: PathBuf) -> PathBuf {
	if path.is_absolute() {
		path
	} else {
		root.join(path)
	}
}

fn run(command: &mut Command) -> Result<()> {
	let status = command
		.status()
		.with_context(|| format!("start {}", command.get_program().to_string_lossy()))?;
	ensure!(
		status.success(),
		"{} failed with {status}",
		command.get_program().to_string_lossy()
	);
	Ok(())
}

fn capture(command: &mut Command) -> Result<Output> {
	command
		.output()
		.with_context(|| format!("start {}", command.get_program().to_string_lossy()))
}

fn main() -> Result<()> {
	let tools = Tools::load()?;
	tools.build()?;
	tools.verify()
}
