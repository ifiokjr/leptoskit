//! Device steps: what `simctl`, `devicectl`, and `adb` can do with no driver
//! session at all.

use std::collections::BTreeMap;
use std::time::Duration;

use super::Error;
use super::Point;
use super::Target;
use super::TargetKind;
use super::tools;

fn device_id(target: &Target) -> Result<&str, Error> {
	target
		.device
		.as_deref()
		.filter(|id| !id.trim().is_empty())
		.ok_or_else(|| {
			Error::Configuration(format!(
				"{:?} device steps require target.device",
				target.kind
			))
		})
}

fn only(target: &Target, kind: TargetKind, what: &str) -> Result<(), Error> {
	if target.kind == kind {
		Ok(())
	} else {
		Err(Error::Configuration(format!(
			"{what} needs a {kind:?} target"
		)))
	}
}

/// Boots the target simulator and waits for it to finish booting.
pub fn boot(target: &Target, timeout: Duration) -> Result<(), Error> {
	only(target, TargetKind::IosSimulator, "booting")?;
	let id = device_id(target)?;
	let mut command = tools::simctl();
	command.arg("boot").arg(id);
	// An already-booted simulator answers "Unable to boot device in state
	// Booted"; waiting for it below is still the right outcome.
	if let Err(error) = super::tool(&mut command, timeout)
		&& !error.to_string().contains("Booted")
	{
		return Err(error);
	}
	let mut status = tools::simctl();

	status.args(["bootstatus", id, "-b"]);
	super::tool(&mut status, timeout)?;
	Ok(())
}

/// Shuts the target simulator down, or stops the emulator behind an
/// `emulator-PORT` serial.
pub fn shutdown(target: &Target, timeout: Duration) -> Result<(), Error> {
	let id = device_id(target)?;

	match target.kind {
		TargetKind::IosSimulator => {
			let mut command = tools::simctl();
			command.args(["shutdown", id]);
			if let Err(error) = super::tool(&mut command, timeout)
				&& !error.to_string().contains("Shutdown")
			{
				return Err(error);
			}
			Ok(())
		}

		TargetKind::Android if id.starts_with("emulator-") => {
			let mut command = tools::adb(id);
			command.args(["emu", "kill"]);
			super::tool(&mut command, timeout)?;
			Ok(())
		}

		TargetKind::Android => {
			Err(Error::Configuration(
				"shutting down a physical Android device is not a thing; stop the app instead"
					.to_owned(),
			))
		}
		kind => {
			Err(Error::Configuration(format!(
				"shutdown needs an ios_simulator or android emulator target, not {kind:?}"
			)))
		}
	}
}

/// Starts an emulator detached — it outlives this run — and waits for it to
/// finish booting.
pub fn start_emulator(
	target: &Target,
	avd: &str,
	port: Option<u16>,
	timeout: Duration,
) -> Result<(), Error> {
	only(target, TargetKind::Android, "starting an emulator")?;
	let serials = tools::android_list(timeout)
		.unwrap_or_default()
		.lines()
		.filter_map(|line| line.split_whitespace().next().map(str::to_owned))
		.collect::<Vec<_>>();

	let port = match port {
		Some(port) => port,
		None => {
			(5554..=5682)
				.step_by(2)
				.find(|candidate| {
					!serials
						.iter()
						.any(|serial| serial == &format!("emulator-{candidate}"))
				})
				.ok_or_else(|| {
					Error::Configuration("no free emulator port between 5554 and 5682".to_owned())
				})?
		}
	};

	if !(5554..=5682).contains(&port) || port % 2 != 0 {
		return Err(Error::Configuration(
			"emulator port must be even and between 5554 and 5682".to_owned(),
		));
	}

	let serial = format!("emulator-{port}");

	if serials.iter().any(|existing| existing == &serial) {
		return Err(Error::Configuration(format!(
			"device {serial} already exists"
		)));
	}

	let mut command = tools::android_tool("emulator");
	command
		.args(["-avd", avd, "-port", &port.to_string(), "-no-audio"])
		// Full snapshots would make two runs of one journey diverge.
		.arg("-no-snapshot-save");
	tools::spawn_detached(&mut command)?;
	let mut ready = tools::adb(&serial);
	ready.arg("wait-for-device");
	super::tool(&mut ready, timeout)?;
	let boot = "until [ \"$(getprop sys.boot_completed)\" = 1 ]; do sleep 1; done";
	let mut completed = tools::adb(&serial);
	completed.args([
		"shell",
		&format!("timeout {} sh -c '{boot}'", timeout.as_secs()),
	]);
	super::tool(&mut completed, timeout)?;
	Ok(())
}

/// Launches an installed app and returns the tool's output, which carries
/// the process id on simulators.
pub fn start_app(
	target: &Target,
	identifier: &str,
	arguments: &[String],
	environment: &BTreeMap<String, String>,
	timeout: Duration,
) -> Result<String, Error> {
	let id = device_id(target)?;

	match target.kind {
		TargetKind::IosSimulator => {
			let mut command = tools::simctl();
			command.arg("launch").arg(id).arg(identifier);
			for (name, value) in environment {
				// simctl passes child environment through prefixed variables.
				command.env(format!("SIMCTL_CHILD_{name}"), value);
			}

			command.args(arguments);
			Ok(super::tool(&mut command, timeout)?.stdout)
		}
		TargetKind::IosDevice => {
			if !arguments.is_empty() || !environment.is_empty() {
				return Err(Error::Configuration(
					"devicectl launches take no arguments or environment".to_owned(),
				));
			}

			let mut command = tools::devicectl();
			command
				.args(["device", "process", "launch"])
				.arg("--device")
				.arg(id)
				.arg(identifier);
			Ok(super::tool(&mut command, timeout)?.stdout)
		}
		TargetKind::Android => {
			if !environment.is_empty() {
				return Err(Error::Configuration(
					"Android launches take no environment through am".to_owned(),
				));
			}

			let component = launcher_activity(id, identifier, timeout)?;
			let mut command = tools::adb(id);
			command
				.args(["shell", "am", "start", "-n", &component])
				.args(arguments);
			Ok(super::tool(&mut command, timeout)?.stdout)
		}
		kind => {
			Err(Error::Configuration(format!(
				"start_app needs a mobile target, not {kind:?}"
			)))
		}
	}
}

/// Resolves a package's launcher activity, like `dev.app/.MainActivity`.
fn launcher_activity(serial: &str, package: &str, timeout: Duration) -> Result<String, Error> {
	let mut command = tools::adb(serial);
	command.args([
		"shell",
		"cmd",
		"package",
		"resolve-activity",
		"--brief",
		package,
	]);
	let output = super::tool(&mut command, timeout)?;
	let component = output
		.stdout
		.lines()
		.rev()
		.find_map(|line| line.split_whitespace().next())
		.filter(|line| line.contains('/'))
		.ok_or_else(|| {
			Error::Configuration(format!(
				"no launcher activity found for {package}; is it installed?"
			))
		})?;
	Ok(component.to_owned())
}

/// Terminates an installed app.
pub fn stop_app(target: &Target, identifier: &str, timeout: Duration) -> Result<(), Error> {
	let id = device_id(target)?;

	match target.kind {
		TargetKind::IosSimulator => {
			let mut command = tools::simctl();
			command.args(["terminate", id, identifier]);
			super::tool(&mut command, timeout)?;
			Ok(())
		}
		TargetKind::IosDevice => {
			let mut command = tools::devicectl();
			command
				.args(["device", "process", "terminate"])
				.arg("--device")
				.arg(id)
				.arg(identifier);
			super::tool(&mut command, timeout)?;
			Ok(())
		}
		TargetKind::Android => {
			let mut command = tools::adb(id);
			command.args(["shell", "am", "force-stop", identifier]);
			super::tool(&mut command, timeout)?;
			Ok(())
		}
		kind => {
			Err(Error::Configuration(format!(
				"stop_app needs a mobile target, not {kind:?}"
			)))
		}
	}
}

/// Captures the device's whole screen as PNG bytes.
pub fn screenshot(target: &Target, timeout: Duration) -> Result<Vec<u8>, Error> {
	let id = device_id(target)?;

	match target.kind {
		TargetKind::IosSimulator => {
			let path = std::env::temp_dir().join(format!(
				"leptoskit-screenshot-{}-{}.png",
				std::process::id(),
				std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.expect("clock moved backwards")
					.as_nanos()
			));
			let mut command = tools::simctl();
			command.args(["io", id, "screenshot"]).arg(&path);
			let bytes = match super::tool(&mut command, timeout) {
				Ok(_) => std::fs::read(&path),
				Err(error) => {
					let _ = std::fs::remove_file(&path);
					return Err(error);
				}
			};

			let _ = std::fs::remove_file(&path);
			Ok(bytes?)
		}
		TargetKind::Android => {
			let mut command = tools::adb(id);
			command.args(["exec-out", "screencap", "-p"]);
			Ok(super::tool(&mut command, timeout)?.raw)
		}
		kind => {
			Err(Error::Configuration(format!(
				"device screenshots need an ios_simulator or android target, not {kind:?}; the \
				 screenshot step works everywhere a session does"
			)))
		}
	}
}

/// Taps absolute screen pixels with Android's input tool.
pub fn tap_point(target: &Target, x: i64, y: i64, timeout: Duration) -> Result<(), Error> {
	only(target, TargetKind::Android, "coordinate taps")?;
	let mut command = tools::adb(device_id(target)?);
	command.args(["shell", "input", "tap", &x.to_string(), &y.to_string()]);
	super::tool(&mut command, timeout)?;
	Ok(())
}

/// Drags between absolute screen pixels with Android's input tool.
pub fn swipe(
	target: &Target,
	start: Point,
	end: Point,
	duration_ms: Option<u64>,
	timeout: Duration,
) -> Result<(), Error> {
	only(target, TargetKind::Android, "coordinate swipes")?;
	let duration = duration_ms.unwrap_or(300);
	let mut command = tools::adb(device_id(target)?);
	command.args([
		"shell",
		"input",
		"swipe",
		&start.x.to_string(),
		&start.y.to_string(),
		&end.x.to_string(),
		&end.y.to_string(),
		&duration.to_string(),
	]);
	super::tool(&mut command, timeout)?;
	Ok(())
}

/// Presses one Android key event code.
pub fn key(target: &Target, key: &str, timeout: Duration) -> Result<(), Error> {
	only(target, TargetKind::Android, "key events")?;

	if key.is_empty() || key.contains(char::is_whitespace) {
		return Err(Error::Configuration(
			"key must be one keyevent code".to_owned(),
		));
	}

	let mut command = tools::adb(device_id(target)?);
	command.args(["shell", "input", "keyevent", key]);
	super::tool(&mut command, timeout)?;
	Ok(())
}

/// Reads recent system logs.
pub fn logs(
	target: &Target,
	tail: Option<u64>,
	minutes: Option<u64>,
	timeout: Duration,
) -> Result<String, Error> {
	let id = device_id(target)?;

	let text = match target.kind {
		TargetKind::Android => {
			let lines = tail.unwrap_or(500).to_string();
			let mut command = tools::adb(id);
			command.args(["shell", "logcat", "-d", "-v", "time", "-t", &lines]);
			super::tool(&mut command, timeout)?.stdout
		}
		TargetKind::IosSimulator => {
			let window = format!("--last={}m", minutes.unwrap_or(5));
			let mut command = tools::simctl();
			command.args(["spawn", id, "log", "show", &window, "--style", "compact"]);
			let full = super::tool(&mut command, timeout)?.stdout;
			let keep = tail.unwrap_or(500) as usize;
			if full.lines().count() > keep {
				let skipped = full.lines().count() - keep;
				let kept: Vec<&str> = full.lines().skip(skipped).collect();
				format!("… {} earlier lines …\n{}", skipped, kept.join("\n"))
			} else {
				full
			}
		}
		kind => {
			return Err(Error::Configuration(format!(
				"logs need an android or ios_simulator target, not {kind:?}"
			)));
		}
	};

	Ok(text)
}
