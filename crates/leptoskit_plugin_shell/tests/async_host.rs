#![cfg(all(
	feature = "async-host",
	any(target_os = "macos", target_os = "linux", target_os = "windows")
))]

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use leptoskit_plugin_shell::EXECUTE;
use leptoskit_plugin_shell::Finish;
use leptoskit_plugin_shell::Output;
use leptoskit_plugin_shell::Request;
use leptoskit_plugin_shell::ShellPlugin;
use leptoskit_plugin_shell::from_config;
use serde_json::Value;
use serde_json::json;

#[test]
fn child_helper() {
	let Ok(mode) = std::env::var("LEPTOSKIT_ASYNC_SHELL_MODE") else {
		return;
	};
	assert!(std::env::var_os("PATH").is_none());
	let marker = std::env::var_os("LEPTOSKIT_ASYNC_SHELL_PID").unwrap();
	std::fs::write(marker, std::process::id().to_string()).unwrap();
	match mode.as_str() {
		"sleep" => std::thread::sleep(Duration::from_secs(120)),
		"binary" => {
			std::io::stdout().write_all(&[0, 255, 128]).unwrap();
			std::io::stderr().write_all(b"failure").unwrap();
			std::process::exit(7);
		}
		"large" => {
			std::io::stdout().write_all(&vec![b'x'; 64 * 1024]).unwrap();
		}
		_ => panic!("invalid fixture mode"),
	}
}

struct Fixture {
	directory: tempfile::TempDir,
	plugin: Arc<ShellPlugin>,
	options: Value,
}
impl Fixture {
	fn new(mode: &str, stdout: usize, timeout: u64) -> Self {
		let directory = tempfile::tempdir().unwrap();
		let options = json!({
			"max_children": 1,
			"commands": { "fixture": {
				"executable": std::env::current_exe().unwrap(), "directory": directory.path(),
				"arguments": [ { "kind":"exact", "value":"--exact" }, { "kind":"one-of", "values":["child_helper"] }, { "kind":"pattern", "expression":"--nocapture", "max_bytes":11 } ],
				"environment": { "LEPTOSKIT_ASYNC_SHELL_MODE": mode, "LEPTOSKIT_ASYNC_SHELL_PID": directory.path().join("pid") },
				"stdout_bytes": stdout, "stderr_bytes": 1024, "timeout_ms": timeout
			} }
		});
		let plugin = Arc::new(from_config(&options.to_string()).unwrap());
		Self {
			directory,
			plugin,
			options,
		}
	}

	fn request() -> Bytes {
		Request {
			command: "fixture".into(),
			arguments: vec![
				"--exact".into(),
				"child_helper".into(),
				"--nocapture".into(),
			],
		}
		.encode()
		.unwrap()
	}

	async fn pid(&self) -> u32 {
		tokio::time::timeout(Duration::from_secs(5), async {
			loop {
				if let Ok(text) = std::fs::read_to_string(self.directory.path().join("pid"))
					&& let Ok(pid) = text.parse()
				{
					return pid;
				}
				tokio::time::sleep(Duration::from_millis(1)).await;
			}
		})
		.await
		.expect("child did not write its readiness marker")
	}
}

#[allow(unsafe_code)]
fn assert_child_finished(pid: u32) {
	assert!(pid > 0);
	#[cfg(unix)]
	{
		let mut status = 0;
		// SAFETY: waitpid only queries this known child; status points to
		// writable storage. WNOHANG prevents blocking if supervision
		// failed to terminate it.
		let result = unsafe { libc::waitpid(pid as libc::pid_t, &raw mut status, libc::WNOHANG) };
		let error = std::io::Error::last_os_error();
		assert_eq!(result, -1, "supervisor left a running or unreaped child");
		assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
	}
}

#[tokio::test]
async fn typed_rpc_preserves_binary_nonzero_status_and_limits() {
	let fixture = Fixture::new("binary", 4096, 5000);
	let reply = fixture
		.plugin
		.invoke(EXECUTE, Fixture::request(), CancellationToken::new())
		.await
		.unwrap();
	let output = Output::decode(reply.bytes()).unwrap();
	assert_eq!(output.code, Some(7));
	assert!(!output.success);
	assert_eq!(output.finish, Finish::Exited);
	assert!(output.stdout.windows(3).any(|bytes| bytes == [0, 255, 128]));
	assert_eq!(output.stderr.as_ref(), b"failure");
	assert!(reply.bytes().len() <= fixture.plugin.reply_limit(EXECUTE).unwrap());
	assert_child_finished(fixture.pid().await);
	let fixture = Fixture::new("large", 128, 5000);
	let reply = fixture
		.plugin
		.invoke(EXECUTE, Fixture::request(), CancellationToken::new())
		.await
		.unwrap();
	let output = Output::decode(reply.bytes()).unwrap();
	assert_eq!(output.finish, Finish::StdoutLimit);
	assert_eq!(output.stdout.len(), 128);
	assert!(!output.success);
	let fixture = Fixture::new("sleep", 1024, 30);
	let reply = fixture
		.plugin
		.invoke(EXECUTE, Fixture::request(), CancellationToken::new())
		.await
		.unwrap();
	assert_eq!(
		Output::decode(reply.bytes()).unwrap().finish,
		Finish::TimedOut
	);
}

#[tokio::test]
async fn cancellation_reaps_before_reply_and_immediately_releases_capacity() {
	let fixture = Fixture::new("sleep", 1024, 30_000);
	for _ in 0..2 {
		let marker = fixture.directory.path().join("pid");
		if marker.exists() {
			std::fs::remove_file(marker).unwrap();
		}
		let token = CancellationToken::new();
		let child_token = token.clone();
		let plugin = Arc::clone(&fixture.plugin);
		let request = Fixture::request();
		let invocation =
			tokio::spawn(async move { plugin.invoke(EXECUTE, request, child_token).await });
		let pid = fixture.pid().await;
		assert!(matches!(
			fixture
				.plugin
				.invoke(EXECUTE, Fixture::request(), CancellationToken::new())
				.await,
			Err(Error::Busy)
		));
		token.cancel();
		assert!(matches!(
			tokio::time::timeout(Duration::from_secs(5), invocation)
				.await
				.unwrap()
				.unwrap(),
			Err(Error::Cancelled)
		));
		assert_child_finished(pid);
	}
}

#[tokio::test]
async fn dropped_invocation_remains_owned_until_shutdown_reaps_child() {
	let fixture = Fixture::new("sleep", 1024, 30_000);
	let plugin = Arc::clone(&fixture.plugin);
	let request = Fixture::request();
	let invocation = tokio::spawn(async move {
		plugin
			.invoke(EXECUTE, request, CancellationToken::new())
			.await
	});
	let pid = fixture.pid().await;
	invocation.abort();
	assert!(invocation.await.unwrap_err().is_cancelled());
	tokio::time::timeout(Duration::from_secs(5), fixture.plugin.shutdown())
		.await
		.unwrap();
	assert_child_finished(pid);
	assert!(matches!(
		fixture
			.plugin
			.invoke(EXECUTE, Fixture::request(), CancellationToken::new())
			.await,
		Err(Error::Cancelled)
	));
}

#[tokio::test]
async fn shutdown_cancels_live_invocation_and_waits_for_reaping() {
	let fixture = Fixture::new("sleep", 1024, 30_000);
	let plugin = Arc::clone(&fixture.plugin);
	let request = Fixture::request();
	let invocation = tokio::spawn(async move {
		plugin
			.invoke(EXECUTE, request, CancellationToken::new())
			.await
	});
	let pid = fixture.pid().await;
	tokio::time::timeout(Duration::from_secs(5), fixture.plugin.shutdown())
		.await
		.unwrap();
	assert_child_finished(pid);
	assert!(matches!(invocation.await.unwrap(), Err(Error::Cancelled)));
}

#[tokio::test]
async fn pre_cancelled_denied_and_malformed_calls_never_start_a_child() {
	let fixture = Fixture::new("sleep", 1024, 30_000);
	let token = CancellationToken::new();
	token.cancel();
	assert!(matches!(
		fixture
			.plugin
			.invoke(EXECUTE, Fixture::request(), token)
			.await,
		Err(Error::Cancelled)
	));
	assert!(matches!(
		fixture
			.plugin
			.invoke(MethodId(1), Fixture::request(), CancellationToken::new())
			.await,
		Err(Error::Unavailable)
	));
	assert!(matches!(
		fixture
			.plugin
			.invoke(
				EXECUTE,
				Bytes::from_static(&[1, 2]),
				CancellationToken::new()
			)
			.await,
		Err(Error::InvalidArgument)
	));
	let mut request = Request::decode(&Fixture::request()).unwrap();
	request.arguments[2] = "x--nocapture".into();
	assert!(matches!(
		fixture
			.plugin
			.invoke(EXECUTE, request.encode().unwrap(), CancellationToken::new())
			.await,
		Err(Error::PermissionDenied)
	));
	request.command = "unlisted".into();
	assert!(matches!(
		fixture
			.plugin
			.invoke(EXECUTE, request.encode().unwrap(), CancellationToken::new())
			.await,
		Err(Error::PermissionDenied)
	));
	assert!(!fixture.directory.path().join("pid").exists());
}

#[test]
fn factory_validates_strict_host_policy_without_a_runtime_or_process() {
	let fixture = Fixture::new("binary", 1024, 5000);
	for (path, invalid) in [
		("/max_children", json!(0)),
		("/max_children", json!(65)),
		("/commands/fixture/executable", json!("relative")),
		("/commands/fixture/directory", json!("relative")),
		("/commands/fixture/timeout_ms", json!(0)),
		("/commands/fixture/timeout_ms", json!(3_600_001)),
		("/commands/fixture/stdout_bytes", json!(4_194_304)),
		("/commands/fixture/arguments/1/values", json!([])),
		("/commands/fixture/arguments/2/expression", json!("[")),
		("/commands/fixture/arguments/2/max_bytes", json!(0)),
		(
			"/commands/fixture/environment",
			json!({ "BAD=KEY": "value" }),
		),
	] {
		let mut options = fixture.options.clone();
		*options.pointer_mut(path).unwrap() = invalid;
		assert!(
			from_config(&options.to_string()).is_err(),
			"accepted invalid option {path}"
		);
	}
	let mut unknown = fixture.options.clone();
	unknown["allow_all"] = json!(true);
	assert!(from_config(&unknown.to_string()).is_err());
	let mut unknown = fixture.options.clone();
	unknown["commands"]["fixture"]["arguments"][0]["extra"] = json!(true);
	assert!(from_config(&unknown.to_string()).is_err());
	let mut unknown = fixture.options.clone();
	unknown["commands"]["fixture"]["shell"] = json!(true);
	assert!(from_config(&unknown.to_string()).is_err());
	assert!(from_config("{}").is_err());
	assert!(from_config(r#"{"commands":{}}"#).is_err());
	assert!(matches!(
		from_config(&" ".repeat(64 * 1024 + 1)),
		Err(Error::TooLarge)
	));
	assert!(!fixture.directory.path().join("pid").exists());
}
