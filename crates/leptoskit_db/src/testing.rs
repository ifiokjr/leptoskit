//! Databases for tests.
//!
//! ```ignore
//! #[tokio::test]
//! async fn publishing_creates_a_post() {
//! 	let db = leptoskit::db::testing::sqlite().await;
//! 	// …
//! }
//! ```

use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;

use crate::Db;
#[cfg(feature = "sqlite")]
use crate::Migration;
use crate::Result;

/// A fresh in-memory SQLite database with every table in this program,
/// created from the code. Fast, and needs no migrations.
///
/// # Panics
///
/// If SQLite can't open the database or the tables don't validate, which is a
/// bug in the test setup rather than a failure to test.
#[cfg(feature = "sqlite")]
pub async fn sqlite() -> Db {
	let db = Db::connect("sqlite::memory:")
		.await
		.expect("SQLite opens an in-memory database");
	create_tables(&db)
		.await
		.expect("every table in this program creates cleanly");
	db
}

/// A fresh in-memory SQLite database with `migrations` applied, as in
/// production.
///
/// # Panics
///
/// If SQLite can't open the database or a migration fails.
#[cfg(feature = "sqlite")]
pub async fn sqlite_migrated(migrations: &[Migration]) -> Db {
	let db = Db::connect("sqlite::memory:")
		.await
		.expect("SQLite opens an in-memory database");

	if let Err(error) = db.migrate(migrations).await {
		panic!("the migrations apply to an empty database: {error}");
	}

	db
}

/// Creates every table linked into this program on `db`, from the code
/// rather than from migrations.
pub async fn create_tables(db: &Db) -> Result<()> {
	let dialect = db.dialect();
	let schema = crate::schema(dialect)
		.map_err(crate::DbError::query)?
		.without_rename_hints();
	let empty = Schema::empty(dialect);
	let plan = leptoskit_db_schema::diff(&empty, &schema, &mut RequireHints)
		.map_err(crate::DbError::query)?;
	let migration = leptoskit_db_schema::render(&plan, &empty, &schema);
	db.run_script(&migration.sql()).await
}
