//! Debug APKs for the repository's Android device verifiers.
//!
//! `leptoskit_android`'s `verify_device` example (the widget probe) and
//! `gallery_verify`'s `verify_mobile` example (the gallery journey) package
//! their Activity the same way, without Gradle:
//!
//! 1. [`Toolchain::rustc`] cross-compiles a crate as a shared library with the
//!    NDK linker.
//! 2. [`Toolchain::package`] compiles the Kotlin host against a pinned Kotlin
//!    stdlib, dexes it with D8, links the manifest with aapt2, adds the library
//!    and the dex files, aligns the archive, and signs it with a local debug
//!    keystore.
//! 3. [`Device::install`] replaces any earlier install on the device, and
//!    [`Device::read_app_file`] pulls the evidence the app writes into its
//!    private storage.
//!
//! The crate runs on the host and depends on no widget backend, so a verifier
//! can use it without pulling Android or GTK crates into a host build.
//! [`Settings::from_env`] reads the environment variables the verifiers
//! document.

use std::env;
use std::env::JoinPathsError;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Output;
use std::str::FromStr;
use std::thread;
use std::time::Duration;

use sha2::Digest;
use sha2::Sha256;

/// The Kotlin runtime the host classes compile and dex against. Newer runtimes
/// carry metadata that D8 for API 28 rejects.
pub const KOTLIN_STDLIB_URL: &str = "https://repo.maven.apache.org/maven2/org/jetbrains/kotlin/kotlin-stdlib/2.1.21/kotlin-stdlib-2.1.21.jar";
/// The SHA-256 of [`KOTLIN_STDLIB_URL`], checked on every build.
pub const KOTLIN_STDLIB_SHA256: &str =
	"263bdc679e1f62012db7b091796279b6d71cf36f4797a98ff1ace05835f201c8";

const DEFAULT_ABI: &str = "arm64-v8a";
const DEFAULT_API_LEVEL: &str = "35";
const DEFAULT_BUILD_TOOLS_VERSION: &str = "36.0.0";
const DEFAULT_NDK_VERSION: &str = "27.1.12297006";
/// The oldest Android release the APKs run on.
const MIN_API: &str = "28";
/// The NDK's prebuilt toolchain directory for this host.
const NDK_HOST: &str = if cfg!(target_os = "macos") {
	"darwin-x86_64"
} else {
	"linux-x86_64"
};
/// The store and key password of the generated debug keystore.
const DEBUG_PASSWORD: &str = "android";

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Why building, installing, or inspecting an APK failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// A required environment variable is unset.
	#[error("set {variable} to {purpose}")]
	MissingVariable {
		variable: &'static str,
		purpose: &'static str,
	},
	#[error("unsupported Android ABI {0}; use arm64-v8a or x86_64")]
	UnsupportedAbi(String),
	/// An SDK, NDK, or JDK tool the build needs is not installed.
	#[error("{tool} is missing at {}", path.display())]
	MissingTool { tool: String, path: PathBuf },
	#[error("could not start {command}")]
	Start {
		command: String,
		#[source]
		source: io::Error,
	},
	#[error("{command} failed with {status}")]
	Failed { command: String, status: ExitStatus },
	#[error("java did not report java.home; set JAVA_HOME to a JDK")]
	UnknownJavaHome,
	/// A pinned download did not match its checksum. The file is removed, so
	/// the next build downloads it again.
	#[error("{} does not match its pinned SHA-256 and was removed", path.display())]
	Checksum { path: PathBuf },
	#[error("D8 produced no dex files in {}", directory.display())]
	NoDex { directory: PathBuf },
	/// The app never wrote a file the verifier expected, such as its verdict or
	/// coverage profile.
	#[error("{package} has no {path} on the device{}", detail(stderr))]
	MissingAppFile {
		package: String,
		path: String,
		stderr: String,
	},
	#[error(transparent)]
	JoinPaths(#[from] JoinPathsError),
	#[error(transparent)]
	Io(#[from] io::Error),
}

fn detail(stderr: &str) -> String {
	if stderr.is_empty() {
		String::new()
	} else {
		format!(": {stderr}")
	}
}

/// Where the Android SDK, NDK, and JDK live, and what to build for.
#[derive(Clone, Debug)]
pub struct Settings {
	/// `$ANDROID_HOME`, else `$ANDROID_SDK_ROOT`. Required.
	pub sdk: Option<PathBuf>,
	/// `$ANDROID_NDK_HOME`; `<sdk>/ndk/<ndk_version>` when unset.
	pub ndk: Option<PathBuf>,
	/// `$ANDROID_NDK_VERSION`, by default `27.1.12297006`.
	pub ndk_version: String,
	/// `$ANDROID_ABI`: `arm64-v8a` (the default) or `x86_64`.
	pub abi: String,
	/// `$ANDROID_API_LEVEL`, by default `35`: the platform the APK links
	/// against.
	pub api_level: String,
	/// `$ANDROID_BUILD_TOOLS_VERSION`, by default `36.0.0`.
	pub build_tools_version: String,
	/// `$JAVA_HOME`; asked of `java` when unset. Every tool runs with it, so
	/// kotlinc, D8, and keytool use the same JDK.
	pub java_home: Option<PathBuf>,
	/// `$CARGO_TARGET_DIR`, relative to the workspace root; `<root>/target`
	/// when unset.
	pub target_dir: Option<PathBuf>,
	/// `$ANDROID_SERIAL`: the dedicated device or emulator that
	/// [`Toolchain::device`] drives.
	pub serial: Option<String>,
	pub programs: Programs,
	pub kotlin_stdlib: PinnedJar,
}

impl Settings {
	/// Reads the settings from the environment variables each field names.
	pub fn from_env() -> Self {
		let text = |name: &str, default: &str| env::var(name).unwrap_or_else(|_| default.into());
		Self {
			sdk: env::var_os("ANDROID_HOME")
				.or_else(|| env::var_os("ANDROID_SDK_ROOT"))
				.map(PathBuf::from),
			ndk: env::var_os("ANDROID_NDK_HOME").map(PathBuf::from),
			ndk_version: text("ANDROID_NDK_VERSION", DEFAULT_NDK_VERSION),
			abi: text("ANDROID_ABI", DEFAULT_ABI),
			api_level: text("ANDROID_API_LEVEL", DEFAULT_API_LEVEL),
			build_tools_version: text("ANDROID_BUILD_TOOLS_VERSION", DEFAULT_BUILD_TOOLS_VERSION),
			java_home: env::var_os("JAVA_HOME").map(PathBuf::from),
			target_dir: env::var_os("CARGO_TARGET_DIR").map(PathBuf::from),
			serial: env::var("ANDROID_SERIAL").ok(),
			programs: Programs {
				cargo: env::var_os("CARGO").unwrap_or_else(|| "cargo".into()),
				java: "java".into(),
				kotlinc: "kotlinc".into(),
				curl: "curl".into(),
				zip: "zip".into(),
			},
			kotlin_stdlib: PinnedJar {
				url: KOTLIN_STDLIB_URL.into(),
				sha256: KOTLIN_STDLIB_SHA256.into(),
			},
		}
	}
}

/// The programs the build finds on `PATH` rather than in the SDK.
#[derive(Clone, Debug)]
pub struct Programs {
	/// `$CARGO`, so a verifier run by cargo builds with the same cargo.
	pub cargo: OsString,
	pub java: OsString,
	pub kotlinc: OsString,
	pub curl: OsString,
	pub zip: OsString,
}

/// A download cached beside the APK and checked against its SHA-256.
#[derive(Clone, Debug)]
pub struct PinnedJar {
	pub url: String,
	/// Lowercase hex.
	pub sha256: String,
}

impl PinnedJar {
	fn file_name(&self) -> &str {
		self.url
			.rsplit_once('/')
			.map_or(self.url.as_str(), |(_, name)| name)
	}
}

/// The device ABIs the verifiers build for.
#[derive(Clone, Copy, Debug)]
enum Abi {
	Arm64V8a,
	X86_64,
}

impl FromStr for Abi {
	type Err = Error;

	fn from_str(name: &str) -> Result<Self> {
		match name {
			"arm64-v8a" => Ok(Self::Arm64V8a),
			"x86_64" => Ok(Self::X86_64),
			_ => Err(Error::UnsupportedAbi(name.into())),
		}
	}
}

impl Abi {
	/// The APK's `lib/<name>` directory.
	fn name(self) -> &'static str {
		match self {
			Self::Arm64V8a => "arm64-v8a",
			Self::X86_64 => "x86_64",
		}
	}

	/// The NDK clang wrapper that links for API 28.
	fn linker(self) -> &'static str {
		match self {
			Self::Arm64V8a => "aarch64-linux-android28-clang",
			Self::X86_64 => "x86_64-linux-android28-clang",
		}
	}

	fn target(self) -> &'static str {
		match self {
			Self::Arm64V8a => "aarch64-linux-android",
			Self::X86_64 => "x86_64-linux-android",
		}
	}
}

/// The SDK, NDK, and JDK one verifier builds with. Every command it creates
/// runs in the workspace root with `JAVA_HOME` set.
#[derive(Debug)]
pub struct Toolchain {
	root: PathBuf,
	sdk: PathBuf,
	ndk: PathBuf,
	abi: Abi,
	api_level: String,
	build_tools: PathBuf,
	java_home: PathBuf,
	target_dir: PathBuf,
	serial: Option<String>,
	programs: Programs,
	kotlin_stdlib: PinnedJar,
}

/// One APK for [`Toolchain::package`].
#[derive(Clone, Copy, Debug)]
pub struct Apk<'a> {
	/// Names the class jar and the signed `<name>.apk`.
	pub name: &'a str,
	/// The manifest, relative to the workspace root.
	pub manifest: &'a str,
	/// The Kotlin sources, relative to the workspace root.
	pub sources: &'a [&'a str],
	/// The library name of the crate [`Toolchain::rustc`] built, packaged as
	/// `lib/<abi>/lib<library>.so`.
	pub library: &'a str,
	/// An absolute directory for the APK, its intermediate files, the cached
	/// Kotlin stdlib, and the debug keystore.
	pub output: &'a Path,
}

/// The SDK and JDK tools [`Toolchain::package`] runs, checked up front so a
/// missing one fails before anything slow starts.
struct BuildTools {
	platform: PathBuf,
	d8: PathBuf,
	aapt2: PathBuf,
	zipalign: PathBuf,
	apksigner: PathBuf,
	keytool: PathBuf,
}

impl Toolchain {
	/// Resolves `settings` against the workspace `root`. Asks `java` for its
	/// home when `JAVA_HOME` is unset.
	pub fn new(root: impl Into<PathBuf>, settings: Settings) -> Result<Self> {
		let root = root.into();
		let sdk = settings.sdk.ok_or(Error::MissingVariable {
			variable: "ANDROID_HOME",
			purpose: "the Android SDK",
		})?;
		let abi = settings.abi.parse()?;
		let java_home = match settings.java_home {
			Some(java_home) => java_home,
			None => java_home(&settings.programs.java)?,
		};
		Ok(Self {
			ndk: settings
				.ndk
				.unwrap_or_else(|| sdk.join("ndk").join(&settings.ndk_version)),
			build_tools: sdk.join("build-tools").join(&settings.build_tools_version),
			target_dir: absolute(
				&root,
				settings.target_dir.unwrap_or_else(|| root.join("target")),
			),
			root,
			sdk,
			abi,
			api_level: settings.api_level,
			java_home,
			serial: settings.serial,
			programs: settings.programs,
			kotlin_stdlib: settings.kotlin_stdlib,
		})
	}

	/// A command that runs `program` in the workspace root with this JDK.
	pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
		let mut command = Command::new(program);
		command
			.current_dir(&self.root)
			.env("JAVA_HOME", &self.java_home);
		command
	}

	/// `cargo rustc` for the library of `package`, linked by the NDK for the
	/// device ABI. `cargo_args` go before the `--`, and `rustc_args` after it,
	/// where they reach `package` alone.
	pub fn rustc(
		&self,
		package: &str,
		cargo_args: &[&str],
		rustc_args: &[&str],
	) -> Result<Command> {
		let linker = require(
			"the Android NDK linker",
			self.ndk
				.join("toolchains/llvm/prebuilt")
				.join(NDK_HOST)
				.join("bin")
				.join(self.abi.linker()),
		)?;
		let target = self.abi.target();
		let mut cargo = self.command(&self.programs.cargo);
		cargo
			.env(
				format!(
					"CARGO_TARGET_{}_LINKER",
					target.to_ascii_uppercase().replace('-', "_")
				),
				linker,
			)
			.args(["rustc", "--locked", "--package", package, "--lib"])
			.args(cargo_args)
			// Android 15 devices may use 16 KiB pages.
			.args([
				"--target",
				target,
				"--",
				"-C",
				"link-arg=-Wl,-z,max-page-size=16384",
			])
			.args(rustc_args);
		Ok(cargo)
	}

	/// Builds `<output>/<name>.apk` from the library [`Toolchain::rustc`]
	/// built and the Kotlin host, aligned and signed with a debug key, and
	/// returns its path.
	pub fn package(&self, apk: &Apk<'_>) -> Result<PathBuf> {
		let tools = self.build_tools()?;
		let output = apk.output;
		let library = format!("lib{}.so", apk.library);
		let native = output.join("lib").join(self.abi.name());
		fs::create_dir_all(&native)?;
		fs::copy(
			self.target_dir
				.join(self.abi.target())
				.join("debug")
				.join(&library),
			native.join(&library),
		)?;
		let stdlib = self.kotlin_stdlib(output)?;
		let jar = output.join(format!("{}.jar", apk.name));
		run(self
			.command(&self.programs.kotlinc)
			.args(apk.sources)
			.args([
				"-language-version",
				"2.1",
				"-api-version",
				"2.1",
				"-no-stdlib",
				"-classpath",
			])
			.arg(env::join_paths([&tools.platform, &stdlib])?)
			.arg("-d")
			.arg(&jar))?;
		let dex = self.dex(&tools, output, &jar, &stdlib)?;
		let unsigned = output.join("unsigned.apk");
		run(self
			.command(&tools.aapt2)
			.args(["link", "-I"])
			.arg(&tools.platform)
			.args(["--manifest", apk.manifest, "-o"])
			.arg(&unsigned))?;
		run(self
			.command(&self.programs.zip)
			.current_dir(output)
			.args(["-q", "-u", "unsigned.apk"])
			.arg(format!("lib/{}/{library}", self.abi.name())))?;
		run(self
			.command(&self.programs.zip)
			.args(["-q", "-j", "-u"])
			.arg(&unsigned)
			.args(dex))?;
		let aligned = output.join("aligned.apk");
		run(self
			.command(&tools.zipalign)
			.args(["-f", "-P", "16", "4"])
			.arg(&unsigned)
			.arg(&aligned))?;
		let keystore = output.join("debug.keystore");
		if !keystore.exists() {
			run(self
				.command(&tools.keytool)
				.args(["-genkeypair", "-keystore"])
				.arg(&keystore)
				.args([
					"-storepass",
					DEBUG_PASSWORD,
					"-keypass",
					DEBUG_PASSWORD,
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
		let signed = output.join(format!("{}.apk", apk.name));
		run(self
			.command(&tools.apksigner)
			.args(["sign", "--ks"])
			.arg(keystore)
			.args(["--ks-pass", &format!("pass:{DEBUG_PASSWORD}"), "--out"])
			.arg(&signed)
			.arg(aligned))?;
		Ok(signed)
	}

	/// The device at `$ANDROID_SERIAL`, driven through the SDK's adb.
	pub fn device(&self) -> Result<Device> {
		let serial = self.serial.clone().ok_or(Error::MissingVariable {
			variable: "ANDROID_SERIAL",
			purpose: "a dedicated device or emulator",
		})?;
		Ok(Device {
			adb: require("adb", self.sdk.join("platform-tools/adb"))?,
			serial,
			root: self.root.clone(),
		})
	}

	fn build_tools(&self) -> Result<BuildTools> {
		let tool = |name: &str| require(name, self.build_tools.join(name));
		Ok(BuildTools {
			platform: require(
				format!("the android-{} platform", self.api_level),
				self.sdk
					.join("platforms")
					.join(format!("android-{}", self.api_level))
					.join("android.jar"),
			)?,
			d8: tool("d8")?,
			aapt2: tool("aapt2")?,
			zipalign: tool("zipalign")?,
			apksigner: tool("apksigner")?,
			keytool: require("keytool", self.java_home.join("bin/keytool"))?,
		})
	}

	/// Downloads the pinned Kotlin stdlib into `directory` once, and checks
	/// it on every build.
	fn kotlin_stdlib(&self, directory: &Path) -> Result<PathBuf> {
		let stdlib = directory.join(self.kotlin_stdlib.file_name());
		if !stdlib.exists() {
			run(self
				.command(&self.programs.curl)
				.args([
					"--fail",
					"--silent",
					"--show-error",
					"--location",
					&self.kotlin_stdlib.url,
					"-o",
				])
				.arg(&stdlib))?;
		}
		let hash = format!("{:x}", Sha256::digest(fs::read(&stdlib)?));
		if hash != self.kotlin_stdlib.sha256 {
			fs::remove_file(&stdlib)?;
			return Err(Error::Checksum { path: stdlib });
		}
		Ok(stdlib)
	}

	/// Dexes `jar` and the stdlib into a fresh `<output>/dex` and returns the
	/// dex files in name order.
	fn dex(
		&self,
		tools: &BuildTools,
		output: &Path,
		jar: &Path,
		stdlib: &Path,
	) -> Result<Vec<PathBuf>> {
		let directory = output.join("dex");
		if directory.exists() {
			fs::remove_dir_all(&directory)?;
		}
		fs::create_dir_all(&directory)?;
		run(self
			.command(&tools.d8)
			.arg("--lib")
			.arg(&tools.platform)
			.args(["--min-api", MIN_API, "--output"])
			.arg(&directory)
			.arg(jar)
			.arg(stdlib))?;
		let mut files = fs::read_dir(&directory)?
			.map(|entry| entry.map(|entry| entry.path()))
			.collect::<io::Result<Vec<_>>>()?;
		files.retain(|path| path.extension().is_some_and(|value| value == "dex"));
		files.sort();
		if files.is_empty() {
			return Err(Error::NoDex { directory });
		}
		Ok(files)
	}
}

/// A device or emulator reached through `adb -s <serial>`.
#[derive(Debug)]
pub struct Device {
	adb: PathBuf,
	serial: String,
	root: PathBuf,
}

impl Device {
	/// An adb command addressed to this device.
	pub fn adb(&self) -> Command {
		let mut command = Command::new(&self.adb);
		command.current_dir(&self.root).args(["-s", &self.serial]);
		command
	}

	/// `adb shell run-as <package>`: the rest of the command runs as the app,
	/// in its data directory.
	pub fn run_as(&self, package: &str) -> Command {
		let mut command = self.adb();
		command.args(["shell", "run-as", package]);
		command
	}

	/// Replaces any install of `package` with `apk`, so the app starts with
	/// empty private storage, and clears the log so the journey's logs stand
	/// alone.
	pub fn install(&self, apk: &Path, package: &str) -> Result<()> {
		// A previous install signed with another debug key would refuse the
		// update. A missing package fails to uninstall, which is fine.
		capture(self.adb().args(["uninstall", package]))?;
		run(self.adb().args(["install", "--no-incremental"]).arg(apk))?;
		run(self.adb().args(["shell", "am", "force-stop", package]))?;
		run(self.adb().args(["logcat", "-c"]))
	}

	/// Checks for `path` in the app's data directory up to `attempts` times,
	/// `interval` apart, and reports whether it appeared.
	pub fn wait_for_app_file(
		&self,
		package: &str,
		path: &str,
		attempts: u32,
		interval: Duration,
	) -> Result<bool> {
		for _ in 0..attempts {
			if capture(self.run_as(package).args(["test", "-f", path]))?
				.status
				.success()
			{
				return Ok(true);
			}
			thread::sleep(interval);
		}
		Ok(false)
	}

	/// Reads `path` from the app's data directory. An empty or unreadable file
	/// is [`Error::MissingAppFile`], since `adb exec-out` does not report the
	/// exit status of the command it runs.
	pub fn read_app_file(&self, package: &str, path: &str) -> Result<Vec<u8>> {
		let output = capture(
			self.adb()
				.args(["exec-out", "run-as", package, "cat", path]),
		)?;
		if !output.status.success() || output.stdout.is_empty() {
			return Err(Error::MissingAppFile {
				package: package.into(),
				path: path.into(),
				stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
			});
		}
		Ok(output.stdout)
	}
}

/// Runs `command` with inherited output and fails unless it succeeds.
pub fn run(command: &mut Command) -> Result<()> {
	let status = command.status().map_err(|source| {
		Error::Start {
			command: describe(command),
			source,
		}
	})?;
	if !status.success() {
		return Err(Error::Failed {
			command: describe(command),
			status,
		});
	}
	Ok(())
}

/// Runs `command` and captures its output, whatever its exit status.
pub fn capture(command: &mut Command) -> Result<Output> {
	command.output().map_err(|source| {
		Error::Start {
			command: describe(command),
			source,
		}
	})
}

/// `path` itself when absolute, else `path` inside `root`.
pub fn absolute(root: &Path, path: PathBuf) -> PathBuf {
	if path.is_absolute() {
		path
	} else {
		root.join(path)
	}
}

/// The program and arguments of `command`, as a shell would show them.
fn describe(command: &Command) -> String {
	let mut text = command.get_program().to_string_lossy().into_owned();
	for argument in command.get_args() {
		text.push(' ');
		text.push_str(&argument.to_string_lossy());
	}
	text
}

/// `path`, if a tool is installed there.
fn require(tool: impl Into<String>, path: PathBuf) -> Result<PathBuf> {
	if path.is_file() {
		Ok(path)
	} else {
		Err(Error::MissingTool {
			tool: tool.into(),
			path,
		})
	}
}

/// Asks `java` for its `java.home`.
fn java_home(java: &OsStr) -> Result<PathBuf> {
	let mut command = Command::new(java);
	command.args(["-XshowSettings:properties", "-version"]);
	let output = capture(&mut command)?;
	if !output.status.success() {
		return Err(Error::Failed {
			command: describe(&command),
			status: output.status,
		});
	}
	String::from_utf8_lossy(&output.stderr)
		.lines()
		.find_map(|line| line.trim().strip_prefix("java.home = "))
		.map(PathBuf::from)
		.ok_or(Error::UnknownJavaHome)
}
