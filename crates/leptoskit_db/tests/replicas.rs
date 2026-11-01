//! Read replicas on a real streaming standby: reads stay on the primary
//! unless a handle asks for a replica; `on_replica_at` never picks one that
//! hasn't replayed past the position asked for, paused replay included;
//! and every commit the change log yields carries a position a replica must
//! reach to show it, with triggers and with replication capture.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use std::time::Duration;

use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::LiveCapture;
use leptoskit_db::Read;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use leptoskit_db::live::LogEntry;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use support::postgres::PostgresCluster;
use support::postgres::Standby;

#[table(name = "replica_notes", live)]
pub struct Note {
	pub id: Id<Self>,
	pub title: String,
}

async fn migrated(url: &str, capture: LiveCapture) -> Db {
	let schema = leptoskit_db::with_live_capture(
		leptoskit_db::schema(leptoskit_db_schema::Dialect::Postgres).unwrap(),
		capture,
	)
	.unwrap();
	let empty = Schema::empty(schema.dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let db = Db::connect(url).await.unwrap();
	db.run_script(&leptoskit_db_schema::render(&plan, &empty, &schema).sql())
		.await
		.unwrap();
	db
}

/// A primary with a live table, and the same database on a standby.
async fn primary_and_standby(capture: LiveCapture) -> Option<(PostgresCluster, Standby, Db)> {
	let cluster = PostgresCluster::start()?;
	let standby = cluster.standby();
	let url = cluster.database();
	let db = migrated(&url, capture)
		.await
		.with_replica(&standby.url(&url))
		.unwrap();
	Some((cluster, standby, db))
}

async fn titles(db: &Db) -> Vec<String> {
	Note::query()
		.order_by(|note| note.title.asc())
		.all(db)
		.await
		.unwrap()
		.into_iter()
		.map(|note| note.title)
		.collect()
}

async fn insert_lsn(db: &Db) -> i64 {
	let rows = db
		.fetch(Statement {
			sql: "SELECT (pg_current_wal_insert_lsn() - '0/0'::pg_lsn)::bigint".into(),
			args: Vec::new(),
		})
		.await
		.unwrap();
	match rows[0][0] {
		Value::Int(lsn) => lsn,
		ref other => panic!("not a position: {other:?}"),
	}
}

/// Waits until the standby shows `title`.
async fn replicated(db: &Db, title: &str) {
	let lsn = insert_lsn(db).await;
	let replica = db
		.on_replica_at(lsn, Duration::from_secs(30))
		.await
		.expect("the standby catches up");
	assert!(titles(&replica).await.contains(&title.to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_stay_on_the_primary_unless_asked() {
	let Some((_cluster, _standby, db)) = primary_and_standby(LiveCapture::Triggers).await else {
		return;
	};
	assert_eq!(db.replica_count(), 1);
	assert!(!db.is_on_replica());

	NewNote {
		title: "first".into(),
	}
	.insert(&db)
	.await
	.unwrap();
	replicated(&db, "first").await;
	let before = db.replica_stats()[0].reads;

	// An ordinary handle, and one acting for a request, read the primary.
	assert_eq!(titles(&db).await, ["first"]);
	assert_eq!(
		titles(&db.with_context(leptoskit_db::Context::new())).await,
		["first"]
	);
	assert_eq!(db.replica_stats()[0].reads, before);

	// Asking reads the standby, and writing from that handle still writes
	// to the primary, which a standby refuses.
	let replica = db.on_replica();
	assert!(replica.is_on_replica());
	assert_eq!(titles(&replica).await, ["first"]);
	assert!(db.replica_stats()[0].reads > before);
	NewNote {
		title: "second".into(),
	}
	.insert(&replica)
	.await
	.unwrap();
	assert_eq!(titles(&db.on_primary()).await, ["first", "second"]);
	assert!(
		db.replica_stats()[0]
			.name
			.starts_with("postgres://127.0.0.1:")
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replica_behind_the_position_is_never_picked() {
	let Some((_cluster, standby, db)) = primary_and_standby(LiveCapture::Triggers).await else {
		return;
	};
	NewNote {
		title: "before".into(),
	}
	.insert(&db)
	.await
	.unwrap();
	replicated(&db, "before").await;

	standby.pause_replay();
	NewNote {
		title: "while paused".into(),
	}
	.insert(&db)
	.await
	.unwrap();
	let lsn = insert_lsn(&db).await;

	// However long it waits, a paused standby stays behind.
	assert!(
		db.on_replica_at(lsn, Duration::from_millis(200))
			.await
			.is_none()
	);
	// It still serves reads that ask for nothing newer, without the write.
	let stale = db.on_replica();
	assert_eq!(titles(&stale).await, ["before"]);

	standby.resume_replay();
	let replica = db
		.on_replica_at(lsn, Duration::from_secs(30))
		.await
		.expect("the standby catches up once it replays");
	assert_eq!(titles(&replica).await, ["before", "while paused"]);
}

/// Every commit the log yields has a position past its own commit record:
/// a replica that has reached it shows the commit, with either capture.
async fn commits_carry_a_position_replicas_must_reach(capture: LiveCapture) {
	let Some((_cluster, standby, db)) = primary_and_standby(capture).await else {
		return;
	};
	let system = db.system("the test reads the change log");
	let _capture = (capture == LiveCapture::Replication).then(|| db.capture_changes());
	let mut log = system.change_log(None).await.unwrap();

	standby.pause_replay();
	let note = NewNote {
		title: "logged".into(),
	}
	.insert(&db)
	.await
	.unwrap();
	let commit = loop {
		match tokio::time::timeout(Duration::from_secs(30), log.next())
			.await
			.expect("the commit reaches the log")
			.unwrap()
		{
			LogEntry::Commit(commit) => break commit,
			LogEntry::Pruned { .. } => {}
		}
	};
	assert_eq!(commit.changes[0].key, [note.id.to_string()]);
	let lsn = commit.lsn.expect("Postgres commits carry a position");

	assert!(
		db.on_replica_at(lsn, Duration::from_millis(200))
			.await
			.is_none(),
		"a paused standby hasn't replayed the commit"
	);

	standby.resume_replay();
	let replica = db
		.on_replica_at(lsn, Duration::from_secs(30))
		.await
		.unwrap();
	assert_eq!(titles(&replica).await, ["logged"]);
	// The position is serialized with the commit, for the relay.
	let json = serde_json::to_value(&commit).unwrap();
	assert_eq!(json["lsn"], lsn);
	assert_eq!(
		serde_json::from_value::<leptoskit_db::live::Commit>(json).unwrap(),
		commit
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn trigger_commits_carry_a_position_replicas_must_reach() {
	commits_carry_a_position_replicas_must_reach(LiveCapture::Triggers).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn replicated_commits_carry_a_position_replicas_must_reach() {
	commits_carry_a_position_replicas_must_reach(LiveCapture::Replication).await;
}

#[tokio::test]
async fn sqlite_has_no_replicas() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	assert!(db.with_replica("postgres://127.0.0.1/app").is_err());
	assert_eq!(db.replica_count(), 0);
	assert!(!db.on_replica().is_on_replica());
	assert!(db.on_replica_at(0, Duration::ZERO).await.is_none());
	assert!(
		db.with_replicas_from_env(&["LEPTOSKIT_TEST_NO_SUCH_REPLICA"])
			.is_ok()
	);
}
