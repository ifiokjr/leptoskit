//! The service on a real port: anonymous capture, the public summary, the
//! app-only list, a live summary that updates when an event arrives, and a
//! signed-in person's own events, which the app forwards them to read.

use std::time::Duration;

use analytics_service::MIGRATIONS;
use futures::SinkExt;
use futures::StreamExt;
use leptoskit_rpc as rpc;
use rpc::server::Anonymous;
use rpc::server::ServiceCallers;
use serde_json::Value;
use serde_json::json;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const TOKEN: &str = "app-token";
const ORIGIN: &str = "http://app.test";
const PREFIX: &str = "/_leptoskit/services/analytics";

/// The service with a fresh, migrated database, on a free port.
async fn start() -> std::net::SocketAddr {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;
	let (router, live) = analytics_service::routes_with(
		db,
		Some(ORIGIN),
		ServiceCallers::new(Anonymous)
			.caller(TOKEN, rpc::server::APP_CALLER)
			.forwarded_identity(analytics_service::SERVICE, TOKEN),
	);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	address
}

async fn post(
	address: std::net::SocketAddr,
	operation: &str,
	args: Value,
	token: Option<&str>,
) -> (u16, String) {
	let mut request = reqwest::Client::new()
		.post(format!("http://{address}{PREFIX}/api/{operation}"))
		.json(&args);

	if let Some(token) = token {
		request = request.bearer_auth(token);
	}

	let response = request.send().await.unwrap();
	(response.status().as_u16(), response.text().await.unwrap())
}

fn capture(event: &str) -> Value {
	json!({ "event": event, "distinct_id": "visitor-1", "properties": { "page": "/" } })
}

#[tokio::test(flavor = "multi_thread")]
async fn browsers_capture_and_read_the_summary_but_only_the_app_reads_events() {
	let address = start().await;

	let (status, _) = post(address, "capture", capture("signup"), None).await;
	assert_eq!(status, 200, "anyone may capture");

	let (status, body) = post(address, "capture", capture("not a name!"), None).await;
	assert_eq!(status, 400, "a bad event name is refused: {body}");

	let (status, body) = post(address, "summary", json!({ "days": 7 }), None).await;
	assert_eq!(status, 200, "the aggregate is public: {body}");
	let summary: analytics_service::EventSummary = serde_json::from_str(&body).unwrap();
	assert_eq!((summary.total, summary.visitors), (1, 1));
	assert_eq!(summary.events[0].name, "signup");

	let (status, _) = post(address, "recent", json!({ "limit": 10 }), None).await;
	assert_eq!(status, 403, "a browser can't read individual events");

	let (status, _) = post(address, "recent", json!({ "limit": 10 }), Some("wrong")).await;
	assert_eq!(status, 403, "nor can a wrong token");

	let (status, body) = post(address, "recent", json!({ "limit": 10 }), Some(TOKEN)).await;
	assert_eq!(status, 200, "the app can: {body}");
	let recent: Vec<analytics_service::RecentEvent> = serde_json::from_str(&body).unwrap();
	assert_eq!(recent.len(), 1);
	assert_eq!(recent[0].properties["page"], "/");

	// The operations live under the service's prefix only.
	let status = reqwest::Client::new()
		.post(format!("http://{address}/api/capture"))
		.json(&capture("signup"))
		.send()
		.await
		.unwrap()
		.status();
	assert_eq!(status, 404);
}

/// The token the app's server would mint for `person`, as it does when a
/// signed-in browser asks for one.
fn signed_in(person: uuid::Uuid) -> String {
	rpc::server::IdentityKey::new(analytics_service::SERVICE, TOKEN)
		.expect("a key")
		.mint(
			person,
			std::time::SystemTime::now(),
			rpc::server::IDENTITY_LIFETIME,
		)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_in_person_reads_their_own_events_and_nobody_elses() {
	let address = start().await;
	let ada = uuid::Uuid::from_u128(0xada);
	let grace = uuid::Uuid::from_u128(0x9ace);
	assert_eq!(
		analytics_service::PERSON_PREFIX,
		rpc::server::PERSON_CALLER_PREFIX
	);

	// Ada's events are recorded under her, whatever visitor id the browser
	// sent; an anonymous visitor's stay theirs.
	for (event, token) in [
		("signup", Some(signed_in(ada))),
		("clicked_buy", Some(signed_in(ada))),
		("signup", Some(signed_in(grace))),
		("signup", None),
	] {
		let (status, body) = post(address, "capture", capture(event), token.as_deref()).await;
		assert_eq!(status, 200, "{body}");
	}

	let (status, body) = post(
		address,
		"mine",
		json!({ "limit": 10 }),
		Some(&signed_in(ada)),
	)
	.await;
	assert_eq!(status, 200, "{body}");
	let mine: Vec<analytics_service::RecentEvent> = serde_json::from_str(&body).unwrap();
	assert_eq!(
		mine.iter()
			.map(|event| event.name.as_str())
			.collect::<Vec<_>>(),
		["clicked_buy", "signup"],
		"only Ada's, newest first"
	);
	assert!(
		mine.iter()
			.all(|event| event.distinct_id == format!("person:{ada}"))
	);

	// Nobody signed in, the app itself, and a token that isn't this
	// service's own, read nobody's.
	let (status, _) = post(address, "mine", json!({ "limit": 10 }), None).await;
	assert_eq!(status, 403);
	let (status, _) = post(address, "mine", json!({ "limit": 10 }), Some(TOKEN)).await;
	assert_eq!(status, 403);
	let elsewhere = rpc::server::IdentityKey::new("other", TOKEN).unwrap().mint(
		ada,
		std::time::SystemTime::now(),
		rpc::server::IDENTITY_LIFETIME,
	);
	let (status, _) = post(address, "mine", json!({ "limit": 10 }), Some(&elsewhere)).await;
	assert_eq!(status, 401, "the client fetches a new token");

	// Nobody can write to a signed-in person's events by naming them.
	let forged = json!({ "event": "signup", "distinct_id": format!("person:{ada}") });
	let (status, _) = post(address, "capture", forged, None).await;
	assert_eq!(status, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_in_persons_live_events_update_when_they_capture() {
	let address = start().await;
	let ada = uuid::Uuid::from_u128(0xada);
	let mut request = format!("ws://{address}{PREFIX}/live")
		.into_client_request()
		.unwrap();
	request
		.headers_mut()
		.insert("origin", ORIGIN.parse().unwrap());
	let (mut socket, _) = connect_async(request).await.expect("the live socket opens");
	let send = |frame: Value| tungstenite::Message::Text(frame.to_string().into());
	// The browser's socket signs in with its token for the service.
	socket
		.send(send(json!({ "type": "auth", "token": signed_in(ada) })))
		.await
		.unwrap();
	socket.send(send(json!({ "type": "hello" }))).await.unwrap();
	socket
		.send(send(
			json!({ "type": "sub", "id": 1, "query": "mine", "args": { "limit": 10 } }),
		))
		.await
		.unwrap();
	// The first value: nothing of Ada's yet.
	wait_for(&mut socket, "\"ok\":[]").await;

	let (status, _) = post(address, "capture", capture("signup"), Some(&signed_in(ada))).await;
	assert_eq!(status, 200);
	wait_for(&mut socket, &format!("person:{ada}")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_summary_updates_when_an_event_arrives() {
	let address = start().await;
	let mut request = format!("ws://{address}{PREFIX}/live")
		.into_client_request()
		.unwrap();
	request
		.headers_mut()
		.insert("origin", ORIGIN.parse().unwrap());
	let (mut socket, _) = connect_async(request).await.expect("the live socket opens");

	let send = |frame: Value| tungstenite::Message::Text(frame.to_string().into());
	socket.send(send(json!({ "type": "hello" }))).await.unwrap();
	socket
		.send(send(
			json!({ "type": "sub", "id": 1, "query": "summary", "args": { "days": 7 } }),
		))
		.await
		.unwrap();

	// The first value: nothing yet.
	wait_for(&mut socket, "\"total\":0").await;

	let (status, _) = post(address, "capture", capture("clicked_buy"), None).await;
	assert_eq!(status, 200);

	// The capture's commit re-runs the summary, and the socket hears it.
	wait_for(&mut socket, "\"total\":1").await;
}

/// Reads frames until one contains `needle`.
async fn wait_for<S>(socket: &mut S, needle: &str)
where
	S: StreamExt<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin,
{
	let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
	loop {
		let frame = tokio::time::timeout_at(deadline, socket.next())
			.await
			.unwrap_or_else(|_| panic!("no frame with {needle} within 20 s"))
			.expect("the socket stays open")
			.unwrap();

		if let tungstenite::Message::Text(text) = frame
			&& text.contains(needle)
		{
			return;
		}
	}
}
