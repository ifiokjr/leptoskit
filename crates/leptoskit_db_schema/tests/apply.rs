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
use std::process::Stdio;

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

		if let Some(search) = posts_search(&scenario) {
			for statement in SEARCH_WRITES {
				migrated.execute_batch(statement).unwrap();
			}

			assert_search(&scenario, &search, |word| {
				migrated
					.query_row(
						"SELECT count(*) FROM \"posts_lk_search\" WHERE \"posts_lk_search\" MATCH \
						 ?1",
						[format!("\"{word}\"")],
						|row| row.get(0),
					)
					.unwrap()
			});
			let (rows, posts): (i64, i64) = migrated
				.query_row(
					"SELECT (SELECT count(*) FROM \"posts_lk_search_rows\"), (SELECT count(*) \
					 FROM \"posts\")",
					[],
					|row| Ok((row.get(0)?, row.get(1)?)),
				)
				.unwrap();
			assert_eq!(
				rows, posts,
				"{}: every post is numbered once",
				scenario.name
			);
		}
	}
}

/// Writes after a migration that gives a post a searched index: one new
/// post, one retitled, and one deleted, so the triggers or generated column
/// must follow each.
const SEARCH_WRITES: &[&str] = &[
	"INSERT INTO posts (id, author_id, title, status) VALUES (13, 2, 'Zebra crossings', 'draft')",
	"UPDATE posts SET title = 'Goodbye' WHERE id = 10",
	"DELETE FROM posts WHERE id = 12",
];

/// The posts table's search index after a scenario, if it has one.
fn posts_search(scenario: &Scenario) -> Option<leptoskit_db_schema::Search> {
	(scenario.after)(Dialect::Postgres)
		.tables
		.into_iter()
		.find(|table| table.name.name == "posts")?
		.search
}

/// Each word's matches after [`SEARCH_WRITES`].
fn assert_search(
	scenario: &Scenario,
	search: &leptoskit_db_schema::Search,
	count: impl Fn(&str) -> i64,
) {
	// English stems `crossings` to `crossing`; simple keeps every form.
	// Stopwords such as `again` are left out: Postgres's `english` drops
	// them and SQLite's `porter` keeps them.
	let crossing = i64::from(search.language == "english");

	for (word, expected) in [
		("zebra", 1),
		("crossing", crossing),
		("hello", 0),
		("goodbye", 1),
		("mine", 0),
		("first", 1),
	] {
		assert_eq!(
			count(word),
			expected,
			"{}: posts matching `{word}` after migrating and writing",
			scenario.name
		);
	}
}

#[test]
fn postgres_migrations_match_fresh_schemas_and_keep_rows() {
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};

	let available = cluster.query(
		"postgres",
		"SELECT name FROM pg_available_extensions WHERE name IN ('postgis', 'vector')",
	);
	let required = env::var("LEPTOSKIT_REQUIRE_POSTGRES_EXTENSIONS").unwrap_or_default();

	for (number, scenario) in support::scenarios().into_iter().enumerate() {
		if scenario.only == Some(Dialect::Sqlite) {
			continue;
		}

		// Scenarios with shapes or vectors need the extensions, which a
		// Postgres without them skips unless CI says they're installed.
		let mut needed = (scenario.before)(Dialect::Postgres).extensions();
		needed.extend((scenario.after)(Dialect::Postgres).extensions());

		if let Some(missing) = needed
			.iter()
			.find(|extension| !available.lines().any(|line| line.trim() == **extension))
		{
			assert!(
				!required.split(',').any(|name| name.trim() == *missing),
				"LEPTOSKIT_REQUIRE_POSTGRES_EXTENSIONS names `{missing}`, but this Postgres can't \
				 install it"
			);
			eprintln!(
				"skipping {}: this Postgres has no `{missing}` extension",
				scenario.name
			);
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
		if let Some(search) = posts_search(&scenario) {
			for statement in SEARCH_WRITES {
				cluster.run(&migrated, statement);
			}

			assert_search(&scenario, &search, |word| {
				cluster
					.query(
						&migrated,
						&format!(
							"SELECT count(*) FROM posts WHERE lk_search @@ plainto_tsquery('{}', \
							 '{word}')",
							search.language
						),
					)
					.trim()
					.parse()
					.unwrap()
			});
		}

		assert_rows(&scenario, |table| {
			let qualified = match table.split_once('_') {
				Some((namespace @ ("auth" | "content" | "reports"), name)) => {
					format!("{namespace}.\"{name}\"")
				}
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
	stand_in_for_geo_functions(&connection);
	connection
		.pragma_update(None, "foreign_keys", true)
		.unwrap();
	apply_sqlite(&connection, create, "create");
	connection
}

/// The functions geospatial columns' checks and spatial indexes' triggers
/// call, which the database layer registers on every connection. These
/// stand-ins only let the migrations run; `leptoskit_db`'s own tests run the
/// real ones.
fn stand_in_for_geo_functions(connection: &Connection) {
	use rusqlite::functions::FunctionFlags;

	let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;

	for name in ["ST_MinX", "ST_MaxX", "ST_MinY", "ST_MaxY"] {
		connection
			.create_scalar_function(name, 1, flags, |_| Ok(Option::<f64>::None))
			.unwrap();
	}

	connection
		.create_scalar_function("ST_IsEmpty", 1, flags, |_| Ok(1))
		.unwrap();
	connection
		.create_scalar_function("lk_geo_check", 4, flags, |_| Ok(1))
		.unwrap();
	connection
		.create_scalar_function("lk_st_isvalid", 1, flags, |_| Ok(1))
		.unwrap();
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

		let mut triggers: Vec<String> = connection
			.prepare("SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = ?1")
			.unwrap()
			.query_map([&table], |row| row.get::<_, String>(0))
			.unwrap()
			.map(|sql| format!("{table} trigger {}", sql.unwrap()))
			.collect();
		triggers.sort();
		lines.extend(triggers);
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
		let Some((binaries, server)) = Self::find_binaries() else {
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
		// No pipes: on Windows the server inherits pg_ctl's output handles, so
		// waiting for them to close would wait for the server to exit. Its
		// output goes to the log instead.
		let log = directory.path().join("log");
		let started = Command::new(binaries.join("pg_ctl"))
			.arg("-D")
			.arg(&data)
			// The `postgres` beside where the binaries were found: a Nix
			// `postgresql.withPackages` environment links `initdb` to plain
			// Postgres but builds its own `postgres`, which finds the
			// extensions.
			.arg("-p")
			.arg(&server)
			.arg("-l")
			.arg(&log)
			.arg("-o")
			.arg(format!(
				"-p {port} -c listen_addresses=127.0.0.1 -F{socket_option}"
			))
			.args(["-w", "start"])
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status()
			.expect("run pg_ctl");
		assert!(
			started.success(),
			"pg_ctl couldn't start Postgres:\n{}",
			fs::read_to_string(&log).unwrap_or_default()
		);

		Some(Self {
			binaries,
			directory,
			port,
		})
	}

	/// Postgres's binary directory: `LEPTOSKIT_POSTGRES_BIN`, then `PGBIN`
	/// (set on GitHub's Windows runners), then wherever `initdb` is on the
	/// path, then the newest Debian-style install (GitHub's Ubuntu runners).
	fn find_binaries() -> Option<(PathBuf, PathBuf)> {
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

		let initdb = from_env
			.chain(from_path)
			.chain(debian)
			.find(|directory| has_initdb(directory))?;
		let server = initdb.join(if cfg!(windows) {
			"postgres.exe"
		} else {
			"postgres"
		});
		Some((resolve_symlinks(&initdb)?, server))
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
				"SELECT schemaname, tablename, policyname, cmd, qual, with_check FROM pg_policies \
				 WHERE schemaname {USER_SCHEMAS} ORDER BY 1, 2, 3"
			),
			format!(
				"SELECT n.nspname, c.relname, c.relrowsecurity, c.relforcerowsecurity FROM \
				 pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind = 'r' \
				 AND n.nspname {USER_SCHEMAS} ORDER BY 1, 2"
			),
			format!(
				"SELECT n.nspname, c.relname, c.relkind, c.reloptions FROM pg_class c JOIN \
				 pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('v', 'm') AND \
				 n.nspname {USER_SCHEMAS} ORDER BY 1, 2"
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
		// PostGIS's own table and views stay when the last shape goes, with
		// the extension, which a migration never drops.
		const POSTGIS: [&str; 3] = [
			"|spatial_ref_sys|",
			"|geometry_columns|",
			"|geography_columns|",
		];
		queries
			.iter()
			.flat_map(|query| {
				let mut lines: Vec<String> = self
					.query(database, query)
					.lines()
					.filter(|line| !POSTGIS.iter().any(|name| line.contains(name)))
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
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status();
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

/// The real directory behind `directory`'s `initdb`. On Unix, Nix profiles
/// and Homebrew link the binaries, and initdb finds its data files relative
/// to where it really is. Windows needs no resolving, and canonical paths
/// there take the `\\?\` form that initdb can't use.
fn resolve_symlinks(directory: &Path) -> Option<PathBuf> {
	if cfg!(windows) {
		return Some(directory.to_path_buf());
	}

	fs::canonicalize(directory.join("initdb"))
		.ok()?
		.parent()
		.map(Path::to_path_buf)
}
