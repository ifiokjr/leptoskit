//! Test execution on the current desktop, browser, or an explicitly selected
//! device.
//!
//! Every command has a deadline and writes stdout/stderr to an isolated
//! artifact directory. Device IDs are mandatory: a test must never land on an
//! arbitrary connected phone. Only simulators booted by this invocation are
//! shut down.

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::bail;
use clap::Args;
use clap::Subcommand;
use clap::ValueEnum;
use serde::Deserialize;
use serde::Serialize;

/// Testing commands.
#[derive(Debug, Subcommand)]
pub enum TestCommand {
	/// Runs real native UI/system-dialog scenarios through an OS driver.
	Native {
		/// JSON scenario containing the session and ordered actions/assertions.
		#[arg(long)]
		suite: PathBuf,
	},
	/// Runs a compiled test executable and retains its logs and JSON report.
	Run(RunArgs),
	/// Builds the browser extension and checks it in Chrome for Testing:
	/// every page shows something, the worker runs, and each content script
	/// puts its frame on a page with a strict content security policy.
	Extension,
	/// Runs browser component tests through wasm-bindgen-test.
	Web {
		/// Cargo package containing browser tests.
		#[arg(long, default_value = "leptoskit_testing_library")]
		package: String,
		/// Browser driver to use.
		#[arg(long, value_enum, default_value = "chrome")]
		browser: Browser,
		/// Maximum build and test duration.
		#[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(1..))]
		timeout: u64,
		/// Parent directory for a unique run's artifacts.
		#[arg(long, default_value = "target/leptoskit-tests")]
		artifacts: PathBuf,
	},
}

/// A desktop executable runs on the current OS; mobile binaries are
/// cross-compiled.
#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestPlatform {
	/// The current host (macOS, Windows, or Linux).
	Host,
	/// An Apple simulator, selected by UDID.
	Ios,
	/// An Android device or emulator, selected by adb serial.
	Android,
}

/// Browser used by wasm-bindgen-test's `WebDriver` runner.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Browser {
	/// Chrome via chromedriver.
	Chrome,
	/// Firefox via geckodriver.
	Firefox,
}

/// Options for a compiled test executable.
#[derive(Debug, Args)]
pub struct RunArgs {
	/// Compiled executable for the selected target.
	#[arg(long)]
	pub binary: PathBuf,
	/// Execution platform.
	#[arg(long, value_enum, default_value = "host")]
	pub platform: TestPlatform,
	/// iOS simulator UDID or Android adb serial (required for mobile).
	#[arg(long)]
	pub device: Option<String>,
	/// Boot this installed Android AVD on the port in --device emulator-PORT.
	#[arg(long)]
	pub avd: Option<String>,
	/// Deadline per setup, execution, or cleanup command, in seconds.
	#[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..))]
	pub timeout: u64,
	/// Parent directory for a unique run's logs and report.
	#[arg(long, default_value = "target/leptoskit-tests")]
	pub artifacts: PathBuf,
	/// Arguments passed literally to the test executable, after --.
	#[arg(last = true)]
	pub args: Vec<OsString>,
}

#[derive(Debug, Serialize)]
struct Step {
	name: String,
	exit_code: Option<i32>,
	timed_out: bool,
	elapsed_ms: u128,
	stdout: String,
	stderr: String,
}

#[derive(Debug, Serialize)]
struct Report {
	schema_version: u8,
	passed: bool,
	platform: String,
	device: Option<String>,
	error: Option<String>,
	steps: Vec<Step>,
}

struct Run {
	directory: PathBuf,
	timeout: Duration,
	report: Report,
}

static RUN_ID: AtomicU64 = AtomicU64::new(0);

impl Run {
	fn new(
		parent: &Path,
		timeout: u64,
		platform: String,
		device: Option<String>,
	) -> anyhow::Result<Self> {
		let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
		let directory = parent.join(format!(
			"{}-{stamp}-{}",
			std::process::id(),
			RUN_ID.fetch_add(1, Ordering::Relaxed)
		));
		fs::create_dir_all(parent)?;
		fs::create_dir(&directory)?;
		Ok(Self {
			directory,
			timeout: Duration::from_secs(timeout),
			report: Report {
				schema_version: 1,
				passed: false,
				platform,
				device,
				error: None,
				steps: Vec::new(),
			},
		})
	}

	fn command(&mut self, name: &str, command: &mut Command) -> anyhow::Result<PathBuf> {
		self.command_observing(name, command, None)
	}

	fn command_observing(
		&mut self,
		name: &str,
		command: &mut Command,
		mut companion: Option<&mut std::process::Child>,
	) -> anyhow::Result<PathBuf> {
		let prefix = format!("{:02}-{name}", self.report.steps.len());
		let stdout = format!("{prefix}.stdout.log");
		let stderr = format!("{prefix}.stderr.log");
		let output = self.directory.join(&stdout);
		command
			.stdin(Stdio::null())
			.stdout(fs::File::create(&output)?)
			.stderr(fs::File::create(self.directory.join(&stderr))?);
		isolate(command);
		let started = Instant::now();
		let mut child = command
			.spawn()
			.with_context(|| format!("starting {name}; check that its tool is installed"))?;
		let mut companion_failure = None;
		let (status, timed_out) = loop {
			if let Some(companion) = companion.as_mut()
				&& let Some(status) = companion.try_wait()?
			{
				companion_failure = Some(format!(
					"emulator exited during {name} ({status}); see emulator logs"
				));
				terminate(&mut child)?;
				break (child.wait()?, false);
			}
			if let Some(status) = child.try_wait()? {
				break (status, false);
			}
			if started.elapsed() >= self.timeout {
				terminate(&mut child).with_context(|| format!("terminating timed-out {name}"))?;
				break (child.wait()?, true);
			}
			std::thread::sleep(Duration::from_millis(20));
		};
		self.report.steps.push(Step {
			name: name.to_owned(),
			exit_code: status.code(),
			timed_out,
			elapsed_ms: started.elapsed().as_millis(),
			stdout,
			stderr,
		});
		if let Some(error) = companion_failure {
			bail!("{error}");
		}
		if timed_out {
			bail!(
				"{name} exceeded {} seconds; logs: {}",
				self.timeout.as_secs(),
				self.directory.display()
			);
		}
		if !status.success() {
			bail!(
				"{name} failed ({status}); logs: {}",
				self.directory.display()
			);
		}
		Ok(output)
	}

	fn finish(mut self, result: anyhow::Result<()>) -> anyhow::Result<()> {
		self.report.passed = result.is_ok();
		self.report.error = result.as_ref().err().map(|error| format!("{error:#}"));
		let path = self.directory.join("report.json");
		fs::write(&path, serde_json::to_vec_pretty(&self.report)?)?;
		println!("Test report: {}", path.display());
		result
	}
}

pub(crate) fn isolate(command: &mut Command) {
	#[cfg(unix)]
	{
		use std::os::unix::process::CommandExt;
		command.process_group(0);
	}
	#[cfg(not(unix))]
	let _ = command;
}

fn terminate(child: &mut std::process::Child) -> anyhow::Result<()> {
	#[cfg(unix)]
	{
		use rustix::process::Pid;
		use rustix::process::Signal;
		use rustix::process::kill_process_group;
		let pid = Pid::from_raw(i32::try_from(child.id())?).context("invalid process ID")?;
		match kill_process_group(pid, Signal::KILL) {
			Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
			Err(error) => Err(error.into()),
		}
	}
	#[cfg(windows)]
	{
		let status = Command::new("taskkill")
			.args(["/F", "/T", "/PID", &child.id().to_string()])
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status()?;
		if !status.success() && child.try_wait()?.is_none() {
			bail!("taskkill could not terminate the test process tree");
		}
		Ok(())
	}
	#[cfg(not(any(unix, windows)))]
	{
		child.kill()?;
		Ok(())
	}
}

/// Dispatches testing commands and returns an error on any failed phase.
pub fn run(command: TestCommand) -> anyhow::Result<()> {
	match command {
		TestCommand::Native { suite } => {
			let scenario: leptoskit_testing::native::suite::Suite = serde_json::from_slice(
				&fs::read(&suite).with_context(|| format!("reading {}", suite.display()))?,
			)?;
			let artifacts = scenario.run()?;
			println!(
				"Native test report: {}",
				artifacts.join("index.html").display()
			);
			Ok(())
		}
		TestCommand::Run(args) => run_binary(&args),
		TestCommand::Extension => crate::extension::test::run(),
		TestCommand::Web {
			package,
			browser,
			timeout,
			artifacts,
		} => {
			let mut run = Run::new(&artifacts, timeout, "web".to_owned(), None)?;
			let mut command = Command::new("cargo");
			command
				.args([
					"test",
					"--target",
					"wasm32-unknown-unknown",
					"--package",
					&package,
				])
				.env("WASM_BINDGEN_USE_BROWSER", "1")
				.env("WASM_BINDGEN_TEST_ONLY_WEB", "1")
				.env_remove("NO_HEADLESS")
				.env_remove("CHROMEDRIVER")
				.env_remove("GECKODRIVER")
				.env_remove("SAFARIDRIVER");
			match browser {
				Browser::Chrome => {
					command.env(
						"CHROMEDRIVER",
						std::env::var_os("CHROMEDRIVER").unwrap_or_else(|| "chromedriver".into()),
					)
				}
				Browser::Firefox => {
					command.env(
						"GECKODRIVER",
						std::env::var_os("GECKODRIVER").unwrap_or_else(|| "geckodriver".into()),
					)
				}
			};
			let result = run.command("browser", &mut command).map(|_| ());
			run.finish(result)
		}
	}
}

fn run_binary(args: &RunArgs) -> anyhow::Result<()> {
	let mut run = Run::new(
		&args.artifacts,
		args.timeout,
		format!("{:?}", args.platform).to_lowercase(),
		args.device.clone(),
	)?;
	let result = (|| {
		let binary = fs::canonicalize(&args.binary).context("test executable does not exist")?;
		if !binary.is_file() {
			bail!("--binary must be a file");
		}
		match args.platform {
			TestPlatform::Host => {
				if args.device.is_some() || args.avd.is_some() {
					bail!("--device and --avd require a mobile platform");
				}
				run.command("test", Command::new(binary).args(&args.args))?;
				Ok(())
			}
			TestPlatform::Ios => ios(&mut run, args, &binary),
			TestPlatform::Android => android(&mut run, args, &binary),
		}
	})();
	run.finish(result)
}

#[derive(Deserialize)]
struct Simulators {
	devices: std::collections::BTreeMap<String, Vec<Simulator>>,
}

#[derive(Deserialize)]
struct Simulator {
	udid: String,
	state: String,
	#[serde(rename = "isAvailable")]
	available: bool,
}

fn device(args: &RunArgs) -> anyhow::Result<&str> {
	let id = args
		.device
		.as_deref()
		.context("--device is required for mobile tests")?;
	if id.is_empty() || id.starts_with('-') || id.chars().any(char::is_whitespace) {
		bail!("invalid device identifier");
	}
	Ok(id)
}

fn ios(run: &mut Run, args: &RunArgs, binary: &Path) -> anyhow::Result<()> {
	if !cfg!(target_os = "macos") {
		bail!("iOS simulators require a macOS host with Xcode");
	}
	if args.avd.is_some() {
		bail!("--avd is only supported for Android");
	}
	let id = device(args)?;
	let path = run.command(
		"devices",
		Command::new("xcrun").args(["simctl", "list", "devices", "available", "--json"]),
	)?;
	let devices: Simulators = serde_json::from_slice(&fs::read(path)?)?;
	let simulator = devices
		.devices
		.iter()
		.filter(|(runtime, _)| runtime.starts_with("com.apple.CoreSimulator.SimRuntime.iOS-"))
		.flat_map(|(_, devices)| devices)
		.find(|sim| sim.udid == id && sim.available)
		.context("selected simulator is not installed or available")?;
	let owned = match simulator.state.as_str() {
		"Booted" => false,
		"Shutdown" => true,
		state => bail!("simulator is {state}; wait for it to finish transitioning"),
	};
	if owned {
		run.command("boot", Command::new("xcrun").args(["simctl", "boot", id]))?;
	}
	let result = (|| {
		run.command(
			"boot-ready",
			Command::new("xcrun").args(["simctl", "bootstatus", id, "-b"]),
		)?;
		run.command(
			"test",
			Command::new("xcrun")
				.args(["simctl", "spawn", id])
				.arg(binary)
				.args(&args.args),
		)?;
		Ok(())
	})();
	let cleanup = if owned {
		run.command(
			"shutdown",
			Command::new("xcrun").args(["simctl", "shutdown", id]),
		)
		.map(|_| ())
	} else {
		Ok(())
	};
	combine(result, cleanup)
}

pub(crate) fn android_tool(name: &str) -> PathBuf {
	let executable = if cfg!(windows) {
		format!("{name}.exe")
	} else {
		name.to_owned()
	};
	std::env::var_os("ANDROID_HOME")
		.or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
		.map_or_else(
			|| PathBuf::from(&executable),
			|sdk| {
				PathBuf::from(sdk)
					.join(if name == "adb" {
						"platform-tools"
					} else {
						"emulator"
					})
					.join(&executable)
			},
		)
}

fn adb(id: &str) -> Command {
	let mut command = Command::new(android_tool("adb"));
	command.args(["-s", id]);
	command
}

/// Quote a single argument for Android's remote POSIX shell. adb does not
/// preserve argv boundaries across `shell`, even when Command does locally.
pub(crate) fn shell_quote(value: &str) -> String {
	format!("'{}'", value.replace('\'', "'\\''"))
}

fn android(run: &mut Run, args: &RunArgs, binary: &Path) -> anyhow::Result<()> {
	let id = device(args)?;
	let mut emulator = None;
	if let Some(avd) = &args.avd {
		let port: u16 = id
			.strip_prefix("emulator-")
			.context("--avd requires --device emulator-PORT")?
			.parse()?;
		if !(5554..=5682).contains(&port) || !port.is_multiple_of(2) {
			bail!("emulator port must be even and between 5554 and 5682");
		}
		let devices = run.command("devices", Command::new(android_tool("adb")).arg("devices"))?;
		if fs::read_to_string(devices)?
			.lines()
			.any(|line| line.split_whitespace().next() == Some(id))
		{
			bail!("device {id} already exists; omit --avd to use it");
		}
		let mut launch = Command::new(android_tool("emulator"));
		isolate(&mut launch);
		emulator = Some(
			launch
				.args([
					"-avd",
					avd,
					"-port",
					&port.to_string(),
					"-no-window",
					"-no-audio",
					"-no-snapshot-save",
				])
				.stdin(Stdio::null())
				.stdout(fs::File::create(run.directory.join("emulator.stdout.log"))?)
				.stderr(fs::File::create(run.directory.join("emulator.stderr.log"))?)
				.spawn()
				.context("starting Android emulator")?,
		);
	}
	let remote = format!(
		"/data/local/tmp/leptoskit-{}",
		run.directory
			.file_name()
			.context("run directory")?
			.to_string_lossy()
	);
	let mut installed = false;
	let result = (|| {
		run.command_observing(
			"device-ready",
			adb(id).arg("wait-for-device"),
			emulator.as_mut(),
		)?;
		let boot = format!(
			"timeout {} sh -c 'until [ \"$(getprop sys.boot_completed)\" = 1 ]; do sleep 1; done'",
			args.timeout
		);
		run.command_observing(
			"boot-ready",
			adb(id).args(["shell", &boot]),
			emulator.as_mut(),
		)?;
		if let Some(child) = &mut emulator
			&& let Some(status) = child.try_wait()?
		{
			bail!("the emulator launched for this run exited ({status})");
		}
		run.command(
			"install-directory",
			adb(id).args(["shell", &format!("mkdir {}", shell_quote(&remote))]),
		)?;
		installed = true;
		let executable = format!("{remote}/test");
		run.command("install", adb(id).arg("push").arg(binary).arg(&executable))?;
		run.command(
			"permissions",
			adb(id).args(["shell", &format!("chmod 700 {}", shell_quote(&executable))]),
		)?;
		let mut command = format!(
			"timeout -s KILL {} {}",
			args.timeout,
			shell_quote(&executable)
		);
		for argument in &args.args {
			let argument = argument
				.to_str()
				.context("Android test arguments must be UTF-8")?;
			if argument.contains('\0') {
				bail!("test arguments cannot contain NUL");
			}
			command.push(' ');
			command.push_str(&shell_quote(argument));
		}
		run.command("test", adb(id).args(["shell", &command]))?;
		Ok(())
	})();
	let cleanup = if installed {
		run.command(
			"remove-test",
			adb(id).args(["shell", &format!("rm -rf {}", shell_quote(&remote))]),
		)
		.map(|_| ())
	} else {
		Ok(())
	};
	let result = combine(result, cleanup);
	if let Some(mut child) = emulator {
		// Terminate only the process tree we own. A serial can be reused by a
		// concurrent runner after an emulator exits, so adb emu kill is unsafe.
		let reaped = (|| -> anyhow::Result<()> {
			if child.try_wait()?.is_none() {
				terminate(&mut child)?;
			}
			child.wait()?;
			Ok(())
		})();
		return combine(result, reaped);
	}
	result
}

fn combine(result: anyhow::Result<()>, cleanup: anyhow::Result<()>) -> anyhow::Result<()> {
	match (result, cleanup) {
		(Ok(()), result) | (result, Ok(())) => result,
		(Err(error), Err(cleanup)) => {
			Err(error.context(format!("cleanup also failed: {cleanup:#}")))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn android_arguments_are_shell_literals() {
		assert_eq!(
			shell_quote("a 'quote'; $(touch /bad)"),
			"'a '\\''quote'\\''; $(touch /bad)'"
		);
		assert_eq!(shell_quote(""), "''");
	}

	#[test]
	fn an_exited_emulator_cancels_the_readiness_command() {
		let parent =
			std::env::temp_dir().join(format!("leptoskit-companion-{}", std::process::id()));
		let mut run = Run::new(&parent, 10, "test".to_owned(), None).unwrap();
		let exe = std::env::current_exe().unwrap();
		let mut companion = Command::new(&exe)
			.args(["--ignored", "--exact", "testing::tests::companion_exit"])
			.stdout(Stdio::null())
			.spawn()
			.unwrap();
		companion.wait().unwrap();
		let result = run.command_observing(
			"ready",
			Command::new(exe).args(["--ignored", "--exact", "testing::tests::companion_wait"]),
			Some(&mut companion),
		);
		assert!(result.unwrap_err().to_string().contains("emulator exited"));
		assert!(!run.report.steps[0].timed_out);
		assert_ne!(run.report.steps[0].exit_code, Some(0));
		fs::remove_dir_all(parent).unwrap();
	}

	#[test]
	#[ignore = "subprocess fixture: an_exited_emulator_cancels_the_readiness_command runs it"]
	fn companion_exit() {}

	#[test]
	#[ignore = "subprocess fixture: an_exited_emulator_cancels_the_readiness_command runs it"]
	fn companion_wait() {
		std::thread::sleep(Duration::from_secs(30));
	}
}
