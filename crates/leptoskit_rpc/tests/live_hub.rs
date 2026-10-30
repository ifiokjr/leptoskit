//! The live hub: a subscription runs the query as its caller, every commit
//! that could affect it re-runs it, and every connection watching gets the
//! new value.

#![cfg(all(
	feature = "live",
	feature = "sqlite",
	any(feature = "postgres", feature = "sqlite")
))]

use std::time::Duration;

use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Expr;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_rpc::Identity;
use leptoskit_rpc::live::LiveHub;
use leptoskit_rpc::live::ServerFrame;
use serde_json::json;

/// A note every open screen sees.
#[table(name = "hub_notes", live, public)]
#[policy(select, insert, |_, _| Expr::always())]
pub struct Note {
	pub id: Id<Self>,
	pub title: String,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// Every note, oldest first.
#[leptoskit_rpc::query(crate = "leptoskit_rpc")]
pub async fn hub_notes(cx: &leptoskit_rpc::QueryCx) -> leptoskit_rpc::Result<Vec<PublicNote>> {
	let rows = Note::query()
		.order_by(|note| note.created_at.asc())
		.all(cx.db())
		.await?;
	Ok(rows.into_iter().map(Into::into).collect())
}

/// Notes whose title matches, to prove a write to another row sends nothing.
#[leptoskit_rpc::query(name = "wanted_notes", crate = "leptoskit_rpc")]
pub async fn wanted(cx: &leptoskit_rpc::QueryCx) -> leptoskit_rpc::Result<Vec<PublicNote>> {
	let rows = Note::query()
		.filter(|note| note.title.eq("wanted"))
		.all(cx.db())
		.await?;
	Ok(rows.into_iter().map(Into::into).collect())
}

/// Who a private note belongs to.
#[derive(Clone, Debug)]
pub struct Viewer(pub String);

/// A note only its author sees.
#[table(name = "hub_private_notes", live, public)]
#[policy(select, insert, |note, cx| cx.global(|me: &Viewer| note.author.eq(me.0.clone())))]
pub struct PrivateNote {
	pub id: Id<Self>,
	pub author: String,
	pub title: String,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// The signed-in person's notes.
#[leptoskit_rpc::query(crate = "leptoskit_rpc")]
pub async fn private_notes(
	cx: &leptoskit_rpc::QueryCx,
) -> leptoskit_rpc::Result<Vec<PublicPrivateNote>> {
	let rows = PrivateNote::query()
		.order_by(|note| note.created_at.asc())
		.all(cx.db())
		.await?;
	Ok(rows.into_iter().map(Into::into).collect())
}

/// Adds a note as the signed-in person.
#[leptoskit_rpc::mutation(crate = "leptoskit_rpc")]
pub async fn add_private_note(
	cx: &leptoskit_rpc::MutationCx,
	title: String,
) -> leptoskit_rpc::Result<Id<PrivateNote>> {
	let viewer = cx.require::<Viewer>()?;
	let note = NewPrivateNote {
		author: viewer.0.clone(),
		title,
	}
	.insert(cx.tx())
	.await?;
	Ok(note.id)
}

/// Adds a note.
#[leptoskit_rpc::mutation(crate = "leptoskit_rpc")]
pub async fn add_note(
	cx: &leptoskit_rpc::MutationCx,
	title: String,
) -> leptoskit_rpc::Result<Id<Note>> {
	let note = NewNote { title }.insert(cx.tx()).await?;
	Ok(note.id)
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

async fn next(connection: &mut tokio::sync::mpsc::UnboundedReceiver<ServerFrame>) -> ServerFrame {
	tokio::time::timeout(Duration::from_secs(2), connection.recv())
		.await
		.expect("a frame")
		.expect("the connection is open")
}

#[tokio::test]
async fn a_write_reaches_every_connection_watching_the_query() {
	let db = database().await;
	let hub = LiveHub::new(db.clone());

	let (ada, mut ada_frames) = hub.connect(Identity::anonymous()).await;
	let (grace, mut grace_frames) = hub.connect(Identity::anonymous()).await;

	assert!(matches!(
		next(&mut ada_frames).await,
		ServerFrame::Welcome { .. }
	));
	assert!(matches!(
		next(&mut grace_frames).await,
		ServerFrame::Welcome { .. }
	));

	assert_eq!(
		hub.subscribe(&ada, 1, "hub_notes", json!({}))
			.await
			.unwrap(),
		json!([])
	);
	assert_eq!(
		hub.subscribe(&grace, 1, "hub_notes", json!({}))
			.await
			.unwrap(),
		json!([])
	);

	let initial = next(&mut ada_frames).await;
	assert!(matches!(initial, ServerFrame::Transition { to: 0, .. }));
	assert!(matches!(
		next(&mut grace_frames).await,
		ServerFrame::Transition { to: 0, .. }
	));

	hub.mutate(&ada, "request-1", "add_note", json!({ "title": "hi" }))
		.await
		.unwrap();
	assert!(matches!(
		next(&mut ada_frames).await,
		ServerFrame::Done { .. }
	));

	assert_eq!(hub.poll().await.unwrap(), 2);

	for frames in [&mut ada_frames, &mut grace_frames] {
		match next(frames).await {
			ServerFrame::Transition { to, updates, .. } => {
				assert_eq!(to, 1);
				assert_eq!(updates.len(), 1);
				match &updates[0].outcome {
					leptoskit_rpc::live::UpdateOutcome::Patch { patch } => {
						assert_eq!(patch.upsert.len(), 1);
						assert_eq!(patch.upsert[0].value["title"], "hi");
					}
					leptoskit_rpc::live::UpdateOutcome::Value { value } => {
						assert_eq!(value.as_array().unwrap().len(), 1);
						assert_eq!(value[0]["title"], "hi");
					}
					leptoskit_rpc::live::UpdateOutcome::Error { error } => {
						panic!("the re-run failed: {error:?}")
					}
				}
			}
			other => panic!("expected a transition, got {other:?}"),
		}
	}
}

#[tokio::test]
async fn a_write_outside_the_read_set_sends_nothing() {
	let db = database().await;
	let hub = LiveHub::new(db.clone());
	let (client, mut frames) = hub.connect(Identity::anonymous()).await;
	assert!(matches!(
		next(&mut frames).await,
		ServerFrame::Welcome { .. }
	));

	hub.subscribe(&client, 1, "wanted_notes", json!({}))
		.await
		.unwrap();
	assert!(matches!(
		next(&mut frames).await,
		ServerFrame::Transition { .. }
	));

	hub.mutate(
		&client,
		"request-1",
		"add_note",
		json!({ "title": "other" }),
	)
	.await
	.unwrap();
	assert!(matches!(next(&mut frames).await, ServerFrame::Done { .. }));

	assert_eq!(hub.poll().await.unwrap(), 0);
	assert!(
		tokio::time::timeout(Duration::from_millis(100), frames.recv())
			.await
			.is_err(),
		"a row outside the filter sends no transition"
	);
}

#[tokio::test]
async fn unsubscribing_stops_the_updates() {
	let db = database().await;
	let hub = LiveHub::new(db);
	let (client, mut frames) = hub.connect(Identity::anonymous()).await;
	assert!(matches!(
		next(&mut frames).await,
		ServerFrame::Welcome { .. }
	));
	hub.subscribe(&client, 1, "hub_notes", json!({}))
		.await
		.unwrap();
	assert!(matches!(
		next(&mut frames).await,
		ServerFrame::Transition { .. }
	));

	hub.unsubscribe(&client, 1).await;
	hub.mutate(&client, "request-1", "add_note", json!({ "title": "hi" }))
		.await
		.unwrap();
	assert!(matches!(next(&mut frames).await, ServerFrame::Done { .. }));
	assert_eq!(hub.poll().await.unwrap(), 0);
}

#[tokio::test]
async fn a_policy_hides_another_persons_rows_from_the_transition() {
	let db = database().await;
	let hub = LiveHub::new(db);
	let ada = Identity::new(Context::new().with(Viewer("ada".into())));
	let grace = Identity::new(Context::new().with(Viewer("grace".into())));
	let (ada_connection, mut ada_frames) = hub.connect(ada).await;
	let (grace_connection, mut grace_frames) = hub.connect(grace).await;

	assert!(matches!(
		next(&mut ada_frames).await,
		ServerFrame::Welcome { .. }
	));
	assert!(matches!(
		next(&mut grace_frames).await,
		ServerFrame::Welcome { .. }
	));
	hub.subscribe(&ada_connection, 1, "private_notes", json!({}))
		.await
		.unwrap();
	hub.subscribe(&grace_connection, 1, "private_notes", json!({}))
		.await
		.unwrap();
	assert!(matches!(
		next(&mut ada_frames).await,
		ServerFrame::Transition { .. }
	));
	assert!(matches!(
		next(&mut grace_frames).await,
		ServerFrame::Transition { .. }
	));

	hub.mutate(
		&ada_connection,
		"request-1",
		"add_private_note",
		json!({ "title": "ada only" }),
	)
	.await
	.unwrap();
	assert!(matches!(
		next(&mut ada_frames).await,
		ServerFrame::Done { .. }
	));

	assert_eq!(hub.poll().await.unwrap(), 1);
	match next(&mut ada_frames).await {
		ServerFrame::Transition { updates, .. } => {
			assert_eq!(updates.len(), 1);
		}
		other => panic!("expected ada's transition, got {other:?}"),
	}
	assert!(
		tokio::time::timeout(Duration::from_millis(100), grace_frames.recv())
			.await
			.is_err(),
		"grace never receives ada's row"
	);
}
