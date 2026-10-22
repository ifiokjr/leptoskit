//! Builds the optional Android plugin integration fixture from a fresh
//! scaffold.
//!
//! ```sh
//! cargo build -p leptoskit_cli --bin leptoskit
//! cargo run -p leptoskit_cli --example native_plugin_suite -- \
//!   --cli target/debug/leptoskit --destination target/native-plugin-suite \
//!   --target android-x86_64
//! ```
//! Then run `./gradlew :app:connectedDebugAndroidTest` in the generated
//! project's `hosts/android` directory with `ANDROID_SERIAL` set.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use clap::Parser;

const OPTIONAL_PLUGINS: &[(&str, &str)] = &[
	("permissions", "leptoskit_plugin_permissions"),
	("network", "leptoskit_plugin_network"),
	("application", "leptoskit_plugin_application"),
	("localization", "leptoskit_plugin_localization"),
	("accelerometer", "leptoskit_plugin_accelerometer"),
	("gyroscope", "leptoskit_plugin_gyroscope"),
	("magnetometer", "leptoskit_plugin_magnetometer"),
	("barometer", "leptoskit_plugin_barometer"),
	("light_sensor", "leptoskit_plugin_light_sensor"),
	("pedometer", "leptoskit_plugin_pedometer"),
	("document-picker", "leptoskit_plugin_document_picker"),
	("screen-orientation", "leptoskit_plugin_screen_orientation"),
	("brightness", "leptoskit_plugin_brightness"),
	("keep-awake", "leptoskit_plugin_keep_awake"),
	("sqlite", "leptoskit_plugin_sqlite"),
	("store", "leptoskit_plugin_store"),
	("speech", "leptoskit_plugin_speech"),
	("logging", "leptoskit_plugin_logging"),
	("location", "leptoskit_plugin_location"),
	(
		"local-authentication",
		"leptoskit_plugin_local_authentication",
	),
	("notifications", "leptoskit_plugin_notifications"),
	("image_manipulator", "leptoskit_plugin_image_manipulator"),
	("screen-capture", "leptoskit_plugin_screen_capture"),
];

const ANDROID_TESTS: &[&str] = &[
	"leptoskit_bridge_ffi/android-tests/ActivityResultBrokerTest.kt",
	"leptoskit_plugin_secure_store/tests/android/SecureStorePluginTest.kt",
	"leptoskit_plugin_permissions/tests/android/PermissionsPluginTest.kt",
	"leptoskit_plugin_network/tests/android/NetworkPluginTest.kt",
	"leptoskit_plugin_application/tests/android/ApplicationPluginTest.kt",
	"leptoskit_plugin_localization/tests/android/LocalizationPluginTest.kt",
	"leptoskit_sensor/android-tests/MotionSensorTest.kt",
	"leptoskit_sensor/android-tests/SensorEncodingTest.kt",
	"leptoskit_plugin_brightness/tests/android/BrightnessPluginTest.kt",
	"leptoskit_plugin_keep_awake/tests/android/KeepAwakePluginTest.kt",
	"leptoskit_plugin_speech/tests/android/SpeechPluginTest.kt",
	"leptoskit_plugin_location/android-tests/LocationPluginTest.kt",
	"leptoskit_plugin_local_authentication/tests/android/LocalAuthenticationPluginTest.kt",
	"leptoskit_plugin_notifications/tests/android/NotificationsPluginTest.kt",
	"leptoskit_plugin_screen_capture/tests/android/ScreenCapturePluginTest.kt",
	"leptoskit_plugin_screen_orientation/tests/android/ScreenOrientationPluginTest.kt",
	"leptoskit_plugin_document_picker/tests/android/DocumentEncodingTest.kt",
];

const DOCUMENT_ENCODING_TEST: &str = r"package dev.leptoskit.plugins.documentpicker

import org.junit.Test

/** Runs the shared synthetic stream fixture without presenting a picker. */
class DocumentEncodingInstrumentationTest {
    @Test
    fun boundedReadsAndCancellation() {
        main()
    }
}
";

#[derive(Parser)]
struct Args {
	/// Built `LeptosKit` CLI binary; its SDK and templates must match this
	/// checkout.
	#[arg(long)]
	cli: PathBuf,
	/// New destination. Existing directories are never overwritten.
	#[arg(long)]
	destination: PathBuf,
	/// Android native architecture, matching the eventual test emulator.
	#[arg(long, value_parser = ["android", "android-x86_64"], default_value = "android-x86_64")]
	target: String,
}

fn main() -> Result<()> {
	let args = Args::parse();
	let cli = args.cli.canonicalize().context("locate --cli binary")?;
	let sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
		.join("../..")
		.canonicalize()
		.context("locate SDK checkout")?;
	let destination = std::path::absolute(&args.destination)?;
	run(Command::new(&cli)
		.args(["generate", "app"])
		.arg(&destination)
		.args([
			"--name",
			"native_plugin_suite",
			"--platform",
			"native",
			"--sdk-path",
		])
		.arg(&sdk))?;

	configure(&destination, &sdk)?;
	run(Command::new(&cli)
		.current_dir(&destination)
		.args(["native", "prepare", &args.target]))?;
	println!(
		"Android plugin suite prepared in {}\nrun the connectedDebugAndroidTest Gradle task there",
		destination.join("hosts/android").display()
	);
	Ok(())
}

fn run(command: &mut Command) -> Result<()> {
	let status = command
		.status()
		.with_context(|| format!("start {command:?}"))?;
	anyhow::ensure!(status.success(), "{command:?} exited with {status}");
	Ok(())
}

fn configure(destination: &Path, sdk: &Path) -> Result<()> {
	edit_toml(&destination.join("Cargo.toml"), |manifest| {
		for (_, package) in OPTIONAL_PLUGINS {
			add_dependency(manifest, sdk, package, Side::Host)?;
		}
		Ok(())
	})?;
	edit_toml(&destination.join("guest/Cargo.toml"), |manifest| {
		for package in [
			"leptoskit_plugin_sqlite",
			"leptoskit_plugin_store",
			"leptoskit_plugin_logging",
			"leptoskit_plugin_crypto",
		] {
			add_dependency(manifest, sdk, package, Side::Guest)?;
		}
		// The guest needs only the wire protocol, not the image codecs.
		add_dependency(
			manifest,
			sdk,
			"leptoskit_plugin_image_manipulator",
			Side::GuestWithoutDefaults,
		)?;
		Ok(())
	})?;
	edit_toml(&destination.join("leptoskit.toml"), |manifest| {
		let plugins = manifest
			.get_mut("native")
			.and_then(toml::Value::as_table_mut)
			.and_then(|native| native.get_mut("plugins"))
			.and_then(toml::Value::as_array_mut)
			.context("scaffold lacks native.plugins")?;
		for (name, _) in OPTIONAL_PLUGINS {
			plugins.push((*name).into());
		}
		Ok(())
	})?;

	let guest = destination.join("guest/src/lib.rs");
	let source = std::fs::read_to_string(&guest)?;
	// Keep the scaffold's actual UI tests and add worker assertions to its file
	// action. Fail loudly if that action changes instead of silently omitting
	// them.
	let marker = "Ok(\"File roundtrip passed\".to_owned())";
	anyhow::ensure!(
		source.matches(marker).count() == 1,
		"scaffold file action changed; update optional worker integration"
	);
	let mut source = source.replacen(
		marker,
		&format!("optional_workers().await?;\n\t{marker}"),
		1,
	);
	source.push('\n');
	source.push_str(include_str!(
		"../tests/fixtures/native_plugin_suite/workers.rs"
	));
	std::fs::write(guest, source)?;

	let tests = destination.join("hosts/android/app/src/androidTest/java");
	for relative in ANDROID_TESTS {
		let source = sdk.join("crates").join(relative);
		let name = source.file_name().context("test source lacks a filename")?;
		let output = tests.join(name);
		anyhow::ensure!(!output.exists(), "duplicate test file {}", output.display());
		std::fs::copy(&source, &output)
			.with_context(|| format!("copy Android test {}", source.display()))?;
	}
	std::fs::write(
		tests.join("SystemBarsTest.kt"),
		include_str!("../tests/fixtures/native_plugin_suite/SystemBarsTest.kt"),
	)?;
	std::fs::write(
		tests.join("LoggingFileTest.kt"),
		include_str!("../tests/fixtures/native_plugin_suite/LoggingFileTest.kt"),
	)?;
	let wrapper = tests.join("DocumentEncodingInstrumentationTest.kt");
	anyhow::ensure!(
		!wrapper.exists(),
		"duplicate test file {}",
		wrapper.display()
	);
	std::fs::write(wrapper, DOCUMENT_ENCODING_TEST)?;
	Ok(())
}

fn edit_toml(path: &Path, edit: impl FnOnce(&mut toml::Table) -> Result<()>) -> Result<()> {
	let mut document = toml::from_str(&std::fs::read_to_string(path)?)?;
	edit(&mut document)?;
	std::fs::write(path, toml::to_string_pretty(&document)?)?;
	Ok(())
}

/// Which crate of the generated app depends on an SDK plugin.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
	Host,
	Guest,
	GuestWithoutDefaults,
}

fn add_dependency(manifest: &mut toml::Table, sdk: &Path, package: &str, side: Side) -> Result<()> {
	let path = sdk.join("crates").join(package);
	anyhow::ensure!(
		path.join("Cargo.toml").is_file(),
		"missing SDK crate {package}"
	);
	let mut dependency = toml::Table::new();
	dependency.insert(
		"path".into(),
		path.to_str().context("SDK path is not UTF-8")?.into(),
	);
	if side != Side::Host {
		dependency.insert("features".into(), vec!["guest"].into());
	}
	if side == Side::GuestWithoutDefaults {
		dependency.insert("default-features".into(), false.into());
	}
	let dependencies = manifest
		.get_mut("dependencies")
		.and_then(toml::Value::as_table_mut)
		.context("scaffold lacks dependencies table")?;
	anyhow::ensure!(
		!dependencies.contains_key(package),
		"duplicate dependency {package}"
	);
	dependencies.insert(package.into(), dependency.into());
	Ok(())
}
