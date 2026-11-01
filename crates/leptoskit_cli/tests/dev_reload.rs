//! `leptoskit dev` reloading a real app on a real platform: build and launch
//! it, edit its screen, and wait until every app `dev` launched shows the
//! rebuilt interface.
//!
//! Each test needs the platform's toolchain and a device, so all are ignored
//! by default. Run them explicitly, one platform or all:
//!
//! ```sh
//! cargo nextest run -p leptoskit_cli --test dev_reload --run-ignored ignored-only
//! ```
//!
//! iOS uses every booted iPhone simulator and boots one when none is.
//! Android uses every device `adb devices` lists and boots the first emulator
//! when none is attached; set `ANDROID_HOME`. Artifacts and the full `dev`
//! log survive under `target/dev-reload`.

use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// The first run builds the toolchain's dependencies and the platform app.
const FIRST_BUILD: Duration = Duration::from_mins(45);
/// A reload rebuilds one guest and swaps it into every running app.
const RELOAD: Duration = Duration::from_mins(5);
const ORIGINAL: &str = "Hello, leptoskit";
const EDITED: &str = "Reloaded by leptoskit dev";

#[test]
#[ignore = "builds and opens the macOS app; run explicitly on a Mac"]
fn macos_app_reloads_in_its_window() {
	reloads("macos", |_, _| {});
}

#[test]
#[ignore = "needs Xcode and an iOS Simulator; run explicitly on a Mac"]
fn ios_simulator_app_reloads() {
	reloads("ios", |_, _| {});
}

#[test]
#[ignore = "needs ANDROID_HOME and an attached device or installed emulator"]
fn android_app_reloads_on_every_attached_device() {
	reloads("android", |session, serials| {
		for serial in serials {
			let screen = android_screen(serial);
			assert!(
				screen.contains(&format!("text=\"{EDITED}\"")),
				"{serial} does not show the edited text\n{screen}\n{}",
				session.transcript()
			);
		}
	});
}

/// Runs `leptoskit dev --platform <platform>` on a new project, edits its
/// screen, and waits for every launched app to log its reload. `verify` then
/// checks the screens, given the device each app runs on.
fn reloads(platform: &str, verify: impl Fn(&Session, &[String])) {
	let app = scaffold(platform);
	let mut session = Session::start(&app, platform);
	session.wait_for(FIRST_BUILD, |line| line.starts_with("watching "));
	let running = format!("[{platform}] running on ");
	let devices: Vec<String> = session
		.lines
		.iter()
		.filter_map(|line| line.strip_prefix(&running).map(str::to_owned))
		.collect();
	assert!(
		!devices.is_empty(),
		"no {platform} app started\n{}",
		session.transcript()
	);

	edit(&app, ORIGINAL, EDITED);
	session.wait_for(RELOAD, |line| {
		line.starts_with(&format!("[{platform}] guest rebuilt in "))
	});

	let reloaded = |line: &str| {
		line.starts_with(&format!("[{platform}]")) && line.contains("leptoskit dev: reloaded")
	};

	while session.lines.iter().filter(|line| reloaded(line)).count() < devices.len() {
		session.wait_for(RELOAD, reloaded);
	}

	verify(&session, &devices);
}

/// A new project depending on this repository's crates, under
/// `target/dev-reload`.
fn scaffold(platform: &str) -> PathBuf {
	let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
		.join("../..")
		.canonicalize()
		.unwrap();
	let stamp = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap()
		.as_millis();
	let directory = repository
		.join("target/dev-reload")
		.join(format!("{platform}-{stamp}"));
	std::fs::create_dir_all(&directory).unwrap();
	let created = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.current_dir(&directory)
		.args(["new", "reload", "--sdk-path"])
		.arg(&repository)
		.output()
		.unwrap();
	assert!(
		created.status.success(),
		"{}",
		String::from_utf8_lossy(&created.stderr)
	);
	println!("dev reload artifacts: {}", directory.display());
	directory.join("reload")
}

fn edit(app: &Path, from: &str, to: &str) {
	let screen = app.join("reload_app/src/routes/home.rs");
	let source = std::fs::read_to_string(&screen).unwrap();
	assert!(source.contains(from), "the scaffold's screen changed");
	std::fs::write(&screen, source.replace(from, to)).unwrap();
}

/// A running `leptoskit dev`, its output so far, and its log file.
struct Session {
	child: Child,
	receiver: mpsc::Receiver<String>,
	lines: Vec<String>,
	log: std::fs::File,
}

impl Session {
	fn start(app: &Path, platform: &str) -> Self {
		let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit"));
		command
			.current_dir(app)
			.args(["dev", "--platform", platform])
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped());
		#[cfg(unix)]
		std::os::unix::process::CommandExt::process_group(&mut command, 0);
		let mut child = command.spawn().unwrap();
		let (sender, receiver) = mpsc::channel();
		let streams: [Box<dyn Read + Send>; 2] = [
			Box::new(child.stdout.take().unwrap()),
			Box::new(child.stderr.take().unwrap()),
		];

		for stream in streams {
			let sender = sender.clone();
			std::thread::spawn(move || {
				for line in BufReader::new(stream).lines().map_while(Result::ok) {
					if sender.send(line).is_err() {
						return;
					}
				}
			});
		}

		let log = std::fs::File::create(app.join("../dev.log")).unwrap();
		Self {
			child,
			receiver,
			lines: Vec::new(),
			log,
		}
	}

	/// Records output until a line matches, failing with the transcript when
	/// `timeout` passes or `dev` exits first.
	fn wait_for(&mut self, timeout: Duration, matches: impl Fn(&str) -> bool) {
		let deadline = Instant::now() + timeout;

		loop {
			let remaining = deadline.saturating_duration_since(Instant::now());

			match self.receiver.recv_timeout(remaining) {
				Ok(line) => {
					let _ = writeln!(self.log, "{line}");
					let found = matches(&line);
					self.lines.push(line);

					if found {
						return;
					}
				}
				Err(mpsc::RecvTimeoutError::Timeout) => {
					panic!("timed out after {timeout:?}\n{}", self.transcript())
				}
				Err(mpsc::RecvTimeoutError::Disconnected) => {
					panic!(
						"`leptoskit dev` exited ({:?})\n{}",
						self.child.try_wait(),
						self.transcript()
					)
				}
			}
		}
	}

	/// The last lines of output, enough to see what went wrong.
	fn transcript(&self) -> String {
		let start = self.lines.len().saturating_sub(80);
		self.lines[start..].join("\n")
	}
}

impl Drop for Session {
	fn drop(&mut self) {
		// The whole group: `dev` and the apps and log streams it started.
		#[cfg(unix)]
		let _ = Command::new("kill")
			.args(["-TERM", &format!("-{}", self.child.id())])
			.status();
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

/// The view hierarchy on screen, from `uiautomator`.
fn android_screen(serial: &str) -> String {
	let adb = std::env::var_os("ANDROID_HOME").map_or_else(
		|| PathBuf::from("adb"),
		|sdk| PathBuf::from(sdk).join("platform-tools/adb"),
	);
	let dumped = Command::new(&adb)
		.args([
			"-s",
			serial,
			"shell",
			"uiautomator dump /data/local/tmp/leptoskit-reload.xml",
		])
		.output()
		.unwrap();
	assert!(
		dumped.status.success(),
		"{}",
		String::from_utf8_lossy(&dumped.stderr)
	);
	let screen = Command::new(&adb)
		.args([
			"-s",
			serial,
			"exec-out",
			"cat /data/local/tmp/leptoskit-reload.xml",
		])
		.output()
		.unwrap();
	String::from_utf8_lossy(&screen.stdout).into_owned()
}
