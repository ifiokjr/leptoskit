//! Exercises the APK pipeline against fake SDK, NDK, and JDK tools that log
//! their arguments, so the tests check every command the verifiers run
//! without an Android SDK or a device.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use android_apk::Error;
use android_apk::KOTLIN_STDLIB_SHA256;
use android_apk::KOTLIN_STDLIB_URL;
use android_apk::Programs;
use android_apk::Settings;
use android_apk::Toolchain;
use android_apk::absolute;
use android_apk::run;

/// Settings for an SDK under `directory`, with `java_home` set so no test
/// asks the host's `java`.
fn settings(directory: &Path) -> Settings {
	let bin = directory.join("bin");
	Settings {
		sdk: Some(directory.join("sdk")),
		ndk: None,
		ndk_version: "27.1.12297006".into(),
		abi: "arm64-v8a".into(),
		api_level: "35".into(),
		build_tools_version: "36.0.0".into(),
		java_home: Some(directory.join("jdk")),
		target_dir: Some("target".into()),
		serial: Some("emulator-5554".into()),
		programs: Programs {
			cargo: bin.join("cargo").into(),
			java: bin.join("java").into(),
			kotlinc: bin.join("kotlinc").into(),
			curl: bin.join("curl").into(),
			zip: bin.join("zip").into(),
		},
		kotlin_stdlib: android_apk::PinnedJar {
			url: "https://example.invalid/kotlin/kotlin-stdlib-test.jar".into(),
			sha256: "unused".into(),
		},
	}
}

#[test]
fn requires_the_sdk() {
	let directory = tempfile::tempdir().unwrap();
	let mut settings = settings(directory.path());
	settings.sdk = None;
	let error = Toolchain::new(directory.path(), settings).unwrap_err();
	assert!(matches!(
		error,
		Error::MissingVariable {
			variable: "ANDROID_HOME",
			..
		}
	));
	assert_eq!(error.to_string(), "set ANDROID_HOME to the Android SDK");
}

#[test]
fn rejects_an_unsupported_abi() {
	let directory = tempfile::tempdir().unwrap();
	let mut settings = settings(directory.path());
	settings.abi = "armeabi-v7a".into();
	let error = Toolchain::new(directory.path(), settings).unwrap_err();
	assert_eq!(
		error.to_string(),
		"unsupported Android ABI armeabi-v7a; use arm64-v8a or x86_64"
	);
}

#[test]
fn pins_the_kotlin_stdlib_by_default() {
	let settings = Settings::from_env();
	assert_eq!(settings.kotlin_stdlib.url, KOTLIN_STDLIB_URL);
	assert_eq!(settings.kotlin_stdlib.sha256, KOTLIN_STDLIB_SHA256);
	assert!(KOTLIN_STDLIB_URL.ends_with("/kotlin-stdlib-2.1.21.jar"));
}

#[test]
fn reports_a_program_that_cannot_start() {
	let directory = tempfile::tempdir().unwrap();
	let missing = directory.path().join("no-such-tool");
	let error = run(Command::new(&missing).arg("--flag")).unwrap_err();
	assert!(matches!(error, Error::Start { .. }));
	assert_eq!(
		error.to_string(),
		format!("could not start {} --flag", missing.display())
	);
}

#[test]
fn resolves_relative_paths_against_the_root() {
	let root = Path::new("/workspace");
	assert_eq!(
		absolute(root, PathBuf::from("target/out")),
		Path::new("/workspace/target/out")
	);
	let elsewhere = std::env::temp_dir();
	assert_eq!(absolute(root, elsewhere.clone()), elsewhere);
}

#[cfg(unix)]
mod fake {
	use std::fs;
	use std::os::unix::fs::PermissionsExt;
	use std::path::Path;
	use std::path::PathBuf;
	use std::sync::Mutex;
	use std::sync::MutexGuard;
	use std::time::Duration;

	use android_apk::Apk;
	use android_apk::Error;
	use android_apk::PinnedJar;
	use android_apk::Settings;
	use android_apk::Toolchain;
	use sha2::Digest;
	use sha2::Sha256;
	use tempfile::TempDir;

	/// Writing a script while another test thread forks can leave the
	/// script's descriptor open in the child, and executing it then fails
	/// with "text file busy". The fake-tool tests take turns.
	static TURN: Mutex<()> = Mutex::new(());

	const HOST: &str = if cfg!(target_os = "macos") {
		"darwin-x86_64"
	} else {
		"linux-x86_64"
	};
	/// Writes `$3` to the file after flag `$1` (with suffix `$2`).
	const WRITE_AFTER: &str = r#"write_after() {
	flag=$1; suffix=$2; content=$3; shift 3
	while [ $# -gt 1 ]; do
		if [ "$1" = "$flag" ]; then printf '%s' "$content" > "$2$suffix"; fi
		shift
	done
}
"#;
	const STDLIB: &str = "kotlin stdlib";

	struct Fixture {
		directory: TempDir,
		_turn: MutexGuard<'static, ()>,
	}

	impl Fixture {
		/// A workspace with a built `libprobe.so` and a complete fake SDK,
		/// NDK, JDK, and `PATH`.
		fn new() -> Self {
			let turn = TURN
				.lock()
				.unwrap_or_else(std::sync::PoisonError::into_inner);
			let fixture = Self {
				directory: tempfile::tempdir().unwrap(),
				_turn: turn,
			};
			fixture.file(
				"workspace/target/aarch64-linux-android/debug/libprobe.so",
				"elf",
			);
			fixture.file("sdk/platforms/android-35/android.jar", "platform");
			fixture.file(
				&format!(
					"sdk/ndk/27.1.12297006/toolchains/llvm/prebuilt/{HOST}/bin/\
					 aarch64-linux-android28-clang"
				),
				"",
			);
			fixture.file(
				&format!(
					"sdk/ndk/27.1.12297006/toolchains/llvm/prebuilt/{HOST}/bin/\
					 x86_64-linux-android28-clang"
				),
				"",
			);
			fixture.tool("bin/curl", r#"write_after -o "" "kotlin stdlib" "$@""#);
			fixture.tool("bin/kotlinc", r#"write_after -d "" jar "$@""#);
			fixture.tool("bin/zip", "");
			fixture.tool(
				"sdk/build-tools/36.0.0/d8",
				r#"write_after --output /classes.dex dex "$@""#,
			);
			fixture.tool(
				"sdk/build-tools/36.0.0/aapt2",
				r#"write_after -o "" apk "$@""#,
			);
			fixture.tool(
				"sdk/build-tools/36.0.0/zipalign",
				r#"for last; do :; done; printf aligned > "$last""#,
			);
			fixture.tool(
				"sdk/build-tools/36.0.0/apksigner",
				r#"write_after --out "" signed "$@""#,
			);
			fixture.tool("jdk/bin/keytool", r#"write_after -keystore "" key "$@""#);
			fixture.tool("sdk/platform-tools/adb", "");
			fixture
		}

		fn path(&self, relative: &str) -> PathBuf {
			self.directory.path().join(relative)
		}

		fn file(&self, relative: &str, contents: &str) {
			let path = self.path(relative);
			fs::create_dir_all(path.parent().unwrap()).unwrap();
			fs::write(path, contents).unwrap();
		}

		/// An executable that logs its name and arguments, then runs `body`.
		fn tool(&self, relative: &str, body: &str) {
			let name = Path::new(relative).file_name().unwrap().to_str().unwrap();
			let log = self.path("log.txt");
			self.file(
				relative,
				&format!(
					"#!/bin/sh\n{WRITE_AFTER}printf '%s\\n' \"{name} $*\" >> '{}'\n{body}\n",
					log.display()
				),
			);
			fs::set_permissions(self.path(relative), fs::Permissions::from_mode(0o755)).unwrap();
		}

		/// The logged invocations, with the fixture directory as `<tmp>`.
		fn log(&self) -> Vec<String> {
			let log = fs::read_to_string(self.path("log.txt")).unwrap_or_default();
			let root = self.directory.path().to_str().unwrap();
			log.lines()
				.map(|line| line.replace(root, "<tmp>"))
				.collect()
		}

		fn clear_log(&self) {
			let _ = fs::remove_file(self.path("log.txt"));
		}

		fn settings(&self) -> Settings {
			let mut settings = super::settings(self.directory.path());
			settings.kotlin_stdlib = PinnedJar {
				url: "https://example.invalid/kotlin/kotlin-stdlib-test.jar".into(),
				sha256: format!("{:x}", Sha256::digest(STDLIB)),
			};
			settings
		}

		fn toolchain(&self) -> Toolchain {
			Toolchain::new(self.path("workspace"), self.settings()).unwrap()
		}

		fn package(&self) -> Result<PathBuf, Error> {
			let output = self.path("out");
			self.toolchain().package(&Apk {
				name: "probe",
				manifest: "android/AndroidManifest.xml",
				sources: &["android/Host.kt", "android/ProbeActivity.kt"],
				library: "probe",
				output: &output,
			})
		}
	}

	#[test]
	fn packages_a_signed_apk_with_the_sdk_tools() {
		let fixture = Fixture::new();
		let apk = fixture.package().unwrap();
		assert_eq!(apk, fixture.path("out/probe.apk"));
		assert_eq!(fs::read_to_string(&apk).unwrap(), "signed");
		assert_eq!(
			fs::read_to_string(fixture.path("out/lib/arm64-v8a/libprobe.so")).unwrap(),
			"elf"
		);
		assert_eq!(
			fixture.log(),
			[
				"curl --fail --silent --show-error --location \
			 https://example.invalid/kotlin/kotlin-stdlib-test.jar -o <tmp>/out/kotlin-stdlib-test.jar",
				"kotlinc android/Host.kt android/ProbeActivity.kt -language-version 2.1 -api-version 2.1 \
			 -no-stdlib -classpath \
			 <tmp>/sdk/platforms/android-35/android.jar:<tmp>/out/kotlin-stdlib-test.jar -d \
			 <tmp>/out/probe.jar",
				"d8 --lib <tmp>/sdk/platforms/android-35/android.jar --min-api 28 --output <tmp>/out/dex \
			 <tmp>/out/probe.jar <tmp>/out/kotlin-stdlib-test.jar",
				"aapt2 link -I <tmp>/sdk/platforms/android-35/android.jar --manifest \
			 android/AndroidManifest.xml -o <tmp>/out/unsigned.apk",
				"zip -q -u unsigned.apk lib/arm64-v8a/libprobe.so",
				"zip -q -j -u <tmp>/out/unsigned.apk <tmp>/out/dex/classes.dex",
				"zipalign -f -P 16 4 <tmp>/out/unsigned.apk <tmp>/out/aligned.apk",
				"keytool -genkeypair -keystore <tmp>/out/debug.keystore -storepass android -keypass \
			 android -alias androiddebugkey -dname CN=Android Debug,O=Android,C=US -keyalg RSA \
			 -validity 10000",
				"apksigner sign --ks <tmp>/out/debug.keystore --ks-pass pass:android --out \
			 <tmp>/out/probe.apk <tmp>/out/aligned.apk",
			]
		);
	}

	#[test]
	fn reuses_the_cached_stdlib_and_keystore() {
		let fixture = Fixture::new();
		fixture.package().unwrap();
		fixture.file("out/dex/stale.dex", "stale");
		fixture.clear_log();
		fixture.package().unwrap();
		let log = fixture.log();
		assert!(!log.iter().any(|line| line.starts_with("curl ")), "{log:?}");
		assert!(
			!log.iter().any(|line| line.starts_with("keytool ")),
			"{log:?}"
		);
		assert!(
			log.contains(&"zip -q -j -u <tmp>/out/unsigned.apk <tmp>/out/dex/classes.dex".into()),
			"a stale dex file was packaged: {log:?}"
		);
	}

	#[test]
	fn reports_a_missing_build_tool_before_building() {
		let fixture = Fixture::new();
		fs::remove_file(fixture.path("sdk/build-tools/36.0.0/apksigner")).unwrap();
		let error = fixture.package().unwrap_err();
		assert!(matches!(&error, Error::MissingTool { tool, .. } if tool == "apksigner"));
		assert_eq!(
			error.to_string(),
			format!(
				"apksigner is missing at {}",
				fixture.path("sdk/build-tools/36.0.0/apksigner").display()
			)
		);
		assert!(fixture.log().is_empty(), "{:?}", fixture.log());
	}

	#[test]
	fn reports_a_missing_platform() {
		let fixture = Fixture::new();
		fs::remove_dir_all(fixture.path("sdk/platforms")).unwrap();
		let error = fixture.package().unwrap_err();
		assert!(
			matches!(&error, Error::MissingTool { tool, .. } if tool == "the android-35 platform")
		);
	}

	#[test]
	fn reports_a_missing_keytool() {
		let fixture = Fixture::new();
		fs::remove_file(fixture.path("jdk/bin/keytool")).unwrap();
		let error = fixture.package().unwrap_err();
		assert!(matches!(&error, Error::MissingTool { tool, .. } if tool == "keytool"));
	}

	#[test]
	fn removes_a_stdlib_that_fails_its_checksum() {
		let fixture = Fixture::new();
		fixture.file("out/kotlin-stdlib-test.jar", "tampered");
		let error = fixture.package().unwrap_err();
		assert!(matches!(&error, Error::Checksum { path } if !path.exists()));
		assert!(fixture.log().is_empty(), "{:?}", fixture.log());
	}

	#[test]
	fn fails_when_d8_writes_no_dex() {
		let fixture = Fixture::new();
		fixture.tool("sdk/build-tools/36.0.0/d8", "");
		let error = fixture.package().unwrap_err();
		assert!(matches!(&error, Error::NoDex { directory } if directory.ends_with("out/dex")));
	}

	#[test]
	fn reports_the_tool_that_failed() {
		let fixture = Fixture::new();
		fixture.tool("sdk/build-tools/36.0.0/aapt2", "exit 3");
		let error = fixture.package().unwrap_err();
		let Error::Failed { command, status } = &error else {
			panic!("expected a failed command, got {error:?}");
		};
		assert_eq!(status.code(), Some(3));
		assert!(
			command.contains("/build-tools/36.0.0/aapt2 link -I "),
			"{command}"
		);
	}

	#[test]
	fn builds_the_library_with_the_ndk_linker() {
		let fixture = Fixture::new();
		let cargo = fixture
			.toolchain()
			.rustc(
				"probe",
				&["--features", "probe"],
				&["-C", "instrument-coverage"],
			)
			.unwrap();
		assert_eq!(cargo.get_program(), fixture.path("bin/cargo"));
		assert_eq!(
			cargo.get_current_dir(),
			Some(fixture.path("workspace").as_path())
		);
		let args: Vec<_> = cargo.get_args().map(|arg| arg.to_str().unwrap()).collect();
		assert_eq!(
			args,
			[
				"rustc",
				"--locked",
				"--package",
				"probe",
				"--lib",
				"--features",
				"probe",
				"--target",
				"aarch64-linux-android",
				"--",
				"-C",
				"link-arg=-Wl,-z,max-page-size=16384",
				"-C",
				"instrument-coverage",
			]
		);
		let envs: Vec<_> = cargo
			.get_envs()
			.map(|(key, value)| (key.to_str().unwrap(), value.map(Path::new)))
			.collect();
		let linker = fixture.path(&format!(
			"sdk/ndk/27.1.12297006/toolchains/llvm/prebuilt/{HOST}/bin/\
			 aarch64-linux-android28-clang"
		));
		assert!(envs.contains(&(
			"CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER",
			Some(linker.as_path())
		)));
		assert!(envs.contains(&("JAVA_HOME", Some(fixture.path("jdk").as_path()))));
	}

	#[test]
	fn builds_x86_64_with_its_own_linker_and_lib_directory() {
		let fixture = Fixture::new();
		fixture.file(
			"workspace/target/x86_64-linux-android/debug/libprobe.so",
			"x86",
		);
		let mut settings = fixture.settings();
		settings.abi = "x86_64".into();
		let toolchain = Toolchain::new(fixture.path("workspace"), settings).unwrap();
		let cargo = toolchain.rustc("probe", &[], &[]).unwrap();
		assert!(
			cargo
				.get_envs()
				.any(|(key, _)| key == "CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER")
		);
		assert!(cargo.get_args().any(|arg| arg == "x86_64-linux-android"));
		let output = fixture.path("out");
		toolchain
			.package(&Apk {
				name: "probe",
				manifest: "android/AndroidManifest.xml",
				sources: &["android/Host.kt"],
				library: "probe",
				output: &output,
			})
			.unwrap();
		assert_eq!(
			fs::read_to_string(fixture.path("out/lib/x86_64/libprobe.so")).unwrap(),
			"x86"
		);
		assert!(
			fixture
				.log()
				.contains(&"zip -q -u unsigned.apk lib/x86_64/libprobe.so".into())
		);
	}

	#[test]
	fn reports_a_missing_ndk_linker() {
		let fixture = Fixture::new();
		fs::remove_dir_all(fixture.path("sdk/ndk")).unwrap();
		let error = fixture.toolchain().rustc("probe", &[], &[]).unwrap_err();
		assert!(
			matches!(&error, Error::MissingTool { tool, .. } if tool == "the Android NDK linker")
		);
	}

	#[test]
	fn uses_an_explicit_ndk() {
		let fixture = Fixture::new();
		fs::rename(fixture.path("sdk/ndk/27.1.12297006"), fixture.path("ndk")).unwrap();
		let mut settings = fixture.settings();
		settings.ndk = Some(fixture.path("ndk"));
		let toolchain = Toolchain::new(fixture.path("workspace"), settings).unwrap();
		assert!(toolchain.rustc("probe", &[], &[]).is_ok());
	}

	#[test]
	fn asks_java_for_its_home() {
		let fixture = Fixture::new();
		fixture.tool(
			"bin/java",
			"printf 'Property settings:\\n    java.home = /opt/jdk-17\\n' >&2",
		);
		let mut settings = fixture.settings();
		settings.java_home = None;
		let toolchain = Toolchain::new(fixture.path("workspace"), settings).unwrap();
		let command = toolchain.command("true");
		assert!(
			command
				.get_envs()
				.any(|(key, value)| key == "JAVA_HOME" && value == Some("/opt/jdk-17".as_ref()))
		);
		assert_eq!(fixture.log(), ["java -XshowSettings:properties -version"]);
	}

	#[test]
	fn reports_a_java_without_a_home() {
		let fixture = Fixture::new();
		fixture.tool("bin/java", "echo 'openjdk 17' >&2");
		let mut settings = fixture.settings();
		settings.java_home = None;
		let error = Toolchain::new(fixture.path("workspace"), settings).unwrap_err();
		assert!(matches!(error, Error::UnknownJavaHome));
	}

	#[test]
	fn reports_a_failing_java() {
		let fixture = Fixture::new();
		fixture.tool("bin/java", "exit 1");
		let mut settings = fixture.settings();
		settings.java_home = None;
		let error = Toolchain::new(fixture.path("workspace"), settings).unwrap_err();
		assert!(
			matches!(&error, Error::Failed { command, .. } if command.ends_with("java -XshowSettings:properties -version"))
		);
	}

	#[test]
	fn requires_a_device_serial() {
		let fixture = Fixture::new();
		let mut settings = fixture.settings();
		settings.serial = None;
		let error = Toolchain::new(fixture.path("workspace"), settings)
			.unwrap()
			.device()
			.unwrap_err();
		assert_eq!(
			error.to_string(),
			"set ANDROID_SERIAL to a dedicated device or emulator"
		);
	}

	#[test]
	fn requires_adb() {
		let fixture = Fixture::new();
		fs::remove_file(fixture.path("sdk/platform-tools/adb")).unwrap();
		let error = fixture.toolchain().device().unwrap_err();
		assert!(matches!(&error, Error::MissingTool { tool, .. } if tool == "adb"));
	}

	#[test]
	fn installs_over_any_previous_install() {
		let fixture = Fixture::new();
		// Nothing is installed yet, so uninstalling fails.
		fixture.tool(
			"sdk/platform-tools/adb",
			r#"case "$*" in *" uninstall "*) exit 1 ;; esac"#,
		);
		let device = fixture.toolchain().device().unwrap();
		device
			.install(&fixture.path("out/probe.apk"), "dev.example.probe")
			.unwrap();
		assert_eq!(
			fixture.log(),
			[
				"adb -s emulator-5554 uninstall dev.example.probe",
				"adb -s emulator-5554 install --no-incremental <tmp>/out/probe.apk",
				"adb -s emulator-5554 shell am force-stop dev.example.probe",
				"adb -s emulator-5554 logcat -c",
			]
		);
	}

	#[test]
	fn reports_a_failed_install() {
		let fixture = Fixture::new();
		fixture.tool(
			"sdk/platform-tools/adb",
			r#"case "$*" in *" install "*) echo INSTALL_FAILED_INSUFFICIENT_STORAGE >&2; exit 1 ;; esac"#,
		);
		let device = fixture.toolchain().device().unwrap();
		let error = device
			.install(&fixture.path("out/probe.apk"), "dev.example.probe")
			.unwrap_err();
		assert!(
			matches!(&error, Error::Failed { command, .. } if command.contains("-s emulator-5554 install --no-incremental")),
			"{error}"
		);
		assert!(
			!fixture.log().iter().any(|line| line.contains("force-stop")),
			"the journey went on after a failed install"
		);
	}

	#[test]
	fn reads_a_file_the_app_wrote() {
		let fixture = Fixture::new();
		fixture.tool(
			"sdk/platform-tools/adb",
			r#"case "$*" in *" cat files/verdict.txt") printf 'VERIFIED\n' ;; esac"#,
		);
		let device = fixture.toolchain().device().unwrap();
		assert_eq!(
			device
				.read_app_file("dev.example.probe", "files/verdict.txt")
				.unwrap(),
			b"VERIFIED\n"
		);
		assert_eq!(
			fixture.log(),
			["adb -s emulator-5554 exec-out run-as dev.example.probe cat files/verdict.txt"]
		);
	}

	#[test]
	fn reports_a_missing_verdict() {
		let fixture = Fixture::new();
		let device = fixture.toolchain().device().unwrap();
		let error = device
			.read_app_file("dev.example.probe", "files/verdict.txt")
			.unwrap_err();
		assert_eq!(
			error.to_string(),
			"dev.example.probe has no files/verdict.txt on the device"
		);
	}

	#[test]
	fn reports_a_missing_profile_with_the_device_error() {
		let fixture = Fixture::new();
		fixture.tool(
			"sdk/platform-tools/adb",
			"echo 'cat: files/coverage.profraw: No such file or directory' >&2; exit 1",
		);
		let device = fixture.toolchain().device().unwrap();
		let error = device
			.read_app_file("dev.example.probe", "files/coverage.profraw")
			.unwrap_err();
		assert_eq!(
			error.to_string(),
			"dev.example.probe has no files/coverage.profraw on the device: cat: \
			 files/coverage.profraw: No such file or directory"
		);
	}

	#[test]
	fn waits_for_a_file_the_app_writes_later() {
		let fixture = Fixture::new();
		let count = fixture.path("checks");
		fixture.tool(
			"sdk/platform-tools/adb",
			&format!(
				"n=$(cat '{0}' 2>/dev/null || echo 0); n=$((n + 1)); echo $n > '{0}'; [ $n -ge 3 ]",
				count.display()
			),
		);
		let device = fixture.toolchain().device().unwrap();
		assert!(
			device
				.wait_for_app_file("dev.example.probe", "files/verdict.txt", 5, Duration::ZERO)
				.unwrap()
		);
		assert_eq!(
			fixture.log(),
			["adb -s emulator-5554 shell run-as dev.example.probe test -f files/verdict.txt"; 3]
		);
	}

	#[test]
	fn stops_waiting_after_the_last_attempt() {
		let fixture = Fixture::new();
		fixture.tool("sdk/platform-tools/adb", "exit 1");
		let device = fixture.toolchain().device().unwrap();
		assert!(
			!device
				.wait_for_app_file("dev.example.probe", "files/verdict.txt", 2, Duration::ZERO)
				.unwrap()
		);
		assert_eq!(fixture.log().len(), 2);
	}

	#[test]
	fn runs_app_commands_as_the_package() {
		let fixture = Fixture::new();
		let device = fixture.toolchain().device().unwrap();
		android_apk::run(
			device
				.run_as("dev.example.probe")
				.args(["rm", "-f", "files/a"]),
		)
		.unwrap();
		assert_eq!(
			fixture.log(),
			["adb -s emulator-5554 shell run-as dev.example.probe rm -f files/a"]
		);
	}
}
