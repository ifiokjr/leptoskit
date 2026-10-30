//! The Postgres database development uses when `db.url_env` isn't set.
//!
//! A project that runs devenv's `services.postgres` uses it; any other runs
//! an embedded Postgres, which the first run downloads once into
//! `~/.cache/leptoskit/postgresql` and later runs reuse. Its data lives in
//! the project's `.leptoskit/db/postgres`, so it survives restarts.

use std::fs;
use std::io::Write as _;
use std::net::TcpStream;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use leptoskit_config::DbDevServer;
use leptoskit_config::DbDialect;
use postgresql_embedded::Settings;
use postgresql_embedded::VersionReq;
use postgresql_embedded::blocking::PostgreSQL;

use super::Project;

/// The Postgres major version the embedded server runs.
const POSTGRES_VERSION: &str = "=18";

/// Binaries to run instead of downloading: a directory holding `initdb`,
/// such as an installed Postgres's `bin`.
const BINARIES_ENV: &str = "LEPTOSKIT_POSTGRES_BIN";

/// Where the development database comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
	/// devenv's `services.postgres`, running beside `leptoskit dev`.
	Devenv,
	/// The embedded Postgres in `.leptoskit/db/postgres`.
	Embedded,
}

/// A development database: its URL, and the server when this process
/// started it, which stops when this is dropped.
pub(crate) struct DevServer {
	pub url: String,
	pub source: Source,
	server: Option<PostgreSQL>,
}

impl std::fmt::Debug for DevServer {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("DevServer")
			.field("source", &self.source)
			.field("started", &self.server.is_some())
			.finish_non_exhaustive()
	}
}

impl DevServer {
	/// Whether this process started the server, rather than finding it
	/// running.
	pub(crate) fn started_here(&self) -> bool {
		self.server.is_some()
	}

	/// The URL with its password hidden, for printing.
	pub(crate) fn display_url(&self) -> String {
		match url::Url::parse(&self.url) {
			Ok(mut url) if url.password().is_some() => {
				let _ = url.set_password(Some("***"));
				url.to_string()
			}
			_ => self.url.clone(),
		}
	}
}

/// The source `db.dev_server` picks for this project, or `None` when there
/// is none: SQLite apps, and `dev_server = "off"`.
pub(crate) fn source(project: &Project) -> Option<Source> {
	if project.config.dialect != DbDialect::Postgres {
		return None;
	}

	match project.config.dev_server {
		DbDevServer::Off => None,
		DbDevServer::Devenv => Some(Source::Devenv),
		DbDevServer::Auto if uses_devenv_postgres(&project.root) => Some(Source::Devenv),
		DbDevServer::Auto | DbDevServer::Embedded => Some(Source::Embedded),
	}
}

/// Whether the project's `devenv.nix` turns on `services.postgres`.
fn uses_devenv_postgres(root: &Path) -> bool {
	fs::read_to_string(root.join("devenv.nix")).is_ok_and(|nix| nix.lines().any(enables_postgres))
}

/// A `devenv.nix` line that sets anything under `services.postgres`,
/// outside a comment.
fn enables_postgres(line: &str) -> bool {
	let code = line.split('#').next().unwrap_or_default();
	code.contains("services.postgres.enable")
		|| code.contains("services.postgres =")
		|| code.contains("services.postgres={")
}

/// The Postgres binaries `LEPTOSKIT_POSTGRES_BIN` names, to run instead of
/// downloading a release.
pub(crate) fn binaries_from_env() -> Option<PathBuf> {
	std::env::var_os(BINARIES_ENV).map(PathBuf::from)
}

/// Starts, or finds, the development database for `source`, and makes sure
/// the app's database exists in it. The embedded server runs `binaries`
/// when given, rather than a downloaded release.
pub(crate) fn start(
	project: &Project,
	source: Source,
	binaries: Option<&Path>,
) -> Result<DevServer> {
	let mut server = match source {
		Source::Devenv => devenv(|name| std::env::var(name).ok())?,
		Source::Embedded => {
			embedded(
				&project.root.join(".leptoskit/db/postgres"),
				&cache_dir()?,
				binaries,
			)?
		}
	};
	adopt_database(&mut server, &project.database_name())?;
	Ok(server)
}

/// Creates `database` on the server, unless it exists, and points the
/// server's URL, which names the `postgres` database, at it.
fn adopt_database(server: &mut DevServer, database: &str) -> Result<()> {
	ensure_database(&server.url, database)?;
	server.url = with_database(&server.url, database)?;
	Ok(())
}

/// devenv's `services.postgres`, as the variables `devenv shell` sets
/// describe it: `PGHOST` (a socket directory or a host), `PGPORT`, and
/// `PGUSER`. The URL names the `postgres` database every server has.
fn devenv(var: impl Fn(&str) -> Option<String>) -> Result<DevServer> {
	let Some(host) = var("PGHOST").filter(|host| !host.is_empty()) else {
		bail!(
			"devenv.nix runs Postgres, but PGHOST isn't set: run `leptoskit dev` inside `devenv \
			 shell`, with `devenv up` running, or set db.dev_server = \"embedded\" in \
			 leptoskit.toml"
		);
	};
	let port = var("PGPORT").unwrap_or_else(|| "5432".to_owned());
	let user = var("PGUSER")
		.or_else(|| var("USER"))
		.or_else(|| var("USERNAME"))
		.context("set PGUSER to the Postgres user devenv created")?;
	let url = devenv_url(&host, &port, &user, "postgres")?;
	let reachable = if is_socket_directory(&host) {
		Path::new(&host).join(format!(".s.PGSQL.{port}")).exists()
	} else {
		port.parse::<u16>()
			.is_ok_and(|port| TcpStream::connect((host.as_str(), port)).is_ok())
	};

	if !reachable {
		bail!("devenv's Postgres isn't running at {host} (port {port}); start it with `devenv up`");
	}

	Ok(DevServer {
		url,
		source: Source::Devenv,
		server: None,
	})
}

/// Whether `host` names a directory of Unix sockets rather than a machine, as
/// libpq decides: any absolute path, which on Windows starts with a drive.
fn is_socket_directory(host: &str) -> bool {
	Path::new(host).is_absolute()
}

/// A URL for a devenv database: over its socket directory when `host` is a
/// path, otherwise over TCP.
fn devenv_url(host: &str, port: &str, user: &str, database: &str) -> Result<String> {
	if is_socket_directory(host) {
		let query = url::form_urlencoded::Serializer::new(String::new())
			.append_pair("host", host)
			.append_pair("port", port)
			.append_pair("user", user)
			.append_pair("sslmode", "disable")
			.finish();
		return Ok(format!("postgres:///{database}?{query}"));
	}

	let mut url = url::Url::parse("postgres://localhost/").expect("a valid URL");
	url.set_host(Some(host))
		.with_context(|| format!("PGHOST `{host}` isn't a host"))?;
	let port = port
		.parse()
		.with_context(|| format!("PGPORT `{port}` isn't a port"))?;
	url.set_port(Some(port))
		.map_err(|()| anyhow::anyhow!("couldn't build the URL"))?;
	url.set_username(user)
		.map_err(|()| anyhow::anyhow!("couldn't build the URL"))?;
	url.set_path(database);
	url.query_pairs_mut().append_pair("sslmode", "disable");
	Ok(url.to_string())
}

/// Where downloaded Postgres releases are kept: `$XDG_CACHE_HOME`, or
/// `~/.cache`, under `leptoskit/postgresql`.
fn cache_dir() -> Result<PathBuf> {
	let base = std::env::var_os("XDG_CACHE_HOME")
		.map(PathBuf::from)
		.filter(|dir| dir.is_absolute())
		.or_else(|| {
			std::env::var_os("HOME")
				.or_else(|| std::env::var_os("USERPROFILE"))
				.map(|home| PathBuf::from(home).join(".cache"))
		})
		.context("neither XDG_CACHE_HOME nor HOME is set, so there is nowhere to keep Postgres")?;
	Ok(base.join("leptoskit").join("postgresql"))
}

/// The embedded server whose files live in `directory`: `data/`, the
/// `password` its superuser has, and the `port` it listens on, both chosen
/// on the first run. When it's already running, such as under `leptoskit
/// dev`, it's used as it is; otherwise it's started, and stops again when
/// the result is dropped.
fn embedded(directory: &Path, cache: &Path, binaries: Option<&Path>) -> Result<DevServer> {
	fs::create_dir_all(directory)
		.with_context(|| format!("couldn't create {}", directory.display()))?;
	let password = remembered(&directory.join("password"), || {
		let mut bytes = [0_u8; 18];
		getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!("{error}"))?;
		Ok(hex::encode(bytes))
	})?;
	let port: u16 = remembered(&directory.join("port"), || {
		let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
			.context("couldn't find a free port for Postgres")?;
		Ok(listener.local_addr()?.port().to_string())
	})?
	.parse()
	.with_context(|| format!("{} isn't a port", directory.join("port").display()))?;
	let url = format!(
		"postgres://{user}:{password}@localhost:{port}/postgres?sslmode=disable",
		user = postgresql_embedded::BOOTSTRAP_SUPERUSER,
	);
	let data = directory.join("data");

	if data.join("postmaster.pid").exists() && TcpStream::connect(("localhost", port)).is_ok() {
		return Ok(DevServer {
			url,
			source: Source::Embedded,
			server: None,
		});
	}

	let mut settings = Settings {
		version: VersionReq::parse(POSTGRES_VERSION).expect("a valid version requirement"),
		installation_dir: cache.to_path_buf(),
		password_file: directory.join("password"),
		data_dir: data,
		host: "localhost".to_owned(),
		port,
		username: postgresql_embedded::BOOTSTRAP_SUPERUSER.to_owned(),
		password,
		temporary: false,
		timeout: Some(Duration::from_mins(2)),
		..Settings::new()
	};

	if let Some(binaries) = binaries {
		settings.installation_dir = installation_of(binaries)?;
		settings.trust_installation_dir = true;
	} else if !downloaded(cache) {
		println!(
			"[db] downloading Postgres {} once into {}; later runs reuse it",
			POSTGRES_VERSION.trim_start_matches('='),
			cache.display()
		);
	}

	let mut server = PostgreSQL::new(settings);
	server.setup().context(
		"couldn't set up the development Postgres; set db.dev_server = \"off\" and DATABASE_URL \
		 to use another one",
	)?;
	server
		.start()
		.context("couldn't start the development Postgres")?;

	Ok(DevServer {
		url,
		source: Source::Embedded,
		server: Some(server),
	})
}

/// The contents of `file`, made with `make` and kept there, readable only
/// by its owner, on the first call.
fn remembered(file: &Path, make: impl FnOnce() -> Result<String>) -> Result<String> {
	if let Ok(existing) = fs::read_to_string(file) {
		return Ok(existing.trim().to_owned());
	}

	let value = make()?;
	let mut options = fs::OpenOptions::new();
	options.write(true).create_new(true);
	#[cfg(unix)]
	std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
	options
		.open(file)
		.and_then(|mut opened| opened.write_all(value.as_bytes()))
		.with_context(|| format!("couldn't write {}", file.display()))?;
	Ok(value)
}

/// Whether the cache holds a release of the major version the embedded
/// server runs.
fn downloaded(cache: &Path) -> bool {
	let major = format!("{}.", POSTGRES_VERSION.trim_start_matches('='));
	fs::read_dir(cache).is_ok_and(|entries| {
		entries
			.flatten()
			.any(|entry| entry.file_name().to_string_lossy().starts_with(&major))
	})
}

/// The installation `binaries` belongs to: the directory above `bin`, found
/// through links, since `initdb` finds its data files relative to itself.
fn installation_of(binaries: &Path) -> Result<PathBuf> {
	let initdb = ["initdb", "initdb.exe"]
		.into_iter()
		.map(|name| binaries.join(name))
		.find(|path| path.exists())
		.with_context(|| format!("{BINARIES_ENV} has no initdb in {}", binaries.display()))?;
	let resolved = fs::canonicalize(&initdb).unwrap_or(initdb);
	resolved
		.parent()
		.and_then(Path::parent)
		.map(Path::to_path_buf)
		.with_context(|| format!("{} isn't inside an installation", resolved.display()))
}

/// Creates `database` on the server `url` points at, unless it exists.
fn ensure_database(url: &str, database: &str) -> Result<()> {
	super::runtime()?.block_on(async {
		let db = leptoskit_db::Db::connect(url).await?;
		let statement = leptoskit_db::Statement {
			sql: "SELECT 1 FROM pg_catalog.pg_database WHERE datname = $1".to_owned(),
			args: vec![leptoskit_db::Value::Text(database.to_owned())],
		};
		let found = leptoskit_db::Connection::fetch(&db, statement).await?;

		if found.is_empty() {
			db.run_script(&format!(
				"CREATE DATABASE {}",
				leptoskit_db_schema::quote_identifier(database)
			))
			.await?;
		}

		anyhow::Ok(())
	})
}

/// `url` with its database replaced by `database`.
pub(crate) fn with_database(url: &str, database: &str) -> Result<String> {
	let mut url = url::Url::parse(url).context("the database URL isn't a URL")?;
	url.set_path(database);
	Ok(url.to_string())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::db::runtime;

	#[test]
	fn devenv_projects_are_told_apart_by_their_postgres_service() {
		assert!(enables_postgres("  services.postgres.enable = true;"));
		assert!(enables_postgres("services.postgres = { enable = true; };"));
		assert!(!enables_postgres("# services.postgres.enable = true;"));
		assert!(!enables_postgres("languages.rust.enable = true;"));

		let root = tempfile::tempdir().unwrap();
		assert!(!uses_devenv_postgres(root.path()));
		fs::write(
			root.path().join("devenv.nix"),
			"{ ... }: {\n  services.postgres.enable = true;\n}\n",
		)
		.unwrap();
		assert!(uses_devenv_postgres(root.path()));
	}

	#[test]
	fn devenv_urls_use_its_socket_or_its_host() {
		assert_eq!(
			devenv_url("/run/devenv/postgres", "5432", "ada", "my_app").unwrap(),
			"postgres:///my_app?host=%2Frun%2Fdevenv%2Fpostgres&port=5432&user=ada&sslmode=disable"
		);
		assert_eq!(
			devenv_url("127.0.0.1", "5433", "ada", "my_app").unwrap(),
			"postgres://ada@127.0.0.1:5433/my_app?sslmode=disable"
		);
		assert!(devenv_url("127.0.0.1", "port", "ada", "my_app").is_err());
	}

	#[test]
	fn devenv_needs_its_shell_and_its_server() {
		let none = |_: &str| None;
		let error = devenv(none).unwrap_err().to_string();
		assert!(error.contains("inside `devenv shell`"), "{error}");

		let sockets = tempfile::tempdir().unwrap();
		let host = sockets.path().to_string_lossy().to_string();
		let vars = |name: &str| {
			match name {
				"PGHOST" => Some(host.clone()),
				"PGPORT" => Some("6543".to_owned()),
				"PGUSER" => Some("ada".to_owned()),
				_ => None,
			}
		};
		let error = devenv(vars).unwrap_err().to_string();
		assert!(error.contains("start it with `devenv up`"), "{error}");

		fs::write(sockets.path().join(".s.PGSQL.6543"), "").unwrap();
		let server = devenv(vars).unwrap();
		assert_eq!(server.source, Source::Devenv);
		assert!(!server.started_here());
		assert!(server.url.contains("user=ada"));
		assert_eq!(server.display_url(), server.url);
	}

	#[test]
	fn values_are_remembered_privately() {
		let directory = tempfile::tempdir().unwrap();
		let file = directory.path().join("password");
		let first = remembered(&file, || Ok("secret".to_owned())).unwrap();
		let second = remembered(&file, || Ok("other".to_owned())).unwrap();
		assert_eq!((first.as_str(), second.as_str()), ("secret", "secret"));

		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			let mode = fs::metadata(&file).unwrap().permissions().mode();
			assert_eq!(mode & 0o777, 0o600);
		}
	}

	#[test]
	fn the_cache_knows_which_releases_it_holds() {
		let cache = tempfile::tempdir().unwrap();
		assert!(!downloaded(cache.path()));
		fs::create_dir(cache.path().join("17.6.0")).unwrap();
		assert!(!downloaded(cache.path()));
		fs::create_dir(cache.path().join("18.1.0")).unwrap();
		assert!(downloaded(cache.path()));
		assert!(installation_of(cache.path()).is_err());
	}

	/// The directory holding `initdb`, from the path, as the database
	/// tests find it. `None` skips the test, unless CI requires Postgres.
	fn local_binaries() -> Option<PathBuf> {
		let found = std::env::var_os(BINARIES_ENV)
			.map(PathBuf::from)
			.into_iter()
			.chain(std::env::split_paths(
				&std::env::var_os("PATH").unwrap_or_default(),
			))
			.find(|directory| directory.join("initdb").exists());

		if found.is_none() {
			assert!(
				std::env::var_os("LEPTOSKIT_REQUIRE_POSTGRES").is_none(),
				"LEPTOSKIT_REQUIRE_POSTGRES is set, but no Postgres binaries were found"
			);
		}

		found
	}

	#[test]
	fn the_embedded_server_keeps_its_data_between_runs() {
		let Some(binaries) = local_binaries() else {
			return;
		};
		let project = tempfile::tempdir().unwrap();
		let directory = project.path().join(".leptoskit/db/postgres");
		let cache = project.path().join("cache");

		let mut first = embedded(&directory, &cache, Some(&binaries)).unwrap();
		assert!(first.started_here());
		assert!(
			first.display_url().contains(":***@"),
			"{}",
			first.display_url()
		);
		adopt_database(&mut first, "my_app").unwrap();
		assert!(first.url.contains("/my_app?"), "{}", first.url);
		let create = "CREATE TABLE kept (id integer)";
		runtime()
			.unwrap()
			.block_on(async {
				let db = leptoskit_db::Db::connect(&first.url).await?;
				db.run_script(create).await?;
				anyhow::Ok(())
			})
			.unwrap();

		// A second user, such as `leptoskit db` beside `leptoskit dev`, finds
		// it running and leaves it be.
		let second = embedded(&directory, &cache, Some(&binaries)).unwrap();
		assert!(!second.started_here());
		drop(second);
		ensure_database(&first.url, "my_app").unwrap();
		drop(first);
		assert!(!directory.join("data/postmaster.pid").exists());

		// Started again, it has the same password, port, and rows.
		let mut again = embedded(&directory, &cache, Some(&binaries)).unwrap();
		assert!(again.started_here());
		adopt_database(&mut again, "my_app").unwrap();
		let tables = runtime()
			.unwrap()
			.block_on(async {
				let db = leptoskit_db::Db::connect(&again.url).await?;
				let statement = leptoskit_db::Statement {
					sql: "SELECT count(*) FROM kept".to_owned(),
					args: Vec::new(),
				};
				anyhow::Ok(leptoskit_db::Connection::fetch(&db, statement).await?)
			})
			.unwrap();
		assert_eq!(tables, [vec![leptoskit_db::Value::Int(0)]]);
		assert!(!cache.exists(), "local binaries download nothing");

		// A devenv socket URL is one the driver reads.
		let socket = devenv_url("/tmp", "5432", "ada", "my_app").unwrap();
		runtime()
			.unwrap()
			.block_on(leptoskit_db::Db::connect(&socket))
			.unwrap();
	}

	#[test]
	fn urls_can_point_at_another_database() {
		assert_eq!(
			with_database("postgres://u:p@localhost:5/postgres?sslmode=disable", "app").unwrap(),
			"postgres://u:p@localhost:5/app?sslmode=disable"
		);
		let socket = devenv_url("/run/pg", "5432", "ada", "postgres").unwrap();
		assert_eq!(
			with_database(&socket, "app").unwrap(),
			"postgres:///app?host=%2Frun%2Fpg&port=5432&user=ada&sslmode=disable"
		);
	}
}
