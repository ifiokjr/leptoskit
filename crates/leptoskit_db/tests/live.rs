//! Change capture against both databases: a live table's writes land in the
//! change log in the writing transaction, its skipped and secret columns are
//! never copied, every commit gets one version, and a rollback leaves no
//! change behind.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use std::time::Duration;

use leptoskit_db::Actor;
use leptoskit_db::ChangeOp;
use leptoskit_db::Commit;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Table;
use leptoskit_db::Timestamp;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

/// A note whose changes every open screen should see. The body preview is
/// wide, so it is never copied; the secret is never copied either.
#[table(name = "notes", live)]
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
	assert!(
		schema
			.tables
			.iter()
			.any(|table| table.name.flat() == "live_changes"),
		"a live table brings the live pack into the schema"
	);
	assert!(
		schema
			.tables
			.iter()
			.any(|table| table.name.flat() == "live_commits"),
		"a live table brings the live pack into the schema"
	);
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

async fn commits(db: &Db) -> Vec<Commit> {
	Commit::query()
		.order_by(|commit| commit.version.asc())
		.all(&db.system("reading the live commit log"))
		.await
		.unwrap()
}

#[tokio::test]
async fn capture_records_inserts_updates_and_deletes() {
	on_both(|db| {
		async move {
			let note = note("buy milk").insert(&db).await.unwrap();
			let changes = db.changes().all().await.unwrap();
			assert_eq!(changes.len(), 1);
			assert_eq!(changes[0].op, ChangeOp::Insert);
			assert_eq!(changes[0].table(), "notes");
			assert_eq!(changes[0].version, Some(1));
			assert_eq!(
				changes[0].new.as_ref().unwrap().0["title"],
				serde_json::json!("buy milk")
			);
			let new = changes[0].new.as_ref().unwrap().0.as_object().unwrap();
			assert!(
				!new.contains_key("secret"),
				"a secret column is never copied"
			);
			assert!(
				!new.contains_key("preview"),
				"a skipped column is never copied"
			);
			assert!(new.contains_key("title"));
			assert!(new.contains_key("created_at"));

			let mut row = Note::get(&db, note.id).await.unwrap();
			row.title = "buy oat milk".into();
			row.save(&db).await.unwrap();

			let changes = db.changes().since(1).all().await.unwrap();
			assert_eq!(changes.len(), 1);
			assert_eq!(changes[0].op, ChangeOp::Update);
			assert_eq!(changes[0].changed.0, vec!["title".to_string()]);
			assert_eq!(
				changes[0].old.as_ref().unwrap().0["title"],
				serde_json::json!("buy milk")
			);
			assert_eq!(
				changes[0].new.as_ref().unwrap().0["title"],
				serde_json::json!("buy oat milk")
			);

			let mut row = Note::get(&db, note.id).await.unwrap();
			row.delete(&db).await.unwrap();

			let changes = db.changes().since(2).all().await.unwrap();
			assert_eq!(changes.len(), 1);
			assert_eq!(changes[0].op, ChangeOp::Delete);
			assert_eq!(
				changes[0].old.as_ref().unwrap().0["title"],
				serde_json::json!("buy oat milk")
			);
		}
	})
	.await;
}

#[tokio::test]
async fn a_skipped_change_still_names_the_column_but_copies_no_value() {
	on_both(|db| {
		async move {
			let note = note("secret keeper").insert(&db).await.unwrap();
			let mut row = Note::get(&db, note.id).await.unwrap();
			row.secret = "changed".into();
			row.preview = vec![9];
			row.save(&db).await.unwrap();

			let changes = db.changes().since(1).all().await.unwrap();
			assert_eq!(changes.len(), 1);
			assert_eq!(
				changes[0].changed.0,
				vec!["preview".to_string(), "secret".to_string()]
			);
			let old = changes[0].old.as_ref().unwrap().0.as_object().unwrap();
			let new = changes[0].new.as_ref().unwrap().0.as_object().unwrap();
			assert!(!old.contains_key("secret") && !new.contains_key("secret"));
			assert!(!old.contains_key("preview") && !new.contains_key("preview"));
			assert_eq!(old["title"], new["title"]);
		}
	})
	.await;
}

#[tokio::test]
async fn one_version_per_commit_and_a_rollback_leaves_none() {
	on_both(|db| {
		async move {
			let first = note("first").insert(&db).await.unwrap();
			note("second").insert(&db).await.unwrap();

			let log = commits(&db).await;
			assert_eq!(log.len(), 2);
			assert_eq!(log[0].version, 1);
			assert_eq!(log[1].version, 2);

			let changes = db.changes().all().await.unwrap();
			assert_eq!(changes.len(), 2);
			assert_eq!(changes[0].version, Some(1));
			assert_eq!(changes[1].version, Some(2));

			let result: leptoskit_db::Result<()> = db
				.transaction(async |tx| {
					note("rolled back").insert(tx).await?;
					Err(leptoskit_db::DbError::NotFound {
						table: "notes".into(),
					})
				})
				.await;
			assert!(result.is_err());
			assert_eq!(db.changes().all().await.unwrap().len(), 2);
			assert_eq!(commits(&db).await.len(), 2);

			let mut row = Note::get(&db, first.id).await.unwrap();
			row.title = "first, changed".into();
			row.save(&db).await.unwrap();
			assert_eq!(commits(&db).await.last().unwrap().version, 3);
		}
	})
	.await;
}

#[tokio::test]
async fn the_actor_of_a_write_is_recorded() {
	on_both(|db| {
		async move {
			let actor = uuid::Uuid::now_v7();
			let acting = db.with_context(Context::new().with(Actor::from_uuid(actor)));
			note("by ada").insert(&acting).await.unwrap();

			let log = commits(&db).await;
			assert_eq!(log.len(), 1);
			assert_eq!(log[0].actor, Some(actor));
		}
	})
	.await;
}

#[tokio::test]
async fn pruning_removes_old_commits_and_changes() {
	on_both(|db| {
		async move {
			note("old").insert(&db).await.unwrap();
			assert_eq!(db.changes().all().await.unwrap().len(), 1);
			assert_eq!(commits(&db).await.len(), 1);

			let removed = leptoskit_db::prune_live(&db, Duration::from_secs(0))
				.await
				.unwrap();
			assert!(removed > 0);
			assert!(db.changes().all().await.unwrap().is_empty());
			assert!(commits(&db).await.is_empty());
		}
	})
	.await;
}

#[tokio::test]
async fn a_live_table_has_the_columns_its_trigger_copies() {
	assert_eq!(
		<Note as Table>::LIVE_COLUMNS,
		&["id", "title", "created_at"]
	);
	assert!(
		<Note as Table>::LIVE,
		"the table macro implements LiveTable"
	);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writers_get_one_ordered_version_each() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let db = Db::connect(&cluster.database()).await.unwrap();
	create_schema(&db).await;

	let mut handles = Vec::new();
	let mut kept = 0;

	for index in 0..20u32 {
		let db = db.clone();
		handles.push(tokio::spawn(async move {
			if index % 5 == 0 {
				let _: Result<(), leptoskit_db::DbError> = db
					.transaction(async |tx| {
						note(&format!("rolled {index}")).insert(tx).await?;
						Err(leptoskit_db::DbError::NotFound {
							table: "notes".into(),
						})
					})
					.await;
				false
			} else {
				note(&format!("kept {index}")).insert(&db).await.unwrap();
				true
			}
		}));
	}

	for handle in handles {
		if handle.await.unwrap() {
			kept += 1;
		}
	}

	let commits = commits(&db).await;
	assert_eq!(commits.len(), kept);
	for pair in commits.windows(2) {
		assert!(
			pair[0].version < pair[1].version,
			"versions are strictly increasing"
		);
	}

	let changes = db.changes().since(0).all().await.unwrap();
	assert_eq!(changes.len(), kept);
	for pair in changes.windows(2) {
		assert!(
			pair[0].version <= pair[1].version,
			"changes follow commit order"
		);
	}
}
