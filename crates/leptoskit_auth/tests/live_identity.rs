//! An open connection loses its first-party identity when its session ends,
//! even without subscriptions. Both transports use the same session guard.

#![cfg(all(
	debug_assertions,
	feature = "rpc",
	feature = "axum",
	feature = "sqlite",
	feature = "postgres"
))]

#[path = "../../leptoskit_rpc/tests/support/nats.rs"]
mod nats;
#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use futures::SinkExt;
use futures::StreamExt;
use http_body_util::BodyExt;
use leptoskit_auth::Auth;
use leptoskit_auth::Keys;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::SignIn;
use leptoskit_auth::Transport;
use leptoskit_db::Connection as _;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db::Id;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use leptoskit_rpc::Identify;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::server::Live;
use postgres::PostgresCluster;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;

const ORIGIN: &str = "https://live.example";
const WAIT: Duration = Duration::from_secs(5);

async fn databases(cluster: Option<&PostgresCluster>) -> Vec<Db> {
	let mut databases = vec![leptoskit_db::testing::sqlite().await];
	if let Some(cluster) = cluster {
		let db = Db::connect(&cluster.database()).await.unwrap();
		leptoskit_db::testing::create_tables(&db).await.unwrap();
		databases.push(db);
	}
	databases
}

async fn set<T>(db: &Db, table: &str, column: &str, value: Value, id: Id<T>) {
	let (table, first, second) = match db.dialect() {
		Dialect::Postgres => (format!("auth.{table}"), "$1", "$2"),
		Dialect::Sqlite => (format!("auth_{table}"), "?1", "?2"),
	};
	db.system("test: change authentication state")
		.execute(Statement {
			sql: format!("UPDATE {table} SET {column} = {first} WHERE id = {second}"),
			args: vec![value, Value::Uuid(id.uuid())],
		})
		.await
		.unwrap();
}

fn auth(db: &Db) -> Auth {
	Auth::builder(db, Keys::single("live", [5; 32]))
		.mailer(leptoskit_auth::Mailbox::new())
		.dev_sign_in()
		.origin(ORIGIN)
		.build()
		.unwrap()
}

async fn sign_in(auth: &Auth, bearer: bool) -> SignIn {
	auth.dev_sign_in(
		"ada@live.example",
		&RequestInfo {
			transport: if bearer {
				Transport::Bearer
			} else {
				Transport::Cookie
			},
			..RequestInfo::default()
		},
	)
	.await
	.unwrap()
}

fn header(sign_in: &SignIn, bearer: bool) -> (&'static str, String) {
	if bearer {
		(
			"authorization",
			format!("Bearer {}", sign_in.token.expose()),
		)
	} else {
		(
			"cookie",
			format!("__Host-session={}", sign_in.token.expose()),
		)
	}
}

enum Connection {
	Sse {
		body: Body,
		buffered: String,
	},
	Socket {
		socket: Box<WebSocketStream<MaybeTlsStream<TcpStream>>>,
		server: tokio::task::JoinHandle<()>,
	},
}

impl Drop for Connection {
	fn drop(&mut self) {
		if let Self::Socket { server, .. } = self {
			server.abort();
		}
	}
}

async fn next_event(body: &mut Body, buffered: &mut String) -> Option<ServerFrame> {
	loop {
		if let Some(end) = buffered.find("\n\n") {
			let event: String = buffered.drain(..end + 2).collect();
			if let Some(data) = event.lines().find_map(|line| line.strip_prefix("data: ")) {
				return Some(serde_json::from_str(data).unwrap());
			}
			continue;
		}
		let frame = body.frame().await?.unwrap();
		if let Ok(bytes) = frame.into_data() {
			buffered.push_str(std::str::from_utf8(&bytes).unwrap());
		}
	}
}

impl Connection {
	async fn open(live: &Live, auth: &Auth, sign_in: &SignIn, bearer: bool, socket: bool) -> Self {
		let router = Router::new().merge(live.router()).layer(auth.layer());
		let (name, value) = header(sign_in, bearer);
		let mut connection = if socket {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let address = listener.local_addr().unwrap();
			let server = tokio::spawn(async move {
				axum::serve(listener, router).await.unwrap();
			});
			let mut request = format!("ws://{address}/live")
				.into_client_request()
				.unwrap();
			request
				.headers_mut()
				.insert("origin", ORIGIN.parse().unwrap());
			request.headers_mut().insert(name, value.parse().unwrap());
			let (socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
			Self::Socket {
				socket: Box::new(socket),
				server,
			}
		} else {
			let response = router
				.oneshot(
					Request::get("/live/events")
						.header("origin", ORIGIN)
						.header(name, value)
						.body(Body::empty())
						.unwrap(),
				)
				.await
				.unwrap();
			assert_eq!(response.status(), StatusCode::OK);
			Self::Sse {
				body: response.into_body(),
				buffered: String::new(),
			}
		};
		assert!(matches!(
			connection.next().await,
			Some(ServerFrame::Welcome { .. })
		));
		connection
	}

	async fn next(&mut self) -> Option<ServerFrame> {
		tokio::time::timeout(WAIT, async {
			match self {
				Self::Socket { socket, .. } => {
					loop {
						match socket.next().await? {
							Ok(Message::Text(text)) => {
								return Some(serde_json::from_str(&text).unwrap());
							}
							Ok(Message::Ping(bytes)) => {
								socket.send(Message::Pong(bytes)).await.unwrap();
							}
							Ok(Message::Close(_)) | Err(_) => return None,
							_ => {}
						}
					}
				}
				Self::Sse { body, buffered } => next_event(body, buffered).await,
			}
		})
		.await
		.expect("the connection must answer within five seconds")
	}

	async fn refused(&mut self) {
		loop {
			match self
				.next()
				.await
				.expect("a refusal before the connection closes")
			{
				ServerFrame::AuthStatus { refused: true, .. } => break,
				ServerFrame::AuthStatus { expiring: true, .. } | ServerFrame::Transition(_) => {}
				frame => panic!("unexpected frame: {frame:?}"),
			}
		}
		assert!(self.next().await.is_none(), "refused connections close");
	}
}

#[tokio::test]
async fn sign_out_ends_idle_cookie_and_bearer_connections_on_both_transports() {
	let cluster = PostgresCluster::start();
	for db in databases(cluster.as_ref()).await {
		let auth = auth(&db);
		let live = Live::new(db.clone()).identify(auth.clone()).origin(ORIGIN);
		for socket in [false, true] {
			for bearer in [false, true] {
				let sign_in = sign_in(&auth, bearer).await;
				let mut connection = Connection::open(&live, &auth, &sign_in, bearer, socket).await;
				auth.sign_out(&sign_in.token, &RequestInfo::default())
					.await
					.unwrap();
				connection.refused().await;
			}
		}
	}
}

#[tokio::test]
async fn disabling_an_account_ends_its_open_connections() {
	let cluster = PostgresCluster::start();
	for db in databases(cluster.as_ref()).await {
		for socket in [false, true] {
			let auth = auth(&db);
			let sign_in = sign_in(&auth, false).await;
			let live = Live::new(db.clone()).identify(auth.clone()).origin(ORIGIN);
			let mut connection = Connection::open(&live, &auth, &sign_in, false, socket).await;
			set(
				&db,
				"accounts",
				"disabled_at",
				Value::Timestamp(leptoskit_db::now()),
				sign_in.session.account_id,
			)
			.await;
			connection.refused().await;
			auth.enable(sign_in.session.account_id).await.unwrap();
		}
	}
}

#[tokio::test]
async fn expiration_ends_an_idle_connection_without_another_write() {
	let cluster = PostgresCluster::start();
	for db in databases(cluster.as_ref()).await {
		let auth = auth(&db);
		let live = Live::new(db.clone()).identify(auth.clone()).origin(ORIGIN);
		for socket in [false, true] {
			for bearer in [false, true] {
				let sign_in = sign_in(&auth, bearer).await;
				let mut connection = Connection::open(&live, &auth, &sign_in, bearer, socket).await;
				let expires = leptoskit_db::now() + chrono::TimeDelta::milliseconds(300);
				set(
					&db,
					"sessions",
					"expires_at",
					Value::Timestamp(expires),
					sign_in.session.id,
				)
				.await;
				connection.refused().await;
			}
		}
	}
}

#[tokio::test]
async fn an_invalid_reauthentication_closes_the_previous_identity() {
	let db = leptoskit_db::testing::sqlite().await;
	let auth = auth(&db);
	let sign_in = sign_in(&auth, true).await;
	assert!(auth.identify_token("invalid").await.is_err());
	let live = Live::new(db).identify(auth.clone()).origin(ORIGIN);
	let mut connection = Connection::open(&live, &auth, &sign_in, true, true).await;
	let Connection::Socket { socket, .. } = &mut connection else {
		unreachable!()
	};
	socket
		.send(Message::Text(
			serde_json::to_string(&ClientFrame::Auth {
				token: "invalid".to_owned(),
			})
			.unwrap()
			.into(),
		))
		.await
		.unwrap();
	connection.refused().await;
}

#[tokio::test]
async fn a_revoked_session_cannot_get_a_new_lease_but_another_device_can() {
	let db = leptoskit_db::testing::sqlite().await;
	let auth = auth(&db);
	let one = sign_in(&auth, true).await;
	let two = sign_in(&auth, true).await;
	let identity = auth.identify_token(one.token.expose()).await.unwrap();
	let other = auth.identify_token(two.token.expose()).await.unwrap();
	assert_ne!(
		auth.lease(&identity).await.unwrap().reads,
		[] as [leptoskit_db::ReadEntry; 0]
	);
	auth.revoke(&two.signed_in(), one.session.id, &RequestInfo::default())
		.await
		.unwrap();
	assert!(auth.lease(&identity).await.is_err());
	assert!(auth.identify_token(one.token.expose()).await.is_err());
	assert!(auth.lease(&other).await.is_ok());
}

static ACTION_CASE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static ACTION_ENTERED: tokio::sync::Notify = tokio::sync::Notify::const_new();
static ACTION_RELEASE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(0);

/// Holds a private result until the connection has changed its identity.
#[leptoskit_rpc::action]
async fn lease_delayed(cx: &leptoskit_rpc::ActionCx) -> leptoskit_rpc::Result<String> {
	let caller = cx.identity().caller.clone().unwrap();
	ACTION_ENTERED.notify_one();
	ACTION_RELEASE.acquire().await.unwrap().forget();
	Ok(caller)
}

#[tokio::test]
async fn an_old_identitys_action_result_is_not_sent_to_the_new_identity() {
	let _case = ACTION_CASE.lock().await;
	let db = leptoskit_db::testing::sqlite().await;
	let auth = auth(&db);
	let ada = sign_in(&auth, true).await;
	let grace = auth
		.dev_sign_in(
			"grace@live.example",
			&RequestInfo {
				transport: Transport::Bearer,
				..RequestInfo::default()
			},
		)
		.await
		.unwrap();
	let live = Live::new(db).identify(auth.clone()).origin(ORIGIN);
	let mut connection = Connection::open(&live, &auth, &ada, true, true).await;
	let Connection::Socket { socket, .. } = &mut connection else {
		unreachable!()
	};
	socket
		.send(Message::Text(
			serde_json::to_string(&ClientFrame::Act {
				request: 7,
				name: "lease_delayed".to_owned(),
				args: serde_json::json!({}),
			})
			.unwrap()
			.into(),
		))
		.await
		.unwrap();
	tokio::time::timeout(WAIT, ACTION_ENTERED.notified())
		.await
		.unwrap();
	socket
		.send(Message::Text(
			serde_json::to_string(&ClientFrame::Auth {
				token: grace.token.expose().to_owned(),
			})
			.unwrap()
			.into(),
		))
		.await
		.unwrap();
	loop {
		if let Some(ServerFrame::AuthStatus {
			refused: false,
			expiring: false,
		}) = connection.next().await
		{
			break;
		}
	}
	ACTION_RELEASE.add_permits(1);
	loop {
		if let Some(ServerFrame::Done {
			request: 7,
			ok,
			err,
			..
		}) = connection.next().await
		{
			assert!(ok.is_none());
			assert_eq!(err.unwrap().status, 401);
			break;
		}
	}
}

#[tokio::test]
async fn session_revocations_reach_relay_consumers_without_database_fallback() {
	let cluster = PostgresCluster::start();
	for db in databases(cluster.as_ref()).await {
		let Some(nats) = nats::NatsServer::start() else {
			return;
		};
		let auth = auth(&db);
		let sign_in = sign_in(&auth, true).await;
		let settings = leptoskit_rpc::server::RelaySettings {
			stall: Duration::from_secs(120),
			..Default::default()
		};
		let relay = leptoskit_rpc::server::Relay::connect_with(&nats.url(), settings)
			.await
			.unwrap();
		let publisher = relay.publish(&db);
		tokio::time::timeout(WAIT, async {
			while !publisher.stats().following {
				tokio::time::sleep(Duration::from_millis(10)).await;
			}
		})
		.await
		.unwrap();
		let live = Live::new(db.clone())
			.identify(auth.clone())
			.origin(ORIGIN)
			.relay(&relay);
		let mut connection = Connection::open(&live, &auth, &sign_in, true, true).await;
		let Connection::Socket { socket, .. } = &mut connection else {
			unreachable!()
		};
		socket
			.send(Message::Text(
				serde_json::to_string(&ClientFrame::Auth {
					token: sign_in.token.expose().to_owned(),
				})
				.unwrap()
				.into(),
			))
			.await
			.unwrap();
		loop {
			if let Some(ServerFrame::AuthStatus {
				refused: false,
				expiring: false,
			}) = connection.next().await
			{
				break;
			}
		}
		auth.sign_out_everywhere(sign_in.session.account_id, &RequestInfo::default())
			.await
			.unwrap();
		connection.refused().await;
		assert_eq!(live.stats().relay_fills, 0);
	}
}

#[tokio::test]
async fn short_token_lifetimes_warn_in_the_future_and_absolute_expiry_cannot_loop_refreshes() {
	let db = leptoskit_db::testing::sqlite().await;
	let auth = Auth::builder(&db, Keys::single("short", [5; 32]))
		.mailer(leptoskit_auth::Mailbox::new())
		.dev_sign_in()
		.sessions(leptoskit_auth::SessionLifetimes {
			access: Duration::from_secs(20),
			..Default::default()
		})
		.build()
		.unwrap();
	let sign_in = sign_in(&auth, true).await;
	let identity = auth.identify_token(sign_in.token.expose()).await.unwrap();
	let lease = auth.lease(&identity).await.unwrap();
	let refresh = lease.refresh_at.unwrap();
	assert!(refresh > std::time::SystemTime::now());
	assert!(refresh < lease.expires_at.unwrap());
	set(
		&db,
		"sessions",
		"expires_at",
		Value::Timestamp(leptoskit_db::now() + chrono::TimeDelta::seconds(10)),
		sign_in.session.id,
	)
	.await;
	assert!(auth.lease(&identity).await.unwrap().refresh_at.is_none());
}

struct Unavailable(Auth);

impl Identify for Unavailable {
	fn identify<'a>(
		&'a self,
		request: &'a axum::http::request::Parts,
	) -> std::pin::Pin<
		Box<dyn Future<Output = leptoskit_rpc::Result<leptoskit_rpc::Identity>> + Send + 'a>,
	> {
		self.0.identify(request)
	}

	fn lease<'a>(
		&'a self,
		_identity: &'a leptoskit_rpc::Identity,
	) -> std::pin::Pin<
		Box<
			dyn Future<Output = leptoskit_rpc::Result<leptoskit_rpc::server::IdentityLease>>
				+ Send
				+ 'a,
		>,
	> {
		Box::pin(std::future::ready(Err(leptoskit_rpc::Error::server(
			"temporarily unavailable",
		))))
	}
}

#[tokio::test]
async fn a_failed_validation_read_reconnects_without_reporting_a_sign_out() {
	let db = leptoskit_db::testing::sqlite().await;
	let auth = auth(&db);
	let sign_in = sign_in(&auth, true).await;
	let live = Live::new(db)
		.identify(Unavailable(auth.clone()))
		.origin(ORIGIN);
	let mut connection = Connection::open(&live, &auth, &sign_in, true, true).await;
	assert!(matches!(
		connection.next().await,
		Some(ServerFrame::Reconnect { .. })
	));
	assert!(connection.next().await.is_none());
}

#[tokio::test]
async fn reauthenticating_the_same_identity_keeps_its_pending_action() {
	let _case = ACTION_CASE.lock().await;
	let db = leptoskit_db::testing::sqlite().await;
	let auth = auth(&db);
	let sign_in = sign_in(&auth, true).await;
	let live = Live::new(db).identify(auth.clone()).origin(ORIGIN);
	let mut connection = Connection::open(&live, &auth, &sign_in, true, true).await;
	let Connection::Socket { socket, .. } = &mut connection else {
		unreachable!()
	};
	socket
		.send(Message::Text(
			serde_json::to_string(&ClientFrame::Act {
				request: 7,
				name: "lease_delayed".to_owned(),
				args: serde_json::json!({}),
			})
			.unwrap()
			.into(),
		))
		.await
		.unwrap();
	tokio::time::timeout(WAIT, ACTION_ENTERED.notified())
		.await
		.unwrap();
	socket
		.send(Message::Text(
			serde_json::to_string(&ClientFrame::Auth {
				token: sign_in.token.expose().to_owned(),
			})
			.unwrap()
			.into(),
		))
		.await
		.unwrap();
	loop {
		if let Some(ServerFrame::AuthStatus {
			refused: false,
			expiring: false,
		}) = connection.next().await
		{
			break;
		}
	}
	ACTION_RELEASE.add_permits(1);
	loop {
		if let Some(ServerFrame::Done {
			request: 7,
			ok,
			err,
			..
		}) = connection.next().await
		{
			assert!(err.is_none());
			assert_eq!(
				ok.unwrap(),
				serde_json::json!(sign_in.session.id.to_string())
			);
			break;
		}
	}
}

struct Roles {
	one: String,
}

impl Identify for Roles {
	fn identify<'a>(
		&'a self,
		request: &'a axum::http::request::Parts,
	) -> std::pin::Pin<
		Box<dyn Future<Output = leptoskit_rpc::Result<leptoskit_rpc::Identity>> + Send + 'a>,
	> {
		let one = request
			.headers
			.get("authorization")
			.and_then(|value| value.to_str().ok())
			.is_some_and(|value| value.strip_prefix("Bearer ") == Some(self.one.as_str()));
		Box::pin(std::future::ready(Ok(leptoskit_rpc::Identity::new(
			leptoskit_db::Context::new().with(if one { "one" } else { "two" }.to_owned()),
		)
		.with_caller("same caller"))))
	}

	fn lease<'a>(
		&'a self,
		identity: &'a leptoskit_rpc::Identity,
	) -> std::pin::Pin<
		Box<
			dyn Future<Output = leptoskit_rpc::Result<leptoskit_rpc::server::IdentityLease>>
				+ Send
				+ 'a,
		>,
	> {
		Box::pin(std::future::ready(Ok(
			leptoskit_rpc::server::IdentityLease {
				continuity: identity.context.get::<String>().cloned(),
				..Default::default()
			},
		)))
	}
}

/// A policy-relevant context value, with no database reads to separate it.
#[leptoskit_rpc::query]
async fn leased_role(cx: &leptoskit_rpc::QueryCx) -> leptoskit_rpc::Result<String> {
	Ok(cx.get::<String>().cloned().unwrap())
}

async fn role(connection: &mut Connection) -> String {
	let Connection::Socket { socket, .. } = connection else {
		unreachable!()
	};
	socket
		.send(Message::Text(
			serde_json::to_string(&ClientFrame::Sub {
				id: 1,
				query: "leased_role".to_owned(),
				args: serde_json::json!({}),
			})
			.unwrap()
			.into(),
		))
		.await
		.unwrap();
	loop {
		if let Some(ServerFrame::Done {
			request: 1,
			ok,
			err,
			..
		}) = connection.next().await
		{
			assert!(err.is_none());
			return ok.unwrap().as_str().unwrap().to_owned();
		}
	}
}

#[tokio::test]
async fn the_same_caller_with_different_policy_revisions_does_not_share_results() {
	let db = leptoskit_db::testing::sqlite().await;
	let auth = auth(&db);
	let one = sign_in(&auth, true).await;
	let two = sign_in(&auth, true).await;
	let live = Live::new(db)
		.identify(Roles {
			one: one.token.expose().to_owned(),
		})
		.origin(ORIGIN);
	let mut first = Connection::open(&live, &auth, &one, true, true).await;
	let mut second = Connection::open(&live, &auth, &two, true, true).await;
	assert_eq!(role(&mut first).await, "one");
	assert_eq!(role(&mut second).await, "two");
}
