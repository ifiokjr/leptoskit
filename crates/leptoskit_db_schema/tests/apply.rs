//! Runs every scenario's migration against real databases.
//!
//! For each scenario, one database starts at `before`, gets seeded, and is
//! migrated; another is created at `after` directly. The two must have the
//! same columns, keys, indexes, and constraints, and the migrated one must
//! still hold every seeded row.
//!
//! SQLite runs in memory. Postgres runs in a throwaway cluster when its
//! binaries can be found (see [`PostgresCluster::find_binaries`]); otherwise
//! the Postgres test is skipped, unless `LEPTOSKIT_REQUIRE_POSTGRES` is set,
//! as it is in CI.

mod support;

use std::env;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::Migration;
use rusqlite::Connection;
use support::Scenario;
use tempfile::TempDir;

#[test]
fn sqlite_migrations_match_fresh_schemas_and_keep_rows() {
	for scenario in support::scenarios() {
		if scenario.only == Some(Dialect::Postgres) {
			continue;
		}

		let migrated = sqlite_database(&support::create(&(scenario.before)(Dialect::Sqlite)));

		for statement in scenario.seed {
			migrated
				.execute_batch(statement)
				.unwrap_or_else(|error| panic!("{}: seeding failed: {error}", scenario.name));
		}

		apply_sqlite(
			&migrated,
			&scenario.migration(Dialect::Sqlite),
			scenario.name,
		);
		let fresh = sqlite_database(&support::create(&(scenario.after)(Dialect::Sqlite)));

		assert_eq!(
			describe_sqlite(&migrated),
			describe_sqlite(&fresh),
			"{}: the migrated SQLite schema differs from a fresh one",
			scenario.name
		);
		assert_rows(&scenario, |table| {
			migrated
				.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |row| {
					row.get(0)
				})
				.unwrap()
		});
	}
}

#[test]
fn postgres_migrations_match_fresh_schemas_and_keep_rows() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};

	for (number, scenario) in support::scenarios().into_iter().enumerate() {
		if scenario.only == Some(Dialect::Sqlite) {
			continue;
		}

		let migrated = format!("migrated_{number}");
		let fresh = format!("fresh_{number}");
		cluster.create_database(&migrated);
		cluster.create_database(&fresh);

		cluster.run(
			&migrated,
			&support::create(&(scenario.before)(Dialect::Postgres)).sql(),
		);

		for statement in scenario.seed {
			cluster.run(&migrated, statement);
		}

		cluster.run(&migrated, &scenario.migration(Dialect::Postgres).sql());
		cluster.run(
			&fresh,
			&support::create(&(scenario.after)(Dialect::Postgres)).sql(),
		);

		assert_eq!(
			cluster.describe(&migrated),
			cluster.describe(&fresh),
			"{}: the migrated Postgres schema differs from a fresh one",
			scenario.name
		);
		assert_rows(&scenario, |table| {
			let qualified = match table.split_once('_') {
				Some((namespace @ ("auth" | "content"), name)) => format!("{namespace}.\"{name}\""),
				_ => format!("\"{table}\""),
			};
			cluster
				.query(&migrated, &format!("SELECT count(*) FROM {qualified}"))
				.trim()
				.parse()
				.unwrap()
		});
	}
}

fn assert_rows(scenario: &Scenario, count: impl Fn(&str) -> i64) {
	for (table, expected) in scenario.expected_rows {
		assert_eq!(
			count(table),
			*expected,
			"{}: rows in `{table}` after migrating",
			scenario.name
		);
	}
}

fn sqlite_database(create: &Migration) -> Connection {
	let connection = Connection::open_in_memory().unwrap();
	connection
		.pragma_update(None, "foreign_keys", true)
		.unwrap();
	apply_sqlite(&connection, create, "create");
	connection
}

/// Applies a migration the way the migrator does: in one transaction, with
/// foreign keys off and checked before commit when the migration asks.
fn apply_sqlite(connection: &Connection, migration: &Migration, name: &str) {
	if migration.foreign_keys_off {
		connection
			.pragma_update(None, "foreign_keys", false)
			.unwrap();
	}

	let transaction = connection.unchecked_transaction().unwrap();

	for statement in &migration.statements {
		transaction
			.execute_batch(statement)
			.unwrap_or_else(|error| panic!("{name}: `{statement}` failed: {error}"));
	}

	let violations: i64 = transaction
		.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
			row.get(0)
		})
		.unwrap();
	assert_eq!(
		violations, 0,
		"{name}: the migration left foreign key violations"
	);
	transaction.commit().unwrap();
	connection
		.pragma_update(None, "foreign_keys", true)
		.unwrap();

	let integrity: String = connection
		.query_row("PRAGMA integrity_check", [], |row| row.get(0))
		.unwrap();
	assert_eq!(integrity, "ok", "{name}: integrity check");
}

/// Every table's columns, key, indexes, and foreign keys, in a stable order.
/// Column order isn't compared: `ADD COLUMN` always appends.
fn describe_sqlite(connection: &Connection) -> Vec<String> {
	let mut lines = Vec::new();
	let tables: Vec<String> = connection
		.prepare(
			"SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
			 ORDER BY name",
		)
		.unwrap()
		.query_map([], |row| row.get(0))
		.unwrap()
		.collect::<Result<_, _>>()
		.unwrap();

	for table in tables {
		let mut columns: Vec<String> = connection
			.prepare(&format!(
				"SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_xinfo('{table}')"
			))
			.unwrap()
			.query_map([], |row| {
				Ok(format!(
					"{table} column {} {} notnull={} default={:?} pk={}",
					row.get::<_, String>(0)?,
					row.get::<_, String>(1)?,
					row.get::<_, i64>(2)?,
					row.get::<_, Option<String>>(3)?,
					row.get::<_, i64>(4)?,
				))
			})
			.unwrap()
			.collect::<Result<_, _>>()
			.unwrap();
		columns.sort();
		lines.extend(columns);

		let mut indexes: Vec<String> = connection
			.prepare(
				"SELECT sql FROM sqlite_schema WHERE type = 'index' AND tbl_name = ?1 AND sql IS \
				 NOT NULL",
			)
			.unwrap()
			.query_map([&table], |row| row.get::<_, String>(0))
			.unwrap()
			.map(|sql| format!("{table} index {}", sql.unwrap()))
			.collect();
		indexes.sort();
		lines.extend(indexes);

		let mut foreign_keys: Vec<String> = connection
			.prepare(&format!(
				"SELECT \"table\", \"from\", \"to\", on_delete FROM \
				 pragma_foreign_key_list('{table}')"
			))
			.unwrap()
			.query_map([], |row| {
				Ok(format!(
					"{table} foreign key {} → {}.{} on delete {}",
					row.get::<_, String>(1)?,
					row.get::<_, String>(0)?,
					row.get::<_, String>(2)?,
					row.get::<_, String>(3)?,
				))
			})
			.unwrap()
			.collect::<Result<_, _>>()
			.unwrap();
		foreign_keys.sort();
		lines.extend(foreign_keys);
	}

	lines
}

/// A throwaway Postgres cluster, listening on a free local port and deleted
/// when dropped.
struct PostgresCluster {
	binaries: PathBuf,
	directory: TempDir,
	port: u16,
}

impl PostgresCluster {
	fn start() -> Option<Self> {
		let Some(binaries) = Self::find_binaries() else {
			assert!(
				env::var_os("LEPTOSKIT_REQUIRE_POSTGRES").is_none(),
				"LEPTOSKIT_REQUIRE_POSTGRES is set, but no Postgres binaries were found"
			);
			eprintln!("skipping: no Postgres binaries (initdb, pg_ctl, psql) found");
			return None;
		};

		let directory = tempfile::tempdir().unwrap();
		let data = directory.path().join("data");
		let port = TcpListener::bind("127.0.0.1:0")
			.unwrap()
			.local_addr()
			.unwrap()
			.port();

		run(Command::new(binaries.join("initdb"))
			.arg("-D")
			.arg(&data)
			.args([
				"-U",
				"postgres",
				"-A",
				"trust",
				"-E",
				"UTF8",
				"--locale=C",
				"--no-sync",
			]));

		// TCP only: the default socket directory may not be writable.
		let socket_option = if cfg!(windows) {
			""
		} else {
			" -c unix_socket_directories=''"
		};
		run(Command::new(binaries.join("pg_ctl"))
			.arg("-D")
			.arg(&data)
			.arg("-l")
			.arg(directory.path().join("log"))
			.arg("-o")
			.arg(format!(
				"-p {port} -c listen_addresses=127.0.0.1 -F{socket_option}"
			))
			.args(["-w", "start"]));

		Some(Self {
			binaries,
			directory,
			port,
		})
	}

	/// Postgres's binary directory: `LEPTOSKIT_POSTGRES_BIN`, then `PGBIN`
	/// (set on GitHub's Windows runners), then wherever `initdb` is on the
	/// path, then the newest Debian-style install (GitHub's Ubuntu runners).
	fn find_binaries() -> Option<PathBuf> {
		let has_initdb = |directory: &Path| {
			directory.join("initdb").exists() || directory.join("initdb.exe").exists()
		};
		let from_env = ["LEPTOSKIT_POSTGRES_BIN", "PGBIN"]
			.into_iter()
			.filter_map(env::var_os)
			.map(PathBuf::from);
		let from_path = env::var_os("PATH")
			.into_iter()
			.flat_map(|path| env::split_paths(&path).collect::<Vec<_>>());
		let mut debian: Vec<PathBuf> = fs::read_dir("/usr/lib/postgresql")
			.into_iter()
			.flatten()
			.filter_map(|entry| Some(entry.ok()?.path().join("bin")))
			.collect();
		debian.sort_by_key(|path| {
			path.parent()
				.and_then(|version| version.file_name()?.to_str()?.parse::<u32>().ok())
				.unwrap_or(0)
		});
		debian.reverse();

		// Resolve symlinks (Nix profiles, Homebrew): initdb finds its data
		// files relative to its real location.
		let initdb = from_env
			.chain(from_path)
			.chain(debian)
			.find(|directory| has_initdb(directory))?;
		let initdb = [initdb.join("initdb"), initdb.join("initdb.exe")]
			.into_iter()
			.find(|path| path.exists())?;
		fs::canonicalize(initdb)
			.ok()?
			.parent()
			.map(Path::to_path_buf)
	}

	fn psql(&self, database: &str) -> Command {
		let mut command = Command::new(self.binaries.join("psql"));
		command
			.args([
				"-X",
				"-q",
				"-A",
				"-t",
				"-v",
				"ON_ERROR_STOP=1",
				"-h",
				"127.0.0.1",
				"-U",
				"postgres",
			])
			.arg("-p")
			.arg(self.port.to_string())
			.arg("-d")
			.arg(database);
		command
	}

	fn create_database(&self, name: &str) {
		run(self
			.psql("postgres")
			.arg("-c")
			.arg(format!("CREATE DATABASE {name}")));
	}

	/// Runs SQL in one transaction, as the migrator will.
	fn run(&self, database: &str, sql: &str) {
		let file = self.directory.path().join("statement.sql");
		fs::write(&file, sql).unwrap();
		run(self.psql(database).arg("-1").arg("-f").arg(&file));
	}

	fn query(&self, database: &str, sql: &str) -> String {
		run(self.psql(database).arg("-c").arg(sql))
	}

	/// Every column, constraint, index, and comment outside the system
	/// schemas, in a stable order. Column order isn't compared: `ADD COLUMN`
	/// always appends.
	fn describe(&self, database: &str) -> Vec<String> {
		const USER_SCHEMAS: &str = "NOT IN ('pg_catalog', 'information_schema', 'pg_toast')";
		let queries = [
			format!(
				"SELECT table_schema, table_name, column_name, data_type, \
				 character_maximum_length, is_nullable, column_default FROM \
				 information_schema.columns WHERE table_schema {USER_SCHEMAS} ORDER BY 1, 2, 3"
			),
			format!(
				"SELECT n.nspname, c.conrelid::regclass, c.conname, pg_get_constraintdef(c.oid) \
				 FROM pg_constraint c JOIN pg_namespace n ON n.oid = c.connamespace WHERE \
				 n.nspname {USER_SCHEMAS} ORDER BY 1, 2, 3"
			),
			format!(
				"SELECT schemaname, tablename, indexname, indexdef FROM pg_indexes WHERE \
				 schemaname {USER_SCHEMAS} ORDER BY 1, 2, 3"
			),
			format!(
				"SELECT n.nspname, c.relname, a.attname, col_description(c.oid, a.attnum), \
				 obj_description(c.oid, 'pg_class') FROM pg_class c JOIN pg_namespace n ON n.oid \
				 = c.relnamespace JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND \
				 NOT a.attisdropped WHERE c.relkind = 'r' AND n.nspname {USER_SCHEMAS} ORDER BY \
				 1, 2, 3"
			),
		];

		// Sorted here: `regclass` columns sort by OID in SQL, not by name.
		queries
			.iter()
			.flat_map(|query| {
				let mut lines: Vec<String> = self
					.query(database, query)
					.lines()
					.map(str::to_string)
					.collect();
				lines.sort();
				lines
			})
			.collect()
	}
}

impl Drop for PostgresCluster {
	fn drop(&mut self) {
		let _ = Command::new(self.binaries.join("pg_ctl"))
			.arg("-D")
			.arg(self.directory.path().join("data"))
			.args(["-m", "immediate", "-w", "stop"])
			.output();
	}
}

/// Runs a command and returns its output, panicking with its stderr if it
/// fails.
fn run(command: &mut Command) -> String {
	let output = command
		.output()
		.unwrap_or_else(|error| panic!("couldn't run {command:?}: {error}"));
	assert!(
		output.status.success(),
		"{command:?} failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	String::from_utf8(output.stdout).unwrap()
}
