//! Behavioural tests for the console client: the origin rules, the durable
//! request journal, reconnect replay, plan/apply binding and deploy refusals.

use std::path::Path;

use super::*;
use crate::console::tests::loopback::Reply;
use crate::console::tests::loopback::Server;

/// A valid-looking scoped credential; the loopback server never validates it.
const KEY: &str = "0123456789abcdef0123456789abcdef";

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
