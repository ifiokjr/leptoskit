//! Behavioural tests for the console command runner.
//!
//! Each test drives the real [`CommandRunner`] with a temporary data
//! directory, a durable control store and service, and a harmless shell
//! script standing in for the deployment CLI. Assertions cover the child
//! process the runner starts, the private logs it keeps, and the result the
//! service records.

#[cfg(unix)]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use std::time::UNIX_EPOCH;

use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::Intent;
use leptoskit_deploy_console::control::Operation;
use leptoskit_deploy_console::control::Project;
#[cfg(unix)]
use leptoskit_deploy_console::control::Service;
use leptoskit_deploy_console::control::Store;
use tempfile::TempDir;

use super::*;

/// A valid owner credential; the service never reaches the network in these
/// tests.
#[cfg(unix)]
const OWNER_KEY: &str = "0123456789abcdef0123456789abcdef";

/// A commit-shaped version, so the runner takes its Git checkout path.
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

#[cfg(unix)]
const TRUNCATION_NOTICE: &[u8] = b"\n[output truncated at 4 MiB]\n";

/// A valid project configuration; analytics is optional so one test can see
/// the service's managed keys.
fn config(analytics: bool) -> String {
	let mut text = "\
[infra]
name = 'demo'
domain = 'app.example.com'
location = 'hel1'
server_type = 'cx23'
ssh_key_ids = [123]
ssh_cidrs = ['192.0.2.1/32']
account_id = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
zone_id = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
updates_domain = 'updates.example.com'
backup_bucket = 'demo-backups'
updates_bucket = 'demo-releases'
image = 'ghcr.io/team/demo'
registry = 'ghcr.io'
registry_username = 'team'
"
	.to_owned();

	if analytics {
		text.push_str("\n[infra.plugins.analytics]\n");
	}

	text
}

struct Fixture {
	data: TempDir,
	_root: TempDir,
	store: Store,
	#[cfg(unix)]
	service: Service,
	project: Project,
}

impl Fixture {
	fn new(analytics: bool) -> Self {
		let root = tempfile::tempdir().unwrap();
		std::fs::write(root.path().join("leptoskit.toml"), config(analytics)).unwrap();

		Self::with_root(root)
	}

	fn with_root(root: TempDir) -> Self {
		let data = tempfile::tempdir().unwrap();
		let store = Store::open(&data.path().join("control").join("console.sqlite3")).unwrap();
		#[cfg(unix)]
		let service = Service::new(store.clone(), OWNER_KEY, "http://localhost:4700").unwrap();
		let project = Project {
			id: "demo".into(),
			environment: "production".into(),
			root: root.path().to_path_buf(),
			revision: 0,
		};
		store.register(&project).unwrap();

		Self {
			data,
			_root: root,
			store,
			#[cfg(unix)]
			service,
			project,
		}
	}

	#[cfg(unix)]
	fn runner(&self, binary: &Path) -> CommandRunner {
		CommandRunner::new(
			self.data.path().to_path_buf(),
			binary.to_path_buf(),
			self.service.clone(),
		)
	}

	fn start(&self, action: Action) -> Operation {
		self.store
			.enqueue(
				&self.project.id,
				&Intent {
					expected_revision: 0,
					idempotency_key: "operation-1".into(),
					action,
				},
			)
			.unwrap();
		self.store.claim().unwrap().unwrap()
	}

	#[cfg(unix)]
	fn log(&self, operation: &Operation, stage: &str, extension: &str) -> PathBuf {
		self.data
			.path()
			.join("logs")
			.join(format!("{}.{stage}.{extension}", operation.id))
	}
}

/// An executable stand-in for the deployment CLI. It records every invocation
/// and the environment it inherited before running `body`.
#[cfg(unix)]
struct FakeCli {
	_directory: TempDir,
	path: PathBuf,
	calls: PathBuf,
	environment: PathBuf,
}

#[cfg(unix)]
impl FakeCli {
	fn new(body: &str) -> Self {
		let directory = tempfile::tempdir().unwrap();
		let calls = directory.path().join("calls.txt");
		let environment = directory.path().join("environment.txt");
		let path = directory.path().join("fake-leptoskit");
		let script = format!(
			"#!/bin/sh\nset -eu\n{{ printf '%s\\n' \"$@\"; printf '%s\\n' '--'; }} >> \
			 {calls}\nfor key in LEPTOSKIT_CONTROL_ORIGIN LEPTOSKIT_CONTROL_LOCAL \
			 LEPTOSKIT_ANALYTICS_READ_KEY LEPTOSKIT_ANALYTICS_WRITE_KEY TF_HTTP_ADDRESS \
			 TF_HTTP_LOCK_ADDRESS TF_HTTP_UNLOCK_ADDRESS TF_HTTP_USERNAME TF_HTTP_PASSWORD \
			 TF_HTTP_LOCK_METHOD TF_HTTP_UNLOCK_METHOD; do\nif value=$(printenv \"$key\"); then \
			 printf '%s=%s\\n' \"$key\" \"$value\"; fi\ndone > {environment}\n{body}",
			calls = quote(&calls),
			environment = quote(&environment),
		);
		std::fs::write(&path, script).unwrap();
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

		Self {
			_directory: directory,
			path,
			calls,
			environment,
		}
	}

	/// Every invocation's arguments, in the order the runner started them.
	fn invocations(&self) -> Vec<Vec<String>> {
		let Ok(text) = std::fs::read_to_string(&self.calls) else {
			return Vec::new();
		};

		text.split("--\n")
			.filter(|block| !block.is_empty())
			.map(|block| block.lines().map(str::to_owned).collect())
			.collect()
	}

	fn environment(&self) -> BTreeMap<String, String> {
		std::fs::read_to_string(&self.environment)
			.unwrap()
			.lines()
			.filter_map(|line| line.split_once('='))
			.map(|(key, value)| (key.to_owned(), value.to_owned()))
			.collect()
	}
}

#[cfg(unix)]
fn quote(path: &Path) -> String {
	let text = path.display().to_string();

	format!("'{}'", text.replace('\'', "'\\''"))
}

#[cfg(unix)]
#[tokio::test]
async fn status_operation_runs_the_child_and_records_private_logs() {
	let fixture = Fixture::new(false);
	let cli =
		FakeCli::new("printf 'hello from the child\\n'\nprintf 'problem from the child\\n' >&2\n");
	let operation = fixture.start(Action::Status);
	let result = fixture
		.runner(&cli.path)
		.run(fixture.project.clone(), operation.clone())
		.await;

	assert!(result.succeeded);
	assert_eq!(result.details["code"], "completed");
	assert!(
		result.message.starts_with("Completed."),
		"unexpected message: {}",
		result.message
	);
	assert_eq!(
		cli.invocations(),
		vec![vec!["infra".to_owned(), "status".to_owned()]]
	);
	let recorded = fixture.store.operation(&operation.id).unwrap().unwrap();
	assert_eq!(
		recorded.message,
		"Running the requested operation through the existing deployment engine"
	);
	let stdout = fixture.log(&operation, "run", "out");
	let stderr = fixture.log(&operation, "run", "err");
	assert_eq!(
		std::fs::read_to_string(&stdout).unwrap(),
		"hello from the child\n"
	);
	assert_eq!(
		std::fs::read_to_string(&stderr).unwrap(),
		"problem from the child\n"
	);

	for path in [&stdout, &stderr] {
		assert_eq!(
			std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
			0o600,
			"{} must be private",
			path.display()
		);
	}

	let environment = cli.environment();
	assert_eq!(
		environment
			.get("LEPTOSKIT_CONTROL_ORIGIN")
			.map(String::as_str),
		Some("http://localhost:4700")
	);
	assert_eq!(
		environment
			.get("LEPTOSKIT_CONTROL_LOCAL")
			.map(String::as_str),
		Some("1")
	);
}

#[cfg(unix)]
#[tokio::test]
async fn failed_child_is_reported_as_a_command_failure() {
	let fixture = Fixture::new(false);
	let cli = FakeCli::new("printf 'before failing\\n'\nprintf 'error detail\\n' >&2\nexit 7\n");
	let operation = fixture.start(Action::Backup);
	let result = fixture
		.runner(&cli.path)
		.run(fixture.project.clone(), operation.clone())
		.await;

	assert!(!result.succeeded);
	assert_eq!(result.details["code"], "command_failed");
	assert!(
		result.message.starts_with("Operation failed."),
		"unexpected message: {}",
		result.message
	);
	assert_eq!(
		std::fs::read_to_string(fixture.log(&operation, "run", "out")).unwrap(),
		"before failing\n"
	);
	assert_eq!(
		std::fs::read_to_string(fixture.log(&operation, "run", "err")).unwrap(),
		"error detail\n"
	);
}

#[cfg(unix)]
#[tokio::test]
async fn an_existing_log_is_never_overwritten() {
	let fixture = Fixture::new(false);
	let cli = FakeCli::new("printf 'first run\\n'\n");
	let operation = fixture.start(Action::Status);
	let runner = fixture.runner(&cli.path);
	let first = runner.run(fixture.project.clone(), operation.clone()).await;
	assert!(first.succeeded);
	let log = fixture.log(&operation, "run", "out");
	let before = std::fs::read(&log).unwrap();

	let second = runner.run(fixture.project.clone(), operation.clone()).await;

	assert!(!second.succeeded);
	assert_eq!(second.details["code"], "worker_failed");
	assert!(
		second
			.message
			.starts_with("Worker failed before completing the operation."),
		"unexpected message: {}",
		second.message
	);
	assert_eq!(std::fs::read(&log).unwrap(), before);
	assert_eq!(
		cli.invocations().len(),
		1,
		"an existing log must prevent a second execution"
	);
}

#[cfg(unix)]
#[tokio::test]
async fn oversized_output_is_truncated_after_draining_the_child() {
	let fixture = Fixture::new(false);
	let cli =
		FakeCli::new("dd if=/dev/zero bs=1024 count=5120 2>/dev/null\nprintf 'drained\\n' >&2\n");
	let operation = fixture.start(Action::Status);
	let result = tokio::time::timeout(
		Duration::from_secs(60),
		fixture
			.runner(&cli.path)
			.run(fixture.project.clone(), operation.clone()),
	)
	.await
	.expect("the runner stopped draining a child that wrote more than 4 MiB");

	assert!(result.succeeded);
	let log = std::fs::read(fixture.log(&operation, "run", "out")).unwrap();
	assert_eq!(log.len(), 4 * 1024 * 1024 + TRUNCATION_NOTICE.len());
	assert!(log[..4 * 1024 * 1024].iter().all(|byte| *byte == 0));
	assert!(log.ends_with(TRUNCATION_NOTICE));
	assert_eq!(
		std::fs::read_to_string(fixture.log(&operation, "run", "err")).unwrap(),
		"drained\n"
	);
}

#[cfg(unix)]
#[tokio::test]
async fn old_logs_are_pruned_and_unrelated_files_are_kept() {
	let fixture = Fixture::new(false);
	let logs = fixture.data.path().join("logs");
	std::fs::create_dir_all(&logs).unwrap();

	for index in 0..70u64 {
		let path = logs.join(format!("archive-{index:02}.out"));
		std::fs::write(&path, "old log").unwrap();
		let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
		file.set_modified(UNIX_EPOCH + Duration::from_secs(index + 1))
			.unwrap();
	}

	std::fs::write(logs.join("keep.txt"), "not a log").unwrap();
	let cli = FakeCli::new("exit 0\n");
	let operation = fixture.start(Action::Status);
	let result = fixture
		.runner(&cli.path)
		.run(fixture.project.clone(), operation.clone())
		.await;
	assert!(result.succeeded);

	for index in 0..6 {
		assert!(
			!logs.join(format!("archive-{index:02}.out")).exists(),
			"archive {index} should have been pruned"
		);
	}

	for index in 6..70 {
		assert!(
			logs.join(format!("archive-{index:02}.out")).exists(),
			"archive {index} should have been kept"
		);
	}

	assert!(logs.join("keep.txt").exists());
	assert!(fixture.log(&operation, "run", "out").exists());
	assert!(fixture.log(&operation, "run", "err").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn a_git_commit_is_built_before_the_deploy_runs() {
	let fixture = Fixture::new(false);
	let cli = FakeCli::new("exit 0\n");
	let operation = fixture.start(Action::Deploy {
		version: COMMIT.into(),
		setup: false,
		maintenance: false,
	});
	let environment = Environment {
		data: fixture.data.path(),
		binary: &cli.path,
		service: &fixture.service,
		bindings: None,
		backend: None,
	};
	let result = execute(&environment, &fixture.project, &operation)
		.await
		.unwrap();

	assert!(result.succeeded);
	let invocations = cli.invocations();
	assert_eq!(invocations.len(), 2, "expected a build and then a deploy");
	assert!(
		invocations[0].iter().any(|argument| argument == "build"),
		"the first invocation should build: {:?}",
		invocations[0]
	);
	assert!(
		invocations[1].iter().any(|argument| argument == "deploy"),
		"the second invocation should deploy: {:?}",
		invocations[1]
	);
	assert!(invocations[1].contains(&COMMIT.to_owned()));
	assert!(fixture.log(&operation, "build", "out").exists());
	assert!(fixture.log(&operation, "run", "out").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn a_failed_build_stops_before_the_deploy() {
	let fixture = Fixture::new(false);
	let cli =
		FakeCli::new("if [ \"$2\" = build ]; then printf 'build failed\\n' >&2; exit 9; fi\n");
	let operation = fixture.start(Action::Deploy {
		version: COMMIT.into(),
		setup: true,
		maintenance: true,
	});
	let environment = Environment {
		data: fixture.data.path(),
		binary: &cli.path,
		service: &fixture.service,
		bindings: None,
		backend: None,
	};
	let result = execute(&environment, &fixture.project, &operation)
		.await
		.unwrap();

	assert!(!result.succeeded);
	assert_eq!(result.details["code"], "command_failed");
	assert_eq!(cli.invocations().len(), 1);
	assert_eq!(
		std::fs::read_to_string(fixture.log(&operation, "build", "err")).unwrap(),
		"build failed\n"
	);
	assert!(!fixture.log(&operation, "run", "out").exists());
}

#[tokio::test]
async fn an_unavailable_commit_fails_before_a_workspace_is_created() {
	let root = tempfile::tempdir().unwrap();
	let status = crate::git::command(root.path())
		.args(["init", "--quiet"])
		.status()
		.unwrap();
	assert!(status.success(), "git init failed");
	let fixture = Fixture::with_root(root);
	let operation = fixture.start(Action::Deploy {
		version: COMMIT.into(),
		setup: false,
		maintenance: false,
	});
	let error = checkout(fixture.data.path(), &fixture.project, &operation)
		.await
		.unwrap_err();

	assert!(
		error.to_string().contains("unavailable"),
		"unexpected error: {error}"
	);
	assert!(
		!fixture
			.data
			.path()
			.join("workspaces")
			.join(&operation.id)
			.exists()
	);
}

#[test]
fn hook_repository_settings_do_not_escape_into_deployment_git_commands() {
	let root = tempfile::tempdir().unwrap();
	let outer = root.path().join("outer");
	std::fs::create_dir(&outer).unwrap();
	assert!(
		crate::git::command(&outer)
			.args(["init", "--quiet"])
			.status()
			.unwrap()
			.success()
	);
	// Linked worktrees use an administrative directory whose name is not
	// `.git`. An inherited GIT_DIR makes `git init` reinitialize that
	// directory as bare instead of initializing the intended repository.
	let admin = root.path().join("hook-admin");
	std::fs::rename(outer.join(".git"), &admin).unwrap();
	let output = std::process::Command::new(std::env::current_exe().unwrap())
		.args([
			"--exact",
			"console::runner::tests::an_unavailable_commit_fails_before_a_workspace_is_created",
			"console::client::tests::deploy_requires_an_explicit_version_outside_git",
			"--test-threads=1",
		])
		.env("GIT_DIR", &admin)
		.env("GIT_COMMON_DIR", &admin)
		.env("GIT_INDEX_FILE", admin.join("index"))
		.env_remove("GIT_WORK_TREE")
		.output()
		.unwrap();
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(
		output.status.success() && stdout.contains("2 passed;"),
		"{stdout}\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let config = crate::git::command(root.path())
		.arg("--git-dir")
		.arg(&admin)
		.args(["config", "--get", "core.bare"])
		.output()
		.unwrap();
	assert!(config.status.success());
	assert_eq!(
		config.stdout, b"false\n",
		"the hook's repository was reinitialized"
	);
}

#[cfg(unix)]
#[tokio::test]
async fn a_failed_checkout_returns_the_generic_worker_failure() {
	let root = tempfile::tempdir().unwrap();
	let status = crate::git::command(root.path())
		.args(["init", "--quiet"])
		.status()
		.unwrap();
	assert!(status.success(), "git init failed");
	let fixture = Fixture::with_root(root);
	let cli = FakeCli::new("exit 0\n");
	let operation = fixture.start(Action::Deploy {
		version: COMMIT.into(),
		setup: false,
		maintenance: false,
	});
	let result = fixture
		.runner(&cli.path)
		.run(fixture.project.clone(), operation.clone())
		.await;

	assert!(!result.succeeded);
	assert_eq!(result.details["code"], "worker_failed");
	assert_eq!(cli.invocations(), Vec::<Vec<String>>::new());
	assert!(
		!fixture
			.data
			.path()
			.join("workspaces")
			.join(&operation.id)
			.exists()
	);
}

#[tokio::test]
async fn a_named_version_uses_the_registered_checkout() {
	let fixture = Fixture::new(false);
	let operation = fixture.start(Action::Deploy {
		version: "release-001".into(),
		setup: false,
		maintenance: false,
	});
	let isolated = checkout(fixture.data.path(), &fixture.project, &operation)
		.await
		.unwrap();

	assert!(isolated.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn missing_configuration_prevents_the_child_from_running() {
	let fixture = Fixture::with_root(tempfile::tempdir().unwrap());
	let cli = FakeCli::new("exit 0\n");
	let operation = fixture.start(Action::Status);
	let result = fixture
		.runner(&cli.path)
		.run(fixture.project.clone(), operation.clone())
		.await;

	assert!(!result.succeeded);
	assert_eq!(result.details["code"], "worker_failed");
	assert_eq!(cli.invocations(), Vec::<Vec<String>>::new());
}

#[cfg(unix)]
#[tokio::test]
async fn hosted_bindings_and_state_reach_the_child() {
	let fixture = Fixture::new(true);
	let cli = FakeCli::new("exit 0\n");
	let operation = fixture.start(Action::Status);
	let mut runner = fixture.runner(&cli.path);
	runner.hosted_bindings = Some(("read-key".into(), "write-key".into()));
	runner.hosted_state = Some(("http://127.0.0.1:9/state".into(), "state-password".into()));
	let result = runner.run(fixture.project.clone(), operation.clone()).await;

	assert!(result.succeeded);
	let environment = cli.environment();

	for (key, value) in [
		("LEPTOSKIT_ANALYTICS_READ_KEY", "read-key"),
		("LEPTOSKIT_ANALYTICS_WRITE_KEY", "write-key"),
		("TF_HTTP_ADDRESS", "http://127.0.0.1:9/state"),
		("TF_HTTP_LOCK_ADDRESS", "http://127.0.0.1:9/state"),
		("TF_HTTP_UNLOCK_ADDRESS", "http://127.0.0.1:9/state"),
		("TF_HTTP_USERNAME", "worker"),
		("TF_HTTP_PASSWORD", "state-password"),
		("TF_HTTP_LOCK_METHOD", "LOCK"),
		("TF_HTTP_UNLOCK_METHOD", "UNLOCK"),
	] {
		assert_eq!(
			environment.get(key).map(String::as_str),
			Some(value),
			"missing {key}"
		);
	}
}

#[cfg(unix)]
#[tokio::test]
async fn the_connected_service_supplies_distinct_analytics_keys() {
	let fixture = Fixture::new(true);
	let cli = FakeCli::new("exit 0\n");
	let operation = fixture.start(Action::Status);
	let result = fixture
		.runner(&cli.path)
		.run(fixture.project.clone(), operation.clone())
		.await;
	assert!(result.succeeded);

	let environment = cli.environment();
	let read = environment.get("LEPTOSKIT_ANALYTICS_READ_KEY").unwrap();
	let write = environment.get("LEPTOSKIT_ANALYTICS_WRITE_KEY").unwrap();
	assert_eq!(read.len(), 64);
	assert_eq!(write.len(), 64);
	assert_ne!(read, write);
	assert!(read.bytes().all(|byte| byte.is_ascii_hexdigit()));
	assert!(write.bytes().all(|byte| byte.is_ascii_hexdigit()));
}
