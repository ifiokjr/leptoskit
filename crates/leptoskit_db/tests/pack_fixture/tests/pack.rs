//! An app using the pack: its tables join the app's schema in their own
//! namespace, the pack's API writes them, and the app reads them.

use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::Dialect;
use leptoskit_db_pack_fixture::Note;
use leptoskit_db_pack_fixture::add_note;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::TableName;

// What an app's db crate writes, so the pack is linked in.
leptoskit_db::packs!(leptoskit_db_pack_fixture);

#[test]
fn pack_tables_join_the_schema_in_their_namespace() {
	let schema = leptoskit_db::schema(Dialect::Postgres).unwrap();
	let names: Vec<&TableName> = schema.tables.iter().map(|table| &table.name).collect();
	assert!(names.contains(&&TableName::namespaced("sample", "notes")));
	assert!(names.contains(&&TableName::namespaced("sample", "signatures")));
	assert_eq!(<Note as leptoskit_db::Table>::PACK, Some("sample"));

	// The same tables export for SQLite, as `sample_notes`.
	leptoskit_db::schema(Dialect::Sqlite).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pack_writes_and_the_app_reads() {
	let db = Db::connect("sqlite::memory:").await.unwrap();
	let schema = leptoskit_db::schema(db.dialect()).unwrap();
	let empty = Schema::empty(db.dialect());
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints).unwrap();
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.statements.join(";\n"))
		.await
		.unwrap();

	let note = add_note(&db, "hello", "ada").await.unwrap();
	let read = Note::get(&db, note.id).await.unwrap();
	assert_eq!(read.body, "hello");
	assert_eq!(read.signatures().count(&db).await.unwrap(), 1);
	assert!(
		add_note(&db, "", "ada").await.is_err(),
		"the pack's rules hold"
	);
}

#[test]
fn only_the_pack_writes_its_tables() {
	trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
