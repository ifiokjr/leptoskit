//! Each role starts only its own part of Huddle: `web` the API and
//! sign-in, `live` the socket, `worker` the job workers, and `capture`
//! capture, so one binary can run as any part of a split deployment.

use huddle_server::Huddle;
use huddle_server::Options;
use leptoskit_auth::Auth;
use leptoskit_auth::Keys;
use leptoskit_rpc::server::Role;
use leptoskit_rpc::server::Roles;

/// Huddle with `roles`, serving on a free port.
async fn serve(roles: Roles) -> (Huddle, String) {
	let db = leptoskit_db::testing::sqlite_migrated(huddle_db::MIGRATIONS).await;
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let base = format!("http://{}", listener.local_addr().unwrap());
	let mut options = Options::new(&db);
	options.roles = roles;
	let auth = Auth::builder(&db, Keys::single("test", [5; 32])).origin(&base);
	let huddle = huddle_server::start(&db, auth, options).unwrap();
	let router = huddle.router.clone();
	tokio::spawn(async move {
		leptoskit::server::serve_listener(listener, router)
			.await
			.unwrap();
	});
	(huddle, base)
}

/// The status of `POST /api/whoami` and of a plain `GET /live`.
async fn statuses(base: &str) -> (u16, u16) {
	let http = reqwest::Client::new();
	let api = http
		.post(format!("{base}/api/whoami"))
		.header("origin", base)
		.header("content-type", "application/json")
		.body("{}")
		.send()
		.await
		.unwrap()
		.status()
		.as_u16();
	let live = http
		.get(format!("{base}/live"))
		.send()
		.await
		.unwrap()
		.status()
		.as_u16();
	(api, live)
}

#[tokio::test(flavor = "multi_thread")]
async fn every_role_by_default() {
	let (huddle, base) = serve(Roles::ALL).await;
	let (api, live) = statuses(&base).await;
	assert_ne!(api, 404, "the API is served");
	assert_ne!(live, 404, "the socket is served");
	assert!(huddle.jobs.is_some(), "the workers run, with capture");
	assert!(
		huddle.capture.is_none(),
		"the workers run capture themselves"
	);
	huddle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn web_serves_the_api_only() {
	let (huddle, base) = serve(Roles::only([Role::Web])).await;
	let (api, live) = statuses(&base).await;
	assert_ne!(api, 404);
	assert_eq!(live, 404, "no socket");
	assert!(huddle.jobs.is_none() && huddle.capture.is_none() && huddle.publisher.is_none());
	assert!(!huddle.live.is_started(), "the live hub never starts");
}

#[tokio::test(flavor = "multi_thread")]
async fn live_serves_the_socket_only() {
	let (huddle, base) = serve(Roles::only([Role::Live])).await;
	huddle.live.start().await.unwrap();
	let (api, live) = statuses(&base).await;
	assert_eq!(api, 404, "no API");
	assert_ne!(live, 404);
	assert!(huddle.jobs.is_none() && huddle.capture.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_and_capture_serve_nothing_but_run_in_the_background() {
	let (huddle, base) = serve(Roles::only([Role::Worker])).await;
	assert_eq!(statuses(&base).await, (404, 404));
	assert!(huddle.jobs.is_some());
	assert!(huddle.capture.is_none());
	huddle.shutdown().await;

	let (huddle, base) = serve(Roles::only([Role::Capture])).await;
	assert_eq!(statuses(&base).await, (404, 404));
	assert!(huddle.jobs.is_none(), "no workers");
	assert!(huddle.capture.is_some(), "capture runs on its own");
	assert!(huddle.publisher.is_none(), "no relay to publish to");
	huddle.shutdown().await;
}
