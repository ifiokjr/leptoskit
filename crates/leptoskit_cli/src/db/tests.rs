//! `leptoskit db` without compiling an app: the target schema is built by
//! hand, as the app's export would produce it.

use leptoskit_config::DbConfig;
use leptoskit_db_schema::Column;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::Index;
use leptoskit_db_schema::NoRenames;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::Table;
use leptoskit_db_schema::TableName;

use super::CreateOptions;
use super::History;
use super::Project;
use super::check;
use super::create;
use super::slug;

fn project(root: &std::path::Path, dialect: leptoskit_config::DbDialect) -> Project {
	Project {
		root: root.to_path_buf(),
		config: DbConfig {
			dialect,
			..DbConfig::default()
		},
		package: "app_db".into(),
		name: "app".into(),
	}
}

fn users(dialect: Dialect, columns: Vec<Column>) -> Schema {
	let name = TableName::new("users");
	let mut all = vec![Column::new("id", ColumnType::Uuid)];
	all.extend(columns);
	let mut table = Table::new(name.clone(), all, vec!["id".into()]);
	table.indexes.push(Index::new(&name, &["id"]));
	Schema::new(dialect, vec![table])
}

fn options() -> CreateOptions {
	CreateOptions {
		allow_destructive: false,
		empty: false,
	}
}

#[test]
fn migrations_build_on_the_latest_snapshot() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let first = users(
		Dialect::Sqlite,
		vec![Column::new("handle", ColumnType::TEXT)],
	);
	let created = create(
		&project,
		"Create users!",
		&first,
		&options(),
		&mut RequireHints,
	)
	.unwrap();

	assert!(created.name.ends_with("_create_users"), "{}", created.name);
	assert_eq!(created.steps, ["+ table users (2 columns)"]);
	assert!(created.summary().contains("wrote "));

	let mut second = first.clone();
	second.tables[0]
		.columns
		.push(Column::new("bio", ColumnType::TEXT).nullable());
	// A rename hint from the code: written into the migration, never into the
	// snapshot.
	second.tables[0].columns[1].name = "username".into();
	second.tables[0].columns[1].renamed_from = Some("handle".into());
	std::thread::sleep(std::time::Duration::from_secs(1));
	let created = create(&project, "add bio", &second, &options(), &mut RequireHints).unwrap();
	assert_eq!(
		created.steps,
		[
			"~ column users.handle renamed to username",
			"+ column users.bio (text)"
		]
	);

	let history = History::read(&project.migrations_dir(), Dialect::Sqlite).unwrap();
	assert_eq!(history.names.len(), 2);
	assert_eq!(history.latest, second.without_rename_hints());
	assert!(
		!std::fs::read_to_string(created.directory.join("schema.json"))
			.unwrap()
			.contains("renamed_from")
	);

	let registry = std::fs::read_to_string(project.migrations_dir().join("registry.txt")).unwrap();
	assert!(registry.starts_with("# The migrations, in the order they apply."));
	let migration = std::fs::read_to_string(created.directory.join("migration.sql")).unwrap();
	assert!(
		migration.contains("RENAME COLUMN \"handle\" TO \"username\""),
		"{migration}"
	);

	// The same tables again: nothing to write, unless it's a data migration.
	let error = create(&project, "again", &second, &options(), &mut RequireHints).unwrap_err();
	assert!(
		error
			.to_string()
			.starts_with("the tables haven't changed since ")
	);
	std::thread::sleep(std::time::Duration::from_secs(1));
	let data = CreateOptions {
		empty: true,
		..options()
	};
	let created = create(&project, "backfill", &second, &data, &mut RequireHints).unwrap();
	assert_eq!(created.steps, [] as [String; 0]);
	let sql = std::fs::read_to_string(created.directory.join("migration.sql")).unwrap();
	assert!(sql.contains("A data migration"));
}

#[test]
fn deleting_data_needs_consent() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Postgres);
	let first = users(
		Dialect::Postgres,
		vec![Column::new("handle", ColumnType::TEXT)],
	);
	create(&project, "create", &first, &options(), &mut RequireHints).unwrap();

	let without_handle = users(Dialect::Postgres, Vec::new());
	let error = create(
		&project,
		"drop handle",
		&without_handle,
		&options(),
		&mut RequireHints,
	)
	.unwrap_err();
	insta::assert_snapshot!(error.to_string());

	let consent = CreateOptions {
		allow_destructive: true,
		..options()
	};
	std::thread::sleep(std::time::Duration::from_secs(1));
	let created = create(
		&project,
		"drop handle",
		&without_handle,
		&consent,
		&mut RequireHints,
	)
	.unwrap();
	assert_eq!(
		created.notes,
		["loses data: every value in `users.handle` is deleted"]
	);

	let mut data = CreateOptions {
		empty: true,
		..options()
	};
	data.allow_destructive = true;
	let with_note = users(
		Dialect::Postgres,
		vec![Column::new("note", ColumnType::TEXT).nullable()],
	);
	let error = create(&project, "data", &with_note, &data, &mut NoRenames).unwrap_err();
	assert!(
		error
			.to_string()
			.starts_with("the tables changed since the latest migration")
	);
}

#[test]
fn ambiguous_renames_need_an_answer() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	create(
		&project,
		"create",
		&users(
			Dialect::Sqlite,
			vec![Column::new("handle", ColumnType::TEXT).nullable()],
		),
		&options(),
		&mut RequireHints,
	)
	.unwrap();
	let renamed = users(
		Dialect::Sqlite,
		vec![Column::new("username", ColumnType::TEXT).nullable()],
	);

	let error = create(&project, "rename", &renamed, &options(), &mut RequireHints).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("If it was renamed, add #[db(renamed_from")
	);
	let consent = CreateOptions {
		allow_destructive: true,
		..options()
	};
	std::thread::sleep(std::time::Duration::from_secs(1));
	let created = create(&project, "replace", &renamed, &consent, &mut NoRenames).unwrap();
	assert_eq!(
		created.steps,
		["+ column users.username (text)", "- column users.handle"]
	);
}

#[test]
fn check_fails_when_the_tables_drift() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let schema = users(Dialect::Sqlite, Vec::new());
	let empty = History::read(&project.migrations_dir(), Dialect::Sqlite).unwrap();
	assert!(empty.names.is_empty() && empty.latest_name().is_none());
	insta::assert_snapshot!(check(&empty, &schema).unwrap_err().to_string());

	create(&project, "create", &schema, &options(), &mut RequireHints).unwrap();
	let history = History::read(&project.migrations_dir(), Dialect::Sqlite).unwrap();
	check(&history, &schema).unwrap();

	let mut grown = schema.clone();
	grown.tables[0]
		.columns
		.push(Column::new("required", ColumnType::TEXT));
	let error = check(&history, &grown).unwrap_err().to_string();
	assert!(error.contains("is NOT NULL with no default"), "{error}");
}

#[test]
fn the_history_must_be_consistent() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let directory = project.migrations_dir();
	let created = create(
		&project,
		"create",
		&users(Dialect::Sqlite, Vec::new()),
		&options(),
		&mut RequireHints,
	)
	.unwrap();

	let error = History::read(&directory, Dialect::Postgres).err().unwrap();
	assert_eq!(
		error.to_string(),
		"the migrations are for sqlite, but leptoskit.toml's db.dialect is postgres"
	);

	std::fs::create_dir(directory.join("20990101T000000_stray")).unwrap();
	let error = History::read(&directory, Dialect::Sqlite).err().unwrap();
	assert!(
		error
			.to_string()
			.starts_with("migration `20990101T000000_stray` isn't listed in registry.txt")
	);
	std::fs::remove_dir(directory.join("20990101T000000_stray")).unwrap();

	std::fs::remove_file(created.directory.join("schema.json")).unwrap();
	let error = History::read(&directory, Dialect::Sqlite).err().unwrap();
	assert!(
		error.to_string().contains("schema.json is missing"),
		"{error}"
	);
}

#[test]
fn migration_names_become_slugs() {
	assert_eq!(slug("Add Posts & Tags").unwrap(), "add_posts_tags");
	assert_eq!(
		slug("  --backfill--handles--  ").unwrap(),
		"backfill_handles"
	);
	assert!(slug("!!!").is_err());
}

/// A project in push mode, and a URL to each database the test runs on:
/// SQLite in a file, then the development Postgres when its binaries are
/// around, which stays up while the server returned with it lives.
fn push_databases(
	root: &std::path::Path,
) -> Vec<(Project, String, Option<super::dev_server::DevServer>)> {
	let mut sqlite = project(root, leptoskit_config::DbDialect::Sqlite);
	sqlite.config.watch = leptoskit_config::DbWatch::Push;
	sqlite.config.migrations = "sqlite_migrations".into();
	let url = format!("sqlite://{}", root.join("dev.sqlite3").display());
	let mut databases = vec![(sqlite, url, None)];

	if let Some(binaries) = super::dev_server::test_binaries() {
		let mut postgres = project(root, leptoskit_config::DbDialect::Postgres);
		postgres.config.watch = leptoskit_config::DbWatch::Push;
		postgres.config.migrations = "postgres_migrations".into();
		postgres.config.dev_server = leptoskit_config::DbDevServer::Embedded;
		let server = super::dev_server::start(
			&postgres,
			super::dev_server::Source::Embedded,
			Some(&binaries),
		)
		.unwrap();
		let url = server.url.clone();
		databases.push((postgres, url, Some(server)));
	}

	databases
}

fn applied_and_pushed(url: &str) -> (Vec<String>, bool) {
	super::runtime()
		.unwrap()
		.block_on(async {
			let db = leptoskit_db::Db::connect(url).await?;
			anyhow::Ok((
				db.applied_migrations().await?,
				db.pushed_schema().await?.is_some(),
			))
		})
		.unwrap()
}

#[test]
fn pushes_follow_the_tables_and_squash_into_one_migration() {
	use super::push::Pushed;
	use super::push::push;

	let root = tempfile::tempdir().unwrap();

	for (project, url, _server) in push_databases(root.path()) {
		let dialect = project.dialect();
		let created = create(
			&project,
			"users",
			&users(dialect, vec![Column::new("handle", ColumnType::TEXT)]),
			&options(),
			&mut RequireHints,
		)
		.unwrap();

		// The first push applies the migrations, then the change.
		let grown = users(
			dialect,
			vec![
				Column::new("handle", ColumnType::TEXT),
				Column::new("bio", ColumnType::TEXT).nullable(),
			],
		);
		let pushed = push(&project, &url, &grown, false).unwrap();
		assert_eq!(
			pushed,
			Pushed::Applied(vec!["+ column users.bio (text)".into()])
		);
		assert!(pushed.message().unwrap().contains("+ column users.bio"));
		assert_eq!(
			push(&project, &url, &grown, false).unwrap(),
			Pushed::Unchanged
		);
		assert_eq!(Pushed::Unchanged.message(), None);
		assert_eq!(applied_and_pushed(&url), (vec![created.name.clone()], true));

		// Deleting data waits for consent, and says how to give it.
		let shrunk = users(
			dialect,
			vec![
				Column::new("bio", ColumnType::TEXT).nullable(),
				Column::new("rank", ColumnType::Int32).nullable(),
			],
		);
		let refused = push(&project, &url, &shrunk, false).unwrap();
		let Pushed::Refused(steps) = &refused else {
			panic!("dropping a column deletes data: {refused:?}");
		};
		assert!(
			steps
				.iter()
				.any(|step| step.starts_with("- column users.handle")),
			"{steps:?}"
		);
		assert!(refused.message().unwrap().contains("--allow-destructive"));
		assert!(matches!(
			push(&project, &url, &shrunk, true).unwrap(),
			Pushed::Applied(_)
		));

		// The migration starts from the latest one, and the database takes
		// it over without running it: its rows stay.
		let id = match dialect {
			Dialect::Postgres => "'00000000-0000-0000-0000-000000000001'",
			Dialect::Sqlite => "x'00000000000000000000000000000001'",
		};
		super::runtime()
			.unwrap()
			.block_on(async {
				let db = leptoskit_db::Db::connect(&url).await?;
				db.run_script(&format!(
					"INSERT INTO \"users\" (\"id\", \"bio\") VALUES ({id}, 'kept')"
				))
				.await
			})
			.unwrap();
		std::thread::sleep(std::time::Duration::from_secs(1));
		let squashed = super::create_squashing(
			&project,
			"profiles",
			&shrunk,
			&CreateOptions {
				allow_destructive: true,
				empty: false,
			},
			&mut NoRenames,
			Some(&url),
		)
		.unwrap();
		assert_eq!(
			applied_and_pushed(&url),
			(vec![created.name.clone(), squashed.name.clone()], false)
		);
		let sql = std::fs::read_to_string(squashed.directory.join("migration.sql")).unwrap();
		assert!(sql.contains("rank") && sql.contains("bio"), "{sql}");
		let rows = super::runtime()
			.unwrap()
			.block_on(async {
				let db = leptoskit_db::Db::connect(&url).await?;
				use leptoskit_db::Connection;
				db.fetch(leptoskit_db::Statement {
					sql: "SELECT count(*) FROM \"users\" WHERE \"bio\" = 'kept'".into(),
					args: Vec::new(),
				})
				.await
			})
			.unwrap();
		assert_eq!(rows, [[leptoskit_db::Value::Int(1)]]);

		// Without pushes, a migration leaves the database alone.
		std::thread::sleep(std::time::Duration::from_secs(1));
		let later = users(
			dialect,
			vec![
				Column::new("bio", ColumnType::TEXT).nullable(),
				Column::new("rank", ColumnType::Int32).nullable(),
				Column::new("motto", ColumnType::TEXT).nullable(),
			],
		);
		super::create_squashing(
			&project,
			"mottos",
			&later,
			&options(),
			&mut RequireHints,
			Some(&url),
		)
		.unwrap();
		assert_eq!(applied_and_pushed(&url).0.len(), 2);
	}
}

#[test]
fn squashing_catches_up_first_and_fails_closed() {
	use super::push::push;

	let root = tempfile::tempdir().unwrap();

	for (project, url, _server) in push_databases(root.path()) {
		let dialect = project.dialect();
		let handle = || Column::new("handle", ColumnType::TEXT);
		let bio = || Column::new("bio", ColumnType::TEXT).nullable();
		create(
			&project,
			"users",
			&users(dialect, vec![handle()]),
			&options(),
			&mut RequireHints,
		)
		.unwrap();
		push(
			&project,
			&url,
			&users(dialect, vec![handle(), bio()]),
			false,
		)
		.unwrap();

		// The pushes are behind the tables in a way that deletes data: nothing
		// is written without consent.
		let without_handle = users(dialect, vec![bio()]);
		let error = super::create_squashing(
			&project,
			"profiles",
			&without_handle,
			&options(),
			&mut RequireHints,
			Some(&url),
		)
		.unwrap_err();
		assert!(error.to_string().contains("behind the tables"), "{error}");
		assert_eq!(
			History::read(&project.migrations_dir(), dialect)
				.unwrap()
				.names
				.len(),
			1
		);

		// Behind in a way that keeps rows: caught up, then adopted.
		std::thread::sleep(std::time::Duration::from_secs(1));
		let rank = users(
			dialect,
			vec![
				handle(),
				bio(),
				Column::new("rank", ColumnType::Int32).nullable(),
			],
		);
		super::create_squashing(
			&project,
			"ranks",
			&rank,
			&options(),
			&mut RequireHints,
			Some(&url),
		)
		.unwrap();
		let (applied, pushed) = applied_and_pushed(&url);
		assert_eq!((applied.len(), pushed), (2, false));

		// Pushing back to the latest migration leaves nothing pushed.
		push(
			&project,
			&url,
			&users(
				dialect,
				vec![
					handle(),
					bio(),
					Column::new("x", ColumnType::TEXT).nullable(),
					Column::new("rank", ColumnType::Int32).nullable(),
				],
			),
			false,
		)
		.unwrap();
		assert!(applied_and_pushed(&url).1);
		push(&project, &url, &rank, true).unwrap();
		assert!(!applied_and_pushed(&url).1);

		// With pushes, a teammate's migration doesn't land on top of them.
		push(
			&project,
			&url,
			&users(
				dialect,
				vec![
					handle(),
					bio(),
					Column::new("rank", ColumnType::Int32).nullable(),
					Column::new("y", ColumnType::TEXT).nullable(),
				],
			),
			false,
		)
		.unwrap();
		std::thread::sleep(std::time::Duration::from_secs(1));
		create(
			&project,
			"theirs",
			&users(
				dialect,
				vec![
					handle(),
					bio(),
					Column::new("rank", ColumnType::Int32).nullable(),
					Column::new("z", ColumnType::TEXT).nullable(),
				],
			),
			&options(),
			&mut RequireHints,
		)
		.unwrap();
		let error = push(&project, &url, &rank, true).unwrap_err();
		assert!(
			format!("{error:#}").contains("pushed in development"),
			"{error:#}"
		);
	}
}

#[test]
fn the_push_and_branch_commands_say_what_they_did() {
	let root = tempfile::tempdir().unwrap();
	let mut sqlite = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	sqlite.config.watch = leptoskit_config::DbWatch::Push;
	let url = format!("sqlite://{}", root.path().join("dev.sqlite3").display());
	let handle = || Column::new("handle", ColumnType::TEXT);
	super::push_command(
		&sqlite,
		&users(Dialect::Sqlite, vec![handle()]),
		&url,
		false,
	)
	.unwrap();
	super::push_command(
		&sqlite,
		&users(Dialect::Sqlite, vec![handle()]),
		&url,
		false,
	)
	.unwrap();
	let refused =
		super::push_command(&sqlite, &users(Dialect::Sqlite, Vec::new()), &url, false).unwrap_err();
	assert!(
		refused.to_string().contains("nothing was pushed"),
		"{refused}"
	);

	// Branch databases need the development Postgres.
	let off = super::branch(&sqlite, &super::BranchCommand::List, None).unwrap_err();
	assert!(off.to_string().contains("development Postgres"), "{off}");

	let Some(binaries) = super::dev_server::test_binaries() else {
		return;
	};
	let git = |args: &[&str]| {
		let status = std::process::Command::new("git")
			.arg("-C")
			.arg(root.path())
			.args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
			.args(args)
			.env("GIT_CONFIG_GLOBAL", root.path().join("no-global-config"))
			.env("GIT_CONFIG_NOSYSTEM", "1")
			.stdout(std::process::Stdio::null())
			.stderr(std::process::Stdio::null())
			.status()
			.unwrap();
		assert!(status.success(), "git {args:?}");
	};
	let mut postgres = project(root.path(), leptoskit_config::DbDialect::Postgres);
	postgres.config.dev_server = leptoskit_config::DbDevServer::Embedded;
	postgres.config.url_env = "LEPTOSKIT_TEST_NEVER_SET".into();

	// Outside a git repository there are no branches to prune.
	super::branch(&postgres, &super::BranchCommand::List, Some(&binaries)).unwrap();
	let outside = super::branch(
		&postgres,
		&super::BranchCommand::Prune { yes: false },
		Some(&binaries),
	)
	.unwrap_err();
	assert!(outside.to_string().contains("git repository"), "{outside}");

	git(&["init", "--quiet", "--initial-branch=main"]);
	git(&["commit", "--quiet", "--allow-empty", "-m", "first"]);
	git(&["checkout", "--quiet", "-b", "done"]);
	super::branch(&postgres, &super::BranchCommand::List, Some(&binaries)).unwrap();
	super::branch(
		&postgres,
		&super::BranchCommand::Prune { yes: true },
		Some(&binaries),
	)
	.unwrap();
	git(&["checkout", "--quiet", "main"]);

	for yes in [false, true] {
		super::branch(
			&postgres,
			&super::BranchCommand::Prune { yes },
			Some(&binaries),
		)
		.unwrap();
	}

	let server = super::dev_server::start(
		&postgres,
		super::dev_server::Source::Embedded,
		Some(&binaries),
	)
	.unwrap();
	let url = super::dev_server::with_database(&server.url, "postgres").unwrap();
	assert_eq!(
		super::branches::list(&postgres, &url).unwrap(),
		[] as [super::branches::BranchDatabase; 0]
	);
}

#[test]
fn the_studio_turns_on_through_whichever_crate_the_db_crate_uses() {
	let manifest = |dependencies: &str| -> toml::Table {
		format!("[package]\nname = \"app_db\"\n\n[dependencies]\n{dependencies}\n")
			.parse()
			.unwrap()
	};

	assert_eq!(
		super::studio_features(
			&manifest("leptoskit_db = \"0.1\""),
			leptoskit_config::DbDialect::Sqlite
		)
		.unwrap(),
		["leptoskit_db/studio", "leptoskit_db/sqlite"]
	);
	assert_eq!(
		super::studio_features(
			&manifest("leptoskit = { version = \"0.1\", features = [\"db\"] }"),
			leptoskit_config::DbDialect::Postgres
		)
		.unwrap(),
		["leptoskit/db-studio", "leptoskit/db-postgres"]
	);
	let error = super::studio_features(
		&manifest("serde = \"1\""),
		leptoskit_config::DbDialect::Postgres,
	)
	.unwrap_err();
	assert!(
		error
			.to_string()
			.contains("neither leptoskit_db nor leptoskit")
	);
}

#[test]
fn the_studio_runs_the_export_test_with_its_settings() {
	let root = tempfile::tempdir().unwrap();
	std::fs::write(
		root.path().join("Cargo.toml"),
		"[workspace]\nmembers = [\"app_db\"]\nresolver = \"2\"\n",
	)
	.unwrap();
	std::fs::create_dir_all(root.path().join("app_db/src")).unwrap();
	std::fs::write(
		root.path().join("app_db/Cargo.toml"),
		"[package]\nname = \"app_db\"\nversion = \"0.0.0\"\nedition = \
		 \"2024\"\n\n[dependencies]\nleptoskit_db = \"0.1\"\n",
	)
	.unwrap();
	std::fs::write(root.path().join("app_db/src/lib.rs"), "").unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);

	// `cargo metadata --no-deps` reads the manifest without resolving it.
	let command = project.studio("sqlite::memory:", 4000).unwrap();
	let args: Vec<String> = command
		.get_args()
		.map(|arg| arg.to_string_lossy().into_owned())
		.collect();
	assert_eq!(
		args,
		[
			"test",
			"--quiet",
			"--package",
			"app_db",
			"--lib",
			"--features",
			"server,leptoskit_db/studio,leptoskit_db/sqlite",
			"--",
			"__leptoskit_db_export_schema",
			"--nocapture",
		]
	);
	let env: std::collections::HashMap<String, String> = command
		.get_envs()
		.filter_map(|(name, value)| {
			Some((
				name.to_string_lossy().into_owned(),
				value?.to_string_lossy().into_owned(),
			))
		})
		.collect();
	assert_eq!(env[leptoskit_db::STUDIO_ENV], "127.0.0.1:4000");
	assert_eq!(env[leptoskit_db::STUDIO_DATABASE_ENV], "sqlite::memory:");
	assert_eq!(env[leptoskit_db::DIALECT_ENV], "sqlite");
	assert!(env[leptoskit_db::STUDIO_MIGRATIONS_ENV].ends_with("migrations"));
}
