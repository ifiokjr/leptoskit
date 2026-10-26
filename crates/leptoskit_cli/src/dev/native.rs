//! The native half of `leptoskit dev`: choosing the platforms to run,
//! finding or starting a device for each, building and launching its app, and
//! sending every rebuilt guest to the apps running it.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::net::Ipv4Addr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use leptoskit_codepush::dev::GUEST_ENV;
use leptoskit_codepush::dev::GUEST_EXTRA;
use leptoskit_codepush::dev::guest_path;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;
use serde::Deserialize;

use super::guests::Guests;
use crate::native::NativeCommand;
use crate::native::Target;
use crate::native::run_tool;
use crate::testing::adb;
use crate::testing::android_tool;
use crate::testing::shell_quote;

/// How long a started Android emulator may take to finish booting.
const EMULATOR_BOOT: Duration = Duration::from_mins(4);
/// How long one adb command may take before its device counts as stuck.
const ADB_TIMEOUT: Duration = Duration::from_mins(1);
/// Installing copies the whole APK to the device.
const ADB_INSTALL_TIMEOUT: Duration = Duration::from_mins(5);
/// How long a boot check waits on a device that may still be starting.
const BOOT_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// The platforms one `leptoskit dev` runs.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Selection {
	/// Whether the web app, or the app's server, runs.
	pub web: bool,
	/// The native platforms to launch, in the order they were named.
	pub native: Vec<Platform>,
	/// Enabled platforms this machine cannot run, each with the reason.
	pub skipped: Vec<(Platform, String)>,
}

/// Chooses the platforms to run: the `requested` ones, or by default every
/// platform `app.platforms` enables, skipping those this machine cannot run.
/// A requested platform that cannot run is an error instead of a skip.
pub fn select(config: &LeptoskitConfig, requested: &[Platform]) -> Result<Selection> {
	select_with(config, requested, host_support)
}

fn select_with(
	config: &LeptoskitConfig,
	requested: &[Platform],
	support: impl Fn(Platform) -> Result<(), String>,
) -> Result<Selection> {
	let explicit = !requested.is_empty();
	let has_web = matches!(
		config.app.default_platform,
		Platform::Web | Platform::Server
	);
	let candidates = if explicit {
		requested.to_vec()
	} else if config.app.default_platform == Platform::Server {
		// A server-only app runs its server whatever `app.platforms` lists.
		std::iter::once(Platform::Server)
			.chain(config.app.platforms.iter().copied())
			.collect()
	} else {
		config.app.platforms.clone()
	};
	let mut selection = Selection::default();

	for platform in candidates {
		match platform {
			Platform::Web | Platform::Server => {
				anyhow::ensure!(
					has_web || !explicit,
					"this project has no web app: app.default_platform is {}",
					config.app.default_platform
				);
				selection.web |= has_web;
			}
			Platform::Native => {
				anyhow::bail!(
					"`native` names no platform; pass ios, android, macos, windows, or linux"
				)
			}
			platform if selection.native.contains(&platform) => {}
			platform => {
				if explicit {
					config.ensure_platform_enabled(platform)?;
				}

				match support(platform) {
					Ok(()) => selection.native.push(platform),
					Err(reason) if explicit => {
						anyhow::bail!("cannot run {platform} here: {reason}")
					}
					Err(reason) => selection.skipped.push((platform, reason)),
				}
			}
		}
	}

	Ok(selection)
}

/// Whether this machine has what `platform` needs, and if not, why.
fn host_support(platform: Platform) -> Result<(), String> {
	match platform {
		Platform::Macos | Platform::Ios if !cfg!(target_os = "macos") => {
			Err(format!("{platform} apps build on a Mac"))
		}
		Platform::Ios if !succeeds(Command::new("xcrun").args(["--find", "simctl"])) => {
			Err("the iOS Simulator needs Xcode's command-line tools".to_owned())
		}
		Platform::Android if !succeeds(Command::new(android_tool("adb")).arg("version")) => {
			Err("set ANDROID_HOME to an Android SDK with platform-tools installed".to_owned())
		}
		Platform::Windows if !cfg!(windows) => Err("Windows apps build on Windows".to_owned()),
		Platform::Linux if !cfg!(target_os = "linux") => {
			Err("Linux apps build on Linux".to_owned())
		}
		Platform::Harmonyos => {
			Err(
				"`leptoskit dev` does not run HarmonyOS apps yet; use `leptoskit native build \
				 harmonyos`"
					.to_owned(),
			)
		}
		_ => Ok(()),
	}
}

fn succeeds(command: &mut Command) -> bool {
	command
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.status()
		.is_ok_and(|status| status.success())
}

/// Where one platform's app runs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Device {
	/// This machine, for a desktop platform.
	Local(Target),
	/// A booted iOS Simulator.
	Simulator { udid: String, name: String },
	/// An Android emulator or device, by its adb serial.
	Android { serial: String, target: Target },
}

impl Device {
	fn target(&self) -> Target {
		match self {
			Self::Local(target) | Self::Android { target, .. } => *target,
			Self::Simulator { .. } => Target::IosSimulator,
		}
	}

	fn label(&self) -> &str {
		match self {
			Self::Local(_) => "this machine",
			Self::Simulator { name, .. } => name,
			Self::Android { serial, .. } => serial,
		}
	}
}

/// The devices `platform` runs on: this machine for a desktop, every booted
/// iOS simulator or attached Android device, or one started for the purpose
/// when none is. Naming devices in `wanted`, by simulator name or UDID or by
/// adb serial, narrows a mobile platform to those; a platform none of them
/// belongs to gets no devices.
fn devices(platform: Platform, wanted: &[String]) -> Result<Vec<Device>> {
	match platform {
		Platform::Macos => Ok(vec![Device::Local(Target::Macos)]),
		Platform::Windows => Ok(vec![Device::Local(Target::Windows)]),
		Platform::Linux => Ok(vec![Device::Local(Target::Linux)]),
		Platform::Ios => simulators(wanted),
		Platform::Android => android_devices(wanted),
		other => anyhow::bail!("`leptoskit dev` has no devices for {other}"),
	}
}

#[derive(Debug, Deserialize)]
struct SimulatorList {
	devices: BTreeMap<String, Vec<Simulator>>,
}

#[derive(Debug, Deserialize)]
struct Simulator {
	udid: String,
	name: String,
	state: String,
	#[serde(rename = "isAvailable", default)]
	available: bool,
}

impl Simulator {
	fn booted(&self) -> bool {
		self.state == "Booted"
	}
}

/// The `iOS-26-5` in `com.apple.CoreSimulator.SimRuntime.iOS-26-5`, as a
/// comparable version, or `None` for a watchOS, tvOS, or visionOS runtime.
fn ios_runtime_version(runtime: &str) -> Option<Vec<u32>> {
	let version = runtime.rsplit('.').next()?.strip_prefix("iOS-")?;
	version.split('-').map(|part| part.parse().ok()).collect()
}

/// Every available iPhone and iPad simulator with its runtime's version.
fn ios_simulators(list: &SimulatorList) -> impl Iterator<Item = (Vec<u32>, &Simulator)> {
	list.devices
		.iter()
		.filter_map(|(runtime, simulators)| Some((ios_runtime_version(runtime)?, simulators)))
		.flat_map(|(version, simulators)| {
			simulators
				.iter()
				.filter(|simulator| simulator.available)
				.map(move |simulator| (version.clone(), simulator))
		})
}

/// The simulators to run on: those named in `wanted`, preferring a booted
/// one and then the newest runtime for each name; or every booted one; or
/// the first iPhone on the newest runtime when none is booted.
fn choose_simulators<'a>(list: &'a SimulatorList, wanted: &[String]) -> Vec<&'a Simulator> {
	let newest_first = |left: &(Vec<u32>, &Simulator), right: &(Vec<u32>, &Simulator)| {
		right
			.1
			.booted()
			.cmp(&left.1.booted())
			.then(right.0.cmp(&left.0))
	};

	if !wanted.is_empty() {
		let mut chosen: Vec<&Simulator> = Vec::new();

		for want in wanted {
			let mut matches: Vec<_> = ios_simulators(list)
				.filter(|(_, simulator)| simulator.udid == *want || simulator.name == *want)
				.collect();
			matches.sort_by(newest_first);

			if let Some((_, simulator)) = matches.first()
				&& !chosen.iter().any(|chosen| chosen.udid == simulator.udid)
			{
				chosen.push(simulator);
			}
		}

		return chosen;
	}

	let booted: Vec<&Simulator> = ios_simulators(list)
		.map(|(_, simulator)| simulator)
		.filter(|simulator| simulator.booted())
		.collect();

	if !booted.is_empty() {
		return booted;
	}

	let mut iphones: Vec<_> = ios_simulators(list)
		.filter(|(_, simulator)| simulator.name.starts_with("iPhone"))
		.collect();
	iphones.sort_by(newest_first);
	iphones
		.first()
		.map(|(_, simulator)| *simulator)
		.into_iter()
		.collect()
}

fn simulators(wanted: &[String]) -> Result<Vec<Device>> {
	let output = Command::new("xcrun")
		.args(["simctl", "list", "devices", "--json"])
		.output()
		.context("could not list the iOS Simulators")?;
	anyhow::ensure!(output.status.success(), "xcrun simctl list failed");
	let list: SimulatorList =
		serde_json::from_slice(&output.stdout).context("could not read the iOS Simulator list")?;
	let chosen = choose_simulators(&list, wanted);
	anyhow::ensure!(
		!chosen.is_empty() || !wanted.is_empty(),
		"no iPhone simulator is installed; add one in Xcode's Devices and Simulators window"
	);
	let mut booting = false;

	for simulator in chosen.iter().filter(|simulator| !simulator.booted()) {
		println!("[ios] booting {}", simulator.name);
		run_tool(Command::new("xcrun").args(["simctl", "bootstatus", &simulator.udid, "-b"]))?;
		booting = true;
	}

	if booting {
		// The simulator runs headless without its window; showing it is a
		// convenience, so a failure to open it is not an error.
		let _ = Command::new("open").args(["-a", "Simulator"]).status();
	}

	Ok(chosen
		.into_iter()
		.map(|simulator| {
			Device::Simulator {
				udid: simulator.udid.clone(),
				name: simulator.name.clone(),
			}
		})
		.collect())
}

/// The serials of attached devices ready for commands, from `adb devices`.
/// Unauthorized and offline devices, and adb's own messages, are skipped.
fn parse_adb_devices(output: &str) -> Vec<String> {
	output
		.lines()
		.filter_map(|line| {
			let mut fields = line.split_whitespace();
			let serial = fields.next()?;
			(fields.next()? == "device").then(|| serial.to_owned())
		})
		.collect()
}

fn adb_serials() -> Result<Vec<String>> {
	let output = Command::new(android_tool("adb"))
		.arg("devices")
		.output()
		.context("could not run adb")?;
	anyhow::ensure!(output.status.success(), "adb devices failed");
	Ok(parse_adb_devices(&String::from_utf8_lossy(&output.stdout)))
}

/// Runs adb against one device and returns its trimmed output, failing when
/// the device does not answer within `timeout`: adb waits forever on a
/// device that is still booting.
fn adb_run(serial: &str, arguments: &[&str], timeout: Duration) -> Result<String> {
	let command = arguments.join(" ");
	let mut child = adb(serial)
		.args(arguments)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.with_context(|| format!("could not run `adb {command}` on {serial}"))?;
	let deadline = Instant::now() + timeout;

	while child.try_wait()?.is_none() {
		if Instant::now() >= deadline {
			let _ = child.kill();
			let _ = child.wait();
			anyhow::bail!(
				"`adb {command}` got no answer from {serial} within {} seconds",
				timeout.as_secs()
			);
		}

		std::thread::sleep(Duration::from_millis(100));
	}

	let output = child.wait_with_output()?;
	anyhow::ensure!(
		output.status.success(),
		"`adb {command}` failed on {serial}: {}",
		String::from_utf8_lossy(&output.stderr).trim()
	);
	Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn adb_shell(serial: &str, command: &str) -> Result<String> {
	adb_run(serial, &["shell", command], ADB_TIMEOUT)
}

/// Whether Android on `serial` has finished starting, asked briefly so a
/// device that is still booting cannot stall the caller.
fn booted(serial: &str) -> bool {
	adb_run(
		serial,
		&["shell", "getprop sys.boot_completed"],
		BOOT_PROBE_TIMEOUT,
	)
	.is_ok_and(|value| value == "1")
}

/// Every attached device that is ready, or a booted emulator when none is
/// attached, or only the serials in `wanted` when any are named. A device
/// that cannot be used is skipped with the reason.
fn android_devices(wanted: &[String]) -> Result<Vec<Device>> {
	let mut serials = adb_serials()?;

	if !wanted.is_empty() {
		serials.retain(|serial| wanted.contains(serial));

		if serials.is_empty() {
			return Ok(Vec::new());
		}
	} else if serials.is_empty() {
		serials.push(start_emulator()?);
	}

	let devices: Vec<Device> = serials
		.into_iter()
		.filter_map(|serial| {
			android_device(&serial)
				.inspect_err(|error| eprintln!("[android] skipping {serial}: {error:#}"))
				.ok()
		})
		.collect();
	anyhow::ensure!(!devices.is_empty(), "no attached Android device is ready");
	Ok(devices)
}

/// Waits for `serial` to finish booting, then reads which build it runs.
fn android_device(serial: &str) -> Result<Device> {
	if !booted(serial) {
		println!("[android] waiting for {serial} to finish booting");
		let deadline = Instant::now() + EMULATOR_BOOT;

		while !booted(serial) {
			anyhow::ensure!(
				Instant::now() < deadline,
				"it did not finish booting within {} seconds",
				EMULATOR_BOOT.as_secs()
			);
			std::thread::sleep(Duration::from_secs(1));
		}
	}

	let target = match adb_shell(serial, "getprop ro.product.cpu.abi")?.as_str() {
		"arm64-v8a" => Target::Android,
		"x86_64" => Target::AndroidX86_64,
		abi => anyhow::bail!("it runs {abi}; leptoskit builds arm64-v8a and x86_64"),
	};
	Ok(Device::Android {
		serial: serial.to_owned(),
		target,
	})
}

/// Boots the first installed emulator and returns its serial once Android
/// has started. It runs in its own process group, so it outlives this
/// `leptoskit dev` and the next one finds it already attached.
fn start_emulator() -> Result<String> {
	let output = Command::new(android_tool("emulator"))
		.arg("-list-avds")
		.output()
		.context("could not run the Android emulator")?;
	let avds = String::from_utf8_lossy(&output.stdout);
	let avd = avds
		.lines()
		.map(str::trim)
		.find(|line| !line.is_empty() && !line.starts_with("INFO"))
		.context(
			"no Android device is attached and no emulator is installed; create one in Android \
			 Studio's Device Manager",
		)?;
	println!("[android] starting the {avd} emulator");
	let mut command = Command::new(android_tool("emulator"));
	command
		.args(["-avd", avd])
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null());
	crate::testing::isolate(&mut command);
	command.spawn().context("could not start the emulator")?;
	let deadline = Instant::now() + EMULATOR_BOOT;

	loop {
		for serial in adb_serials()? {
			if serial.starts_with("emulator-") && booted(&serial) {
				return Ok(serial);
			}
		}

		anyhow::ensure!(
			Instant::now() < deadline,
			"the {avd} emulator did not finish booting within {} seconds",
			EMULATOR_BOOT.as_secs()
		);
		std::thread::sleep(Duration::from_secs(1));
	}
}

/// The native platforms `leptoskit dev` runs: the apps it launched, and the
/// server their guests come from. Dropping it stops the apps it started on
/// this machine.
pub struct NativeDev {
	root: PathBuf,
	/// One target per running platform, whose guest build serves all of the
	/// platform's devices.
	builds: Vec<(Platform, Target)>,
	guests: Guests,
	apps: Vec<Child>,
}

impl NativeDev {
	/// Serves guests on a loopback port, then builds and launches every
	/// platform's app, on the `wanted` devices only when any are named. A
	/// platform that fails to start is reported and skipped, so one missing
	/// toolchain does not stop the others.
	pub fn start(
		root: &Path,
		config: &LeptoskitConfig,
		platforms: &[Platform],
		wanted: &[String],
		runtime: &tokio::runtime::Runtime,
	) -> Result<Self> {
		let guests = Guests::new(platforms.iter().map(ToString::to_string));
		let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
			.context("could not bind the development guest server")?;
		listener
			.set_nonblocking(true)
			.context("could not configure the development guest server")?;
		let port = listener.local_addr()?.port();
		let listener = {
			let _guard = runtime.enter();
			tokio::net::TcpListener::from_std(listener)
				.context("could not adopt the development guest listener")?
		};
		let router = guests.router();
		runtime.spawn(async move {
			if let Err(error) = axum::serve(listener, router).await {
				eprintln!("development guest server stopped: {error}");
			}
		});

		let mut native = Self {
			root: root.to_path_buf(),
			builds: Vec::new(),
			guests,
			apps: Vec::new(),
		};

		for &platform in platforms {
			if let Err(error) = native.launch_platform(config, platform, wanted, port) {
				eprintln!("[{platform}] could not start: {error:#}");
			}
		}

		Ok(native)
	}

	/// Whether no native platform started.
	pub fn is_empty(&self) -> bool {
		self.builds.is_empty()
	}

	/// Builds the app once per target its devices need, then launches it on
	/// each device. A device that fails is reported and skipped.
	fn launch_platform(
		&mut self,
		config: &LeptoskitConfig,
		platform: Platform,
		wanted: &[String],
		port: u16,
	) -> Result<()> {
		let devices = devices(platform, wanted)?;

		if devices.is_empty() {
			println!("[{platform}] skipping: no {platform} device matches --device");
			return Ok(());
		}

		let mut built = Vec::new();

		for device in &devices {
			let target = device.target();

			if built.contains(&target) {
				continue;
			}

			println!("[{platform}] building the app for {}", device.label());
			crate::native::run(NativeCommand::Build {
				target,
				release: false,
			})?;
			self.guests.publish(
				&platform.to_string(),
				crate::native::built_guest(&self.root, target)?,
			);
			built.push(target);
		}

		for device in &devices {
			match launch(&self.root, config, device, platform, port) {
				Ok(apps) => {
					self.apps.extend(apps);
					println!("[{platform}] running on {}", device.label());
				}
				Err(error) => {
					eprintln!(
						"[{platform}] could not start on {}: {error:#}",
						device.label()
					);
				}
			}
		}

		if let Some(&target) = built.first() {
			self.builds.push((platform, target));
		}

		Ok(())
	}

	/// Rebuilds each platform's guest and offers it to the apps running it.
	pub fn reload(&self, config: &LeptoskitConfig) {
		for &(platform, target) in &self.builds {
			let started = Instant::now();

			match crate::native::rebuild_guest(&self.root, config, target) {
				Ok(wasm) => {
					self.guests.publish(&platform.to_string(), wasm);
					println!("[{platform}] guest rebuilt in {:?}", started.elapsed());
				}
				Err(error) => eprintln!("[{platform}] guest build failed: {error:#}"),
			}
		}
	}
}

impl Drop for NativeDev {
	fn drop(&mut self) {
		for app in &mut self.apps {
			let _ = app.kill();
			let _ = app.wait();
		}
	}
}

/// Installs and opens the app on `device`, pointed at its platform's guest.
/// Returns the processes whose output carries the app's logs.
fn launch(
	root: &Path,
	config: &LeptoskitConfig,
	device: &Device,
	platform: Platform,
	port: u16,
) -> Result<Vec<Child>> {
	let url = format!(
		"http://127.0.0.1:{port}{}",
		guest_path(&platform.to_string())
	);

	match device {
		Device::Local(Target::Macos) => {
			let mut command =
				Command::new(root.join(".leptoskit/macos/App.app/Contents/MacOS/App"));
			command.env(GUEST_ENV, &url);
			Ok(vec![spawn_logged(command, platform)?])
		}
		Device::Local(target) => {
			let executable = if *target == Target::Windows {
				format!("{}.exe", config.app.name)
			} else {
				config.app.name.clone()
			};
			let mut command = Command::new(
				crate::native::generated_dir(root, *target)
					.join("app")
					.join(executable),
			);
			command.env(GUEST_ENV, &url);
			Ok(vec![spawn_logged(command, platform)?])
		}
		Device::Simulator { udid, .. } => {
			let app =
				root.join(".leptoskit/ios/build/Build/Products/Debug-iphonesimulator/App.app");
			run_tool(
				Command::new("xcrun")
					.args(["simctl", "install", udid])
					.arg(&app),
			)?;
			let mut command = Command::new("xcrun");
			command
				.args([
					"simctl",
					"launch",
					"--console-pty",
					"--terminate-running-process",
					udid,
					&config.native.package_id,
				])
				.env(format!("SIMCTL_CHILD_{GUEST_ENV}"), &url);
			Ok(vec![spawn_logged(command, platform)?])
		}
		Device::Android { serial, target } => {
			let package = &config.native.package_id;
			// The app reaches the server at its own loopback address.
			let forward = format!("tcp:{port}");
			adb_run(serial, &["reverse", &forward, &forward], ADB_TIMEOUT)?;
			let apk = newest_apk(
				&crate::native::host_dir(root, *target).join("app/build/outputs/apk/debug"),
			)?;
			let apk = apk.to_str().context("the APK path is not UTF-8")?;
			adb_run(serial, &["install", "-r", apk], ADB_INSTALL_TIMEOUT)?;
			let resolved = adb_shell(
				serial,
				&format!(
					"cmd package resolve-activity --brief {}",
					shell_quote(package)
				),
			)?;
			let activity = resolved
				.lines()
				.rev()
				.find(|line| line.contains('/'))
				.with_context(|| format!("{package} has no launcher activity"))?;
			adb_shell(
				serial,
				&format!(
					"am start -W -S -n {} --es {GUEST_EXTRA} {}",
					shell_quote(activity),
					shell_quote(&url)
				),
			)?;
			let pid = adb_shell(serial, &format!("pidof {}", shell_quote(package)))?;
			let mut logcat = adb(serial);
			logcat.args(["logcat", "-v", "tag", "-T", "1", &format!("--pid={pid}")]);
			Ok(vec![spawn_logged(logcat, platform)?])
		}
	}
}

/// The most recently built APK in `directory`.
fn newest_apk(directory: &Path) -> Result<PathBuf> {
	let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;

	for entry in std::fs::read_dir(directory)
		.with_context(|| format!("could not read {}", directory.display()))?
	{
		let path = entry?.path();

		if path.extension().is_none_or(|extension| extension != "apk") {
			continue;
		}

		let modified = path.metadata()?.modified()?;

		if newest.as_ref().is_none_or(|(time, _)| modified > *time) {
			newest = Some((modified, path));
		}
	}

	newest
		.map(|(_, path)| path)
		.with_context(|| format!("Gradle built no APK in {}", directory.display()))
}

/// Starts `command` with each line of its output prefixed by the platform.
fn spawn_logged(mut command: Command, platform: Platform) -> Result<Child> {
	command
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped());
	let mut child = command
		.spawn()
		.with_context(|| format!("could not start {}", command.get_program().display()))?;
	let streams: [Option<Box<dyn Read + Send>>; 2] = [
		child
			.stdout
			.take()
			.map(|stream| Box::new(stream) as Box<dyn Read + Send>),
		child
			.stderr
			.take()
			.map(|stream| Box::new(stream) as Box<dyn Read + Send>),
	];

	for stream in streams.into_iter().flatten() {
		std::thread::spawn(move || {
			for line in BufReader::new(stream).lines().map_while(Result::ok) {
				println!("[{platform}] {}", line.trim_end_matches('\r'));
			}
		});
	}

	Ok(child)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn enabled(platforms: &[Platform]) -> LeptoskitConfig {
		let mut config = LeptoskitConfig::default();
		config.app.platforms = platforms.to_vec();
		config
	}

	#[expect(
		clippy::unnecessary_wraps,
		reason = "it stands in for the support check, which can fail"
	)]
	fn anywhere(_: Platform) -> Result<(), String> {
		Ok(())
	}

	fn only_apple(platform: Platform) -> Result<(), String> {
		match platform {
			Platform::Windows | Platform::Linux => Err("not here".to_owned()),
			_ => Ok(()),
		}
	}

	#[test]
	fn by_default_every_enabled_platform_runs_and_the_rest_are_skipped() {
		let config = enabled(&Platform::APPLICATION);
		let selection = select_with(&config, &[], only_apple).unwrap();
		assert!(selection.web);
		assert_eq!(
			selection.native,
			[Platform::Ios, Platform::Android, Platform::Macos]
		);
		assert_eq!(
			selection.skipped,
			[
				(Platform::Windows, "not here".to_owned()),
				(Platform::Linux, "not here".to_owned())
			]
		);
	}

	#[test]
	fn named_platforms_run_alone_and_must_be_able_to() {
		let config = enabled(&Platform::APPLICATION);
		let selection =
			select_with(&config, &[Platform::Android, Platform::Android], anywhere).unwrap();
		assert_eq!(
			selection,
			Selection {
				web: false,
				native: vec![Platform::Android],
				skipped: Vec::new(),
			}
		);

		let error = select_with(&config, &[Platform::Linux], only_apple).unwrap_err();
		assert_eq!(error.to_string(), "cannot run linux here: not here");
		let error = select_with(&config, &[Platform::Native], anywhere).unwrap_err();
		assert!(error.to_string().contains("names no platform"), "{error}");

		let web_only = enabled(&[Platform::Web]);
		assert!(select_with(&web_only, &[Platform::Ios], anywhere).is_err());
	}

	#[test]
	fn a_server_app_runs_its_server_and_a_native_app_has_no_web() {
		let mut config = enabled(&[Platform::Macos]);
		config.app.default_platform = Platform::Server;
		let selection = select_with(&config, &[], anywhere).unwrap();
		assert!(selection.web);
		assert_eq!(selection.native, [Platform::Macos]);

		config.app.default_platform = Platform::Native;
		assert!(!select_with(&config, &[], anywhere).unwrap().web);
		let error = select_with(&config, &[Platform::Web], anywhere).unwrap_err();
		assert!(error.to_string().contains("no web app"), "{error}");
	}

	fn udids(chosen: &[&Simulator]) -> Vec<String> {
		chosen
			.iter()
			.map(|simulator| simulator.udid.clone())
			.collect()
	}

	#[test]
	fn simulators_prefer_booted_devices_then_the_newest_iphone() {
		let list: SimulatorList = serde_json::from_str(
			r#"{"devices": {
				"com.apple.CoreSimulator.SimRuntime.iOS-18-5": [
					{"udid": "OLD", "name": "iPhone 17 Pro", "state": "Shutdown", "isAvailable": true}
				],
				"com.apple.CoreSimulator.SimRuntime.iOS-26-5": [
					{"udid": "IPAD", "name": "iPad Pro", "state": "Shutdown", "isAvailable": true},
					{"udid": "GONE", "name": "iPhone 17e", "state": "Shutdown", "isAvailable": false},
					{"udid": "NEW", "name": "iPhone 17 Pro", "state": "Shutdown", "isAvailable": true}
				],
				"com.apple.CoreSimulator.SimRuntime.watchOS-26-0": [
					{"udid": "WATCH", "name": "Apple Watch", "state": "Booted", "isAvailable": true}
				]
			}}"#,
		)
		.unwrap();
		assert_eq!(
			udids(&choose_simulators(&list, &[])),
			["NEW"],
			"a booted watch is no iOS device, so the newest iPhone boots"
		);
		assert_eq!(
			udids(&choose_simulators(
				&list,
				&["iPhone 17 Pro".into(), "IPAD".into()]
			)),
			["NEW", "IPAD"],
			"named devices run, the newest runtime first, by name or UDID"
		);
		assert!(choose_simulators(&list, &["iPhone 17e".into()]).is_empty());

		let booted: SimulatorList = serde_json::from_str(
			r#"{"devices": {
				"com.apple.CoreSimulator.SimRuntime.iOS-18-5": [
					{"udid": "ON", "name": "iPhone 17 Pro", "state": "Booted", "isAvailable": true}
				],
				"com.apple.CoreSimulator.SimRuntime.iOS-26-5": [
					{"udid": "OFF", "name": "iPhone 17 Pro", "state": "Shutdown", "isAvailable": true}
				]
			}}"#,
		)
		.unwrap();
		assert_eq!(udids(&choose_simulators(&booted, &[])), ["ON"]);
		assert_eq!(
			udids(&choose_simulators(&booted, &["iPhone 17 Pro".into()])),
			["ON"],
			"a booted simulator wins over a newer one with the same name"
		);
	}

	#[test]
	fn adb_devices_lists_only_ready_serials() {
		let output = "* daemon not running; starting now at tcp:5037\n* daemon started \
		              successfully\nList of devices \
		              attached\nemulator-5554\tdevice\nR58M\tunauthorized\n0A1B\toffline\n";
		assert_eq!(parse_adb_devices(output), ["emulator-5554"]);
	}
}
