//! `leptoskit doctor` against a controlled `PATH`: every tool is a stub whose
//! version output, exit status, and files are fixture data, so each check can
//! report a pass, a warning, or a required failure without the host's real
//! toolchain.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

/// Where the cargo metadata check finds its client crate.
const CLIENT: &str = r#"{"packages":[{"name":"app","manifest_path":"/tmp/app/Cargo.toml","targets":[{"name":"app","kind":["cdylib"]}],"features":{"devtools":["leptoskit/devtools"]}}],"target_directory":"/tmp/app/target","workspace_root":"/tmp/app"}"#;

/// A scratch project plus the `bin/` directory doctor searches first.
struct Lab {
	root: tempfile::TempDir,
}

impl Lab {
	fn new() -> Self {
		let root = tempfile::tempdir().expect("scratch project");
		fs::create_dir_all(root.path().join("bin")).expect("stub bin");
		Self { root }
	}

	fn path(&self) -> &Path {
		self.root.path()
	}

	fn bin(&self) -> PathBuf {
		self.path().join("bin")
	}

	/// Writes a shell stub for `name` and makes it executable.
	fn tool(&self, name: &str, body: &str) {
		let path = self.bin().join(name);
		fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).expect("stub");
		fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable stub");
	}

	/// Writes a stub that prints `line` for every invocation.
	fn reports(&self, name: &str, line: &str) {
		self.tool(name, &format!("printf '%s\\n' '{line}'"));
	}

	/// Writes the project's `leptoskit.toml`.
	fn config(&self, text: &str) {
		fs::write(self.path().join("leptoskit.toml"), text).expect("config");
	}

	/// Writes a `Cargo.lock` document.
	fn lockfile(&self, text: &str) {
		fs::write(self.path().join("Cargo.lock"), text).expect("lockfile");
	}

	/// Creates `$SYSROOT/lib/rustlib/<target>`, which `rust_target_installed`
	/// reads instead of asking rustup.
	fn rust_target(&self, target: &str) {
		fs::create_dir_all(self.path().join("sysroot/lib/rustlib").join(target))
			.expect("rust target");
	}

	/// A rustc that answers `--print sysroot` with this lab's sysroot.
	fn rustc(&self) {
		let sysroot = self.path().join("sysroot");
		self.tool(
			"rustc",
			&format!(
				"if [ \"$1\" = --print ]; then printf '%s\\n' '{}'; else printf '%s\\n' 'rustc \
				 1.90.0 (fixture)'; fi",
				sysroot.display()
			),
		);
	}

	/// An Android SDK under the lab's `sdk/` with every piece doctor checks.
	fn android_sdk(&self) -> PathBuf {
		let sdk = self.path().join("sdk");
		fs::create_dir_all(sdk.join("platforms/android-36")).expect("platform");
		fs::create_dir_all(sdk.join("cmake/3.31.11")).expect("cmake");
		fs::create_dir_all(sdk.join("ndk/27.2.12479018")).expect("ndk");
		fs::create_dir_all(sdk.join("platform-tools")).expect("platform tools");
		fs::write(
			sdk.join("ndk/27.2.12479018/source.properties"),
			"Pkg.Desc = Android NDK\n",
		)
		.expect("ndk properties");
		fs::write(sdk.join("platform-tools/adb"), "#!/bin/sh\n").expect("adb");
		sdk
	}

	/// A cargo stub whose `metadata --no-deps` answer carries `features`.
	fn cargo_metadata(&self, features: &str) {
		let metadata = CLIENT.replace(
			r#""features":{"devtools":["leptoskit/devtools"]}"#,
			&format!(r#""features":{features}"#),
		);
		self.tool(
			"cargo",
			&format!(
				"if [ \"$1\" = metadata ]; then printf '%s' '{metadata}'; else printf '%s\\n' \
				 'cargo 1.90.0 (fixture)'; fi"
			),
		);
	}

	/// Runs `leptoskit doctor <args>` with exactly `env` on top of a minimal
	/// environment, so no host tool or variable changes the report.
	fn doctor(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
		let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit"));
		command
			.arg("doctor")
			.args(args)
			.current_dir(self.path())
			.env_clear()
			.env("PATH", self.bin())
			.env("HOME", self.path());
		for (key, value) in env {
			command.env(key, value);
		}
		// `env_clear` would otherwise drop the coverage profile path, and the
		// checks this test drives would never be recorded.
		if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
			command.env("LLVM_PROFILE_FILE", profile);
		}
		command.output().expect("run leptoskit doctor")
	}
}

fn text(bytes: &[u8]) -> String {
	String::from_utf8_lossy(bytes).into_owned()
}

fn succeeds(output: &Output) {
	assert!(
		output.status.success(),
		"stdout:\n{}\nstderr:\n{}",
		text(&output.stdout),
		text(&output.stderr)
	);
}

fn fails(output: &Output) {
	assert!(
		!output.status.success(),
		"stdout:\n{}\nstderr:\n{}",
		text(&output.stdout),
		text(&output.stderr)
	);
}

fn carries(output: &Output, expected: &str) {
	assert!(
		text(&output.stdout).contains(expected),
		"missing {expected:?}\nstdout:\n{}\nstderr:\n{}",
		text(&output.stdout),
		text(&output.stderr)
	);
}

fn complains(output: &Output, expected: &str) {
	assert!(
		text(&output.stderr).contains(expected),
		"missing {expected:?}\nstdout:\n{}\nstderr:\n{}",
		text(&output.stdout),
		text(&output.stderr)
	);
}

/// A project that selects every platform this module checks, with the
/// extension tools present.
fn everything() -> Lab {
	let lab = Lab::new();
	lab.config(
		"[app]\nname = \"app\"\nplatforms = [\"web\", \"ios\", \"macos\", \"android\", \"linux\", \
		 \"harmonyos\"]\n[extension]\nbrowsers = [\"chromium\"]\n[extension.popup]\nroute = \
		 \"/\"\n",
	);
	lab.android_sdk();
	let hvigorw = lab.path().join("hvigorw");
	fs::write(&hvigorw, "#!/bin/sh\n").expect("hvigorw");
	for file in ["chrome", "firefox", "geckodriver"] {
		fs::write(lab.path().join(file), "#!/bin/sh\n").expect("browser stub");
	}
	for target in [
		"wasm32-unknown-unknown",
		"aarch64-apple-ios",
		"aarch64-apple-ios-sim",
		"aarch64-linux-android",
		"x86_64-linux-android",
	] {
		lab.rust_target(target);
	}
	lab.reports("cargo", "cargo 1.90.0 (fixture)");
	lab.rustc();
	lab.reports("wasm-opt", "wasm-opt version 123 (fixture)");
	lab.reports("swift", "Swift version 6.1 (fixture)");
	lab.reports("xcodebuild", "Xcode 18.0");
	lab.tool(
		"xcrun",
		"if [ \"$1\" = simctl ]; then printf '%s\\n' '-- iOS 18.0 (22A3351) --'; fi",
	);
	lab.reports("pkg-config", "4.10.0");
	lab.tool(
		"java",
		"printf '%s\\n' 'openjdk version \"21.0.2\" 2024-01-16' >&2",
	);
	lab.reports("ohpm", "ohpm 5.0.0");
	lab.reports("hvigorw", "hvigor 5.0.0");
	lab
}

#[cfg(target_os = "macos")]
#[test]
fn every_check_passes_with_the_tools_a_full_project_needs() {
	let lab = everything();
	let sdk = lab.path().join("sdk");
	let hvigorw = lab.path().join("hvigorw");
	let chrome = lab.path().join("chrome");
	let firefox = lab.path().join("firefox");
	let geckodriver = lab.path().join("geckodriver");
	let output = lab.doctor(
		&[],
		&[
			("ANDROID_HOME", sdk.to_str().unwrap()),
			("HVIGORW", hvigorw.to_str().unwrap()),
			("LEPTOSKIT_CHROME", chrome.to_str().unwrap()),
			("LEPTOSKIT_FIREFOX", firefox.to_str().unwrap()),
			("LEPTOSKIT_GECKODRIVER", geckodriver.to_str().unwrap()),
		],
	);
	succeeds(&output);
	for row in [
		"✔ cargo",
		"✔ rustc",
		"✔ wasm32-unknown-unknown",
		"✔ wasm-opt",
		"✔ project wasm-bindgen",
		"✔ Chrome for Testing",
		"✔ Firefox",
		"✔ geckodriver",
		"✔ iOS: Xcode",
		"✔ iOS: simulator runtime",
		"✔ iOS: aarch64-apple-ios",
		"✔ iOS: aarch64-apple-ios-sim",
		"✔ macOS: swift",
		"✔ Android: Java",
		"✔ Android: SDK",
		"✔ Android: platform",
		"✔ Android: NDK",
		"✔ Android: CMake",
		"✔ Android: aarch64-linux-android",
		"✔ Android: x86_64-linux-android",
		"✔ Android: platform tools",
		"✔ HarmonyOS: hvigorw",
		"✔ HarmonyOS: ohpm",
		"✔ Linux: pkg-config",
		"✔ Linux: GTK 4",
	] {
		carries(&output, row);
	}
	carries(&output, "no Cargo.lock yet; `leptoskit build` creates one");
}

#[test]
fn one_required_failure_reports_the_singular_restart_condition() {
	let lab = Lab::new();
	lab.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"macos\"]\n");
	lab.rust_target("wasm32-unknown-unknown");
	lab.reports("cargo", "cargo 1.90.0 (fixture)");
	lab.rustc();
	// `swift` is deliberately absent.
	let output = lab.doctor(&[], &[]);
	fails(&output);
	carries(&output, "✖ macOS: swift");
	complains(
		&output,
		"1 required check failed; builds will not work until it passes\n",
	);
}

#[test]
fn optional_tools_warn_without_failing_the_run() {
	let lab = Lab::new();
	lab.config("[app]\nname = \"app\"\nplatforms = [\"web\"]\n");
	lab.rust_target("wasm32-unknown-unknown");
	lab.reports("cargo", "cargo 1.90.0 (fixture)");
	lab.rustc();
	let output = lab.doctor(&[], &[]);
	succeeds(&output);
	carries(&output, "▲ wasm-opt");
	carries(
		&output,
		"release builds need it unless `bundle.wasm_opt = false`",
	);
}

#[test]
fn android_reports_missing_sdk_platform_ndk_and_cmake_before_any_build() {
	let lab = Lab::new();
	lab.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"android\"]\n");
	lab.rust_target("wasm32-unknown-unknown");
	lab.rust_target("aarch64-linux-android");
	lab.reports("cargo", "cargo 1.90.0 (fixture)");
	lab.rustc();
	lab.tool("java", "printf '%s\\n' 'openjdk version \"21.0.2\"' >&2");
	let output = lab.doctor(&[], &[]);
	fails(&output);
	carries(&output, "✖ Android: SDK");
	carries(
		&output,
		"missing: set ANDROID_HOME (or ANDROID_SDK_ROOT) to the SDK",
	);
	carries(&output, "missing android-36; install it with `sdkmanager");
	carries(&output, "install an NDK, or set ANDROID_NDK_HOME");
	carries(&output, "install it with `sdkmanager \"cmake;3.31.11\"`");
	carries(&output, "▲ Android: x86_64-linux-android");
	carries(&output, "▲ Android: platform tools");
	complains(&output, "4 required checks failed");
}

#[test]
fn an_android_sdk_without_a_platform_or_cmake_versions_is_reported() {
	let lab = Lab::new();
	lab.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"android\"]\n");
	lab.rust_target("wasm32-unknown-unknown");
	lab.rust_target("aarch64-linux-android");
	lab.rust_target("x86_64-linux-android");
	lab.reports("cargo", "cargo 1.90.0 (fixture)");
	lab.rustc();
	lab.tool("java", "printf '%s\\n' 'openjdk version \"21.0.2\"' >&2");
	let sdk = lab.path().join("sdk");
	fs::create_dir_all(sdk.join("cmake")).expect("empty cmake");
	fs::create_dir_all(sdk.join("ndk/27.0.0")).expect("ndk");
	fs::write(sdk.join("ndk/27.0.0/source.properties"), "fixture\n").expect("properties");
	let output = lab.doctor(&[], &[("ANDROID_HOME", sdk.to_str().unwrap())]);
	fails(&output);
	carries(&output, "the SDK's cmake directory is empty");
	carries(&output, "missing android-36");
	carries(
		&output,
		"missing; `leptoskit dev` needs adb from platform-tools",
	);
}

#[test]
fn an_old_java_and_gtk_are_named_with_their_required_versions() {
	let lab = Lab::new();
	lab.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"android\", \"linux\"]\n");
	lab.rust_target("wasm32-unknown-unknown");
	lab.rust_target("aarch64-linux-android");
	lab.rust_target("x86_64-linux-android");
	lab.reports("cargo", "cargo 1.90.0 (fixture)");
	lab.rustc();
	let sdk = lab.android_sdk();
	lab.tool("java", "printf '%s\\n' 'openjdk version \"1.8.0_392\"' >&2");
	lab.reports("pkg-config", "4.4.0");
	let output = lab.doctor(&[], &[("ANDROID_HOME", sdk.to_str().unwrap())]);
	fails(&output);
	carries(&output, "Java 17 or newer is required; this is 1.8.0_392");
	carries(&output, "GTK 4.6 or newer is required; this is 4.4.0");
	complains(&output, "2 required checks failed");
}

#[test]
fn a_java_probe_that_exits_nonzero_is_reported_with_its_status() {
	let lab = Lab::new();
	lab.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"android\"]\n");
	lab.rust_target("wasm32-unknown-unknown");
	lab.rust_target("aarch64-linux-android");
	lab.rust_target("x86_64-linux-android");
	lab.reports("cargo", "cargo 1.90.0 (fixture)");
	lab.rustc();
	let sdk = lab.android_sdk();
	lab.tool(
		"java",
		"printf '%s\\n' 'openjdk version \"21.0.2\"' >&2; exit 1",
	);
	let output = lab.doctor(&[], &[("ANDROID_HOME", sdk.to_str().unwrap())]);
	fails(&output);
	carries(&output, "`java -version` exited with");
}

#[test]
fn linux_reports_a_missing_or_failing_gtk_pkg_config() {
	let missing = Lab::new();
	missing.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"linux\"]\n");
	missing.rust_target("wasm32-unknown-unknown");
	missing.reports("cargo", "cargo 1.90.0 (fixture)");
	missing.rustc();
	let output = missing.doctor(&[], &[]);
	fails(&output);
	carries(&output, "✖ Linux: pkg-config");
	carries(&output, "✖ Linux: GTK 4");

	let broken = Lab::new();
	broken.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"linux\"]\n");
	broken.rust_target("wasm32-unknown-unknown");
	broken.reports("cargo", "cargo 1.90.0 (fixture)");
	broken.rustc();
	broken.tool("pkg-config", "exit 1");
	let output = broken.doctor(&[], &[]);
	fails(&output);
	carries(
		&output,
		"not found; install the GTK 4 development packages (libgtk-4-dev on Debian and Ubuntu)",
	);
}

#[cfg(target_os = "macos")]
#[test]
fn ios_reports_a_missing_toolchain_runtime_and_targets() {
	let lab = Lab::new();
	lab.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"ios\"]\n");
	lab.rust_target("wasm32-unknown-unknown");
	lab.reports("cargo", "cargo 1.90.0 (fixture)");
	lab.rustc();
	lab.tool("xcodebuild", "printf '%s\\n' 'Xcode 18.0'; exit 1");
	lab.tool(
		"xcrun",
		"if [ \"$1\" = simctl ]; then printf '%s\\n' '-- No runtimes installed --'; fi",
	);
	let output = lab.doctor(&[], &[]);
	fails(&output);
	carries(&output, "`xcodebuild -version` exited with");
	carries(
		&output,
		"none installed; add one in Xcode under Settings > Platforms",
	);
	carries(&output, "rustup target add aarch64-apple-ios");

	let broken = Lab::new();
	broken.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"ios\"]\n");
	broken.rust_target("wasm32-unknown-unknown");
	broken.reports("cargo", "cargo 1.90.0 (fixture)");
	broken.rustc();
	broken.reports("xcodebuild", "Xcode 18.0");
	broken.tool("xcrun", "exit 1");
	let output = broken.doctor(&[], &[]);
	fails(&output);
	carries(&output, "`xcrun simctl list runtimes` exited with");

	let absent = Lab::new();
	absent.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"ios\"]\n");
	absent.rust_target("wasm32-unknown-unknown");
	absent.reports("cargo", "cargo 1.90.0 (fixture)");
	absent.rustc();
	absent.reports("xcodebuild", "Xcode 18.0");
	let output = absent.doctor(&[], &[]);
	fails(&output);
	carries(&output, "could not run xcrun");
}

#[cfg(not(target_os = "macos"))]
#[test]
fn ios_requires_macos_and_xcode() {
	let lab = Lab::new();
	lab.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"ios\"]\n");
	lab.rust_target("wasm32-unknown-unknown");
	lab.reports("cargo", "cargo 1.90.0 (fixture)");
	lab.rustc();
	let output = lab.doctor(&[], &[]);
	fails(&output);
	carries(&output, "✖ iOS: Xcode");
	carries(&output, "iOS applications require macOS and Xcode");
}

#[test]
fn a_rustc_that_cannot_report_its_sysroot_fails_the_target_check() {
	let lab = Lab::new();
	lab.config("[app]\nname = \"app\"\nplatforms = [\"web\"]\n");
	lab.reports("cargo", "cargo 1.90.0 (fixture)");
	lab.tool(
		"rustc",
		"if [ \"$1\" = --print ]; then exit 1; else printf '%s\\n' 'rustc 1.90.0'; fi",
	);
	let output = lab.doctor(&[], &[]);
	fails(&output);
	carries(&output, "`rustc --print sysroot` exited with");
}

#[test]
fn harmonyos_reports_a_missing_or_invalid_hvigorw_and_ohpm() {
	let missing = Lab::new();
	missing.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"harmonyos\"]\n");
	missing.rust_target("wasm32-unknown-unknown");
	missing.reports("cargo", "cargo 1.90.0 (fixture)");
	missing.rustc();
	let output = missing.doctor(&[], &[]);
	succeeds(&output);
	carries(&output, "▲ HarmonyOS: hvigorw");
	carries(
		&output,
		"missing: install DevEco Studio and put hvigorw on PATH or in HVIGORW",
	);
	carries(&output, "DevEco's package manager, for `ohpm install`");

	let invalid = Lab::new();
	invalid.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"harmonyos\"]\n");
	invalid.rust_target("wasm32-unknown-unknown");
	invalid.reports("cargo", "cargo 1.90.0 (fixture)");
	invalid.rustc();
	let absent = invalid.path().join("absent-hvigorw");
	let output = invalid.doctor(&[], &[("HVIGORW", absent.to_str().unwrap())]);
	succeeds(&output);
	carries(&output, "does not exist");

	let installed = Lab::new();
	installed.config("[app]\nname = \"app\"\nplatforms = [\"web\", \"harmonyos\"]\n");
	installed.rust_target("wasm32-unknown-unknown");
	installed.reports("cargo", "cargo 1.90.0 (fixture)");
	installed.rustc();
	installed.reports("hvigorw", "hvigor 5.0.0");
	installed.reports("ohpm", "ohpm 5.0.0");
	let output = installed.doctor(&[], &[]);
	succeeds(&output);
	carries(&output, "✔ HarmonyOS: hvigorw");
	carries(&output, "✔ HarmonyOS: ohpm");
	carries(&output, "on PATH");
}

#[test]
fn project_checks_report_configuration_and_lockfile_problems() {
	let invalid = Lab::new();
	invalid.config("[app\nname =");
	let output = invalid.doctor(&[], &[]);
	fails(&output);
	carries(&output, "✖ project wasm-bindgen");
	carries(&output, "could not parse");

	let quiet = Lab::new();
	quiet.config("[app]\nname = \"app\"\nplatforms = [\"web\"]\n");
	quiet.lockfile("version = 4\n");
	quiet.rust_target("wasm32-unknown-unknown");
	quiet.reports("cargo", "cargo 1.90.0 (fixture)");
	quiet.rustc();
	let output = quiet.doctor(&[], &[]);
	succeeds(&output);
	carries(&output, "no wasm-bindgen in Cargo.lock");

	let mismatched = Lab::new();
	mismatched.config("[app]\nname = \"app\"\n");
	mismatched
		.lockfile("version = 4\n\n[[package]]\nname = \"wasm-bindgen\"\nversion = \"0.2.1\"\n");
	let output = mismatched.doctor(&[], &[]);
	fails(&output);
	carries(&output, "the project locks wasm-bindgen 0.2.1");

	let malformed = Lab::new();
	malformed.config("[app]\nname = \"app\"\n");
	malformed.lockfile("[[[");
	let output = malformed.doctor(&[], &[]);
	fails(&output);
	carries(&output, "could not parse Cargo.lock");
}

#[test]
fn extension_checks_name_the_browser_tools_that_are_missing() {
	let lab = Lab::new();
	lab.config(
		"[app]\nname = \"app\"\nplatforms = [\"web\"]\n[extension]\nbrowsers = [\"chromium\", \
		 \"firefox\"]\n[extension.popup]\nroute = \"/\"\n[extension.firefox]\nid = \
		 \"app@example.com\"\n",
	);
	lab.rust_target("wasm32-unknown-unknown");
	lab.reports("cargo", "cargo 1.90.0 (fixture)");
	lab.rustc();
	let output = lab.doctor(&[], &[]);
	succeeds(&output);
	carries(&output, "▲ Chrome for Testing");
	carries(&output, "extensions run in Chrome for Testing");
	// Firefox is also searched for under `/Applications`, so its row depends
	// on the host; the other browser tools are only found through PATH.
	carries(&output, "Firefox");
	carries(&output, "▲ geckodriver");
	carries(
		&output,
		"install geckodriver on PATH or set LEPTOSKIT_GECKODRIVER",
	);
}

#[test]
fn devtools_reports_whether_the_client_declares_the_feature() {
	let declared = Lab::new();
	declared.config("[app]\nname = \"app\"\nplatforms = [\"web\"]\n");
	declared.cargo_metadata(r#"{"devtools":["leptoskit/devtools"]}"#);
	declared.rust_target("wasm32-unknown-unknown");
	declared.rustc();
	let output = declared.doctor(&[], &[]);
	succeeds(&output);
	carries(&output, "✔ devtools");
	carries(&output, "app declares `devtools`");

	let missing = Lab::new();
	missing.config("[app]\nname = \"app\"\nplatforms = [\"web\"]\n");
	missing.cargo_metadata("{}");
	missing.rust_target("wasm32-unknown-unknown");
	missing.rustc();
	let output = missing.doctor(&[], &[]);
	succeeds(&output);
	carries(&output, "▲ devtools");
	carries(
		&output,
		"add `devtools = [\"leptoskit/devtools\"]` to app's [features]",
	);
}

#[test]
fn all_checks_every_platform_the_machine_can_build() {
	let lab = Lab::new();
	lab.rust_target("wasm32-unknown-unknown");
	lab.rust_target("aarch64-linux-android");
	lab.rust_target("x86_64-linux-android");
	lab.reports("cargo", "cargo 1.90.0 (fixture)");
	lab.rustc();
	lab.tool("java", "printf '%s\\n' 'openjdk version \"21.0.2\"' >&2");
	let sdk = lab.android_sdk();
	let output = lab.doctor(&["--all"], &[("ANDROID_HOME", sdk.to_str().unwrap())]);
	carries(&output, "✔ Android: Java");
	carries(&output, "✔ Android: SDK");
	carries(&output, "Android: aarch64-linux-android");
	let report = text(&output.stdout);
	assert!(
		!report.contains("HarmonyOS"),
		"--all leaves DevEco's tools out\n{report}"
	);
	assert_eq!(
		report.contains("iOS: Xcode"),
		cfg!(target_os = "macos"),
		"iOS checks only run where Xcode can\n{report}"
	);
	assert_eq!(
		report.contains("Linux: GTK 4"),
		cfg!(target_os = "linux"),
		"Linux checks only run on Linux\n{report}"
	);
}
