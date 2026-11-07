//! `leptoskit db` without compiling an app: the target schema is built by
//! hand, as the app's export would produce it.

use leptoskit_config::DbConfig;
use leptoskit_db_schema::Column;
use leptoskit_db_schema::ColumnType;
use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::Index;
use leptoskit_db_schema::NoRenames;
use leptoskit_db_schema::Pack;
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
		drop_packs: Vec::new(),
	}
}

#[test]
fn reverse_generation_preserves_renames_and_marks_data_loss() {
	for dialect in [Dialect::Sqlite, Dialect::Postgres] {
		let before = users(dialect, vec![Column::new("handle", ColumnType::TEXT)]);
		let mut after = before.clone();
		after.tables[0].name = TableName::new("people");
		after.tables[0].renamed_from = Some(TableName::new("users"));
		after.tables[0].columns[1].name = "username".into();
		after.tables[0].columns[1].renamed_from = Some("handle".into());
		let forward = leptoskit_db_schema::diff(&before, &after, &mut RequireHints).unwrap();
		let down = super::reverse_sql(&before, &after, &forward, false).unwrap();
		assert!(down.contains("RENAME TO \"users\""), "{down}");
		assert!(
			down.contains("RENAME COLUMN \"username\" TO \"handle\""),
			"{down}"
		);
		assert!(!down.contains("irreversible"));
		assert!(
			super::reverse_sql(&before, &after, &forward, true)
				.unwrap()
				.contains("-- leptoskit: irreversible")
		);
		let removed = users(dialect, vec![]);
		let forward = leptoskit_db_schema::diff(&before, &removed, &mut NoRenames).unwrap();
		assert!(
			super::reverse_sql(&before, &removed, &forward, false)
				.unwrap()
				.contains("-- leptoskit: irreversible")
		);
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
		let pushed = push(&project, &url, &grown, false, &[]).unwrap();
		assert_eq!(
			pushed,
			Pushed::Applied(vec!["+ column users.bio (text)".into()])
		);
		assert!(pushed.message().unwrap().contains("+ column users.bio"));
		assert_eq!(
			push(&project, &url, &grown, false, &[]).unwrap(),
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
		let refused = push(&project, &url, &shrunk, false, &[]).unwrap();
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
			push(&project, &url, &shrunk, true, &[]).unwrap(),
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
				drop_packs: Vec::new(),
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
				use leptoskit_db::Read;
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
			&[],
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
			&[],
		)
		.unwrap();
		assert!(applied_and_pushed(&url).1);
		push(&project, &url, &rank, true, &[]).unwrap();
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
			&[],
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
		let error = push(&project, &url, &rank, true, &[]).unwrap_err();
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
		&[],
	)
	.unwrap();
	super::push_command(
		&sqlite,
		&users(Dialect::Sqlite, vec![handle()]),
		&url,
		false,
		&[],
	)
	.unwrap();
	let refused = super::push_command(
		&sqlite,
		&users(Dialect::Sqlite, Vec::new()),
		&url,
		false,
		&[],
	)
	.unwrap_err();
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
		let status = crate::git::command(root.path())
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

#[test]
fn merging_accounts_needs_every_unique_reference_covered() {
	use leptoskit_db_schema::ForeignKey;
	use leptoskit_db_schema::MergeStrategy;

	// Without sign-in's tables there's nothing to check.
	let plain = users(Dialect::Postgres, Vec::new());
	assert_eq!(super::check_account_merges(&plain).unwrap(), None);

	let accounts = TableName::namespaced("auth", "accounts");
	let follows = TableName::new("follows");
	let mut follow = Table::new(
		follows.clone(),
		vec![
			Column::new("follower_id", ColumnType::Uuid),
			Column::new("followee_id", ColumnType::Uuid),
		],
		vec!["follower_id".into(), "followee_id".into()],
	);

	for column in ["follower_id", "followee_id"] {
		follow.foreign_keys.push(ForeignKey::new(
			&follows,
			&[column],
			accounts.clone(),
			&["id"],
		));
	}

	let mut schema = Schema::new(
		Dialect::Postgres,
		vec![
			Table::new(
				accounts,
				vec![Column::new("id", ColumnType::Uuid)],
				vec!["id".into()],
			),
			follow,
		],
	);
	let error = super::check_account_merges(&schema)
		.unwrap_err()
		.to_string();
	assert!(
		error.starts_with("merging accounts can't bring their data along"),
		"{error}"
	);
	assert!(
		error.contains("follows.follower_id is in a unique constraint"),
		"{error}"
	);

	for table in &mut schema.tables {
		if table.name == follows {
			table.on_account_merge = Some(MergeStrategy::ReassignSkipConflicts);
		}
	}

	assert_eq!(
		super::check_account_merges(&schema).unwrap().unwrap(),
		"merging accounts brings their data along: 0 table(s) keyed by the account, 2 \
		 reference(s) to it"
	);
}

#[test]
fn check_live_finds_a_missing_or_extra_trigger() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let url = format!("sqlite://{}", root.path().join("check.sqlite").display());
	let table = TableName::new("notes");
	let mut schema = Schema::new(
		Dialect::Sqlite,
		vec![Table::new(
			table.clone(),
			vec![Column::new("id", ColumnType::Uuid)],
			vec!["id".into()],
		)],
	);
	schema = schema.with_routines(
		Vec::new(),
		Vec::new(),
		vec![leptoskit_db_schema::Trigger {
			name: "live_notes_insert".into(),
			table: table.clone(),
			timing: leptoskit_db_schema::TriggerTiming::After,
			events: vec![leptoskit_db_schema::TriggerEvent::Insert],
			deferred: false,
			columns: vec!["id".into()],
			action: leptoskit_db_schema::TriggerAction::Statements {
				when: None,
				sql: "SELECT 1;".into(),
			},
		}],
	);

	let runtime = super::runtime().unwrap();
	runtime.block_on(async {
		let db = super::Db::connect(&url).await.unwrap();
		db.run_script(
			"CREATE TABLE notes (id BLOB PRIMARY KEY);\nCREATE TRIGGER live_notes_insert AFTER \
			 INSERT ON notes FOR EACH ROW BEGIN SELECT 1; END;",
		)
		.await
		.unwrap();
	});
	super::check_live(&project, &schema, Some(url.clone())).unwrap();

	runtime.block_on(async {
		let db = super::Db::connect(&url).await.unwrap();
		db.run_script("DROP TRIGGER live_notes_insert;")
			.await
			.unwrap();
	});
	let error = super::check_live(&project, &schema, Some(url)).unwrap_err();
	assert!(
		error.to_string().contains("missing live triggers"),
		"{error}"
	);
}

#[test]
fn check_warns_about_public_vectors_and_large_exact_searches() {
	use leptoskit_db_schema::VectorDistance;
	use leptoskit_db_schema::VectorElement;
	use leptoskit_db_schema::VectorOptions;

	let mut articles = Table::new(
		TableName::new("articles"),
		vec![
			Column::new("id", ColumnType::Int64),
			Column::new(
				"embedding",
				ColumnType::Vector {
					element: VectorElement::Float32,
					dimensions: 3,
				},
			),
			Column::new(
				"code",
				ColumnType::Vector {
					element: VectorElement::Bit,
					dimensions: 8,
				},
			),
		],
		vec!["id".into()],
	);
	articles.vectors.push(VectorOptions::new(
		"embedding",
		Some(VectorDistance::Cosine),
	));
	let mut code = VectorOptions::new("code", Some(VectorDistance::Hamming));
	code.public = true;
	articles.vectors.push(code);
	let schema = Schema::new(Dialect::Sqlite, vec![articles]);

	assert_eq!(
		super::public_vectors(&schema),
		[
			"`articles.code` is a vector clients can see: vectors are large, and an embedding can \
			 give away the text it was made from; drop `public` unless clients need it"
		]
	);
	assert_eq!(
		super::public_vectors(&schema.without_rename_hints()),
		Vec::<String>::new()
	);

	let counts = [
		(TableName::new("articles"), 150_000),
		(TableName::new("notes"), 10),
	];
	assert_eq!(
		super::exact_searches(&counts, 100_000),
		[
			"`articles` has 150000 rows, and SQLite measures every one in each vector search; \
			 past 100000 (`db.vector.exact_search_rows`) searches slow down, and Postgres's HNSW \
			 indexes suit it better"
		]
	);
	assert_eq!(
		super::exact_searches(&counts, 200_000),
		Vec::<String>::new()
	);
}

/// `lk db migrate` runs SQLite migrations in this process, so their
/// spatial and vector checks need the functions the database layer
/// registers on each connection.
#[test]
fn sqlite_connections_check_spatial_and_vector_columns() {
	super::runtime()
		.unwrap()
		.block_on(async {
			let db = leptoskit_db::Db::connect("sqlite::memory:").await?;
			db.run_script("SELECT lk_geo_check(NULL, 'Point', 4326, 1); SELECT vec_version();")
				.await?;
			anyhow::Ok(())
		})
		.unwrap();
}

#[test]
fn postgres_live_check_reports_changed_and_missing_catalog_objects() {
	use leptoskit_db_schema::Function;
	use leptoskit_db_schema::Sequence;
	use leptoskit_db_schema::Trigger;
	use leptoskit_db_schema::TriggerAction;
	use leptoskit_db_schema::TriggerEvent;
	use leptoskit_db_schema::TriggerTiming;
	let root = tempfile::tempdir().unwrap();
	let Some((project, url, _server)) = push_databases(root.path())
		.into_iter()
		.find(|(project, ..)| project.dialect() == Dialect::Postgres)
	else {
		assert!(
			std::env::var_os("LEPTOSKIT_REQUIRE_POSTGRES").is_none(),
			"Postgres is required for this test"
		);
		return;
	};
	let runtime = super::runtime().unwrap();
	let execute = |sql: &str| {
		runtime.block_on(async {
			let db = super::Db::connect(&url).await.unwrap();
			db.run_script(sql).await.unwrap();
		});
	};
	execute(
		"CREATE SCHEMA live; CREATE TABLE notes (id uuid PRIMARY KEY); CREATE TABLE live.other \
		 (id uuid PRIMARY KEY); CREATE SEQUENCE live.fixture_sequence; CREATE FUNCTION \
		 live.fixture_observe() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RETURN NEW; END;$$; \
		 CREATE TRIGGER live_notes_insert AFTER INSERT ON notes FOR EACH ROW EXECUTE FUNCTION \
		 live.fixture_observe(); CREATE TRIGGER live_other_insert AFTER INSERT ON live.other FOR \
		 EACH ROW EXECUTE FUNCTION live.fixture_observe();",
	);
	let function = Function::new(
		TableName::namespaced("live", "fixture_observe"),
		"trigger",
		"plpgsql",
		"BEGIN RETURN NEW; END;",
	);
	let trigger = |table: TableName, name: &str| {
		Trigger {
			name: name.into(),
			table,
			timing: TriggerTiming::After,
			events: vec![TriggerEvent::Insert],
			deferred: false,
			columns: vec!["id".into()],
			action: TriggerAction::Function {
				function: function.name.clone(),
				arguments: Vec::new(),
			},
		}
	};
	let mut schema = users(Dialect::Postgres, Vec::new()).with_routines(
		vec![Sequence::new(TableName::namespaced(
			"live",
			"fixture_sequence",
		))],
		vec![function.clone()],
		vec![
			trigger(TableName::new("notes"), "live_notes_insert"),
			trigger(TableName::namespaced("live", "other"), "live_other_insert"),
		],
	);
	let check = |schema: &Schema| super::check_live(&project, schema, Some(url.clone()));
	check(&schema).unwrap();
	let mut changed = schema.clone();
	changed.functions[0] = Function::new(
		function.name.clone(),
		"trigger",
		"plpgsql",
		"BEGIN RETURN NULL; END;",
	);
	assert!(
		check(&changed)
			.unwrap_err()
			.to_string()
			.contains("has changed")
	);
	execute("DROP TRIGGER live_notes_insert ON notes;");
	assert!(
		check(&schema)
			.unwrap_err()
			.to_string()
			.contains("missing live triggers")
	);
	execute(
		"CREATE TRIGGER live_notes_insert AFTER INSERT ON notes FOR EACH ROW EXECUTE FUNCTION \
		 live.fixture_observe(); CREATE TRIGGER extra_live_insert AFTER INSERT ON notes FOR EACH \
		 ROW EXECUTE FUNCTION live.fixture_observe();",
	);
	assert!(
		check(&schema)
			.unwrap_err()
			.to_string()
			.contains("live triggers the schema doesn't")
	);
	execute(
		"DROP TRIGGER extra_live_insert ON notes; DROP FUNCTION live.fixture_observe() CASCADE;",
	);
	schema.triggers.clear();
	assert!(
		check(&schema)
			.unwrap_err()
			.to_string()
			.contains("missing live functions")
	);
	execute(
		"CREATE FUNCTION live.fixture_observe() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN \
		 RETURN NEW; END;$$; DROP SEQUENCE live.fixture_sequence;",
	);
	assert!(
		check(&schema)
			.unwrap_err()
			.to_string()
			.contains("missing live sequences")
	);
	execute("CREATE SEQUENCE live.fixture_sequence;");
	check(&schema).unwrap();
}

/// A fresh connection to the SQLite file at `url`.
fn db(url: &str) -> leptoskit_db::Db {
	super::runtime()
		.unwrap()
		.block_on(async { super::Db::connect(url).await })
		.unwrap()
}

/// The migration names the database at `url` records.
fn applied(url: &str) -> Vec<String> {
	let database = db(url);
	super::runtime()
		.unwrap()
		.block_on(async { database.applied_migrations().await })
		.unwrap()
}

/// The first integer `sql` returns, opening `url` again.
fn scalar(url: &str, sql: &str) -> i64 {
	let database = db(url);
	let rows = super::runtime()
		.unwrap()
		.block_on(async {
			use leptoskit_db::Read as _;
			database
				.fetch(leptoskit_db::Statement {
					sql: sql.to_owned(),
					args: Vec::new(),
				})
				.await
		})
		.unwrap();
	match rows.first().and_then(|row| row.first()) {
		Some(leptoskit_db::Value::Int(count)) => *count,
		other => panic!("{sql} returned {other:?}; expected one integer"),
	}
}

/// Whether the `users` table still has the `bio` column.
fn bio_column(url: &str) -> i64 {
	scalar(
		url,
		"SELECT count(*) FROM pragma_table_info('users') WHERE name = 'bio'",
	)
}

/// Whether the `users` table itself still exists.
fn users_table(url: &str) -> i64 {
	scalar(
		url,
		"SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'users'",
	)
}

/// The one row the durable-state assertions count.
fn insert_kept_row(url: &str) {
	let database = db(url);
	super::runtime()
		.unwrap()
		.block_on(async {
			database
				.run_script(
					"INSERT INTO \"users\" (\"id\", \"handle\") VALUES \
					 (x'00000000000000000000000000000001', 'kept')",
				)
				.await
		})
		.unwrap();
}

/// Applies the first `count` migrations from the project, as the commands
/// themselves would, on whatever the database already has.
fn apply_migrations(project: &std::path::Path, url: &str, count: usize) {
	let already = applied(url).len();
	let migrations = leptoskit_db::load_migrations(&project.join("migrations")).unwrap();
	let database = db(url);
	let applied = super::runtime()
		.unwrap()
		.block_on(async { database.migrate(&migrations[..count]).await })
		.unwrap();
	assert_eq!(applied.len(), count - already);
}

/// `run()` resolves the project from the process-wide working directory and
/// the database URL from the environment, so each public command executes in
/// an isolated child test process. The parent prepares the project, checks
/// what the command printed, and then opens the database file itself to prove
/// what the command left behind.
mod child {
	use std::path::PathBuf;
	use std::process::Command;
	use std::process::Output;

	use leptoskit_db_schema::Column;
	use leptoskit_db_schema::ColumnType;
	use leptoskit_db_schema::Dialect;

	use super::users;
	use crate::db::DbCommand;
	use crate::db::run;

	/// The child test that executes one command; `LEPTOSKIT_TEST_DB_COMMAND`
	/// names it, and `LEPTOSKIT_TEST_DB_EXPECT` marks a refused one.
	const HELPER: &str = "db::tests::child::helper";
	/// A `PATH` with only system tools, for the schema-export fixture.
	const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

	/// A harmless `cargo` that hands over a prepared schema, or fails the
	/// way a scenario needs.
	#[cfg(unix)]
	fn export_script() -> &'static str {
		r#"#!/bin/sh
set -eu
case "${LEPTOSKIT_TEST_DB_EXPORT:-ok}" in
	ok) cp "$LEPTOSKIT_TEST_DB_SCHEMA_FILE" "$LEPTOSKIT_DB_SCHEMA_OUT" ;;
	missing) ;;
	invalid) printf 'not a schema at all' > "$LEPTOSKIT_DB_SCHEMA_OUT" ;;
	fail) printf 'the db crate failed to build\n' >&2
		exit 1 ;;
esac
"#
	}

	/// A project with two real migrations, and the SQLite URL they apply to.
	pub(super) struct Fixture {
		directory: tempfile::TempDir,
	}

	impl Fixture {
		pub(super) fn new() -> Self {
			let directory = tempfile::tempdir().unwrap();
			let project = directory.path().join("project");
			std::fs::create_dir_all(project.join("migrations")).unwrap();
			std::fs::write(
				project.join("leptoskit.toml"),
				"[db]\ndialect = \"sqlite\"\n",
			)
			.unwrap();

			let migrations = [
				(
					"20260101T000100_create_users",
					"CREATE TABLE \"users\" (\"id\" BLOB NOT NULL PRIMARY KEY, \"handle\" TEXT);\n",
					"DROP TABLE \"users\";\n",
					users(
						Dialect::Sqlite,
						vec![Column::new("handle", ColumnType::TEXT)],
					),
				),
				(
					"20260101T000200_add_bio",
					"ALTER TABLE \"users\" ADD COLUMN \"bio\" TEXT;\n",
					"ALTER TABLE \"users\" DROP COLUMN \"bio\";\n",
					users(
						Dialect::Sqlite,
						vec![
							Column::new("handle", ColumnType::TEXT),
							Column::new("bio", ColumnType::TEXT).nullable(),
						],
					),
				),
			];
			let mut registry = String::from("# The migrations, in the order they apply.\n");

			for (name, up, down, schema) in migrations {
				let directory = project.join("migrations").join(name);
				std::fs::create_dir(&directory).unwrap();
				std::fs::write(directory.join("migration.sql"), up).unwrap();
				std::fs::write(directory.join("down.sql"), down).unwrap();
				std::fs::write(directory.join("schema.json"), schema.to_json()).unwrap();
				registry.push_str(name);
				registry.push('\n');
			}

			std::fs::write(project.join("migrations").join("registry.txt"), registry).unwrap();
			Self { directory }
		}

		pub(super) fn project(&self) -> PathBuf {
			self.directory.path().join("project")
		}

		/// Where the fixture keeps its files, for byte-level assertions.
		pub(super) fn path(&self) -> &std::path::Path {
			self.directory.path()
		}

		pub(super) fn migrations(&self) -> PathBuf {
			self.project().join("migrations")
		}

		/// The database URL the commands work on: a real SQLite file.
		pub(super) fn url(&self) -> String {
			format!(
				"sqlite://{}",
				self.directory.path().join("dev.sqlite3").display()
			)
		}

		/// A `cargo` that exports the schema the migrations leave behind.
		#[cfg(unix)]
		pub(super) fn export_environment(&self) -> Vec<(&'static str, String)> {
			use std::os::unix::fs::PermissionsExt as _;

			let bin = self.directory.path().join("bin");
			std::fs::create_dir(&bin).unwrap();
			let cargo = bin.join("cargo");
			std::fs::write(&cargo, export_script()).unwrap();
			std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();

			let schema = self.directory.path().join("exported-schema.json");
			std::fs::write(
				&schema,
				users(
					Dialect::Sqlite,
					vec![
						Column::new("handle", ColumnType::TEXT),
						Column::new("bio", ColumnType::TEXT).nullable(),
					],
				)
				.to_json(),
			)
			.unwrap();
			vec![
				("CARGO", cargo.display().to_string()),
				(
					"LEPTOSKIT_TEST_DB_SCHEMA_FILE",
					schema.display().to_string(),
				),
			]
		}

		/// Runs one command in the child, with `environment` layered on top
		/// of the clean process it gets.
		fn run(&self, command: &str, environment: &[(&str, &str)]) -> Output {
			let mut child = Command::new(std::env::current_exe().unwrap());
			child
				.args(["--exact", HELPER, "--nocapture", "--test-threads=1"])
				.env_clear()
				.env("PATH", SYSTEM_PATH)
				.env("TMPDIR", self.directory.path())
				.env("LEPTOSKIT_TEST_DB_COMMAND", command)
				.current_dir(self.project());

			for (name, value) in environment {
				child.env(name, value);
			}

			// Windows networking needs the system root in the child's
			// environment; the parent's value is passed through without
			// reading anything else.
			if let Some(system_root) = std::env::var_os("SystemRoot") {
				child.env("SystemRoot", system_root);
			}

			if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
				child.env("LLVM_PROFILE_FILE", profile);
			}

			child.output().unwrap()
		}

		/// Runs `command`, asserting the child passed, and returns what the
		/// command printed.
		pub(super) fn run_ok(&self, command: &str, environment: &[(&str, &str)]) -> String {
			let output = self.run(command, environment);
			assert!(
				output.status.success(),
				"{command} failed\nstdout:\n{}\nstderr:\n{}",
				String::from_utf8_lossy(&output.stdout),
				String::from_utf8_lossy(&output.stderr),
			);
			String::from_utf8(output.stdout).unwrap()
		}

		/// Runs `command`, asserting the child passed and that the command
		/// was refused with `expected`.
		pub(super) fn run_refused(
			&self,
			command: &str,
			environment: &[(&str, &str)],
			expected: &str,
		) {
			let mut environment = environment.to_vec();
			environment.push(("LEPTOSKIT_TEST_DB_EXPECT", expected));
			self.run_ok(command, &environment);
		}
	}

	/// One command in a prepared project; see [`Fixture`].
	#[test]
	fn helper() {
		let Ok(command) = std::env::var("LEPTOSKIT_TEST_DB_COMMAND") else {
			return;
		};
		let url = || std::env::var("LEPTOSKIT_TEST_DB_URL").ok();
		let yes = std::env::var("LEPTOSKIT_TEST_DB_YES").as_deref() == Ok("1");
		let command = match command.as_str() {
			"migrate" => DbCommand::Migrate { url: url() },
			"rollback" => {
				DbCommand::Rollback {
					url: url(),
					steps: std::env::var("LEPTOSKIT_TEST_DB_STEPS")
						.ok()
						.and_then(|steps| steps.parse().ok())
						.unwrap_or(1),
					yes,
				}
			}
			"reset" => DbCommand::Reset { url: url(), yes },
			"status" => DbCommand::Status { url: url() },
			other => panic!("unknown command {other}"),
		};

		let result = run(command);

		if let Ok(expected) = std::env::var("LEPTOSKIT_TEST_DB_EXPECT") {
			let error = result.expect_err("the command should have been refused");
			assert!(
				error.to_string().contains(&expected),
				"expected {expected:?} in {error}"
			);
		} else {
			result.expect("the command should have succeeded");
		}
	}
}

#[test]
fn migrate_applies_the_pending_migrations_and_stops_when_current() {
	let fixture = child::Fixture::new();
	let url = fixture.url();

	// The first run takes the URL from the variable `db.url_env` names.
	let stdout = fixture.run_ok("migrate", &[("DATABASE_URL", &url)]);
	assert!(
		stdout.contains("applied 20260101T000100_create_users"),
		"{stdout}"
	);
	assert!(
		stdout.contains("applied 20260101T000200_add_bio"),
		"{stdout}"
	);
	assert_eq!(
		applied(&url),
		["20260101T000100_create_users", "20260101T000200_add_bio"]
	);
	insert_kept_row(&url);

	// A second run, this time with `--url`, has nothing to do, and the
	// row written in between stays.
	let stdout = fixture.run_ok("migrate", &[("LEPTOSKIT_TEST_DB_URL", &url)]);
	assert!(stdout.contains("the database is up to date"), "{stdout}");
	assert_eq!(scalar(&url, "SELECT count(*) FROM \"users\""), 1);
	assert_eq!(applied(&url).len(), 2);
}

#[test]
fn migrate_says_where_the_database_url_must_come_from() {
	let fixture = child::Fixture::new();
	fixture.run_refused(
		"migrate",
		&[],
		"set DATABASE_URL to the database URL, or pass --url",
	);
	assert_eq!(users_table(&fixture.url()), 0, "nothing was created");
}

#[test]
fn rollback_needs_consent_and_a_positive_step_count() {
	let fixture = child::Fixture::new();
	let url = fixture.url();
	apply_migrations(&fixture.project(), &url, 2);
	insert_kept_row(&url);

	fixture.run_refused(
		"rollback",
		&[
			("LEPTOSKIT_TEST_DB_URL", &url),
			("LEPTOSKIT_TEST_DB_YES", "0"),
		],
		"rollback can delete data; stop application writers",
	);
	fixture.run_refused(
		"rollback",
		&[
			("LEPTOSKIT_TEST_DB_URL", &url),
			("LEPTOSKIT_TEST_DB_YES", "1"),
			("LEPTOSKIT_TEST_DB_STEPS", "0"),
		],
		"--steps must be positive",
	);

	// Both refusals left the migrations, the column, and the row alone.
	assert_eq!(applied(&url).len(), 2);
	assert_eq!(bio_column(&url), 1);
	assert_eq!(scalar(&url, "SELECT count(*) FROM \"users\""), 1);
}

#[test]
fn rollback_reverts_newest_first_until_the_registry_is_empty() {
	let fixture = child::Fixture::new();
	let url = fixture.url();
	apply_migrations(&fixture.project(), &url, 2);
	insert_kept_row(&url);

	// One step reverses only the newest migration, and the data the
	// earlier migration brought stays.
	let stdout = fixture.run_ok(
		"rollback",
		&[
			("LEPTOSKIT_TEST_DB_URL", &url),
			("LEPTOSKIT_TEST_DB_YES", "1"),
			("LEPTOSKIT_TEST_DB_STEPS", "1"),
		],
	);
	assert!(
		stdout.contains("reverted 20260101T000200_add_bio"),
		"{stdout}"
	);
	assert_eq!(applied(&url), ["20260101T000100_create_users"]);
	assert_eq!(bio_column(&url), 0);
	assert_eq!(scalar(&url, "SELECT count(*) FROM \"users\""), 1);

	// Two steps now ask for more than the one migration still applied.
	fixture.run_refused(
		"rollback",
		&[
			("LEPTOSKIT_TEST_DB_URL", &url),
			("LEPTOSKIT_TEST_DB_YES", "1"),
			("LEPTOSKIT_TEST_DB_STEPS", "2"),
		],
		"rollback needs an applied registry prefix and a positive step count",
	);
	assert_eq!(applied(&url), ["20260101T000100_create_users"]);

	let stdout = fixture.run_ok(
		"rollback",
		&[
			("LEPTOSKIT_TEST_DB_URL", &url),
			("LEPTOSKIT_TEST_DB_YES", "1"),
			("LEPTOSKIT_TEST_DB_STEPS", "1"),
		],
	);
	assert!(
		stdout.contains("reverted 20260101T000100_create_users"),
		"{stdout}"
	);
	assert_eq!(applied(&url), Vec::<String>::new());
	assert_eq!(users_table(&url), 0);

	// More steps than the applied history stops before touching
	// anything, and the refusal left nothing to revert.
	fixture.run_refused(
		"rollback",
		&[
			("LEPTOSKIT_TEST_DB_URL", &url),
			("LEPTOSKIT_TEST_DB_YES", "1"),
			("LEPTOSKIT_TEST_DB_STEPS", "3"),
		],
		"rollback needs an applied registry prefix and a positive step count",
	);
	assert_eq!(applied(&url), Vec::<String>::new());
}

#[test]
fn a_failed_reverse_leaves_the_durable_state_alone() {
	let fixture = child::Fixture::new();
	let url = fixture.url();
	apply_migrations(&fixture.project(), &url, 2);
	insert_kept_row(&url);

	// A reviewed-looking down.sql whose SQL fails halfway through.
	std::fs::write(
		fixture
			.migrations()
			.join("20260101T000200_add_bio")
			.join("down.sql"),
		"DELETE FROM \"users\";\nALTER TABLE \"users\" DROP COLUMN \"bio\";\nTHIS IS NOT SQL;\n",
	)
	.unwrap();

	fixture.run_refused(
		"rollback",
		&[
			("LEPTOSKIT_TEST_DB_URL", &url),
			("LEPTOSKIT_TEST_DB_YES", "1"),
		],
		"migration `20260101T000200_add_bio` failed and was rolled back",
	);

	// The reverse runs in one transaction with the history deletion, so
	// the migration and everything it added survive the failure.
	assert_eq!(applied(&url).len(), 2);
	assert_eq!(bio_column(&url), 1);
	assert_eq!(scalar(&url, "SELECT count(*) FROM \"users\""), 1);
}

#[test]
fn an_unreviewed_reverse_is_refused_before_any_changes() {
	let fixture = child::Fixture::new();
	let url = fixture.url();
	apply_migrations(&fixture.project(), &url, 2);
	insert_kept_row(&url);

	std::fs::write(
		fixture
			.migrations()
			.join("20260101T000200_add_bio")
			.join("down.sql"),
		"-- leptoskit: irreversible\n-- Implement and review data recovery before removing the \
		 marker above.\n",
	)
	.unwrap();

	fixture.run_refused(
		"rollback",
		&[
			("LEPTOSKIT_TEST_DB_URL", &url),
			("LEPTOSKIT_TEST_DB_YES", "1"),
		],
		"migration `20260101T000200_add_bio` has no reviewed down.sql; rollback stopped before \
		 making changes",
	);

	assert_eq!(applied(&url).len(), 2);
	assert_eq!(bio_column(&url), 1);
	assert_eq!(scalar(&url, "SELECT count(*) FROM \"users\""), 1);
}

#[test]
fn reset_refuses_without_consent_and_keeps_every_byte() {
	let fixture = child::Fixture::new();
	let url = fixture.url();
	apply_migrations(&fixture.project(), &url, 2);
	insert_kept_row(&url);
	let database = fixture.path().join("dev.sqlite3");
	let before = std::fs::read(&database).unwrap();

	fixture.run_refused(
		"reset",
		&[
			("LEPTOSKIT_TEST_DB_URL", &url),
			("LEPTOSKIT_TEST_DB_YES", "0"),
		],
		"this deletes every row in the database; pass --yes to confirm",
	);

	// The refusal happens before the command touches the database.
	assert_eq!(std::fs::read(&database).unwrap(), before);
	assert_eq!(scalar(&url, "SELECT count(*) FROM \"users\""), 1);
	assert_eq!(applied(&url).len(), 2);
}

#[test]
fn reset_rebuilds_a_local_database_and_refuses_other_targets() {
	let fixture = child::Fixture::new();
	let url = fixture.url();
	apply_migrations(&fixture.project(), &url, 2);
	insert_kept_row(&url);

	let stdout = fixture.run_ok(
		"reset",
		&[
			("LEPTOSKIT_TEST_DB_URL", &url),
			("LEPTOSKIT_TEST_DB_YES", "1"),
		],
	);
	assert!(
		stdout.contains("reset the database and applied 2 migration(s)"),
		"{stdout}"
	);
	assert_eq!(scalar(&url, "SELECT count(*) FROM \"users\""), 0);
	assert_eq!(users_table(&url), 1);
	assert_eq!(applied(&url).len(), 2);

	// A database on another host is never deleted from here.
	fixture.run_refused(
		"reset",
		&[
			(
				"LEPTOSKIT_TEST_DB_URL",
				"postgres://demo@db.example.com/app",
			),
			("LEPTOSKIT_TEST_DB_YES", "1"),
		],
		"reset only deletes local databases; db.example.com isn't local",
	);

	// Neither is something that isn't a URL at all.
	fixture.run_refused(
		"reset",
		&[
			("LEPTOSKIT_TEST_DB_URL", "not a database url"),
			("LEPTOSKIT_TEST_DB_YES", "1"),
		],
		"the database URL isn't a URL",
	);

	assert_eq!(applied(&url).len(), 2);
	assert_eq!(scalar(&url, "SELECT count(*) FROM \"users\""), 0);
}

#[cfg(unix)]
#[test]
fn status_reports_the_history_and_what_the_database_needs() {
	let fixture = child::Fixture::new();
	let url = fixture.url();
	apply_migrations(&fixture.project(), &url, 1);

	let export = fixture.export_environment();
	let environment: Vec<(&str, &str)> = export
		.iter()
		.map(|(name, value)| (*name, value.as_str()))
		.chain(std::iter::once(("LEPTOSKIT_TEST_DB_URL", url.as_str())))
		.collect();
	let stdout = fixture.run_ok("status", &environment);
	assert!(
		stdout.contains("2 migration(s); the latest is 20260101T000200_add_bio"),
		"{stdout}"
	);
	assert!(
		stdout.contains("the tables match the latest migration"),
		"{stdout}"
	);
	assert!(stdout.contains("the database needs:"), "{stdout}");
	assert!(stdout.contains("  20260101T000200_add_bio"), "{stdout}");

	// Without a database URL, status still reports what it can.
	let environment: Vec<(&str, &str)> = export
		.iter()
		.map(|(name, value)| (*name, value.as_str()))
		.collect();
	let stdout = fixture.run_ok("status", &environment);
	assert!(
		stdout.contains("to see which migrations the database needs"),
		"{stdout}"
	);

	// Fully migrated: nothing is pending any more.
	apply_migrations(&fixture.project(), &url, 2);
	let environment: Vec<(&str, &str)> = export
		.iter()
		.map(|(name, value)| (*name, value.as_str()))
		.chain(std::iter::once(("LEPTOSKIT_TEST_DB_URL", url.as_str())))
		.collect();
	let stdout = fixture.run_ok("status", &environment);
	assert!(
		stdout.contains("the database has every migration"),
		"{stdout}"
	);
}

#[cfg(unix)]
#[test]
fn status_reports_what_is_wrong_with_the_schema_export() {
	let fixture = child::Fixture::new();
	let export = fixture.export_environment();

	for (mode, expected) in [
		("fail", "couldn't read the tables in"),
		("missing", "doesn't export its tables"),
		("invalid", "the schema snapshot isn't valid"),
	] {
		let mut environment: Vec<(&str, &str)> = export
			.iter()
			.map(|(name, value)| (*name, value.as_str()))
			.collect();
		environment.push(("LEPTOSKIT_TEST_DB_EXPORT", mode));
		fixture.run_refused("status", &environment, expected);
	}
}

#[test]
fn release_markers_freeze_the_migrations_above_them() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let first = users(
		Dialect::Sqlite,
		vec![Column::new("handle", ColumnType::TEXT)],
	);
	create(
		&project,
		"create users",
		&first,
		&options(),
		&mut RequireHints,
	)
	.unwrap();

	super::record_release(&project, "1.4.0", Some("monochange:rel_1_4_0")).unwrap();
	let history = History::read(&project.migrations_dir(), project.dialect()).unwrap();
	assert_eq!(history.names.len(), 1);
	assert_eq!(history.published, 1);
	assert_eq!(history.releases.len(), 1);
	assert_eq!(history.releases[0].version, "1.4.0");
	assert_eq!(
		history.releases[0].source.as_deref(),
		Some("monochange:rel_1_4_0")
	);

	// A migration created after the release stays a draft below the marker,
	// and the marker still freezes exactly the one above it.
	let mut second = first.clone();
	second.tables[0]
		.columns
		.push(Column::new("bio", ColumnType::TEXT).nullable());
	std::thread::sleep(std::time::Duration::from_secs(1));
	create(&project, "add bio", &second, &options(), &mut RequireHints).unwrap();
	let history = History::read(&project.migrations_dir(), project.dialect()).unwrap();
	assert_eq!(history.names.len(), 2);
	assert_eq!(history.published, 1, "the marker froze the first only");

	let registry = std::fs::read_to_string(project.migrations_dir().join("registry.txt")).unwrap();
	let marker = registry
		.lines()
		.position(|line| line.starts_with("# release 1.4.0"))
		.unwrap();
	let draft = registry
		.lines()
		.position(|line| line == history.names[1])
		.unwrap();
	assert!(
		marker < draft,
		"the draft lands below the marker:\n{registry}"
	);
}

#[test]
fn a_release_marker_refuses_a_drifted_snapshot() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let first = users(
		Dialect::Sqlite,
		vec![Column::new("handle", ColumnType::TEXT)],
	);
	create(
		&project,
		"create users",
		&first,
		&options(),
		&mut RequireHints,
	)
	.unwrap();
	super::record_release(&project, "1.4.0", None).unwrap();

	// Editing a frozen migration's snapshot no longer matches the pin.
	let name = History::read(&project.migrations_dir(), project.dialect())
		.unwrap()
		.names[0]
		.clone();
	let snapshot = project.migrations_dir().join(name).join("schema.json");
	let json = std::fs::read_to_string(&snapshot).unwrap();
	std::fs::write(&snapshot, format!("{json}\n")).unwrap();
	let error = History::read(&project.migrations_dir(), project.dialect())
		.err()
		.expect("the drifted pin is refused")
		.to_string();
	assert!(error.contains("pins the wrong schema"), "{error}");
}

#[test]
fn release_markers_are_validated_when_read() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let directory = project.migrations_dir();
	std::fs::create_dir_all(&directory).unwrap();

	for (registry, expected) in [
		(
			"# release 1.0.0 snapshot 0123456789abcdef\n",
			"no migration above it".to_owned(),
		),
		("# release 1.0.0 snapshot\n", "isn't `# release".to_owned()),
	] {
		std::fs::write(directory.join("registry.txt"), registry).unwrap();
		let error = History::read(&project.migrations_dir(), project.dialect())
			.err()
			.expect("the marker is refused")
			.to_string();
		assert!(error.contains(&expected), "{error} for {registry}");
	}
}

#[test]
fn recording_a_release_twice_is_refused() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let first = users(
		Dialect::Sqlite,
		vec![Column::new("handle", ColumnType::TEXT)],
	);
	create(
		&project,
		"create users",
		&first,
		&options(),
		&mut RequireHints,
	)
	.unwrap();
	super::record_release(&project, "1.4.0", None).unwrap();
	let error = super::record_release(&project, "1.4.0", None)
		.unwrap_err()
		.to_string();
	assert!(error.contains("already recorded"), "{error}");
}

#[test]
fn consolidation_folds_the_drafts_since_a_release_into_one() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let first = users(
		Dialect::Sqlite,
		vec![Column::new("handle", ColumnType::TEXT)],
	);
	create(
		&project,
		"create users",
		&first,
		&options(),
		&mut RequireHints,
	)
	.unwrap();
	super::record_release(&project, "1.4.0", None).unwrap();

	let mut second = first.clone();
	second.tables[0]
		.columns
		.push(Column::new("bio", ColumnType::TEXT).nullable());
	std::thread::sleep(std::time::Duration::from_secs(1));
	create(&project, "add bio", &second, &options(), &mut RequireHints).unwrap();

	let mut third = second.clone();
	third.tables[0]
		.columns
		.push(Column::new("karma", ColumnType::Int32).nullable());
	std::thread::sleep(std::time::Duration::from_secs(1));
	create(&project, "add karma", &third, &options(), &mut RequireHints).unwrap();

	super::consolidate(&project, &third, &[], &[], &mut RequireHints).unwrap();

	let history = History::read(&project.migrations_dir(), project.dialect()).unwrap();
	assert_eq!(
		history.names.len(),
		2,
		"the release's and the consolidated one"
	);
	let consolidated = history.names.last().unwrap();
	assert!(consolidated.ends_with("_consolidated"), "{consolidated}");
	assert!(
		history.names[0].ends_with("_create_users"),
		"the release's migration stays, first: {}",
		history.names[0]
	);

	// The consolidated migration covers both drafts in one diff.
	let sql = std::fs::read_to_string(
		project
			.migrations_dir()
			.join(consolidated)
			.join("migration.sql"),
	)
	.unwrap();
	assert!(sql.contains("\"bio\""), "{sql}");
	assert!(sql.contains("\"karma\""), "{sql}");

	let replaces: Vec<String> = serde_json::from_str(
		&std::fs::read_to_string(
			project
				.migrations_dir()
				.join(consolidated)
				.join("replaces.json"),
		)
		.unwrap(),
	)
	.unwrap();
	assert_eq!(replaces.len(), 2, "{replaces:?}");
	assert!(
		!project.migrations_dir().join(&replaces[0]).exists(),
		"the folded directories are gone"
	);
}

#[test]
fn consolidation_refuses_hand_edits_and_keeps_the_newest() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let first = users(
		Dialect::Sqlite,
		vec![Column::new("handle", ColumnType::TEXT)],
	);
	create(
		&project,
		"create users",
		&first,
		&options(),
		&mut RequireHints,
	)
	.unwrap();
	super::record_release(&project, "1.4.0", None).unwrap();

	let mut second = first.clone();
	second.tables[0]
		.columns
		.push(Column::new("bio", ColumnType::TEXT).nullable());
	std::thread::sleep(std::time::Duration::from_secs(1));
	create(&project, "add bio", &second, &options(), &mut RequireHints).unwrap();

	let mut third = second.clone();
	third.tables[0]
		.columns
		.push(Column::new("karma", ColumnType::Int32).nullable());
	std::thread::sleep(std::time::Duration::from_secs(1));
	let kept = create(&project, "add karma", &third, &options(), &mut RequireHints).unwrap();

	// A statement written by hand stops the fold unless the migration is
	// kept.
	let hand_written = project
		.migrations_dir()
		.join(&kept.name)
		.join("migration.sql");
	let sql = std::fs::read_to_string(&hand_written).unwrap();
	std::fs::write(&hand_written, format!("{sql}\n-- hand written\n")).unwrap();
	let error = super::consolidate(&project, &third, &[], &[], &mut RequireHints)
		.unwrap_err()
		.to_string();
	assert!(
		error.contains(&kept.name) && error.contains("--keep"),
		"{error}"
	);

	let keep = [kept.name.clone()];
	super::consolidate(&project, &third, &keep, &[], &mut RequireHints).unwrap();
	let history = History::read(&project.migrations_dir(), project.dialect()).unwrap();
	assert_eq!(history.names.len(), 3, "released, consolidated, kept");
	assert_eq!(history.names[2], kept.name, "the kept migration runs last");
}

#[test]
fn amend_mode_rewrites_the_draft_since_the_release() {
	let root = tempfile::tempdir().unwrap();
	let mut project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	project.config.draft = leptoskit_config::DbDraft::Amend;

	let first = users(
		Dialect::Sqlite,
		vec![Column::new("handle", ColumnType::TEXT)],
	);
	create(
		&project,
		"create users",
		&first,
		&options(),
		&mut RequireHints,
	)
	.unwrap();
	super::record_release(&project, "1.4.0", None).unwrap();

	let mut second = first.clone();
	second.tables[0]
		.columns
		.push(Column::new("bio", ColumnType::TEXT).nullable());
	std::thread::sleep(std::time::Duration::from_secs(1));
	let draft = create(&project, "add bio", &second, &options(), &mut RequireHints).unwrap();

	// The next change amends the draft in place: one name, one migration,
	// both columns.
	let mut third = second.clone();
	third.tables[0]
		.columns
		.push(Column::new("karma", ColumnType::Int32).nullable());
	let amended = create(&project, "add karma", &third, &options(), &mut RequireHints).unwrap();
	assert_eq!(amended.name, draft.name, "the draft keeps its name");

	let history = History::read(&project.migrations_dir(), project.dialect()).unwrap();
	assert_eq!(history.names.len(), 2, "released and the one draft");
	let sql = std::fs::read_to_string(
		project
			.migrations_dir()
			.join(&draft.name)
			.join("migration.sql"),
	)
	.unwrap();
	assert!(
		sql.contains("\"bio\"") && sql.contains("\"karma\""),
		"{sql}"
	);
	assert!(
		amended.notes.iter().any(|note| note.contains("db reset")),
		"{:?}",
		amended.notes
	);
}

/// `schema` with a `sample` pack owning its `users` table.
fn sample_pack(schema: &Schema) -> Schema {
	let tables = schema
		.tables
		.iter()
		.map(|table| table.name.clone())
		.collect();
	schema.clone().with_packs(vec![Pack::new("sample", tables)])
}

/// Rewrites the latest snapshot as format 1, which has no pack membership.
fn make_latest_format_one(project: &Project) {
	let history = History::read(&project.migrations_dir(), project.dialect()).unwrap();
	let name = history.names.last().unwrap();
	let path = project.migrations_dir().join(name).join("schema.json");
	let mut json: serde_json::Value =
		serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
	json["format"] = serde_json::json!(1);
	json.as_object_mut().unwrap().remove("packs");
	std::fs::write(path, serde_json::to_string_pretty(&json).unwrap()).unwrap();
}

#[test]
fn a_missing_pack_needs_explicit_removal_even_with_destructive_consent() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let dialect = Dialect::Sqlite;
	let baseline = sample_pack(&users(
		dialect,
		vec![Column::new("handle", ColumnType::TEXT)],
	));
	create(&project, "users", &baseline, &options(), &mut RequireHints).unwrap();
	let before = History::read(&project.migrations_dir(), dialect)
		.unwrap()
		.names
		.len();
	let removed = Schema::new(dialect, Vec::new());
	let consent = CreateOptions {
		allow_destructive: true,
		..options()
	};

	// `--allow-destructive` only consents to table data loss; it doesn't say
	// the pack itself was meant to go.
	let error = create(&project, "drop", &removed, &consent, &mut RequireHints)
		.unwrap_err()
		.to_string();
	assert!(error.contains("schema pack(s) `sample`"), "{error}");
	assert!(error.contains("--drop-pack sample"), "{error}");
	assert_eq!(
		History::read(&project.migrations_dir(), dialect)
			.unwrap()
			.names
			.len(),
		before,
		"nothing was written"
	);

	// A name that isn't actually disappearing is a typo, not consent.
	let typo = CreateOptions {
		drop_packs: vec!["auth".into()],
		..consent
	};
	let error = create(&project, "drop", &removed, &typo, &mut RequireHints)
		.unwrap_err()
		.to_string();
	assert!(error.contains("doesn't lose it"), "{error}");
	assert_eq!(
		History::read(&project.migrations_dir(), dialect)
			.unwrap()
			.names
			.len(),
		before
	);

	// Naming the pack is explicit intent, and the new snapshot records that
	// the schema has no packs left.
	let explicit = CreateOptions {
		drop_packs: vec!["sample".into()],
		..consent
	};
	let created = create(&project, "drop", &removed, &explicit, &mut RequireHints).unwrap();
	assert!(
		created.steps.iter().any(|step| step == "- table users"),
		"{:?}",
		created.steps
	);
	let snapshot: Schema =
		Schema::from_json(&std::fs::read_to_string(created.directory.join("schema.json")).unwrap())
			.unwrap();
	assert!(
		snapshot
			.pack_membership()
			.is_some_and(|packs| packs.is_empty()),
		"the new snapshot records no packs"
	);
}

#[test]
fn changes_within_a_present_pack_stay_ordinary() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let dialect = Dialect::Sqlite;
	let handle = Column::new("handle", ColumnType::TEXT);
	create(
		&project,
		"users",
		&sample_pack(&users(dialect, vec![handle.clone()])),
		&options(),
		&mut RequireHints,
	)
	.unwrap();

	let grown = sample_pack(&users(
		dialect,
		vec![handle, Column::new("bio", ColumnType::TEXT).nullable()],
	));
	let created = create(&project, "bio", &grown, &options(), &mut RequireHints).unwrap();
	assert_eq!(created.steps, ["+ column users.bio (text)"]);
	let snapshot: Schema =
		Schema::from_json(&std::fs::read_to_string(created.directory.join("schema.json")).unwrap())
			.unwrap();
	assert!(
		snapshot
			.pack("sample")
			.is_some_and(|pack| pack.owns(&TableName::new("users")))
	);
}

#[test]
fn a_legacy_snapshot_has_no_membership_evidence() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let dialect = Dialect::Sqlite;
	create(
		&project,
		"users",
		&sample_pack(&users(
			dialect,
			vec![Column::new("handle", ColumnType::TEXT)],
		)),
		&options(),
		&mut RequireHints,
	)
	.unwrap();
	make_latest_format_one(&project);

	// The old snapshot can't prove the pack was ever there, so it can't call
	// its disappearance a missing pack. Writing the next snapshot upgrades
	// the history to format 2.
	let consent = CreateOptions {
		allow_destructive: true,
		..options()
	};
	create(
		&project,
		"drop",
		&Schema::new(dialect, Vec::new()),
		&consent,
		&mut RequireHints,
	)
	.unwrap();
}

#[test]
fn check_reports_a_missing_pack() {
	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let dialect = Dialect::Sqlite;
	create(
		&project,
		"users",
		&sample_pack(&users(
			dialect,
			vec![Column::new("handle", ColumnType::TEXT)],
		)),
		&options(),
		&mut RequireHints,
	)
	.unwrap();
	let history = History::read(&project.migrations_dir(), dialect).unwrap();
	let error = check(&history, &Schema::new(dialect, Vec::new()))
		.unwrap_err()
		.to_string();
	assert!(error.contains("schema pack(s) `sample`"), "{error}");
}

#[test]
fn watch_reports_a_missing_pack_before_writing() {
	use super::watch::Change;
	use super::watch::classify;

	let root = tempfile::tempdir().unwrap();
	let project = project(root.path(), leptoskit_config::DbDialect::Sqlite);
	let dialect = Dialect::Sqlite;
	create(
		&project,
		"users",
		&sample_pack(&users(
			dialect,
			vec![Column::new("handle", ColumnType::TEXT)],
		)),
		&options(),
		&mut RequireHints,
	)
	.unwrap();
	let history = History::read(&project.migrations_dir(), dialect).unwrap();
	let change = classify(&history, &Schema::new(dialect, Vec::new()));
	assert_eq!(change, Change::MissingPacks(vec!["sample".into()]));
	let message = change.message(leptoskit_config::DbWatch::Migrate).unwrap();
	assert!(message.contains("--drop-pack sample"), "{message}");
}

#[test]
fn pushes_refuse_a_missing_pack_and_keep_its_pushed_state() {
	let root = tempfile::tempdir().unwrap();

	for (project, url, _server) in push_databases(root.path()) {
		let dialect = project.dialect();
		let handle = Column::new("handle", ColumnType::TEXT);
		let bio = Column::new("bio", ColumnType::TEXT).nullable();
		create(
			&project,
			"users",
			&sample_pack(&users(dialect, vec![handle.clone()])),
			&options(),
			&mut RequireHints,
		)
		.unwrap();

		let pushed = sample_pack(&users(dialect, vec![handle.clone(), bio]));
		super::push::push(&project, &url, &pushed, false, &[]).unwrap();
		assert!(applied_and_pushed(&url).1);

		// The SQLite connection holds a row, so a refused push must leave it
		// exactly where the successful one left it.
		let count = || {
			match dialect {
				Dialect::Sqlite => {
					scalar(
						&url,
						"SELECT count(*) FROM pragma_table_info('users') WHERE name = 'bio'",
					)
				}
				Dialect::Postgres => {
					scalar(
						&url,
						"SELECT count(*) FROM information_schema.columns WHERE table_name = \
						 'users' AND column_name = 'bio'",
					)
				}
			}
		};
		assert_eq!(count(), 1);

		let without = Schema::new(dialect, Vec::new());
		let error = super::push::push(&project, &url, &without, true, &[])
			.unwrap_err()
			.to_string();
		assert!(error.contains("schema pack(s) `sample`"), "{error}");
		assert!(applied_and_pushed(&url).1, "the push state survives");
		assert_eq!(count(), 1, "no SQL ran");

		// Squashing and creating both refuse the same loss before writing.
		let consent = CreateOptions {
			allow_destructive: true,
			..options()
		};
		let error = super::create_squashing(
			&project,
			"drop",
			&without,
			&consent,
			&mut RequireHints,
			Some(&url),
		)
		.unwrap_err()
		.to_string();
		assert!(error.contains("schema pack(s) `sample`"), "{error}");
		assert_eq!(
			History::read(&project.migrations_dir(), dialect)
				.unwrap()
				.names
				.len(),
			1,
			"no migration was written"
		);

		// Naming the pack lets the push remove it.
		super::push::push(&project, &url, &without, true, &["sample".into()]).unwrap();
		assert!(!applied_and_pushed(&url).1);
	}
}
