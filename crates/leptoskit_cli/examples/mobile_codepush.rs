//! Runs the native UI/code-push acceptance test against an actual mobile
//! device.
//!
//! `cargo run -p leptoskit_cli --example mobile_codepush -- --platform ios
//! --device SIMULATOR_UUID --sdk-path /path/to/leptoskit --cli
//! /path/to/leptoskit`

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware;
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::get;
use clap::Parser;
use clap::ValueEnum;
use leptoskit_codepush::routes::codepush_router;
use leptoskit_codepush::signing::ReleaseSigningKey;
use leptoskit_codepush::store::ReleaseStore;
use semver::Version;

const APP: &str = "mobile_codepush";
const CHANNEL: &str = "milestone";
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/mobile_codepush");

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Platform {
	Ios,
	Android,
}

#[derive(Debug, Parser)]
struct Args {
	#[arg(long, value_enum)]
	platform: Platform,
	/// Booted simulator UUID or adb device serial.
	#[arg(long)]
	device: String,
	#[arg(long)]
	sdk_path: PathBuf,
	#[arg(long)]
	cli: PathBuf,
	/// A new output directory. Never overwrites an earlier run.
	#[arg(long)]
	output: Option<PathBuf>,
	/// Rust target for the emulator. Android defaults to arm64.
	#[arg(long, default_value = "android")]
	android_target: String,
}

struct Releases {
	store: ReleaseStore,
	signer: ReleaseSigningKey,
	replacement: PathBuf,
	broken: PathBuf,
	online: Arc<AtomicBool>,
}

#[tokio::main]
async fn main() -> Result<()> {
	let args = Args::parse();
	let sdk = args.sdk_path.canonicalize().context("resolve SDK")?;
	let cli = args.cli.canonicalize().context("resolve CLI binary")?;
	let platform = match args.platform {
		Platform::Ios => "ios",
		Platform::Android => "android",
	};
	let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
	let output = args
		.output
		.clone()
		.unwrap_or_else(|| sdk.join(format!("target/mobile-codepush-{platform}/{stamp}")));
	anyhow::ensure!(!output.exists(), "{} already exists", output.display());
	fs::create_dir_all(&output)?;
	let output = output.canonicalize()?;
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
	let port = listener.local_addr()?.port();
	let server_url = format!("http://127.0.0.1:{port}");
	let mut seed = [0; 32];
	getrandom::fill(&mut seed).context("generate ephemeral test signing seed")?;
	let signer = ReleaseSigningKey::from_seed(&seed);
	seed.fill(0);
	let app = output.join("app");
	println!(
		"Retaining build logs, releases, and device evidence in {}",
		output.display()
	);
	let mut create = Command::new(&cli);
	create.args([
		"new",
		path(&app)?,
		"--platform",
		"native",
		"--name",
		APP,
		"--sdk-path",
		path(&sdk)?,
	]);
	run(&mut create, &output.join("01-scaffold.log"))?;
	let mut config: toml::Value = toml::from_str(&fs::read_to_string(app.join("leptoskit.toml"))?)?;
	config["native"]["update_contract"] = "mobile-v1".into();
	config["codepush"]["enabled"] = true.into();
	config["codepush"]["server"] = server_url.clone().into();
	config["codepush"]["channel"] = CHANNEL.into();
	config["codepush"]["allow_http"] = true.into();
	config["codepush"]["public_keys"] =
		toml::Value::Array(vec![signer.public_key().to_hex().into()]);
	let guest = fs::read_to_string(Path::new(FIXTURES).join("guest.rs"))?;
	let baseline = output.join("baseline");
	let replacement = output.join("replacement");
	let broken = output.join("broken");
	for (version, title, step, trap, destination) in [
		("0.1.0", "Counter 1", 1, false, &baseline),
		("0.2.0", "Counter 2", 10, false, &replacement),
		("0.3.0", "Broken", 10, true, &broken),
	] {
		write_config(&app, &mut config, version)?;
		fs::write(
			app.join("guest/src/lib.rs"),
			guest
				.replace("__TITLE__", title)
				.replace(
					"const STEP: u32 = 1;",
					&format!("const STEP: u32 = {step};"),
				)
				.replace(
					"const TRAP: bool = false;",
					&format!("const TRAP: bool = {trap};"),
				),
		)?;
		let mut command = Command::new(&cli);
		command
			.current_dir(&app)
			.args(["native", "bundle", "--out", path(destination)?]);
		run(
			&mut command,
			&output.join(format!("02-bundle-{version}.log")),
		)?;
	}
	write_config(&app, &mut config, "0.1.0")?;
	fs::write(
		app.join("guest/src/lib.rs"),
		guest.replace("__TITLE__", "Counter 1"),
	)?;
	let package = config["native"]["package_id"]
		.as_str()
		.context("native package ID")?;
	let swift = fs::read_to_string(Path::new(FIXTURES).join("AppUITests.swift"))?
		.replace("__SERVER__", &server_url);
	fs::write(app.join("hosts/ios/AppUITests.swift"), swift)?;
	let kotlin = fs::read_to_string(Path::new(FIXTURES).join("NativeAppTest.kt"))?
		.replace("__SERVER__", &server_url)
		.replace("dev.leptoskit.generated", package);
	fs::write(
		app.join("hosts/android/app/src/androidTest/java/NativeAppTest.kt"),
		kotlin,
	)?;
	let store = ReleaseStore::open(output.join("releases"))?;
	store.ingest_directory(
		&signer,
		APP,
		CHANNEL,
		&Version::new(0, 1, 0),
		0,
		&baseline,
		false,
	)?;
	let online = Arc::new(AtomicBool::new(true));
	let releases = Arc::new(Releases {
		store: store.clone(),
		signer,
		replacement,
		broken,
		online: online.clone(),
	});
	let router = Router::new()
		.route("/control/replacement", get(publish_replacement))
		.route("/control/broken", get(publish_broken))
		.route("/control/offline", get(offline))
		.with_state(releases)
		.merge(codepush_router(store).layer(middleware::from_fn_with_state(online, gate)));
	let server = tokio::spawn(async move { axum::serve(listener, router).await });
	let build_result = match args.platform {
		Platform::Ios => ios(&cli, &app, &output, &args.device, package),
		Platform::Android => {
			android(
				&cli,
				&app,
				&output,
				&args.device,
				package,
				port,
				&args.android_target,
			)
		}
	};
	server.abort();
	build_result?;
	println!(
		"PASS: native bundled UI, signed download, activation, offline process relaunch, startup \
		 trap rollback, persisted rollback"
	);
	Ok(())
}

/// Holds all release endpoints offline while test control remains reachable.
async fn gate(
	State(online): State<Arc<AtomicBool>>,
	request: axum::extract::Request,
	next: Next,
) -> Response {
	if !online.load(Ordering::SeqCst) {
		return axum::response::IntoResponse::into_response(StatusCode::SERVICE_UNAVAILABLE);
	}
	next.run(request).await
}

async fn publish_replacement(State(releases): State<Arc<Releases>>) -> (StatusCode, String) {
	publish(&releases, &releases.replacement, 2, 1)
}

async fn publish_broken(State(releases): State<Arc<Releases>>) -> (StatusCode, String) {
	publish(&releases, &releases.broken, 3, 2)
}

fn publish(releases: &Releases, directory: &Path, minor: u64, build: u64) -> (StatusCode, String) {
	releases.online.store(true, Ordering::SeqCst);
	match releases.store.ingest_directory(
		&releases.signer,
		APP,
		CHANNEL,
		&Version::new(0, minor, 0),
		build,
		directory,
		false,
	) {
		Ok(_) => (StatusCode::OK, "published".into()),
		Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
	}
}

async fn offline(State(releases): State<Arc<Releases>>) -> StatusCode {
	releases.online.store(false, Ordering::SeqCst);
	StatusCode::OK
}

fn write_config(app: &Path, config: &mut toml::Value, version: &str) -> Result<()> {
	config["app"]["version"] = version.into();
	fs::write(app.join("leptoskit.toml"), toml::to_string_pretty(config)?)?;
	Ok(())
}

fn ios(cli: &Path, app: &Path, output: &Path, device: &str, package: &str) -> Result<()> {
	let mut prepare = Command::new(cli);
	prepare
		.current_dir(app)
		.args(["native", "prepare", "ios-simulator"]);
	run(&mut prepare, &output.join("03-native-prepare.log"))?;
	let installed = Command::new("xcrun")
		.args(["simctl", "get_app_container", device, package])
		.output()
		.context("inspect prior simulator installation")?;
	if installed.status.success() {
		let mut uninstall = Command::new("xcrun");
		uninstall.args(["simctl", "uninstall", device, package]);
		run(&mut uninstall, &output.join("03-uninstall-previous.log"))?;
	}
	let mut tests = Command::new("xcodebuild");
	tests.current_dir(app).args([
		"-project",
		".leptoskit/ios/Host.xcodeproj",
		"-scheme",
		"App",
		"-sdk",
		"iphonesimulator",
		"-destination",
		&format!("platform=iOS Simulator,id={device}"),
		"-derivedDataPath",
		path(&output.join("xcode-build"))?,
		"-resultBundlePath",
		path(&output.join("milestone.xcresult"))?,
		"CODE_SIGNING_ALLOWED=NO",
		// The Rust host library is built only for aarch64-apple-ios-sim.
		"ARCHS=arm64",
		"test",
	]);
	run(&mut tests, &output.join("04-xcode-test.log"))
}

fn android(
	cli: &Path,
	app: &Path,
	output: &Path,
	device: &str,
	package: &str,
	port: u16,
	target: &str,
) -> Result<()> {
	let mut prepare = Command::new(cli);
	prepare.current_dir(app).args(["native", "prepare", target]);
	run(&mut prepare, &output.join("03-native-prepare.log"))?;
	let mut gradle = Command::new(app.join("hosts/android/gradlew"));
	gradle.current_dir(app.join("hosts/android")).args([
		"--no-daemon",
		"assembleDebug",
		"assembleDebugAndroidTest",
	]);
	run(&mut gradle, &output.join("04-gradle-build.log"))?;
	adb(
		device,
		&["reverse", &format!("tcp:{port}"), &format!("tcp:{port}")],
		output,
		"05-adb-reverse",
	)?;
	for (name, apk) in [
		("06-install-app", "app-debug.apk"),
		("07-install-tests", "app-debug-androidTest.apk"),
	] {
		let apk_dir = if name == "06-install-app" {
			"apk/debug"
		} else {
			"apk/androidTest/debug"
		};
		adb(
			device,
			&[
				"install",
				"-r",
				path(
					&app.join("hosts/android/app/build/outputs")
						.join(apk_dir)
						.join(apk),
				)?,
			],
			output,
			name,
		)?;
	}
	// A slow emulator's System UI ANR dialog would cover the app under test.
	adb(
		device,
		&[
			"shell",
			"settings",
			"put",
			"global",
			"hide_error_dialogs",
			"1",
		],
		output,
		"07-hide-error-dialogs",
	)?;
	adb(
		device,
		&["shell", "pm", "clear", package],
		output,
		"07-clear-previous-state",
	)?;
	for (index, method) in [
		"bundledAndDownloadedGuest",
		"offlineGuestAndStartupTrap",
		"persistedRollback",
	]
	.iter()
	.enumerate()
	{
		adb(
			device,
			&["shell", "am", "force-stop", package],
			output,
			&format!("08-stop-{index}"),
		)?;
		adb(
			device,
			&[
				"shell",
				"am",
				"instrument",
				"-w",
				"-r",
				"-e",
				"class",
				&format!("{package}.NativeAppTest#{method}"),
				&format!("{package}.test/androidx.test.runner.AndroidJUnitRunner"),
			],
			output,
			&format!("09-instrument-{index}"),
		)?;
		let log = fs::read_to_string(output.join(format!("09-instrument-{index}.log")))?;
		anyhow::ensure!(
			log.contains("OK (1 test)") && !log.contains("FAILURES!!!"),
			"Android instrumentation failed; inspect retained log"
		);
	}
	adb(
		device,
		&[
			"pull",
			&format!("/sdcard/Android/data/{package}/files/leptoskit-codepush"),
			path(&output.join("screenshots"))?,
		],
		output,
		"10-screenshots",
	)
}

fn adb(device: &str, arguments: &[&str], output: &Path, name: &str) -> Result<()> {
	let mut command = Command::new("adb");
	command.args(["-s", device]).args(arguments);
	run(&mut command, &output.join(format!("{name}.log")))
}

/// Retains complete subprocess output without flooding the calling terminal.
fn run(command: &mut Command, log: &Path) -> Result<()> {
	println!("Running {command:?}; log {}", log.display());
	let file = fs::File::create(log)?;
	let status = command
		.stdout(Stdio::from(file.try_clone()?))
		.stderr(Stdio::from(file))
		.status()
		.with_context(|| format!("execute {command:?}"))?;
	anyhow::ensure!(
		status.success(),
		"{command:?} failed ({status}); inspect {}",
		log.display()
	);
	Ok(())
}

fn path(value: &Path) -> Result<&str> {
	value.to_str().context("fixture paths must be UTF-8")
}
