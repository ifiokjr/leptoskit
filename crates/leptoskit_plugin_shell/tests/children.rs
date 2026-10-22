#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;

use leptoskit_plugin_shell::ArgumentPolicy;
use leptoskit_plugin_shell::CommandPolicy;
use leptoskit_plugin_shell::Error;
use leptoskit_plugin_shell::Finish;
use leptoskit_plugin_shell::Limits;
use leptoskit_plugin_shell::Request;
use leptoskit_plugin_shell::Shell;

#[test]
fn child_helper() {
	let Ok(mode) = std::env::var("LEPTOSKIT_SCOPED_CHILD_TEST_MODE") else {
		return;
	};
	match mode.as_str() {
		"success" => {
			std::io::stdout().write_all(b"completed").unwrap();
		}
		"output" => {
			assert!(std::env::var_os("PATH").is_none());
			std::io::stdout().write_all(&[b'O', 0xff, 0]).unwrap();
			std::io::stderr().write_all(b"ERR").unwrap();
			std::process::exit(7);
		}
		"sleep" => std::thread::sleep(Duration::from_secs(120)),
		"inherited-pipes" => {
			// The descendant owns inherited pipe handles after its parent
			// exits. It terminates itself; the shell only supervises the
			// parent.
			let _descendant = std::process::Command::new(std::env::current_exe().unwrap())
				.args(["--exact", "child_helper", "--nocapture"])
				.env("LEPTOSKIT_SCOPED_CHILD_TEST_MODE", "pipe-holder")
				.spawn()
				.unwrap();
			std::process::exit(0);
		}
		"pipe-holder" => {
			std::thread::sleep(Duration::from_secs(2));
			std::process::exit(0);
		}
		"stdout" => {
			std::io::stdout()
				.write_all(&vec![b'x'; 256 * 1024])
				.unwrap();
		}
		"stderr" => {
			std::io::stderr()
				.write_all(&vec![b'x'; 256 * 1024])
				.unwrap();
		}
		_ => panic!("unknown fixture mode"),
	}
}

fn scoped(mode: &str, limits: Limits) -> Shell {
	Shell::new(
		BTreeMap::from([(
			"fixture".into(),
			CommandPolicy {
				executable: std::env::current_exe().unwrap(),
				directory: std::env::current_dir().unwrap(),
				arguments: vec![
					ArgumentPolicy::Exact("--exact".into()),
					ArgumentPolicy::Exact("child_helper".into()),
					ArgumentPolicy::Exact("--nocapture".into()),
				],
				environment: BTreeMap::from([(
					"LEPTOSKIT_SCOPED_CHILD_TEST_MODE".into(),
					mode.into(),
				)]),
				limits,
			},
		)]),
		1,
	)
	.unwrap()
}
fn request() -> Request {
	Request {
		command: "fixture".into(),
		arguments: vec![
			"--exact".into(),
			"child_helper".into(),
			"--nocapture".into(),
		],
	}
}

#[tokio::test]
async fn nonzero_exit_and_binary_output_are_not_fabricated_success() {
	let completed = scoped("success", Limits::default())
		.spawn(&request())
		.unwrap()
		.wait()
		.await
		.unwrap();
	assert!(completed.success);
	assert_eq!(completed.code, Some(0));
	assert_eq!(completed.finish, Finish::Exited);
	assert!(completed.stdout.windows(9).any(|part| part == b"completed"));
	let shell = scoped("output", Limits::default());
	let output = shell.spawn(&request()).unwrap().wait().await.unwrap();
	assert_eq!(output.code, Some(7));
	assert!(!output.success);
	assert_eq!(output.finish, Finish::Exited);
	assert!(output.stdout.windows(3).any(|part| part == [b'O', 0xff, 0]));
	assert_eq!(&output.stderr[..], b"ERR");
	assert_eq!(
		leptoskit_plugin_shell::Output::decode(&output.encode().unwrap()).unwrap(),
		output
	);
}

#[test]
fn spawning_without_a_runtime_returns_an_error_before_launching() {
	let shell = scoped("success", Limits::default());
	assert!(matches!(shell.spawn(&request()), Err(Error::NoRuntime)));
}
#[tokio::test]
async fn cancellation_reaps_and_releases_capacity() {
	let shell = scoped("sleep", Limits::default());
	let child = shell.spawn(&request()).unwrap();
	assert!(matches!(shell.spawn(&request()), Err(Error::Busy)));
	let output = child.cancel_and_wait().await.unwrap();
	assert_eq!(output.finish, Finish::Cancelled);
	assert!(!output.success);
	let another = shell.spawn(&request()).unwrap();
	assert_eq!(
		another.cancel_and_wait().await.unwrap().finish,
		Finish::Cancelled
	);
}
#[tokio::test]
async fn dropping_owner_cancels_even_without_waiting() {
	let shell = scoped("sleep", Limits::default());
	let child = shell.spawn(&request()).unwrap();
	let cancellation = child.cancellation();
	drop(child);
	tokio::time::timeout(Duration::from_secs(5), cancellation.closed())
		.await
		.unwrap();
	let another = shell.spawn(&request()).unwrap();
	another.cancel_and_wait().await.unwrap();
}
#[tokio::test]
async fn aborting_wait_future_still_cancels_owned_child() {
	let shell = scoped("sleep", Limits::default());
	let child = shell.spawn(&request()).unwrap();
	let cancellation = child.cancellation();
	let waiter = tokio::spawn(child.wait());
	tokio::task::yield_now().await;
	waiter.abort();
	assert!(waiter.await.unwrap_err().is_cancelled());
	tokio::time::timeout(Duration::from_secs(5), cancellation.closed())
		.await
		.unwrap();
	shell
		.spawn(&request())
		.unwrap()
		.cancel_and_wait()
		.await
		.unwrap();
}
#[tokio::test]
async fn timeout_and_output_limits_terminate_and_bound_capture() {
	let output = scoped(
		"sleep",
		Limits {
			duration: Duration::from_millis(25),
			..Limits::default()
		},
	)
	.spawn(&request())
	.unwrap()
	.wait()
	.await
	.unwrap();
	assert_eq!(output.finish, Finish::TimedOut);
	for (mode, expected) in [
		("stdout", Finish::StdoutLimit),
		("stderr", Finish::StderrLimit),
	] {
		let output = scoped(
			mode,
			Limits {
				stdout_bytes: 128,
				stderr_bytes: 128,
				..Limits::default()
			},
		)
		.spawn(&request())
		.unwrap()
		.wait()
		.await
		.unwrap();
		assert_eq!(output.finish, expected);
		assert!(output.stdout.len() <= 128);
		assert!(output.stderr.len() <= 128);
		assert!(!output.success);
	}
}
#[tokio::test]
async fn deadline_includes_pipe_draining_after_the_direct_child_exits() {
	let output = tokio::time::timeout(
		Duration::from_secs(5),
		scoped(
			"inherited-pipes",
			Limits {
				duration: Duration::from_secs(1),
				..Limits::default()
			},
		)
		.spawn(&request())
		.unwrap()
		.wait(),
	)
	.await
	.expect("inherited output pipes must not hang the supervisor")
	.unwrap();
	assert_eq!(output.code, Some(0), "the direct child exited normally");
	assert_eq!(output.finish, Finish::TimedOut);
	assert!(!output.success, "incomplete capture cannot report success");
}
#[tokio::test]
async fn scope_rejects_unknown_commands_extra_arguments_and_argument_changes() {
	let shell = scoped("output", Limits::default());
	let mut denied = request();
	denied.command = "arbitrary".into();
	assert!(matches!(shell.spawn(&denied), Err(Error::Denied)));
	let mut denied = request();
	denied.arguments.push("extra".into());
	assert!(matches!(shell.spawn(&denied), Err(Error::Denied)));
	let mut denied = request();
	denied.arguments[1] = "different_test".into();
	assert!(matches!(shell.spawn(&denied), Err(Error::Denied)));
}
