//! A service's own database in development, which `leptoskit dev`,
//! `leptoskit serve`, and `leptoskit db --service` share: a SQLite file
//! under `.leptoskit/services/<name>/`, or on the app's Postgres server a
//! database and a login role of the service's own.
//!
//! On Postgres the role is named after the main branch's database,
//! `<app>_service_<name>`, and owns the service's database on every branch,
//! so a branch's copy, `<app>__<branch>_service_<name>`, holds tables the
//! same role owns. Only the role connects to it, and the role can't connect
//! to the app's database.

use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::bail;
use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_db::Value;
use leptoskit_db_schema::quote_identifier;

use crate::db::Project;
use crate::db::branches;
use crate::db::dev_server;

/// The SQLite URL of a service's development database, with its directory
/// created.
///
/// # Errors
///
/// When the directory can't be created.
pub(crate) fn sqlite_url(root: &Path, service: &str) -> Result<String> {
	let path = super::sqlite_path(root, service);

	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent)
			.with_context(|| format!("could not create {}", parent.display()))?;
	}

	Ok(format!("sqlite://{}", path.display()))
}

/// The names a service's Postgres database goes by beside the app's
/// database `app_database`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PostgresNames {
	/// The service's database beside `app_database`.
	pub database: String,
	/// The role that owns it, which is also the main branch's database.
	pub role: String,
	/// The branch, when `app_database` is a branch's own copy.
	pub branch: Option<String>,
	/// The main branch's app database, which the role mustn't reach either:
	/// `app_database` itself off a branch.
	pub app_main: String,
}

impl PostgresNames {
	/// The names for `service` beside `app_database`, which is a branch's
	/// database when it's `<main>__<branch>` and `app` (the app's database
	/// settings) has branch databases.
	pub(crate) fn of(app: Option<&Project>, app_database: &str, service: &str) -> Self {
		let main = app
			.filter(|app| app.config.branch_databases)
			.map(Project::database_name)
			.filter(|main| {
				app_database
					.strip_prefix(main.as_str())
					.is_some_and(|rest| rest.starts_with("__"))
			});
		let branch = main.as_ref().and_then(|_| {
			app.and_then(|app| branches::branches(&app.root))
				.and_then(|branches| branches.current)
		});
		let app_main = main.unwrap_or_else(|| app_database.to_owned());

		Self {
			database: super::postgres_database(app_database, service),
			role: super::postgres_database(&app_main, service),
			branch,
			app_main,
		}
	}
}

/// What [`postgres_url`] does with a service's database that exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Existing {
	/// Keeps it.
	Keep,
	/// Drops it, and every row in it, and creates it again empty.
	Recreate,
}

/// The URL of `service`'s own database on the Postgres server the app's
/// database `app_url` is on, creating the database and its role when
/// they're missing.
///
/// A branch's database starts as a copy of the main branch's, the way the
/// app's branch databases do, labelled with its branch so `leptoskit db
/// branch prune` deletes it with the app's. Without a superuser to create
/// roles, the service shares the app's login and gets only a database of
/// its own, which a warning says.
///
/// # Errors
///
/// When the server refuses the setup, or the database was created by an
/// older leptoskit under another owner.
pub(crate) fn postgres_url(
	app: Option<&Project>,
	root: &Path,
	app_url: &str,
	service: &str,
	existing: Existing,
) -> Result<String> {
	let app_database = database_of(app_url)?;
	let names = PostgresNames::of(app, &app_database, service);
	let server = dev_server::with_database(app_url, "postgres")?;
	let superuser = crate::db::runtime()?.block_on(async {
		let db = Db::connect(&server).await?;
		let rows = db
			.fetch(Statement {
				sql: "SELECT rolsuper FROM pg_catalog.pg_roles WHERE rolname = current_user"
					.to_owned(),
				args: vec![],
			})
			.await?;
		anyhow::Ok(matches!(
			rows.first().and_then(|row| row.first()),
			Some(Value::Bool(true))
		))
	})?;

	if existing == Existing::Recreate {
		branches::drop_database(&server, &names.database)?;
	}

	if !superuser {
		crate::ui::warning(format_args!(
			"services.{service}: the app's Postgres login can't create roles, so the service \
			 shares it, with a database of its own but no isolation from the app's"
		));
		create(&server, &names, None, existing)?;
		return dev_server::with_database(app_url, &names.database);
	}

	let directory = super::dev_dir(root, service);
	std::fs::create_dir_all(&directory)
		.with_context(|| format!("could not create {}", directory.display()))?;
	let password = dev_server::remembered(&directory.join("postgres-password"), || {
		let mut bytes = [0_u8; 18];
		getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!("{error}"))?;
		Ok(hex::encode(bytes))
	})?;
	let role = quote_identifier(&names.role);
	let script = format!(
		"DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = {name}) THEN \
		 CREATE ROLE {role} LOGIN; END IF; END $$;\nALTER ROLE {role} LOGIN PASSWORD \
		 '{password}';\n",
		name = literal(&names.role),
	);
	crate::db::runtime()?.block_on(async {
		let db = Db::connect(&server).await?;
		anyhow::Ok(db.run_script(&script).await?)
	})?;
	create(&server, &names, Some(&names.role), existing)?;

	// Only the service's role connects to its database, and it can't reach
	// the app's, on this branch or the main one, whose own login, a
	// superuser here, still can.
	let mut lockdown = String::new();

	for database in [&names.database, &app_database, &names.app_main] {
		if database == &names.app_main && !exists(&server, database)? {
			continue;
		}

		let _ = writeln!(
			lockdown,
			"REVOKE CONNECT ON DATABASE {} FROM PUBLIC;",
			quote_identifier(database)
		);
	}
	crate::db::runtime()?.block_on(async {
		let db = Db::connect(&server).await?;
		anyhow::Ok(db.run_script(&lockdown).await?)
	})?;

	let url = dev_server::with_database(app_url, &names.database)?;
	dev_server::with_login(&url, &names.role, &password)
}

/// Creates the service's database unless it exists: owned by `owner`, and
/// on a branch copied from the main branch's when that exists, unless it's
/// being created again empty.
fn create(
	server: &str,
	names: &PostgresNames,
	owner: Option<&str>,
	existing: Existing,
) -> Result<()> {
	let found = crate::db::runtime()?.block_on(async {
		let db = Db::connect(server).await?;
		let rows = db
			.fetch(Statement {
				sql: "SELECT pg_catalog.pg_get_userbyid(datdba) FROM pg_catalog.pg_database WHERE \
				      datname = $1"
					.to_owned(),
				args: vec![Value::Text(names.database.clone())],
			})
			.await?;
		anyhow::Ok(rows.into_iter().next().and_then(|row| {
			match row.into_iter().next() {
				Some(Value::Text(owner)) => Some(owner),
				_ => None,
			}
		}))
	})?;

	if let Some(found) = found {
		if let Some(owner) = owner
			&& found != owner
		{
			bail!(
				"the database `{}` belongs to `{found}`, not the service's role `{owner}`, \
				 because an older leptoskit created it; `leptoskit db reset --service <name> \
				 --yes` creates it again under its role, deleting its rows",
				names.database
			);
		}

		return Ok(());
	}

	let template = (names.database != names.role && existing == Existing::Keep)
		.then(|| {
			names
				.branch
				.as_deref()
				.map(|branch| (names.role.as_str(), branch))
		})
		.flatten()
		.filter(|(main, _)| exists(server, main).unwrap_or(false));
	let name = names.database.as_str();

	if !branches::ensure_owned_database(server, name, owner, template)? {
		return Ok(());
	}

	match (template, names.branch.as_deref()) {
		(Some((main, branch)), _) => {
			crate::ui::tagged(
				"db",
				format_args!("branch `{branch}` has its own `{name}`, copied from `{main}`"),
			);
		}
		// Empty, because the main branch's service has no database yet, but
		// still the branch's, to prune with it.
		(None, Some(branch)) if names.database != names.role => {
			branches::label_database(server, name, branch)?;
			crate::ui::tagged(
				"db",
				format_args!("branch `{branch}` has its own `{name}`, created empty"),
			);
		}
		(None, _) => crate::ui::tagged("db", format_args!("created the database `{name}`")),
	}

	Ok(())
}

fn exists(server: &str, database: &str) -> Result<bool> {
	crate::db::runtime()?.block_on(async {
		let db = Db::connect(server).await?;
		let rows = db
			.fetch(Statement {
				sql: "SELECT 1 FROM pg_catalog.pg_database WHERE datname = $1".to_owned(),
				args: vec![Value::Text(database.to_owned())],
			})
			.await?;
		anyhow::Ok(!rows.is_empty())
	})
}

/// The database a Postgres URL names.
///
/// # Errors
///
/// When it isn't a URL, or names no database.
pub(crate) fn database_of(url: &str) -> Result<String> {
	let parsed = url::Url::parse(url).context("the app's database URL isn't a URL")?;
	let name = parsed.path().trim_start_matches('/');
	anyhow::ensure!(!name.is_empty(), "the app's database URL names no database");
	Ok(name.to_owned())
}

/// A SQL string literal.
fn literal(text: &str) -> String {
	format!("'{}'", text.replace('\'', "''"))
}

/// Where a service's migrations are for the app's engine: `migrations/`
/// beside its `Cargo.toml` on SQLite, and `migrations-postgres/` on
/// Postgres, as a service that supports both ships them. `None` for a
/// remote service.
#[must_use]
pub fn migrations_dir(
	service: &super::Service,
	dialect: leptoskit_config::DbDialect,
) -> Option<PathBuf> {
	let manifest = match &service.code {
		super::Code::Workspace { manifest, .. } | super::Code::External { manifest, .. } => {
			manifest
		}
		super::Code::Remote => return None,
	};
	let directory = match dialect {
		leptoskit_config::DbDialect::Sqlite => "migrations",
		leptoskit_config::DbDialect::Postgres => "migrations-postgres",
	};
	Some(manifest.parent()?.join(directory))
}

#[cfg(test)]
mod tests {
	use leptoskit_config::DbConfig;
	use leptoskit_config::DbDevServer;
	use leptoskit_config::DbDialect;

	use super::*;

	fn git(root: &Path, args: &[&str]) {
		let status = crate::git::command(root)
			.args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
			.args(args)
			.env("GIT_CONFIG_GLOBAL", root.join("no-global-config"))
			.env("GIT_CONFIG_NOSYSTEM", "1")
			.stdout(std::process::Stdio::null())
			.stderr(std::process::Stdio::null())
			.status()
			.unwrap();
		assert!(status.success(), "git {args:?}");
	}

	/// What `url` sees: the result of `sql`'s first column, or `None` when
	/// it can't connect or the query fails.
	fn first(url: &str, sql: &str) -> Option<Value> {
		crate::db::runtime().unwrap().block_on(async {
			let db = Db::connect(url).await.ok()?;
			let rows = db
				.fetch(Statement {
					sql: sql.to_owned(),
					args: vec![],
				})
				.await
				.ok()?;
			rows.into_iter().next()?.into_iter().next()
		})
	}

	/// The analytics example's Postgres migrations: what the service applies
	/// at startup.
	fn analytics_migrations() -> Vec<leptoskit_db::Migration> {
		let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
			.join("../../examples/analytics/analytics_service/migrations-postgres");
		leptoskit_db::load_migrations(&directory).unwrap()
	}

	/// The whole development path on a real Postgres: the service's role and
	/// database, its migrations, the app's database closed to it, a branch's
	/// copy, and `reset`'s recreation.
	#[test]
	fn a_service_gets_its_own_role_and_database_on_the_development_postgres() {
		let Some(binaries) = dev_server::test_binaries() else {
			return;
		};
		let root = tempfile::tempdir().unwrap();
		git(root.path(), &["init", "--quiet", "--initial-branch=main"]);
		git(
			root.path(),
			&["commit", "--quiet", "--allow-empty", "-m", "initial"],
		);
		let app = Project {
			root: root.path().to_path_buf(),
			config: DbConfig {
				dialect: DbDialect::Postgres,
				dev_server: DbDevServer::Embedded,
				..DbConfig::default()
			},
			package: "app_db".into(),
			name: "my-app".into(),
			service: None,
		};
		let server =
			dev_server::start(&app, dev_server::Source::Embedded, Some(&binaries)).unwrap();
		let service = |app_url: &str, existing| {
			postgres_url(Some(&app), root.path(), app_url, "analytics", existing).unwrap()
		};

		let url = service(&server.url, Existing::Keep);
		let parsed = url::Url::parse(&url).unwrap();
		assert_eq!(parsed.username(), "my_app_service_analytics");
		assert_eq!(parsed.path(), "/my_app_service_analytics");
		assert_eq!(
			service(&server.url, Existing::Keep),
			url,
			"a second start keeps the role's password"
		);

		// The service migrates and writes its own database, as its role.
		crate::db::runtime().unwrap().block_on(async {
			let db = Db::connect(&url).await.unwrap();
			let applied = db.migrate(&analytics_migrations()).await.unwrap();
			assert_ne!(applied, Vec::<String>::new());
			db.run_script(
				"INSERT INTO events (id, name, distinct_id, properties, created_at) VALUES \
				 (gen_random_uuid(), 'signup', 'v1', '{}', now())",
			)
			.await
			.unwrap();
		});
		assert_eq!(
			first(
				&url,
				"SELECT pg_get_userbyid(relowner) FROM pg_class WHERE relname = 'events'"
			),
			Some(Value::Text("my_app_service_analytics".into())),
			"the service's role owns its tables"
		);

		// Its role can't reach the app's database; the app's login still can.
		let app_as_service = dev_server::with_database(&url, "my_app").unwrap();
		assert_eq!(first(&app_as_service, "SELECT 1"), None);
		assert_eq!(first(&server.url, "SELECT 1"), Some(Value::Int(1)));

		// A branch's database starts as a copy of the main branch's.
		git(root.path(), &["checkout", "--quiet", "-b", "feat/charts"]);
		let postgres = dev_server::with_database(&server.url, "postgres").unwrap();
		let branch = branches::ensure(&app, &postgres).unwrap();
		assert_eq!(branch, "my_app__feat_charts");
		let branch_url = service(
			&dev_server::with_database(&server.url, &branch).unwrap(),
			Existing::Keep,
		);
		let parsed = url::Url::parse(&branch_url).unwrap();
		assert_eq!(parsed.username(), "my_app_service_analytics");
		assert_eq!(parsed.path(), "/my_app__feat_charts_service_analytics");
		assert_eq!(
			first(&branch_url, "SELECT count(*) FROM events"),
			Some(Value::Int(1))
		);
		for app_database in ["my_app", "my_app__feat_charts"] {
			let reach = dev_server::with_database(&branch_url, app_database).unwrap();
			assert_eq!(first(&reach, "SELECT 1"), None, "{app_database}");
		}
		let listed: Vec<String> = branches::list(&app, &postgres)
			.unwrap()
			.into_iter()
			.map(|database| database.name)
			.collect();
		assert_eq!(
			listed,
			[
				"my_app__feat_charts",
				"my_app__feat_charts_service_analytics"
			],
			"`db branch prune` finds the service's copy with the app's"
		);

		// `db reset --service` creates it again, empty.
		let reset = service(
			&dev_server::with_database(&server.url, &branch).unwrap(),
			Existing::Recreate,
		);
		assert_eq!(reset, branch_url);
		assert_eq!(first(&reset, "SELECT count(*) FROM events"), None);
		assert_eq!(
			first(&url, "SELECT count(*) FROM events"),
			Some(Value::Int(1)),
			"the main branch's is untouched"
		);

		// Without a main branch's copy, a branch's starts empty, still its own.
		let cards = postgres_url(
			Some(&app),
			root.path(),
			&dev_server::with_database(&server.url, &branch).unwrap(),
			"cards",
			Existing::Keep,
		)
		.unwrap();
		assert!(
			cards.contains("/my_app__feat_charts_service_cards"),
			"{cards}"
		);
		let listed = branches::list(&app, &postgres).unwrap();
		assert!(
			listed.iter().any(|database| {
				database.name == "my_app__feat_charts_service_cards"
					&& database.branch == "feat/charts"
			}),
			"{listed:?}"
		);
	}

	#[test]
	fn names_follow_the_main_branchs_database() {
		let app = Project {
			root: PathBuf::from("/nowhere"),
			config: DbConfig::default(),
			package: "app_db".into(),
			name: "app".into(),
			service: None,
		};
		assert_eq!(
			PostgresNames::of(Some(&app), "app", "analytics"),
			PostgresNames {
				database: "app_service_analytics".into(),
				role: "app_service_analytics".into(),
				branch: None,
				app_main: "app".into(),
			}
		);
		let branch = PostgresNames::of(Some(&app), "app__feat_x", "analytics");
		assert_eq!(branch.database, "app__feat_x_service_analytics");
		assert_eq!(branch.role, "app_service_analytics");
		assert_eq!(branch.app_main, "app");
		// A database `db.url_env` names isn't a branch's.
		assert_eq!(
			PostgresNames::of(Some(&app), "staging", "analytics").role,
			"staging_service_analytics"
		);
		assert_eq!(
			PostgresNames::of(None, "app__feat_x", "analytics").role,
			"app__feat_x_service_analytics"
		);
	}

	#[test]
	fn migrations_follow_the_apps_engine() {
		let service = super::super::Service {
			settings: leptoskit_config::ResolvedService::merge(
				"analytics",
				&toml::from_str("package = \"analytics\"").unwrap(),
				None,
			)
			.unwrap(),
			code: super::super::Code::Workspace {
				package: "analytics".into(),
				manifest: PathBuf::from("services/analytics/Cargo.toml"),
			},
		};
		assert_eq!(
			migrations_dir(&service, DbDialect::Sqlite),
			Some(PathBuf::from("services/analytics/migrations"))
		);
		assert_eq!(
			migrations_dir(&service, DbDialect::Postgres),
			Some(PathBuf::from("services/analytics/migrations-postgres"))
		);
	}
}
