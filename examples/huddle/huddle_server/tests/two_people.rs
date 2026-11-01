//! Two people use Huddle at once, against the real server on SQLite,
//! through the same wire protocol every client speaks: `POST /api/<name>`
//! with the session cookie, and the `/live` WebSocket.
//!
//! Ada signs in first, so she's the admin. Grace signs in second. Between
//! them they check every primitive the example shows: a message reaching
//! the other socket with the unread count in the same transition, a private
//! channel appearing and vanishing with a membership, typing signals and
//! presence, an invite job finishing with an inbox notice, a failed tip
//! offering Retry and then settling, and signing out everywhere reaching an
//! open socket.

use std::time::Duration;

use futures::SinkExt;
use futures::StreamExt;
use huddle_api::services;
use huddle_server::Options;
use leptoskit_auth::Auth;
use leptoskit_auth::Keys;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::live::apply;
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpStream;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const WAIT: Duration = Duration::from_secs(30);

/// Someone signed in: their cookie, and the server's address.
struct Person {
	http: reqwest::Client,
	base: String,
	cookie: String,
}

impl Person {
	async fn sign_in(base: &str, email: &str) -> Self {
		let http = reqwest::Client::new();
		let response = http
			.post(format!("{base}/auth/dev/sign-in"))
			.header("origin", base)
			.header("content-type", "application/json")
			.body(json!({ "email": email }).to_string())
			.send()
			.await
			.unwrap();
		assert!(response.status().is_success(), "{}", response.status());
		let cookie = response
			.headers()
			.get("set-cookie")
			.and_then(|value| value.to_str().ok())
			.and_then(|value| value.split(';').next())
			.expect("a session cookie")
			.to_owned();
		Self {
			http,
			base: base.to_owned(),
			cookie,
		}
	}

	/// Calls an operation, as every client does.
	async fn call(&self, name: &str, args: Value) -> Result<Value, String> {
		let response = self
			.http
			.post(format!("{}/api/{name}", self.base))
			.header("origin", &self.base)
			.header("cookie", &self.cookie)
			.header("content-type", "application/json")
			.body(args.to_string())
			.send()
			.await
			.unwrap();
		let status = response.status();
		let text = response.text().await.unwrap();

		if status.is_success() {
			Ok(serde_json::from_str(&text).unwrap())
		} else {
			Err(format!("{status}: {text}"))
		}
	}

	async fn ok(&self, name: &str, args: Value) -> Value {
		self.call(name, args)
			.await
			.unwrap_or_else(|error| panic!("{name}: {error}"))
	}

	async fn socket(&self) -> Socket {
		let url = format!("{}/live", self.base.replace("http://", "ws://"));
		let mut request = url.into_client_request().unwrap();
		request
			.headers_mut()
			.insert("cookie", self.cookie.parse().unwrap());
		let (stream, _) = tokio_tungstenite::connect_async(request).await.unwrap();
		let mut socket = Socket {
			stream,
			results: std::collections::HashMap::new(),
		};
		let ServerFrame::Welcome { .. } = socket.next().await else {
			panic!("welcome first");
		};
		socket
	}
}

/// One live connection, and the result of each subscription so far.
struct Socket {
	stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
	results: std::collections::HashMap<u32, Value>,
}

impl Socket {
	async fn send(&mut self, frame: ClientFrame) {
		let text = serde_json::to_string(&frame).unwrap();
		self.stream.send(Message::Text(text.into())).await.unwrap();
	}

	async fn next(&mut self) -> ServerFrame {
		loop {
			let message = tokio::time::timeout(WAIT, self.stream.next())
				.await
				.expect("a frame arrives in time")
				.expect("the socket stays open")
				.unwrap();

			if let Message::Text(text) = message {
				let frame: ServerFrame = serde_json::from_str(&text).unwrap();

				if let ServerFrame::Transition(transition) = &frame {
					for update in &transition.updates {
						let value = self.results.entry(update.id).or_insert(Value::Null);
						apply(value, &update.update);
					}
				}

				return frame;
			}
		}
	}

	/// Subscribes, and returns the first value.
	async fn subscribe(&mut self, id: u32, query: &str, args: Value) -> Value {
		self.send(ClientFrame::Sub {
			id,
			query: query.to_owned(),
			args,
		})
		.await;

		loop {
			if let ServerFrame::Done {
				request, ok, err, ..
			} = self.next().await
				&& request == id
			{
				assert_eq!(err, None, "{query}");
				let value = ok.unwrap();
				self.results.insert(id, value.clone());
				return value;
			}
		}
	}

	/// Reads frames until subscription `id`'s result satisfies `check`.
	async fn until(&mut self, id: u32, check: impl Fn(&Value) -> bool) -> Value {
		let deadline = tokio::time::Instant::now() + WAIT;

		loop {
			if let Some(value) = self.results.get(&id)
				&& check(value)
			{
				return value.clone();
			}

			assert!(
				tokio::time::Instant::now() < deadline,
				"subscription {id} never got there: {:?}",
				self.results.get(&id)
			);
			let _ = self.next().await;
		}
	}

	/// Reads frames until one matches `pick`.
	async fn frame<T>(&mut self, pick: impl Fn(&ServerFrame) -> Option<T>) -> T {
		loop {
			let frame = self.next().await;

			if let Some(found) = pick(&frame) {
				return found;
			}
		}
	}
}

fn has_name(value: &Value, name: &str) -> bool {
	value
		.as_array()
		.is_some_and(|rows| rows.iter().any(|row| row["name"] == name))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[expect(clippy::too_many_lines, reason = "one story, told in order")]
async fn two_people_huddle() {
	let db = leptoskit_db::testing::sqlite_migrated(huddle_db::MIGRATIONS).await;
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let base = format!("http://{}", listener.local_addr().unwrap());
	let mut options = Options::new(&db);
	options.origin = Some(base.clone());
	options.mail_delay = Duration::from_millis(200);
	options.settlement.delay = Duration::from_millis(200);
	// Every tip is declined, until the test changes its mind.
	options.settlement.failure_rate = 1.0;
	options.jobs = leptoskit_rpc::server::Jobs::new(db.clone())
		.poll(Duration::from_millis(100))
		.queue("mail", 2)
		.queue("settlement", 2);
	let auth = Auth::builder(&db, Keys::single("test", [3; 32])).origin(&base);
	let huddle = huddle_server::start(&db, auth, options).unwrap();
	let mailbox = huddle.mailbox.clone();
	let router = huddle.router.clone();
	let server = tokio::spawn(async move {
		leptoskit::server::serve_listener(listener, router)
			.await
			.unwrap();
	});

	let ada = Person::sign_in(&base, "ada@huddle.test").await;
	let grace = Person::sign_in(&base, "grace.hopper@huddle.test").await;

	let me = ada.ok("whoami", json!({})).await;
	assert_eq!(me["name"], "Ada");
	assert_eq!(me["is_admin"], true, "the first person in is the admin");
	let grace_me = grace.ok("whoami", json!({})).await;
	assert_eq!(grace_me["name"], "Grace Hopper");
	assert_eq!(grace_me["is_admin"], false);
	let grace_id = grace_me["id"].clone();
	let ada_id = me["id"].clone();

	let channels = ada.ok("channels", json!({})).await;
	assert!(has_name(&channels, "general"), "{channels}");
	let general = channels[0]["id"].clone();

	// --- A message reaches the other socket, with its unread count. ---
	let mut grace_live = grace.socket().await;
	grace_live
		.subscribe(1, "messages", json!({ "channel": general, "limit": 50 }))
		.await;
	let sidebar = grace_live.subscribe(2, "channels", json!({})).await;
	assert_eq!(sidebar[0]["unread"], 0);

	let sent = ada
		.ok(
			"send_message",
			json!({ "channel": general, "body": "hello, Grace", "client_key": "ada-1" }),
		)
		.await;
	// Sending the same key again, as a client does when the answer is lost,
	// stores one message.
	let again = ada
		.ok(
			"send_message",
			json!({ "channel": general, "body": "hello, Grace", "client_key": "ada-1" }),
		)
		.await;
	assert_eq!(sent, again);

	let transition = grace_live
		.frame(|frame| {
			match frame {
				ServerFrame::Transition(transition) => Some(transition.clone()),
				_ => None,
			}
		})
		.await;
	let ids: Vec<u32> = transition.updates.iter().map(|update| update.id).collect();
	assert!(
		ids.contains(&1) && ids.contains(&2),
		"the message and the unread count arrive in one transition: {ids:?}"
	);
	let messages = grace_live.results[&1].clone();
	assert_eq!(messages.as_array().unwrap().len(), 1);
	assert_eq!(messages[0]["body"], "hello, Grace");
	assert_eq!(messages[0]["author"], "Ada");
	assert_eq!(grace_live.results[&2][0]["unread"], 1);

	// --- A private channel appears with a membership, and vanishes. ---
	let launch = ada
		.ok(
			"create_channel",
			json!({ "name": "launch", "private": true, "members": [] }),
		)
		.await;
	assert!(
		!has_name(&grace.ok("channels", json!({})).await, "launch"),
		"Grace can't see a private channel she isn't in"
	);
	ada.ok("add_member", json!({ "channel": launch, "user": grace_id }))
		.await;
	grace_live
		.until(2, |sidebar| has_name(sidebar, "launch"))
		.await;
	ada.ok(
		"remove_member",
		json!({ "channel": launch, "user": grace_id }),
	)
	.await;
	grace_live
		.until(2, |sidebar| !has_name(sidebar, "launch"))
		.await;
	assert!(
		grace
			.call("messages", json!({ "channel": launch, "limit": 50 }))
			.await
			.unwrap()
			.as_array()
			.unwrap()
			.is_empty(),
		"once removed, its messages are gone too"
	);

	// --- Typing and presence pass between them. ---
	let mut ada_live = ada.socket().await;
	let room = serde_json::to_string(&json!({ "channel": general })).unwrap();
	ada_live
		.send(ClientFrame::Join {
			channel: room.clone(),
			name: "room".into(),
			args: json!({ "channel": general }),
			state: json!({ "user": ada_id, "name": "Ada" }),
		})
		.await;
	grace_live
		.send(ClientFrame::Join {
			channel: room.clone(),
			name: "room".into(),
			args: json!({ "channel": general }),
			state: json!({ "user": grace_id, "name": "Grace Hopper" }),
		})
		.await;
	let here = ada_live
		.frame(|frame| {
			match frame {
				ServerFrame::Presence { members, .. } if members.len() == 2 => {
					Some(members.clone())
				}
				_ => None,
			}
		})
		.await;
	let names: Vec<&Value> = here.iter().map(|member| &member.state["name"]).collect();
	assert!(names.contains(&&json!("Ada")) && names.contains(&&json!("Grace Hopper")));

	ada_live
		.send(ClientFrame::Signal {
			channel: room.clone(),
			message: json!({ "type": "typing", "user": ada_id, "name": "Ada" }),
		})
		.await;
	let typing = grace_live
		.frame(|frame| {
			match frame {
				ServerFrame::Signal { message, .. } => Some(message.clone()),
				_ => None,
			}
		})
		.await;
	assert_eq!(typing["type"], "typing");
	assert_eq!(typing["name"], "Ada");

	// --- An invite: Sending…, then Email sent, then a notice. ---
	ada_live
		.subscribe(10, "invites", json!({ "channel": general }))
		.await;
	ada_live
		.subscribe(11, "rpc.inbox", json!({ "limit": 50 }))
		.await;
	ada.ok(
		"invite",
		json!({ "channel": general, "email": "lin@huddle.test" }),
	)
	.await;
	ada_live
		.until(10, |invites| invites[0]["status"] == "sending")
		.await;
	ada_live
		.until(10, |invites| invites[0]["status"] == "sent")
		.await;
	ada_live
		.until(11, |inbox| {
			inbox["notices"].as_array().is_some_and(|notices| {
				notices
					.iter()
					.any(|n| n["title"] == "Invite sent to lin@huddle.test")
			})
		})
		.await;
	assert!(
		mailbox
			.latest_to("lin@huddle.test")
			.is_some_and(|mail| mail.subject == "Ada invited you to #general on Huddle")
	);

	// --- A tip the settlement service declines offers Retry. ---
	ada_live.subscribe(12, "tips", json!({})).await;
	ada.ok("send_tip", json!({ "to": grace_id, "amount": 5 }))
		.await;
	ada_live
		.until(12, |tips| tips[0]["status"] == "submitted")
		.await;
	let failed = ada_live
		.until(12, |tips| tips[0]["status"] == "failed")
		.await;
	assert_eq!(
		failed[0]["reason"],
		"The settlement service declined the tip (simulated)"
	);
	let inbox = ada_live
		.until(11, |inbox| {
			inbox["notices"].as_array().is_some_and(|notices| {
				notices
					.iter()
					.any(|n| n["title"] == "Your tip to Grace Hopper failed")
			})
		})
		.await;
	let notice = inbox["notices"]
		.as_array()
		.unwrap()
		.iter()
		.find(|notice| notice["title"] == "Your tip to Grace Hopper failed")
		.unwrap()
		.clone();
	assert_eq!(notice["action"]["label"], "Retry");
	assert_eq!(notice["action"]["operation"], "retry_tip");

	// The service recovers; Retry runs the notice's own action.
	let mut recovered = (*services::get()).clone();
	recovered.settlement.failure_rate = 0.0;
	services::install(recovered);
	ada.ok("retry_tip", notice["action"]["arguments"].clone())
		.await;
	ada_live
		.until(12, |tips| tips[0]["status"] == "confirmed")
		.await;
	ada_live
		.until(11, |inbox| {
			inbox["notices"].as_array().is_some_and(|notices| {
				notices
					.iter()
					.any(|n| n["title"] == "Your tip to Grace Hopper went through")
			})
		})
		.await;

	// A job's own mutation can't be called from outside.
	let forged = grace
		.call(
			"set_tip_status",
			json!({ "tip": failed[0]["id"], "status": "confirmed", "receipt": null, "reason": null }),
		)
		.await;
	assert!(forged.unwrap_err().starts_with("403"));

	// --- Signing out everywhere reaches Grace's open socket. ---
	let session = grace_live.subscribe(20, "my_session", json!({})).await;
	assert_eq!(session["active"], true);
	assert!(
		grace
			.call("sign_out_everywhere", json!({ "user": ada_id }))
			.await
			.unwrap_err()
			.starts_with("403"),
		"only an admin signs people out"
	);
	ada.ok("sign_out_everywhere", json!({ "user": grace_id }))
		.await;
	loop {
		if matches!(grace_live.next().await, ServerFrame::AuthStatus { refused: true, .. }) {
			break;
		}
	}
	assert_eq!(grace.ok("whoami", json!({})).await, Value::Null);

	server.abort();
	huddle.shutdown().await;
}
