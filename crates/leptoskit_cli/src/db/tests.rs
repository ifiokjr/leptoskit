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
