//! The platform tools device control shells out to: `simctl`, `devicectl`,
//! `adb`, and the emulator.
//!
//! Every invocation has a deadline, so a wedged tool fails the step instead
//! of hanging the run.

use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use super::Error;

/// A finished tool invocation.
pub struct ToolOutput {
	/// The captured standard output, lossily decoded.
	pub stdout: String,
	/// The captured standard error, lossily decoded.
	pub stderr: String,
	/// The raw captured standard output, for binary payloads like screenshots.
	pub raw: Vec<u8>,
	/// The exit code, when the process exited on its own.
	pub code: Option<i32>,
}

impl ToolOutput {
	/// Whether the tool exited successfully.
	pub fn success(&self) -> bool {
		self.code == Some(0)
	}

	/// The failure detail: the exit status plus any stderr excerpt.
	pub fn failure(&self) -> String {
		let excerpt: String = self.stderr.chars().take(400).collect();
		format!(
			"exit {}{}",
			self.code
				.map_or("killed".to_owned(), |code| code.to_string()),
			if excerpt.trim().is_empty() {
				String::new()
			} else {
				format!(": {}", excerpt.trim())
			}
		)
	}
}

/// Runs one command to completion under a deadline, terminating its process
/// group on expiry.
pub fn run(command: &mut Command, timeout: Duration) -> Result<ToolOutput, Error> {
	isolate(command);
	command
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped());
	let name = command.get_program().to_string_lossy().into_owned();
	let spawn = |command: &mut Command| {
		command.spawn().map_err(|error| {
			Error::Tool {
				name: name.clone(),
				detail: format!("starting: {error}"),
			}
		})
	};
	let mut child = spawn(command)?;
	let (raw, err, timed_out) = std::thread::scope(|scope| -> (Vec<u8>, Vec<u8>, bool) {
		// Drain the pipes on their own threads; a full pipe would otherwise
		// let the tool block on writing while the wait loop runs.
		let stdout = child.stdout.take().map(|mut pipe| {
			scope.spawn(move || {
				let mut bytes = Vec::new();
				std::io::Read::read_to_end(&mut pipe, &mut bytes).ok();
				bytes
			})
		});
		let stderr = child.stderr.take().map(|mut pipe| {
			scope.spawn(move || {
				let mut bytes = Vec::new();
				std::io::Read::read_to_end(&mut pipe, &mut bytes).ok();
				bytes
			})
		});
		let started = Instant::now();
		let mut timed_out = false;
		let status = loop {
			match child.try_wait() {
				Ok(Some(status)) => break Some(status),
				Ok(None) => {}
				Err(error) => {
					eprintln!("waiting for {name}: {error}");
					break None;
				}
			}
			if started.elapsed() >= timeout {
				if let Err(error) = terminate(&mut child) {
					eprintln!("terminating timed-out {name}: {error}");
				}
				timed_out = true;
				break None;
			}
			std::thread::sleep(Duration::from_millis(20));
		};
		if timed_out {
			// Give the killed process a moment to close its pipes so the
			// reader threads finish inside this scope.
			let _ = child.wait();
		}
		let _ = status;
		(
			stdout.map_or_else(Vec::new, |handle| handle.join().ok().unwrap_or_default()),
			stderr.map_or_else(Vec::new, |handle| handle.join().ok().unwrap_or_default()),
			timed_out,
		)
	});
	if timed_out {
		return Err(Error::Tool {
			name,
			detail: format!("exceeded {} seconds", timeout.as_secs()),
		});
	}
	let code = child.wait().ok().and_then(|status| status.code());
	Ok(ToolOutput {
		stdout: String::from_utf8_lossy(&raw).into_owned(),
		stderr: String::from_utf8_lossy(&err).into_owned(),
		raw,
		code,
	})
}

/// Spawns a detached process that outlives this run, for emulators a control
/// document boots but no process owns.
pub fn spawn_detached(command: &mut Command) -> Result<(), Error> {
	isolate(command);
	command
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()
		.map(|_| ())
		.map_err(|error| {
			Error::Tool {
				name: command.get_program().to_string_lossy().into_owned(),
				detail: format!("starting: {error}"),
			}
		})
}

fn isolate(command: &mut Command) {
	#[cfg(unix)]
	{
		use std::os::unix::process::CommandExt;
		command.process_group(0);
	}
	#[cfg(not(unix))]
	let _ = command;
}

fn terminate(child: &mut Child) -> Result<(), std::io::Error> {
	#[cfg(unix)]
	{
		use rustix::process::Pid;
		use rustix::process::Signal;
		use rustix::process::kill_process_group;
		let Some(pid) = i32::try_from(child.id()).ok().and_then(Pid::from_raw) else {
			return Ok(());
		};
		match kill_process_group(pid, Signal::KILL) {
			Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
			Err(error) => Err(std::io::Error::from(error)),
		}
	}
	#[cfg(not(unix))]
	{
		child.kill()
	}
}

/// Resolves `adb` or `emulator` against the Android SDK variables.
pub fn android_tool(name: &str) -> Command {
	let executable = if cfg!(windows) {
		format!("{name}.exe")
	} else {
		name.to_owned()
	};
	let path = std::env::var_os("ANDROID_HOME")
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
		);
	Command::new(path)
}

/// An `adb` invocation for one device serial.
pub fn adb(serial: &str) -> Command {
	let mut command = android_tool("adb");
	command.args(["-s", serial]);
	command
}

/// An `xcrun simctl` invocation.
pub fn simctl() -> Command {
	let mut command = Command::new("xcrun");
	command.arg("simctl");
	command
}

/// An `xcrun devicectl` invocation.
pub fn devicectl() -> Command {
	let mut command = Command::new("xcrun");
	command.arg("devicectl");
	command
}

/// Runs a listing command whose tool may not be installed, returning its
/// standard output or [`None`] when the tool is missing or fails.
pub fn list(mut command: Command, timeout: Duration) -> Option<String> {
	match run(&mut command, timeout) {
		Ok(output) if output.success() => Some(output.stdout),
		_ => None,
	}
}

/// Reads `xcrun simctl list devices --json`, or [`None`] without Xcode.
pub fn simulator_list(timeout: Duration) -> Option<String> {
	let mut command = Command::new("xcrun");
	command.args(["simctl", "list", "devices", "--json"]);
	list(command, timeout)
}

/// Reads `xcrun devicectl list devices --json`, or [`None`] without Xcode.
pub fn iphone_list(timeout: Duration) -> Option<String> {
	let mut command = Command::new("xcrun");
	command.args(["devicectl", "list", "devices", "--json"]);
	list(command, timeout)
}

/// Reads `adb devices -l`, or [`None`] without an Android SDK.
pub fn android_list(timeout: Duration) -> Option<String> {
	let mut command = android_tool("adb");
	command.args(["devices", "-l"]);
	list(command, timeout)
}

/// Reads the installed Android virtual devices, or [`None`] without an
/// Android SDK.
pub fn avd_list(timeout: Duration) -> Option<String> {
	let mut command = android_tool("emulator");
	command.arg("-list-avds");
	list(command, timeout)
}

/// Finds an executable on `PATH`, like `which`.
pub fn find_in_path(program: &str) -> Option<PathBuf> {
	let path = std::env::var_os("PATH")?;
	let executable = if cfg!(windows) {
		format!("{program}.exe")
	} else {
		program.to_owned()
	};
	std::env::split_paths(&path)
		.map(|directory| directory.join(&executable))
		.find(|candidate| candidate.is_file())
}

/// A free TCP port on loopback, handed back for a driver to bind. Racing
/// another process into the port fails its startup loudly rather than
/// silently misrouting.
pub fn free_port() -> Option<u16> {
	std::net::TcpListener::bind(("127.0.0.1", 0))
		.ok()?
		.local_addr()
		.ok()?
		.port()
		.into()
}
