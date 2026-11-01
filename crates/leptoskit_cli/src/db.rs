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
	/// Fails when the tables changed without a migration, the migrations
	/// directory is inconsistent, or, with sign-in, a table that holds an
	/// account's data doesn't say how its rows move when accounts merge.
	/// With `--live`, also checks the connected database's capture
	/// triggers. For CI.
	Check {
		/// Check the connected database's live pack: every trigger the schema
		/// expects exists, unchanged, and no capture trigger copies a wide
		/// column.
		#[arg(long)]
		live: bool,
		/// The database URL `--live` checks; defaults to the variable
		/// `db.url_env` names.
		#[arg(long, requires = "live")]
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
	},
}

/// Runs a `leptoskit db` subcommand.
pub fn run(command: DbCommand) -> Result<()> {
	let project = Project::load()?;

	match command {
		DbCommand::Migration {
			command:
				MigrationCommand::Create {
					name,
					allow_destructive,
					no_renames,
					empty,
				},
		} => {
			let target = project.export()?;
			let options = CreateOptions {
				allow_destructive,
				empty,
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
		DbCommand::Push {
			url,
			allow_destructive,
		} => {
			let target = project.export()?;
			let (url, _server) = project.database(url)?;
			push_command(&project, &target, &url, allow_destructive)
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

/// `leptoskit db push`: pushes `target` to the database at `url`, and fails
/// when that would delete data without `allow_destructive`.
fn push_command(
	project: &Project,
	target: &Schema,
	url: &str,
	allow_destructive: bool,
) -> Result<()> {
	let pushed = push::push(project, url, target, allow_destructive)?;
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

/// The app's database settings, resolved.
pub(crate) struct Project {
	pub root: PathBuf,
	pub config: DbConfig,
	pub package: String,
	/// The app's name, `app.name`.
	pub name: String,
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
		})
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
		if let Some(url) = url.or_else(|| std::env::var(&self.config.url_env).ok()) {
			return Ok((url, None));
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

	/// The schema the app's tables declare, from the test
	/// `export_schema!()` adds to the db crate.
	pub(crate) fn export(&self) -> Result<Schema> {
		let out =
			tempfile::NamedTempFile::new().context("couldn't create a file for the schema")?;
		let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
		let mut command = Command::new(cargo);
		command
			.current_dir(&self.root)
			.args(["test", "--quiet", "--package", &self.package, "--lib"])
			.env(leptoskit_db::SCHEMA_OUT_ENV, out.path())
			.env(leptoskit_db::DIALECT_ENV, self.config.dialect.as_str())
			.env(leptoskit_db::POLICIES_ENV, self.config.policies.as_str())
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
}

impl History {
	/// Reads `registry.txt` and the latest snapshot, checking that every
	/// listed migration exists and every migration is listed.
	pub(crate) fn read(directory: &Path, dialect: Dialect) -> Result<Self> {
		let registry = directory.join("registry.txt");

		if !registry.exists() {
			return Ok(Self {
				names: Vec::new(),
				latest: Schema::empty(dialect),
			});
		}

		let text = fs::read_to_string(&registry)
			.with_context(|| format!("couldn't read {}", registry.display()))?;
		let names: Vec<String> = leptoskit_db::registry_names(&text)
			.map(ToString::to_string)
			.collect();

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
		Ok(Self { names, latest })
	}

	pub(crate) fn latest_name(&self) -> Option<&str> {
		self.names.last().map(String::as_str)
	}
}

/// How `create` behaves.
pub(crate) struct CreateOptions {
	pub allow_destructive: bool,
	pub empty: bool,
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
			"review migration.sql, then commit it with schema.json and registry.txt"
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
	let directory = project.migrations_dir();
	let history = History::read(&directory, project.dialect())?;
	let slug = slug(name)?;
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

	let destructive: Vec<String> = plan
		.destructive_steps()
		.map(|step| {
			match step.risk() {
				Risk::LosesData(reason) => format!("  {step}: {reason}"),
				_ => format!("  {step}"),
			}
		})
		.collect();

	if !destructive.is_empty() && !options.allow_destructive {
		bail!(
			"this migration deletes data:\n{}\npass --allow-destructive if that's intended; if a \
			 column or table was renamed, mark it with #[db(renamed_from = \"...\")]",
			destructive.join("\n")
		);
	}

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
	fs::create_dir_all(&migration_dir)
		.with_context(|| format!("couldn't create {}", migration_dir.display()))?;
	fs::write(migration_dir.join("migration.sql"), sql)?;
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
		Some(url) => push::prepare_squash(project, url, target, options.allow_destructive)?,
		None => None,
	};
	let created = create(project, name, target, options, renames)?;

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
	let target = target.without_rename_hints();

	if target == history.latest {
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
	let rows = runtime()?.block_on(async {
		let db = Db::connect(&url).await?;
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
	})?;

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

	for warning in warnings {
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
	Ok(())
}

fn status(project: &Project, url: Option<String>) -> Result<()> {
	let history = History::read(&project.migrations_dir(), project.dialect())?;
	println!(
		"{} migration(s); the latest is {}",
		history.names.len(),
		history.latest_name().unwrap_or("none")
	);

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
	let (url, _server) = project.database(url)?;
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
