//! The live socket: a browser or native client subscribes over `/live`, and
//! a write from anywhere reaches it as a transition.

#![cfg(all(feature = "live", feature = "sqlite"))]

use std::time::Duration;

use futures::SinkExt;
use futures::StreamExt;
use leptoskit_db::Db;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_rpc::live::ClientFrame;
use leptoskit_rpc::live::ServerFrame;
use leptoskit_rpc::server::Server;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

/// A note a client watches.
#[table(name = "socket_notes", live, public)]
#[policy(select, insert, |_, _| Expr::always())]
pub struct Note {
	pub id: Id<Self>,
	pub title: String,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// Every note.
#[leptoskit_rpc::query(crate = "leptoskit_rpc")]
pub async fn socket_notes(cx: &leptoskit_rpc::QueryCx) -> leptoskit_rpc::Result<Vec<PublicNote>> {
	let rows = Note::query()
		.order_by(|note| note.created_at.asc())
		.all(cx.db())
		.await?;
	Ok(rows.into_iter().map(Into::into).collect())
}

async fn database() -> Db {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	let schema = leptoskit_db::schema(leptoskit_db_schema::Dialect::Sqlite).unwrap();
	let empty = Schema::empty(leptoskit_db_schema::Dialect::Sqlite);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.statements.join(";\n"))
		.await
		.unwrap();
	db
}

fn frame(text: Message) -> ServerFrame {
	let Message::Text(text) = text else {
		panic!("expected a text frame");
	};
	serde_json::from_str(text.as_str()).expect("a server frame")
}

#[tokio::test]
async fn a_socket_receives_a_write_made_elsewhere() {
	let db = database().await;
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let server = Server::new(db.clone());
	let router = server.router::<()>();
	tokio::spawn(async move {
		axum::serve(listener, router).await.unwrap();
	});

	let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/live"))
		.await
		.expect("the socket opens");
	let welcome = tokio::time::timeout(Duration::from_secs(2), socket.next())
		.await
		.expect("a welcome")
		.expect("the socket is open")
		.expect("a frame");
	assert!(matches!(frame(welcome), ServerFrame::Welcome { .. }));

	socket
		.send(Message::Text(
			serde_json::to_string(&ClientFrame::Sub {
				query: 1,
				name: "socket_notes".into(),
				args: json!({}),
			})
			.unwrap()
			.into(),
		))
		.await
		.unwrap();
	let initial = tokio::time::timeout(Duration::from_secs(2), socket.next())
		.await
		.expect("an initial transition")
		.expect("the socket is open")
		.expect("a frame");
	assert!(matches!(
		frame(initial),
		ServerFrame::Transition { updates, .. } if updates.len() == 1
	));

	// Another machine, another writer: a direct insert, not through the socket.
	NewNote {
		title: "from elsewhere".into(),
	}
	.insert(&db)
	.await
	.unwrap();

	let transition = tokio::time::timeout(Duration::from_secs(2), socket.next())
		.await
		.expect("a transition")
		.expect("the socket is open")
		.expect("a frame");
	match frame(transition) {
		ServerFrame::Transition { updates, .. } => {
			assert_eq!(updates.len(), 1);
			let text = serde_json::to_string(&updates[0]).unwrap();
			assert!(text.contains("from elsewhere"), "{text}");
		}
		other => panic!("expected a transition, got {other:?}"),
	}

	socket.close(None).await.unwrap();
}
