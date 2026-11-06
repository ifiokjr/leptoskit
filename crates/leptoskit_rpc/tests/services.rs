//! Operations another service runs, as an app and its services see them:
//!
//! - an app's server never serves a service's operation, even when its body is
//!   linked into the app's build, as in a workspace-wide build;
//! - the app's server calls a service's operation over HTTP at the service's
//!   URL, as an anonymous caller, and only `call_as_app` signs with the
//!   service's token, which the service knows as the app;
//! - a service's own process runs its operations in process;
//! - a service's live socket works under its prefix, and the app's live
//!   connection refuses a service, at the upgrade and when it signs in;
//! - a service's jobs, cron schedules, streams and channels belong to the
//!   service even when linked into the app's build: the app's workers never run
//!   or schedule them, the app can't enqueue them, and the app's live socket
//!   doesn't serve them.

#![cfg(all(feature = "live", feature = "sqlite"))]
#![allow(
	unsafe_code,
	reason = "the tests set the variables `leptoskit dev` gives the app"
)]

use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::body::Body;
use futures::StreamExt;
use http::Request;
use http::StatusCode;
use leptoskit_rpc as rpc;
use leptoskit_rpc::client;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::server::Anonymous;
use leptoskit_rpc::server::Live;
use leptoskit_rpc::server::ServiceCallers;
use leptoskit_rpc::testing::TestJobs;
use serde::Deserialize;
use serde::Serialize;
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt as _;

/// The `echo` service's operation, with its body linked into this build:
/// what `#[rpc::query(service = "echo")]` expands to when the declaring
/// crate's `service` feature is on, as in a workspace-wide build of an app
/// and its services. Says who called.
#[derive(Clone, Serialize, Deserialize)]
struct Ping {
	word: String,
}

impl rpc::Operation for Ping {
	type Context = rpc::QueryCx;
	type Output = String;

	const KIND: rpc::Kind = rpc::Kind::Query;
	const NAME: &'static str = "ping";
	const SERVICE: Option<&'static str> = Some("echo");

	fn run(self, cx: &rpc::QueryCx) -> impl Future<Output = rpc::Result<String>> + Send {
		let caller = cx
			.identity()
			.caller
			.clone()
			.unwrap_or_else(|| "nobody".to_owned());
		std::future::ready(Ok(format!("{} from {caller}", self.word)))
	}
}

#[allow(deprecated)]
const _: () = {
	rpc::__private::inventory::submit! {
		rpc::server::Registration {
			name: <Ping as rpc::Operation>::NAME,
			kind: <Ping as rpc::Operation>::KIND,
			module_path: module_path!(),
			service: <Ping as rpc::Operation>::SERVICE,
			handler: rpc::__private::handle::<Ping>,
		}
	}
};

/// The app's own operation of the same name: allowed, since each is served
/// by its own server.
#[rpc::query(name = "ping")]
async fn app_ping(_cx: &rpc::QueryCx, word: String) -> rpc::Result<String> {
	Ok(format!("{word} from the app"))
}

fn hi() -> Ping {
	Ping {
		word: "hi".to_owned(),
	}
}

fn app_ping_op() -> AppPing {
	AppPing {
		word: "hi".to_owned(),
	}
}

async fn serve(app: axum::Router) -> std::net::SocketAddr {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
	address
}

async fn post(router: axum::Router, path: &str) -> (StatusCode, String) {
	let response = router
		.oneshot(
			Request::post(path)
				.header("content-type", "application/json")
				.body(Body::from(r#"{"word":"hi"}"#))
				.unwrap(),
		)
		.await
		.unwrap();
	let status = response.status();
	let body = http_body_util::BodyExt::collect(response.into_body())
		.await
		.unwrap()
		.to_bytes();
	(status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test(flavor = "multi_thread")]
async fn each_server_serves_only_its_own_operations() {
	let db = leptoskit_db::testing::sqlite().await;

	// The app's server routes its own `ping`, not the service's.
	let app = rpc::server::Server::new(db.clone());
	assert_eq!(
		post(app.router(), "/api/ping").await,
		(StatusCode::OK, r#""hi from the app""#.to_owned())
	);

	// The service's server routes the service's `ping`, not the app's, and
	// the built-in `rpc.*` operations it needs for its own jobs and inbox.
	let service = rpc::server::Server::new(db).service("echo");
	assert_eq!(service.service_name(), Some("echo"));
	assert_eq!(
		post(service.router(), "/api/ping").await,
		(StatusCode::OK, r#""hi from nobody""#.to_owned())
	);
	let served: Vec<&str> = rpc::server::registrations_in(Some("echo"))
		.unwrap()
		.into_iter()
		.map(|registration| registration.name)
		.collect();
	assert!(served.contains(&"ping"), "{served:?}");
	assert!(served.contains(&"rpc.inbox"), "{served:?}");
	assert_eq!(
		rpc::server::registration_in(Some("echo"), "ping").map(|r| r.service),
		Some(Some("echo"))
	);
	assert_eq!(
		rpc::server::registration("ping").map(|r| r.service),
		Some(None)
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_apps_server_calls_a_service_anonymously_unless_as_the_app() {
	let db = leptoskit_db::testing::sqlite().await;
	// The app's server is installed for in-process calls, and its build
	// links the service's body: the call still goes to the service.
	rpc::server::Server::new(db.clone()).install();

	// Before `leptoskit dev` says where the service is, the call says so.
	let error = client::call(hi()).await.unwrap_err();
	assert!(
		error.message().contains("LEPTOSKIT_SERVICE_ECHO_URL"),
		"{error}"
	);

	let service = rpc::server::Server::new(db)
		.service("echo")
		.identify(ServiceCallers::new(Anonymous).caller("t0k3n", rpc::server::APP_CALLER));
	let address =
		serve(axum::Router::new().nest(&rpc::service_prefix("echo"), service.router())).await;
	// `router` installs the service's server; this process is the app's.
	rpc::server::Server::new(leptoskit_db::testing::sqlite().await).install();

	// SAFETY: nextest runs each test in its own process, and the variables
	// are set before anything reads them.
	unsafe {
		std::env::set_var("LEPTOSKIT_SERVICE_ECHO_URL", format!("http://{address}"));
	}
	assert_eq!(client::call(hi()).await, Ok("hi from nobody".to_owned()));

	// SAFETY: as above.
	unsafe {
		std::env::set_var("LEPTOSKIT_SERVICE_ECHO_TOKEN", "t0k3n");
	}
	// `call`, which the query scopes and server rendering use, stays
	// anonymous though the server holds the token: a page renders only what
	// the browser could fetch.
	assert_eq!(client::call(hi()).await, Ok("hi from nobody".to_owned()));
	// Only `call_as_app` signs as the app.
	assert_eq!(
		client::call_as_app(hi()).await,
		Ok("hi from app".to_owned())
	);
	let error = client::call_as_app(app_ping_op()).await.unwrap_err();
	assert!(error.message().contains("app's own operations"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_runs_its_own_operations_in_process() {
	let db = leptoskit_db::testing::sqlite().await;
	// The service's process: its installed server is the service's, and no
	// URL for itself is set.
	rpc::server::Server::new(db).service("echo").install();
	assert_eq!(client::call(hi()).await, Ok("hi from nobody".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_services_live_socket_works_under_its_prefix() {
	let db = leptoskit_db::testing::sqlite().await;
	let live = rpc::server::Server::new(db).service("echo").live();
	let prefix = rpc::service_prefix("echo");
	let address = serve(axum::Router::new().nest(&prefix, live.router())).await;

	let (mut socket, _) = connect_async(format!("ws://{address}{prefix}/live"))
		.await
		.unwrap();
	let message = tokio::time::timeout(Duration::from_secs(20), socket.next())
		.await
		.expect("a frame arrives")
		.unwrap()
		.unwrap();
	let tungstenite::Message::Text(text) = message else {
		panic!("expected a text frame: {message:?}");
	};
	assert!(matches!(
		serde_json::from_str::<ServerFrame>(&text).unwrap(),
		ServerFrame::Welcome { .. }
	));

	// Server-sent events, the socket's fallback, are under the prefix too.
	let events = reqwest::Client::new()
		.get(format!("http://{address}{prefix}/live/events"))
		.send()
		.await
		.unwrap();
	assert_ne!(events.status(), 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_apps_live_connection_refuses_a_service() {
	let db = leptoskit_db::testing::sqlite().await;
	let live = Live::new(db).identify(ServiceCallers::new(Anonymous).caller("svc", "service:echo"));
	let address = serve(live.router()).await;

	let mut request = format!("ws://{address}/live")
		.into_client_request()
		.unwrap();
	request
		.headers_mut()
		.insert("authorization", "Bearer svc".parse().unwrap());
	match connect_async(request).await {
		Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 403),
		other => panic!("the socket opened for a service: {:?}", other.map(|_| ())),
	}

	let events = reqwest::Client::new()
		.get(format!("http://{address}/live/events"))
		.bearer_auth("svc")
		.send()
		.await
		.unwrap();
	assert_eq!(events.status(), 403);
	assert!(
		events.text().await.unwrap().contains("call its operations"),
		"the refusal says what to do instead"
	);

	// Anyone else still connects.
	assert!(connect_async(format!("ws://{address}/live")).await.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_apps_live_connection_refuses_a_service_signing_in_after_it_opens() {
	let db = leptoskit_db::testing::sqlite().await;
	let live = Live::new(db).identify(ServiceCallers::new(Anonymous).caller("svc", "service:echo"));
	let address = serve(live.router()).await;

	// Opened anonymously, which the upgrade allows…
	let (mut socket, _) = connect_async(format!("ws://{address}/live")).await.unwrap();
	let _welcome = socket.next().await;

	// …then signed in with a service's token: refused, and closed.
	let frame = serde_json::to_string(&ClientFrame::Auth {
		token: "svc".into(),
	})
	.unwrap();
	futures::SinkExt::send(&mut socket, tungstenite::Message::Text(frame.into()))
		.await
		.unwrap();

	let refused = tokio::time::timeout(Duration::from_secs(20), async {
		while let Some(Ok(message)) = socket.next().await {
			if let tungstenite::Message::Text(text) = message
				&& let Ok(ServerFrame::AuthStatus { refused, .. }) =
					serde_json::from_str::<ServerFrame>(&text)
			{
				return refused;
			}
		}
		false
	})
	.await
	.expect("an answer arrives");
	assert!(refused, "the service's sign-in is refused");

	let closed = tokio::time::timeout(Duration::from_secs(20), async {
		while let Some(message) = socket.next().await {
			if matches!(message, Ok(tungstenite::Message::Close(_)) | Err(_)) {
				break;
			}
		}
	})
	.await;
	assert!(closed.is_ok(), "the server closes the socket");
}

/// A service's operation as the app's build declares it: this test crate
/// has no `service` feature, as an app's server build of the service's
/// crate doesn't, so `#[rpc::query(service = "...")]` gives the typed stub.
#[allow(
	unexpected_cfgs,
	reason = "this test crate stands in for a service crate without its `service` feature"
)]
mod elsewhere {
	use leptoskit_rpc as rpc;

	/// Lives in the `far-away` service.
	#[rpc::query(service = "far-away")]
	pub async fn far(cx: &rpc::QueryCx, n: u32) -> rpc::Result<u32> {
		let _ = cx;
		Ok(n)
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_operation_without_its_body_is_a_stub_the_client_sends_away() {
	use rpc::Operation as _;

	assert_eq!(elsewhere::Far::SERVICE, Some("far-away"));
	assert_eq!(elsewhere::Far::NAME, "far");
	// No body, so nothing registers it: no server here could serve it.
	assert!(rpc::server::registration_in(Some("far-away"), "far").is_none());

	rpc::server::Server::new(leptoskit_db::testing::sqlite().await).install();
	let error = client::call(elsewhere::Far { n: 1 }).await.unwrap_err();
	assert!(
		error
			.message()
			.contains("LEPTOSKIT_SERVICE_FAR_AWAY_URL isn't set"),
		"{error}"
	);
}

/// How many times the `echo` service's jobs ran.
static ROLLED_UP: AtomicU32 = AtomicU32::new(0);
static SETTLED: AtomicU32 = AtomicU32::new(0);

/// The `echo` service's cron job, with its body linked into this build, as
/// `#[rpc::cron("* * * * *")] #[rpc::job(service = "echo")]` gives it in a
/// workspace-wide build.
#[derive(Clone, Serialize, Deserialize)]
struct RollUp {}

impl rpc::Job for RollUp {
	type Output = ();

	const NAME: &'static str = "roll_up";
	const SERVICE: Option<&'static str> = Some("echo");

	fn run(self, _cx: &rpc::JobCx) -> impl Future<Output = rpc::Result<()>> + Send {
		ROLLED_UP.fetch_add(1, Ordering::SeqCst);
		std::future::ready(Ok(()))
	}
}

/// The `echo` service's job without a schedule.
#[derive(Clone, Serialize, Deserialize)]
struct Settle {}

impl rpc::Job for Settle {
	type Output = ();

	const NAME: &'static str = "settle";
	const SERVICE: Option<&'static str> = Some("echo");

	fn run(self, _cx: &rpc::JobCx) -> impl Future<Output = rpc::Result<()>> + Send {
		SETTLED.fetch_add(1, Ordering::SeqCst);
		std::future::ready(Ok(()))
	}
}

#[allow(deprecated)]
const _: () = {
	rpc::__private::inventory::submit! {
		rpc::server::jobs::JobRegistration {
			name: "roll_up",
			options: rpc::JobOptions::DEFAULT,
			handler: rpc::__private::run_job::<RollUp>,
			on_failure: None,
			service: Some("echo"),
		}
	}
	rpc::__private::inventory::submit! {
		rpc::server::jobs::JobRegistration {
			name: "settle",
			options: rpc::JobOptions::DEFAULT,
			handler: rpc::__private::run_job::<Settle>,
			on_failure: None,
			service: Some("echo"),
		}
	}
	rpc::__private::inventory::submit! {
		rpc::server::jobs::cron::CronRegistration {
			name: "roll_up",
			expression: "* * * * *",
			schedule: rpc::server::jobs::cron::Schedule {
				minutes: (1 << 60) - 1,
				hours: (1 << 24) - 1,
				days: !1,
				months: ((1 << 13) - 1) & !1,
				weekdays: (1 << 7) - 1,
				any_day: true,
				any_weekday: true,
			},
			enqueue: |db, key, tick| rpc::__private::enqueue_tick(RollUp {}, db, key, tick),
			service: Some("echo"),
		}
	}
	rpc::__private::inventory::submit! {
		rpc::server::stream::StreamRegistration {
			name: "tail",
			resumable: false,
			open: |_, _, _| Box::pin(async {}),
			service: Some("echo"),
		}
	}
	rpc::__private::inventory::submit! {
		rpc::server::channel::ChannelRegistration {
			name: "room",
			authorize: |_, _| Box::pin(async { Ok(()) }),
			service: Some("echo"),
		}
	}
};

#[tokio::test(flavor = "multi_thread")]
async fn the_app_never_schedules_or_enqueues_a_services_job() {
	let db = leptoskit_db::testing::sqlite().await;
	// The app's process: its server is installed, and its build links the
	// service's cron job.
	rpc::server::Server::new(db.clone()).install();

	let app = TestJobs::new(&db);
	app.advance(Duration::from_secs(600)).await;
	assert!(
		app.all().await.iter().all(|job| job.name != "roll_up"),
		"the app's scheduler leaves the service's schedule alone"
	);

	let error = rpc::enqueue(&db, Settle {}).await.unwrap_err();
	assert!(
		error
			.message()
			.contains("only the `echo` service enqueues it"),
		"{error}"
	);

	// The service's process schedules its own.
	rpc::server::Server::new(db.clone())
		.service("echo")
		.install();
	let echo = TestJobs::new(&db).service(Some("echo"));
	echo.advance(Duration::from_secs(180)).await;
	let ticks = echo
		.all()
		.await
		.into_iter()
		.filter(|job| job.name == "roll_up")
		.count();
	assert!(ticks >= 2, "{ticks} ticks");
	echo.run_jobs_until_idle().await;
	assert_eq!(
		ROLLED_UP.load(Ordering::SeqCst),
		u32::try_from(ticks).unwrap()
	);

	// And the service's process won't enqueue the app's jobs either: a
	// service's job enqueued from its own process works, as above.
	assert!(rpc::enqueue(&db, Settle {}).await.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_apps_workers_never_run_a_services_job() {
	let db = leptoskit_db::testing::sqlite().await;
	// No server is installed, so nothing stops this row going in: as if a
	// service's job ended up in the app's database some other way.
	rpc::enqueue(&db, Settle {}).await.unwrap();

	let app = TestJobs::new(&db);
	app.run_jobs_until_idle().await;
	assert_eq!(
		SETTLED.load(Ordering::SeqCst),
		0,
		"the app's workers don't run the service's job"
	);

	// The service's workers do, once any backoff has passed.
	let echo = TestJobs::new(&db).service(Some("echo"));
	echo.advance(Duration::from_secs(3600)).await;
	echo.run_jobs_until_idle().await;
	assert_eq!(SETTLED.load(Ordering::SeqCst), 1);
}

#[test]
fn the_app_doesnt_serve_a_services_streams_or_channels() {
	// What the app's live socket looks up: nothing.
	assert!(rpc::server::stream::registration("tail").is_none());
	assert!(rpc::server::channel::registration("room").is_none());
	// What the service's looks up.
	assert!(rpc::server::stream::registration_in(Some("echo"), "tail").is_some());
	assert!(rpc::server::channel::registration_in(Some("echo"), "room").is_some());
	assert!(rpc::server::stream::registration_in(Some("other"), "tail").is_none());
}
