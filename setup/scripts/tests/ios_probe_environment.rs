//! Exercises the real probe's sanitized environment without a compiler or
//! device.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
	fn new() -> Self {
		let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
		let root = std::env::temp_dir().join(format!("ios-probe-{}-{id}", std::process::id()));
		fs::create_dir(&root).expect("exclusive fixture directory");
		let fixture = Self(root);
		fixture.write(
			"setup/scripts/ios_probe_run.sh",
			include_str!("../ios_probe_run.sh"),
		);
		fixture.write(
			"target-lib/libcore-fixture.rlib",
			"installed target control",
		);
		fixture.executable("xcode/bin/clang", "exit 99");
		fixture.executable(
			"xcode/bin/xcrun",
			r#"case "$*" in
    '--find clang') printf '%s/xcode/bin/clang\n' "$PWD" ;;
    '--sdk macosx --show-sdk-path') printf '/fixture/macos-sdk\n' ;;
    '--sdk iphonesimulator --show-sdk-path') printf '/fixture/simulator-sdk\n' ;;
    *) printf 'simulator command was reached\n' > simulator-called; exit 98 ;;
esac"#,
		);
		fixture.executable(
			"selected/bin/rustc",
			r#"printf 'rustc=%s\ntoolchain=%s\n' "$0" "$RUSTUP_TOOLCHAIN" >> receipt
test "$*" = '--print target-libdir --target aarch64-apple-ios-sim'
printf '%s/target-lib\n' "$PWD""#,
		);
		fixture.executable(
			"selected/bin/cargo",
			r#"printf 'cargo=%s\nargs=%s\nhome=%s\ncargo_home=%s\nrustup_home=%s\n' \
    "$0" "$*" "$HOME" "$CARGO_HOME" "$RUSTUP_HOME" >> receipt
printf 'rustc_env=%s\ntoolchain=%s\n' "$RUSTC" "$RUSTUP_TOOLCHAIN" >> receipt
printf 'cc=%s\nwrapper=%s\ncontamination=%s\n' \
    "${CC-unset}" "${RUSTC_WRAPPER-unset}" "${NIX_PROBE_CONTAMINATION-unset}" >> receipt
exit 71"#,
		);

		fixture
	}

	fn write(&self, relative: &str, contents: &str) {
		let path = self.0.join(relative);
		fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture directories");
		fs::write(path, contents).expect("fixture source");
	}

	fn executable(&self, relative: &str, body: &str) {
		self.write(relative, &format!("#!/bin/bash\nset -eu\n{body}\n"));
		fs::set_permissions(self.0.join(relative), fs::Permissions::from_mode(0o755))
			.expect("executable fixture");
	}

	fn run_with_compiler(&self, compiler: Option<&Path>) -> Output {
		let mut command = Command::new("/bin/bash");
		command
			.arg(self.0.join("setup/scripts/ios_probe_run.sh"))
			.env(
				"PATH",
				format!(
					"{}/selected/bin:{}/xcode/bin:/usr/bin:/bin",
					self.0.display(),
					self.0.display()
				),
			)
			.env("CARGO_HOME", self.0.join("cargo-home"))
			.env("RUSTUP_HOME", self.0.join("rustup-home"))
			.env("RUSTUP_TOOLCHAIN", "fixture-selected-toolchain")
			.env("CC", "/fixture/nix-clang-wrapper")
			.env("RUSTC_WRAPPER", "/fixture/nix-rustc-wrapper")
			.env("NIX_PROBE_CONTAMINATION", "must disappear")
			.env_remove("RUSTC")
			.env_remove("LEPTOSKIT_CLEAN_BUILD_ENV");

		if let Some(compiler) = compiler {
			command.env("RUSTC", compiler);
		}

		let output = command.output().expect("launch actual probe script");
		assert!(
			!self.0.join("simulator-called").exists(),
			"no simulator command"
		);

		output
	}

	fn run(&self) -> Output {
		self.run_with_compiler(None)
	}

	fn remove(&self, relative: &str) {
		fs::remove_file(self.0.join(relative)).expect("remove selected fixture input");
	}

	fn receipt(&self) -> String {
		fs::read_to_string(self.0.join("receipt")).expect("selected tool receipt")
	}
}

impl Drop for Fixture {
	fn drop(&mut self) {
		fs::remove_dir_all(&self.0).expect("remove only owned fixture");
	}
}

#[test]
fn preserves_selected_rust_tools_and_toolchain_without_nix_contamination() {
	let fixture = Fixture::new();
	let output = fixture.run();
	assert_eq!(
		output.status.code(),
		Some(71),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let receipt = fixture.receipt();
	println!("selected-tool receipt:\n{receipt}");

	for (key, path) in [
		("cargo", "selected/bin/cargo"),
		("rustc", "selected/bin/rustc"),
		("rustc_env", "selected/bin/rustc"),
		("cargo_home", "cargo-home"),
		("rustup_home", "rustup-home"),
	] {
		assert!(
			receipt.contains(&format!("{key}={}\n", fixture.0.join(path).display())),
			"{receipt}"
		);
	}

	assert!(receipt.contains(&format!(
		"home={}\n",
		std::env::var("HOME").expect("caller HOME")
	)));
	assert_eq!(
		receipt
			.matches("toolchain=fixture-selected-toolchain\n")
			.count(),
		2
	);
	assert!(receipt.contains(
		"args=build --release --package platform_probe --target aarch64-apple-ios-sim\n"
	));
	assert!(receipt.contains("cc=unset\nwrapper=unset\ncontamination=unset\n"));
}

#[test]
fn respects_explicit_compiler_outside_the_cargo_directory() {
	let fixture = Fixture::new();
	let compiler = fixture.0.join("override/rustc");
	fs::create_dir(compiler.parent().expect("override parent")).expect("override directory");
	fs::rename(fixture.0.join("selected/bin/rustc"), &compiler).expect("separate compiler");
	let output = fixture.run_with_compiler(Some(&compiler));
	assert_eq!(output.status.code(), Some(71));
	let receipt = fixture.receipt();
	println!("explicit-compiler receipt:\n{receipt}");
	assert!(receipt.contains(&format!("rustc={}\n", compiler.display())));
	assert!(receipt.contains(&format!("rustc_env={}\n", compiler.display())));
}

fn rejects_missing_input(relative: &str, diagnostic: &str) {
	let fixture = Fixture::new();
	fixture.remove(relative);
	let output = fixture.run();
	println!(
		"missing {relative}: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	assert_eq!(output.status.code(), Some(1));
	assert!(String::from_utf8_lossy(&output.stderr).contains(diagnostic));
	let receipt = fs::read_to_string(fixture.0.join("receipt")).unwrap_or_default();
	assert!(!receipt.contains("cargo="), "build must not start");
}

#[test]
fn rejects_missing_cargo_before_simulator_work() {
	rejects_missing_input("selected/bin/cargo", "requires cargo on PATH");
}

#[test]
fn rejects_missing_rustc_before_simulator_work() {
	rejects_missing_input("selected/bin/rustc", "requires rustc");
}

#[test]
fn rejects_missing_target_before_build_or_simulator_work() {
	rejects_missing_input(
		"target-lib/libcore-fixture.rlib",
		"requires the installed aarch64-apple-ios-sim",
	);
}
