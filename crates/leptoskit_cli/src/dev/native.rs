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
use crate::testing::android_tool;
use crate::testing::shell_quote;
use crate::ui;

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
/// platform `app.platforms` enables, skipping those `tools` say this machine
/// cannot run. A requested platform that cannot run is an error instead of a
/// skip.
pub fn select(
	config: &LeptoskitConfig,
	requested: &[Platform],
	tools: &Tools,
) -> Result<Selection> {
	select_with(config, requested, |platform| tools.support(platform))
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
			// `dev::run` runs a terminal app named alone before selecting.
			Platform::Tui => {
				anyhow::ensure!(
					!explicit,
					"the terminal app needs this terminal to itself; run it alone with `leptoskit \
					 dev --platform tui`"
				);
				selection.skipped.push((
					platform,
					"it needs this terminal to itself; run it alone with `leptoskit dev \
					 --platform tui`"
						.to_owned(),
				));
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

/// The programs `leptoskit dev` drives to run the native platforms: Xcode's
/// simulator tools, the Android SDK's, and the native builds. They are found
/// once, so every device and rebuild uses the same ones.
#[derive(Debug, Clone)]
pub struct Tools {
	xcrun: PathBuf,
	adb: PathBuf,
	emulator: PathBuf,
	/// Opens the Simulator app, to show the window of a simulator `dev`
	/// booted.
	open: PathBuf,
	/// Builds a platform's debug app, as `leptoskit native build` does, and
	/// returns the guest it embeds.
	build_app: fn(&Path, Target) -> Result<Vec<u8>>,
	/// Rebuilds only the guest a platform's debug app embeds.
	rebuild_guest: fn(&Path, &LeptoskitConfig, Target) -> Result<Vec<u8>>,
}

impl Tools {
	/// Xcode's tools from `PATH`, the Android SDK's from `ANDROID_HOME`, and
	/// the builds `leptoskit native build` runs.
	pub fn from_environment() -> Self {
		Self {
			xcrun: PathBuf::from("xcrun"),
			adb: android_tool("adb"),
			emulator: android_tool("emulator"),
			open: PathBuf::from("open"),
			build_app: build_debug_app,
			rebuild_guest: crate::native::rebuild_guest,
		}
	}

	fn xcrun(&self) -> Command {
		Command::new(&self.xcrun)
	}

	/// adb, addressed to the device `serial`.
	fn adb(&self, serial: &str) -> Command {
		let mut command = Command::new(&self.adb);
		command.args(["-s", serial]);
		command
	}

	/// Whether this machine has what `platform` needs, and if not, why.
	fn support(&self, platform: Platform) -> Result<(), String> {
		match platform {
			Platform::Macos | Platform::Ios if !cfg!(target_os = "macos") => {
				Err(format!("{platform} apps build on a Mac"))
			}
			Platform::Ios if !succeeds(self.xcrun().args(["--find", "simctl"])) => {
				Err("the iOS Simulator needs Xcode's command-line tools".to_owned())
			}
			Platform::Android if !succeeds(Command::new(&self.adb).arg("version")) => {
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

	/// The devices `platform` runs on: this machine for a desktop, every
	/// booted iOS simulator or attached Android device, or one started for
	/// the purpose when none is. Naming devices in `wanted`, by simulator
	/// name or UDID or by adb serial, narrows a mobile platform to those; a
	/// platform none of them belongs to gets no devices.
	fn devices(&self, platform: Platform, wanted: &[String]) -> Result<Vec<Device>> {
		match platform {
			Platform::Macos => Ok(vec![Device::Local(Target::Macos)]),
			Platform::Windows => Ok(vec![Device::Local(Target::Windows)]),
			Platform::Linux => Ok(vec![Device::Local(Target::Linux)]),
			Platform::Ios => self.simulators(wanted),
			Platform::Android => self.android_devices(wanted),
			other => anyhow::bail!("`leptoskit dev` has no devices for {other}"),
		}
	}

	fn simulators(&self, wanted: &[String]) -> Result<Vec<Device>> {
		let output = self
			.xcrun()
			.args(["simctl", "list", "devices", "--json"])
			.output()
			.context("could not list the iOS Simulators")?;
		anyhow::ensure!(output.status.success(), "xcrun simctl list failed");
		let list: SimulatorList = serde_json::from_slice(&output.stdout)
			.context("could not read the iOS Simulator list")?;
		let chosen = choose_simulators(&list, wanted);
		anyhow::ensure!(
			!chosen.is_empty() || !wanted.is_empty(),
			"no iPhone simulator is installed; add one in Xcode's Devices and Simulators window"
		);
		let mut booting = false;

		for simulator in chosen.iter().filter(|simulator| !simulator.booted()) {
			ui::tagged("ios", format_args!("booting {}", simulator.name));
			run_tool(
				self.xcrun()
					.args(["simctl", "bootstatus", &simulator.udid, "-b"]),
			)?;
			booting = true;
		}

		if booting {
			// The simulator runs headless without its window; showing it is a
			// convenience, so a failure to open it is not an error.
			let _ = Command::new(&self.open).args(["-a", "Simulator"]).status();
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

	fn adb_serials(&self) -> Result<Vec<String>> {
		let output = Command::new(&self.adb)
			.arg("devices")
			.output()
			.context("could not run adb")?;
		anyhow::ensure!(output.status.success(), "adb devices failed");
		Ok(parse_adb_devices(&String::from_utf8_lossy(&output.stdout)))
	}

	/// Runs adb against one device and returns its trimmed output, failing
	/// when the device does not answer within `timeout`: adb waits forever on
	/// a device that is still booting.
	fn adb_run(&self, serial: &str, arguments: &[&str], timeout: Duration) -> Result<String> {
		let command = arguments.join(" ");
		let mut child = self
			.adb(serial)
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
					timeout.as_secs_f32()
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

	fn adb_shell(&self, serial: &str, command: &str) -> Result<String> {
		self.adb_run(serial, &["shell", command], ADB_TIMEOUT)
	}

	/// Whether Android on `serial` has finished starting, asked briefly so a
	/// device that is still booting cannot stall the caller.
	fn booted(&self, serial: &str) -> bool {
		self.adb_run(
			serial,
			&["shell", "getprop sys.boot_completed"],
			BOOT_PROBE_TIMEOUT,
		)
		.is_ok_and(|value| value == "1")
	}

	/// Every attached device that is ready, or a booted emulator when none is
	/// attached, or only the serials in `wanted` when any are named. A device
	/// that cannot be used is skipped with the reason.
	fn android_devices(&self, wanted: &[String]) -> Result<Vec<Device>> {
		let mut serials = self.adb_serials()?;

		if !wanted.is_empty() {
			serials.retain(|serial| wanted.contains(serial));

			if serials.is_empty() {
				return Ok(Vec::new());
			}
		} else if serials.is_empty() {
			serials.push(self.start_emulator()?);
		}

		let devices: Vec<Device> = serials
			.into_iter()
			.filter_map(|serial| {
				self.android_device(&serial)
					.inspect_err(|error| {
						ui::tagged_error("android", format_args!("skipping {serial}: {error:#}"));
					})
					.ok()
			})
			.collect();
		anyhow::ensure!(!devices.is_empty(), "no attached Android device is ready");
		Ok(devices)
	}

	/// Waits for `serial` to finish booting, then reads which build it runs.
	fn android_device(&self, serial: &str) -> Result<Device> {
		if !self.booted(serial) {
			ui::tagged(
				"android",
				format_args!("waiting for {serial} to finish booting"),
			);
			let deadline = Instant::now() + EMULATOR_BOOT;

			while !self.booted(serial) {
				anyhow::ensure!(
					Instant::now() < deadline,
					"it did not finish booting within {} seconds",
					EMULATOR_BOOT.as_secs()
				);
				std::thread::sleep(Duration::from_secs(1));
			}
		}

		let target = match self
			.adb_shell(serial, "getprop ro.product.cpu.abi")?
			.as_str()
		{
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
	fn start_emulator(&self) -> Result<String> {
		let output = Command::new(&self.emulator)
			.arg("-list-avds")
			.output()
			.context("could not run the Android emulator")?;
		let avds = String::from_utf8_lossy(&output.stdout);
		let avd = avds
			.lines()
			.map(str::trim)
			.find(|line| !line.is_empty() && !line.starts_with("INFO"))
			.context(
				"no Android device is attached and no emulator is installed; create one in \
				 Android Studio's Device Manager",
			)?;
		ui::tagged("android", format_args!("starting the {avd} emulator"));
		let mut command = Command::new(&self.emulator);
		command
			.args(["-avd", avd])
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null());
		crate::testing::isolate(&mut command);
		command.spawn().context("could not start the emulator")?;
		let deadline = Instant::now() + EMULATOR_BOOT;

		loop {
			for serial in self.adb_serials()? {
				if serial.starts_with("emulator-") && self.booted(&serial) {
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

	/// Installs and opens the app on `device`, pointed at its platform's
	/// guest. Returns the processes whose output carries the app's logs.
	fn launch(
		&self,
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
				run_tool(self.xcrun().args(["simctl", "install", udid]).arg(&app))?;
				let mut command = self.xcrun();
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
				self.adb_run(serial, &["reverse", &forward, &forward], ADB_TIMEOUT)?;
				let apk = newest_apk(
					&crate::native::host_dir(root, *target).join("app/build/outputs/apk/debug"),
				)?;
				let apk = apk.to_str().context("the APK path is not UTF-8")?;
				self.adb_run(serial, &["install", "-r", apk], ADB_INSTALL_TIMEOUT)?;
				let resolved = self.adb_shell(
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
				self.adb_shell(
					serial,
					&format!(
						"am start -W -S -n {} --es {GUEST_EXTRA} {}",
						shell_quote(activity),
						shell_quote(&url)
					),
				)?;
				let pid = self.adb_shell(serial, &format!("pidof {}", shell_quote(package)))?;
				let mut logcat = self.adb(serial);
				logcat.args(["logcat", "-v", "tag", "-T", "1", &format!("--pid={pid}")]);
				Ok(vec![spawn_logged(logcat, platform)?])
			}
		}
	}
}

/// Builds `target`'s debug app with `leptoskit native build` and returns the
/// guest it embeds.
fn build_debug_app(root: &Path, target: Target) -> Result<Vec<u8>> {
	crate::native::run(NativeCommand::Build {
		target,
		release: false,
	})?;
	crate::native::built_guest(root, target)
}

/// The native platforms `leptoskit dev` runs: the apps it launched, and the
/// server their guests come from. Dropping it stops the apps it started on
/// this machine.
pub struct NativeDev {
	root: PathBuf,
	tools: Tools,
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
		tools: Tools,
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
				ui::warning(format_args!(
					"the development guest server stopped: {error}"
				));
			}
		});

		let mut native = Self {
			root: root.to_path_buf(),
			tools,
			builds: Vec::new(),
			guests,
			apps: Vec::new(),
		};

		for &platform in platforms {
			if let Err(error) = native.launch_platform(config, platform, wanted, port) {
				ui::tagged_error(platform, format_args!("could not start: {error:#}"));
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
		let devices = self.tools.devices(platform, wanted)?;

		if devices.is_empty() {
			ui::tagged(
				platform,
				format_args!("skipping: no {platform} device matches --device"),
			);
			return Ok(());
		}

		let mut built = Vec::new();

		for device in &devices {
			let target = device.target();

			if built.contains(&target) {
				continue;
			}

			ui::tagged(
				platform,
				format_args!("building the app for {}", device.label()),
			);
			let guest = (self.tools.build_app)(&self.root, target)?;
			self.guests.publish(&platform.to_string(), guest);
			built.push(target);
		}

		for device in &devices {
			match self
				.tools
				.launch(&self.root, config, device, platform, port)
			{
				Ok(apps) => {
					self.apps.extend(apps);
					ui::tagged(platform, format_args!("running on {}", device.label()));
				}
				Err(error) => {
					ui::tagged_error(
						platform,
						format_args!("could not start on {}: {error:#}", device.label()),
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

			match (self.tools.rebuild_guest)(&self.root, config, target) {
				Ok(wasm) => {
					self.guests.publish(&platform.to_string(), wasm);
					ui::tagged(
						platform,
						format_args!("guest rebuilt in {}", ui::seconds(started.elapsed())),
					);
				}
				Err(error) => {
					ui::tagged_error(platform, format_args!("guest build failed: {error:#}"));
				}
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
				ui::tagged(platform, line.trim_end_matches('\r'));
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
	fn the_terminal_app_is_skipped_by_default_and_never_shares_the_terminal() {
		let config = enabled(&[Platform::Web, Platform::Macos, Platform::Tui]);
		let selection = select_with(&config, &[], anywhere).unwrap();
		assert!(selection.web);
		assert_eq!(selection.native, [Platform::Macos]);
		assert_eq!(selection.skipped.len(), 1);
		assert_eq!(selection.skipped[0].0, Platform::Tui);
		assert!(
			selection.skipped[0].1.contains("--platform tui"),
			"{:?}",
			selection.skipped
		);

		let error = select_with(&config, &[Platform::Macos, Platform::Tui], anywhere).unwrap_err();
		assert!(error.to_string().contains("run it alone"), "{error}");
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

	#[expect(
		clippy::unnecessary_wraps,
		reason = "it stands in for a build, which can fail"
	)]
	fn built(_: &Path, _: Target) -> Result<Vec<u8>> {
		Ok(b"first guest".to_vec())
	}

	#[expect(
		clippy::unnecessary_wraps,
		reason = "it stands in for a build, which can fail"
	)]
	fn rebuilt(_: &Path, _: &LeptoskitConfig, _: Target) -> Result<Vec<u8>> {
		Ok(b"second guest".to_vec())
	}

	fn unbuildable(_: &Path, _: Target) -> Result<Vec<u8>> {
		anyhow::bail!("the toolchain is missing")
	}

	fn unrebuildable(_: &Path, _: &LeptoskitConfig, _: Target) -> Result<Vec<u8>> {
		anyhow::bail!("the guest does not compile")
	}

	/// Tools that find no device tool, with builds that succeed.
	fn tools(dir: &Path) -> Tools {
		let missing = dir.join("missing");
		Tools {
			xcrun: missing.clone(),
			adb: missing.clone(),
			emulator: missing.clone(),
			open: missing,
			build_app: built,
			rebuild_guest: rebuilt,
		}
	}

	fn app_config() -> LeptoskitConfig {
		let mut config = enabled(&Platform::APPLICATION);
		config.app.name = "counter".to_owned();
		config.native.package_id = "com.example.counter".to_owned();
		config
	}

	/// A program standing in for adb, xcrun, `open`, the emulator, or a built
	/// app: a shell script that appends each call's arguments to
	/// `<name>.log` beside it, then runs `body`.
	#[cfg(unix)]
	fn fake(dir: &Path, name: &str, body: &str) -> PathBuf {
		use std::os::unix::fs::PermissionsExt as _;

		std::fs::create_dir_all(dir).unwrap();
		let path = dir.join(name);
		let log = dir.join(format!("{name}.log"));
		std::fs::write(
			&path,
			format!(
				"#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n{body}\n",
				log.display()
			),
		)
		.unwrap();
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
		path
	}

	/// The arguments of every call to the fake `name` in `dir`, in order.
	#[cfg(unix)]
	fn calls(dir: &Path, name: &str) -> Vec<String> {
		std::fs::read_to_string(dir.join(format!("{name}.log")))
			.unwrap_or_default()
			.lines()
			.map(str::to_owned)
			.collect()
	}

	/// What a fake app wrote to `path`, once it has.
	#[cfg(unix)]
	fn written(path: &Path) -> String {
		let deadline = Instant::now() + Duration::from_secs(10);

		loop {
			if let Ok(text) = std::fs::read_to_string(path)
				&& !text.is_empty()
			{
				return text;
			}

			assert!(
				Instant::now() < deadline,
				"{} was never written",
				path.display()
			);
			std::thread::sleep(Duration::from_millis(20));
		}
	}

	#[cfg(unix)]
	fn finish(apps: Vec<Child>) {
		for mut app in apps {
			assert!(app.wait().unwrap().success());
		}
	}

	#[test]
	fn the_environment_names_the_device_tools() {
		let tools = Tools::from_environment();
		assert_eq!(tools.xcrun, Path::new("xcrun"));
		assert_eq!(tools.adb.file_stem().unwrap(), "adb");
		assert_eq!(tools.emulator.file_stem().unwrap(), "emulator");
		assert_eq!(tools.open, Path::new("open"));
	}

	#[test]
	fn support_names_what_this_machine_lacks() {
		let dir = tempfile::tempdir().unwrap();
		let tools = tools(dir.path());
		let reason = |platform| tools.support(platform).unwrap_err();
		assert!(reason(Platform::Android).contains("ANDROID_HOME"));
		assert!(reason(Platform::Harmonyos).contains("HarmonyOS"));

		if cfg!(target_os = "macos") {
			assert!(reason(Platform::Ios).contains("Xcode"));
			assert_eq!(tools.support(Platform::Macos), Ok(()));
		} else {
			assert!(reason(Platform::Ios).contains("on a Mac"));
			assert!(reason(Platform::Macos).contains("on a Mac"));
		}

		assert_eq!(
			tools.support(Platform::Linux).is_ok(),
			cfg!(target_os = "linux")
		);
		assert_eq!(tools.support(Platform::Windows).is_ok(), cfg!(windows));
		assert_eq!(tools.support(Platform::Web), Ok(()));

		let selection = select(&enabled(&[Platform::Web, Platform::Android]), &[], &tools).unwrap();
		assert!(selection.web);
		assert_eq!(selection.skipped[0].0, Platform::Android);
	}

	#[test]
	fn desktops_run_on_this_machine_and_only_native_platforms_have_devices() {
		let dir = tempfile::tempdir().unwrap();
		let tools = tools(dir.path());

		for (platform, target) in [
			(Platform::Macos, Target::Macos),
			(Platform::Windows, Target::Windows),
			(Platform::Linux, Target::Linux),
		] {
			let devices = tools.devices(platform, &[]).unwrap();
			assert_eq!(devices, [Device::Local(target)]);
			assert_eq!(devices[0].label(), "this machine");
		}

		assert!(tools.devices(Platform::Web, &[]).is_err());
		let error = tools.devices(Platform::Ios, &[]).unwrap_err();
		assert!(
			error
				.to_string()
				.contains("could not list the iOS Simulators"),
			"{error}"
		);
		let error = tools.devices(Platform::Android, &[]).unwrap_err();
		assert_eq!(error.to_string(), "could not run adb");
	}

	#[test]
	fn a_platform_that_cannot_start_is_reported_and_skipped() {
		let dir = tempfile::tempdir().unwrap();
		let mut tools = tools(dir.path());
		tools.build_app = unbuildable;
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let native = NativeDev::start(
			dir.path(),
			&app_config(),
			&[Platform::Linux, Platform::Android],
			&[],
			tools,
			&runtime,
		)
		.unwrap();
		assert!(native.is_empty());
	}

	#[test]
	fn a_device_that_fails_to_launch_leaves_its_guest_rebuilding() {
		let dir = tempfile::tempdir().unwrap();
		let mut tools = tools(dir.path());
		tools.rebuild_guest = unrebuildable;
		let runtime = tokio::runtime::Runtime::new().unwrap();
		// No app was built at the path a Linux build writes, so it cannot open.
		let native = NativeDev::start(
			dir.path(),
			&app_config(),
			&[Platform::Linux],
			&[],
			tools,
			&runtime,
		)
		.unwrap();
		assert!(!native.is_empty());
		native.reload(&app_config());
	}

	#[cfg(unix)]
	#[test]
	fn android_runs_wherever_adb_does() {
		let dir = tempfile::tempdir().unwrap();
		let mut tools = tools(dir.path());
		tools.adb = fake(dir.path(), "adb", "exit 0");
		assert_eq!(tools.support(Platform::Android), Ok(()));
	}

	#[cfg(unix)]
	#[test]
	fn android_runs_on_every_ready_device_and_skips_the_rest() {
		let dir = tempfile::tempdir().unwrap();
		let mut tools = tools(dir.path());
		tools.adb = fake(
			dir.path(),
			"adb",
			r#"case "$*" in
  devices) printf 'List of devices attached\nemulator-5554\tdevice\nR58M\tdevice\n0A1B\toffline\n' ;;
  *"getprop sys.boot_completed") echo 1 ;;
  "-s emulator-5554 shell getprop ro.product.cpu.abi") echo x86_64 ;;
  "-s R58M shell getprop ro.product.cpu.abi") echo armeabi-v7a ;;
  *) exit 1 ;;
esac"#,
		);
		assert_eq!(
			tools.devices(Platform::Android, &[]).unwrap(),
			[Device::Android {
				serial: "emulator-5554".to_owned(),
				target: Target::AndroidX86_64,
			}],
			"the 32-bit device is skipped"
		);
		let error = tools
			.devices(Platform::Android, &["R58M".to_owned()])
			.unwrap_err();
		assert_eq!(error.to_string(), "no attached Android device is ready");
		assert_eq!(
			tools
				.devices(Platform::Android, &["elsewhere".to_owned()])
				.unwrap(),
			[] as [Device; 0]
		);

		let runtime = tokio::runtime::Runtime::new().unwrap();
		let native = NativeDev::start(
			dir.path(),
			&app_config(),
			&[Platform::Android],
			&["elsewhere".to_owned()],
			tools,
			&runtime,
		)
		.unwrap();
		assert!(native.is_empty(), "no attached device matches --device");
	}

	#[cfg(unix)]
	#[test]
	fn devices_that_run_the_same_build_share_it() {
		static BUILDS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

		#[expect(
			clippy::unnecessary_wraps,
			reason = "it stands in for a build, which can fail"
		)]
		fn counted(_: &Path, _: Target) -> Result<Vec<u8>> {
			BUILDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
			Ok(b"shared guest".to_vec())
		}

		let dir = tempfile::tempdir().unwrap();
		let mut tools = tools(dir.path());
		tools.build_app = counted;
		tools.adb = fake(
			dir.path(),
			"adb",
			r#"case "$*" in
  devices) printf 'List of devices attached\nR58M\tdevice\n0A1B\tdevice\n' ;;
  *"getprop sys.boot_completed") echo 1 ;;
  *"getprop ro.product.cpu.abi") echo arm64-v8a ;;
  *) exit 1 ;;
esac"#,
		);
		let runtime = tokio::runtime::Runtime::new().unwrap();
		// Neither device answers `adb reverse`, so neither app opens.
		let native = NativeDev::start(
			dir.path(),
			&app_config(),
			&[Platform::Android],
			&[],
			tools,
			&runtime,
		)
		.unwrap();
		assert!(!native.is_empty());
		assert_eq!(BUILDS.load(std::sync::atomic::Ordering::SeqCst), 1);
	}

	#[cfg(unix)]
	#[test]
	fn a_device_that_is_still_booting_is_waited_for() {
		let dir = tempfile::tempdir().unwrap();
		let booted = dir.path().join("booted");
		let mut tools = tools(dir.path());
		tools.adb = fake(
			dir.path(),
			"adb",
			&format!(
				r#"case "$*" in
  *"getprop sys.boot_completed") if [ -f '{0}' ]; then echo 1; else touch '{0}'; fi ;;
  *"getprop ro.product.cpu.abi") echo arm64-v8a ;;
esac"#,
				booted.display()
			),
		);
		assert_eq!(
			tools.android_device("R58M").unwrap(),
			Device::Android {
				serial: "R58M".to_owned(),
				target: Target::Android,
			}
		);
		let probes = calls(dir.path(), "adb")
			.iter()
			.filter(|call| call.ends_with("sys.boot_completed"))
			.count();
		assert_eq!(probes, 2);
	}

	#[cfg(unix)]
	#[test]
	fn with_nothing_attached_the_first_emulator_boots() {
		let dir = tempfile::tempdir().unwrap();
		let running = dir.path().join("running");
		let mut tools = tools(dir.path());
		tools.emulator = fake(
			dir.path(),
			"emulator",
			&format!(
				r#"case "$1" in
  -list-avds) printf 'INFO    | Storing crashdata\nPixel_9\nPixel_Tablet\n' ;;
  -avd) touch '{}' ;;
esac"#,
				running.display()
			),
		);
		tools.adb = fake(
			dir.path(),
			"adb",
			&format!(
				r#"case "$*" in
  devices) printf 'List of devices attached\n'; if [ -f '{0}' ]; then printf 'emulator-5554\tdevice\n'; fi ;;
  *"getprop sys.boot_completed") if [ -f '{1}' ]; then echo 1; else touch '{1}'; fi ;;
  *"getprop ro.product.cpu.abi") echo arm64-v8a ;;
esac"#,
				running.display(),
				dir.path().join("probed").display()
			),
		);
		assert_eq!(
			tools.devices(Platform::Android, &[]).unwrap(),
			[Device::Android {
				serial: "emulator-5554".to_owned(),
				target: Target::Android,
			}]
		);
		assert_eq!(
			calls(dir.path(), "emulator"),
			["-list-avds", "-avd Pixel_9"]
		);

		tools.emulator = fake(dir.path(), "bare-emulator", "echo 'INFO    | no AVDs'");
		let error = tools.start_emulator().unwrap_err();
		assert!(
			error.to_string().contains("no emulator is installed"),
			"{error}"
		);
	}

	#[cfg(unix)]
	#[test]
	fn adb_commands_that_fail_or_hang_are_reported() {
		let dir = tempfile::tempdir().unwrap();
		let mut tools = tools(dir.path());
		tools.adb = fake(
			dir.path(),
			"adb",
			r#"case "$*" in
  "-s R58M shell hang") exec sleep 5 ;;
  *) echo 'device unauthorized' >&2; exit 1 ;;
esac"#,
		);
		let error = tools
			.adb_run("R58M", &["shell", "hang"], Duration::from_millis(200))
			.unwrap_err();
		assert_eq!(
			error.to_string(),
			"`adb shell hang` got no answer from R58M within 0.2 seconds"
		);
		let error = tools.adb_shell("R58M", "getprop").unwrap_err();
		assert_eq!(
			error.to_string(),
			"`adb shell getprop` failed on R58M: device unauthorized"
		);
		assert!(!tools.booted("R58M"));
		assert_eq!(
			tools.adb_serials().unwrap_err().to_string(),
			"adb devices failed"
		);

		tools.adb = dir.path().join("missing");
		let error = tools.adb_shell("R58M", "getprop").unwrap_err();
		assert_eq!(
			error.to_string(),
			"could not run `adb shell getprop` on R58M"
		);
	}

	#[cfg(unix)]
	#[test]
	fn a_named_simulator_boots_and_its_window_opens() {
		let dir = tempfile::tempdir().unwrap();
		let list = dir.path().join("simulators.json");
		std::fs::write(
			&list,
			r#"{"devices": {"com.apple.CoreSimulator.SimRuntime.iOS-26-5": [
				{"udid": "NEW", "name": "iPhone 17 Pro", "state": "Shutdown", "isAvailable": true},
				{"udid": "ON", "name": "iPad Pro", "state": "Booted", "isAvailable": true}
			]}}"#,
		)
		.unwrap();
		let mut tools = tools(dir.path());
		tools.xcrun = fake(
			dir.path(),
			"xcrun",
			&format!(
				r#"case "$*" in
  "simctl list devices --json") cat '{}' ;;
  "simctl bootstatus "*) ;;
  *) exit 1 ;;
esac"#,
				list.display()
			),
		);
		tools.open = fake(dir.path(), "open", "");
		assert_eq!(
			tools.devices(Platform::Ios, &[]).unwrap(),
			[Device::Simulator {
				udid: "ON".to_owned(),
				name: "iPad Pro".to_owned(),
			}],
			"the booted simulator runs and nothing boots"
		);
		assert_eq!(calls(dir.path(), "open"), [] as [String; 0]);

		let devices = tools
			.devices(Platform::Ios, &["iPhone 17 Pro".to_owned()])
			.unwrap();
		assert_eq!(
			devices,
			[Device::Simulator {
				udid: "NEW".to_owned(),
				name: "iPhone 17 Pro".to_owned(),
			}]
		);
		assert_eq!(devices[0].label(), "iPhone 17 Pro");
		assert_eq!(devices[0].target(), Target::IosSimulator);
		assert_eq!(
			calls(dir.path(), "xcrun").last().unwrap(),
			"simctl bootstatus NEW -b"
		);
		assert_eq!(calls(dir.path(), "open"), ["-a Simulator"]);
		assert_eq!(
			tools
				.devices(Platform::Ios, &["iPhone 99".to_owned()])
				.unwrap(),
			[] as [Device; 0]
		);
	}

	#[cfg(unix)]
	#[test]
	fn a_machine_without_simulators_says_how_to_add_one() {
		let dir = tempfile::tempdir().unwrap();
		let mut tools = tools(dir.path());
		let error = |tools: &Tools| tools.devices(Platform::Ios, &[]).unwrap_err().to_string();

		tools.xcrun = fake(dir.path(), "empty", r#"echo '{"devices": {}}'"#);
		assert!(error(&tools).contains("no iPhone simulator is installed"));
		tools.xcrun = fake(dir.path(), "garbled", "echo 'not json'");
		assert_eq!(error(&tools), "could not read the iOS Simulator list");
		tools.xcrun = fake(dir.path(), "failing", "exit 1");
		assert_eq!(error(&tools), "xcrun simctl list failed");

		let device = Device::Simulator {
			udid: "NEW".to_owned(),
			name: "iPhone 17 Pro".to_owned(),
		};
		assert!(
			tools
				.launch(dir.path(), &app_config(), &device, Platform::Ios, 4321)
				.is_err(),
			"an app that does not install is not launched"
		);
		tools.xcrun = fake(
			dir.path(),
			"unbootable",
			r#"case "$*" in
  "simctl list devices --json") echo '{"devices": {"com.apple.CoreSimulator.SimRuntime.iOS-26-5": [{"udid": "NEW", "name": "iPhone 17 Pro", "state": "Shutdown", "isAvailable": true}]}}' ;;
  *) exit 1 ;;
esac"#,
		);
		assert!(tools.devices(Platform::Ios, &[]).is_err());
	}

	#[cfg(unix)]
	#[test]
	fn desktop_apps_open_from_their_build_with_the_guest_url() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path();
		let tools = tools(root);
		let generated = crate::native::generated_dir(root, Target::Linux).join("app");

		for (platform, target, executable) in [
			(
				Platform::Macos,
				Target::Macos,
				root.join(".leptoskit/macos/App.app/Contents/MacOS/App"),
			),
			(Platform::Linux, Target::Linux, generated.join("counter")),
			(
				Platform::Windows,
				Target::Windows,
				generated.join("counter.exe"),
			),
		] {
			let url = root.join(format!("{platform}.url"));
			fake(
				executable.parent().unwrap(),
				executable.file_name().unwrap().to_str().unwrap(),
				&format!(
					"printf '%s' \"${GUEST_ENV}\" > '{}'\necho started\necho warning >&2",
					url.display()
				),
			);
			let apps = tools
				.launch(root, &app_config(), &Device::Local(target), platform, 4321)
				.unwrap();
			finish(apps);
			assert_eq!(
				std::fs::read_to_string(&url).unwrap(),
				format!("http://127.0.0.1:4321/_leptoskit/dev/{platform}/guest")
			);
		}
	}

	#[cfg(unix)]
	#[test]
	fn a_simulator_installs_the_app_and_opens_it_with_the_guest_url() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path();
		let url = root.join("ios.url");
		let mut tools = tools(root);
		tools.xcrun = fake(
			root,
			"xcrun",
			&format!(
				r#"case "$1 $2" in
  "simctl launch") printf '%s' "$SIMCTL_CHILD_{GUEST_ENV}" > '{}' ;;
esac"#,
				url.display()
			),
		);
		let device = Device::Simulator {
			udid: "NEW".to_owned(),
			name: "iPhone 17 Pro".to_owned(),
		};
		finish(
			tools
				.launch(root, &app_config(), &device, Platform::Ios, 4321)
				.unwrap(),
		);
		let app = root.join(".leptoskit/ios/build/Build/Products/Debug-iphonesimulator/App.app");
		assert_eq!(
			calls(root, "xcrun"),
			[
				format!("simctl install NEW {}", app.display()),
				"simctl launch --console-pty --terminate-running-process NEW com.example.counter"
					.to_owned(),
			]
		);
		assert_eq!(
			std::fs::read_to_string(&url).unwrap(),
			"http://127.0.0.1:4321/_leptoskit/dev/ios/guest"
		);
	}

	#[cfg(unix)]
	#[test]
	fn an_android_device_installs_the_newest_apk_and_opens_its_launcher_activity() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path();
		let apks =
			crate::native::host_dir(root, Target::Android).join("app/build/outputs/apk/debug");
		std::fs::create_dir_all(&apks).unwrap();
		std::fs::write(apks.join("output-metadata.json"), "{}").unwrap();
		std::fs::write(apks.join("app-debug.apk"), "new").unwrap();
		std::fs::write(apks.join("stale.apk"), "old").unwrap();
		std::fs::File::options()
			.write(true)
			.open(apks.join("stale.apk"))
			.unwrap()
			.set_modified(std::time::UNIX_EPOCH)
			.unwrap();
		let mut tools = tools(root);
		tools.adb = fake(
			root,
			"adb",
			r#"case "$*" in
  *"resolve-activity --brief 'com.example.counter'") printf 'priority=0 preferredOrder=0\ncom.example.counter/.MainActivity\n' ;;
  *"pidof 'com.example.counter'") echo 4242 ;;
  *" logcat "*) echo 'I/LeptosKit: running' ;;
esac"#,
		);
		let device = Device::Android {
			serial: "R58M".to_owned(),
			target: Target::Android,
		};
		assert_eq!(device.label(), "R58M");
		finish(
			tools
				.launch(root, &app_config(), &device, Platform::Android, 4321)
				.unwrap(),
		);
		assert_eq!(
			calls(root, "adb"),
			[
				"-s R58M reverse tcp:4321 tcp:4321".to_owned(),
				format!("-s R58M install -r {}", apks.join("app-debug.apk").display()),
				"-s R58M shell cmd package resolve-activity --brief 'com.example.counter'".to_owned(),
				"-s R58M shell am start -W -S -n 'com.example.counter/.MainActivity' --es \
				 leptoskit.dev_guest 'http://127.0.0.1:4321/_leptoskit/dev/android/guest'"
					.to_owned(),
				"-s R58M shell pidof 'com.example.counter'".to_owned(),
				"-s R58M logcat -v tag -T 1 --pid=4242".to_owned(),
			]
		);
	}

	#[cfg(unix)]
	#[test]
	fn an_android_launch_names_what_is_missing() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path();
		let mut tools = tools(root);
		tools.adb = fake(root, "adb", "echo 'No activity found'");
		let device = Device::Android {
			serial: "R58M".to_owned(),
			target: Target::Android,
		};
		let error = |tools: &Tools| {
			tools
				.launch(root, &app_config(), &device, Platform::Android, 4321)
				.unwrap_err()
				.to_string()
		};
		assert!(error(&tools).starts_with("could not read"));

		let apks =
			crate::native::host_dir(root, Target::Android).join("app/build/outputs/apk/debug");
		std::fs::create_dir_all(&apks).unwrap();
		assert!(error(&tools).starts_with("Gradle built no APK"));

		std::fs::write(apks.join("app-debug.apk"), "apk").unwrap();
		assert_eq!(
			error(&tools),
			"com.example.counter has no launcher activity"
		);
	}

	#[cfg(unix)]
	#[test]
	fn native_dev_launches_each_platform_and_serves_its_rebuilt_guest_until_dropped() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path();
		let url = root.join("linux.url");
		let pid = root.join("linux.pid");
		fake(
			&crate::native::generated_dir(root, Target::Linux).join("app"),
			"counter",
			&format!(
				"echo $$ > '{}'\nprintf '%s' \"${GUEST_ENV}\" > '{}'\nexec sleep 30",
				pid.display(),
				url.display()
			),
		);
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let native = NativeDev::start(
			root,
			&app_config(),
			&[Platform::Linux],
			&[],
			tools(root),
			&runtime,
		)
		.unwrap();
		assert!(!native.is_empty());

		let mut guests = leptoskit_codepush::dev::DevGuests::new(&written(&url), b"").unwrap();
		assert_eq!(guests.next_guest().unwrap(), Some(b"first guest".to_vec()));
		native.reload(&app_config());
		assert_eq!(guests.next_guest().unwrap(), Some(b"second guest".to_vec()));

		let pid = written(&pid);
		drop(native);
		let alive = Command::new("kill")
			.args(["-0", pid.trim()])
			.stderr(Stdio::null())
			.status()
			.unwrap()
			.success();
		assert!(!alive, "dropping the dev session stops the apps it started");
	}
}
