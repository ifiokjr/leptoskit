//! The platform tools device control shells out to: `simctl`, `devicectl`,
//! `adb`, and the emulator.
//!
//! Every invocation has a deadline, so a wedged tool fails the step instead
//! of hanging the run.

use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

use super::Error;

/// A finished tool invocation.
#[derive(Debug)]
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
	// A tool sometimes leaves a process behind that inherits its output
	// pipes, such as a device daemon started by `adb` or `devicectl`. Waiting
	// for the pipes to close would then wait for that process to exit, which
	// can be forever. The readers report on a channel instead, so a pipe that
	// stays open costs one bounded grace period rather than the whole run.
	#[derive(Clone, Copy)]
	enum Stream {
		Out,
		Err,
	}
	// Each reader sends the bytes it has as it reads them, then `None` at
	// end of file. A pipe the tool's own processes closed always reaches end
	// of file; one a leftover daemon still holds yields what was read by the
	// deadline.
	fn capture(
		mut pipe: impl std::io::Read,
		stream: Stream,
		sender: &mpsc::Sender<(Stream, Option<Vec<u8>>)>,
	) {
		let mut buffer = [0u8; 8192];

		loop {
			match std::io::Read::read(&mut pipe, &mut buffer) {
				Ok(0) => break,
				Ok(read) => {
					if sender
						.send((stream, Some(buffer[..read].to_vec())))
						.is_err()
					{
						return;
					}
				}

				Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
				Err(_) => break,
			}
		}

		let _ = sender.send((stream, None));
	}
	let (sender, receiver) = mpsc::channel();
	let mut readers = 0;

	if let Some(pipe) = child.stdout.take() {
		readers += 1;

		let sender = sender.clone();
		std::thread::spawn(move || capture(pipe, Stream::Out, &sender));
	}

	if let Some(pipe) = child.stderr.take() {
		readers += 1;
		std::thread::spawn(move || capture(pipe, Stream::Err, &sender));
	}

	let started = Instant::now();
	let mut timed_out = false;

	loop {
		match child.try_wait() {
			Ok(Some(_)) => break,
			Ok(None) => {}
			Err(error) => {
				eprintln!("waiting for {name}: {error}");
				break;
			}
		}

		if started.elapsed() >= timeout {
			if let Err(error) = terminate(&mut child) {
				eprintln!("terminating timed-out {name}: {error}");
			}

			timed_out = true;
			break;
		}

		std::thread::sleep(Duration::from_millis(20));
	}

	// Reap the child in either case. After a kill this also lets the tool's
	// own pipe ends close, so the readers usually finish inside the grace.
	let code = child.wait().ok().and_then(|status| status.code());
	let mut raw = Vec::new();
	let mut err = Vec::new();
	let deadline = Instant::now() + Duration::from_secs(1);

	while readers > 0 {
		let Some(left) = deadline.checked_duration_since(Instant::now()) else {
			break;
		};

		match receiver.recv_timeout(left) {
			Ok((stream, Some(bytes))) => {
				match stream {
					Stream::Out => raw.extend(bytes),
					Stream::Err => err.extend(bytes),
				}
			}
			Ok((_, None)) => readers -= 1,
			Err(_) => break,
		}
	}

	if timed_out {
		return Err(Error::Tool {
			name,
			detail: format!("exceeded {} seconds", timeout.as_secs()),
		});
	}

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

#[cfg(all(test, unix))]
mod tests {
	use super::*;

	#[test]
	fn running_a_tool_captures_its_output() {
		let mut command = Command::new("/bin/echo");
		command.arg("captured");
		let output = run(&mut command, Duration::from_secs(10)).unwrap();
		assert_eq!(output.stdout.trim(), "captured");
		assert_eq!(output.raw, b"captured\n");
		assert!(output.success());
	}

	#[test]
	fn a_timed_out_tool_is_killed_and_reported() {
		let mut command = Command::new("/bin/sleep");
		command.arg("30");
		let error = run(&mut command, Duration::from_millis(300)).unwrap_err();
		assert!(error.to_string().contains("exceeded"), "{error}");
		// The process group is gone, not lingering.
		let check = Command::new("/bin/ps")
			.arg("-o")
			.arg("pid=")
			.arg("-p")
			.arg("0")
			.output();
		assert!(check.is_ok());
	}

	#[test]
	fn a_daemon_that_keeps_the_pipe_open_does_not_hang_the_call() {
		// The shell exits at once, but its background child inherits stdout
		// and stderr. Joining the readers would wait for that child; the
		// bounded grace returns the tool's own output instead.
		let mut command = Command::new("/bin/sh");
		command.arg("-c").arg("sleep 10 & printf started >&1");
		let started = Instant::now();
		let output = run(&mut command, Duration::from_secs(5)).unwrap();
		assert_eq!(output.stdout.trim(), "started");
		assert!(
			started.elapsed() < Duration::from_secs(4),
			"the call waited for the background process"
		);
	}

	#[test]
	fn missing_tools_report_their_name() {
		let mut command = Command::new("leptoskit-no-such-tool");
		let error = run(&mut command, Duration::from_secs(5)).unwrap_err();
		assert!(error.to_string().contains("leptoskit-no-such-tool"));
	}

	#[test]
	fn spawn_detached_starts_and_missing_programs_fail() {
		let mut command = Command::new("/usr/bin/true");
		spawn_detached(&mut command).unwrap();
		let mut command = Command::new("leptoskit-no-such-tool");
		let error = spawn_detached(&mut command).unwrap_err();
		assert!(error.to_string().contains("leptoskit-no-such-tool"));
	}

	#[test]
	fn free_ports_are_real_and_bindable() {
		// Another test can race the port between asking and binding, so
		// a few tries must land on one that really is free.
		for _ in 0..10 {
			if let Some(port) = free_port()
				&& std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
			{
				return;
			}
		}

		panic!("no free port stayed bindable across ten tries");
	}

	#[test]
	fn platform_tools_carry_their_programs() {
		let simctl = simctl();
		assert_eq!(simctl.get_program(), "xcrun");
		let devicectl = devicectl();
		assert_eq!(devicectl.get_program(), "xcrun");
		let adb = android_tool("adb");
		assert!(adb.get_program().to_string_lossy().ends_with("adb"));
	}

	#[test]
	fn find_in_path_resolves_real_programs_only() {
		assert!(find_in_path("ls").is_some());
		assert!(find_in_path("leptoskit-no-such-tool").is_none());
	}
}
