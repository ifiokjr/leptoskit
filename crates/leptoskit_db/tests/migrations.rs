//! Applying migrations: embedded and loaded, on SQLite and Postgres.

#![cfg(all(feature = "postgres", feature = "sqlite"))]

mod support;

use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Id;
use leptoskit_db::Migration;
use leptoskit_db::MigrationError;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

#[table(name = "notes")]
pub struct Note {
	pub id: Id<Self>,
	pub body: String,
	pub pinned: bool,
}

#[table(name = "labels")]
pub struct Label {
	pub id: Id<Self>,
	pub note_id: Id<Note>,
	pub text: String,
}

/// Three SQLite migrations: create the tables, add a column, and rebuild
/// `notes` while `labels` references it.
static MIGRATIONS: &[Migration] = leptoskit_db::migrations!("tests/fixtures/migrations");

fn names(migrations: &[Migration]) -> Vec<&str> {
	migrations
		.iter()
		.map(|migration| migration.name.as_ref())
		.collect()
}

async fn note(db: &Db, body: &str) -> Note {
	let note = NewNote {
		body: body.into(),
		pinned: false,
	}
	.insert(db)
	.await
	.unwrap();
	NewLabel {
		note_id: note.id,
		text: "x".into(),
	}
	.insert(db)
	.await
	.unwrap();
	note
}

#[tokio::test(flavor = "multi_thread")]
async fn migrations_apply_once_in_registry_order() {
	assert_eq!(
		names(MIGRATIONS),
		[
			"20261028T0900_create",
			"20261029T0900_pin_notes",
			"20261030T0900_check_note_bodies"
		]
	);
	assert!(MIGRATIONS[2].foreign_keys_off() && !MIGRATIONS[0].foreign_keys_off());

	let db = Db::connect("sqlite::memory:").await.unwrap();
	assert_eq!(
		db.migrate(&MIGRATIONS[..2]).await.unwrap(),
		names(&MIGRATIONS[..2])
	);
	let kept = note(&db, "before the rebuild").await;

	// The rebuild drops `notes` while `labels` points at it, so it runs with
	// foreign keys off and checked before it commits.
	assert_eq!(
		db.migrate(MIGRATIONS).await.unwrap(),
		["20261030T0900_check_note_bodies"]
	);
	assert_eq!(db.migrate(MIGRATIONS).await.unwrap(), [] as [String; 0]);
	assert_eq!(
		Note::get(&db, kept.id).await.unwrap().body,
		"before the rebuild"
	);
	assert_eq!(Label::query().count(&db).await.unwrap(), 1);

	let empty = NewNote {
		body: String::new(),
		pinned: true,
	}
	.insert(&db)
	.await
	.unwrap_err();
	assert!(
		matches!(empty, DbError::Check { .. }),
		"the rebuilt table checks bodies: {empty:?}"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn applied_migrations_are_history() {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;

	let mut edited = MIGRATIONS.to_vec();
	edited[1] = Migration::new(
		edited[1].name.to_string(),
		format!("{}\n-- edited", edited[1].sql),
	);
	let error = db.migrate(&edited).await.unwrap_err();
	assert!(
		matches!(&error, MigrationError::Changed { name } if name == "20261029T0900_pin_notes")
	);
	insta::assert_snapshot!(error.to_string());

	let error = db.migrate(&MIGRATIONS[..1]).await.unwrap_err();
	assert!(
		matches!(&error, MigrationError::Unknown { name } if name == "20261029T0900_pin_notes")
	);
	insta::assert_snapshot!(error.to_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn failing_migrations_roll_back() {
	let db = leptoskit_db::testing::sqlite_migrated(MIGRATIONS).await;
	note(&db, "kept").await;

	let orphan = Migration::new(
		"20261031T0900_orphan",
		"-- leptoskit: foreign-keys-off\nINSERT INTO \"labels\" VALUES (randomblob(16), \
		 randomblob(16), 'orphan');",
	);
	let mut with_orphan = MIGRATIONS.to_vec();
	with_orphan.push(orphan);
	let error = db.migrate(&with_orphan).await.unwrap_err();
	assert!(
		matches!(error, MigrationError::ForeignKeys { violations: 1, .. }),
		"{error:?}"
	);
	assert_eq!(
		Label::query().count(&db).await.unwrap(),
		1,
		"the orphan was rolled back"
	);

	let mut broken = MIGRATIONS.to_vec();
	broken.push(Migration::new(
		"20261031T0900_broken",
		"ALTER TABLE \"notes\" ADD COLUMN;",
	));
	let error = db.migrate(&broken).await.unwrap_err();
	assert!(
		matches!(&error, MigrationError::Failed { name, .. } if name == "20261031T0900_broken")
	);
	assert_eq!(
		error.to_string(),
		"migration `20261031T0900_broken` failed and was rolled back"
	);

	// Neither was recorded, so a corrected migration still applies.
	let mut fixed = MIGRATIONS.to_vec();
	fixed.push(Migration::new(
		"20261031T0900_broken",
		"CREATE INDEX \"notes_body_idx\" ON \"notes\" (\"body\");",
	));
	assert_eq!(db.migrate(&fixed).await.unwrap(), ["20261031T0900_broken"]);
}

#[test]
fn loading_at_run_time_matches_embedding() {
	let directory =
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/migrations");
	let loaded = leptoskit_db::load_migrations(&directory).unwrap();
	assert_eq!(loaded, MIGRATIONS);
	assert_eq!(loaded[0].checksum, leptoskit_db::checksum(&loaded[0].sql));

	let missing = leptoskit_db::load_migrations(&directory.join("nowhere")).unwrap_err();
	assert!(matches!(missing, MigrationError::Read { .. }));
	assert!(missing.to_string().starts_with("couldn't read "));
	assert_eq!(
		leptoskit_db::registry_names("# comment\n\n a \nb\n").collect::<Vec<_>>(),
		["a", "b"]
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn code_tables_and_migrated_tables_agree() {
	let from_code = leptoskit_db::testing::sqlite().await;
	let kept = note(&from_code, "from code").await;
	assert_eq!(
		Note::get(&from_code, kept.id).await.unwrap().body,
		"from code"
	);
}

/// A Postgres migration for this file's tables, written to a directory the
/// way `leptoskit db` will.
fn postgres_migrations(directory: &std::path::Path) -> Vec<Migration> {
	let schema = leptoskit_db::schema(leptoskit_db::Dialect::Postgres).unwrap();
	let empty = Schema::empty(leptoskit_db::Dialect::Postgres);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let sql = leptoskit_db_schema::render(&plan, &empty, &schema).sql();
	std::fs::create_dir_all(directory.join("20261028T0900_create")).unwrap();
	std::fs::write(directory.join("20261028T0900_create/migration.sql"), sql).unwrap();
	std::fs::write(directory.join("registry.txt"), "20261028T0900_create\n").unwrap();
	leptoskit_db::load_migrations(directory).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_servers_starting_together_migrate_once() {
	let Some(cluster) = support::postgres::PostgresCluster::start() else {
		return;
	};
	let directory = tempfile::tempdir().unwrap();
	let migrations = postgres_migrations(directory.path());
	let url = cluster.database();
	let first = Db::connect(&url).await.unwrap();
	let second = Db::connect(&url).await.unwrap();

	let (one, two) = tokio::join!(first.migrate(&migrations), second.migrate(&migrations));
	let mut applied = [one.unwrap(), two.unwrap()].concat();
	applied.sort();
	assert_eq!(
		applied,
		["20261028T0900_create"],
		"exactly one server applied it"
	);
	assert_eq!(first.migrate(&migrations).await.unwrap(), [] as [String; 0]);

	let kept = note(&first, "on postgres").await;
	assert_eq!(
		Note::get(&second, kept.id).await.unwrap().body,
		"on postgres"
	);

	let mut broken = migrations.clone();
	broken.push(Migration::new(
		"20261029T0900_broken",
		"ALTER TABLE \"notes\" ADD COLUMN;",
	));
	let error = first.migrate(&broken).await.unwrap_err();
	assert!(matches!(error, MigrationError::Failed { .. }), "{error:?}");
	assert_eq!(
		first.migrate(&migrations).await.unwrap(),
		Vec::<String>::new()
	);

	let mut edited = migrations.clone();
	edited[0] = Migration::new(edited[0].name.to_string(), "-- edited");
	assert!(matches!(
		first.migrate(&edited).await.unwrap_err(),
		MigrationError::Changed { .. }
	));

	let fresh = Db::connect(&cluster.database()).await.unwrap();
	leptoskit_db::testing::create_tables(&fresh).await.unwrap();
	assert_eq!(Note::query().count(&fresh).await.unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn databases_report_their_applied_migrations() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	assert_eq!(db.applied_migrations().await.unwrap(), [] as [String; 0]);
	db.migrate(&MIGRATIONS[..2]).await.unwrap();
	assert_eq!(
		db.applied_migrations().await.unwrap(),
		names(&MIGRATIONS[..2])
	);

	if let Some(cluster) = support::postgres::PostgresCluster::start() {
		let directory = tempfile::tempdir().unwrap();
		let migrations = postgres_migrations(directory.path());
		let db = Db::connect(&cluster.database()).await.unwrap();
		assert_eq!(db.applied_migrations().await.unwrap(), [] as [String; 0]);
		db.migrate(&migrations).await.unwrap();
		assert_eq!(
			db.applied_migrations().await.unwrap(),
			["20261028T0900_create"]
		);
	}
}
