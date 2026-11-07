//! The app's `[services]`, as the deployment files run them.
//!
//! Every process service runs the app's image with its own command,
//! `/app/services/<name>`, which the `Dockerfile` copies in from the release
//! build. A remote service runs nothing: the app only gets its URL and token.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::ServiceTrust;
use leptoskit_config::services::env_segment;

use crate::services::Code;
use crate::services::env;

/// Where the image keeps the services' binaries.
pub const SERVICES_DIR: &str = "/app/services";

/// Where a service's volumes are mounted in its container.
pub const VOLUMES_DIR: &str = "/volumes";

/// A service, as the deployment files run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployService {
	/// Its name.
	pub name: String,
	/// Where `leptoskit build --release` leaves its binary, relative to the
	/// project root. `None` for a remote service.
	pub build_path: Option<String>,
	/// The port it listens on; `None` for a remote service.
	pub port: Option<u16>,
	/// A remote service's base URL.
	pub url: Option<String>,
	/// Its public path prefixes: empty when it isn't public.
	pub public_prefixes: Vec<String>,
	/// How far the app trusts it with its origin.
	pub trust: ServiceTrust,
	/// Whether it has a database of its own.
	pub database: bool,
	/// Its database pool size, when set.
	pub pool: Option<u32>,
	/// Its volumes.
	pub volumes: Vec<String>,
	/// Its plain variables.
	pub env: BTreeMap<String, String>,
	/// The secrets passed through to it alone.
	pub secrets: Vec<String>,
	/// The variables set on the app, before substitution.
	pub bind: BTreeMap<String, String>,
	/// Its migrations directory for the app's engine, when it has a
	/// database and code in this checkout or cargo's copy of it.
	pub migrations: Option<PathBuf>,
}

impl DeployService {
	/// Whether the deployment runs it as a process.
	#[must_use]
	pub fn is_process(&self) -> bool {
		self.build_path.is_some()
	}

	/// Where its binary is in the image.
	#[must_use]
	pub fn command(&self) -> String {
		format!("{SERVICES_DIR}/{}", self.name)
	}

	/// The app's variable holding its private URL.
	#[must_use]
	pub fn url_env(&self) -> String {
		env::url(&self.name)
	}

	/// The secret holding its token: the app reads it under this name, and
	/// the service as `LEPTOSKIT_SERVICE_TOKEN`.
	#[must_use]
	pub fn token_env(&self) -> String {
		env::token(&self.name)
	}

	/// The secret holding its database's URL, for platforms that can't give
	/// every process its own `DATABASE_URL` under one name.
	#[must_use]
	pub fn database_url_env(&self) -> String {
		format!("LEPTOSKIT_SERVICE_{}_DATABASE_URL", env_segment(&self.name))
	}

	/// The secret holding its database role's password.
	#[must_use]
	pub fn database_password_env(&self) -> String {
		format!(
			"LEPTOSKIT_SERVICE_{}_DATABASE_PASSWORD",
			env_segment(&self.name)
		)
	}

	/// Its Postgres database and role, beside the app's database `app`.
	#[must_use]
	pub fn postgres_name(&self, app: &str) -> String {
		crate::services::postgres_database(app, &self.name)
	}

	/// The variable naming a volume's directory, and the directory.
	#[must_use]
	pub fn volume_env(&self) -> Vec<(String, String)> {
		self.volumes
			.iter()
			.map(|volume| (env::volume(volume), format!("{VOLUMES_DIR}/{volume}")))
			.collect()
	}

	/// The app's variables for this service: its URL, then its `bind`
	/// entries with `{url}`, `{token}`, and `{public_path}` filled in.
	/// `token` is the target's way of writing the token's value, such as
	/// `${LEPTOSKIT_SERVICE_ANALYTICS_TOKEN}`.
	///
	/// `escape` writes the `bind` entry's own text in the target's syntax
	/// before the placeholders are filled in, so a `$` the app's author wrote
	/// stays a `$` while the token reference stays a reference.
	#[must_use]
	pub fn app_env(
		&self,
		url: &str,
		token: &str,
		escape: impl Fn(&str) -> String,
	) -> Vec<(String, String)> {
		let public_path = leptoskit_config::services::public_path(&self.name);
		let mut variables = vec![(self.url_env(), url.to_owned())];

		for (name, value) in &self.bind {
			variables.push((
				name.clone(),
				env::substitute(&escape(value), url, token, &public_path),
			));
		}

		variables
	}

	/// Whether a `bind` entry needs the token, so a target that keeps
	/// secrets apart from plain values has to treat it as one.
	#[must_use]
	pub fn binds_token(&self, name: &str) -> bool {
		self.bind
			.get(name)
			.is_some_and(|value| value.contains("{token}"))
	}
}

/// Reads the project's services. Workspace services build with the app's
/// server into `target/release`; installed ones into their own target
/// directory, as `leptoskit build` does.
///
/// # Errors
///
/// When cargo can't read the workspace or fetch an installed service, a
/// service's settings don't merge, or its binary can't be named.
pub fn read(root: &Path, config: &LeptoskitConfig) -> Result<Vec<DeployService>> {
	if config.services.is_empty() {
		return Ok(Vec::new());
	}

	let metadata = crate::wasm::cargo_metadata(root)?;
	// With `services.lock` in place, the files describe exactly what a
	// locked release build fetches; without one, resolving writes it.
	let locked = root.join(crate::services::LOCK_FILE).is_file();
	let resolved = crate::services::resolve(config, root, &metadata, locked)?;
	let mut services = Vec::new();

	let dialect = crate::services::dialect(config);

	for service in resolved {
		let migrations = service
			.settings
			.database
			.then(|| crate::services::database::migrations_dir(&service, dialect))
			.flatten();
		let settings = &service.settings;
		let binary = |package: &str, manifest: &Path| -> Result<String> {
			if let Some(bin) = &settings.bin {
				return Ok(bin.clone());
			}

			let dir = manifest
				.parent()
				.context("a package's manifest has no directory")?;
			super::binary(dir, package)
				.map(|(name, _)| name)
				.with_context(|| {
					format!("services.{}: set `bin` to the binary to run", settings.name)
				})
		};
		let build_path = match &service.code {
			Code::Workspace { package, manifest } => {
				Some(format!("target/release/{}", binary(package, manifest)?))
			}
			Code::External { package, manifest } => {
				Some(format!(
					"target/leptoskit/services/target/release/{}",
					binary(package, manifest)?
				))
			}
			Code::Remote => None,
		};
		let url = match &settings.source {
			leptoskit_config::ServiceSource::Remote { url } => {
				Some(url.trim_end_matches('/').to_owned())
			}
			_ => None,
		};

		services.push(DeployService {
			name: settings.name.clone(),
			build_path,
			port: settings.port,
			url,
			public_prefixes: settings.public_prefixes(),
			trust: settings.trust,
			database: settings.database,
			pool: settings.pool,
			volumes: settings.volumes.clone(),
			env: settings.env.clone(),
			secrets: settings.secrets.clone(),
			bind: settings.bind.clone(),
			migrations,
		});
	}

	Ok(services)
}

/// The SQL that gives each database-backed service a role and a database
/// of its own on the app's Postgres, run as a superuser with `psql`. The
/// passwords arrive as psql variables named after their secrets, so they
/// never appear in the SQL itself. Every statement is safe to run again:
/// an existing role keeps its database and gets the current password. Only
/// the app's role connects to the app's database `app`, and only a
/// service's role to its own.
#[must_use]
pub fn postgres_setup_sql(services: &[DeployService], app: &str) -> String {
	let mut sql = String::new();
	let mut databases = services
		.iter()
		.filter(|service| service.is_process() && service.database)
		.peekable();

	if databases.peek().is_some() {
		// Only the app's own role, which owns its database, connects to it.
		let _ = writeln!(sql, "REVOKE CONNECT ON DATABASE \"{app}\" FROM PUBLIC;");
	}

	for service in databases {
		let name = service.postgres_name(app);
		let password = service.database_password_env();
		let _ = write!(
			sql,
			"SELECT format('CREATE ROLE %I LOGIN', '{name}') WHERE NOT EXISTS (SELECT FROM \
			 pg_roles WHERE rolname = '{name}')\\gexec\nALTER ROLE \"{name}\" LOGIN PASSWORD \
			 :'{password}';\nSELECT format('CREATE DATABASE %I OWNER %I', '{name}', '{name}') \
			 WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = '{name}')\\gexec\nREVOKE \
			 CONNECT ON DATABASE \"{name}\" FROM PUBLIC;\n"
		);
	}

	sql
}

/// The connection budget: how many connections the app and its services
/// can open at once, against Postgres's limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budget {
	/// One line per process kind, with its share.
	pub lines: Vec<(String, u32)>,
	/// The total.
	pub total: u32,
	/// Postgres's `max_connections`.
	pub limit: u32,
}

/// Connections kept back for administrators, backups, and migrations.
pub const RESERVED_CONNECTIONS: u32 = 5;

impl Budget {
	/// The budget for an app whose server pool is `app_pool`, on `hosts`
	/// hosts, with `services`. Each process holds its pool and one more
	/// connection for live queries' `LISTEN`, and a deploy briefly runs one
	/// more of each while it replaces them, so every kind counts once more.
	#[must_use]
	pub fn new(app_pool: u32, hosts: u32, services: &[DeployService], limit: u32) -> Self {
		let mut lines = vec![(
			format!(
				"the app: {} processes (one per host, and one more while a deploy replaces it) × \
				 ({app_pool} pool + 1 live)",
				hosts + 1
			),
			(hosts + 1) * (app_pool + 1),
		)];

		for service in services
			.iter()
			.filter(|service| service.is_process() && service.database)
		{
			let pool = service
				.pool
				.unwrap_or(leptoskit_config::infra::InfraPostgres::DEFAULT_POOL);
			lines.push((
				format!(
					"services.{}: 2 processes (one, and one more while a deploy replaces it) × \
					 ({pool} pool + 1 live)",
					service.name
				),
				2 * (pool + 1),
			));
		}

		lines.push((
			"kept back for administrators, backups, and migrations".to_owned(),
			RESERVED_CONNECTIONS,
		));
		let total = lines.iter().map(|(_, count)| count).sum();
		Self {
			lines,
			total,
			limit,
		}
	}

	/// Whether everything fits.
	#[must_use]
	pub fn fits(&self) -> bool {
		self.total <= self.limit
	}

	/// What the budget is made of, one line each.
	#[must_use]
	pub fn explain(&self) -> String {
		let mut text = String::new();

		for (line, count) in &self.lines {
			let _ = write!(text, "\n  {count:>4}  {line}");
		}

		let _ = write!(
			text,
			"\n  {:>4}  in all, against max_connections = {}",
			self.total, self.limit
		);
		text
	}
}
