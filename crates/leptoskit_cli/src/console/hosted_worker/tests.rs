//! Behavioural tests for the dedicated hosted worker: pairing credentials,
//! managed-capacity policy, pending-work replay and bounded job validation.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

#[cfg(unix)]
use tempfile::TempDir;

use super::*;
use crate::console::tests::loopback::Reply;
use crate::console::tests::loopback::Server;
use crate::console::tests::private_tempdir;

fn config(hosted_state: bool) -> String {
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
	if hosted_state {
		text.push_str("hosted_state = true\n");
	}
	text
}

fn infra(compute: &str, budget: bool) -> leptoskit_config::infra::InfraConfig {
	let mut text = format!(
		"\
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
compute = '{compute}'
"
	);
	if budget {
		text.push_str("\n[infra.budget]\ncurrency = 'USD'\nmonthly_compute = 20.0\n");
	}
	let config = LeptoskitConfig::from_toml(&text, Path::new("fixture")).unwrap();
	config.infra.unwrap()
}

#[cfg(unix)]
fn repository(hosted_state: bool) -> TempDir {
	let root = tempfile::tempdir().unwrap();
	std::fs::write(root.path().join("leptoskit.toml"), config(hosted_state)).unwrap();
	root
}

#[cfg(unix)]
fn credential(data: &Path, value: &Value, mode: u32) {
	let path = data.join("worker.json");
	std::fs::write(&path, value.to_string()).unwrap();
	std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn request_posts_json_and_reports_rejections() {
	let server = Server::start(vec![
		Reply::json(r#"{"key":"paired-key"}"#),
		Reply::json(""),
		Reply::status(401, "{}"),
		Reply::redirect("/elsewhere"),
		Reply::json("not json"),
	]);

	let value = request(
		server.origin(),
		"/api/v1/worker/pair",
		None,
		&serde_json::json!({"code":"pairing-code"}),
	)
	.unwrap();
	assert_eq!(value["key"], "paired-key");
	let first = &server.requests()[0];
	assert_eq!(first.method, "POST");
	assert_eq!(first.path, "/api/v1/worker/pair");
	assert_eq!(first.header("authorization"), None);
	assert_eq!(first.json(), serde_json::json!({"code":"pairing-code"}));

	assert_eq!(
		request(
			server.origin(),
			"/api/v1/worker/claim",
			Some("key"),
			&serde_json::json!({})
		)
		.unwrap(),
		Value::Null
	);
	assert_eq!(
		server.requests()[1].header("authorization"),
		Some("Bearer key")
	);

	for _ in 0..2 {
		let error = request(
			server.origin(),
			"/api/v1/worker/claim",
			Some("key"),
			&serde_json::json!({}),
		)
		.unwrap_err();
		assert_eq!(
			error.to_string(),
			"hosted worker request rejected; check account, pairing and subscription"
		);
	}
	assert!(
		request(
			server.origin(),
			"/api/v1/worker/claim",
			Some("key"),
			&serde_json::json!({}),
		)
		.is_err(),
		"invalid JSON must fail"
	);
	assert_eq!(
		server.requests().len(),
		5,
		"a redirect must be seen as a rejection, not followed"
	);
}

#[test]
fn request_rejects_a_body_over_the_one_mebibyte_limit() {
	let server = Server::start(vec![Reply::json("x".repeat(1024 * 1024 + 1))]);

	let error = request(
		server.origin(),
		"/api/v1/worker/claim",
		Some("key"),
		&serde_json::json!({}),
	)
	.unwrap_err();

	assert!(
		error.to_string().contains("larger than request limit"),
		"unexpected error: {error}"
	);
}

#[test]
fn request_reports_an_unreachable_hosted_app() {
	let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	let address = listener.local_addr().unwrap();
	drop(listener);

	let error = request(
		&format!("http://{address}"),
		"/api/v1/worker/claim",
		Some("key"),
		&serde_json::json!({}),
	)
	.unwrap_err();

	assert_eq!(error.to_string(), "hosted connection failed");
}

#[test]
fn managed_capacity_requires_digitalocean_and_a_reviewed_budget() {
	let customer = infra("hetzner", false);
	check_managed_policy(&customer, &serde_json::json!("customer")).unwrap();

	let error =
		check_managed_policy(&customer, &serde_json::json!("leptoskit_digitalocean")).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("requires DigitalOcean and an explicit reviewed compute budget"),
		"unexpected error: {error}"
	);

	let unbudgeted = infra("digitalocean", false);
	check_managed_policy(&unbudgeted, &serde_json::json!("leptoskit_digitalocean")).unwrap_err();

	let managed = infra("digitalocean", true);
	check_managed_policy(&managed, &serde_json::json!("leptoskit_digitalocean")).unwrap();
}

#[test]
fn run_validates_the_origin_repository_and_configuration() {
	let data = private_tempdir();

	let error = run(
		"http://console.example.com",
		Path::new("missing"),
		data.path(),
	)
	.unwrap_err();
	assert!(
		error
			.to_string()
			.contains("deployment service URL needs an HTTPS origin"),
		"unexpected error: {error}"
	);

	let error = run(
		"http://localhost:4700",
		&data.path().join("missing-repository"),
		data.path(),
	)
	.unwrap_err();
	assert!(
		error.downcast_ref::<std::io::Error>().is_some(),
		"unexpected error: {error}"
	);

	let bare = tempfile::tempdir().unwrap();
	std::fs::write(bare.path().join("leptoskit.toml"), "").unwrap();
	let error = run("http://localhost:4700", bare.path(), data.path()).unwrap_err();
	assert!(
		error.to_string().contains("worker needs [infra]"),
		"unexpected error: {error}"
	);

	let unhosted = {
		let root = tempfile::tempdir().unwrap();
		std::fs::write(root.path().join("leptoskit.toml"), config(false)).unwrap();
		root
	};
	let error = run("http://localhost:4700", unhosted.path(), data.path()).unwrap_err();
	assert!(
		error.to_string().contains("set infra.hosted_state=true"),
		"unexpected error: {error}"
	);
}

#[cfg(unix)]
#[test]
fn saved_credentials_are_private_and_bound_to_this_installation() {
	let root = repository(true);
	let data = private_tempdir();
	let server = Server::start(Vec::new());

	credential(
		data.path(),
		&serde_json::json!({
			"url": server.origin(),
			"project": "demo",
			"key": "installation-key",
			"ownership": "customer",
		}),
		0o644,
	);
	let error = run(server.origin(), root.path(), data.path()).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("worker credential must be private"),
		"unexpected error: {error}"
	);

	credential(
		data.path(),
		&serde_json::json!({
			"url": server.origin(),
			"project": "other",
			"key": "installation-key",
			"ownership": "customer",
		}),
		0o600,
	);
	let error = run(server.origin(), root.path(), data.path()).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("worker credential is bound to another installation"),
		"unexpected error: {error}"
	);

	credential(
		data.path(),
		&serde_json::json!({
			"url": "http://localhost:1",
			"project": "demo",
			"key": "installation-key",
			"ownership": "customer",
		}),
		0o600,
	);
	let error = run(server.origin(), root.path(), data.path()).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("worker credential is bound to another installation"),
		"unexpected error: {error}"
	);

	credential(
		data.path(),
		&serde_json::json!({
			"url": server.origin(),
			"project": "demo",
			"ownership": "customer",
		}),
		0o600,
	);
	let error = run(server.origin(), root.path(), data.path()).unwrap_err();
	assert!(
		error.to_string().contains("invalid worker credential"),
		"unexpected error: {error}"
	);

	credential(
		data.path(),
		&serde_json::json!({
			"url": server.origin(),
			"project": "demo",
			"key": "installation-key",
			"ownership": "leptoskit_digitalocean",
		}),
		0o600,
	);
	let error = run(server.origin(), root.path(), data.path()).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("requires DigitalOcean and an explicit reviewed compute budget"),
		"unexpected error: {error}"
	);

	assert!(server.requests().is_empty());
}

#[cfg(unix)]
#[test]
fn a_claimed_value_must_be_a_valid_job_for_this_project() {
	let root = repository(true);
	let data = private_tempdir();
	let server = Server::start(vec![Reply::json(r#"{"unexpected":true}"#)]);
	credential(
		data.path(),
		&serde_json::json!({
			"url": server.origin(),
			"project": "demo",
			"key": "installation-key",
			"ownership": "customer",
		}),
		0o600,
	);

	let error = run(server.origin(), root.path(), data.path()).unwrap_err();

	assert!(
		error.to_string().contains("missing field"),
		"unexpected error: {error}"
	);
	let requests = server.requests();
	assert_eq!(requests.len(), 1);
	assert_eq!(requests[0].method, "POST");
	assert_eq!(requests[0].path, "/api/v1/worker/claim");
	assert_eq!(
		requests[0].header("authorization"),
		Some("Bearer installation-key")
	);
	assert_eq!(requests[0].json(), serde_json::json!({}));

	let store = Store::open(&data.path().join("local.sqlite3")).unwrap();
	let projects = store.projects().unwrap();
	assert_eq!(projects.len(), 1);
	assert_eq!(projects[0].id, "demo");
	assert_eq!(
		std::fs::metadata(data.path().join("worker.json"))
			.unwrap()
			.permissions()
			.mode()
			& 0o777,
		0o600
	);
	assert!(data.path().join("service.lock").exists());
	assert!(!data.path().join("pending.json").exists());
}

#[cfg(unix)]
#[test]
fn pending_work_is_replayed_before_a_new_claim() {
	let root = repository(true);
	let data = private_tempdir();
	let server = Server::start(vec![
		Reply::json("{}"),
		Reply::json(r#"{"unexpected":true}"#),
	]);
	credential(
		data.path(),
		&serde_json::json!({
			"url": server.origin(),
			"project": "demo",
			"key": "installation-key",
			"ownership": "customer",
		}),
		0o600,
	);
	let result = serde_json::json!({
		"id": "op-1",
		"succeeded": true,
		"message": "done",
		"details": {"code": "completed"},
	});
	let pending = data.path().join("pending.json");
	std::fs::write(
		&pending,
		serde_json::json!({"operation": "op-1", "result": result}).to_string(),
	)
	.unwrap();

	let error = run(server.origin(), root.path(), data.path()).unwrap_err();

	assert!(
		error.to_string().contains("missing field"),
		"unexpected error: {error}"
	);
	let requests = server.requests();
	assert_eq!(requests.len(), 2);
	assert_eq!(requests[0].path, "/api/v1/worker/finish");
	assert_eq!(requests[0].json(), result);
	assert_eq!(
		requests[0].header("authorization"),
		Some("Bearer installation-key")
	);
	assert_eq!(requests[1].path, "/api/v1/worker/claim");
	assert!(
		!pending.exists(),
		"an acknowledged result must leave the pending journal"
	);
}

#[cfg(unix)]
#[test]
fn uncertain_pending_work_is_never_replayed_as_a_result() {
	let root = repository(true);
	let data = private_tempdir();
	let server = Server::start(Vec::new());
	credential(
		data.path(),
		&serde_json::json!({
			"url": server.origin(),
			"project": "demo",
			"key": "installation-key",
			"ownership": "customer",
		}),
		0o600,
	);
	let pending = data.path().join("pending.json");
	std::fs::write(
		&pending,
		serde_json::json!({"operation": "op-1"}).to_string(),
	)
	.unwrap();

	let error = run(server.origin(), root.path(), data.path()).unwrap_err();

	assert!(
		error
			.to_string()
			.contains("reconcile its pending operation"),
		"unexpected error: {error}"
	);
	assert!(pending.exists());
	assert!(
		server.requests().is_empty(),
		"an uncertain operation must not contact the hosted app"
	);
}

#[cfg(unix)]
#[test]
fn a_claimed_operation_for_another_project_is_refused() {
	let root = repository(true);
	let data = private_tempdir();
	let job = serde_json::json!({
		"operation": {
			"id": "op-1",
			"project": "other",
			"revision": 1,
			"intent": {
				"expected_revision": 0,
				"idempotency_key": "job-key",
				"action": {"kind": "status"},
			},
			"phase": "running",
			"created": 1,
			"updated": 1,
			"message": "",
			"result": null,
		},
		"analytics_read_key": "read-key",
		"analytics_ingest_key": "ingest-key",
	});
	let server = Server::start(vec![Reply::json(job.to_string())]);
	credential(
		data.path(),
		&serde_json::json!({
			"url": server.origin(),
			"project": "demo",
			"key": "installation-key",
			"ownership": "customer",
		}),
		0o600,
	);

	let error = run(server.origin(), root.path(), data.path()).unwrap_err();

	assert!(
		error
			.to_string()
			.contains("hosted operation addressed a different project"),
		"unexpected error: {error}"
	);
	assert!(!data.path().join("pending.json").exists());
	assert_eq!(server.requests().len(), 1);
}

/// `run()` reads `LEPTOSKIT_PAIRING_CODE` from the process-wide environment,
/// so each pairing scenario executes in an isolated child test process. The
/// parent holds the loopback server and reads back the files the scenario
/// wrote.
#[cfg(unix)]
mod child {
	use std::path::Path;
	use std::path::PathBuf;
	use std::process::Command;
	use std::process::Output;

	use super::*;

	/// The child test that pairs one worker.
	const HELPER: &str = "console::hosted_worker::tests::child::helper";

	pub(super) struct Fixture {
		directory: TempDir,
		root: PathBuf,
		data: PathBuf,
	}

	impl Fixture {
		pub(super) fn new() -> Self {
			let directory = tempfile::tempdir().unwrap();
			let root = directory.path().join("repository");
			std::fs::create_dir(&root).unwrap();
			std::fs::write(root.join("leptoskit.toml"), config(true)).unwrap();
			let data = directory.path().join("data");
			Self {
				directory,
				root,
				data,
			}
		}

		pub(super) fn credential(&self) -> PathBuf {
			self.data.join("worker.json")
		}

		/// Runs `run()` in the child against the parent's server, expecting
		/// an error that contains `expected`.
		fn run(&self, expected: &str, origin: &str, pairing_code: Option<&str>) -> Output {
			let mut command = Command::new(std::env::current_exe().unwrap());
			command
				.args(["--exact", HELPER, "--nocapture", "--test-threads=1"])
				.env_clear()
				.env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
				.env("TMPDIR", self.directory.path())
				.env("LEPTOSKIT_TEST_WORKER_EXPECT", expected)
				.env("LEPTOSKIT_TEST_WORKER_ORIGIN", origin)
				.env("LEPTOSKIT_TEST_WORKER_ROOT", &self.root)
				.env("LEPTOSKIT_TEST_WORKER_DATA", &self.data)
				.current_dir(self.directory.path());

			if let Some(code) = pairing_code {
				command.env("LEPTOSKIT_PAIRING_CODE", code);
			}

			if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
				command.env("LLVM_PROFILE_FILE", profile);
			}

			command.output().unwrap()
		}

		/// Runs `run()`, asserting the child passed, and returns what it
		/// printed.
		pub(super) fn run_refused(
			&self,
			expected: &str,
			origin: &str,
			pairing_code: Option<&str>,
		) -> String {
			let output = self.run(expected, origin, pairing_code);
			assert!(
				output.status.success(),
				"pairing failed\nstdout:\n{}\nstderr:\n{}",
				String::from_utf8_lossy(&output.stdout),
				String::from_utf8_lossy(&output.stderr),
			);
			String::from_utf8(output.stdout).unwrap()
		}
	}

	/// One pairing scenario; see [`Fixture`].
	#[test]
	fn helper() {
		let Ok(expected) = std::env::var("LEPTOSKIT_TEST_WORKER_EXPECT") else {
			return;
		};
		let origin = std::env::var("LEPTOSKIT_TEST_WORKER_ORIGIN").unwrap();
		let root = std::env::var("LEPTOSKIT_TEST_WORKER_ROOT").unwrap();
		let data = std::env::var("LEPTOSKIT_TEST_WORKER_DATA").unwrap();

		let error = run(&origin, Path::new(&root), Path::new(&data)).unwrap_err();
		assert!(
			error.to_string().contains(&expected),
			"expected {expected:?} in {error}"
		);
	}
}

#[cfg(unix)]
#[test]
fn pairing_stores_a_private_credential_bound_to_this_installation() {
	let server = Server::start(vec![
		Reply::json(
			r#"{"installation":{"project":"demo","ownership":"customer"},"key":"paired-key"}"#,
		),
		Reply::json(r#"{"unexpected":true}"#),
	]);
	let fixture = child::Fixture::new();

	// The pairing completes, the claim afterwards carries the paired key,
	// and an unusable reply stops the worker.
	let stdout = fixture.run_refused("missing field", server.origin(), Some("pair-code-123"));
	assert!(
		stdout.contains("Dedicated worker connected for demo"),
		"{stdout}"
	);

	// The paired credential names this installation and stays private.
	let saved: Value =
		serde_json::from_slice(&std::fs::read(fixture.credential()).unwrap()).unwrap();
	assert_eq!(
		saved,
		serde_json::json!({
			"url": server.origin(),
			"project": "demo",
			"key": "paired-key",
			"ownership": "customer",
		})
	);
	assert_eq!(
		std::fs::metadata(fixture.credential())
			.unwrap()
			.permissions()
			.mode()
			& 0o777,
		0o600
	);

	let requests = server.requests();
	assert_eq!(requests.len(), 2);
	assert_eq!(requests[0].path, "/api/v1/worker/pair");
	assert_eq!(requests[0].header("authorization"), None);
	assert_eq!(
		requests[0].json(),
		serde_json::json!({ "code": "pair-code-123" })
	);
	assert_eq!(requests[1].path, "/api/v1/worker/claim");
	assert_eq!(
		requests[1].header("authorization"),
		Some("Bearer paired-key")
	);
}

#[cfg(unix)]
#[test]
fn pairing_refusals_store_nothing_and_never_claim() {
	for (installation, expected) in [
		(
			r#"{"installation":{"project":"other","ownership":"customer"},"key":"paired-key"}"#,
			"paired project differs from registered repository",
		),
		(
			r#"{"installation":{"project":"demo","ownership":"customer"}}"#,
			"invalid worker key",
		),
		(
			r#"{"installation":{"project":"demo","ownership":"leptoskit_digitalocean"},"key":"paired-key"}"#,
			"requires DigitalOcean and an explicit reviewed compute budget",
		),
	] {
		let server = Server::start(vec![Reply::json(installation)]);
		let fixture = child::Fixture::new();
		fixture.run_refused(expected, server.origin(), Some("pair-code-123"));
		assert!(
			!fixture.credential().exists(),
			"{expected}: nothing may be stored"
		);
		assert_eq!(
			server.requests().len(),
			1,
			"{expected}: only the pairing may be sent"
		);
	}

	// Without a pairing grant nothing is contacted at all.
	let server = Server::start(Vec::new());
	let fixture = child::Fixture::new();
	fixture.run_refused("set LEPTOSKIT_PAIRING_CODE", server.origin(), None);
	assert!(!fixture.credential().exists());
	assert!(server.requests().is_empty());
}
