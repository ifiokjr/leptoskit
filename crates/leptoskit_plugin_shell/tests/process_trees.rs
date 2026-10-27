#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use leptoskit_plugin_shell::ArgumentPolicy;
use leptoskit_plugin_shell::CommandPolicy;
use leptoskit_plugin_shell::Error;
use leptoskit_plugin_shell::Finish;
use leptoskit_plugin_shell::Limits;
use leptoskit_plugin_shell::Request;
use leptoskit_plugin_shell::Shell;

const MODE: &str = "LEPTOSKIT_TREE_TEST_MODE";
const DIRECTORY: &str = "LEPTOSKIT_TREE_TEST_DIRECTORY";
const ARGUMENTS: [&str; 3] = ["--exact", "tree_helper", "--nocapture"];

#[test]
fn tree_helper() {
	let Ok(mode) = std::env::var(MODE) else {
		return;
	};
	let directory = std::env::var_os(DIRECTORY).unwrap();
	let directory = Path::new(&directory);

	if mode != "leaf" {
		let descendant = std::process::Command::new(std::env::current_exe().unwrap())
			.args(ARGUMENTS)
			.env(MODE, if mode == "branch" { "leaf" } else { "branch" })
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.unwrap();
		// Intentionally leave descendants running, including a grandchild.
		drop(descendant);
	}

	if mode == "branch" || mode == "leaf" {
		std::fs::write(directory.join(mode), std::process::id().to_string()).unwrap();
		std::thread::sleep(Duration::from_secs(120));
		return;
	}

	let deadline = std::time::Instant::now() + Duration::from_secs(10);
	while !["branch", "leaf"].iter().all(|name| {
		std::fs::read_to_string(directory.join(name))
			.ok()
			.and_then(|text| text.parse::<u32>().ok())
			.is_some()
	}) {
		assert!(std::time::Instant::now() < deadline);
		std::thread::sleep(Duration::from_millis(5));
	}

	#[cfg(unix)]
	if mode == "leave-group" {
		leave_group();
	}

	std::fs::write(directory.join("ready"), b"ready").unwrap();
	match mode.as_str() {
		"exit" => std::process::exit(7),
		"stdout" => std::io::stdout().write_all(&vec![b'x'; 64 * 1024]).unwrap(),
		"stderr" => std::io::stderr().write_all(&vec![b'x'; 64 * 1024]).unwrap(),
		"hold" => {}
		#[cfg(unix)]
		"leave-group" => {}
		_ => panic!("invalid tree fixture mode"),
	}

	std::thread::sleep(Duration::from_secs(120));
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn leave_group() {
	// SAFETY: these calls only query this fixture's parent and its group.
	let parent_group = unsafe { libc::getpgid(libc::getppid()) };
	assert!(parent_group > 0);
	// SAFETY: move only this fixture into its parent's existing group. Its
	// descendants remain in the supervisor's original group.
	let result = unsafe { libc::setpgid(0, parent_group) };
	assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
}

impl Drop for Fixture {
	fn drop(&mut self) {
		if self.cleaned.get() {
			return;
		}

		// Keep a failing regression run from leaking its deliberately orphaned
		// fixtures. These PIDs are never supplied by a guest or a shell
		// request.
		for name in ["branch", "leaf"] {
			if let Ok(text) = std::fs::read_to_string(self.directory.path().join(name))
				&& let Ok(pid) = text.parse::<u32>()
			{
				cleanup_process(pid);
			}
		}
	}
}

#[allow(unsafe_code)]
fn cleanup_process(pid: u32) {
	#[cfg(unix)]
	{
		// SAFETY: only a PID written by this test's descendant is signalled.
		unsafe { libc::kill(i32::try_from(pid).unwrap(), libc::SIGKILL) };
	}
	#[cfg(windows)]
	{
		use windows_sys::Win32::Foundation::CloseHandle;
		use windows_sys::Win32::System::Threading::OpenProcess;
		use windows_sys::Win32::System::Threading::PROCESS_TERMINATE;
		use windows_sys::Win32::System::Threading::TerminateProcess;

		// SAFETY: fixture PID only; every acquired handle is closed once.
		unsafe {
			let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
			if !handle.is_null() {
				TerminateProcess(handle, 1);
				CloseHandle(handle);
			}
		}
	}
}

struct Fixture {
	directory: tempfile::TempDir,
	shell: Shell,
	cleaned: Cell<bool>,
}

impl Fixture {
	fn new(mode: &str, duration: Duration) -> Self {
		let directory = tempfile::tempdir().unwrap();
		let shell = Shell::new(
			BTreeMap::from([(
				"fixture".into(),
				CommandPolicy {
					executable: std::env::current_exe().unwrap(),
					directory: directory.path().into(),
					arguments: ARGUMENTS
						.map(|value| ArgumentPolicy::Exact(value.into()))
						.into(),
					environment: BTreeMap::from([
						(MODE.into(), mode.into()),
						(DIRECTORY.into(), directory.path().to_str().unwrap().into()),
					]),
					limits: Limits {
						stdout_bytes: 1024,
						stderr_bytes: 1024,
						duration,
					},
				},
			)]),
			1,
		)
		.unwrap();

		Self {
			directory,
			shell,
			cleaned: Cell::new(false),
		}
	}

	fn request() -> Request {
		Request {
			command: "fixture".into(),
			arguments: ARGUMENTS.map(String::from).into(),
		}
	}

	async fn ready(&self) -> [u32; 2] {
		tokio::time::timeout(Duration::from_secs(15), async {
			while !self.directory.path().join("ready").exists() {
				tokio::time::sleep(Duration::from_millis(5)).await;
			}
		})
		.await
		.expect("tree never became ready");

		["branch", "leaf"].map(|name| {
			std::fs::read_to_string(self.directory.path().join(name))
				.unwrap()
				.parse()
				.unwrap()
		})
	}

	fn assert_finished(&self, pids: [u32; 2]) {
		for pid in pids {
			assert!(!process_alive(pid), "descendant {pid} outlived supervision");
		}

		self.cleaned.set(true);
	}

	async fn assert_capacity(&self) {
		let another = self.shell.spawn(&Self::request()).unwrap();
		another.cancel_and_wait().await.unwrap();
	}
}

#[allow(unsafe_code)]
fn process_alive(pid: u32) -> bool {
	#[cfg(unix)]
	{
		// SAFETY: signal zero only queries this fixture's known process ID.
		let result = unsafe { libc::kill(i32::try_from(pid).unwrap(), 0) };
		if result == 0 {
			return true;
		}

		assert_eq!(
			std::io::Error::last_os_error().raw_os_error(),
			Some(libc::ESRCH)
		);
		false
	}
	#[cfg(windows)]
	{
		use windows_sys::Win32::Foundation::CloseHandle;
		use windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER;
		use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
		use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
		use windows_sys::Win32::System::Threading::OpenProcess;
		use windows_sys::Win32::System::Threading::PROCESS_SYNCHRONIZE;
		use windows_sys::Win32::System::Threading::WaitForSingleObject;

		// SAFETY: the handle is opened for synchronization only and closed
		// once.
		let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
		if handle.is_null() {
			assert_eq!(
				std::io::Error::last_os_error().raw_os_error(),
				Some(ERROR_INVALID_PARAMETER as i32)
			);
			return false;
		}

		// SAFETY: handle remains valid until after the nonblocking wait.
		let status = unsafe { WaitForSingleObject(handle, 0) };
		unsafe { CloseHandle(handle) };
		assert!(status == WAIT_TIMEOUT || status == WAIT_OBJECT_0);
		status == WAIT_TIMEOUT
	}
}

#[tokio::test]
async fn normal_exit_timeout_and_output_limits_clean_descendants_before_returning_capacity() {
	for (mode, expected) in [
		("exit", Finish::Exited),
		("hold", Finish::TimedOut),
		("stdout", Finish::StdoutLimit),
		("stderr", Finish::StderrLimit),
	] {
		let fixture = Fixture::new(
			mode,
			Duration::from_secs(if mode == "hold" { 8 } else { 30 }),
		);
		let child = fixture.shell.spawn(&Fixture::request()).unwrap();
		let pids = fixture.ready().await;
		let output = tokio::time::timeout(Duration::from_secs(15), child.wait())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(output.finish, expected);
		if mode == "exit" {
			assert_eq!(output.code, Some(7));
		}

		fixture.assert_finished(pids);
		fixture.assert_capacity().await;
	}
}

#[cfg(feature = "async-host")]
#[tokio::test]
async fn shutdown_joins_descendants_of_live_and_dropped_invocations() {
	use std::sync::Arc;

	use leptoskit_bridge::async_host::AsyncPlugin;
	use leptoskit_bridge::async_host::CancellationToken;

	for abort in [false, true] {
		let fixture = Fixture::new("hold", Duration::from_secs(30));
		let options = serde_json::json!({
			"max_children": 1,
			"commands": { "fixture": {
				"executable": std::env::current_exe().unwrap(),
				"directory": fixture.directory.path(),
				"arguments": ARGUMENTS.map(|value| serde_json::json!({ "kind": "exact", "value": value })),
				"environment": { MODE: "hold", DIRECTORY: fixture.directory.path() }
			} }
		});
		let plugin = Arc::new(leptoskit_plugin_shell::from_config(&options.to_string()).unwrap());
		let request = Fixture::request().encode().unwrap();
		let task_plugin = Arc::clone(&plugin);
		let invocation = tokio::spawn(async move {
			task_plugin
				.invoke(
					leptoskit_plugin_shell::EXECUTE,
					request,
					CancellationToken::new(),
				)
				.await
		});
		let pids = fixture.ready().await;

		if abort {
			invocation.abort();
		}

		tokio::time::timeout(Duration::from_secs(15), plugin.shutdown())
			.await
			.unwrap();
		fixture.assert_finished(pids);
		if abort {
			assert!(invocation.await.unwrap_err().is_cancelled());
		} else {
			assert!(matches!(
				invocation.await.unwrap(),
				Err(leptoskit_bridge::Error::Cancelled)
			));
		}
	}
}

#[tokio::test]
async fn failed_spawn_returns_the_unused_capacity() {
	let fixture = Fixture::new("hold", Duration::from_secs(30));
	std::fs::remove_dir(fixture.directory.path()).unwrap();
	assert!(matches!(
		fixture.shell.spawn(&Fixture::request()),
		Err(Error::Io(_))
	));
	std::fs::create_dir(fixture.directory.path()).unwrap();
	fixture.assert_capacity().await;
}

#[test]
fn runtime_shutdown_kills_the_container_without_releasing_unjoined_capacity() {
	let fixture = Fixture::new("hold", Duration::from_secs(30));
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.unwrap();
	let (child, pids) = runtime.block_on(async {
		let child = fixture.shell.spawn(&Fixture::request()).unwrap();
		let pids = fixture.ready().await;
		(child, pids)
	});
	drop(runtime);
	drop(child);

	let deadline = std::time::Instant::now() + Duration::from_secs(15);
	while pids.into_iter().any(process_alive) {
		assert!(
			std::time::Instant::now() < deadline,
			"runtime shutdown leaked descendants"
		);
		std::thread::sleep(Duration::from_millis(5));
	}
	fixture.assert_finished(pids);

	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.unwrap();
	runtime.block_on(async {
		assert!(matches!(
			fixture.shell.spawn(&Fixture::request()),
			Err(Error::Busy)
		));
	});
}

#[cfg(unix)]
#[tokio::test]
async fn failed_cleanup_reports_an_error_and_forfeits_its_capacity() {
	let fixture = Fixture::new("leave-group", Duration::from_secs(30));
	let child = fixture.shell.spawn(&Fixture::request()).unwrap();
	let leader = child.id();
	let pids = fixture.ready().await;

	// The descendants are still in the original group, but the owned leader
	// cannot be joined until cleanup times out and its handle kills it.
	let error = tokio::time::timeout(Duration::from_secs(15), child.cancel_and_wait())
		.await
		.unwrap()
		.unwrap_err();
	assert!(matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::TimedOut));

	tokio::time::timeout(Duration::from_secs(15), async {
		while [leader, pids[0], pids[1]].into_iter().any(process_alive) {
			tokio::time::sleep(Duration::from_millis(5)).await;
		}
	})
	.await
	.expect("failed cleanup left a fixture running");
	fixture.assert_finished(pids);
	assert!(matches!(
		fixture.shell.spawn(&Fixture::request()),
		Err(Error::Busy)
	));
}

#[tokio::test]
async fn cancellation_drop_and_dropped_wait_clean_descendants_before_signalling_completion() {
	for mode in ["cancel", "drop", "abort"] {
		let fixture = Fixture::new("hold", Duration::from_secs(30));
		let child = fixture.shell.spawn(&Fixture::request()).unwrap();
		let pids = fixture.ready().await;
		let signal = child.cancellation();
		assert!(matches!(
			fixture.shell.spawn(&Fixture::request()),
			Err(Error::Busy)
		));

		match mode {
			"cancel" => {
				assert_eq!(
					child.cancel_and_wait().await.unwrap().finish,
					Finish::Cancelled
				);
			}
			"drop" => drop(child),
			"abort" => {
				let wait = tokio::spawn(child.wait());
				tokio::task::yield_now().await;
				wait.abort();
				assert!(wait.await.unwrap_err().is_cancelled());
			}
			_ => unreachable!(),
		}

		tokio::time::timeout(Duration::from_secs(15), signal.closed())
			.await
			.unwrap();
		fixture.assert_finished(pids);
		fixture.assert_capacity().await;
	}
}
