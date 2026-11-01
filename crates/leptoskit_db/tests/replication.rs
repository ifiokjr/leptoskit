//! Replication capture on Postgres: the change log it writes reads exactly
//! as the capture triggers' would, from ORM writes and raw SQL alike, with
//! `secret` and skipped columns never copied, versions in commit order and
//! each commit's write-ahead log position recorded; a rollback never
//! reaches it; one server captures at a time, and another takes over; the
//! slot never moves past what the log holds; migrations switch between
//! triggers and replication; and the partitioned log is pruned by dropping
//! partitions.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;

use leptoskit_db::Actor;
use leptoskit_db::Connection;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::LiveCapture;
use leptoskit_db::Migration;
use leptoskit_db::Read;
use leptoskit_db::Statement;
use leptoskit_db::Timestamp;
use leptoskit_db::Value;
use leptoskit_db::live::Capture;
use leptoskit_db::live::CaptureSettings;
use leptoskit_db::live::Commit;
use leptoskit_db::live::Invalidation;
use leptoskit_db::live::LogEntry;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use support::postgres::PostgresCluster;

/// A note with a value of most kinds, a secret that's never copied, and a
/// wide preview that isn't either.
#[table(name = "rep_notes", live)]
pub struct Note {
	pub id: Id<Self>,
	pub title: String,
	pub score: f64,
	pub count: i32,
	pub done: bool,
	pub extra: Json<serde_json::Value>,
	pub summary: Option<String>,
	#[db(secret)]
	pub secret: String,
	#[db(live = skip)]
	pub preview: Vec<u8>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A pin on a note, which the database deletes with it.
#[table(name = "rep_pins", live)]
pub struct Pin {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub note_id: Id<Note>,
}

fn note(title: &str) -> NewNote {
	NewNote {
		title: title.into(),
		score: 1.5,
		count: 3,
		done: false,
		extra: Json(serde_json::json!({ "tags": ["a", "b"], "n": 1 })),
		summary: None,
		secret: format!("secret-of-{title}"),
		preview: vec![1, 2, 3],
	}
}

/// The schema's migration from nothing, captured by `capture`.
fn migration_to(capture: LiveCapture) -> (Schema, String) {
	let schema = leptoskit_db::with_live_capture(
		leptoskit_db::schema(leptoskit_db_schema::Dialect::Postgres).unwrap(),
		capture,
	)
	.unwrap();
	let empty = Schema::empty(schema.dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let sql = leptoskit_db_schema::render(&plan, &empty, &schema).sql();
	(schema, sql)
}

async fn database(cluster: &PostgresCluster, capture: LiveCapture) -> Db {
	let db = Db::connect(&cluster.database()).await.unwrap();
	db.run_script(&migration_to(capture).1).await.unwrap();
	db
}

fn settings() -> CaptureSettings {
	CaptureSettings::default()
		.poll(Duration::from_millis(10))
		.standby(Duration::from_millis(50))
		.idle(Duration::from_millis(50))
}

/// Waits until `check` holds, or fails after a while.
async fn eventually(what: &str, mut check: impl AsyncFnMut() -> bool) {
	let waited = tokio::time::timeout(Duration::from_secs(30), async {
		while !check().await {
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
	})
	.await;
	assert!(waited.is_ok(), "timed out waiting until {what}");
}

/// Capture with `settings`, once it's capturing.
async fn capturing(db: &Db, settings: CaptureSettings) -> Capture {
	let capture = db.capture_changes_with(settings);
	eventually("capture starts", async || capture.stats().capturing).await;
	capture
}

/// Every commit in the log after `version`, once capture has caught up.
async fn commits_since(db: &Db, version: i64) -> Vec<Commit> {
	tokio::time::timeout(Duration::from_secs(30), db.live_caught_up())
		.await
		.expect("capture catches up")
		.unwrap();
	let system = db.system("reading the change log");
	let newest = system.live_version().await.unwrap();
	let mut log = system.change_log(Some(version)).await.unwrap();
	let mut commits = Vec::new();

	while log.version() < newest {
		match tokio::time::timeout(Duration::from_secs(10), log.next())
			.await
			.unwrap()
			.unwrap()
		{
			LogEntry::Commit(commit) => commits.push(commit),
			LogEntry::Pruned { .. } => {}
		}
	}

	commits
}

/// The same writes, every way: ORM inserts, updates, and deletes, one of
/// them only to a secret, raw SQL, a cascade, a rollback, and a transaction
/// that says who acts in it and why.
async fn write_everything(db: &Db) {
	let system = db.system("writing");
	let mut first = note("first").insert(&system).await.unwrap();
	first.title = "first, edited".into();
	first.extra = Json(serde_json::json!({ "tags": [], "n": 2 }));
	first.save(&system).await.unwrap();
	first.secret = "a new secret".into();
	first.save(&system).await.unwrap();

	let pinned = note("pinned").insert(&system).await.unwrap();
	NewPin { note_id: pinned.id }.insert(&system).await.unwrap();
	pinned.destroy(&system).await.unwrap();

	system
		.run_script(
			"INSERT INTO rep_notes (id, title, score, count, done, extra, summary, secret, \
			 preview, created_at) VALUES ('00000000-0000-7000-8000-000000000001', 'by hand', \
			 'NaN', -7, true, '{\"k\": null}', 'sum', 's', '\\x0102', \
			 '2026-10-31T12:00:00.123456Z'); UPDATE rep_notes SET summary = NULL, score = 2.25 \
			 WHERE title = 'by hand'",
		)
		.await
		.unwrap();

	let failed: Result<(), DbError> = system
		.transaction(async |tx| {
			note("never").insert(tx).await?;
			Err(DbError::NotFound {
				table: "on purpose".into(),
			})
		})
		.await;
	assert!(failed.is_err());

	let actor = uuid::Uuid::from_u128(42);
	db.with_context(Context::new().with(Actor::from_uuid(actor)))
		.transaction(async |tx| {
			tx.set_origin("notes.create").await?;
			note("by ada").insert(tx).await
		})
		.await
		.unwrap();

	first.destroy(&system).await.unwrap();
}

/// A commit as two databases can compare it: the generated IDs replaced by
/// the order they appear in, and the commit's time and version left out.
fn comparable(commits: &[Commit]) -> Vec<serde_json::Value> {
	let mut ids: HashMap<String, String> = HashMap::new();
	let mut out = Vec::new();

	for commit in commits {
		let changes: Vec<serde_json::Value> = commit
			.changes
			.iter()
			.map(|change| {
				let old = change.old.clone().map(|image| renamed(image, &mut ids));
				let new = change.new.clone().map(|image| renamed(image, &mut ids));
				serde_json::json!({
					"table": change.table.to_string(),
					"op": format!("{:?}", change.op),
					"key": change.key.iter().map(|key| id_name(&mut ids, key)).collect::<Vec<_>>(),
					"old": old,
					"new": new,
					"changed": change.changed,
				})
			})
			.collect();
		out.push(serde_json::json!({
			"actor": commit.actor,
			"origin": commit.origin,
			"changes": changes,
		}));
	}

	out
}

/// The name an ID gets: `id0` for the first seen, and so on.
fn id_name(ids: &mut HashMap<String, String>, id: &str) -> String {
	let next = format!("id{}", ids.len());
	ids.entry(id.to_string()).or_insert(next).clone()
}

/// An image with its IDs named and its creation time left out.
fn renamed(
	mut image: serde_json::Map<String, serde_json::Value>,
	ids: &mut HashMap<String, String>,
) -> serde_json::Map<String, serde_json::Value> {
	for column in ["id", "note_id"] {
		let Some(serde_json::Value::String(id)) = image.get(column).cloned() else {
			continue;
		};
		image.insert(column.into(), id_name(ids, &id).into());
	}

	image.remove("created_at");
	image
}

/// Each commit's version and write-ahead log position, oldest first.
async fn positions(db: &Db) -> Vec<(i64, Option<i64>, String)> {
	db.fetch(Statement {
		sql: "SELECT version, lsn, origin FROM live.commits ORDER BY version".into(),
		args: Vec::new(),
	})
	.await
	.unwrap()
	.into_iter()
	.map(|row| {
		let mut row = row.into_iter();
		let version = match row.next() {
			Some(Value::Int(version)) => version,
			other => panic!("{other:?}"),
		};
		let lsn = match row.next() {
			Some(Value::Int(lsn)) => Some(lsn),
			_ => None,
		};
		let origin = match row.next() {
			Some(Value::Text(origin)) => origin,
			other => panic!("{other:?}"),
		};
		(version, lsn, origin)
	})
	.collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replication_records_what_the_triggers_record() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let triggers = database(&cluster, LiveCapture::Triggers).await;
	write_everything(&triggers).await;
	let by_triggers = comparable(&commits_since(&triggers, 0).await);

	let replication = database(&cluster, LiveCapture::Replication).await;
	let capture = capturing(&replication, settings()).await;
	let start = replication.live_version().await.unwrap();
	write_everything(&replication).await;
	let commits = commits_since(&replication, start).await;
	let by_replication = comparable(&commits);

	assert_eq!(
		serde_json::to_string_pretty(&by_replication).unwrap(),
		serde_json::to_string_pretty(&by_triggers).unwrap()
	);

	// Never a secret or a skipped column; the secret's change is still named.
	let edited_secret = &commits[2].changes[0];
	assert_eq!(edited_secret.changed, ["secret"]);

	for change in commits.iter().flat_map(|commit| &commit.changes) {
		for image in [&change.old, &change.new].into_iter().flatten() {
			assert!(!image.contains_key("secret") && !image.contains_key("preview"));
		}
	}

	// Versions follow commit order, and each commit has its position.
	let recorded = positions(&replication).await;
	let captured: Vec<i64> = recorded
		.iter()
		.filter(|(version, ..)| *version > start)
		.map(|(_, lsn, _)| lsn.expect("a captured commit has its position"))
		.collect();
	assert_eq!(captured.len(), commits.len());
	assert!(
		captured.windows(2).all(|pair| pair[0] < pair[1]),
		"{captured:?}"
	);
	assert!(
		recorded[0].2 == "leptoskit.pruned",
		"making the slot marks a gap: {recorded:?}"
	);
	assert!(capture.stats().commits >= commits.len() as u64);
	capture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_transaction_still_open_is_captured_once_it_commits() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let db = database(&cluster, LiveCapture::Replication).await;
	let capture = capturing(&db, settings()).await;
	let system = db.system("writing");
	let start = db.live_version().await.unwrap();

	// One transaction writes and stays open while another commits and is
	// captured: the slot moves on, but never past what the open one needs.
	let open = system.begin().await.unwrap();
	let early = note("written first, committed last")
		.insert(&open)
		.await
		.unwrap();
	let later = note("committed first").insert(&system).await.unwrap();
	let rolled_back = system.begin().await.unwrap();
	note("rolled back").insert(&rolled_back).await.unwrap();
	rolled_back.rollback().await.unwrap();

	let commits = commits_since(&db, start).await;
	assert_eq!(commits.len(), 1);
	assert_eq!(commits[0].changes[0].key, [later.id.to_string()]);

	open.commit().await.unwrap();
	let commits = commits_since(&db, start).await;
	let keys: Vec<&str> = commits
		.iter()
		.map(|commit| commit.changes[0].key[0].as_str())
		.collect();
	assert_eq!(keys, [later.id.to_string(), early.id.to_string()]);
	capture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_slot_never_moves_past_what_the_log_holds() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let db = database(&cluster, LiveCapture::Replication).await;
	// The first write to the log fails before it commits, as if the server
	// died there: the slot stays put, and the next try records it.
	let capture = capturing(&db, settings().fail_writes(1)).await;
	let start = db.live_version().await.unwrap();
	let first = note("first").insert(&db).await.unwrap();
	let commits = commits_since(&db, start).await;
	assert_eq!(commits.len(), 1);
	assert_eq!(commits[0].changes[0].key, [first.id.to_string()]);
	capture.shutdown().await;

	// This time the log has the commit but the slot never moves, as if the
	// server died between the two: whoever captures next reads it again and
	// skips it.
	let capture = capturing(&db, settings().skip_advances(usize::MAX)).await;
	let second = note("second").insert(&db).await.unwrap();
	eventually("the commit is recorded", async || {
		capture.stats().commits == 1
	})
	.await;
	let stats = db.live_stats().await.unwrap();
	assert!(stats.live_capture_lag.unwrap() > 0, "{stats:?}");
	capture.shutdown().await;

	let capture = capturing(&db, settings()).await;
	let third = note("third").insert(&db).await.unwrap();
	let keys: Vec<String> = commits_since(&db, start)
		.await
		.into_iter()
		.map(|commit| commit.changes[0].key[0].clone())
		.collect();
	assert_eq!(
		keys,
		[
			first.id.to_string(),
			second.id.to_string(),
			third.id.to_string()
		],
		"each commit once"
	);
	capture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_server_captures_and_another_takes_over() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let url = cluster.database();
	let db = Db::connect(&url).await.unwrap();
	db.run_script(&migration_to(LiveCapture::Replication).1)
		.await
		.unwrap();
	let elsewhere = Db::connect(&url).await.unwrap();
	let here = db.capture_changes_with(settings());
	let there = elsewhere.capture_changes_with(settings());
	eventually("one captures", async || {
		here.stats().capturing || there.stats().capturing
	})
	.await;
	tokio::time::sleep(Duration::from_millis(300)).await;
	assert!(
		here.stats().capturing != there.stats().capturing,
		"exactly one captures"
	);

	let (leader, standby) = if here.stats().capturing {
		(here, there)
	} else {
		(there, here)
	};
	let start = db.live_version().await.unwrap();
	let first = note("first").insert(&db).await.unwrap();
	assert_eq!(commits_since(&db, start).await.len(), 1);
	assert_eq!(standby.stats().commits, 0);

	// The leader goes away; the other takes over.
	leader.shutdown().await;
	eventually("the standby takes over", async || standby.stats().capturing).await;
	let second = note("second").insert(&db).await.unwrap();

	// Its connection dies, as when its server does; it captures again once
	// it's back.
	db.run_script(
		"SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = \
		 'leptoskit live capture'",
	)
	.await
	.unwrap();
	let third = note("third").insert(&db).await.unwrap();
	let keys: Vec<String> = commits_since(&db, start)
		.await
		.into_iter()
		.map(|commit| commit.changes[0].key[0].clone())
		.collect();
	assert_eq!(
		keys,
		[
			first.id.to_string(),
			second.id.to_string(),
			third.id.to_string()
		]
	);
	standby.shutdown().await;
	let stats = db.live_stats().await.unwrap();
	assert_eq!(stats.capture, LiveCapture::Replication);
	assert!(!stats.slot_active);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn migrations_switch_between_triggers_and_replication() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let db = Db::connect(&cluster.database()).await.unwrap();
	let (triggers, first_sql) = migration_to(LiveCapture::Triggers);
	let (replication, _) = migration_to(LiveCapture::Replication);
	let switch = |from: &Schema, to: &Schema| {
		let plan = leptoskit_db_schema::diff(from, to, &mut RequireHints).unwrap();
		leptoskit_db_schema::render(&plan, from, to).sql()
	};
	let migrations = [
		Migration::new("20261101T0000_live", first_sql),
		Migration::new("20261101T0001_replicate", switch(&triggers, &replication)),
		Migration::new("20261101T0002_triggers", switch(&replication, &triggers)),
	];
	db.migrate(&migrations[..1]).await.unwrap();
	let capture = db.capture_changes_with(settings());
	let system = db.system("writing");

	// Triggers record a commit as it happens.
	note("by trigger").insert(&system).await.unwrap();
	assert_eq!(db.live_version().await.unwrap(), 1);
	let mut feed = system.changes().await.unwrap();

	// Replication: capture makes its slot, and readers hear they may have
	// missed what was written before it.
	db.migrate(&migrations[..2]).await.unwrap();
	eventually("capture starts", async || capture.stats().capturing).await;
	let replicated = note("by replication").insert(&system).await.unwrap();
	assert_eq!(next(&mut feed).await, Invalidation::Everything);
	assert_eq!(next(&mut feed).await, rows_of(&[replicated.id.to_string()]));
	assert_eq!(
		db.live_stats().await.unwrap().capture,
		LiveCapture::Replication
	);

	// Back to triggers: capture forgets its slot, marking the gap again.
	db.migrate(&migrations).await.unwrap();
	eventually("the slot is dropped", async || {
		!capture.stats().capturing && db.live_stats().await.unwrap().live_capture_lag.is_none()
	})
	.await;
	assert_eq!(next(&mut feed).await, Invalidation::Everything);
	let by_trigger = note("by trigger again").insert(&system).await.unwrap();
	assert_eq!(next(&mut feed).await, rows_of(&[by_trigger.id.to_string()]));
	assert_eq!(
		db.live_stats().await.unwrap().capture,
		LiveCapture::Triggers
	);
	capture.shutdown().await;
}

async fn next(feed: &mut leptoskit_db::live::Feed) -> Invalidation {
	tokio::time::timeout(Duration::from_secs(20), feed.next())
		.await
		.expect("a change arrives")
		.unwrap()
}

fn rows_of(keys: &[String]) -> Invalidation {
	Invalidation::Rows(vec![leptoskit_db::live::ChangedRows {
		table: "rep_notes".into(),
		keys: keys.iter().map(|key| vec![key.clone()]).collect(),
	}])
}

/// The range partitions of a log table, by name.
async fn partitions(db: &Db, table: &str) -> Vec<String> {
	let mut names: Vec<String> = db
		.fetch(Statement {
			sql: "SELECT c.relname FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid WHERE \
			      i.inhparent = to_regclass($1)"
				.into(),
			args: vec![Value::Text(format!("live.{table}"))],
		})
		.await
		.unwrap()
		.into_iter()
		.filter_map(|row| {
			match row.into_iter().next() {
				Some(Value::Text(name)) => Some(name),
				_ => None,
			}
		})
		.collect();
	names.sort();
	names
}

async fn count(db: &Db, sql: &str) -> i64 {
	match db
		.fetch(Statement {
			sql: sql.into(),
			args: Vec::new(),
		})
		.await
		.unwrap()
		.first()
		.and_then(|row| row.first())
	{
		Some(Value::Int(count)) => *count,
		other => panic!("{other:?}"),
	}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_partitioned_log_is_pruned_by_dropping_partitions() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};

	for capture in [LiveCapture::Triggers, LiveCapture::Replication] {
		prunes_by_dropping_partitions(&database(&cluster, capture).await, capture).await;
	}
}

async fn prunes_by_dropping_partitions(db: &Db, mode: LiveCapture) {
	let capture = match mode {
		LiveCapture::Triggers => None,
		LiveCapture::Replication => Some(capturing(db, settings()).await),
	};
	let system = db.system("writing");

	// Written before maintenance first runs, so into the default partition.
	note("early").insert(&system).await.unwrap();
	commits_since(db, 0).await;
	makes_ranges_ahead(db, mode).await;

	// A reader that falls behind, and one that keeps up.
	let behind = db.system("behind").changes_since(0).await.unwrap();
	let newest = db.live_version().await.unwrap();
	let current = db.system("current").changes_since(newest).await.unwrap();

	// Pruning everything: the newest commit stays as the marker, and the
	// default partition's older rows go.
	let removed = db.prune_changes(Duration::ZERO).await.unwrap();
	assert!(removed > 0, "{mode:?}");
	assert_eq!(
		count(
			db,
			&format!("SELECT count(*) FROM live.commits_default WHERE version < {newest}")
		)
		.await,
		0,
		"{mode:?}: nothing older in the default partition"
	);
	assert_eq!(
		count(db, "SELECT count(*) FROM live.changes_default").await,
		0,
		"{mode:?}: nor its changes"
	);

	readers_hear_the_marker(db, behind, current, mode).await;

	if let Some(capture) = capture {
		capture.shutdown().await;
	}
}

/// One round of maintenance makes ranges ahead of both tables, and the
/// writes after it land in them.
async fn makes_ranges_ahead(db: &Db, mode: LiveCapture) {
	assert_eq!(
		db.maintain_changes(Duration::from_secs(3600))
			.await
			.unwrap(),
		0
	);
	let ranges = partitions(db, "commits").await;
	assert!(ranges.len() >= 3, "{ranges:?}");
	assert!(ranges.contains(&"commits_default".to_string()));
	assert!(partitions(db, "changes").await.len() >= 3);

	let start = db.live_version().await.unwrap();
	let system = db.system("writing");

	for index in 0..12 {
		note(&format!("note {index}"))
			.insert(&system)
			.await
			.unwrap();
	}

	assert_eq!(commits_since(db, start).await.len(), 12);
	let in_default = count(
		db,
		&format!("SELECT count(*) FROM live.commits_default WHERE version > {start}"),
	)
	.await;
	assert_eq!(in_default, 0, "{mode:?}: writes land in the ranges");
}

/// The reader behind hears whatever whole commits are left below the
/// marker, then that it may have missed some, then what comes next; the
/// reader that kept up hears only what comes next.
async fn readers_hear_the_marker(
	db: &Db,
	mut behind: leptoskit_db::live::Feed,
	mut current: leptoskit_db::live::Feed,
	mode: LiveCapture,
) {
	let after_prune = note("after").insert(&db.system("writing")).await.unwrap();
	let expected = rows_of(&[after_prune.id.to_string()]);
	let mut heard_everything = false;

	loop {
		match next(&mut behind).await {
			Invalidation::Everything => heard_everything = true,
			rows if rows == expected => break,
			Invalidation::Rows(_) => assert!(!heard_everything, "nothing older after the marker"),
		}
	}

	assert!(heard_everything, "{mode:?}");
	assert_eq!(next(&mut current).await, expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_ranges_below_the_marker_are_dropped() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let db = database(&cluster, LiveCapture::Triggers).await;
	let system = db.system("writing");

	// Ranges of four versions and four changes, made by hand.
	db.run_script(
		"CREATE TABLE live.commits_p1 PARTITION OF live.commits FOR VALUES FROM (1) TO (5); \
		 CREATE TABLE live.commits_p5 PARTITION OF live.commits FOR VALUES FROM (5) TO (9); \
		 CREATE TABLE live.commits_p9 PARTITION OF live.commits FOR VALUES FROM (9) TO (100); \
		 CREATE TABLE live.changes_p1 PARTITION OF live.changes FOR VALUES FROM (1) TO (5); \
		 CREATE TABLE live.changes_p5 PARTITION OF live.changes FOR VALUES FROM (5) TO (9); \
		 CREATE TABLE live.changes_p9 PARTITION OF live.changes FOR VALUES FROM (9) TO (100);",
	)
	.await
	.unwrap();

	for index in 0..10 {
		note(&format!("note {index}"))
			.insert(&system)
			.await
			.unwrap();
	}

	assert_eq!(db.live_version().await.unwrap(), 10);
	// Every commit up to 9 is old enough: 9 is the marker, so the ranges
	// wholly below it go, and the one holding it stays.
	db.run_script(
		"UPDATE live.commits SET committed_at = now() - interval '1 hour' WHERE version <= 9",
	)
	.await
	.unwrap();
	let removed = db.prune_changes(Duration::from_secs(60)).await.unwrap();
	assert_eq!(removed, 8);
	assert_eq!(
		partitions(&db, "commits").await,
		["commits_default", "commits_p9"]
	);
	assert_eq!(
		partitions(&db, "changes").await,
		["changes_default", "changes_p9"]
	);
	assert_eq!(
		positions(&db)
			.await
			.into_iter()
			.map(|(version, _, origin)| (version, origin))
			.collect::<Vec<_>>(),
		[(9, "leptoskit.pruned".to_string()), (10, "sql".to_string())]
	);
	assert_eq!(
		count(&db, "SELECT count(*) FROM live.changes").await,
		1,
		"only the newest commit's change"
	);
	assert_eq!(
		count(
			&db,
			"SELECT count(*) FROM live.changes ch WHERE NOT EXISTS (SELECT FROM live.commits c \
			 WHERE c.txid = ch.txid)"
		)
		.await,
		0,
		"no change without its commit"
	);
	assert_eq!(count(&db, "SELECT count(*) FROM live.pending").await, 1);

	// Scheduled, the same happens on its own.
	for index in 0..3 {
		note(&format!("later {index}"))
			.insert(&system)
			.await
			.unwrap();
	}

	let upkeep = db.live_maintenance(Duration::ZERO, Duration::from_secs(1));
	eventually("maintenance prunes", async || {
		count(
			&db,
			"SELECT count(*) FROM live.commits WHERE version = 13 AND origin = 'leptoskit.pruned'",
		)
		.await
			== 1
	})
	.await;
	upkeep.shutdown().await;
	assert!(
		partitions(&db, "commits").await.len() > 2,
		"and makes ranges ahead"
	);
}

/// A deferred trigger that runs at commit and can pause or fail it.
const COMMIT_GATE: &str = "
CREATE TABLE commit_gate (id bigint PRIMARY KEY, fail boolean NOT NULL, pause_ms int NOT NULL);
CREATE FUNCTION commit_gate() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
	PERFORM pg_sleep(NEW.pause_ms / 1000.0);
	IF NEW.fail THEN
		RAISE EXCEPTION 'rolled back at commit';
	END IF;
	RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER commit_gate AFTER INSERT ON commit_gate
	DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION commit_gate();
";

/// Many writers commit at once, some pausing at commit and some failing
/// there, while a reader follows the log: it hears of every committed note
/// once, in commit order, and never of one that rolled back; and versions
/// follow the write-ahead log's order.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_writers_are_read_once_each_in_commit_order() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};
	let db = database(&cluster, LiveCapture::Replication).await;
	db.run_script(COMMIT_GATE).await.unwrap();
	let capture = capturing(&db, settings()).await;
	let start = db.live_version().await.unwrap();
	let mut writers = Vec::new();

	for writer in 0..8i64 {
		let db = db.clone();
		writers.push(tokio::spawn(async move {
			let mut committed = Vec::new();

			for round in 0..25i64 {
				let id = writer * 1000 + round;
				let fail = (id * 7) % 4 == 0;
				let pause = i32::try_from((id * 13) % 5).unwrap();
				let result = db
					.transaction(async |tx| {
						let note = note(&format!("note {id}")).insert(tx).await?;
						tx.execute(Statement {
							sql: "INSERT INTO commit_gate (id, fail, pause_ms) VALUES ($1, $2, $3)"
								.into(),
							args: vec![Value::Int(id), Value::Bool(fail), Value::Int(pause.into())],
						})
						.await?;
						Ok::<_, DbError>(note.id)
					})
					.await;

				match result {
					Ok(note) => committed.push(note.to_string()),
					Err(error) => assert!(fail, "only a failing gate rolls back: {error}"),
				}
			}

			committed
		}));
	}

	let mut committed = HashSet::new();

	for writer in writers {
		committed.extend(writer.await.unwrap());
	}

	let commits = commits_since(&db, start).await;
	let heard: Vec<String> = commits
		.iter()
		.flat_map(|commit| commit.changes.iter().map(|change| change.key[0].clone()))
		.collect();
	let unique: HashSet<String> = heard.iter().cloned().collect();
	assert_eq!(unique.len(), heard.len(), "each note is heard once");
	assert_eq!(unique, committed, "every committed note, and only those");
	assert!(committed.len() > 100, "{}", committed.len());

	let positions: Vec<i64> = positions(&db)
		.await
		.into_iter()
		.filter(|(version, ..)| *version > start)
		.filter_map(|(_, lsn, _)| lsn)
		.collect();
	assert_eq!(positions.len(), commits.len());
	assert!(
		positions.windows(2).all(|pair| pair[0] < pair[1]),
		"versions follow the write-ahead log"
	);
	capture.shutdown().await;
}
