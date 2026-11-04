#![cfg(feature = "server")]

use axum::body::Body;
use axum::body::to_bytes;
use axum::http::Request;
use axum::http::StatusCode;
use leptoskit::analytics::*;
use leptoskit_deploy_console::control::*;
use tower::ServiceExt as _;

const OWNER: &str = "test-only-management-owner-key-0123456789";
fn setup() -> (tempfile::TempDir, Store) {
	let root = tempfile::tempdir().unwrap();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
	}
	let store = Store::open(&root.path().join("console.sqlite3")).unwrap();
	store
		.register(&Project {
			id: "notes".into(),
			environment: "production".into(),
			root: root.path().canonicalize().unwrap(),
			revision: 0,
		})
		.unwrap();
	(root, store)
}
fn intent(revision: u64, key: &str, action: Action) -> Intent {
	Intent {
		expected_revision: revision,
		idempotency_key: key.into(),
		action,
	}
}
fn clock() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_secs()
}
fn snapshot(instance: &str, count: usize) -> Snapshot {
	let analytics = Analytics::new(
		Resource {
			service: "notes".into(),
			environment: "production".into(),
			release: "v1".into(),
			instance: instance.into(),
		},
		Limits::default(),
		clock,
	)
	.unwrap();
	for _ in 0..count {
		analytics.record(
			SeriesKey {
				source: Source::Backend,
				signal: Signal::Http,
				operation: "/notes/{id}".into(),
				outcome: Outcome::Ok,
			},
			10.0,
			None,
		);
	}
	analytics.snapshot()
}

#[test]
fn queue_is_durable_idempotent_and_preserves_uncertain_runs() {
	let (root, store) = setup();
	let request = intent(0, "request-1", Action::Status);
	let first = store.enqueue("notes", &request).unwrap();
	assert_eq!(store.enqueue("notes", &request).unwrap().id, first.id);
	assert!(
		store
			.enqueue("notes", &intent(0, "request-1", Action::Plan))
			.is_err()
	);
	assert!(
		store
			.enqueue("notes", &intent(0, "request-2", Action::Status))
			.is_err()
	);
	assert_eq!(store.claim().unwrap().unwrap().id, first.id);
	let other = Store::open(&root.path().join("console.sqlite3")).unwrap();
	assert!(other.claim().unwrap().is_none());
	other.interrupt_running().unwrap();
	assert_eq!(
		store.operation(&first.id).unwrap().unwrap().phase,
		Phase::Interrupted
	);
	assert!(store.claim().unwrap().is_none());
	assert_eq!(store.projects().unwrap()[0].revision, 1);
	let next = other
		.enqueue("notes", &intent(1, "request-2", Action::Status))
		.unwrap();
	assert_eq!(next.phase, Phase::Queued);
}

#[test]
fn apply_is_bound_to_the_reviewed_plan_and_cannot_reuse_it() {
	let (_root, store) = setup();
	let plan = store
		.enqueue("notes", &intent(0, "plan-1", Action::Plan))
		.unwrap();
	store.claim().unwrap();
	store
		.finish(
			&plan.id,
			&RunResult {
				succeeded: true,
				message: "Reviewed plan".into(),
				details: serde_json::json!({"changes":[]}),
			},
		)
		.unwrap();
	let apply = store
		.enqueue(
			"notes",
			&intent(
				1,
				"apply-1",
				Action::Apply {
					plan_id: plan.id.clone(),
				},
			),
		)
		.unwrap();
	store.claim().unwrap();
	store
		.finish(
			&apply.id,
			&RunResult {
				succeeded: true,
				message: "Applied".into(),
				details: serde_json::Value::Null,
			},
		)
		.unwrap();
	assert!(
		store
			.enqueue(
				"notes",
				&intent(
					2,
					"apply-2",
					Action::Apply {
						plan_id: plan.id.clone()
					}
				)
			)
			.is_err()
	);
	let next = store
		.enqueue("notes", &intent(2, "plan-2", Action::Plan))
		.unwrap();
	store.claim().unwrap();
	store
		.finish(
			&next.id,
			&RunResult {
				succeeded: true,
				message: "New plan".into(),
				details: serde_json::Value::Null,
			},
		)
		.unwrap();
	assert!(
		store
			.enqueue(
				"notes",
				&intent(3, "apply-3", Action::Apply { plan_id: plan.id })
			)
			.is_err()
	);
}

#[test]
fn fleet_history_survives_restarts_and_repeated_snapshots_do_not_double_count() {
	let (root, store) = setup();
	let first = snapshot("one", 3);
	store.ingest("notes", &first).unwrap();
	store.ingest("notes", &first).unwrap();
	store.ingest("notes", &snapshot("two", 2)).unwrap();
	let from = clock().saturating_sub(3600);
	let to = clock() + 60;
	let fleet = store.fleet("notes", from, to).unwrap();
	assert_eq!(fleet.instances, 2);
	assert_eq!(fleet.series[0].windows[0].count, 5);
	let mut updated = first.clone();
	updated.series[0].windows[0].count = 4;
	updated.series[0].windows[0].latency[1] = 4;
	updated.series[0].windows[0].duration_sum_ms = 40.0;
	store.ingest("notes", &updated).unwrap();
	store.ingest("notes", &first).unwrap(); // Out-of-order old update cannot regress counts.
	drop(store);
	let reopened = Store::open(&root.path().join("console.sqlite3")).unwrap();
	assert_eq!(
		reopened.fleet("notes", from, to).unwrap().series[0].windows[0].count,
		6
	);
	let mut invalid = updated;
	invalid.resource.environment = "another".into();
	assert!(reopened.ingest("notes", &invalid).is_err());
	assert!(reopened.fleet("notes", 0, clock()).is_err());
}

#[test]
fn paired_down_migration_takes_a_consistent_backup() {
	let (root, store) = setup();
	store
		.enqueue("notes", &intent(0, "before-backup", Action::Status))
		.unwrap();
	let backup = root.path().join("before-down.sqlite3");
	store.rollback(&backup).unwrap();
	assert_eq!(Store::open(&backup).unwrap().operations().unwrap().len(), 1);
	assert!(
		Store::open(&root.path().join("console.sqlite3"))
			.unwrap()
			.projects()
			.unwrap()
			.is_empty()
	);
}

#[tokio::test]
async fn restoring_preserves_previous_history_and_never_revives_scoped_credentials() {
	let (root, store) = setup();
	let key = store
		.issue(Scope::Read, Some("notes"), 3600, false)
		.unwrap();
	let first = store
		.enqueue("notes", &intent(0, "before-restore", Action::Status))
		.unwrap();
	let backup = root.path().join("known-good.sqlite3");
	store.backup(&backup).unwrap();
	store.claim().unwrap();
	store
		.finish(
			&first.id,
			&RunResult {
				succeeded: true,
				message: "done".into(),
				details: serde_json::Value::Null,
			},
		)
		.unwrap();
	store
		.enqueue("notes", &intent(1, "after-backup", Action::Status))
		.unwrap();
	drop(store);
	let database = root.path().join("console.sqlite3");
	let previous = root.path().join("previous.sqlite3");
	Store::restore(&database, &backup, &previous).unwrap();
	let restored = Store::open(&database).unwrap();
	assert_eq!(restored.operations().unwrap().len(), 1);
	assert_eq!(
		Store::open(&previous).unwrap().operations().unwrap().len(),
		2
	);
	let service = Service::new(restored, OWNER, "https://deploy.example.com").unwrap();
	assert_eq!(
		service
			.router()
			.oneshot(request("GET", "/api/v1/overview", Some(&key), None))
			.await
			.unwrap()
			.status(),
		StatusCode::UNAUTHORIZED
	);
}

fn request(
	method: &str,
	path: &str,
	key: Option<&str>,
	body: Option<serde_json::Value>,
) -> Request<Body> {
	let mut request = Request::builder()
		.method(method)
		.uri(path)
		.header("content-type", "application/json");
	if let Some(key) = key {
		request = request.header("authorization", format!("Bearer {key}"));
	}
	request
		.body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
		.unwrap()
}

#[tokio::test]
async fn scoped_api_and_browser_sessions_enforce_the_same_project_permissions() {
	let (_root, store) = setup();
	let service = Service::new(store.clone(), OWNER, "https://deploy.example.com").unwrap();
	let app = service.router();
	let read = store
		.issue(Scope::Read, Some("notes"), 3600, false)
		.unwrap();
	let ingest = store
		.issue(Scope::Ingest, Some("notes"), 3600, false)
		.unwrap();
	assert_eq!(
		app.clone()
			.oneshot(request("GET", "/api/v1/overview", None, None))
			.await
			.unwrap()
			.status(),
		StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		app.clone()
			.oneshot(request("GET", "/api/v1/overview", Some(&read), None))
			.await
			.unwrap()
			.status(),
		StatusCode::OK
	);
	let body = serde_json::to_value(intent(0, "denied", Action::Status)).unwrap();
	for key in [&read, &ingest] {
		assert_eq!(
			app.clone()
				.oneshot(request(
					"POST",
					"/api/v1/projects/notes/operations",
					Some(key),
					Some(body.clone())
				))
				.await
				.unwrap()
				.status(),
			StatusCode::FORBIDDEN
		);
	}
	assert_eq!(
		app.clone()
			.oneshot(request(
				"POST",
				"/api/v1/projects/notes/analytics/ingest",
				Some(&ingest),
				Some(serde_json::to_value(snapshot("one", 1)).unwrap())
			))
			.await
			.unwrap()
			.status(),
		StatusCode::NO_CONTENT
	);
	assert_eq!(
		app.clone()
			.oneshot(request(
				"POST",
				"/api/v1/login",
				None,
				Some(serde_json::json!({"key":OWNER}))
			))
			.await
			.unwrap()
			.status(),
		StatusCode::FORBIDDEN
	);
	let mut login = request(
		"POST",
		"/api/v1/login",
		None,
		Some(serde_json::json!({"key":OWNER})),
	);
	login
		.headers_mut()
		.insert("origin", "https://deploy.example.com".parse().unwrap());
	let response = app.clone().oneshot(login).await.unwrap();
	assert_eq!(response.status(), StatusCode::NO_CONTENT);
	let cookie = response.headers()["set-cookie"].to_str().unwrap();
	assert!(
		cookie.contains("HttpOnly")
			&& cookie.contains("Secure")
			&& cookie.contains("SameSite=Strict")
	);
	let cookie = cookie.split(';').next().unwrap();
	let mut denied = request(
		"POST",
		"/api/v1/projects/notes/operations",
		None,
		Some(body.clone()),
	);
	denied
		.headers_mut()
		.insert("cookie", cookie.parse().unwrap());
	assert_eq!(
		app.clone().oneshot(denied).await.unwrap().status(),
		StatusCode::FORBIDDEN
	);
	let mut allowed = request(
		"POST",
		"/api/v1/projects/notes/operations",
		None,
		Some(body),
	);
	allowed
		.headers_mut()
		.insert("cookie", cookie.parse().unwrap());
	allowed
		.headers_mut()
		.insert("origin", "https://deploy.example.com".parse().unwrap());
	assert_eq!(
		app.clone().oneshot(allowed).await.unwrap().status(),
		StatusCode::OK
	);
	store.revoke(&read).unwrap();
	assert_eq!(
		app.clone()
			.oneshot(request("GET", "/api/v1/overview", Some(&read), None))
			.await
			.unwrap()
			.status(),
		StatusCode::UNAUTHORIZED
	);
	let response = app
		.oneshot(request("GET", "/", Some(OWNER), None))
		.await
		.unwrap();
	let html = String::from_utf8(
		to_bytes(response.into_body(), 2 * 1024 * 1024)
			.await
			.unwrap()
			.to_vec(),
	)
	.unwrap();
	assert!(html.contains("Your applications") && html.contains("notes"));
	assert!(!html.contains(OWNER));
}

#[tokio::test]
async fn the_application_exporter_delivers_to_the_durable_service_without_management_credentials() {
	use leptoskit_analytics::export::Config;
	use leptoskit_analytics::export::Protocol;
	use leptoskit_analytics::server::ServerConfig;
	use leptoskit_analytics::server::{self};
	let (_root, store) = setup();
	let service = Service::new(store.clone(), OWNER, "http://localhost:4700").unwrap();
	let ingest = service.managed_app_key("notes", true).unwrap();
	assert_ne!(ingest, OWNER);
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let router = service.router();
	let serving = tokio::spawn(async move {
		axum::serve(listener, router).await.unwrap();
	});
	let (app, analytics) = server::mount(
		axum::Router::new().route("/notes/{id}", axum::routing::get(|| async { "note" })),
		ServerConfig {
			export: Some(Config {
				endpoint: format!("http://{address}/api/v1/projects/notes/analytics/ingest"),
				key: Some(ingest),
				protocol: Protocol::Leptoskit,
			}),
			resource: Resource {
				service: "notes".into(),
				environment: "production".into(),
				release: "exported-v1".into(),
				instance: "exported-one".into(),
			},
			limits: Limits::default(),
			read_key: OWNER.into(),
			public_origin: "https://app.example.com".into(),
			browser_routes: Vec::new(),
		},
	)
	.unwrap();
	app.clone()
		.oneshot(request("GET", "/notes/private?token=hidden", None, None))
		.await
		.unwrap();
	tokio::time::timeout(std::time::Duration::from_secs(5), async {
		loop {
			let fleet = store
				.fleet("notes", clock().saturating_sub(3600), clock() + 60)
				.unwrap();
			if !fleet.series.is_empty() {
				assert_eq!(fleet.series[0].key.operation, "/notes/{id}");
				assert_eq!(fleet.series[0].windows[0].count, 1);
				assert!(fleet.hosts[0].host.is_some());
				break;
			}
			tokio::time::sleep(std::time::Duration::from_millis(10)).await;
		}
	})
	.await
	.unwrap();
	drop(app);
	drop(analytics);
	serving.abort();
}
