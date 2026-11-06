//! Running services: the runtime contract's environment for each service and
//! for the app, their databases and volumes, and their processes.
//!
//! `leptoskit dev` and `leptoskit serve` both [`prepare`] the environment once
//! per session, so a service keeps its token across rebuilds, then [`start`]
//! the services, each waited on until `/readyz` answers, before the app.

use std::collections::BTreeMap;
use std::io::BufRead as _;
use std::io::BufReader;
use std::io::Read as _;
use std::io::Write as _;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context as _;
use anyhow::Result;
use leptoskit_config::DbDialect;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::ServiceSource;

use super::BuiltService;
use super::Code;
use super::env;
use crate::ui::BAD;

/// How long a service has to answer `/readyz` before `leptoskit` gives up.
pub const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// What the services need from the session that runs them.
pub struct Session<'a> {
	/// The project.
	pub config: &'a LeptoskitConfig,
	/// The project root.
	pub root: &'a Path,
	/// The origin browsers reach the app at, such as `http://127.0.0.1:3000`.
	pub public_origin: String,
	/// The app server's private base URL.
	pub app_url: String,
	/// The app's database URL, when known: a service's Postgres database goes
	/// on the same server.
	pub app_database_url: Option<String>,
	/// Each process service's token, made once per session.
	pub tokens: &'a BTreeMap<String, String>,
}

/// One token per service that runs as a process: 32 random bytes, in hex.
///
/// # Errors
///
/// When the system has no randomness to give.
pub fn tokens(services: &[BuiltService]) -> Result<BTreeMap<String, String>> {
	let mut tokens = BTreeMap::new();

	for built in services {
		if matches!(built.service.code, Code::Remote) {
			continue;
		}

		let mut bytes = [0u8; 32];
		getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!("no randomness: {error}"))?;
		tokens.insert(built.service.name().to_owned(), hex::encode(bytes));
	}

	Ok(tokens)
}

/// A service ready to start.
#[derive(Debug, Clone)]
pub struct Launch {
	/// Its name.
	pub name: String,
	/// Its executable.
	pub binary: PathBuf,
	/// The address it binds.
	pub address: SocketAddr,
	/// Its environment, beyond what it inherits.
	pub env: Vec<(String, String)>,
}

/// The services' launches, and what the app receives for them.
#[derive(Debug, Clone, Default)]
pub struct Prepared {
	/// The services to start, in name order.
	pub launches: Vec<Launch>,
	/// The variables the app receives: each service's URL, token, and
	/// bindings.
	pub app_env: Vec<(String, String)>,
}

/// Works out every service's environment and the app's, creating each
/// service's volumes and database on the way.
///
/// # Errors
///
/// When a volume or database can't be created, or a service that needs a
/// database has no server to put it on.
pub fn prepare(services: &[BuiltService], session: &Session<'_>) -> Result<Prepared> {
	let mut prepared = Prepared::default();

	for built in services {
		let settings = &built.service.settings;
		let name = settings.name.as_str();
		let public_path = settings.public_path();

		let (url, token) = match (&settings.source, &built.binary) {
			(ServiceSource::Remote { url }, _) => {
				let token = std::env::var(env::token(name)).unwrap_or_else(|_| {
					crate::ui::warning(format_args!(
						"services.{name} is remote and {} isn't set, so the app can't present its \
						 token",
						env::token(name)
					));
					String::new()
				});
				(url.trim_end_matches('/').to_owned(), token)
			}

			(_, Some(binary)) => {
				let port = settings
					.port
					.with_context(|| format!("services.{name} has no port"))?;
				let address = SocketAddr::from(([127, 0, 0, 1], port));
				let token = session
					.tokens
					.get(name)
					.cloned()
					.with_context(|| format!("services.{name} has no token"))?;
				let env = service_env(built, session, address, &token)?;
				prepared.launches.push(Launch {
					name: name.to_owned(),
					binary: binary.clone(),
					address,
					env,
				});
				(format!("http://{address}"), token)
			}

			(_, None) => anyhow::bail!("services.{name} wasn't built"),
		};

		prepared.app_env.push((env::url(name), url.clone()));
		prepared.app_env.push((env::token(name), token.clone()));

		for (variable, value) in &settings.bind {
			prepared.app_env.push((
				variable.clone(),
				env::substitute(value, &url, &token, &public_path),
			));
		}
	}

	Ok(prepared)
}

/// One process service's own variables.
fn service_env(
	built: &BuiltService,
	session: &Session<'_>,
	address: SocketAddr,
	token: &str,
) -> Result<Vec<(String, String)>> {
	let settings = &built.service.settings;
	let name = settings.name.as_str();
	let mut vars = vec![
		(env::NAME.to_owned(), name.to_owned()),
		(env::ADDR.to_owned(), address.to_string()),
		(env::TOKEN.to_owned(), token.to_owned()),
		(env::PUBLIC_ORIGIN.to_owned(), session.public_origin.clone()),
		(env::APP_URL.to_owned(), session.app_url.clone()),
	];

	for volume in &settings.volumes {
		let directory = super::dev_dir(session.root, name).join(volume);
		std::fs::create_dir_all(&directory)
			.with_context(|| format!("could not create {}", directory.display()))?;
		vars.push((env::volume(volume), directory.display().to_string()));
	}

	if settings.database {
		vars.push((env::DATABASE_URL.to_owned(), database_url(built, session)?));

		if let Some(pool) = settings.pool {
			vars.push((env::DATABASE_POOL.to_owned(), pool.to_string()));
		}
	}

	vars.extend(settings.env.clone());

	// A service starts with a cleared environment (see [`inherited`]), so
	// these are the only secrets from `leptoskit`'s own environment it sees.
	for secret in &settings.secrets {
		match std::env::var(secret) {
			Ok(value) => vars.push((secret.clone(), value)),
			Err(_) => {
				crate::ui::warning(format_args!(
					"services.{name} expects the secret {secret}, which isn't set"
				));
			}
		}
	}

	Ok(vars)
}

/// The service's own database: a SQLite file under `.leptoskit/services/`,
/// or a database of its own on the app's Postgres server, created when
/// missing. The service applies its own migrations.
fn database_url(built: &BuiltService, session: &Session<'_>) -> Result<String> {
	let name = built.service.name();
	// The app's engine, or SQLite when the app has no database of its own.
	let dialect = session
		.config
		.db
		.as_ref()
		.map_or(DbDialect::Sqlite, |db| db.dialect);

	match dialect {
		DbDialect::Sqlite => {
			let path = super::sqlite_path(session.root, name);

			if let Some(parent) = path.parent() {
				std::fs::create_dir_all(parent)
					.with_context(|| format!("could not create {}", parent.display()))?;
			}

			Ok(format!("sqlite://{}", path.display()))
		}

		DbDialect::Postgres => {
			let app = session.app_database_url.as_deref().with_context(|| {
				format!(
					"services.{name} asks for a database beside the app's, but the app's Postgres \
					 URL isn't known; set {}",
					session
						.config
						.db
						.as_ref()
						.map_or("DATABASE_URL", |db| db.url_env.as_str())
				)
			})?;
			let app_database = app_database_name(app)?;
			let database = super::postgres_database(&app_database, name);
			let server = crate::db::dev_server::with_database(app, "postgres")?;

			if crate::db::branches::ensure_database(&server, &database, None)? {
				crate::ui::tagged(name, format_args!("created the database `{database}`"));
			}

			crate::db::dev_server::with_database(app, &database)
		}
	}
}

/// The database a Postgres URL names.
fn app_database_name(url: &str) -> Result<String> {
	let parsed = url::Url::parse(url).context("the app's database URL isn't a URL")?;
	let name = parsed.path().trim_start_matches('/');
	anyhow::ensure!(!name.is_empty(), "the app's database URL names no database");
	Ok(name.to_owned())
}

/// What a service inherits from `leptoskit`'s own environment: what a
/// program needs to run at all, and nothing that could hold a secret.
///
/// - `PATH`, `HOME`, `USER`, `LOGNAME`: finding programs and per-user files.
/// - `TMPDIR`, `TMP`, `TEMP`: temporary files.
/// - `LANG`, `LANGUAGE`, `LC_*`, `TZ`: locale and time zone.
/// - `RUST_LOG`, `RUST_BACKTRACE`, `RUST_LIB_BACKTRACE`: its diagnostics.
/// - `SSL_CERT_FILE`, `SSL_CERT_DIR`: the TLS roots its outbound calls trust.
/// - `HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY` (either case): reaching the
///   network the way the developer's machine does.
/// - `TERM`, `NO_COLOR`, `CLICOLOR`, `CLICOLOR_FORCE`: how its output looks.
/// - On Windows, `SystemRoot`, `windir`, `ComSpec`, `PATHEXT`, `SystemDrive`,
///   `USERPROFILE`, `APPDATA`, `LOCALAPPDATA`, and `ProgramData`, without which
///   processes there fail to start or to find their files.
///
/// Everything else, such as the app's `DATABASE_URL` or a cloud token in
/// the developer's shell, reaches a service only when its config lists it
/// in `secrets`.
const INHERITED: &[&str] = &[
	"PATH",
	"HOME",
	"USER",
	"LOGNAME",
	"TMPDIR",
	"TMP",
	"TEMP",
	"LANG",
	"LANGUAGE",
	"TZ",
	"RUST_LOG",
	"RUST_BACKTRACE",
	"RUST_LIB_BACKTRACE",
	"SSL_CERT_FILE",
	"SSL_CERT_DIR",
	"HTTP_PROXY",
	"HTTPS_PROXY",
	"NO_PROXY",
	"TERM",
	"NO_COLOR",
	"CLICOLOR",
	"CLICOLOR_FORCE",
	"SYSTEMROOT",
	"WINDIR",
	"COMSPEC",
	"PATHEXT",
	"SYSTEMDRIVE",
	"USERPROFILE",
	"APPDATA",
	"LOCALAPPDATA",
	"PROGRAMDATA",
];

/// Whether a service inherits the variable `name` from `leptoskit`.
#[must_use]
pub fn is_inherited(name: &str) -> bool {
	name.starts_with("LC_")
		|| INHERITED
			.iter()
			.any(|allowed| allowed.eq_ignore_ascii_case(name))
}

/// The part of `vars` a service inherits.
pub fn inherited(
	vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
	vars.into_iter()
		.filter(|(name, _)| name.to_str().is_some_and(is_inherited))
		.collect()
}

/// The services' processes, stopped when dropped: each gets `SIGTERM`, and
/// `SIGKILL` once its grace period passes.
pub struct Running {
	children: Vec<(String, Child)>,
	grace: Duration,
}

impl Default for Running {
	fn default() -> Self {
		Self {
			children: Vec::new(),
			grace: crate::server::DEV_STOP_GRACE,
		}
	}
}

impl Running {
	/// Whether any service is running.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.children.is_empty()
	}

	/// Stops every service, together.
	pub fn stop(&mut self) {
		let mut children: Vec<&mut Child> =
			self.children.iter_mut().map(|(_, child)| child).collect();
		crate::server::stop_gracefully(&mut children, self.grace);
		self.children.clear();
	}
}

impl Drop for Running {
	fn drop(&mut self) {
		self.stop();
	}
}

/// Starts each service with a cleared environment holding only what it
/// [inherits](INHERITED), `env` (shared by every service, such as the drain
/// delay `leptoskit dev` sets), and its own variables, and waits until it's
/// ready. Their output is tagged with their names. Stopping them gives each
/// `grace` between `SIGTERM` and `SIGKILL`.
///
/// # Errors
///
/// When a service's port is already taken, or it can't start, exits, or
/// isn't ready within [`READY_TIMEOUT`]; the ones already started are
/// stopped.
pub fn start(prepared: &Prepared, env: &[(String, String)], grace: Duration) -> Result<Running> {
	let mut running = Running {
		children: Vec::new(),
		grace,
	};

	for launch in &prepared.launches {
		// A stale process on the port would pass the readiness check below in
		// the new one's place.
		if let Err(error) =
			crate::server::ensure_port_free(launch.address, &format!("services.{}", launch.name))
		{
			running.stop();
			return Err(error);
		}

		let mut child = Command::new(&launch.binary)
			.env_clear()
			.envs(inherited(std::env::vars_os()))
			.envs(env.iter().map(|(name, value)| (name, value)))
			.envs(launch.env.iter().map(|(name, value)| (name, value)))
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.with_context(|| {
				format!(
					"could not start services.{} ({})",
					launch.name,
					launch.binary.display()
				)
			})?;
		tag_output(&launch.name, child.stdout.take(), false);
		tag_output(&launch.name, child.stderr.take(), true);
		crate::ui::tagged(&launch.name, format_args!("starting on {}", launch.address));
		let ready = wait_ready(&mut child, launch.address, READY_TIMEOUT).and_then(|()| {
			// Whatever answered must be this child, still running.
			match child.try_wait().context("could not check the service")? {
				Some(status) => {
					anyhow::bail!("{BAD}it exited with {status}{BAD:#} just after it was ready")
				}
				None => Ok(()),
			}
		});
		running.children.push((launch.name.clone(), child));

		if let Err(error) = ready {
			running.stop();
			return Err(error.context(format!("services.{} didn't start", launch.name)));
		}

		crate::ui::tagged(&launch.name, "ready");
	}

	Ok(running)
}

/// Prints each line `stream` writes, tagged with the service's name.
fn tag_output(name: &str, stream: Option<impl std::io::Read + Send + 'static>, error: bool) {
	let Some(stream) = stream else {
		return;
	};
	let name = name.to_owned();
	std::thread::spawn(move || {
		for line in BufReader::new(stream).lines() {
			let Ok(line) = line else {
				break;
			};

			if error {
				crate::ui::tagged_error(&name, line);
			} else {
				crate::ui::tagged(&name, line);
			}
		}
	});
}

/// Waits until `GET /readyz` on `address` answers 200, the child exits, or
/// `timeout` passes.
fn wait_ready(child: &mut Child, address: SocketAddr, timeout: Duration) -> Result<()> {
	let started = Instant::now();

	loop {
		if let Some(status) = child.try_wait().context("could not check the service")? {
			anyhow::bail!("{BAD}it exited with {status}{BAD:#} before it was ready");
		}

		if ready(address) {
			return Ok(());
		}

		anyhow::ensure!(
			started.elapsed() < timeout,
			"it didn't answer GET /readyz with 200 on {address} within {}s",
			timeout.as_secs()
		);
		std::thread::sleep(Duration::from_millis(100));
	}
}

/// Whether `GET /readyz` on `address` answers 200.
pub fn ready(address: SocketAddr) -> bool {
	let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(500)) else {
		return false;
	};
	let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
	let request = format!("GET /readyz HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");

	if stream.write_all(request.as_bytes()).is_err() {
		return false;
	}

	let mut head = [0u8; 12];
	stream.read_exact(&mut head).is_ok()
		&& (head.starts_with(b"HTTP/1.1 200") || head.starts_with(b"HTTP/1.0 200"))
}
