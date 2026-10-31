//! The live hub end to end: a WebSocket subscribes to a query, a write
//! commits, and the connection receives the change as a transition, then a
//! mutation runs over the same socket.

#![cfg(all(feature = "live", feature = "sqlite"))]

use std::time::Duration;

use futures::SinkExt;
use futures::StreamExt;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::table;
use leptoskit_rpc as rpc;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::live::Update;
use leptoskit_rpc::server::Live;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

/// A note everyone may read.
#[table(name = "live_notes", live)]
pub struct Note {
	pub id: Id<Self>,
	pub body: String,
}

/// One note, as the query returns it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Title {
	pub id: String,
	pub body: String,
}

/// Every note, in the order they were written.
#[rpc::query]
pub async fn live_titles(cx: &rpc::QueryCx) -> rpc::Result<Vec<Title>> {
	let notes = Note::query()
		.order_by(|note| note.body.asc())
		.all(cx.db())
		.await?;
	Ok(notes
		.into_iter()
		.map(|note| {
			Title {
				id: note.id.to_string(),
				body: note.body,
			}
		})
		.collect())
}

/// Adds a note.
#[rpc::mutation]
pub async fn add_live_note(cx: &rpc::MutationCx, body: String) -> rpc::Result<String> {
	let note = NewNote { body }.insert(cx.tx()).await?;
	Ok(note.id.to_string())
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn start() -> (Db, String, JoinHandle<()>) {
	let db = leptoskit_db::testing::sqlite().await;
	let app = Live::new(db.clone()).router::<()>();
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let server = tokio::spawn(async move {
		axum::serve(listener, app).await.unwrap();
	});
	(db, format!("ws://{address}/live"), server)
}

async fn send(socket: &mut Socket, frame: ClientFrame) {
	let text = serde_json::to_string(&frame).unwrap();
	socket.send(Message::Text(text.into())).await.unwrap();
}

async fn next(socket: &mut Socket) -> ServerFrame {
	loop {
		let message = tokio::time::timeout(Duration::from_secs(20), socket.next())
			.await
			.expect("a frame arrives")
			.unwrap()
			.unwrap();
		match message {
			Message::Text(text) => return serde_json::from_str(&text).unwrap(),
			Message::Ping(_) | Message::Pong(_) => {}
			other => panic!("unexpected frame: {other:?}"),
		}
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subscribed_query_receives_each_write_as_a_patch() {
	let (db, url, server) = start().await;
	let (mut socket, _) = connect_async(&url).await.unwrap();

	let ServerFrame::Welcome {
		heartbeat_seconds,
		version,
		..
	} = next(&mut socket).await
	else {
		panic!("welcome first");
	};
	assert_eq!(heartbeat_seconds, 30);
	assert_eq!(version, 0);

	send(
		&mut socket,
		ClientFrame::Sub {
			id: 1,
			query: "live_titles".into(),
			args: json!({}),
		},
	)
	.await;
	let ServerFrame::Done {
		request, ok, err, ..
	} = next(&mut socket).await
	else {
		panic!("the subscription's first value");
	};
	assert_eq!(request, 1);
	assert_eq!(err, None);
	assert_eq!(ok.unwrap(), json!([]));

	let note = NewNote {
		body: "first".into(),
	}
	.insert(&db)
	.await
	.unwrap();

	let ServerFrame::Transition(transition) = next(&mut socket).await else {
		panic!("the write arrives as a transition");
	};
	assert_eq!(transition.from, 0);
	assert_eq!(transition.to, 1);
	assert_eq!(transition.updates.len(), 1);
	let update = &transition.updates[0];
	assert_eq!(update.id, 1);
	let Update::Patch(patch) = &update.update else {
		panic!("a list of rows patches, got {:?}", update.update);
	};
	assert_eq!(patch.remove, Vec::<String>::new());
	assert_eq!(patch.upsert.len(), 1);
	assert_eq!(patch.upsert[0].value["body"], "first");

	// A mutation over the same socket answers with the version it reached.
	send(
		&mut socket,
		ClientFrame::Mutate {
			request: 2,
			name: "add_live_note".into(),
			args: json!({ "body": "second" }),
			key: Some("key-1".into()),
		},
	)
	.await;
	let ServerFrame::Done {
		request,
		ok,
		err,
		version,
	} = next(&mut socket).await
	else {
		panic!("the mutation's answer");
	};
	assert_eq!(request, 2);
	assert_eq!(err, None);
	assert!(ok.unwrap().as_str().is_some());
	assert!(version >= 1);

	let ServerFrame::Transition(transition) = next(&mut socket).await else {
		panic!("the mutation's write arrives too");
	};
	assert_eq!(transition.updates.len(), 1);
	assert_eq!(note.body, "first");

	drop(socket);
	server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_subscription_is_refused_and_the_socket_stays_open() {
	let (_db, url, server) = start().await;
	let (mut socket, _) = connect_async(&url).await.unwrap();
	let _ = next(&mut socket).await;

	send(
		&mut socket,
		ClientFrame::Sub {
			id: 7,
			query: "missing".into(),
			args: json!({}),
		},
	)
	.await;
	let ServerFrame::Done {
		request, ok, err, ..
	} = next(&mut socket).await
	else {
		panic!("a refusal");
	};
	assert_eq!(request, 7);
	assert!(ok.is_none());
	assert!(err.unwrap().message.contains("no operation is named"));

	send(&mut socket, ClientFrame::Ping).await;
	assert_eq!(next(&mut socket).await, ServerFrame::Pong);

	server.abort();
}
