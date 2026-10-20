#![cfg(not(target_arch = "wasm32"))]

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::Plugin;
use leptoskit_plugin_sqlite::CLOSE;
use leptoskit_plugin_sqlite::Database;
use leptoskit_plugin_sqlite::EXECUTE;
use leptoskit_plugin_sqlite::ExecuteResult;
use leptoskit_plugin_sqlite::MAX_BYTES;
use leptoskit_plugin_sqlite::MAX_ROWS;
use leptoskit_plugin_sqlite::QUERY;
use leptoskit_plugin_sqlite::QueryResult;
use leptoskit_plugin_sqlite::Statement;
use leptoskit_plugin_sqlite::TRANSACTION;
use leptoskit_plugin_sqlite::Value;
use leptoskit_plugin_sqlite::decode_transaction_result;
use leptoskit_plugin_sqlite::encode_transaction;

fn statement(sql: &str) -> Statement {
	Statement::new(sql, vec![])
}
fn database() -> (tempfile::TempDir, Database) {
	let root = tempfile::tempdir().unwrap();
	let database = Database::new(root.path()).unwrap();
	(root, database)
}
fn count(db: &mut Database) -> i64 {
	let result = db
		.query(&statement("SELECT COUNT(*) FROM entries"))
		.unwrap();
	let Value::Integer(count) = result.rows[0][0] else {
		panic!("count must be an integer")
	};
	count
}

#[test]
fn bindings_preserve_all_values_and_parameter_text_is_not_sql() {
	let (_root, mut db) = database();
	db.execute(&statement(
		"CREATE TABLE entries (id INTEGER PRIMARY KEY, nul, integer_value, real_value, \
		 text_value, blob_value)",
	))
	.unwrap();
	let values = vec![
		Value::Null,
		Value::Integer(i64::MIN),
		Value::Real(1.125),
		Value::Text("'; DROP TABLE entries; --\0🙂".into()),
		Value::Blob(vec![0, 255, 128, 1]),
	];
	let insert = Statement::new(
		"INSERT INTO entries (nul, integer_value, real_value, text_value, blob_value) VALUES (?, \
		 ?, ?, ?, ?)",
		values.clone(),
	);
	let request = insert.encode().unwrap();
	assert_eq!(Statement::decode(&request).unwrap(), insert);
	let result = ExecuteResult::decode(&db.invoke(EXECUTE, request).unwrap()).unwrap();
	assert_eq!(
		result,
		ExecuteResult {
			changes: 1,
			last_insert_rowid: 1
		}
	);
	let response = db
		.invoke(
			QUERY,
			statement("SELECT nul, integer_value, real_value, text_value, blob_value FROM entries")
				.encode()
				.unwrap(),
		)
		.unwrap();
	let rows = QueryResult::decode(&response).unwrap();
	assert_eq!(rows.rows, [values]);
	assert_eq!(
		rows.columns,
		[
			"nul",
			"integer_value",
			"real_value",
			"text_value",
			"blob_value"
		]
	);
	assert_eq!(count(&mut db), 1);
}

#[test]
fn failed_batch_rolls_back_and_next_transaction_works() {
	let (_root, mut db) = database();
	db.execute(&statement("CREATE TABLE entries (value INTEGER UNIQUE)"))
		.unwrap();
	let failed = [
		statement("INSERT INTO entries VALUES (1)"),
		statement("INSERT INTO entries VALUES (1)"),
	];
	assert!(
		db.invoke(TRANSACTION, encode_transaction(&failed).unwrap())
			.is_err()
	);
	assert_eq!(count(&mut db), 0);
	let success = [
		statement("INSERT INTO entries VALUES (2)"),
		statement("INSERT INTO entries VALUES (3)"),
	];
	let response = db
		.invoke(TRANSACTION, encode_transaction(&success).unwrap())
		.unwrap();
	let results = decode_transaction_result(&response).unwrap();
	assert_eq!(results.len(), 2);
	assert!(results.iter().all(|result| result.changes == 1));
	assert_eq!(count(&mut db), 2);
}

#[test]
fn a_single_statement_with_fail_conflict_also_rolls_back() {
	let (_root, mut db) = database();
	db.execute(&statement("CREATE TABLE entries (value INTEGER UNIQUE)"))
		.unwrap();
	assert!(
		db.execute(&statement("INSERT OR FAIL INTO entries VALUES (1), (1)"))
			.is_err()
	);
	assert_eq!(count(&mut db), 0);
}

#[test]
fn persists_across_close_and_new_plugin_instance() {
	let (root, mut db) = database();
	db.execute(&statement("CREATE TABLE entries (value TEXT)"))
		.unwrap();
	db.execute(&Statement::new(
		"INSERT INTO entries VALUES (?)",
		vec![Value::Text("saved".into())],
	))
	.unwrap();
	assert!(db.invoke(CLOSE, Bytes::new()).unwrap().is_empty());
	assert_eq!(count(&mut db), 1);
	drop(db);
	let mut reopened = Database::new(root.path()).unwrap();
	assert_eq!(count(&mut reopened), 1);
}

#[test]
fn malformed_sql_binding_count_and_multiple_statements_fail_without_changes() {
	let (_root, mut db) = database();
	db.execute(&statement("CREATE TABLE entries (value INTEGER)"))
		.unwrap();
	for sql in [
		"INSERT broken SQL",
		"INSERT INTO entries VALUES (?)",
		"INSERT INTO entries VALUES (1); INSERT INTO entries VALUES (2)",
	] {
		assert!(db.execute(&statement(sql)).is_err(), "{sql}");
	}
	assert_eq!(count(&mut db), 0);
	assert!(
		db.query(&statement("INSERT INTO entries VALUES (8) RETURNING value"))
			.is_err()
	);
	assert_eq!(count(&mut db), 0);
}

#[test]
fn denies_external_files_extensions_and_guest_transaction_control() {
	let (root, mut db) = database();
	let outside = root.path().join("escape.sqlite3");
	for sql in [
		format!("ATTACH DATABASE '{}' AS escaped", outside.display()),
		format!("VACUUM INTO '{}'", outside.display()),
		"PRAGMA writable_schema = 1".into(),
		"PRAGMA temp_store_directory = '/tmp'".into(),
		"BEGIN".into(),
		"COMMIT".into(),
		"SAVEPOINT guest".into(),
		"SELECT load_extension('/tmp/library')".into(),
		"CREATE VIRTUAL TABLE external USING csv(filename='/tmp/file')".into(),
	] {
		assert!(db.execute(&statement(&sql)).is_err(), "{sql}");
	}
	assert!(!outside.exists());
	assert!(
		db.query(&statement("SELECT * FROM pragma_database_list"))
			.is_err()
	);
	assert!(
		db.query(&statement("SELECT load_extension('/tmp/library')"))
			.is_err()
	);
	assert_eq!(
		db.query(&statement("SELECT 42")).unwrap().rows,
		[vec![Value::Integer(42)]]
	);
}

#[test]
fn row_byte_and_instruction_limits_recover_for_following_calls() {
	let (_root, mut db) = database();
	let many = statement(&format!(
		"WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x < {MAX_ROWS}) \
		 SELECT x FROM n"
	));
	assert!(db.query(&many).is_err());
	let wide = statement("SELECT randomblob(600000), randomblob(600000)");
	assert!(db.query(&wide).is_err());
	let expensive = statement(
		"WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x < 10000000) SELECT \
		 SUM(x) FROM n",
	);
	assert!(db.query(&expensive).is_err());
	assert_eq!(
		db.query(&statement("SELECT 7")).unwrap().rows,
		[vec![Value::Integer(7)]]
	);
	assert!(
		Statement::new("SELECT ?", vec![Value::Blob(vec![0; MAX_BYTES])])
			.encode()
			.is_err()
	);
}

#[test]
fn foreign_key_enforcement_and_atomic_ddl() {
	let (_root, mut db) = database();
	db.execute(&statement("CREATE TABLE parent (id INTEGER PRIMARY KEY)"))
		.unwrap();
	db.execute(&statement(
		"CREATE TABLE child (parent_id INTEGER REFERENCES parent(id))",
	))
	.unwrap();
	assert!(
		db.execute(&statement("INSERT INTO child VALUES (123)"))
			.is_err()
	);
	assert!(
		db.transaction(&[
			statement("CREATE TABLE should_rollback (id)"),
			statement("invalid SQL")
		])
		.is_err()
	);
	assert!(
		db.query(&statement("SELECT * FROM should_rollback"))
			.is_err()
	);
}

#[cfg(unix)]
#[test]
fn symlinked_directory_database_and_sidecars_cannot_escape() {
	use std::os::unix::fs::PermissionsExt;
	use std::os::unix::fs::symlink;
	for name in [
		"sqlite",
		"sqlite/app.sqlite3",
		"sqlite/app.sqlite3-journal",
		"sqlite/app.sqlite3-wal",
		"sqlite/app.sqlite3-shm",
	] {
		let (root, mut db) = database();
		let outside = tempfile::tempdir().unwrap();
		let sentinel = outside.path().join("sentinel");
		std::fs::write(&sentinel, b"untouched").unwrap();
		if name != "sqlite" {
			std::fs::create_dir(root.path().join("sqlite")).unwrap();
			std::fs::set_permissions(
				root.path().join("sqlite"),
				std::fs::Permissions::from_mode(0o700),
			)
			.unwrap();
		}
		symlink(
			if name == "sqlite" {
				outside.path()
			} else {
				&sentinel
			},
			root.path().join(name),
		)
		.unwrap();
		assert!(db.query(&statement("SELECT 1")).is_err(), "{name}");
		assert_eq!(std::fs::read(&sentinel).unwrap(), b"untouched");
		assert!(!outside.path().join("app.sqlite3").exists());
	}
}

#[test]
fn malformed_wire_and_empty_transactions_are_rejected() {
	let (_root, mut db) = database();
	let encoded = statement("SELECT 1").encode().unwrap();
	for end in 0..encoded.len() {
		assert!(Statement::decode(&encoded[..end]).is_err());
	}
	let mut trailing = encoded.to_vec();
	trailing.push(1);
	assert_eq!(Statement::decode(&trailing), Err(Error::InvalidArgument));
	assert_eq!(encode_transaction(&[]), Err(Error::InvalidArgument));
	assert_eq!(
		db.invoke(TRANSACTION, Bytes::from_static(&[0, 0, 0, 0])),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		db.invoke(CLOSE, Bytes::from_static(&[1])),
		Err(Error::InvalidArgument)
	);
	for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
		assert!(
			Statement::new("SELECT ?", vec![Value::Real(value)])
				.encode()
				.is_err()
		);
	}
}

#[cfg(unix)]
#[test]
fn preexisting_hard_links_and_shared_directories_are_rejected() {
	use std::os::unix::fs::PermissionsExt;
	let (root, mut db) = database();
	std::fs::create_dir(root.path().join("sqlite")).unwrap();
	std::fs::set_permissions(
		root.path().join("sqlite"),
		std::fs::Permissions::from_mode(0o755),
	)
	.unwrap();
	assert!(db.query(&statement("SELECT 1")).is_err());
	std::fs::set_permissions(
		root.path().join("sqlite"),
		std::fs::Permissions::from_mode(0o700),
	)
	.unwrap();
	let sentinel = root.path().join("sentinel");
	std::fs::write(&sentinel, b"untouched").unwrap();
	std::fs::hard_link(&sentinel, root.path().join("sqlite/app.sqlite3")).unwrap();
	assert!(db.query(&statement("SELECT 1")).is_err());
	assert_eq!(std::fs::read(&sentinel).unwrap(), b"untouched");
}

#[test]
fn strict_wire_limits_cover_tags_numbers_columns_and_rows() {
	let sample = QueryResult {
		columns: vec!["x".into()],
		rows: vec![vec![Value::Integer(i64::MAX)]],
	}
	.encode()
	.unwrap();
	for end in 0..sample.len() {
		assert!(QueryResult::decode(&sample[..end]).is_err());
	}
	let mut trailing = sample.to_vec();
	trailing.push(0);
	assert_eq!(QueryResult::decode(&trailing), Err(Error::InvalidArgument));
	let oversized = QueryResult {
		columns: vec!["x".repeat(MAX_BYTES)],
		rows: vec![],
	};
	assert_eq!(oversized.encode(), Err(Error::TooLarge));
	let wrong_width = QueryResult {
		columns: vec!["x".into()],
		rows: vec![vec![]],
	};
	assert_eq!(wrong_width.encode(), Err(Error::InvalidArgument));
	let mut unknown_tag = sample.to_vec();
	// columns u32, string length u32, one column byte, rows u32, then tag.
	unknown_tag[13..17].copy_from_slice(&99_u32.to_le_bytes());
	assert_eq!(
		QueryResult::decode(&unknown_tag),
		Err(Error::InvalidArgument)
	);
	let mut nonfinite = sample.to_vec();
	nonfinite[13..17].copy_from_slice(&2_u32.to_le_bytes());
	nonfinite[21..29].copy_from_slice(&f64::INFINITY.to_le_bytes());
	assert_eq!(QueryResult::decode(&nonfinite), Err(Error::InvalidArgument));
	assert_eq!(
		ExecuteResult {
			changes: 7,
			last_insert_rowid: -1
		}
		.encode()
		.as_ref(),
		&[
			7, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255
		]
	);
}

#[test]
fn interrupted_batch_rolls_back_earlier_writes() {
	let (_root, mut db) = database();
	db.execute(&statement("CREATE TABLE entries (value INTEGER)"))
		.unwrap();
	assert!(
		db.transaction(&[
			statement("INSERT INTO entries VALUES (1)"),
			statement(
				"WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x < \
				 10000000) INSERT INTO entries SELECT x FROM n"
			),
		])
		.is_err()
	);
	assert_eq!(count(&mut db), 0);
	db.execute(&statement("INSERT INTO entries VALUES (2)"))
		.unwrap();
	assert_eq!(count(&mut db), 1);
}

#[test]
fn construction_does_no_io_and_rejects_guest_style_relative_paths() {
	let root = tempfile::tempdir().unwrap();
	let absent = root.path().join("absent");
	let mut db = Database::new(&absent).unwrap();
	assert!(!absent.exists());
	assert!(db.query(&statement("SELECT 1")).is_err());
	assert!(!absent.exists());
	assert!(Database::new("relative/path").is_err());
}
