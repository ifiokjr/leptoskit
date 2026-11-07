//! A service's own database in development, which `leptoskit dev`,
//! `leptoskit serve`, and `leptoskit db --service` share: a SQLite file
//! under `.leptoskit/services/<name>/`, or on the development Postgres
//! `leptoskit` runs a database and a login role of the service's own.
//!
//! On Postgres the role is named after the main branch's database,
//! `<app>_service_<name>`, and owns the service's database on every branch,
//! so a branch's copy, `<app>__<branch>__service__<name>`, holds tables the
//! same role owns. Only the role connects to it, and the role can't connect
//! to any of the app's databases.
//!
//! Nothing here touches a server `leptoskit` didn't start: a service of an
//! app whose database is elsewhere gets its URL from
//! `LEPTOSKIT_SERVICE_<NAME>_DATABASE_URL`, the way a deploy gives it one.

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

/// The variable that gives a service its database's URL when `leptoskit`
/// doesn't make it one: `LEPTOSKIT_SERVICE_<NAME>_DATABASE_URL`, the name
/// deploys keep it under.
#[must_use]
pub fn url_env(service: &str) -> String {
	format!(
		"LEPTOSKIT_SERVICE_{}_DATABASE_URL",
		leptoskit_config::services::env_segment(service)
	)
}

/// The error for a service whose app database is on a server `leptoskit`
/// didn't start, so it creates nothing there.
pub(crate) fn not_on_the_dev_server(service: &str, app_url_env: &str) -> anyhow::Error {
	anyhow::anyhow!(
		"services.{service} needs a database of its own, and the app's database ({app_url_env}) \
		 is on a server leptoskit doesn't run, so it won't create roles or databases there; set \
		 {} to the service's database, or unset {app_url_env} to use the development Postgres",
		url_env(service)
	)
}

/// The names a service's Postgres database goes by beside the app's
/// database `app_database`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PostgresNames {
	/// The service's database beside `app_database`.
	pub database: String,
	/// The role that owns it, which is also the main branch's service
	/// database.
	pub role: String,
	/// The branch, when `app_database` is a branch's own copy.
	pub branch: Option<String>,
	/// The main branch's app database, whose branch copies start with it
	/// and `__`.
	pub app_main: String,
}

impl PostgresNames {
	/// The names for `service` beside `app_database`, which is a branch's
	/// database when it's `<main>__<branch>` and `app` has branch databases.
	///
	/// A branch's service database is `<main>__<branch>__service__<name>`.
	/// A branch's part never holds `__`, so no branch's app database, and no
	/// other branch's or service's database, can have the same name.
	pub(crate) fn of(app: &Project, app_database: &str, service: &str) -> Self {
		let main = app.database_name();
		let on_branch = app.config.branch_databases
			&& app_database
				.strip_prefix(main.as_str())
				.is_some_and(|rest| rest.starts_with("__"));
		let branch = on_branch
			.then(|| branches::branches(&app.root).and_then(|branches| branches.current))
			.flatten();
		let role = super::postgres_database(&main, service);
		let database = if on_branch {
			super::shortened(format!(
				"{app_database}__service__{}",
				service.replace('-', "_")
			))
		} else {
			role.clone()
		};

		Self {
			database,
			role,
			branch,
			app_main: main,
		}
	}
}

/// What [`dev_postgres_url`] does with a service's database that exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Existing {
	/// Keeps it.
	Keep,
	/// Drops it, and every row in it, and creates it again empty.
	Recreate,
}

/// The URL of `service`'s own database on the development Postgres
/// `leptoskit` runs, whose app database is `app_url`, creating the database
/// and its role when they're missing. Callers make sure `app_url` is that
/// server's: this drops databases, creates roles, and changes who may
/// connect.
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
pub(crate) fn dev_postgres_url(
	app: &Project,
	app_url: &str,
	service: &str,
	existing: Existing,
) -> Result<String> {
	let app_database = database_of(app_url)?;
	let names = PostgresNames::of(app, &app_database, service);
	let server = dev_server::with_database(app_url, "postgres")?;
	let superuser = first(
		&server,
		"SELECT rolsuper FROM pg_catalog.pg_roles WHERE rolname = current_user",
		vec![],
	)? == Some(Value::Bool(true));

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

	let password = role(&server, &app.root, service, &names.role)?;
	create(&server, &names, Some(&names.role), existing)?;
	lock_down(&server, &names, &app_database)?;
	let url = dev_server::with_database(app_url, &names.database)?;
	dev_server::with_login(&url, &names.role, &password)
}

/// Makes the service's login role unless it exists, and returns its
/// password, kept in `.leptoskit/services/<name>/postgres-password`.
///
/// The password is set only when the role is made, or when this checkout
/// has none kept for an existing role, as after deleting `.leptoskit`; it
/// reaches Postgres only as a SCRAM verifier, so no log holds it.
fn role(server: &str, root: &Path, service: &str, role: &str) -> Result<String> {
	let directory = super::dev_dir(root, service);
	std::fs::create_dir_all(&directory)
		.with_context(|| format!("could not create {}", directory.display()))?;
	let file = directory.join("postgres-password");
	let kept = file.is_file();
	let password = dev_server::remembered(&file, || {
		let mut bytes = [0_u8; 18];
		getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!("{error}"))?;
		Ok(hex::encode(bytes))
	})?;
	let exists = first(
		server,
		"SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = $1",
		vec![Value::Text(role.to_owned())],
	)?
	.is_some();

	if exists && kept {
		return Ok(password);
	}

	let verifier = postgres_protocol::password::scram_sha_256(password.as_bytes());
	let quoted = quote_identifier(role);
	let script = if exists {
		format!("ALTER ROLE {quoted} LOGIN PASSWORD {};", literal(&verifier))
	} else {
		format!(
			"CREATE ROLE {quoted} LOGIN PASSWORD {};",
			literal(&verifier)
		)
	};
	crate::db::runtime()?.block_on(async {
		let db = Db::connect(server).await?;
		anyhow::Ok(db.run_script(&script).await?)
	})?;
	Ok(password)
}

/// Revokes `PUBLIC`'s `CONNECT` on the service's database and on every one
/// of the app's on the development server: the main branch's, the current
/// branch's, and every other branch's copy, which `CREATE DATABASE …
/// TEMPLATE` gives `PUBLIC`'s default access again. Only each database's
/// owner, and the app's login, a superuser here, still connect.
fn lock_down(server: &str, names: &PostgresNames, app_database: &str) -> Result<()> {
	let rows = crate::db::runtime()?.block_on(async {
		let db = Db::connect(server).await?;
		anyhow::Ok(
			db.fetch(Statement {
				sql: "SELECT datname FROM pg_catalog.pg_database WHERE datname = $1 OR \
				      starts_with(datname, $2) ORDER BY datname"
					.to_owned(),
				args: vec![
					Value::Text(names.app_main.clone()),
					Value::Text(format!("{}__", names.app_main)),
				],
			})
			.await?,
		)
	})?;
	let mut databases: Vec<String> = rows
		.into_iter()
		.filter_map(|row| {
			match row.into_iter().next() {
				Some(Value::Text(name)) => Some(name),
				_ => None,
			}
		})
		.collect();

	for database in [&names.database, &app_database.to_owned()] {
		if !databases.contains(database) {
			databases.push(database.clone());
		}
	}

	let mut script = String::new();

	for database in &databases {
		let _ = writeln!(
			script,
			"REVOKE CONNECT ON DATABASE {} FROM PUBLIC;",
			quote_identifier(database)
		);
	}

	crate::db::runtime()?.block_on(async {
		let db = Db::connect(server).await?;
		anyhow::Ok(db.run_script(&script).await?)
	})
}

/// The first column of the first row `sql` returns on `url`.
fn first(url: &str, sql: &str, args: Vec<Value>) -> Result<Option<Value>> {
	crate::db::runtime()?.block_on(async {
		let db = Db::connect(url).await?;
		let rows = db
			.fetch(Statement {
				sql: sql.to_owned(),
				args,
			})
			.await?;
		anyhow::Ok(
			rows.into_iter()
				.next()
				.and_then(|row| row.into_iter().next()),
		)
	})
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

	/// The service's first start on the main branch: its role's password
	/// reaches Postgres only as a SCRAM verifier, a second start leaves it
	/// alone, and a checkout that lost the kept password sets a new one.
	/// Returns the service's URL.
	fn passwords_are_set_once_as_verifiers(app: &Project, app_url: &str) -> String {
		let service = || dev_postgres_url(app, app_url, "analytics", Existing::Keep).unwrap();
		let admin = dev_server::with_database(app_url, "postgres").unwrap();
		let verifier = || {
			first(
				&admin,
				"SELECT rolpassword FROM pg_authid WHERE rolname = 'my_app_service_analytics'",
			)
		};
		let url = service();
		let parsed = url::Url::parse(&url).unwrap();
		assert_eq!(parsed.username(), "my_app_service_analytics");
		assert_eq!(parsed.path(), "/my_app_service_analytics");
		let set = verifier();
		assert!(
			matches!(&set, Some(Value::Text(text)) if text.starts_with("SCRAM-SHA-256$")),
			"the password reaches Postgres only as a verifier: {set:?}"
		);
		assert_eq!(service(), url, "a second start keeps the role's password");
		assert_eq!(verifier(), set, "and doesn't set it again");

		std::fs::remove_file(
			super::super::dev_dir(&app.root, "analytics").join("postgres-password"),
		)
		.unwrap();
		let url = service();
		assert_ne!(verifier(), set);
		assert_eq!(first(&url, "SELECT 1"), Some(Value::Int(1)));
		url
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
			dev_postgres_url(&app, app_url, "analytics", existing).unwrap()
		};

		// Another branch's database, made before the service ever ran.
		git(root.path(), &["checkout", "--quiet", "-b", "feat/other"]);
		let postgres = dev_server::with_database(&server.url, "postgres").unwrap();
		assert_eq!(
			branches::ensure(&app, &postgres).unwrap(),
			"my_app__feat_other"
		);
		git(root.path(), &["checkout", "--quiet", "main"]);

		let url = passwords_are_set_once_as_verifiers(&app, &server.url);

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
		let branch = branches::ensure(&app, &postgres).unwrap();
		assert_eq!(branch, "my_app__feat_charts");
		let branch_url = service(
			&dev_server::with_database(&server.url, &branch).unwrap(),
			Existing::Keep,
		);
		let parsed = url::Url::parse(&branch_url).unwrap();
		assert_eq!(parsed.username(), "my_app_service_analytics");
		assert_eq!(parsed.path(), "/my_app__feat_charts__service__analytics");
		assert_eq!(
			first(&branch_url, "SELECT count(*) FROM events"),
			Some(Value::Int(1))
		);
		// Nor any branch's app database, even one made before the service ran.
		for app_database in ["my_app", "my_app__feat_charts", "my_app__feat_other"] {
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
				"my_app__feat_charts__service__analytics",
				"my_app__feat_other"
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
		let cards = dev_postgres_url(
			&app,
			&dev_server::with_database(&server.url, &branch).unwrap(),
			"cards",
			Existing::Keep,
		)
		.unwrap();
		assert!(
			cards.contains("/my_app__feat_charts__service__cards"),
			"{cards}"
		);
		let listed = branches::list(&app, &postgres).unwrap();
		assert!(
			listed.iter().any(|database| {
				database.name == "my_app__feat_charts__service__cards"
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
			PostgresNames::of(&app, "app", "analytics"),
			PostgresNames {
				database: "app_service_analytics".into(),
				role: "app_service_analytics".into(),
				branch: None,
				app_main: "app".into(),
			}
		);
		let branch = PostgresNames::of(&app, "app__feat_x", "analytics");
		assert_eq!(branch.database, "app__feat_x__service__analytics");
		assert_eq!(branch.role, "app_service_analytics");
		assert_eq!(branch.app_main, "app");

		// A branch's slug never holds `__`, so a branch named like another
		// branch's service database gets a different database.
		let on = |current: &str| {
			branches::Branches {
				current: Some(current.to_owned()),
				main: "main".into(),
			}
		};
		let lookalike = branches::database_for("app", &on("x-service-analytics"));
		assert_eq!(lookalike, "app__x_service_analytics");
		assert_ne!(
			PostgresNames::of(&app, "app__x", "analytics").database,
			lookalike
		);

		// Long branches that share their first 50 bytes still differ.
		let shop = Project {
			name: "shop".into(),
			..app
		};
		let names: Vec<String> = ["part-1", "part-2"]
			.into_iter()
			.map(|part| {
				let branch = branches::database_for(
					"shop",
					&on(&format!(
						"feature/add-service-database-support-to-the-cli-{part}"
					)),
				);
				PostgresNames::of(&shop, &branch, "analytics").database
			})
			.collect();
		assert!(names.iter().all(|name| name.len() <= 63), "{names:?}");
		assert_ne!(names[0], names[1]);
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
