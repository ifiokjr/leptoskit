//! The change log against both databases: a live table's writes land in the
//! log in the writing transaction, whoever writes them, raw SQL and the
//! database's own cascades included; skipped and secret columns are never
//! copied; every commit gets one version, in commit order; a rollback leaves
//! nothing behind; and pruning keeps the version line going.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use std::collections::HashSet;
use std::time::Duration;

use leptoskit_db::Actor;
use leptoskit_db::Connection;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Id;
use leptoskit_db::Table;
use leptoskit_db::Timestamp;
use leptoskit_db::live::ChangeLog;
use leptoskit_db::live::ChangeOp;
use leptoskit_db::live::ChangedRows;
use leptoskit_db::live::Commit;
use leptoskit_db::live::Invalidation;
use leptoskit_db::live::LogEntry;
use leptoskit_db::table;
use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

/// A note whose changes every open screen should see. The preview is wide,
/// so it is never copied; the secret is never copied either.
#[table(name = "log_notes", live)]
pub struct Note {
	pub id: Id<Self>,
	pub title: String,
	#[db(secret)]
	pub secret: String,
	#[db(live = skip)]
	pub preview: Vec<u8>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A pin on a note, which the database deletes with it.
#[table(name = "log_pins", live)]
pub struct Pin {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub note_id: Id<Note>,
}

/// A mention of a note, which outlives it with its link emptied.
#[table(name = "log_mentions", live)]
pub struct Mention {
	pub id: Id<Self>,
	pub note_id: Option<Id<Note>>,
}

fn note(title: &str) -> NewNote {
	NewNote {
		title: title.into(),
		secret: format!("secret-of-{title}"),
		preview: vec![1, 2, 3],
	}
}

async fn create_schema(db: &Db) {
	let dialect = db.dialect();
	let schema = leptoskit_db::schema(dialect).unwrap();

	for pack_table in ["live_changes", "live_commits"] {
		assert!(
			schema
				.tables
				.iter()
				.any(|table| table.name.flat() == pack_table),
			"a live table brings the live pack into the schema"
		);
	}

	let empty = Schema::empty(dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.statements.join(";\n"))
		.await
		.unwrap();
}

async fn on_both<F, Fut>(check: F)
where
	F: Fn(Db) -> Fut,
	Fut: Future<Output = ()>,
{
	let db = Db::connect("sqlite::memory:").await.unwrap();
	create_schema(&db).await;
	check(db).await;

	if let Some(cluster) = support::postgres::PostgresCluster::start() {
		let db = Db::connect(&cluster.database()).await.unwrap();
		create_schema(&db).await;
		check(db).await;
	}
}

fn system(db: &Db) -> Db {
	db.system("reading the change log")
}

/// Every commit after `version`, up to the newest.
async fn commits_since(db: &Db, version: i64) -> Vec<Commit> {
	let newest = db.live_version().await.unwrap();
	let mut log = system(db).change_log(Some(version)).await.unwrap();
	let mut commits = Vec::new();

	while log.version() < newest {
		match tokio::time::timeout(Duration::from_secs(10), log.next())
			.await
			.expect("the log has the commit")
			.unwrap()
		{
			LogEntry::Commit(commit) => commits.push(commit),
			LogEntry::Pruned { through } => panic!("pruned through {through}"),
		}
	}

	commits
}

/// The one change in the one commit after `version`.
async fn only_change(db: &Db, version: i64) -> leptoskit_db::live::Change {
	let mut commits = commits_since(db, version).await;
	assert_eq!(commits.len(), 1, "{commits:?}");
	let mut changes = commits.remove(0).changes;
	assert_eq!(changes.len(), 1, "{changes:?}");
	changes.remove(0)
}

#[tokio::test]
async fn capture_records_inserts_updates_and_deletes() {
	on_both(async |db| {
		let note = note("buy milk").insert(&db).await.unwrap();
		let change = only_change(&db, 0).await;
		assert_eq!(change.op, ChangeOp::Insert);
		assert_eq!(change.table.to_string(), "log_notes");
		assert_eq!(change.key, [note.id.to_string()]);
		assert!(change.old.is_none());
		let new = change.new.unwrap();
		assert_eq!(new["title"], "buy milk");
		assert_eq!(
			new["id"],
			note.id.to_string(),
			"an id reads the same everywhere"
		);
		assert!(
			!new.contains_key("secret"),
			"a secret column is never copied"
		);
		assert!(
			!new.contains_key("preview"),
			"a skipped column is never copied"
		);
		assert!(new.contains_key("created_at"));

		let mut row = Note::get(&db, note.id).await.unwrap();
		row.title = "buy oat milk".into();
		row.save(&db).await.unwrap();
		let change = only_change(&db, 1).await;
		assert_eq!(change.op, ChangeOp::Update);
		assert_eq!(change.changed, ["title"]);
		assert_eq!(change.old.unwrap()["title"], "buy milk");
		assert_eq!(change.new.unwrap()["title"], "buy oat milk");

		// An update that changes nothing is still a write subscribers hear of.
		row.save(&db).await.unwrap();
		let change = only_change(&db, 2).await;
		assert_eq!(change.op, ChangeOp::Update);
		assert_eq!(change.changed, Vec::<String>::new());

		row.destroy(&db).await.unwrap();
		let change = only_change(&db, 3).await;
		assert_eq!(change.op, ChangeOp::Delete);
		assert_eq!(change.old.unwrap()["title"], "buy oat milk");
		assert!(change.new.is_none());
	})
	.await;
}

#[tokio::test]
async fn a_skipped_change_still_names_the_column_but_copies_no_value() {
	on_both(async |db| {
		let note = note("secret keeper").insert(&db).await.unwrap();
		let mut row = Note::get(&db, note.id).await.unwrap();
		row.secret = "changed".into();
		row.preview = vec![9];
		row.save(&db).await.unwrap();

		let change = only_change(&db, 1).await;
		assert_eq!(change.changed, ["preview", "secret"]);
		let (old, new) = (change.old.unwrap(), change.new.unwrap());
		assert!(!old.contains_key("secret") && !new.contains_key("secret"));
		assert!(!old.contains_key("preview") && !new.contains_key("preview"));
		assert_eq!(old["title"], new["title"]);
	})
	.await;
}

#[tokio::test]
async fn one_version_per_commit_and_a_rollback_leaves_none() {
	on_both(async |db| {
		let first = note("first").insert(&db).await.unwrap();
		note("second").insert(&db).await.unwrap();
		let versions = |commits: Vec<Commit>| -> Vec<i64> {
			commits.iter().map(|commit| commit.version).collect()
		};
		assert_eq!(versions(commits_since(&db, 0).await), [1, 2]);

		let failed: Result<(), DbError> = db
			.transaction(async |tx| {
				note("rolled back").insert(tx).await?;
				Err(DbError::NotFound {
					table: "on purpose".into(),
				})
			})
			.await;
		assert!(failed.is_err());
		assert_eq!(db.live_version().await.unwrap(), 2);

		// Two rows in one transaction are one commit.
		db.transaction(async |tx| {
			note("third").insert(tx).await?;
			note("fourth").insert(tx).await?;
			Ok::<_, DbError>(())
		})
		.await
		.unwrap();
		let commits = commits_since(&db, 2).await;
		assert_eq!(versions(commits.clone()), [3]);
		assert_eq!(commits[0].changes.len(), 2);

		let mut row = Note::get(&db, first.id).await.unwrap();
		row.title = "first, changed".into();
		row.save(&db).await.unwrap();
		assert_eq!(db.live_version().await.unwrap(), 4);
	})
	.await;
}

#[tokio::test]
async fn a_transaction_records_its_actor_and_origin() {
	on_both(async |db| {
		let actor = uuid::Uuid::now_v7();
		let acting = db.with_context(Context::new().with(Actor::from_uuid(actor)));
		acting
			.transaction(async |tx| {
				tx.set_origin("notes.create").await?;
				note("by ada").insert(tx).await
			})
			.await
			.unwrap();
		// Without a transaction, there's no one to say.
		note("by nobody").insert(&db).await.unwrap();

		let commits = commits_since(&db, 0).await;
		assert_eq!(commits.len(), 2);
		assert_eq!(
			(commits[0].actor, commits[0].origin.as_str()),
			(Some(actor), "notes.create")
		);
		assert_eq!(
			(commits[1].actor, commits[1].origin.as_str()),
			(None, "sql")
		);
		assert!(commits[0].committed_at <= commits[1].committed_at);
	})
	.await;
}

/// SQL literals for a note's columns, as each database writes them.
fn raw_note(dialect: Dialect, id: Id<Note>, title: &str) -> String {
	let (id, preview) = match dialect {
		Dialect::Postgres => (format!("'{id}'"), "'\\x0102'".to_string()),
		Dialect::Sqlite => (format!("x'{}'", id.uuid().simple()), "x'0102'".to_string()),
	};

	format!(
		"INSERT INTO log_notes (id, title, secret, preview, created_at) VALUES ({id}, '{title}', \
		 's', {preview}, '2026-10-31T12:00:00.000Z')"
	)
}

fn raw_id(dialect: Dialect, id: impl std::fmt::Display + Copy, simple: &str) -> String {
	match dialect {
		Dialect::Postgres => format!("'{id}'"),
		Dialect::Sqlite => format!("x'{simple}'"),
	}
}

#[tokio::test]
async fn raw_sql_is_captured_and_heard() {
	on_both(async |db| {
		let dialect = db.dialect();
		let mut feed = system(&db).changes().await.unwrap();
		let id = Id::<Note>::new();
		db.run_script(&raw_note(dialect, id, "written by hand"))
			.await
			.unwrap();
		let heard = tokio::time::timeout(Duration::from_secs(10), feed.next())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(
			heard,
			Invalidation::Rows(vec![ChangedRows {
				table: "log_notes".into(),
				keys: vec![vec![id.to_string()]],
			}])
		);

		let quoted = raw_id(dialect, id, &id.uuid().simple().to_string());
		db.run_script(&format!(
			"UPDATE log_notes SET title = 'edited by hand' WHERE id = {quoted}"
		))
		.await
		.unwrap();
		let change = only_change(&db, 1).await;
		assert_eq!(
			(change.op, change.new.unwrap()["title"].clone()),
			(ChangeOp::Update, "edited by hand".into())
		);
		assert_eq!(Note::get(&db, id).await.unwrap().title, "edited by hand");
	})
	.await;
}

#[tokio::test]
async fn the_database_s_own_cascades_fire_the_capture_triggers() {
	on_both(async |db| {
		let dialect = db.dialect();
		let parent = note("parent").insert(&db).await.unwrap();
		let pin = NewPin { note_id: parent.id }.insert(&db).await.unwrap();
		let mention = NewMention {
			note_id: Some(parent.id),
		}
		.insert(&db)
		.await
		.unwrap();
		let before = db.live_version().await.unwrap();

		// Raw SQL, so only the database's foreign keys reach the other rows.
		let quoted = raw_id(dialect, parent.id, &parent.id.uuid().simple().to_string());
		db.run_script(&format!("DELETE FROM log_notes WHERE id = {quoted}"))
			.await
			.unwrap();

		let commits = commits_since(&db, before).await;
		assert_eq!(commits.len(), 1);
		let mut changes: Vec<(String, ChangeOp, String)> = commits[0]
			.changes
			.iter()
			.map(|change| (change.table.to_string(), change.op, change.key[0].clone()))
			.collect();
		changes.sort_by(|left, right| left.0.cmp(&right.0));
		assert_eq!(
			changes,
			[
				(
					"log_mentions".to_string(),
					ChangeOp::Update,
					mention.id.to_string()
				),
				(
					"log_notes".to_string(),
					ChangeOp::Delete,
					parent.id.to_string()
				),
				("log_pins".to_string(), ChangeOp::Delete, pin.id.to_string()),
			]
		);
		let emptied = commits[0]
			.changes
			.iter()
			.find(|change| change.op == ChangeOp::Update)
			.unwrap();
		assert_eq!(emptied.changed, ["note_id"]);
		assert_eq!(
			emptied.new.as_ref().unwrap()["note_id"],
			serde_json::Value::Null
		);
	})
	.await;
}

#[tokio::test]
async fn pruning_keeps_the_version_line_and_marks_how_far_it_went() {
	on_both(async |db| {
		for title in ["one", "two", "three"] {
			note(title).insert(&db).await.unwrap();
		}

		// Nothing is old enough yet.
		assert_eq!(
			db.prune_changes(Duration::from_secs(3600)).await.unwrap(),
			0
		);
		assert_eq!(commits_since(&db, 0).await.len(), 3);

		// Everything is: the newest commit stays as the marker.
		assert_eq!(db.prune_changes(Duration::ZERO).await.unwrap(), 2);
		assert_eq!(db.prune_changes(Duration::ZERO).await.unwrap(), 0);
		assert_eq!(db.live_version().await.unwrap(), 3);
		let mut log = system(&db).change_log(Some(1)).await.unwrap();
		assert_eq!(log.next().await.unwrap(), LogEntry::Pruned { through: 3 });

		// Versions carry on after the marker, on SQLite too, which counts
		// them from the log.
		note("four").insert(&db).await.unwrap();
		let LogEntry::Commit(commit) = log.next().await.unwrap() else {
			panic!("expected the new commit");
		};
		assert_eq!(commit.version, 4);
		assert_eq!(commits_since(&db, 3).await.len(), 1);
	})
	.await;
}

#[tokio::test]
async fn the_log_is_for_system_handles() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	let error = db
		.with_context(Context::new())
		.change_log(None)
		.await
		.unwrap_err();
	assert!(error.to_string().contains("system"), "{error}");
	assert!(format!("{:?}", system(&db).change_log(None).await.unwrap()).starts_with("ChangeLog"));
}

#[tokio::test]
async fn a_database_without_live_tables_has_an_empty_log() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	assert_eq!(db.live_version().await.unwrap(), 0);
	assert_eq!(db.prune_changes(Duration::ZERO).await.unwrap(), 0);
	let mut feed = db.changes().await.unwrap();
	assert_eq!(feed.version(), 0);
	assert!(
		tokio::time::timeout(Duration::from_millis(100), feed.next())
			.await
			.is_err(),
		"nothing to hear"
	);
}

#[tokio::test]
async fn a_live_table_has_the_columns_its_trigger_copies() {
	assert_eq!(<Note as Table>::LIVE_COLUMNS, ["id", "title", "created_at"]);
	const { assert!(<Note as Table>::LIVE && !<Note as Table>::AUDIT) };
}

// ---- Postgres ordering ---------------------------------------------------
/// A deferred trigger that runs at commit, after `live.seal()` has taken
/// the version, and can pause or fail the commit.
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

/// `live.seal()` without its advisory lock, to show what the lock is for.
const SEAL_WITHOUT_LOCK: &str = "
DO $$ BEGIN
	EXECUTE replace(pg_get_functiondef('live.seal()'::regprocedure),
		'PERFORM pg_advisory_xact_lock(', 'PERFORM abs(');
END $$;
";

async fn gate(tx: &leptoskit_db::Tx, id: i64, fail: bool, pause_ms: i32) -> Result<(), DbError> {
	tx.execute(leptoskit_db::Statement {
		sql: "INSERT INTO commit_gate (id, fail, pause_ms) VALUES ($1, $2, $3)".into(),
		args: vec![
			leptoskit_db::Value::Int(id),
			leptoskit_db::Value::Bool(fail),
			leptoskit_db::Value::Int(pause_ms.into()),
		],
	})
	.await
	.map(drop)
}

/// The notes a log reader heard of until it reached `until`, in order,
/// checking that versions only go up.
async fn heard_until(log: &mut ChangeLog, until: i64, heard: &mut Vec<String>) {
	while log.version() < until {
		let before = log.version();

		match tokio::time::timeout(Duration::from_secs(30), log.next())
			.await
			.expect("the log reaches the newest version")
			.unwrap()
		{
			LogEntry::Commit(commit) => {
				assert!(commit.version > before, "versions only go up");
				heard.extend(
					commit
						.changes
						.into_iter()
						.map(|change| change.key[0].clone()),
				);
			}
			LogEntry::Pruned { through } => panic!("pruned through {through}"),
		}
	}
}

/// Many writers commit at once, some pausing after they take a version and
/// some failing after it, while a reader follows `version > n`: it hears of
/// every committed note once, in version order, and never of one that
/// rolled back. Without the commit lock, a reader passes a version before
/// a lower one becomes visible, and misses it.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_writers_are_read_once_each_in_version_order() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let db = Db::connect(&cluster.database()).await.unwrap();
	create_schema(&db).await;
	db.run_script(COMMIT_GATE).await.unwrap();
	let mut log = system(&db).change_log(None).await.unwrap();
	let writing = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));

	// The reader follows along while the writers write.
	let reader = tokio::spawn({
		let writing = std::sync::Arc::clone(&writing);
		async move {
			let mut heard = Vec::new();

			loop {
				match tokio::time::timeout(Duration::from_millis(50), log.next()).await {
					Ok(Ok(LogEntry::Commit(commit))) => {
						heard.extend(
							commit
								.changes
								.into_iter()
								.map(|change| change.key[0].clone()),
						);
					}
					Ok(Ok(LogEntry::Pruned { through })) => panic!("pruned through {through}"),
					Ok(Err(error)) => panic!("{error}"),
					Err(_) if !writing.load(std::sync::atomic::Ordering::Relaxed) => break,
					Err(_) => {}
				}
			}

			(log, heard)
		}
	});

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
						gate(tx, id, fail, pause).await?;
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

	writing.store(false, std::sync::atomic::Ordering::Relaxed);
	let (mut log, mut heard) = reader.await.unwrap();
	let newest = db.live_version().await.unwrap();
	heard_until(&mut log, newest, &mut heard).await;

	let unique: HashSet<String> = heard.iter().cloned().collect();
	assert_eq!(unique.len(), heard.len(), "each note is heard once");
	assert_eq!(unique, committed, "every committed note, and only those");
	assert!(committed.len() > 100, "{}", committed.len());
}

/// What a reader following the log hears when one commit has taken its
/// version but not finished, and another commits meanwhile.
async fn heard_while_a_commit_waits(url: &str, lock: bool) -> (Vec<String>, [String; 2]) {
	let db = Db::connect(url).await.unwrap();
	create_schema(&db).await;
	db.run_script(COMMIT_GATE).await.unwrap();

	if !lock {
		db.run_script(SEAL_WITHOUT_LOCK).await.unwrap();
	}

	let mut log = system(&db).change_log(None).await.unwrap();

	// The first commit takes its version, then pauses for a second.
	let first = tokio::spawn({
		let db = db.clone();
		async move {
			db.transaction(async |tx| {
				let note = note("first").insert(tx).await?;
				gate(tx, 1, false, 1000).await?;
				Ok::<_, DbError>(note.id.to_string())
			})
			.await
			.unwrap()
		}
	});
	tokio::time::sleep(Duration::from_millis(300)).await;

	// The second commits meanwhile, or waits its turn.
	let second = tokio::spawn({
		let db = db.clone();
		async move {
			let note = note("second").insert(&db).await.unwrap();
			note.id.to_string()
		}
	});
	tokio::time::sleep(Duration::from_millis(300)).await;

	// The reader reads whatever is visible now.
	let mut heard = Vec::new();

	while let Ok(entry) = tokio::time::timeout(Duration::from_millis(100), log.next()).await {
		if let LogEntry::Commit(commit) = entry.unwrap() {
			heard.extend(
				commit
					.changes
					.into_iter()
					.map(|change| change.key[0].clone()),
			);
		}
	}

	let notes = [first.await.unwrap(), second.await.unwrap()];
	heard_until(&mut log, db.live_version().await.unwrap(), &mut heard).await;
	(heard, notes)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_version_becomes_visible_only_after_every_lower_one() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};

	let (heard, [first, second]) = heard_while_a_commit_waits(&cluster.database(), true).await;
	assert_eq!(heard, [first, second]);

	// Without the lock, the second commit is visible first, the reader
	// moves past the first's version, and never hears of it.
	let (heard, [_, second]) = heard_while_a_commit_waits(&cluster.database(), false).await;
	assert_eq!(heard, [second]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serializable_writers_never_conflict_through_the_log() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let db = Db::connect(&cluster.database()).await.unwrap();
	create_schema(&db).await;

	for _ in 0..5 {
		let left = db.begin_serializable().await.unwrap();
		let right = db.begin_serializable().await.unwrap();
		note("left").insert(&left).await.unwrap();
		note("right").insert(&right).await.unwrap();
		left.commit().await.unwrap();
		right.commit().await.unwrap();
	}

	assert_eq!(db.live_version().await.unwrap(), 10);
}
