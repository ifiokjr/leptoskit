//! `platforms()` as an application's build script runs it: Cargo's
//! environment in, `cargo:` directives out. Each case runs this test binary
//! again as a child with the environment a build script would see, since
//! the environment is process-wide.

use std::path::Path;
use std::process::Command;

const PLATFORMS: [&str; 8] = [
	"WEB",
	"IOS",
	"ANDROID",
	"MACOS",
	"WINDOWS",
	"LINUX",
	"TUI",
	"EXTENSION",
];

/// Runs `platforms()` when started as a child by [`run`], and does nothing
/// otherwise.
#[test]
fn probe() {
	if std::env::var_os("LEPTOSKIT_BUILD_PROBE").is_none() {
		return;
	}

	match leptoskit_build::platforms() {
		Ok(()) => println!("probe:ok"),
		Err(error) => {
			let source = std::error::Error::source(&error).is_some();
			println!("probe:err:{error}:source={source}");
		}
	}
}

/// What a build script with `manifest` as its package directory and
/// `features` turned on prints.
fn run(manifest: Option<&Path>, features: &[&str]) -> String {
	let mut command = Command::new(std::env::current_exe().unwrap());
	command
		.args(["probe", "--exact", "--nocapture", "--test-threads=1"])
		.env("LEPTOSKIT_BUILD_PROBE", "1")
		.env_remove("CARGO_MANIFEST_DIR");

	for platform in PLATFORMS {
		command.env_remove(format!("CARGO_FEATURE_{platform}"));
	}

	for feature in features {
		command.env(format!("CARGO_FEATURE_{feature}"), "1");
	}

	if let Some(manifest) = manifest {
		command.env("CARGO_MANIFEST_DIR", manifest);
	}

	let output = command.output().unwrap();
	assert!(output.status.success(), "{output:?}");
	String::from_utf8(output.stdout).unwrap()
}

fn application() -> tempfile::TempDir {
	let application = tempfile::tempdir().unwrap();
	std::fs::create_dir(application.path().join("src")).unwrap();
	application
}

#[test]
fn an_application_without_a_platform_feature_uses_its_shared_modules() {
	let application = application();
	let printed = run(Some(application.path()), &[]);

	assert!(printed.contains("probe:ok"), "{printed}");
	assert!(
		printed.contains("cargo:rustc-env=LEPTOSKIT_PLATFORM=shared"),
		"{printed}"
	);
	// The source directory is watched as a whole, so an override added
	// between builds selects itself on the next one.
	let source = application.path().join("src");
	assert!(
		printed.contains(&format!("cargo:rerun-if-changed={}", source.display())),
		"{printed}"
	);
	let manifest = application.path().join("Cargo.toml");
	assert!(
		printed.contains(&format!("cargo:rerun-if-changed={}", manifest.display())),
		"{printed}"
	);
}

#[test]
fn a_platform_feature_selects_its_modules() {
	let application = application();

	let printed = run(Some(application.path()), &["IOS"]);
	assert!(
		printed.contains("cargo:rustc-env=LEPTOSKIT_PLATFORM=ios"),
		"{printed}"
	);

	let printed = run(Some(application.path()), &["WEB", "EXTENSION"]);
	assert!(
		printed.contains("cargo:rustc-env=LEPTOSKIT_PLATFORM=extension"),
		"{printed}"
	);
}

#[test]
fn two_device_platforms_fail_the_build_and_say_how_to_fix_it() {
	let application = application();
	let printed = run(Some(application.path()), &["WEB", "ANDROID"]);

	// Cargo shows `cargo:error` as the build's error.
	assert!(printed.contains("cargo:error="), "{printed}");
	assert!(printed.contains("enabled: web, android"), "{printed}");
	assert!(printed.contains("--no-default-features"), "{printed}");
	assert!(!printed.contains("LEPTOSKIT_PLATFORM"), "{printed}");
}

#[test]
fn outside_a_build_script_it_says_where_it_belongs() {
	let printed = run(None, &[]);

	assert!(
		printed.contains("must run from the application's Cargo build script:source=false"),
		"{printed}"
	);
}

#[test]
fn an_application_needs_a_source_directory() {
	let missing = tempfile::tempdir().unwrap();
	let printed = run(Some(missing.path()), &[]);
	assert!(printed.contains("probe:err:could not read"), "{printed}");
	// The filesystem's reason is kept as the error's source.
	assert!(printed.contains("source=true"), "{printed}");

	let file = tempfile::tempdir().unwrap();
	std::fs::write(file.path().join("src"), "").unwrap();
	let printed = run(Some(file.path()), &[]);
	assert!(
		printed.contains("must be a source directory:source=false"),
		"{printed}"
	);
}
