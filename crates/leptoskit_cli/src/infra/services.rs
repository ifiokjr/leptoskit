//! The app's `[services]` on managed hosts.
//!
//! Every process service is a Kamal role on the primary host, running the
//! release image with its own command. Its private address is a Docker
//! network alias on the `kamal` network, `<infra.name>-<service>`, which every
//! container of the role carries, so the app reaches it at the same name
//! across deploys; the app's own containers carry `<infra.name>-app` for the
//! services' calls back. A public service's paths reach it through
//! kamal-proxy's path routing on the app's host, with the prefix kept, so
//! the Cloudflare tunnel still sends everything to kamal-proxy.
//!
//! Kamal's secrets are global, so a service's role maps its own secrets to
//! the runtime contract's names with Kamal's `NAME:SECRET` aliases:
//! `LEPTOSKIT_SERVICE_TOKEN` from `LEPTOSKIT_SERVICE_<NAME>_TOKEN`, and
//! `DATABASE_URL` from `LEPTOSKIT_SERVICE_<NAME>_DATABASE_URL`. The app's
//! environment moves from the top level to its `web` role, so no service
//! inherits the app's secrets.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Write as _;

use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::ServiceTrust;
use leptoskit_config::infra::InfraConfig;
use serde_json::Value;
use serde_json::json;

use super::remote;
use crate::deploy::Project;
use crate::deploy::services::Budget;
use crate::deploy::services::DeployService;
use crate::deploy::services::VOLUMES_DIR;
use crate::deploy::services::postgres_setup_sql;

/// The app's database and role on the managed Postgres.
pub(super) const APP_DATABASE: &str = "leptoskit";

/// The uid the image's `app` user runs as, which owns the services'
/// volumes on the host.
const APP_UID: u32 = 10001;

/// The network alias of the app's containers.
pub(super) fn app_alias(c: &InfraConfig) -> String {
	format!("{}-app", c.name)
}

/// The network alias of a service's containers.
fn alias(c: &InfraConfig, service: &DeployService) -> String {
	format!("{}-{}", c.name, service.name)
}

/// The Kamal role a service runs as.
pub(super) fn role(service: &DeployService) -> String {
	format!("service-{}", service.name)
}

/// The host directory of a service's volume.
fn volume_dir(c: &InfraConfig, service: &DeployService, volume: &str) -> String {
	format!(
		"/var/lib/leptoskit/{}/services/{}/{volume}",
		c.name, service.name
	)
}

/// Whether the managed hosts can run the project's services, and whether
/// their database pools fit in Postgres.
///
/// # Errors
///
/// When a service needs something the managed hosts can't give it: several
/// hosts, an isolated service with public paths, a remote one with public
/// paths, or more connections than Postgres allows.
pub(super) fn check(c: &InfraConfig, project: &Project) -> Result<()> {
	let processes: Vec<&DeployService> = project
		.services
		.iter()
		.filter(|service| service.is_process())
		.collect();

	if let Some(service) = processes.first() {
		ensure!(
			c.instances == 1,
			"services.{}: managed services run on one host for now, because their private \
			 addresses are Docker network aliases that only containers on the same host resolve; \
			 set infra.instances = 1",
			service.name
		);
	}

	for service in &processes {
		ensure!(
			service.public_prefixes.is_empty() || service.trust == ServiceTrust::FirstParty,
			"services.{}: an isolated service's public paths must not see the app's cookies, and \
			 the managed edge can't remove them without taking over the zone's transform rules; \
			 keep it private (public = false), or set trust = \"first-party\" once you trust its \
			 code with the app's origin",
			service.name
		);
	}

	for service in project
		.services
		.iter()
		.filter(|service| !service.is_process())
	{
		ensure!(
			service.public_prefixes.is_empty(),
			"services.{}: kamal-proxy routes only to containers on the host, so a remote \
			 service's public paths need your own edge; make it private (public = false)",
			service.name
		);
	}

	budget(c, project)
}

/// Whether the app's and the services' pools fit in Postgres.
///
/// # Errors
///
/// When they don't, with what each part takes.
pub(super) fn budget(c: &InfraConfig, project: &Project) -> Result<()> {
	let budget = Budget::new(
		c.postgres.app_pool,
		u32::from(c.instances),
		&project.services,
		c.postgres.max_connections,
	);
	ensure!(
		budget.fits(),
		"the database pools need more connections than Postgres allows:{}\nraise \
		 infra.postgres.max_connections, or lower infra.postgres.app_pool and the services' `pool`",
		budget.explain()
	);
	Ok(())
}

/// What the services add to Kamal's secrets file.
#[derive(Debug, Default)]
pub(super) struct Secrets {
	/// Secrets read from the operator's environment under their own names.
	pub names: BTreeSet<String>,
	/// The app's variables that carry a token, with their values written in
	/// terms of it.
	pub lines: Vec<(String, String)>,
}

/// Adds the services to the Kamal configuration: the app's variables for
/// them, and one role per process service. Returns what they add to the
/// secrets file.
pub(super) fn kamal(
	config: &mut Value,
	c: &InfraConfig,
	project: &Project,
	primary: &str,
) -> Secrets {
	let mut secrets = Secrets::default();

	if project.services.is_empty() {
		return secrets;
	}

	let app_secret = |config: &mut Value, name: &str| {
		if let Some(list) = config["env"]["secret"].as_array_mut()
			&& !list.iter().any(|entry| entry == name)
		{
			list.push(json!(name));
		}
	};

	for service in &project.services {
		let url = match (&service.url, service.port) {
			(Some(url), _) => url.clone(),
			(None, Some(port)) => format!("http://{}:{port}", alias(c, service)),
			(None, None) => continue,
		};
		let token = service.token_env();
		app_secret(config, &token);
		secrets.names.insert(token.clone());

		// Kamal's secrets file reads `\$` as a `$`, and `${…}` as the
		// token's value; the plain values are JSON, where `$` is itself.
		for (variable, value) in service.app_env(&url, &format!("${{{token}}}"), |text| {
			if service
				.bind
				.values()
				.any(|bound| bound == text && bound.contains("{token}"))
			{
				text.replace('$', "\\$")
			} else {
				text.to_owned()
			}
		}) {
			if service.binds_token(&variable) {
				app_secret(config, &variable);
				secrets.lines.push((variable, value));
			} else {
				config["env"]["clear"][&variable] = json!(value);
			}
		}

		if !service.is_process() {
			continue;
		}

		let port = service.port.unwrap_or_default();
		let mut clear = json!({
			"LEPTOSKIT_SERVICE_NAME": service.name,
			"LEPTOSKIT_SERVICE_ADDR": format!("0.0.0.0:{port}"),
			"LEPTOSKIT_PUBLIC_ORIGIN": format!("https://{}", c.domain),
			"LEPTOSKIT_APP_URL": format!("http://{}:{}", app_alias(c), project.port),
		});
		let mut secret = vec![format!("LEPTOSKIT_SERVICE_TOKEN:{token}")];

		if service.database {
			let url = service.database_url_env();
			secret.push(format!("DATABASE_URL:{url}"));
			secrets.names.insert(url);

			if let Some(pool) = service.pool {
				clear[crate::services::env::DATABASE_POOL] = json!(pool.to_string());
			}
		}

		for (variable, path) in service.volume_env() {
			clear[variable] = json!(path);
		}

		for (variable, value) in &service.env {
			clear[variable] = json!(value);
		}

		for name in &service.secrets {
			secret.push(name.clone());
			secrets.names.insert(name.clone());
		}

		let volumes: Vec<String> = service
			.volumes
			.iter()
			.map(|volume| format!("{}:{VOLUMES_DIR}/{volume}", volume_dir(c, service, volume)))
			.collect();
		let mut options = json!({"read-only": true, "tmpfs": "/tmp:rw,nosuid,nodev,size=64m",
			"cap-drop": "ALL", "security-opt": "no-new-privileges:true",
			"network-alias": alias(c, service)});

		if !volumes.is_empty() {
			options["volume"] = json!(volumes);
		}

		let mut entry = json!({"hosts": [primary], "cmd": service.command(), "options": options,
			"env": {"clear": clear, "secret": secret}});

		if !service.public_prefixes.is_empty() {
			entry["proxy"] = json!({"host": c.domain, "app_port": port, "ssl": false,
				"forward_headers": true, "path_prefix": service.public_prefixes.join(","),
				"strip_path_prefix": false,
				"healthcheck": {"path": "/readyz", "interval": 2, "timeout": 5}});
		}

		config["servers"][role(service)] = entry;
	}

	if project.services.iter().any(DeployService::is_process) {
		config["servers"]["web"]["options"]["network-alias"] = json!(app_alias(c));
	}

	secrets
}

/// Moves the app's environment from the top level, where every role
/// inherits it, to its own `web` role.
pub(super) fn keep_app_env_to_itself(config: &mut Value, project: &Project) {
	if !project.services.iter().any(DeployService::is_process) {
		return;
	}

	if let Some(env) = config
		.as_object_mut()
		.and_then(|config| config.remove("env"))
	{
		config["servers"]["web"]["env"] = env;
	}
}

/// The services' secret values `lk` works out itself: each database-backed
/// service's URL, from its password, on the primary's private address.
///
/// # Errors
///
/// When a token, a database password, or a secret a service passes through
/// isn't set.
pub(super) fn values(project: &Project, private: &str) -> Result<BTreeMap<String, String>> {
	values_with(project, private, |name| std::env::var(name).ok())
}

/// [`values`], reading the operator's environment through `lookup`.
pub(super) fn values_with(
	project: &Project,
	private: &str,
	lookup: impl Fn(&str) -> Option<String>,
) -> Result<BTreeMap<String, String>> {
	let required = |name: &str| -> Result<String> {
		lookup(name)
			.filter(|value| !value.is_empty())
			.ok_or_else(|| anyhow::anyhow!("set required service secret {name}"))
	};
	let mut values = BTreeMap::new();

	for service in &project.services {
		required(&service.token_env())?;

		if !service.is_process() {
			continue;
		}

		for name in &service.secrets {
			required(name)?;
		}

		if service.database {
			let role = service.postgres_name(APP_DATABASE);
			let password = required(&service.database_password_env())?;
			let mut url = url::Url::parse(&format!(
				"postgres://{role}@{private}:5432/{role}?sslmode=disable"
			))?;
			url.set_password(Some(&password))
				.map_err(|()| anyhow::anyhow!("couldn't bind a service's database password"))?;
			values.insert(service.database_url_env(), url.to_string());
		}
	}

	Ok(values)
}

/// Prepares the primary host for the services before a release boots
/// them: their volumes' directories, owned by the image's user, and on
/// Postgres a role and database for each database-backed service. Both
/// are safe to repeat; a service added later gets its own on its first
/// release.
///
/// # Errors
///
/// When the host can't be reached, or Postgres refuses the setup.
pub(super) fn prepare(ip: &str, c: &InfraConfig, project: &Project) -> Result<()> {
	let directories: Vec<String> = project
		.services
		.iter()
		.filter(|service| service.is_process())
		.flat_map(|service| {
			service
				.volumes
				.iter()
				.map(|volume| remote::quote(&volume_dir(c, service, volume)))
				.collect::<Vec<_>>()
		})
		.collect();

	if !directories.is_empty() {
		remote::execute(
			ip,
			&format!(
				"set -eu; install -d -m 700 -o {APP_UID} -g {APP_UID} {}",
				directories.join(" ")
			),
			None,
		)?;
	}

	let sql = postgres_setup_sql(&project.services, APP_DATABASE);

	if sql.is_empty() {
		return Ok(());
	}

	// The passwords go over SSH's standard input as psql variables, never
	// on a command line another process could read.
	let mut input = String::new();

	for service in project
		.services
		.iter()
		.filter(|service| service.is_process() && service.database)
	{
		let name = service.database_password_env();
		let password = std::env::var(&name).unwrap_or_default();
		ensure!(!password.is_empty(), "set required service secret {name}");
		let _ = writeln!(input, "\\set {name} '{}'", psql_quote(&password));
	}

	input.push_str(&sql);
	remote::execute(
		ip,
		&format!(
			"docker exec -i {}-db psql -X -q -U leptoskit_admin -d postgres -v ON_ERROR_STOP=1",
			c.name
		),
		Some(input.as_bytes()),
	)?;
	Ok(())
}

/// `value` inside a psql single-quoted meta-command argument.
fn psql_quote(value: &str) -> String {
	value.replace('\\', "\\\\").replace('\'', "''")
}
