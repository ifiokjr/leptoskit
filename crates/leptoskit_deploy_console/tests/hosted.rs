#![cfg(feature = "server")]
use axum::Router;
use axum::body::Body;
use axum::body::to_bytes;
use axum::http::Request;
use axum::http::StatusCode;
use leptoskit_deploy_console::hosted::Billing;
use leptoskit_deploy_console::hosted::Hosted;
use leptoskit_deploy_console::hosted::Ownership;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt as _;
const SECRET: &str = "test-only-hosted-encryption-secret-0123456789";
fn now() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_secs()
}
fn setup() -> (tempfile::TempDir, Hosted, Billing, String, String) {
	let dir = tempfile::tempdir().unwrap();
	let host = Hosted::open(
		&dir.path().join("cloud"),
		SECRET.into(),
		"http://localhost:4701",
		None,
	)
	.unwrap();
	let billing = Billing::new(
		"test".into(),
		"test-hook".into(),
		"price_control".into(),
		"price_team".into(),
	)
	.unwrap();
	let a = host.accounts().create("alice", "cus_alice").unwrap();
	let b = host.accounts().create("bob", "cus_bob").unwrap();
	for account in ["alice", "bob"] {
		activate(&billing, &host, account, "active");
	}
	(dir, host, billing, a, b)
}
fn activate(billing: &Billing, host: &Hosted, account: &str, status: &str) {
	billing.apply_subscription(host,account,&json!({"id":format!("sub_{account}"),"customer":format!("cus_{account}"),"status":status,"items":{"data":[{"quantity":1,"price":{"id":"price_team"},"current_period_end":now()+3600}]}})).unwrap();
}
async fn call(
	app: &Router,
	method: &str,
	path: &str,
	key: Option<&str>,
	value: Option<Value>,
) -> (StatusCode, Value) {
	let mut request = Request::builder()
		.method(method)
		.uri(path)
		.header("content-type", "application/json");
	if let Some(key) = key {
		request = request.header("authorization", format!("Bearer {key}"));
	}
	let response = app
		.clone()
		.oneshot(
			request
				.body(value.map_or_else(Body::empty, |v| Body::from(v.to_string())))
				.unwrap(),
		)
		.await
		.unwrap();
	let status = response.status();
	let body = to_bytes(response.into_body(), 10 * 1024 * 1024)
		.await
		.unwrap();
	(status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}
async fn pair(app: &Router, host: &Hosted, account: &str, project: &str) -> String {
	let code = host.accounts().pairing(account, project).unwrap();
	let (status, value) = call(
		app,
		"POST",
		"/api/v1/worker/pair",
		None,
		Some(json!({"code":code})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(
		call(
			app,
			"POST",
			"/api/v1/worker/pair",
			None,
			Some(json!({"code":code}))
		)
		.await
		.0,
		StatusCode::UNAUTHORIZED
	);
	value["key"].as_str().unwrap().into()
}
#[tokio::test]
async fn tenants_pair_scoped_workers_and_use_the_standard_cli_contract() {
	let (_dir, host, billing, alice, bob) = setup();
	for account in ["alice", "bob"] {
		host.register(account, "notes", "production", Ownership::Customer)
			.unwrap();
	}
	let app = host.router();
	let worker = pair(&app, &host, "alice", "notes").await;
	let intent =
		json!({"expected_revision":0,"idempotency_key":"request-1","action":{"kind":"status"}});
	let (status, operation) = call(
		&app,
		"POST",
		"/api/v1/projects/notes/operations",
		Some(&alice),
		Some(intent.clone()),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	let (_, again) = call(
		&app,
		"POST",
		"/api/v1/projects/notes/operations",
		Some(&alice),
		Some(intent),
	)
	.await;
	assert_eq!(operation["id"], again["id"]);
	let path = format!("/api/v1/operations/{}", operation["id"].as_str().unwrap());
	assert_eq!(
		call(&app, "GET", &path, Some(&bob), None).await.0,
		StatusCode::NOT_FOUND
	);
	let (_, job) = call(
		&app,
		"POST",
		"/api/v1/worker/claim",
		Some(&worker),
		Some(json!({})),
	)
	.await;
	assert_eq!(job["operation"]["id"], operation["id"]);
	check_export(&app, &job, &alice, &bob).await;
	assert!(
		call(
			&app,
			"POST",
			"/api/v1/worker/claim",
			Some(&worker),
			Some(json!({}))
		)
		.await
		.1
		.is_null()
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/overview", Some(&worker), None)
			.await
			.0,
		StatusCode::UNAUTHORIZED
	);
	let done = json!({"id":operation["id"],"succeeded":true,"message":"Healthy","details":{}});
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/finish",
			Some(&worker),
			Some(done.clone())
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/finish",
			Some(&worker),
			Some(done)
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	check_agent_scopes(&app, &host, &billing, &alice, &worker, &path).await;
}
#[tokio::test]
async fn hosted_state_is_encrypted_locked_versioned_and_tenant_scoped() {
	let (dir, host, _billing, _alice, _bob) = setup();
	for account in ["alice", "bob"] {
		host.register(account, "notes", "production", Ownership::Customer)
			.unwrap();
	}
	let app = host.router();
	let worker = pair(&app, &host, "alice", "notes").await;
	let path = "/api/v1/state/notes";
	let lock = json!({"ID":"lock-1","Operation":"OperationTypeApply"});
	let state = json!({"version":4,"serial":1,"lineage":"fixture-lineage","secret":"sensitive-provider-output"});
	assert_eq!(
		call(
			&app,
			"POST",
			&format!("{path}?ID=lock-1"),
			Some(&worker),
			Some(state.clone())
		)
		.await
		.0,
		StatusCode::CONFLICT
	);
	assert_eq!(
		call(&app, "LOCK", path, Some(&worker), Some(lock.clone()))
			.await
			.0,
		StatusCode::OK
	);
	assert_eq!(
		call(
			&app,
			"LOCK",
			path,
			Some(&worker),
			Some(json!({"ID":"other"}))
		)
		.await
		.0,
		StatusCode::LOCKED
	);
	assert_eq!(
		call(
			&app,
			"POST",
			&format!("{path}?ID=lock-1"),
			Some(&worker),
			Some(state.clone())
		)
		.await
		.0,
		StatusCode::OK
	);
	assert_eq!(call(&app, "GET", path, Some(&worker), None).await.1, state);
	assert!(host.state_read("bob", "notes").unwrap().is_none());
	let raw = std::fs::read(dir.path().join("cloud/accounts.sqlite3-wal")).unwrap();
	assert!(
		!raw.windows(b"sensitive-provider-output".len())
			.any(|w| w == b"sensitive-provider-output")
	);
	assert_eq!(
		call(
			&app,
			"POST",
			&format!("{path}?ID=lock-1"),
			Some(&worker),
			Some(json!({"version":4,"serial":1,"lineage":"fixture-lineage","changed":true}))
		)
		.await
		.0,
		StatusCode::CONFLICT
	);
	assert_eq!(
		call(&app, "UNLOCK", path, Some(&worker), Some(lock))
			.await
			.0,
		StatusCode::OK
	);
	let reopened = Hosted::open(
		&dir.path().join("cloud"),
		SECRET.into(),
		"http://localhost:4701",
		None,
	)
	.unwrap();
	assert_eq!(
		reopened.state_read("alice", "notes").unwrap().unwrap(),
		serde_json::to_vec(&state).unwrap()
	);
	assert!(
		Hosted::open(
			&dir.path().join("cloud"),
			"a-different-persistent-encryption-key-012345".into(),
			"http://localhost:4701",
			None
		)
		.is_err()
	);
	host.backup(&dir.path().join("backup")).unwrap();
	assert!(dir.path().join("backup/alice/journal.sqlite3").is_file());
}
#[tokio::test]
async fn expired_subscriptions_and_managed_capacity_do_not_grant_unapproved_resources() {
	let (_dir, host, billing, alice, _) = setup();
	activate(&billing, &host, "alice", "incomplete");
	assert!(
		host.register("alice", "notes", "production", Ownership::Customer)
			.is_err()
	);
	activate(&billing, &host, "alice", "active");
	assert_eq!(
		call(
			&host.router(),
			"POST",
			"/api/v1/installations",
			Some(&alice),
			Some(
				json!({"project":"notes","environment":"production","ownership":"leptoskit_digitalocean"})
			)
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert!(
		billing
			.apply_subscription(&host, "alice", &json!({"customer":"cus_bob"}))
			.is_err()
	);
}

async fn check_agent_scopes(
	app: &Router,
	host: &Hosted,
	billing: &Billing,
	alice: &str,
	worker: &str,
	path: &str,
) {
	let (_, scoped) = call(
		app,
		"POST",
		"/api/v1/tokens",
		Some(alice),
		Some(json!({"project":"notes","scope":"deploy"})),
	)
	.await;
	let agent = scoped["key"].as_str().unwrap();
	assert!(agent.starts_with("alice."));
	assert_eq!(
		call(app, "GET", "/api/v1/overview", Some(agent), None)
			.await
			.0,
		StatusCode::OK
	);
	assert_eq!(
		call(app, "GET", "/api/v1/account", Some(agent), None)
			.await
			.0,
		StatusCode::UNAUTHORIZED
	);
	activate(billing, host, "alice", "past_due");
	let deploy = json!({"expected_revision":1,"idempotency_key":"release-1","action":{"kind":"deploy","version":"v1"}});
	assert_eq!(
		call(
			app,
			"POST",
			"/api/v1/projects/notes/operations",
			Some(agent),
			Some(deploy)
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		call(app, "GET", path, Some(alice), None).await.0,
		StatusCode::OK
	);
	call(
		app,
		"POST",
		"/api/v1/tokens/revoke",
		Some(alice),
		Some(json!({"key":agent})),
	)
	.await;
	assert_eq!(
		call(app, "GET", "/api/v1/overview", Some(agent), None)
			.await
			.0,
		StatusCode::UNAUTHORIZED
	);
	host.accounts().revoke_worker("alice", "notes").unwrap();
	assert_eq!(
		call(
			app,
			"POST",
			"/api/v1/worker/claim",
			Some(worker),
			Some(json!({}))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
}

#[tokio::test]
async fn offline_restore_verifies_encrypted_state_and_revokes_restored_access() {
	let (dir, host, _billing, alice, _bob) = setup();
	host.register("alice", "notes", "production", Ownership::Customer)
		.unwrap();
	let app = host.router();
	let worker = pair(&app, &host, "alice", "notes").await;
	let lock = json!({"ID":"restore-test"});
	host.state_lock("alice", "notes", &lock, false).unwrap();
	let body =
		serde_json::to_vec(&json!({"version":4,"serial":1,"lineage":"restore-lineage"})).unwrap();
	host.state_write("alice", "notes", "restore-test", &body)
		.unwrap();
	host.backup(&dir.path().join("backup")).unwrap();
	let restored = dir.path().join("restored");
	Hosted::restore_new(&dir.path().join("backup"), &restored, SECRET.into()).unwrap();
	let recovered = Hosted::open(&restored, SECRET.into(), "http://localhost:4701", None).unwrap();
	assert_eq!(
		recovered.state_read("alice", "notes").unwrap().unwrap(),
		body
	);
	assert_eq!(
		call(
			&recovered.router(),
			"GET",
			"/api/v1/account",
			Some(&alice),
			None
		)
		.await
		.0,
		StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		call(
			&recovered.router(),
			"POST",
			"/api/v1/worker/claim",
			Some(&worker),
			Some(json!({}))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	let rotated = recovered.accounts().rotate("alice").unwrap();
	assert_eq!(
		call(
			&recovered.router(),
			"GET",
			"/api/v1/account",
			Some(&rotated),
			None
		)
		.await
		.0,
		StatusCode::OK
	);
	assert!(Hosted::restore_new(&dir.path().join("backup"), &restored, SECRET.into()).is_err());
	assert!(
		Hosted::restore_new(
			&dir.path().join("backup"),
			&dir.path().join("bad-key"),
			"an-incorrect-persistent-master-key-012345".into()
		)
		.is_err()
	);
	assert!(!dir.path().join("bad-key").exists());
	recovered
		.rollback_schema(&dir.path().join("before-down"))
		.unwrap();
	drop(recovered);
	let empty = Hosted::open(&restored, SECRET.into(), "http://localhost:4701", None).unwrap();
	assert!(empty.accounts().view("alice").is_err());
}

async fn check_export(app: &Router, job: &Value, alice: &str, bob: &str) {
	use leptoskit::analytics::Analytics;
	use leptoskit::analytics::Limits;
	use leptoskit::analytics::Resource;
	let analytics = Analytics::new(
		Resource {
			service: "notes".into(),
			environment: "production".into(),
			release: "v1".into(),
			instance: "one".into(),
		},
		Limits::default(),
		now,
	)
	.unwrap();
	let write = job["analytics_ingest_key"].as_str().unwrap();
	assert_eq!(
		call(
			app,
			"POST",
			"/api/v1/projects/notes/analytics/ingest",
			Some(write),
			Some(serde_json::to_value(analytics.snapshot()).unwrap())
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	assert_eq!(
		call(
			app,
			"GET",
			"/api/v1/projects/notes/analytics",
			Some(alice),
			None
		)
		.await
		.1["instances"],
		1
	);
	assert_eq!(
		call(
			app,
			"GET",
			"/api/v1/projects/notes/analytics",
			Some(bob),
			None
		)
		.await
		.1["instances"],
		0
	);
}
