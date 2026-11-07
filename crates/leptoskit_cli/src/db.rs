//! `leptoskit db`: migrations generated from the app's `#[db::table]` structs.
//!
//! The schema comes from the app itself: the db crate calls
//! `leptoskit::db::export_schema!()`, which adds a test that writes every
//! table as `schema.json` when this command asks. A new migration is the
//! difference between that schema and the snapshot committed beside the
//! latest migration, so generating one never needs a database.

use std::collections::BTreeSet;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::io::BufRead;
use std::io::IsTerminal;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use anyhow::ensure;
use leptoskit_config::DbConfig;
use leptoskit_config::DbDialect;
use leptoskit_config::DbDraft;
use leptoskit_config::DbWatch;
use leptoskit_config::LeptoskitConfig;
use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_db_schema::Column;
use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::DiffError;
use leptoskit_db_schema::NoRenames;
use leptoskit_db_schema::RenameResolver;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Risk;
use leptoskit_db_schema::Schema;
use leptoskit_db_schema::Table;
use leptoskit_db_schema::TableName;
use sha2::Digest as _;
use sha2::Sha256;

use crate::cli::invoked_name;

pub(crate) mod branches;
pub(crate) mod dev_server;
mod push;
mod watch;

pub(crate) use watch::DevDatabase;
pub(crate) use watch::StudioProcess;

/// The `leptoskit db` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum DbCommand {
	/// Manages migrations.
	Migration {
		#[command(subcommand)]
		command: MigrationCommand,
	},
	/// Manages release markers: the lines in `registry.txt` that say which
	/// migrations have shipped, so they can no longer change.
	Release {
		#[command(subcommand)]
		command: ReleaseCommand,
	},
	/// Fails when the tables changed without a migration, the migrations
	/// directory is inconsistent, or, with sign-in, a table that holds an
	/// account's data doesn't say how its rows move when accounts merge.
	/// Warns about vector columns clients can see, and, on SQLite, with a
	/// database to look at, about tables with vector columns grown past
	/// `db.vector.exact_search_rows`. With `--live`, also checks the
	/// connected database's capture triggers. For CI.
	Check {
		/// Check the connected database's live pack: every trigger the schema
		/// expects exists, unchanged, and no capture trigger copies a wide
		/// column.
		#[arg(long)]
		live: bool,
		/// The database `--live` checks and vector row counts read; defaults
		/// to the variable `db.url_env` names. Without either, row counts are
		/// skipped.
		#[arg(long)]
		url: Option<String>,
	},
	/// Shows the migrations, whether the tables changed since the latest, and,
	/// when the database URL is set, which ones the database still needs.
	Status {
		/// The database URL; defaults to the variable `db.url_env` names.
		#[arg(long)]
		url: Option<String>,
	},
	/// Applies the migrations the database still needs.
	Migrate {
		/// The database URL; defaults to the variable `db.url_env` names.
		#[arg(long)]
		url: Option<String>,
	},
	/// Runs reviewed down.sql files, newest first. Stop application writers
	/// and back up the database before using this command.
	Rollback {
		/// Database URL; defaults to `db.url_env`.
		#[arg(long)]
		url: Option<String>,
		/// Number of applied migrations to reverse.
		#[arg(long, default_value_t = 1)]
		steps: usize,
		/// Confirm reverse SQL may delete data.
		#[arg(long)]
		yes: bool,
	},
	/// Brings the development database to the tables without writing a
	/// migration, as `leptoskit dev` does with `db.watch = "push"`.
	/// `leptoskit db migration create` later turns every push into one
	/// migration.
	Push {
		/// The database URL; defaults to the variable `db.url_env` names.
		#[arg(long)]
		url: Option<String>,
		/// Allow steps that delete data, such as dropping a column.
		#[arg(long)]
		allow_destructive: bool,
		/// Remove this schema pack deliberately. Required when the database
		/// still has a pack the tables no longer include.
		#[arg(long = "drop-pack", value_name = "PACK")]
		drop_packs: Vec<String>,
	},
	/// Lists or deletes the databases git branches use on the development
	/// Postgres.
	Branch {
		#[command(subcommand)]
		command: BranchCommand,
	},
	/// Serves the studio: a browser UI for the database, on this machine only,
	/// at an address with a token made for the run. It reads every table
	/// through the app's own policies, as the viewer you choose, and changes
	/// rows only in edit mode, after you confirm each change. Release builds
	/// never contain it.
	Studio {
		/// The database URL; defaults to the variable `db.url_env` names.
		#[arg(long)]
		url: Option<String>,
		/// The port to listen on, on 127.0.0.1; by default a free one.
		#[arg(long, default_value_t = 0)]
		port: u16,
	},
	/// Recreates a local development database from the migrations, deleting
	/// every row.
	Reset {
		/// The database URL; defaults to the variable `db.url_env` names.
		#[arg(long)]
		url: Option<String>,
		/// Confirms that every row may be deleted.
		#[arg(long)]
		yes: bool,
	},
}

/// The `leptoskit db branch` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum BranchCommand {
	/// Lists each branch's database.
	List,
	/// Deletes the databases of branches that are deleted or merged into the
	/// main branch.
	Prune {
		/// Confirms that the databases, and their rows, may be deleted.
		#[arg(long)]
		yes: bool,
	},
}

/// The `leptoskit db migration` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum MigrationCommand {
	/// Writes a migration for everything the tables changed since the latest
	/// one.
	///
	/// When a column or table disappeared while a similar one appeared, asks
	/// whether it was renamed. Without a terminal, the answer has to come from
	/// `#[db(renamed_from = "...")]`, or `--no-renames`.
	Create {
		/// What the migration does, such as `add_posts`.
		name: String,
		/// Allow steps that delete data, such as dropping a column.
		#[arg(long)]
		allow_destructive: bool,
		/// Treat every disappeared table or column as dropped, never renamed.
		#[arg(long)]
		no_renames: bool,
		/// Write an empty migration for changing data rather than tables.
		#[arg(long)]
		empty: bool,
		/// Remove this schema pack deliberately. Required when the tables no
		/// longer include a pack the latest migration knew about, even with
		/// `--allow-destructive`; name each removed pack once.
		#[arg(long = "drop-pack", value_name = "PACK")]
		drop_packs: Vec<String>,
	},
	/// Folds every migration after the latest release marker into one, written
	/// from a fresh diff of the release's baseline to the current tables, so
	/// the migrations directory holds one entry per release that changed the
	/// schema. Databases that already applied the folded migrations record
	/// the consolidated one without running it; fresh databases run it.
	///
	/// A migration whose SQL no longer matches what its snapshots generate —
	/// a data migration, or one written by an older leptoskit — is refused
	/// unless it is named in `--keep`, which leaves it and every migration
	/// after it in place, after the consolidated one.
	Consolidate {
		/// Migration names to leave out, and keep, after the consolidated
		/// one. Only the newest migrations can be kept.
		#[arg(long = "keep")]
		keep: Vec<String>,
		/// Treat every disappeared table or column as dropped, never renamed.
		#[arg(long)]
		no_renames: bool,
		/// Remove this schema pack deliberately. Required when the tables no
		/// longer include a pack the latest migration knew about.
		#[arg(long = "drop-pack", value_name = "PACK")]
		drop_packs: Vec<String>,
	},
}

/// The `leptoskit db release` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum ReleaseCommand {
	/// Records that the migrations above the marker shipped in a release,
	/// freezing them: they can no longer change, and the next release's
	/// `migration consolidate` folds everything after them into one migration.
	Record {
		/// The release's version, such as `1.4.0`.
		#[arg(id = "release_version", value_name = "VERSION")]
		version: String,
		/// Where the release came from, such as a monochange release id.
		#[arg(long)]
		id: Option<String>,
	},
}

/// Runs a `leptoskit db` subcommand on the app's database, or on the
/// database of the service `service` names.
pub fn run(command: DbCommand, service: Option<&str>) -> Result<()> {
	let project = project_for(&command, service)?;

	match command {
		DbCommand::Migration {
			command:
				MigrationCommand::Create {
					name,
					allow_destructive,
					no_renames,
					empty,
					drop_packs,
				},
		} => {
			let target = project.export()?;
			let options = CreateOptions {
				allow_destructive,
				empty,
				drop_packs,
			};
			// In push mode, the migration takes over the development
			// database's pushes.
			let (url, _server) = if project.config.watch == DbWatch::Push {
				let (url, server) = project.database(None)?;
				(Some(url), server)
			} else {
				(None, None)
			};
			let created = if no_renames {
				create_squashing(
					&project,
					&name,
					&target,
					&options,
					&mut NoRenames,
					url.as_deref(),
				)
			} else if std::io::stdin().is_terminal() {
				create_squashing(&project, &name, &target, &options, &mut Ask, url.as_deref())
			} else {
				create_squashing(
					&project,
					&name,
					&target,
					&options,
					&mut RequireHints,
					url.as_deref(),
				)
			}?;

			print!("{}", created.summary());
			Ok(())
		}
		DbCommand::Migration {
			command: MigrationCommand::Consolidate {
				keep,
				no_renames,
				drop_packs,
			},
		} => run_consolidate(&project, &keep, no_renames, &drop_packs),
		DbCommand::Release {
			command: ReleaseCommand::Record { version, id },
		} => record_release(&project, &version, id.as_deref()),
		DbCommand::Check { live, url } => {
			let history = History::read(&project.migrations_dir(), project.dialect())?;
			let target = project.export()?;
			check(&history, &target)?;
			println!(
				"the tables match {}",
				history.latest_name().unwrap_or("an empty database")
			);

			if let Some(summary) = check_account_merges(&target)? {
				println!("{summary}");
			}

			for warning in public_vectors(&target) {
				println!("warning: {warning}");
			}

			let url = url.or_else(|| std::env::var(&project.config.url_env).ok());

			if let (Some(url), Dialect::Sqlite) = (&url, project.dialect()) {
				let counts = runtime()?.block_on(count_searched_rows(url, &target))?;

				for warning in exact_searches(&counts, project.config.vector.exact_search_rows) {
					println!("warning: {warning}");
				}
			}

			if live {
				check_live(&project, &target, url)?;
			}

			Ok(())
		}
		DbCommand::Status { url } => status(&project, url),
		DbCommand::Studio { url, port } => {
			let (url, _server) = project.database(url)?;
			println!("building the studio into {}", project.package);
			let status = project
				.studio(&url, port)?
				.status()
				.with_context(|| format!("couldn't run cargo for {}", project.package))?;
			ensure!(status.success(), "the studio stopped: {status}");
			Ok(())
		}
		DbCommand::Migrate { url } => {
			let (url, _server) = project.database(url)?;
			let migrations = leptoskit_db::load_migrations(&project.migrations_dir())?;
			let applied = runtime()?.block_on(async {
				let db = Db::connect(&url).await?;
				anyhow::Ok(db.migrate(&migrations).await?)
			})?;

			if applied.is_empty() {
				println!("the database is up to date");
			} else {
				for name in applied {
					println!("applied {name}");
				}
			}

			Ok(())
		}
		DbCommand::Rollback { url, steps, yes } => {
			ensure!(
				yes,
				"rollback can delete data; stop application writers, verify a backup, then pass \
				 --yes"
			);
			ensure!(steps > 0, "--steps must be positive");
			let (url, _server) = project.database(url)?;
			let migrations = leptoskit_db::load_migrations(&project.migrations_dir())?;
			let reverted = runtime()?.block_on(async {
				let db = Db::connect(&url).await?;
				anyhow::Ok(db.rollback(&migrations, steps).await?)
			})?;
			for name in reverted {
				println!("reverted {name}");
			}
			Ok(())
		}
		DbCommand::Push {
			url,
			allow_destructive,
			drop_packs,
		} => {
			let target = project.export()?;
			let (url, _server) = project.database(url)?;
			push_command(&project, &target, &url, allow_destructive, &drop_packs)
		}
		DbCommand::Branch { command } => {
			branch(
				&project,
				&command,
				dev_server::binaries_from_env().as_deref(),
			)
		}
		DbCommand::Reset { url, yes } => reset(&project, url, yes),
	}
}

/// The database `command` works on: the app's, or the service's that
/// `--service` names, refusing what doesn't apply to a service.
fn project_for(command: &DbCommand, service: Option<&str>) -> Result<Project> {
	let Some(service) = service else {
		return Project::load();
	};
	ensure!(
		!matches!(command, DbCommand::Branch { .. }),
		"a service's branch databases follow the app's: `{} db branch` lists and prunes them with \
		 the app's, so run it without --service",
		invoked_name()
	);
	let (config, root) = LeptoskitConfig::load()?;
	let project = Project::service(&config, root, service)?;

	if matches!(
		command,
		DbCommand::Migration { .. } | DbCommand::Release { .. }
	) {
		project.ensure_writable()?;
	}

	Ok(project)
}

/// `leptoskit db push`: pushes `target` to the database at `url`, and fails
/// when that would delete data without `allow_destructive`.
fn push_command(
	project: &Project,
	target: &Schema,
	url: &str,
	allow_destructive: bool,
	drop_packs: &[String],
) -> Result<()> {
	let pushed = push::push(project, url, target, allow_destructive, drop_packs)?;
	let refused = matches!(pushed, push::Pushed::Refused(_));
	print!(
		"{}",
		pushed
			.message()
			.unwrap_or_else(|| "the database already has the tables\n".to_owned())
	);
	ensure!(!refused, "nothing was pushed");
	Ok(())
}

/// Lists or prunes the branch databases on the development Postgres,
/// running `binaries` when the embedded server has to start.
fn branch(project: &Project, command: &BranchCommand, binaries: Option<&Path>) -> Result<()> {
	ensure!(
		std::env::var_os(&project.config.url_env).is_none(),
		"branch databases live on the development Postgres, but {} names another database",
		project.config.url_env
	);
	let Some(source) = dev_server::source(project) else {
		bail!("branch databases need the development Postgres; db.dev_server is off");
	};
	let server = dev_server::start(project, source, binaries)?;
	let url = dev_server::with_database(&server.url, "postgres")?;
	let databases = branches::list(project, &url)?;

	match command {
		BranchCommand::List => {
			if databases.is_empty() {
				println!(
					"no branch has its own database; the main branch's is {}",
					project.database_name()
				);
			}

			for database in &databases {
				println!("{}  {}", database.name, database.branch);
			}
		}
		BranchCommand::Prune { yes } => {
			let prunable = branches::prunable(&project.root, &databases)?;

			if prunable.is_empty() {
				println!("every branch database belongs to a branch still in use");
				return Ok(());
			}

			for (database, gone) in &prunable {
				let why = match gone {
					branches::Gone::Deleted => "deleted",
					branches::Gone::Merged => "merged",
				};

				if *yes {
					branches::drop_database(&url, &database.name)?;
					println!(
						"deleted {} (branch `{}`, {why})",
						database.name, database.branch
					);
				} else {
					println!(
						"would delete {} (branch `{}`, {why})",
						database.name, database.branch
					);
				}
			}

			if !yes {
				println!("pass --yes to delete them, and every row in them");
			}
		}
	}

	Ok(())
}

/// The app's database settings, resolved, or a service's.
pub(crate) struct Project {
	pub root: PathBuf,
	pub config: DbConfig,
	pub package: String,
	/// The app's name, `app.name`, or the service's.
	pub name: String,
	/// The service whose database this is, under `--service`.
	pub service: Option<ServiceProject>,
}

/// What a service's database adds to its [`Project`].
pub(crate) struct ServiceProject {
	/// The app's database settings, whose development server the service's
	/// database shares; `None` when the app has no `[db]`, so the service's
	/// is SQLite.
	pub app: Option<Box<Project>>,
	/// The service crate's manifest and the target directory to build it in,
	/// when it isn't a member of the app's workspace.
	pub external: Option<(PathBuf, PathBuf)>,
	/// Where cargo fetched it from, when that's a registry or git: its
	/// migrations ship with it, so `leptoskit` doesn't write to them.
	pub installed: Option<&'static str>,
}

impl Project {
	/// The project's database settings, or an error naming what's missing
	/// when it has none.
	pub(crate) fn load() -> Result<Self> {
		let (config, root) = LeptoskitConfig::load()?;
		Self::of(&config, root).with_context(|| {
			format!(
				"the database is off; add a `[db]` table to {}",
				leptoskit_config::CONFIG_FILE
			)
		})
	}

	/// The database settings in `config`, when it has some.
	pub(crate) fn of(config: &LeptoskitConfig, root: PathBuf) -> Option<Self> {
		let db = config.db.as_ref()?;
		Some(Self {
			package: db.package(&config.app),
			name: config.app.name.clone(),
			config: db.clone(),
			root,
			service: None,
		})
	}

	/// The database of the service `[services.<name>]` declares: on the
	/// app's engine, with the service crate's tables, and its migrations in
	/// `migrations/` beside its `Cargo.toml`, or `migrations-postgres/` when
	/// the app is on Postgres.
	pub(crate) fn service(config: &LeptoskitConfig, root: PathBuf, name: &str) -> Result<Self> {
		if !config.services.contains_key(name) {
			let known: Vec<&str> = config.services.keys().map(String::as_str).collect();
			bail!(
				"there's no [services.{name}] in {}{}",
				leptoskit_config::CONFIG_FILE,
				if known.is_empty() {
					String::new()
				} else {
					format!("; its services are {}", known.join(", "))
				}
			);
		}

		let metadata = crate::wasm::cargo_metadata(&root)?;
		let service = crate::services::resolve(config, &root, &metadata, false)?
			.into_iter()
			.find(|service| service.name() == name)
			.with_context(|| format!("[services.{name}] didn't resolve"))?;
		let (package, external) = match &service.code {
			crate::services::Code::Workspace { package, .. } => (package.clone(), None),
			crate::services::Code::External { package, manifest } => {
				let target = metadata
					.target_directory
					.join("leptoskit")
					.join("services")
					.join("target");
				(package.clone(), Some((manifest.clone(), target)))
			}
			crate::services::Code::Remote => {
				bail!(
					"[services.{name}] is remote: it runs elsewhere, with a database leptoskit \
					 doesn't manage"
				)
			}
		};
		ensure!(
			service.settings.database,
			"[services.{name}] has no database of its own; set `database = true` in it, or in its \
			 crate's [package.metadata.leptoskit.service]"
		);

		let app = Self::of(config, root.clone());
		let dialect = crate::services::dialect(config);
		let migrations = crate::services::database::migrations_dir(&service, dialect)
			.context("a service's manifest has no directory")?;
		let defaults = DbConfig::default();
		let db = DbConfig {
			dialect,
			package: Some(package.clone()),
			features: service.settings.features.clone(),
			migrations,
			url_env: format!(
				"LEPTOSKIT_SERVICE_{}_DATABASE_URL",
				leptoskit_config::services::env_segment(name)
			),
			dev_server: app
				.as_ref()
				.map_or(defaults.dev_server, |app| app.config.dev_server),
			draft: app.as_ref().map_or(defaults.draft, |app| app.config.draft),
			branch_databases: app.as_ref().is_some_and(|app| app.config.branch_databases),
			..defaults
		};
		let installed = match &service.settings.source {
			leptoskit_config::ServiceSource::Crate { .. } => Some("a registry"),
			leptoskit_config::ServiceSource::Git { .. } => Some("git"),
			_ => None,
		};

		Ok(Self {
			root,
			config: db,
			package,
			name: name.to_owned(),
			service: Some(ServiceProject {
				app: app.map(Box::new),
				external,
				installed,
			}),
		})
	}

	/// Fails for a service installed from a registry or git, whose
	/// migrations directory is in cargo's copy of its source.
	fn ensure_writable(&self) -> Result<()> {
		if let Some(from) = self.service.as_ref().and_then(|service| service.installed) {
			bail!(
				"services.{} comes from {from}, and its migrations ship with it; change them in \
				 its own repository",
				self.name
			);
		}

		Ok(())
	}

	/// The development database's name: the app's, with `_` for `-`.
	pub(crate) fn database_name(&self) -> String {
		self.name.replace('-', "_")
	}

	pub(crate) fn dialect(&self) -> Dialect {
		match self.config.dialect {
			DbDialect::Postgres => Dialect::Postgres,
			DbDialect::Sqlite => Dialect::Sqlite,
		}
	}

	pub(crate) fn migrations_dir(&self) -> PathBuf {
		self.root.join(&self.config.migrations)
	}

	/// The database a command works on: `--url`, the variable `db.url_env`
	/// names, or the development database, which stays up while the server
	/// returned with it lives.
	fn database(&self, url: Option<String>) -> Result<(String, Option<dev_server::DevServer>)> {
		self.database_with(url, crate::services::database::Existing::Keep)
	}

	/// [`Self::database`], with a service's development database on
	/// Postgres created again, empty, under `existing`.
	fn database_with(
		&self,
		url: Option<String>,
		existing: crate::services::database::Existing,
	) -> Result<(String, Option<dev_server::DevServer>)> {
		if let Some(url) = url.or_else(|| std::env::var(&self.config.url_env).ok()) {
			return Ok((url, None));
		}

		if let Some(service) = &self.service {
			return self.service_database(service, existing);
		}

		let Some(source) = dev_server::source(self) else {
			bail!(
				"set {} to the database URL, or pass --url",
				self.config.url_env
			);
		};
		let binaries = dev_server::binaries_from_env();
		let server = dev_server::start(self, source, binaries.as_deref())?;
		println!("using the development database {}", server.display_url());
		Ok((server.url.clone(), Some(server)))
	}

	/// A service's development database, where `leptoskit dev` gives it one:
	/// its SQLite file, or its own database on the app's Postgres, beside
	/// the database the app's `db.url_env` names or the branch's on the
	/// development server.
	fn service_database(
		&self,
		service: &ServiceProject,
		existing: crate::services::database::Existing,
	) -> Result<(String, Option<dev_server::DevServer>)> {
		let Some(app) = service
			.app
			.as_deref()
			.filter(|_| self.dialect() == Dialect::Postgres)
		else {
			let url = crate::services::database::sqlite_url(&self.root, &self.name)?;
			println!("using the development database {url}");
			return Ok((url, None));
		};
		let (app_url, server) = if let Ok(url) = std::env::var(&app.config.url_env) {
			(url, None)
		} else {
			let Some(source) = dev_server::source(app) else {
				bail!(
					"set {} to the service's database URL, or {} to the app's, or pass --url",
					self.config.url_env,
					app.config.url_env
				);
			};
			let binaries = dev_server::binaries_from_env();
			let server = dev_server::start(app, source, binaries.as_deref())?;
			(server.url.clone(), Some(server))
		};
		let url = crate::services::database::postgres_url(
			Some(app),
			&self.root,
			&app_url,
			&self.name,
			existing,
		)?;
		let database = crate::services::database::database_of(&url)?;
		println!("using the service's development database `{database}`");
		Ok((url, server))
	}

	/// The schema the app's tables declare, from the test
	/// `export_schema!()` adds to the db crate.
	pub(crate) fn export(&self) -> Result<Schema> {
		let out =
			tempfile::NamedTempFile::new().context("couldn't create a file for the schema")?;
		let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
		let mut command = Command::new(cargo);
		command.current_dir(&self.root).args([
			"test",
			"--quiet",
			"--package",
			&self.package,
			"--lib",
		]);
		self.outside_the_workspace(&mut command);
		command
			.env(leptoskit_db::SCHEMA_OUT_ENV, out.path())
			.env(leptoskit_db::DIALECT_ENV, self.config.dialect.as_str())
			.env(leptoskit_db::POLICIES_ENV, self.config.policies.as_str())
			.env(
				leptoskit_db::LIVE_CAPTURE_ENV,
				self.config.live_capture.as_str(),
			)
			.env(
				leptoskit_db::GEOPACKAGE_ENV,
				if self.config.geo.geopackage { "1" } else { "0" },
			);

		if !self.config.features.is_empty() {
			command.args(["--features", &self.config.features.join(",")]);
		}

		command.args(["--", "__leptoskit_db_export_schema"]);
		println!("reading the tables in {}", self.package);
		let output = command
			.output()
			.with_context(|| format!("couldn't run cargo for {}", self.package))?;
		let json = fs::read_to_string(out.path()).unwrap_or_default();

		if !output.status.success() {
			bail!(
				"couldn't read the tables in {}:\n{}{}",
				self.package,
				String::from_utf8_lossy(&output.stdout),
				String::from_utf8_lossy(&output.stderr)
			);
		}

		ensure!(
			!json.is_empty(),
			"{} doesn't export its tables; add `leptoskit::db::export_schema!();` to its \
			 src/lib.rs",
			self.package
		);
		Ok(Schema::from_json(&json)?)
	}
}

impl Project {
	/// Points `command`, a cargo command, at a service crate outside the
	/// app's workspace, building it where installed services build.
	fn outside_the_workspace(&self, command: &mut Command) {
		if let Some((manifest, target)) = self
			.service
			.as_ref()
			.and_then(|service| service.external.as_ref())
		{
			command
				.arg("--manifest-path")
				.arg(manifest)
				.arg("--target-dir")
				.arg(target);
		}
	}

	/// The command that serves the studio on `127.0.0.1:port` over the
	/// database at `url`: the db crate's `export_schema!()` test, built with
	/// the studio and the dialect's driver, which serves until it's stopped.
	pub(crate) fn studio(&self, url: &str, port: u16) -> Result<Command> {
		let manifest = self.manifest()?;
		let mut features = self.config.features.clone();
		features.extend(studio_features(&manifest, self.config.dialect)?);
		let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
		let mut command = Command::new(cargo);
		command
			.current_dir(&self.root)
			.args(["test", "--quiet", "--package", &self.package, "--lib"])
			.args(["--features", &features.join(",")])
			.env(leptoskit_db::STUDIO_ENV, format!("127.0.0.1:{port}"))
			.env(leptoskit_db::STUDIO_DATABASE_ENV, url)
			.env(leptoskit_db::STUDIO_MIGRATIONS_ENV, self.migrations_dir())
			.env(leptoskit_db::DIALECT_ENV, self.config.dialect.as_str())
			.env(leptoskit_db::POLICIES_ENV, self.config.policies.as_str())
			.env(
				leptoskit_db::LIVE_CAPTURE_ENV,
				self.config.live_capture.as_str(),
			)
			.args(["--", "__leptoskit_db_export_schema", "--nocapture"]);
		Ok(command)
	}

	/// The db crate's `Cargo.toml`.
	fn manifest(&self) -> Result<toml::Table> {
		let metadata = crate::wasm::cargo_metadata(&self.root)?;
		let package = metadata
			.packages
			.iter()
			.find(|package| package.name == self.package)
			.with_context(|| format!("no package in the workspace is named {}", self.package))?;
		let text = fs::read_to_string(&package.manifest_path)
			.with_context(|| format!("couldn't read {}", package.manifest_path.display()))?;
		text.parse()
			.with_context(|| format!("couldn't parse {}", package.manifest_path.display()))
	}
}

/// The features that turn the studio on in a db crate with `manifest`,
/// through whichever of `leptoskit_db` and `leptoskit` it depends on, with
/// the dialect's driver, which the studio connects with.
fn studio_features(manifest: &toml::Table, dialect: DbDialect) -> Result<Vec<String>> {
	let depends_on = |name: &str| {
		manifest
			.get("dependencies")
			.and_then(toml::Value::as_table)
			.is_some_and(|dependencies| dependencies.contains_key(name))
	};
	let dialect = dialect.as_str();

	if depends_on("leptoskit_db") {
		Ok(vec![
			"leptoskit_db/studio".into(),
			format!("leptoskit_db/{dialect}"),
		])
	} else if depends_on("leptoskit") {
		Ok(vec![
			"leptoskit/db-studio".into(),
			format!("leptoskit/db-{dialect}"),
		])
	} else {
		bail!("the db crate depends on neither leptoskit_db nor leptoskit, so it has no studio")
	}
}

/// The committed migrations, and the schema the latest one produces.
pub(crate) struct History {
	pub names: Vec<String>,
	pub latest: Schema,
	/// The release markers, in the order they appear in `registry.txt`.
	pub releases: Vec<ReleaseMarker>,
	/// How many migrations the latest release froze. Everything before that
	/// point has shipped and can no longer change; everything after it is
	/// still a draft the next release will consolidate.
	pub published: usize,
}

/// A release marker from `registry.txt`, freezing every migration above it.
#[derive(Debug, Clone)]
pub(crate) struct ReleaseMarker {
	pub version: String,
	pub source: Option<String>,
	pub snapshot: String,
}

impl History {
	/// Reads `registry.txt` and the latest snapshot, checking that every
	/// listed migration exists, every migration is listed, and every release
	/// marker still pins the schema the release ended at.
	pub(crate) fn read(directory: &Path, dialect: Dialect) -> Result<Self> {
		let registry = directory.join("registry.txt");

		if !registry.exists() {
			return Ok(Self {
				names: Vec::new(),
				latest: Schema::empty(dialect),
				releases: Vec::new(),
				published: 0,
			});
		}

		let text = fs::read_to_string(&registry)
			.with_context(|| format!("couldn't read {}", registry.display()))?;

		let mut names: Vec<String> = Vec::new();
		let mut releases: Vec<ReleaseMarker> = Vec::new();
		let mut published = 0;

		for line in text.lines().map(str::trim) {
			if !line.starts_with("# release ") {
				if !line.is_empty() && !line.starts_with('#') {
					names.push(line.to_owned());
				}
				continue;
			}

			let Some(release) = leptoskit_db::parse_release(line) else {
				bail!(
					"registry.txt has a release marker that isn't `# release <version> \
					 [(<source>)] snapshot <sha256>`:\n  {line}"
				);
			};

			let Some(above) = names.last() else {
				bail!(
					"the release marker for {} has no migration above it; a release freezes the \
					 migrations that shipped in it",
					release.version
				);
			};

			// The marker pins the schema the release ended at, so editing a
			// published migration's snapshot is caught here, before the
			// checksums of applied databases ever see it.
			let path = directory.join(above).join("schema.json");
			let snapshot = Sha256::digest(
				fs::read(&path)
					.with_context(|| format!("couldn't read {}", path.display()))?
					.as_slice(),
			);
			let pinned = snapshot
				.iter()
				.fold(String::with_capacity(64), |mut hex, byte| {
					let _ = write!(hex, "{byte:02x}");
					hex
				});
			ensure!(
				pinned == release.snapshot,
				"the release marker for {} pins the wrong schema: `{above}` now hashes to \
				 {pinned}. Restore the marker and the migration from git if it changed, or \
				 rerecord the release",
				release.version
			);

			published = names.len();
			releases.push(ReleaseMarker {
				version: release.version.to_owned(),
				source: release.source.map(str::to_owned),
				snapshot: release.snapshot.to_owned(),
			});
		}

		for name in &names {
			for file in ["migration.sql", "schema.json"] {
				let path = directory.join(name).join(file);
				ensure!(
					path.exists(),
					"registry.txt lists `{name}`, but {} is missing",
					path.display()
				);
			}
		}

		for entry in fs::read_dir(directory)?.flatten() {
			let name = entry.file_name().to_string_lossy().to_string();
			ensure!(
				!entry.path().is_dir() || names.contains(&name),
				"migration `{name}` isn't listed in registry.txt; if two branches each added a \
				 migration, delete yours, update the branch, and create it again"
			);
		}

		let latest = match names.last() {
			Some(name) => {
				let path = directory.join(name).join("schema.json");
				let json = fs::read_to_string(&path)
					.with_context(|| format!("couldn't read {}", path.display()))?;
				Schema::from_json(&json)
					.with_context(|| format!("{} isn't a valid snapshot", path.display()))?
			}
			None => Schema::empty(dialect),
		};

		ensure!(
			latest.dialect == dialect,
			"the migrations are for {}, but leptoskit.toml's db.dialect is {dialect}",
			latest.dialect
		);
		Ok(Self {
			names,
			latest,
			releases,
			published,
		})
	}

	pub(crate) fn latest_name(&self) -> Option<&str> {
		self.names.last().map(String::as_str)
	}
}

/// How `create` behaves.
pub(crate) struct CreateOptions {
	pub allow_destructive: bool,
	pub empty: bool,
	/// Packs whose removal the developer named explicitly.
	pub drop_packs: Vec<String>,
}

/// The packs `previous` records that `target` no longer owns, excluding
/// leptoskit's builtin packs, whose tables follow the app's own flags.
///
/// A format 1 snapshot carries no membership evidence, so nothing can be
/// called missing from it; that limitation is documented.
pub(crate) fn removed_packs(previous: &Schema, target: &Schema) -> Result<Vec<String>> {
	let Some(previous) = previous.pack_membership() else {
		return Ok(Vec::new());
	};

	let Some(target) = target.pack_membership() else {
		bail!(
			"the exported tables carry no schema-pack membership (format 1); upgrade the \
			 leptoskit version that exports them"
		);
	};

	let present: HashSet<&str> = target.iter().map(|pack| pack.name.as_str()).collect();

	Ok(previous
		.iter()
		.filter(|pack| !pack.builtin && !present.contains(pack.name.as_str()))
		.map(|pack| pack.name.clone())
		.collect())
}

/// Refuses a pack that disappeared unless the developer named it in
/// `--drop-pack`. Generic `--allow-destructive` consent doesn't cover this:
/// removing a pack by accident is far likelier than meaning to delete every
/// table a library owns.
pub(crate) fn check_packs(previous: &Schema, target: &Schema, drop_packs: &[String]) -> Result<()> {
	check_pack_baselines(&[previous], target, drop_packs, true)
}

/// Validate removal intent against every schema an operation may replace.
/// Before reading push state, only missing-pack refusal is final; unknown
/// flags are checked once the complete set of baselines is available.
pub(crate) fn check_pack_baselines(
	previous: &[&Schema],
	target: &Schema,
	drop_packs: &[String],
	complete: bool,
) -> Result<()> {
	let mut missing = BTreeSet::new();

	for baseline in previous {
		missing.extend(removed_packs(baseline, target)?);
	}

	let requested: HashSet<&str> = drop_packs.iter().map(String::as_str).collect();

	ensure!(
		requested.len() == drop_packs.len(),
		"--drop-pack names each pack once"
	);

	if complete
		&& previous
			.iter()
			.all(|schema| schema.pack_membership().is_none())
		&& !drop_packs.is_empty()
	{
		bail!(
			"the recorded schema predates schema-pack membership, so there is no pack to drop; \
			 remove --drop-pack"
		);
	}

	let unexpected: Vec<&str> = requested
		.iter()
		.copied()
		.filter(|name| !missing.iter().any(|pack| pack == name))
		.collect();

	if complete && !unexpected.is_empty() {
		bail!(
			"--drop-pack names {} but the recorded schema doesn't lose it: a pack can only be \
			 dropped when it is absent from the exported tables",
			unexpected
				.iter()
				.map(|name| format!("`{name}`"))
				.collect::<Vec<_>>()
				.join(", ")
		);
	}

	let unexplained: Vec<&String> = missing
		.iter()
		.filter(|name| !requested.contains(name.as_str()))
		.collect();

	if unexplained.is_empty() {
		return Ok(());
	}

	let names = unexplained
		.iter()
		.map(|name| format!("`{name}`"))
		.collect::<Vec<_>>()
		.join(", ");
	let drops = unexplained
		.iter()
		.map(|name| format!("--drop-pack {name}"))
		.collect::<Vec<_>>()
		.join(" ");

	bail!(
		"the tables no longer include the schema pack(s) {names}. Removing a pack from the schema \
		 removes its tables, so this isn't an ordinary change: an omitted `packs!` entry, a lost \
		 dependency, or a missing feature looks the same as a deliberate removal. \
		 `--allow-destructive` doesn't cover it. If you meant to remove {} from the app, pass \
		 {drops}; otherwise add the pack back with `leptoskit::db::packs!(…)`",
		if unexplained.len() == 1 { "it" } else { "them" },
	)
}

/// What `create` wrote.
#[derive(Debug)]
pub(crate) struct Created {
	pub name: String,
	pub directory: PathBuf,
	pub steps: Vec<String>,
	pub notes: Vec<String>,
}

impl Created {
	fn summary(&self) -> String {
		let mut summary = String::new();

		for step in &self.steps {
			let _ = writeln!(summary, "  {step}");
		}

		for note in &self.notes {
			let _ = writeln!(summary, "  note: {note}");
		}

		let _ = writeln!(
			summary,
			"wrote migration {} in {}",
			self.name,
			self.directory.display()
		);
		let _ = writeln!(
			summary,
			"review migration.sql and down.sql, then commit them with schema.json and registry.txt"
		);
		summary
	}
}

/// Writes a migration from the latest snapshot to `target`.
pub(crate) fn create(
	project: &Project,
	name: &str,
	target: &Schema,
	options: &CreateOptions,
	renames: &mut dyn RenameResolver,
) -> Result<Created> {
	create_from(project, name, target, options, renames, None)
}

fn create_from(
	project: &Project,
	name: &str,
	target: &Schema,
	options: &CreateOptions,
	renames: &mut dyn RenameResolver,
	pushed: Option<&Schema>,
) -> Result<Created> {
	let directory = project.migrations_dir();
	let history = History::read(&directory, project.dialect())?;

	// In amend mode the drafts since the latest release are one migration
	// that keeps up with the tables, except data migrations, which stack.
	if project.config.draft == DbDraft::Amend
		&& !options.empty
		&& history.names.len() > history.published
	{
		return amend_latest(project, &history, target, options, renames, pushed);
	}

	let slug = slug(name)?;
	let baselines: Vec<&Schema> = std::iter::once(&history.latest).chain(pushed).collect();
	check_pack_baselines(&baselines, target, &options.drop_packs, true)?;
	let plan = leptoskit_db_schema::diff(&history.latest, target, renames)?;

	if options.empty {
		ensure!(
			plan.is_empty(),
			"the tables changed since the latest migration; create a migration for that first, \
			 then the data migration"
		);
	} else {
		ensure!(
			!plan.is_empty(),
			"the tables haven't changed since {}; pass --empty to write a data migration",
			history.latest_name().unwrap_or("the start")
		);
	}

	refuse_destructive(&plan, options.allow_destructive)?;

	let migration = leptoskit_db_schema::render(&plan, &history.latest, target);
	let sql = if options.empty {
		format!(
			"{}\n-- A data migration: write the statements that change rows here.\n",
			migration.sql()
		)
	} else {
		migration.sql()
	};

	let name = format!("{}_{slug}", chrono::Utc::now().format("%Y%m%dT%H%M%S"));
	let migration_dir = directory.join(&name);
	ensure!(
		!migration_dir.exists(),
		"{} already exists",
		migration_dir.display()
	);
	let down = reverse_sql(&history.latest, target, &plan, options.empty)?;
	fs::create_dir_all(&migration_dir)
		.with_context(|| format!("couldn't create {}", migration_dir.display()))?;
	fs::write(migration_dir.join("migration.sql"), sql)?;
	fs::write(migration_dir.join("down.sql"), down)?;
	fs::write(
		migration_dir.join("schema.json"),
		target.without_rename_hints().to_json(),
	)?;

	let registry = directory.join("registry.txt");
	let mut text = fs::read_to_string(&registry).unwrap_or_else(|_| {
		"# The migrations, in the order they apply. `leptoskit db migration create` adds to it.\n"
			.to_owned()
	});

	if !text.ends_with('\n') {
		text.push('\n');
	}

	text.push_str(&name);
	text.push('\n');
	fs::write(&registry, text)?;

	Ok(Created {
		directory: migration_dir,
		steps: plan.steps.iter().map(ToString::to_string).collect(),
		notes: migration.notes,
		name,
	})
}

/// Re-diffs snapshots with inverse rename hints rather than reversing SQL
/// statements, so dependencies, triggers, and SQLite rebuilds stay valid.
fn reverse_sql(
	previous: &Schema,
	target: &Schema,
	forward: &leptoskit_db_schema::Plan,
	data: bool,
) -> Result<String> {
	use leptoskit_db_schema::Step;
	let current = target.without_rename_hints();
	let mut desired = previous.without_rename_hints();
	for step in &forward.steps {
		if let Step::RenameTable { from, to } = step
			&& let Some(table) = desired.tables.iter_mut().find(|table| &table.name == from)
		{
			table.renamed_from = Some(to.clone());
		}
	}
	for step in &forward.steps {
		if let Step::RenameColumn { table, from, to } = step
			&& let Some(table) = desired
				.tables
				.iter_mut()
				.find(|old| &old.name == table || old.renamed_from.as_ref() == Some(table))
			&& let Some(column) = table.columns.iter_mut().find(|column| &column.name == from)
		{
			column.renamed_from = Some(to.clone());
		}
	}
	let reverse = match leptoskit_db_schema::diff(&current, &desired, &mut NoRenames) {
		Ok(reverse) => reverse,
		Err(error) if data || forward.destructive_steps().next().is_some() => {
			return Ok(format!(
				"-- leptoskit: irreversible\n-- Write and review reverse SQL and data recovery \
				 before removing this marker.\n-- {}\n",
				error.to_string().replace('\n', "\n-- ")
			));
		}
		Err(error) => return Err(error.into()),
	};
	let sql = leptoskit_db_schema::render(&reverse, &current, &desired).sql();
	if data || forward.destructive_steps().next().is_some() {
		Ok(format!(
			"-- leptoskit: irreversible\n-- Implement and review data recovery before removing \
			 the marker above.\n-- Schema reversal alone cannot recover deleted or transformed \
			 data.\n{sql}"
		))
	} else {
		Ok(sql)
	}
}

/// Writes a migration from the latest snapshot to `target`, as [`create`]
/// does, and when the development database at `dev_url` has pushes, makes
/// the migration theirs: catches the database up with `target`, then
/// records the migration as applied there and forgets the pushes. When that
/// fails, the migration is removed again.
pub(crate) fn create_squashing(
	project: &Project,
	name: &str,
	target: &Schema,
	options: &CreateOptions,
	renames: &mut dyn RenameResolver,
	dev_url: Option<&str>,
) -> Result<Created> {
	let squash = match dev_url {
		Some(url) => {
			push::prepare_squash(
				project,
				url,
				target,
				options.allow_destructive,
				&options.drop_packs,
			)?
		}
		None => None,
	};

	let created = create_from(
		project,
		name,
		target,
		options,
		renames,
		squash.as_ref().map(|squash| &squash.previous),
	)?;

	if let Some(squash) = squash {
		let sql = fs::read_to_string(created.directory.join("migration.sql"))?;
		let migration = leptoskit_db::Migration::new(&created.name, sql);

		if let Err(error) = squash.adopt(&migration, target) {
			watch::discard(project, &created)?;
			return Err(error.context(
				"the development database couldn't take the migration over from its pushes, so \
				 the migration was removed again",
			));
		}

		println!(
			"the development database's pushes are now migration {}",
			created.name
		);
	}

	Ok(created)
}

/// Fails when `target` differs from the latest snapshot, listing what changed.
pub(crate) fn check(history: &History, target: &Schema) -> Result<()> {
	check_packs(&history.latest, target, &[])?;
	check_database(history, target)
}

fn check_database(history: &History, target: &Schema) -> Result<()> {
	let target = target.without_rename_hints();

	if target.same_database(&history.latest) {
		return Ok(());
	}

	let changes = match leptoskit_db_schema::diff(&history.latest, &target, &mut NoRenames) {
		Ok(plan) => {
			plan.steps
				.iter()
				.map(|step| format!("  {step}"))
				.collect::<Vec<_>>()
				.join("\n")
		}
		Err(error) => format!("  {error}"),
	};

	bail!(
		"the tables changed since {}:\n{changes}\nrun `{} db migration create <name>` and commit \
		 the result",
		history.latest_name().unwrap_or("the start"),
		invoked_name()
	)
}

/// A warning for each vector column clients can see: embeddings are large,
/// and they can give away the text they were made from.
pub(crate) fn public_vectors(target: &Schema) -> Vec<String> {
	target
		.tables
		.iter()
		.flat_map(|table| {
			table
				.vectors
				.iter()
				.filter(|vector| vector.public)
				.map(move |vector| {
					format!(
						"`{}.{}` is a vector clients can see: vectors are large, and an embedding \
						 can give away the text it was made from; drop `public` unless clients \
						 need it",
						table.name, vector.column
					)
				})
		})
		.collect()
}

/// How many rows each table with a searched vector column has.
async fn count_searched_rows(url: &str, target: &Schema) -> Result<Vec<(TableName, u64)>> {
	let db = Db::connect(url).await?;
	let mut counts = Vec::new();

	for table in &target.tables {
		if !table.vectors.iter().any(|vector| vector.distance.is_some()) {
			continue;
		}

		let rows = db
			.fetch(Statement {
				sql: format!(
					"SELECT count(*) FROM {}",
					leptoskit_db_schema::quote_identifier(&table.name.flat())
				),
				args: Vec::new(),
			})
			.await?;

		let count = match rows.first().and_then(|row| row.first()) {
			Some(leptoskit_db::Value::Int(count)) => u64::try_from(*count).unwrap_or_default(),
			_ => 0,
		};

		counts.push((table.name.clone(), count));
	}

	Ok(counts)
}

/// A warning for each table, of `counts`, with more rows than SQLite's
/// exact vector searches suit.
pub(crate) fn exact_searches(counts: &[(TableName, u64)], most: u64) -> Vec<String> {
	counts
		.iter()
		.filter(|(_, count)| *count > most)
		.map(|(table, count)| {
			format!(
				"`{table}` has {count} rows, and SQLite measures every one in each vector search; \
				 past {most} (`db.vector.exact_search_rows`) searches slow down, and Postgres's \
				 HNSW indexes suit it better"
			)
		})
		.collect()
}

/// The table sign-in keeps accounts in.
fn accounts_table() -> TableName {
	TableName::namespaced("auth", "accounts")
}

/// With sign-in's tables, fails when a table that holds an account's data
/// can't take part in merging two accounts as declared, such as a unique
/// reference without `on_account_merge`. Otherwise says how many tables
/// follow the account, or `None` without sign-in.
pub(crate) fn check_account_merges(target: &Schema) -> Result<Option<String>> {
	let accounts = accounts_table();

	if target.table(&accounts).is_none() {
		return Ok(None);
	}

	let data = leptoskit_db_schema::account_data(target, &accounts);

	if !data.is_covered() {
		let problems: Vec<String> = data
			.problems
			.iter()
			.map(|problem| format!("  {problem}"))
			.collect();
		bail!(
			"merging accounts can't bring their data along until every table says how:
{}",
			problems.join("\n")
		);
	}

	Ok(Some(format!(
		"merging accounts brings their data along: {} table(s) keyed by the account, {} \
		 reference(s) to it",
		data.keyed.len(),
		data.references.len()
	)))
}

/// Checks the connected database's live pack against `target`: every capture
/// trigger, function, and sequence the schema expects is there and unchanged,
/// and no capture trigger copies a wide column that should be skipped.
#[expect(
	clippy::too_many_lines,
	reason = "the check reads each dialect's catalog in one pass, then compares one expected set"
)]
pub(crate) fn check_live(project: &Project, target: &Schema, url: Option<String>) -> Result<()> {
	if target.triggers.is_empty() && target.functions.is_empty() && target.sequences.is_empty() {
		println!("no live tables to check");

		return Ok(());
	}

	let (url, _server) = project.database(url)?;
	let dialect = project.dialect();
	let rows = live_catalog(&url, dialect)?;

	let mut found_triggers = BTreeSet::new();
	let mut found_functions = BTreeSet::new();
	let mut found_sequences = BTreeSet::new();

	match dialect {
		Dialect::Postgres => {
			for row in &rows[0] {
				let [
					leptoskit_db::Value::Text(namespace),
					leptoskit_db::Value::Text(table),
					leptoskit_db::Value::Text(trigger),
				] = row.as_slice()
				else {
					continue;
				};
				let table = if namespace == "public" {
					table.clone()
				} else {
					format!("{namespace}.{table}")
				};

				found_triggers.insert((table, trigger.clone()));
			}

			for row in &rows[1] {
				let [
					leptoskit_db::Value::Text(_),
					leptoskit_db::Value::Text(function),
					leptoskit_db::Value::Text(body),
				] = row.as_slice()
				else {
					continue;
				};
				found_functions.insert(function.clone());

				if let Some(expected) = target
					.functions
					.iter()
					.find(|expected| expected.name.name == *function)
				{
					ensure!(
						body.contains(expected.body.trim()),
						"the database's `{}` has changed: run `{} db migrate`",
						expected.name,
						invoked_name()
					);
				}
			}

			for row in &rows[2] {
				let [
					leptoskit_db::Value::Text(_),
					leptoskit_db::Value::Text(sequence),
				] = row.as_slice()
				else {
					continue;
				};
				found_sequences.insert(sequence.clone());
			}
		}
		Dialect::Sqlite => {
			for row in &rows[0] {
				let [
					leptoskit_db::Value::Text(table),
					leptoskit_db::Value::Text(trigger),
				] = row.as_slice()
				else {
					continue;
				};
				found_triggers.insert((table.clone(), trigger.clone()));
			}
		}
	}

	let expected_triggers: BTreeSet<(String, String)> = target
		.triggers
		.iter()
		.map(|trigger| {
			(
				match dialect {
					Dialect::Postgres => trigger.table.to_string(),
					Dialect::Sqlite => trigger.table.flat(),
				},
				trigger.name.clone(),
			)
		})
		.collect();
	let missing: Vec<String> = expected_triggers
		.difference(&found_triggers)
		.map(|(table, trigger)| format!("{trigger} on {table}"))
		.collect();
	let extra: Vec<String> = found_triggers
		.difference(&expected_triggers)
		.map(|(table, trigger)| format!("{trigger} on {table}"))
		.collect();

	ensure!(
		missing.is_empty(),
		"the database is missing live triggers:\n  {}\nrun `{} db migrate`",
		missing.join("\n  "),
		invoked_name()
	);
	ensure!(
		extra.is_empty(),
		"the database has live triggers the schema doesn't:\n  {}\nrun `{} db migrate` to replace \
		 them",
		extra.join("\n  "),
		invoked_name()
	);

	if dialect == Dialect::Postgres {
		let expected_functions: BTreeSet<String> = target
			.functions
			.iter()
			.map(|function| function.name.name.clone())
			.collect();
		let missing: Vec<String> = expected_functions
			.difference(&found_functions)
			.cloned()
			.collect();
		ensure!(
			missing.is_empty(),
			"the database is missing live functions:\n  {}\nrun `{} db migrate`",
			missing.join("\n  "),
			invoked_name()
		);

		let expected_sequences: BTreeSet<String> = target
			.sequences
			.iter()
			.map(|sequence| sequence.name.name.clone())
			.collect();
		let missing: Vec<String> = expected_sequences
			.difference(&found_sequences)
			.cloned()
			.collect();
		ensure!(
			missing.is_empty(),
			"the database is missing live sequences:\n  {}\nrun `{} db migrate`",
			missing.join("\n  "),
			invoked_name()
		);
	}

	for warning in wide_column_warnings(target) {
		println!("warning: {warning}");
	}

	println!(
		"the database has every live trigger{}",
		if dialect == Dialect::Postgres {
			", function, and sequence"
		} else {
			""
		}
	);
	check_replication(&url, target)
}

/// The live catalog's rows: triggers always, functions and sequences on
/// Postgres.
fn live_catalog(url: &str, dialect: Dialect) -> Result<Vec<Vec<Vec<leptoskit_db::Value>>>> {
	runtime()?.block_on(async {
		let db = Db::connect(url).await?;
		let statements: Vec<Statement> = match dialect {
			Dialect::Postgres => {
				vec![
					Statement {
						sql: "SELECT n.nspname, c.relname, t.tgname FROM pg_trigger t JOIN \
						      pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = \
						      c.relnamespace WHERE NOT t.tgisinternal"
							.into(),
						args: Vec::new(),
					},
					Statement {
						sql: "SELECT n.nspname, p.proname, pg_get_functiondef(p.oid) FROM pg_proc \
						      p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = \
						      'live'"
							.into(),
						args: Vec::new(),
					},
					Statement {
						sql: "SELECT n.nspname, c.relname FROM pg_class c JOIN pg_namespace n ON \
						      n.oid = c.relnamespace WHERE c.relkind = 'S' AND n.nspname = 'live'"
							.into(),
						args: Vec::new(),
					},
				]
			}
			Dialect::Sqlite => {
				vec![Statement {
					sql: "SELECT tbl_name, name FROM sqlite_schema WHERE type = 'trigger'".into(),
					args: Vec::new(),
				}]
			}
		};

		let mut rows = Vec::new();

		for statement in statements {
			rows.push(db.fetch(statement).await?);
		}

		anyhow::Ok(rows)
	})
}

/// Columns copied into the change log that should be skipped.
fn wide_column_warnings(target: &Schema) -> HashSet<String> {
	let mut warnings = HashSet::new();

	for trigger in &target.triggers {
		if !leptoskit_db_schema::live::is_capture(trigger) {
			continue;
		}

		let Some(table) = target.table(&trigger.table) else {
			continue;
		};

		for column in &trigger.columns {
			let Some(column) = table.column(column) else {
				continue;
			};

			if matches!(
				column.ty,
				leptoskit_db_schema::ColumnType::Bytes
					| leptoskit_db_schema::ColumnType::Array { .. }
			) {
				warnings.insert(format!(
					"{}.{} is copied into the change log; mark it #[db(live = skip)]",
					table.name, column.name
				));
			}
		}
	}

	warnings
}

/// With replication capture: the database publishes the live tables, runs
/// with `wal_level = logical`, and how far capture lags behind it.
fn check_replication(url: &str, target: &Schema) -> Result<()> {
	if target
		.publication(leptoskit_db_schema::live::PUBLICATION)
		.is_none()
	{
		return Ok(());
	}

	let (published, logical, stats) = runtime()?.block_on(async {
		let db = Db::connect(url).await?;
		let rows = db
			.fetch(Statement {
				sql: "SELECT EXISTS (SELECT FROM pg_publication WHERE pubname = $1), \
				      current_setting('wal_level') = 'logical'"
					.into(),
				args: vec![leptoskit_db::Value::Text(
					leptoskit_db_schema::live::PUBLICATION.into(),
				)],
			})
			.await?;
		let flag = |index: usize| {
			matches!(
				rows.first().and_then(|row| row.get(index)),
				Some(leptoskit_db::Value::Bool(true))
			)
		};
		let stats = db.system("lk db check").live_stats().await?;
		anyhow::Ok((flag(0), flag(1), stats))
	})?;

	ensure!(
		published,
		"the database doesn't publish the live tables for replication capture: run `{} db migrate`",
		invoked_name()
	);
	ensure!(
		logical,
		"replication capture needs Postgres to run with `wal_level = logical`; set it and restart \
		 the database"
	);

	match stats.live_capture_lag {
		None => {
			println!(
				"no server has captured changes yet: start capture on the servers with \
				 `Db::capture_changes`"
			);
		}
		Some(lag) if stats.slot_active => {
			println!("replication capture is running, {lag} bytes behind the write-ahead log");
		}
		Some(lag) => {
			println!(
				"warning: no server is capturing changes, and Postgres is keeping {lag} bytes of \
				 write-ahead log for when one does"
			);
		}
	}

	Ok(())
}

/// Stops a plan that deletes data, unless `allow_destructive` says it was
/// meant to.
fn refuse_destructive(plan: &leptoskit_db_schema::Plan, allow_destructive: bool) -> Result<()> {
	let destructive: Vec<String> = plan
		.destructive_steps()
		.map(|step| {
			match step.risk() {
				Risk::LosesData(reason) => format!("  {step}: {reason}"),
				_ => format!("  {step}"),
			}
		})
		.collect();

	if !destructive.is_empty() && !allow_destructive {
		bail!(
			"this migration deletes data:\n{}\npass --allow-destructive if that's intended; if a \
			 column or table was renamed, mark it with #[db(renamed_from = \"...\")]",
			destructive.join("\n")
		);
	}

	Ok(())
}

/// Rewrites the newest unreleased migration from the snapshot before it, so
/// drafting between releases leaves one migration rather than a stack.
fn amend_latest(
	project: &Project,
	history: &History,
	target: &Schema,
	options: &CreateOptions,
	renames: &mut dyn RenameResolver,
	pushed: Option<&Schema>,
) -> Result<Created> {
	let name = history
		.names
		.last()
		.expect("the caller checked for drafts")
		.clone();
	let previous = if history.names.len() == 1 {
		Schema::empty(project.dialect())
	} else {
		let path = project
			.migrations_dir()
			.join(&history.names[history.names.len() - 2])
			.join("schema.json");
		let json = fs::read_to_string(&path)
			.with_context(|| format!("couldn't read {}", path.display()))?;
		Schema::from_json(&json)
			.with_context(|| format!("{} isn't a valid snapshot", path.display()))?
	};

	let baselines: Vec<&Schema> = [&history.latest, &previous]
		.into_iter()
		.chain(pushed)
		.collect();
	check_pack_baselines(&baselines, target, &options.drop_packs, true)?;
	let plan = leptoskit_db_schema::diff(&previous, target, renames)?;
	ensure!(
		!plan.is_empty(),
		"the tables haven't changed since {name}; pass --empty to write a data migration"
	);
	refuse_destructive(&plan, options.allow_destructive)?;
	let migration = leptoskit_db_schema::render(&plan, &previous, target);
	let down = reverse_sql(&previous, target, &plan, false)?;
	let directory = project.migrations_dir().join(&name);
	fs::write(directory.join("migration.sql"), migration.sql())?;
	fs::write(directory.join("down.sql"), down)?;
	fs::write(
		directory.join("schema.json"),
		target.without_rename_hints().to_json(),
	)?;

	Ok(Created {
		name,
		directory,
		steps: plan.steps.iter().map(ToString::to_string).collect(),
		notes: vec![
			"a development database that applied this draft's earlier shape needs `db reset \
			 --yes` to come back in line"
				.to_owned(),
		],
	})
}

/// Runs `migration consolidate` with the tables the app exports now.
fn run_consolidate(
	project: &Project,
	keep: &[String],
	no_renames: bool,
	drop_packs: &[String],
) -> Result<()> {
	let target = project.export()?;
	let renames: &mut dyn RenameResolver = if no_renames {
		&mut NoRenames
	} else if std::io::stdin().is_terminal() {
		&mut Ask
	} else {
		&mut RequireHints
	};
	consolidate(project, &target, keep, drop_packs, renames)
}

/// Folds the migrations after the latest release marker into one, from a
/// fresh diff of the release's baseline to the current tables.
fn consolidate(
	project: &Project,
	target: &Schema,
	keep: &[String],
	drop_packs: &[String],
	renames: &mut dyn RenameResolver,
) -> Result<()> {
	let directory = project.migrations_dir();
	let history = History::read(&directory, project.dialect())?;

	// The tables must match the latest migration, so the consolidated
	// migration covers exactly what the folded ones did.
	check_database(&history, target)?;

	let unreleased = &history.names[history.published..];

	if unreleased.len() < 2 {
		check_packs(&history.latest, target, drop_packs)?;
		println!(
			"{} migration(s) since {} — nothing to consolidate",
			unreleased.len(),
			history
				.releases
				.last()
				.map_or("the start", |release| release.version.as_str()),
		);
		return Ok(());
	}

	// `--keep` may only hold the newest migrations: the kept ones run after
	// the consolidated one, so anything but a suffix would reorder history.
	let kept = if keep.is_empty() {
		0
	} else {
		ensure!(
			keep.len() <= unreleased.len(),
			"--keep lists more migrations than the unreleased draft contains"
		);
		let suffix = &unreleased[unreleased.len() - keep.len()..];
		ensure!(
			suffix.iter().zip(keep).all(|(name, kept)| name == kept),
			"--keep only applies to the newest migrations since the release; the draft is:\n{}",
			unreleased
				.iter()
				.map(|name| format!("  {name}"))
				.collect::<Vec<_>>()
				.join("\n")
		);
		keep.len()
	};
	let folded = &unreleased[..unreleased.len() - kept];
	if folded.is_empty() {
		check_packs(&history.latest, target, drop_packs)?;
		println!("all unreleased migrations are kept — nothing to consolidate");
		return Ok(());
	}

	// The baseline the release ended at: the snapshot of the last frozen
	// migration, or the empty schema before any.
	let baseline = match history.published {
		0 => Schema::empty(project.dialect()),
		published => {
			let path = directory
				.join(&history.names[published - 1])
				.join("schema.json");
			let json = fs::read_to_string(&path)
				.with_context(|| format!("couldn't read {}", path.display()))?;
			Schema::from_json(&json)
				.with_context(|| format!("{} isn't a valid snapshot", path.display()))?
		}
	};

	// The release baseline may still carry a pack that the drafts removed;
	// consolidating writes that removal into fresh SQL, so it needs the same
	// explicit intent.
	check_pack_baselines(&[&history.latest, &baseline], target, drop_packs, true)?;

	// A folded migration whose SQL no longer matches what the diff of its
	// snapshots generates carries hand-written statements; folding it would
	// silently discard them.
	let mut previous = baseline.clone();
	for name in folded {
		let path = directory.join(name).join("schema.json");
		let json = fs::read_to_string(&path)
			.with_context(|| format!("couldn't read {}", path.display()))?;
		let snapshot = Schema::from_json(&json)
			.with_context(|| format!("{} isn't a valid snapshot", path.display()))?;
		let plan = leptoskit_db_schema::diff(&previous, &snapshot, &mut NoRenames)?;
		let generated = leptoskit_db_schema::render(&plan, &previous, &snapshot).sql();
		let on_disk = fs::read_to_string(directory.join(name).join("migration.sql"))?;
		ensure!(
			on_disk == generated,
			"migration {name} no longer matches what its snapshots generate, so it has 			 \
			 statements written by hand, or came from an older leptoskit. Consolidating it 			 \
			 would discard them: move them into a data migration, or pass `--keep {name}` to 			 \
			 leave it and the migrations after it in place"
		);
		previous = snapshot;
	}

	// Kept migrations run afterwards. Folding through the final target would
	// apply their schema changes twice and record the wrong intermediate state.
	let folded_target = &previous;
	let plan = leptoskit_db_schema::diff(&baseline, folded_target, renames)?;
	let migration = leptoskit_db_schema::render(&plan, &baseline, folded_target);
	let down = reverse_sql(&baseline, folded_target, &plan, false)?;
	let name = format!(
		"{}_consolidated",
		chrono::Utc::now().format("%Y%m%dT%H%M%S")
	);
	let migration_dir = directory.join(&name);
	ensure!(
		!migration_dir.exists(),
		"{} already exists",
		migration_dir.display()
	);
	fs::create_dir_all(&migration_dir)
		.with_context(|| format!("couldn't create {}", migration_dir.display()))?;
	fs::write(migration_dir.join("migration.sql"), migration.sql())?;
	fs::write(migration_dir.join("down.sql"), down)?;
	fs::write(
		migration_dir.join("schema.json"),
		folded_target.without_rename_hints().to_json(),
	)?;
	fs::write(
		migration_dir.join("replaces.json"),
		serde_json::to_string_pretty(folded)?,
	)?;
	for name in folded {
		fs::remove_dir_all(directory.join(name))
			.with_context(|| format!("couldn't remove {}", directory.join(name).display()))?;
	}

	// The registry keeps its comments and markers; the folded lines become
	// the consolidated one, and any kept migrations follow it.
	let registry = directory.join("registry.txt");
	let original = fs::read_to_string(&registry)?;
	let mut text = String::new();
	let mut placed = false;
	for line in original.lines() {
		if folded.iter().any(|folded| folded == line.trim()) {
			if !placed {
				text.push_str(&name);
				text.push('\n');
				placed = true;
			}
			continue;
		}
		text.push_str(line);
		text.push('\n');
	}
	if !placed {
		text.push_str(&name);
		text.push('\n');
	}
	fs::write(&registry, text)?;

	println!(
		"folded {} migration(s) into {}",
		folded.len(),
		migration_dir.display()
	);
	println!(
		"a database that already applied them records it without running it; a fresh one runs it \
		 once"
	);
	Ok(())
}

/// Writes a release marker after the latest migration, pinning the schema the
/// release ends at. Everything above the marker has shipped; the next release
/// consolidates everything below it into one migration.
fn record_release(project: &Project, version: &str, id: Option<&str>) -> Result<()> {
	ensure!(
		!version.is_empty()
			&& !version
				.chars()
				.any(|character| character.is_whitespace() || "()<#".contains(character)),
		"a version can't be empty or contain whitespace, parentheses, or a `#`"
	);
	if let Some(id) = id {
		ensure!(
			!id.is_empty() && !id.chars().any(|character| "()<#\n".contains(character)),
			"a release id can't be empty or contain parentheses, a `#`, or a newline"
		);
	}

	let directory = project.migrations_dir();
	let history = History::read(&directory, project.dialect())?;
	ensure!(
		!history.names.is_empty(),
		"there are no migrations to release; create one first"
	);
	ensure!(
		!history
			.releases
			.iter()
			.any(|release| release.version == version),
		"release {version} is already recorded"
	);

	let latest = history.latest_name().expect("checked for names above");
	let schema = fs::read(directory.join(latest).join("schema.json"))?;
	let snapshot =
		Sha256::digest(&schema)
			.iter()
			.fold(String::with_capacity(64), |mut hex, byte| {
				let _ = write!(hex, "{byte:02x}");
				hex
			});
	let marker = match id {
		Some(id) => format!("# release {version} ({id}) snapshot {snapshot}"),
		None => format!("# release {version} snapshot {snapshot}"),
	};

	let registry = directory.join("registry.txt");
	let mut text = fs::read_to_string(&registry).unwrap_or_else(|_| {
		"# The migrations, in the order they apply. `leptoskit db migration create` adds to it.\n"
			.to_owned()
	});

	if !text.ends_with('\n') {
		text.push('\n');
	}

	text.push_str(&marker);
	text.push('\n');
	fs::write(&registry, &text)?;

	println!("{marker}");
	println!(
		"{} migration(s) are frozen through {version}; `db migration consolidate` at the next \
		 release folds everything after them into one migration",
		history.names.len()
	);
	Ok(())
}

fn status(project: &Project, url: Option<String>) -> Result<()> {
	let history = History::read(&project.migrations_dir(), project.dialect())?;
	println!(
		"{} migration(s); the latest is {}",
		history.names.len(),
		history.latest_name().unwrap_or("none")
	);

	if let Some(release) = history.releases.last() {
		let since = history.names.len() - history.published;
		let source = release
			.source
			.as_ref()
			.map(|source| format!(" ({source})"))
			.unwrap_or_default();
		println!(
			"release {}{source} froze {} migration(s); {} came after it",
			release.version, history.published, since
		);
		println!("it pins snapshot {}", release.snapshot);
	} else {
		println!("no release has been recorded; `db release record <version>` freezes them");
	}

	match check(&history, &project.export()?) {
		Ok(()) => println!("the tables match the latest migration"),
		Err(error) => println!("{error}"),
	}

	let (url, _server) = match project.database(url) {
		Ok(database) => database,
		Err(error) => {
			println!("{error:#}, to see which migrations the database needs");
			return Ok(());
		}
	};

	let (applied, pushed) = runtime()?.block_on(async {
		let db = Db::connect(&url).await?;
		anyhow::Ok((db.applied_migrations().await?, db.pushed_schema().await?))
	})?;

	if pushed.is_some() {
		println!(
			"the database has changes pushed in development; `{} db migration create <name>` \
			 turns them into a migration",
			invoked_name()
		);
	}

	let pending: Vec<&String> = history
		.names
		.iter()
		.filter(|name| !applied.contains(name))
		.collect();

	if pending.is_empty() {
		println!("the database has every migration");
	} else {
		println!("the database needs:");

		for name in pending {
			println!("  {name}");
		}
	}

	Ok(())
}

/// Deletes every row in a local database and applies the migrations again.
fn reset(project: &Project, url: Option<String>, yes: bool) -> Result<()> {
	ensure!(
		yes,
		"this deletes every row in the database; pass --yes to confirm"
	);
	// A service's database on the development Postgres is created again,
	// under the service's own role.
	let (url, _server) =
		project.database_with(url, crate::services::database::Existing::Recreate)?;
	let history = History::read(&project.migrations_dir(), project.dialect())?;
	let migrations = leptoskit_db::load_migrations(&project.migrations_dir())?;

	if let Some(path) = url
		.strip_prefix("sqlite://")
		.or_else(|| url.strip_prefix("sqlite:"))
	{
		if path != ":memory:" {
			for suffix in ["", "-wal", "-shm"] {
				let file = PathBuf::from(format!("{path}{suffix}"));

				if file.exists() {
					fs::remove_file(&file)
						.with_context(|| format!("couldn't delete {}", file.display()))?;
				}
			}
		}
	} else {
		let parsed = url::Url::parse(&url).context("the database URL isn't a URL")?;
		let local = matches!(
			parsed.host_str(),
			Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
		);
		ensure!(
			local,
			"reset only deletes local databases; {} isn't local",
			parsed.host_str().unwrap_or("the host")
		);
		let mut script =
			String::from("DROP SCHEMA IF EXISTS public CASCADE;\nCREATE SCHEMA public;\n");

		for namespace in namespaces(&history.latest) {
			let _ = writeln!(
				script,
				"DROP SCHEMA IF EXISTS {} CASCADE;",
				leptoskit_db_schema::quote_identifier(&namespace)
			);
		}

		runtime()?.block_on(async {
			let db = Db::connect(&url).await?;
			anyhow::Ok(db.run_script(&script).await?)
		})?;
	}

	let applied = runtime()?.block_on(async {
		let db = Db::connect(&url).await?;
		anyhow::Ok(db.migrate(&migrations).await?)
	})?;
	println!(
		"reset the database and applied {} migration(s)",
		applied.len()
	);
	Ok(())
}

fn namespaces(schema: &Schema) -> Vec<String> {
	let mut namespaces: Vec<String> = schema
		.tables
		.iter()
		.filter_map(|table| table.name.namespace.clone())
		.collect();
	namespaces.sort();
	namespaces.dedup();
	namespaces
}

/// A migration name for a directory: lowercase letters, digits, and `_`.
pub(crate) fn slug(name: &str) -> Result<String> {
	let mut slug = String::new();

	for character in name.trim().chars() {
		if character.is_ascii_alphanumeric() {
			slug.push(character.to_ascii_lowercase());
		} else if !slug.ends_with('_') && !slug.is_empty() {
			slug.push('_');
		}
	}

	let slug = slug.trim_end_matches('_').to_owned();
	ensure!(
		!slug.is_empty(),
		"name the migration with letters or digits, such as `add_posts`"
	);
	Ok(slug)
}

pub(crate) fn runtime() -> Result<tokio::runtime::Runtime> {
	tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
		.context("could not start the async runtime")
}

/// Asks on the terminal whether something that appeared was renamed.
struct Ask;

impl Ask {
	fn confirm(question: &str) -> bool {
		print!("{question} [y/N] ");
		let _ = std::io::stdout().flush();
		let mut answer = String::new();
		let _ = std::io::stdin().lock().read_line(&mut answer);
		matches!(answer.trim(), "y" | "Y" | "yes")
	}
}

impl RenameResolver for Ask {
	fn table(
		&mut self,
		added: &Table,
		candidates: &[&Table],
	) -> Result<Option<TableName>, DiffError> {
		for candidate in candidates {
			if Self::confirm(&format!(
				"was table `{}` renamed from `{}`?",
				added.name, candidate.name
			)) {
				return Ok(Some(candidate.name.clone()));
			}
		}

		Ok(None)
	}

	fn column(
		&mut self,
		table: &TableName,
		added: &Column,
		candidates: &[&Column],
	) -> Result<Option<String>, DiffError> {
		for candidate in candidates {
			if Self::confirm(&format!(
				"was column `{table}.{}` renamed from `{}`?",
				added.name, candidate.name
			)) {
				return Ok(Some(candidate.name.clone()));
			}
		}

		Ok(None)
	}
}

#[cfg(test)]
mod tests;
