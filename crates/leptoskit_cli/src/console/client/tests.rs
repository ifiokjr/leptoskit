//! Behavioural tests for the console client: the origin rules, the durable
//! request journal, reconnect replay, plan/apply binding and deploy refusals.

use std::path::Path;

use super::*;
use crate::console::tests::loopback::Reply;
use crate::console::tests::loopback::Server;

/// A valid-looking scoped credential; the loopback server never validates it.
const KEY: &str = "0123456789abcdef0123456789abcdef";
/// A second, equally valid credential, for the explicit-key-wins test.
#[cfg(unix)]
const OTHER_KEY: &str = "fedcba9876543210fedcba9876543210";

fn connection(server: &Server, root: &Path) -> Connection {
	Connection {
		url: server.origin().to_owned(),
		key: KEY.into(),
		project: "demo".into(),
		root: root.to_path_buf(),
	}
}

fn overview(revision: u64) -> String {
	serde_json::json!({
		"format_version": 1,
		"projects": [{"id": "demo", "environment": "production", "revision": revision}],
		"operations": [],
	})
	.to_string()
}

fn operation(id: &str, phase: &str) -> String {
	serde_json::json!({
		"id": id,
		"project": "demo",
		"revision": 5,
		"intent": {
			"expected_revision": 4,
			"idempotency_key": "service-key",
			"action": {"kind": "status"},
		},
		"phase": phase,
		"created": 1,
		"updated": 1,
		"message": "",
		"result": null,
	})
	.to_string()
}

fn pending_path(root: &Path) -> PathBuf {
	root.join(".leptoskit/control-request.json")
}

fn save_pending(root: &Path, value: &serde_json::Value) {
	std::fs::create_dir_all(root.join(".leptoskit")).unwrap();
	std::fs::write(pending_path(root), value.to_string()).unwrap();
}

fn release_args() -> crate::infra::ReleaseArgs {
	crate::infra::ReleaseArgs {
		version: None,
		rollback: None,
		setup: false,
		maintenance: false,
		recover: false,
		rollback_schema: false,
		yes: false,
	}
}

#[test]
fn origin_requires_https_or_loopback_http() {
	assert_eq!(
		origin("https://console.example.com").unwrap(),
		"https://console.example.com"
	);
	assert_eq!(
		origin("https://console.example.com/").unwrap(),
		"https://console.example.com"
	);

	for value in [
		"http://localhost:4700",
		"http://127.0.0.1:4700",
		"http://[::1]:4700",
	] {
		assert_eq!(origin(value).unwrap(), value);
	}

	for value in [
		"http://console.example.com",
		"https://console.example.com/path",
		"https://user:password@console.example.com",
		"https://console.example.com?query=1",
		"https://console.example.com#fragment",
		"ftp://localhost",
	] {
		let error = origin(value).unwrap_err();
		assert_eq!(
			error.to_string(),
			"deployment service URL needs an HTTPS origin or localhost HTTP",
			"for {value}"
		);
	}
}

#[test]
fn request_reads_json_and_maps_rejections() {
	let server = Server::start(vec![
		Reply::json(overview(3)),
		Reply::json(""),
		Reply::json("not json"),
		Reply::status(409, r#"{"error":"revision_conflict"}"#),
		Reply::status(401, r#"{"error":"authentication_required"}"#),
		Reply::status(403, r#"{"error":"scope_or_origin_denied"}"#),
		Reply::status(400, "{}"),
	]);
	let root = tempfile::tempdir().unwrap();
	let connection = connection(&server, root.path());

	let value = connection.request("/api/v1/overview", None).unwrap();
	assert_eq!(value["projects"][0]["revision"], 3);
	assert_eq!(
		connection.request("/api/v1/overview", None).unwrap(),
		serde_json::Value::Null
	);
	assert!(
		connection
			.request("/api/v1/overview", None)
			.unwrap_err()
			.to_string()
			.contains("invalid deployment service response")
	);
	for expected in [
		"revision_conflict",
		"authentication_required",
		"scope_or_origin_denied",
		"request_rejected",
	] {
		let error = connection.request("/api/v1/overview", None).unwrap_err();
		assert_eq!(
			error.to_string(),
			format!("deployment service rejected the request: {expected}"),
			"for {expected}"
		);
	}

	let requests = server.requests();
	assert_eq!(requests.len(), 7);
	assert_eq!(requests[0].method, "GET");
	assert_eq!(requests[0].path, "/api/v1/overview");
	assert_eq!(
		requests[0].header("authorization"),
		Some("Bearer 0123456789abcdef0123456789abcdef")
	);
}

#[test]
fn request_never_follows_a_redirect() {
	let server = Server::start(vec![Reply::redirect("/elsewhere")]);
	let root = tempfile::tempdir().unwrap();
	let connection = connection(&server, root.path());

	let error = connection.request("/api/v1/overview", None).unwrap_err();

	assert_eq!(
		error.to_string(),
		"deployment service rejected the request: request_rejected"
	);
	let requests = server.requests();
	assert_eq!(requests.len(), 1);
	assert_eq!(requests[0].path, "/api/v1/overview");
}

#[test]
fn request_reports_an_unreachable_service_without_details() {
	let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	let address = listener.local_addr().unwrap();
	drop(listener);
	let root = tempfile::tempdir().unwrap();
	let connection = Connection {
		url: format!("http://{address}"),
		key: KEY.into(),
		project: "demo".into(),
		root: root.path().to_path_buf(),
	};

	let error = connection.request("/api/v1/overview", None).unwrap_err();

	assert_eq!(
		error.to_string(),
		"deployment service request failed; check the connection and scoped credential"
	);
}

#[test]
fn submit_persists_the_intent_and_removes_it_after_success() {
	let server = Server::start(vec![
		Reply::json(overview(4)),
		Reply::json(operation("op-1", "queued")),
		Reply::json(operation("op-1", "succeeded")),
	]);
	let root = tempfile::tempdir().unwrap();
	let connection = connection(&server, root.path());

	let operation = submit(&connection, Action::Status).unwrap();

	assert_eq!(operation.id, "op-1");
	assert_eq!(operation.phase, Phase::Succeeded);
	assert!(!pending_path(root.path()).exists());
	let requests = server.requests();
	assert_eq!(requests.len(), 3);
	assert_eq!(requests[1].path, "/api/v1/projects/demo/operations");
	let intent = requests[1].json();
	assert_eq!(intent["expected_revision"], 4);
	assert_eq!(intent["action"], serde_json::json!({"kind":"status"}));
	assert_eq!(intent["idempotency_key"].as_str().unwrap().len(), 36);
	assert_eq!(requests[2].path, "/api/v1/operations/op-1");
}

#[test]
fn submit_keeps_the_journal_when_the_outcome_is_unknown() {
	let server = Server::start(vec![
		Reply::json(overview(7)),
		Reply::json(operation("op-1", "queued")),
	]);
	let root = tempfile::tempdir().unwrap();
	let connection = connection(&server, root.path());

	let error = submit(&connection, Action::Plan).unwrap_err();

	assert!(
		error.to_string().contains("request failed"),
		"unexpected error: {error}"
	);
	let saved: serde_json::Value =
		serde_json::from_slice(&std::fs::read(pending_path(root.path())).unwrap()).unwrap();
	assert_eq!(saved["url"], server.origin());
	assert_eq!(saved["project"], "demo");
	assert_eq!(saved["intent"]["expected_revision"], 7);
	assert_eq!(
		saved["intent"]["action"],
		serde_json::json!({"kind":"plan"})
	);
}

#[test]
fn submit_replays_a_saved_intent_without_requesting_a_new_revision() {
	let server = Server::start(vec![Reply::json(operation("op-2", "succeeded"))]);
	let root = tempfile::tempdir().unwrap();
	let connection = connection(&server, root.path());
	save_pending(
		root.path(),
		&serde_json::json!({
			"url": server.origin(),
			"project": "demo",
			"intent": {
				"expected_revision": 9,
				"idempotency_key": "saved-key",
				"action": {"kind": "apply", "plan_id": "plan-1"},
			},
		}),
	);

	let operation = submit(
		&connection,
		Action::Apply {
			plan_id: "plan-1".into(),
		},
	)
	.unwrap();

	assert_eq!(operation.id, "op-2");
	let requests = server.requests();
	assert_eq!(requests.len(), 1, "a replay must not fetch a new revision");
	assert_eq!(requests[0].path, "/api/v1/projects/demo/operations");
	let body = requests[0].json();
	assert_eq!(body["idempotency_key"], "saved-key");
	assert_eq!(body["expected_revision"], 9);
	assert!(!pending_path(root.path()).exists());
}

#[test]
fn submit_refuses_a_different_action_while_a_request_is_pending() {
	let root = tempfile::tempdir().unwrap();
	save_pending(
		root.path(),
		&serde_json::json!({
			"url": "http://127.0.0.1:1",
			"project": "demo",
			"intent": {
				"expected_revision": 1,
				"idempotency_key": "saved-key",
				"action": {"kind": "status"},
			},
		}),
	);
	let connection = Connection {
		url: "http://127.0.0.1:1".into(),
		key: KEY.into(),
		project: "demo".into(),
		root: root.path().to_path_buf(),
	};

	let error = submit(&connection, Action::Plan).unwrap_err();

	assert!(
		error.to_string().contains("a previous request is pending"),
		"unexpected error: {error}"
	);
	assert!(pending_path(root.path()).exists());
}

#[test]
fn submit_removes_the_pending_request_when_the_service_rejects_it() {
	let server = Server::start(vec![
		Reply::json(overview(2)),
		Reply::status(409, r#"{"error":"revision_conflict"}"#),
	]);
	let root = tempfile::tempdir().unwrap();
	let connection = connection(&server, root.path());

	let error = submit(&connection, Action::Status).unwrap_err();

	assert_eq!(
		error.to_string(),
		"deployment service rejected the request: revision_conflict"
	);
	assert!(!pending_path(root.path()).exists());
}

#[test]
fn submit_removes_the_pending_request_after_a_failed_operation() {
	let server = Server::start(vec![
		Reply::json(overview(2)),
		Reply::json(operation("op-3", "failed")),
	]);
	let root = tempfile::tempdir().unwrap();
	let connection = connection(&server, root.path());

	let error = submit(&connection, Action::Status).unwrap_err();

	assert!(
		error.to_string().contains("operation did not succeed"),
		"unexpected error: {error}"
	);
	assert!(!pending_path(root.path()).exists());
}

#[test]
fn submit_requires_the_project_on_the_service() {
	let server = Server::start(vec![Reply::json(
		r#"{"format_version":1,"projects":[],"operations":[]}"#,
	)]);
	let root = tempfile::tempdir().unwrap();
	let connection = connection(&server, root.path());

	let error = submit(&connection, Action::Status).unwrap_err();

	assert!(
		error
			.to_string()
			.contains("project is unavailable on this service"),
		"unexpected error: {error}"
	);
	assert!(!pending_path(root.path()).exists());
	assert_eq!(server.requests().len(), 1);
}

#[cfg(unix)]
#[test]
fn private_write_creates_private_parents_and_files() {
	use std::os::unix::fs::PermissionsExt as _;

	let root = tempfile::tempdir().unwrap();
	let path = root.path().join("nested/control/plan.json");

	private_write(&path, b"first").unwrap();
	assert_eq!(std::fs::read(&path).unwrap(), b"first");
	assert_eq!(
		std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
		0o600
	);
	assert_eq!(
		std::fs::metadata(path.parent().unwrap())
			.unwrap()
			.permissions()
			.mode()
			& 0o777,
		0o700
	);

	private_write(&path, b"second").unwrap();
	assert_eq!(std::fs::read(&path).unwrap(), b"second");
}

#[test]
fn plan_and_apply_bind_the_saved_plan_to_the_service() {
	let server = Server::start(vec![
		Reply::json(overview(2)),
		Reply::json(operation("plan-1", "succeeded")),
		Reply::json(overview(3)),
		Reply::json(operation("apply-1", "succeeded")),
	]);
	let root = tempfile::tempdir().unwrap();
	let connection = connection(&server, root.path());

	plan(&connection).unwrap();
	let saved_path = root.path().join(".leptoskit/control-plan.json");
	let saved: serde_json::Value =
		serde_json::from_slice(&std::fs::read(&saved_path).unwrap()).unwrap();
	assert_eq!(
		saved,
		serde_json::json!({"url": server.origin(), "id": "plan-1"})
	);

	apply(&connection).unwrap();

	let requests = server.requests();
	assert_eq!(requests.len(), 4);
	assert_eq!(
		requests[1].json()["action"],
		serde_json::json!({"kind":"plan"})
	);
	assert_eq!(
		requests[3].json()["action"],
		serde_json::json!({"kind":"apply","plan_id":"plan-1"})
	);
	assert_eq!(requests[3].json()["expected_revision"], 3);
	assert!(saved_path.exists());
}

#[test]
fn apply_requires_a_reviewed_plan() {
	let root = tempfile::tempdir().unwrap();
	let connection = Connection {
		url: "http://127.0.0.1:1".into(),
		key: KEY.into(),
		project: "demo".into(),
		root: root.path().to_path_buf(),
	};

	let error = apply(&connection).unwrap_err();

	assert!(
		error
			.to_string()
			.contains("run lk infra plan and review it before applying"),
		"unexpected error: {error}"
	);
}

#[test]
fn apply_refuses_a_plan_saved_for_another_service() {
	let root = tempfile::tempdir().unwrap();
	std::fs::create_dir_all(root.path().join(".leptoskit")).unwrap();
	std::fs::write(
		root.path().join(".leptoskit/control-plan.json"),
		serde_json::json!({"url": "https://other.example.com", "id": "plan-1"}).to_string(),
	)
	.unwrap();
	let connection = Connection {
		url: "http://127.0.0.1:1".into(),
		key: KEY.into(),
		project: "demo".into(),
		root: root.path().to_path_buf(),
	};

	let error = apply(&connection).unwrap_err();

	assert!(
		error
			.to_string()
			.contains("saved plan belongs to another service"),
		"unexpected error: {error}"
	);
}

#[test]
fn apply_refuses_a_plan_without_an_identifier() {
	let root = tempfile::tempdir().unwrap();
	std::fs::create_dir_all(root.path().join(".leptoskit")).unwrap();
	let connection = Connection {
		url: "http://127.0.0.1:1".into(),
		key: KEY.into(),
		project: "demo".into(),
		root: root.path().to_path_buf(),
	};
	std::fs::write(
		root.path().join(".leptoskit/control-plan.json"),
		serde_json::json!({"url": connection.url, "id": 7}).to_string(),
	)
	.unwrap();

	let error = apply(&connection).unwrap_err();

	assert!(
		error.to_string().contains("invalid saved plan"),
		"unexpected error: {error}"
	);
}

#[test]
fn deploy_refuses_recovery_and_schema_rollback() {
	let root = tempfile::tempdir().unwrap();
	let connection = Connection {
		url: "http://127.0.0.1:1".into(),
		key: KEY.into(),
		project: "demo".into(),
		root: root.path().to_path_buf(),
	};

	let mut recover = release_args();
	recover.version = Some("v1".into());
	recover.recover = true;
	let mut rollback = release_args();
	rollback.rollback = Some("v1".into());
	rollback.rollback_schema = true;
	rollback.yes = true;

	for args in [recover, rollback] {
		let error = deploy(&connection, &args).unwrap_err();
		assert!(
			error
				.to_string()
				.contains("explicit local recovery commands"),
			"unexpected error: {error}"
		);
	}
}

#[test]
fn deploy_sends_a_rollback_action() {
	let server = Server::start(vec![
		Reply::json(overview(6)),
		Reply::json(operation("rollback-1", "succeeded")),
	]);
	let root = tempfile::tempdir().unwrap();
	let connection = connection(&server, root.path());
	let mut args = release_args();
	args.rollback = Some("v1".into());

	deploy(&connection, &args).unwrap();

	let requests = server.requests();
	assert_eq!(
		requests[1].json()["action"],
		serde_json::json!({"kind":"rollback","version":"v1"})
	);
	assert_eq!(requests[1].json()["expected_revision"], 6);
}

#[test]
fn deploy_sends_the_selected_version_and_setup_flags() {
	let server = Server::start(vec![
		Reply::json(overview(1)),
		Reply::json(operation("deploy-1", "succeeded")),
	]);
	let root = tempfile::tempdir().unwrap();
	let connection = connection(&server, root.path());
	let mut args = release_args();
	args.version = Some("v2".into());
	args.setup = true;
	args.maintenance = true;

	deploy(&connection, &args).unwrap();

	assert_eq!(
		server.requests()[1].json()["action"],
		serde_json::json!({
			"kind": "deploy",
			"version": "v2",
			"setup": true,
			"maintenance": true,
		})
	);
}

#[test]
fn deploy_requires_an_explicit_version_outside_git() {
	let root = tempfile::tempdir().unwrap();
	let connection = Connection {
		url: "http://127.0.0.1:1".into(),
		key: KEY.into(),
		project: "demo".into(),
		root: root.path().to_path_buf(),
	};

	let error = deploy(&connection, &release_args()).unwrap_err();

	assert_eq!(error.to_string(), "pass --version for an app outside Git");
}

/// An `[infra]` configuration for a linked or unlinked project.
#[cfg(unix)]
fn infra_config(name: &str, control_url: Option<&str>) -> String {
	let mut text = format!(
		"\
[infra]
name = '{name}'
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
	);
	if let Some(url) = control_url {
		use std::fmt::Write as _;
		let _ = writeln!(text, "control_url = '{url}'");
	}
	text
}

/// `for_project`, `connect`, `status` and `token` resolve the project from
/// the process-wide working directory and the credential from the
/// environment or the user's configuration home, so each discovery scenario
/// runs in an isolated child test process. The parent holds the loopback
/// server, prepares the project files, and reads back what the scenario
/// stored.
#[cfg(unix)]
mod child {
	use std::path::Path;
	use std::path::PathBuf;
	use std::process::Command;
	use std::process::Output;

	use super::*;

	/// The child test that runs one discovery scenario.
	const HELPER: &str = "console::client::tests::child::helper";
	/// A `PATH` with only system tools; the child runs no programs of its own.
	const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

	pub(super) struct Fixture {
		directory: tempfile::TempDir,
	}

	impl Fixture {
		pub(super) fn new() -> Self {
			Self {
				directory: tempfile::tempdir().unwrap(),
			}
		}

		/// A project directory whose `leptoskit.toml` is `config`.
		pub(super) fn project(&self, name: &str, config: &str) -> PathBuf {
			let root = self.directory.path().join(name);
			std::fs::create_dir(&root).unwrap();
			std::fs::write(root.join(leptoskit_config::CONFIG_FILE), config).unwrap();
			root
		}

		/// The XDG configuration home the child resolves credentials in;
		/// the parent reads it back afterwards.
		pub(super) fn config_home(&self, name: &str) -> PathBuf {
			let home = self.directory.path().join(name);
			std::fs::create_dir(&home).unwrap();
			home
		}

		fn run(
			&self,
			scenario: &str,
			project: &Path,
			config_home: &Path,
			environment: &[(&str, &str)],
		) -> Output {
			let mut command = Command::new(std::env::current_exe().unwrap());
			command
				.args(["--exact", HELPER, "--nocapture", "--test-threads=1"])
				.env_clear()
				.env("PATH", SYSTEM_PATH)
				.env("TMPDIR", self.directory.path())
				.env("LEPTOSKIT_TEST_CONSOLE_SCENARIO", scenario)
				.env("XDG_CONFIG_HOME", config_home)
				.current_dir(project);

			for (name, value) in environment {
				command.env(name, value);
			}

			// Windows networking needs the system root in the child's
			// environment; the parent's value is passed through without
			// reading anything else.
			if let Some(system_root) = std::env::var_os("SystemRoot") {
				command.env("SystemRoot", system_root);
			}

			if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
				command.env("LLVM_PROFILE_FILE", profile);
			}

			command.output().unwrap()
		}

		/// Runs `scenario`, asserting the child passed, and returns what it
		/// printed.
		pub(super) fn run_ok(
			&self,
			scenario: &str,
			project: &Path,
			config_home: &Path,
			environment: &[(&str, &str)],
		) -> String {
			let output = self.run(scenario, project, config_home, environment);
			assert!(
				output.status.success(),
				"{scenario} failed\nstdout:\n{}\nstderr:\n{}",
				String::from_utf8_lossy(&output.stdout),
				String::from_utf8_lossy(&output.stderr),
			);
			String::from_utf8(output.stdout).unwrap()
		}
	}

	/// One discovery scenario against the parent's scripted server.
	#[test]
	fn helper() {
		let Ok(scenario) = std::env::var("LEPTOSKIT_TEST_CONSOLE_SCENARIO") else {
			return;
		};
		let origin = || std::env::var("LEPTOSKIT_TEST_ORIGIN").unwrap();
		let refused = |result: Result<()>| {
			let error = result.expect_err("the command should have been refused");
			let expected = std::env::var("LEPTOSKIT_TEST_EXPECT").unwrap();
			assert!(
				error.to_string().contains(&expected),
				"expected {expected:?} in {error}"
			);
		};

		match scenario.as_str() {
			"for-project-none" => assert!(Connection::for_project().unwrap().is_none()),
			"connect" => connect(&origin()).unwrap(),
			"connect-refused" => refused(connect(&origin())),
			"status" => status().unwrap(),
			"status-refused" => refused(status()),
			"status-other-project" => {
				// The service issued a credential for `demo`; this project
				// is a different one, so it must not be found.
				let path = credential_path(&origin(), "demo").unwrap();
				private_write(
					&path,
					&serde_json::to_vec(&Credential {
						key: KEY.to_owned(),
					})
					.unwrap(),
				)
				.unwrap();
				refused(status());
			}
			#[cfg(unix)]
			"status-store-private" => {
				let path = credential_path(&origin(), "demo").unwrap();
				private_write(
					&path,
					&serde_json::to_vec(&Credential {
						key: std::env::var("LEPTOSKIT_TEST_CREDENTIAL_KEY").unwrap(),
					})
					.unwrap(),
				)
				.unwrap();
				{
					use std::os::unix::fs::PermissionsExt as _;
					let mode = u32::from_str_radix(
						&std::env::var("LEPTOSKIT_TEST_CREDENTIAL_MODE").unwrap(),
						8,
					)
					.unwrap();
					std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
				}
				refused(status());
			}
			"token" => token("demo", Scope::Deploy).unwrap(),
			"token-refused" => refused(token("demo", Scope::Deploy)),
			other => panic!("unknown scenario {other}"),
		}
	}
}

#[cfg(unix)]
#[test]
fn for_project_ignores_projects_without_a_connection() {
	let server = Server::start(Vec::new());
	let fixture = child::Fixture::new();
	let config = fixture.config_home("config");

	// An app without managed infrastructure has no connection to find.
	let bare = fixture.project("bare", "");
	fixture.run_ok("for-project-none", &bare, &config, &[]);

	// Neither has one with infrastructure that isn't linked to a service.
	let unlinked = fixture.project("unlinked", &infra_config("demo", None));
	fixture.run_ok("for-project-none", &unlinked, &config, &[]);

	// A local opt-out wins even when everything else is configured.
	let local = fixture.project("local", &infra_config("demo", Some(server.origin())));
	fixture.run_ok(
		"for-project-none",
		&local,
		&config,
		&[("LEPTOSKIT_CONTROL_LOCAL", "1")],
	);

	assert!(server.requests().is_empty());
}

#[cfg(unix)]
#[test]
fn connect_stores_a_private_credential_and_records_the_url() {
	let server = Server::start(vec![Reply::json(overview(0))]);
	let fixture = child::Fixture::new();
	let config = fixture.config_home("config");
	let project = fixture.project("demo", &infra_config("demo", Some(server.origin())));

	let stdout = fixture.run_ok(
		"connect",
		&project,
		&config,
		&[
			("LEPTOSKIT_TEST_ORIGIN", server.origin()),
			("LEPTOSKIT_CONTROL_KEY", KEY),
		],
	);
	assert!(stdout.contains("Connected to"), "{stdout}");

	// The credential is stored once, outside the repository, and private.
	let stored = config.join("leptoskit/control");
	let mut entries: Vec<_> = std::fs::read_dir(&stored)
		.unwrap()
		.filter_map(Result::ok)
		.map(|entry| entry.path())
		.collect();
	assert_eq!(entries.len(), 1);
	let path = entries.pop().unwrap();
	assert_eq!(
		path.extension().and_then(std::ffi::OsStr::to_str),
		Some("json")
	);
	{
		use std::os::unix::fs::PermissionsExt as _;
		assert_eq!(
			std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
			0o600
		);
	}
	let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
	assert_eq!(saved, serde_json::json!({ "key": KEY }));

	// The project configuration now names the service, and still parses.
	let text = std::fs::read_to_string(project.join(leptoskit_config::CONFIG_FILE)).unwrap();
	assert!(
		text.contains(&format!("control_url = \"{}\"", server.origin())),
		"{text}"
	);
	LeptoskitConfig::load_from(&project).unwrap();

	let requests = server.requests();
	assert_eq!(requests.len(), 1);
	assert_eq!(requests[0].path, "/api/v1/overview");
	let authorization = format!("Bearer {KEY}");
	assert_eq!(
		requests[0].header("authorization"),
		Some(authorization.as_str())
	);

	// A service that doesn't know the repository refuses the connection and
	// stores nothing.
	let unregistered = Server::start(vec![Reply::json(
		r#"{"format_version":1,"projects":[],"operations":[]}"#,
	)]);
	let refused = child::Fixture::new();
	let refused_config = refused.config_home("config");
	let refused_project = refused.project("demo", &infra_config("demo", None));
	refused.run_ok(
		"connect-refused",
		&refused_project,
		&refused_config,
		&[
			("LEPTOSKIT_TEST_ORIGIN", unregistered.origin()),
			("LEPTOSKIT_CONTROL_KEY", KEY),
			(
				"LEPTOSKIT_TEST_EXPECT",
				"this repository is not registered on the deployment service",
			),
		],
	);
	let text =
		std::fs::read_to_string(refused_project.join(leptoskit_config::CONFIG_FILE)).unwrap();
	assert!(!text.contains("control_url"), "{text}");
	assert!(!refused_config.join("leptoskit").exists());
	assert_eq!(unregistered.requests().len(), 1);
}

#[cfg(unix)]
#[test]
fn the_stored_connection_is_discovered_and_the_explicit_key_wins() {
	let server = Server::start(vec![
		Reply::json(overview(0)),
		Reply::json(overview(1)),
		Reply::json(overview(2)),
	]);
	let fixture = child::Fixture::new();
	let config = fixture.config_home("config");
	let project = fixture.project("demo", &infra_config("demo", Some(server.origin())));

	// A connect run stores the credential the discovery runs read back.
	fixture.run_ok(
		"connect",
		&project,
		&config,
		&[
			("LEPTOSKIT_TEST_ORIGIN", server.origin()),
			("LEPTOSKIT_CONTROL_KEY", KEY),
		],
	);

	// Discovery without the explicit key uses the stored credential.
	fixture.run_ok("status", &project, &config, &[]);
	// The explicit key wins over the stored one.
	fixture.run_ok(
		"status",
		&project,
		&config,
		&[("LEPTOSKIT_CONTROL_KEY", OTHER_KEY)],
	);

	let requests = server.requests();
	assert_eq!(requests.len(), 3);
	let stored_authorization = format!("Bearer {KEY}");
	let explicit_authorization = format!("Bearer {OTHER_KEY}");
	assert_eq!(
		requests[1].header("authorization"),
		Some(stored_authorization.as_str())
	);
	assert_eq!(
		requests[2].header("authorization"),
		Some(explicit_authorization.as_str())
	);

	// Nothing stored and nothing explicit: there is no connection. The
	// lookup fails before any request is made.
	let missing = child::Fixture::new();
	let missing_config = missing.config_home("config");
	let missing_project = missing.project("demo", &infra_config("demo", Some(server.origin())));
	missing.run_ok(
		"status-refused",
		&missing_project,
		&missing_config,
		&[
			("LEPTOSKIT_TEST_ORIGIN", server.origin()),
			(
				"LEPTOSKIT_TEST_EXPECT",
				"run lk console connect with LEPTOSKIT_CONTROL_KEY set",
			),
		],
	);
	assert_eq!(server.requests().len(), 3);
}

#[cfg(unix)]
#[test]
fn a_stored_credential_must_be_private_and_well_formed() {
	let server = Server::start(Vec::new());
	let fixture = child::Fixture::new();

	for (key, mode, expected) in [
		(KEY, "644", "deployment credential file must be private"),
		("too short", "600", "invalid deployment service credential"),
	] {
		let config = fixture.config_home(&format!("config-{mode}"));
		let project = fixture.project(
			&format!("project-{mode}"),
			&infra_config("demo", Some(server.origin())),
		);
		fixture.run_ok(
			"status-store-private",
			&project,
			&config,
			&[
				("LEPTOSKIT_TEST_ORIGIN", server.origin()),
				("LEPTOSKIT_TEST_CREDENTIAL_KEY", key),
				("LEPTOSKIT_TEST_CREDENTIAL_MODE", mode),
				("LEPTOSKIT_TEST_EXPECT", expected),
			],
		);
	}

	assert!(
		server.requests().is_empty(),
		"a rejected credential must not reach the service"
	);
}

#[cfg(unix)]
#[test]
fn the_stored_credential_follows_the_project() {
	let server = Server::start(Vec::new());
	let fixture = child::Fixture::new();
	let config = fixture.config_home("config");
	let project = fixture.project("other", &infra_config("other", Some(server.origin())));

	fixture.run_ok(
		"status-other-project",
		&project,
		&config,
		&[
			("LEPTOSKIT_TEST_ORIGIN", server.origin()),
			// The credential for `demo` is stored under a name derived from
			// `demo`, so the lookup for this project finds no file at all.
			(
				"LEPTOSKIT_TEST_EXPECT",
				"run lk console connect with LEPTOSKIT_CONTROL_KEY set",
			),
		],
	);

	assert!(
		server.requests().is_empty(),
		"a missing credential must not reach the service"
	);
}

#[cfg(unix)]
#[test]
fn token_requests_a_scoped_credential_and_prints_it_once() {
	let server = Server::start(vec![
		Reply::json(r#"{"key":"scoped-token-value"}"#),
		Reply::json(r#"{"unexpected":true}"#),
	]);
	let fixture = child::Fixture::new();
	let project = fixture.project("demo", &infra_config("demo", Some(server.origin())));

	let stdout = fixture.run_ok(
		"token",
		&project,
		&fixture.config_home("config"),
		&[("LEPTOSKIT_CONTROL_KEY", KEY)],
	);
	assert!(stdout.contains("scoped-token-value"), "{stdout}");

	fixture.run_ok(
		"token-refused",
		&project,
		&fixture.config_home("refused"),
		&[
			("LEPTOSKIT_CONTROL_KEY", KEY),
			("LEPTOSKIT_TEST_EXPECT", "service did not issue a key"),
		],
	);

	let requests = server.requests();
	assert_eq!(requests.len(), 2);
	assert_eq!(requests[0].method, "POST");
	assert_eq!(requests[0].path, "/api/v1/tokens");
	let authorization = format!("Bearer {KEY}");
	assert_eq!(
		requests[0].header("authorization"),
		Some(authorization.as_str())
	);
	assert_eq!(
		requests[0].json(),
		serde_json::json!({ "scope": "deploy", "project": "demo" })
	);
}

#[test]
fn analytics_reads_the_project_history() {
	let server = Server::start(vec![Reply::json(r#"{"series":[]}"#)]);
	let root = tempfile::tempdir().unwrap();
	let connection = connection(&server, root.path());

	analytics(&connection).unwrap();

	let requests = server.requests();
	assert_eq!(requests.len(), 1);
	assert_eq!(requests[0].method, "GET");
	assert_eq!(requests[0].path, "/api/v1/projects/demo/analytics");
	let authorization = format!("Bearer {KEY}");
	assert_eq!(
		requests[0].header("authorization"),
		Some(authorization.as_str())
	);
}

#[test]
fn submit_waits_through_a_running_operation_and_recovers() {
	let server = Server::start(vec![
		Reply::json(overview(4)),
		Reply::json(operation("op-1", "running")),
		Reply::json(operation("op-1", "succeeded")),
	]);
	let root = tempfile::tempdir().unwrap();
	let connection = connection(&server, root.path());

	let finished = submit(&connection, Action::Status).unwrap();

	assert_eq!(finished.phase, Phase::Succeeded);
	assert!(!pending_path(root.path()).exists());
	let requests = server.requests();
	assert_eq!(requests.len(), 3);
	assert_eq!(requests[1].path, "/api/v1/projects/demo/operations");
	assert_eq!(requests[2].method, "GET");
	assert_eq!(requests[2].path, "/api/v1/operations/op-1");
}
