use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::infra::InfraConfig;
use serde_json::json;

use super::Release;
use super::ReleaseArgs;
use super::applied_bindings;
use super::directory;
use super::generate;
use super::history_fingerprint;
use super::lock;
use super::managed_settings;
use super::output;
use super::remote::Reservation;
use super::remote::{self};
use super::remote_history;
use super::runtime;

pub(super) fn history(ip: &str, c: &InfraConfig) -> Result<Vec<Release>> {
	serde_json::from_slice(&remote::read(ip, &c.name, "releases.json")?)
		.context("invalid shared release history; refusing to overwrite")
}

pub(super) fn stop(hosts: &[String], c: &InfraConfig) -> Result<()> {
	for host in hosts {
		remote::execute(
			host,
			&format!(
				"set -eu; ids=$(docker ps -q --filter label=service={} --filter label=role=web); \
				 if [ -n \"$ids\" ]; then docker stop --time 30 $ids; fi",
				c.name
			),
			None,
		)?;
	}
	Ok(())
}

fn schema(root: &Path, config: &LeptoskitConfig) -> Result<Vec<(String, String)>> {
	let migrations = leptoskit_db::load_migrations(
		&root.join(&config.db.as_ref().context("missing database")?.migrations),
	)?;
	Ok(migrations
		.iter()
		.map(|m| (m.name.to_string(), m.checksum.to_string()))
		.collect())
}

pub(super) fn hosts(dir: &Path) -> Result<Vec<String>> {
	let result = Command::new("tofu")
		.current_dir(dir)
		.args(["output", "-json", "server_ips"])
		.output()?;
	ensure!(
		result.status.success(),
		"couldn't read application hosts from applied infrastructure"
	);
	let hosts: Vec<String> = serde_json::from_slice(&result.stdout)?;
	ensure!(
		!hosts.is_empty()
			&& hosts.len() <= 16
			&& hosts
				.iter()
				.all(|s| s.parse::<std::net::Ipv4Addr>().is_ok()),
		"invalid applied application hosts"
	);
	Ok(hosts)
}

fn image(c: &InfraConfig) -> String {
	if c.image.starts_with(&format!("{}/", c.registry)) {
		c.image.clone()
	} else {
		format!("{}/{}", c.registry, c.image)
	}
}

fn digest(ip: &str, c: &InfraConfig, version: &str) -> Result<String> {
	let container = remote::quote(&format!("{}-web-{version}", c.name));
	let script = format!(
		"set -eu; id=$(docker inspect --format '{{{{.Image}}}}' {container}); docker image \
		 inspect --format '{{{{json .RepoDigests}}}}' \"$id\""
	);
	let values: Vec<String> = serde_json::from_slice(&remote::execute(ip, &script, None)?)?;
	values
		.into_iter()
		.find(|value| value.starts_with(&format!("{}@sha256:", image(c))))
		.context("running container has no verified registry digest")
}

fn cached_digest(ip: &str, c: &InfraConfig, version: &str) -> Result<String> {
	let tag = remote::quote(&format!("{}:{version}", image(c)));
	let bytes = remote::execute(
		ip,
		&format!("docker image inspect --format '{{{{json .RepoDigests}}}}' {tag}"),
		None,
	)?;
	let values: Vec<String> = serde_json::from_slice(&bytes)?;
	values
		.into_iter()
		.find(|v| v.starts_with(&format!("{}@sha256:", image(c))))
		.context("pulled image lacks a registry digest")
}

fn prepare_recorded(
	dir: &Path,
	c: &InfraConfig,
	hosts: &[String],
	release: &Release,
	env: &BTreeMap<String, String>,
	reservation: &Reservation,
) -> Result<()> {
	let expected = release
		.image_digest
		.as_ref()
		.context("release predates digest recording; import a verified image as a new version")?;
	execute(dir, &["registry", "login"], env, reservation)?;
	execute(
		dir,
		&["build", "pull", "--version", &release.version],
		env,
		reservation,
	)?;
	for host in hosts {
		ensure!(
			&cached_digest(host, c, &release.version)? == expected,
			"registry tag differs from the recorded image digest; release blocked before database \
			 or traffic changes"
		);
	}
	verify_schema_labels(hosts, c, &release.version, &release.migrations)?;
	if let Some(evidence) = &release.evidence {
		for host in hosts {
			evidence.check_remote(
				host,
				&format!("{}:{}", image(c), release.version),
				&release.migrations,
			)?;
		}
	}
	Ok(())
}

fn verify_schema_labels(
	hosts: &[String],
	c: &InfraConfig,
	version: &str,
	expected: &str,
) -> Result<()> {
	let first = cached_digest(&hosts[0], c, version)?;
	for host in hosts {
		ensure!(
			cached_digest(host, c, version)? == first,
			"hosts pulled different image digests; release blocked before traffic changes"
		);
		let script = format!(
			"docker image inspect --format '{{{{index .Config.Labels \"{}\"}}}}' {}",
			super::build::SCHEMA_LABEL,
			remote::quote(&format!("{}:{version}", image(c)))
		);
		let value = remote::execute(host, &script, None)?;
		ensure!(
			String::from_utf8(value)?.trim() == expected,
			"image migration metadata differs from the expected schema; build this checkout with \
			 `lk infra build --version <tag> --push` before releasing"
		);
	}
	Ok(())
}

fn secrets(config: &LeptoskitConfig, private: &str) -> Result<BTreeMap<String, String>> {
	let c = config.infra.as_ref().context("missing infrastructure")?;
	let required = |name: &str| {
		std::env::var(name)
			.ok()
			.filter(|s| !s.is_empty())
			.with_context(|| format!("set required secret {name}"))
	};
	let admin = required("POSTGRES_PASSWORD")?;
	let password = required("DATABASE_PASSWORD")?;
	ensure!(
		admin != password,
		"administrator and application database passwords must differ"
	);
	let mut db = url::Url::parse(&format!(
		"postgres://leptoskit@{private}:5432/leptoskit?sslmode=disable"
	))?;
	db.set_password(Some(&password))
		.map_err(|()| anyhow::anyhow!("couldn't bind database password"))?;
	let mut values = BTreeMap::new();
	if c.plugins.analytics.is_some() {
		let key = required("LEPTOSKIT_ANALYTICS_READ_KEY")?;
		ensure!(
			(32..=256).contains(&key.len()) && key.bytes().all(|b| (33..127).contains(&b)),
			"LEPTOSKIT_ANALYTICS_READ_KEY needs 32..=256 printable non-space ASCII characters"
		);
		values.insert("LEPTOSKIT_ANALYTICS_READ_KEY".into(), key);
	}
	values.insert(config.db.as_ref().unwrap().url_env.clone(), db.to_string());
	if c.services.cache {
		let mut url = url::Url::parse(&format!("redis://{private}:6379"))?;
		url.set_password(Some(&required("VALKEY_PASSWORD")?))
			.map_err(|()| anyhow::anyhow!("couldn't bind cache password"))?;
		values.insert("VALKEY_URL".into(), url.to_string());
	}
	if c.services.relay {
		let mut url = url::Url::parse(&format!("nats://{private}:4222"))?;
		url.set_username(&required("NATS_TOKEN")?)
			.map_err(|()| anyhow::anyhow!("couldn't bind relay token"))?;
		values.insert(
			config
				.rpc
				.live
				.relay_url_env
				.clone()
				.context("configure rpc.live.relay_url_env")?,
			url.to_string(),
		);
	}
	for name in c
		.secrets
		.iter()
		.chain(config.auth.iter().map(|auth| &auth.keys_env))
	{
		required(name)?;
	}
	required("KAMAL_REGISTRY_PASSWORD")?;
	Ok(values)
}

fn execute(
	dir: &Path,
	args: &[&str],
	env: &BTreeMap<String, String>,
	reservation: &Reservation,
) -> Result<()> {
	reservation.check()?;
	let status = Command::new("kamal")
		.current_dir(dir)
		.args(args)
		.args(["-c", "release.yml"])
		.envs(env)
		// Every boot (including failure recovery) gets the actual selected
		// version, rather than the version baked into a reused config file.
		.env(
			"LEPTOSKIT_RELEASE",
			args.windows(2)
				.find(|pair| pair[0] == "--version")
				.map_or("unknown", |pair| pair[1]),
		)
		.status()
		.context("couldn't run Kamal 2")?;
	reservation.check()?;
	ensure!(
		status.success(),
		"Kamal operation failed ({status}); shared operation status records the failure"
	);
	Ok(())
}

fn record(ip: &str, c: &InfraConfig, owner: &str, phase: &str, version: &str) -> Result<()> {
	remote::write(
		ip,
		&format!("{}/operation.json", remote::base(&c.name)),
		&serde_json::to_vec(&json!({
		"owner": owner, "phase": phase, "version": version, "updated_at": chrono::Utc::now().to_rfc3339()}))?,
	)
}

struct Flight<'a> {
	ip: &'a str,
	config: &'a InfraConfig,
	owner: &'a str,
	version: &'a str,
	finished: bool,
}

impl Flight<'_> {
	fn checked(&self, result: Result<String>, hosts: &[String]) -> Result<String> {
		match result {
			Ok(image) => Ok(image),
			Err(error) => {
				stop(hosts, self.config)?;
				record(
					self.ip,
					self.config,
					self.owner,
					"verification-failed-app-stopped",
					self.version,
				)?;
				Err(error)
			}
		}
	}
}

impl Drop for Flight<'_> {
	fn drop(&mut self) {
		if self.finished {
			return;
		}
		let phase = remote::read(self.ip, &self.config.name, "operation.json")
			.ok()
			.and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
		if phase.as_ref().is_some_and(|value| {
			!matches!(
				value["phase"].as_str(),
				Some("previous-restored" | "verification-failed-app-stopped" | "failed")
			)
		}) {
			let _ = record(self.ip, self.config, self.owner, "failed", self.version);
		}
	}
}

fn bootstrap(
	dir: &Path,
	c: &InfraConfig,
	hosts: &[String],
	env: &BTreeMap<String, String>,
	reservation: &Reservation,
) -> Result<()> {
	let ip = &hosts[0];
	if c.services.relay {
		remote::execute(
			ip,
			&format!(
				"set -eu; install -d -m 700 -o 1000 -g 1000 /var/lib/leptoskit/{}/nats",
				c.name
			),
			None,
		)?;
	}

	for host in hosts {
		remote::execute(host, "cloud-init status --wait >/dev/null", None)?;
	}
	execute(dir, &["server", "bootstrap"], env, reservation)?;
	for accessory in [
		Some("db"),
		Some("tunnel"),
		c.services.cache.then_some("cache"),
		c.services.relay.then_some("relay"),
	]
	.into_iter()
	.flatten()
	{
		let accessory_hosts = if accessory == "tunnel" {
			hosts
		} else {
			&hosts[..1]
		};
		for host in accessory_hosts {
			let inspect = remote::execute(
				host,
				&format!(
					"docker inspect --format '{{{{.State.Status}}}}' {}-{}",
					c.name, accessory
				),
				None,
			);
			if inspect.as_ref().is_ok_and(|v| v == b"running\n") {
				continue;
			}
			let action = if inspect.is_ok() { "start" } else { "boot" };
			execute(
				dir,
				&["accessory", action, accessory, "--hosts", host],
				env,
				reservation,
			)?;
		}
	}
	remote::execute(
		ip,
		&format!(
			"for i in $(seq 1 60); do if docker exec {}-db pg_isready -U leptoskit -d leptoskit \
			 >/dev/null; then exit 0; fi; sleep 1; done; exit 1",
			c.name
		),
		None,
	)?;

	Ok(())
}

fn verify_release(
	hosts: &[String],
	c: &InfraConfig,
	version: &str,
	expected: &str,
	target: Option<&Release>,
) -> Result<String> {
	ensure!(
		remote_history(&hosts[0], &c.name)? == expected,
		"database does not match release; shared history was not advanced"
	);
	let image = digest(&hosts[0], c, version)?;
	for host in hosts {
		ensure!(
			digest(host, c, version)? == image,
			"hosts are running different image digests"
		);
	}
	if let Some(target) = target {
		if let Some(evidence) = &target.evidence {
			for host in hosts {
				evidence.check_remote(host, &image, expected)?;
			}
		}
		ensure!(
			target.image_digest.as_ref() == Some(&image),
			"running image differs from recorded digest"
		);
	}
	Ok(image)
}

fn runtime_files(
	dir: &Path,
	c: &InfraConfig,
	root: &Path,
	config: &LeptoskitConfig,
	hosts: &[String],
	private: &str,
) -> Result<BTreeMap<String, String>> {
	let project = crate::deploy::Project::read(root, config)?;
	ensure!(
		project.has_probes,
		"managed releases require /healthz and /readyz; implement the readiness lifecycle and \
		 declare serve.probes = true"
	);
	let (runtime_config, references) = runtime::kamal(c, &project, hosts, private)?;
	fs::write(dir.join("release.yml"), runtime_config)?;
	fs::write(dir.join("db-init.sh"), runtime::DATABASE_INIT)?;
	fs::write(dir.join("nats.conf"), runtime::NATS_CONFIG)?;
	fs::create_dir_all(dir.join(".kamal"))?;
	fs::write(dir.join(".kamal/secrets"), references)?;
	let mut env = secrets(config, private)?;
	for name in project.secrets.iter().chain(&project.urls) {
		ensure!(
			env.contains_key(name) || std::env::var(name).is_ok_and(|v| !v.is_empty()),
			"set required application secret {name}"
		);
	}
	env.insert("TUNNEL_TOKEN".into(), output(dir, "tunnel_token")?);
	Ok(env)
}

fn recover_previous(
	dir: &Path,
	c: &InfraConfig,
	hosts: &[String],
	history: &[Release],
	env: &BTreeMap<String, String>,
	reservation: &Reservation,
) -> Result<()> {
	let ip = &hosts[0];
	let actual = remote_history(ip, &c.name)?;
	if let Some(previous) = history.last()
		&& actual == previous.migrations
	{
		prepare_recorded(dir, c, hosts, previous, env, reservation)?;
		execute(
			dir,
			&["app", "boot", "--version", &previous.version],
			env,
			reservation,
		)?;
		verify_release(
			hosts,
			c,
			&previous.version,
			&previous.migrations,
			Some(previous),
		)?;
		record(
			ip,
			c,
			&reservation.owner,
			"previous-restored",
			&previous.version,
		)?;
	} else {
		stop(hosts, c)?;
	}
	Ok(())
}

fn release_version(args: &ReleaseArgs) -> Result<&String> {
	let version = args
		.version
		.as_ref()
		.or(args.rollback.as_ref())
		.context("pass --version or --rollback")?;
	super::build::validate_version(version)?;
	Ok(version)
}

fn target_release(
	args: &ReleaseArgs,
	history: &[Release],
	version: &str,
	local_hash: &str,
) -> Result<Option<Release>> {
	let target = if args.rollback.is_some() || args.recover {
		Some(
			history
				.iter()
				.find(|r| r.version == version)
				.cloned()
				.context("version is absent from shared release history")?,
		)
	} else {
		None
	};
	if target.is_none() {
		ensure!(
			!history.iter().any(|r| r.version == version),
			"tag already recorded; use --recover or a new tag"
		);
		ensure!(
			!history.is_empty() || args.setup,
			"first release requires --setup"
		);
		if let Some(previous) = history.last() {
			ensure!(
				previous.migrations == local_hash || args.maintenance,
				"schema changes require --maintenance"
			);
		}
	}
	Ok(target)
}

fn release_evidence(
	args: &ReleaseArgs,
	target: Option<&Release>,
	history: &[Release],
	local_hash: &str,
) -> Result<Option<super::verified::Descriptor>> {
	let evidence = match (&args.verified_descriptor, &args.descriptor_sha256) {
		(Some(path), Some(pin)) => {
			ensure!(
				target.is_none(),
				"recorded releases use their recorded evidence"
			);
			Some(super::verified::read(path, pin)?)
		}
		(None, None) => target.and_then(|r| r.evidence.clone()),
		_ => anyhow::bail!("descriptor and independent pin are both required"),
	};
	ensure!(
		!history.iter().any(|r| r.evidence.is_some()) || evidence.is_some(),
		"verified release history cannot downgrade to legacy evidence"
	);
	if let Some(evidence) = &evidence {
		ensure!(
			evidence.migrations == *target.map_or(local_hash, |r| &r.migrations),
			"verified source schema differs from release schema"
		);
	}
	Ok(evidence)
}

pub(super) fn run(args: &ReleaseArgs) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let c = managed_settings(&config)?;
	let version = release_version(args)?;
	let dir = directory(&root, c);
	generate(&dir, c, false)?;
	let _local = lock(&dir)?;
	let hosts = hosts(&dir)?;
	ensure!(
		hosts.len() == usize::from(c.instances),
		"apply the configured host count before release"
	);
	let ip = &hosts[0];
	applied_bindings(&dir, c)?;
	let private = output(&dir, "private_ip")?;
	ensure!(
		private.parse::<std::net::Ipv4Addr>().is_ok(),
		"invalid primary private address"
	);
	let reservation = Reservation::acquire(ip, &c.name)?;
	let reset: serde_json::Value =
		serde_json::from_slice(&remote::read(ip, &c.name, "derived-reset-required.json")?)?;
	ensure!(
		reset.is_array(),
		"database restore has unfinished relay/cache recovery; retry `lk infra restore --snapshot \
		 <id> --yes` before starting applications"
	);
	let mut history = history(ip, c)?;
	let local_schema = schema(&root, &config)?;
	let local_hash = history_fingerprint(local_schema.clone())?;
	let target = target_release(args, &history, version, &local_hash)?;
	let evidence = release_evidence(args, target.as_ref(), &history, &local_hash)?;
	let env = runtime_files(&dir, c, &root, &config, &hosts, &private)?;
	if c.backups.enabled && args.setup {
		super::recovery::credentials()?;
	}
	record(ip, c, &reservation.owner, "preparing", version)?;
	let mut flight = Flight {
		ip,
		config: c,
		owner: &reservation.owner,
		version,
		finished: false,
	};
	if args.setup {
		bootstrap(&dir, c, &hosts, &env, &reservation)?;
	}
	if let Some(target) = &target {
		prepare_recorded(&dir, c, &hosts, target, &env, &reservation)?;
	} else {
		execute(&dir, &["registry", "login"], &env, &reservation)?;
		execute(
			&dir,
			&["build", "pull", "--version", version],
			&env,
			&reservation,
		)?;
		verify_schema_labels(&hosts, c, version, &local_hash)?;
	}
	if let Some(evidence) = &evidence {
		for host in &hosts {
			evidence.check_remote(
				host,
				&format!("{}:{version}", image(c)),
				&evidence.migrations,
			)?;
		}
	}
	execute(&dir, &["proxy", "boot"], &env, &reservation)?;
	if !args.setup && target.is_none() {
		let actual = remote_history(ip, &c.name)?;
		let previous = &history.last().context("no previous release")?.migrations;
		ensure!(
			actual == *previous || (args.maintenance && actual == local_hash),
			"database drifted from shared history; investigate before a new release"
		);
	}
	if args.maintenance || args.rollback_schema {
		stop(&hosts, c)?;
		if !history.is_empty() && c.backups.enabled {
			super::recovery::invoke(ip, c, "backup", "latest", false, Some(&reservation.owner))?;
		}
	}
	if let Some(target) = &target {
		if args.rollback_schema {
			rollback_schema(ip, &private, &root, &config, &target.schema)?;
		}
		ensure!(
			remote_history(ip, &c.name)? == target.migrations,
			"database differs from target; use --rollback-schema --yes with the newer migration \
			 files, or recover explicitly"
		);
	}
	let result = execute(
		&dir,
		&["app", "boot", "--version", version],
		&env,
		&reservation,
	);
	if let Err(error) = result {
		record(ip, c, &reservation.owner, "failed", version)?;
		if recover_previous(&dir, c, &hosts, &history, &env, &reservation).is_err() {
			eprintln!(
				"Automatic recovery did not complete; inspect shared status and recover \
				 explicitly."
			);
		}
		return Err(error);
	}
	let expected = target
		.as_ref()
		.map_or(&local_hash, |target| &target.migrations);
	let verified = (|| {
		let digest = verify_release(&hosts, c, version, expected, target.as_ref())?;
		if let Some(evidence) = &evidence {
			for host in &hosts {
				evidence.check_remote(host, &digest, expected)?;
			}
		}
		anyhow::Ok(digest)
	})();
	let image = flight.checked(verified, &hosts)?;
	history.push(Release {
		evidence,
		version: version.clone(),
		migrations: expected.clone(),
		schema: target.as_ref().map_or(local_schema, |t| t.schema.clone()),
		image_digest: Some(image),
		completed_at: Some(chrono::Utc::now().to_rfc3339()),
	});
	let bytes = serde_json::to_vec_pretty(&history)?;
	remote::write(
		ip,
		&format!("{}/releases.json", remote::base(&c.name)),
		&bytes,
	)?;
	fs::write(dir.join("releases.json"), &bytes)?;
	record(ip, c, &reservation.owner, "complete", version)?;
	if args.setup && c.backups.enabled {
		super::recovery::install(ip, c, &output(&dir, "backup_endpoint")?, &reservation)?;
		super::recovery::invoke(ip, c, "backup", "latest", false, Some(&reservation.owner))?;
	}
	execute(&dir, &["prune", "all"], &env, &reservation)?;
	println!(
		"Released {version} on {} host(s) at https://{}",
		hosts.len(),
		c.domain
	);
	flight.finished = true;
	Ok(())
}

pub(super) fn prepare() -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let c = managed_settings(&config)?;
	let dir = directory(&root, c);
	generate(&dir, c, false)?;
	let _local = lock(&dir)?;
	applied_bindings(&dir, c)?;
	let hosts = hosts(&dir)?;
	ensure!(
		hosts.len() == usize::from(c.instances),
		"apply the configured hosts before preparing services"
	);
	let private = output(&dir, "private_ip")?;
	ensure!(
		private.parse::<std::net::Ipv4Addr>().is_ok(),
		"invalid private IP"
	);
	let ip = &hosts[0];
	let reservation = Reservation::acquire(ip, &c.name)?;
	let env = runtime_files(&dir, c, &root, &config, &hosts, &private)?;
	record(ip, c, &reservation.owner, "preparing-services", "services")?;
	let mut flight = Flight {
		ip,
		config: c,
		owner: &reservation.owner,
		version: "services",
		finished: false,
	};
	bootstrap(&dir, c, &hosts, &env, &reservation)?;
	execute(&dir, &["proxy", "boot"], &env, &reservation)?;
	if c.backups.enabled {
		super::recovery::install(ip, c, &output(&dir, "backup_endpoint")?, &reservation)?;
	}
	record(ip, c, &reservation.owner, "services-ready", "services")?;
	flight.finished = true;
	println!(
		"Private services are prepared. No application or initial backup was started. Restore the \
		 chosen snapshot before `lk deploy --recover --version <recorded-tag>`."
	);
	Ok(())
}

fn rollback_schema(
	ip: &str,
	private: &str,
	root: &Path,
	config: &LeptoskitConfig,
	target: &[(String, String)],
) -> Result<()> {
	let name = &config
		.infra
		.as_ref()
		.context("missing infrastructure")?
		.name;
	let clients = remote::execute(
		ip,
		&format!(
			"docker exec {name}-db psql -U leptoskit_admin -d postgres -At -c \"SELECT count(*) \
			 FROM pg_stat_activity WHERE datname = 'leptoskit'\""
		),
		None,
	)?;
	ensure!(
		String::from_utf8(clients)?.trim() == "0",
		"database still has clients; stop external writers before down migrations"
	);
	let db_config = config.db.as_ref().context("missing database")?;
	let migrations = leptoskit_db::load_migrations(&root.join(&db_config.migrations))?;
	ensure!(
		target.len() <= migrations.len()
			&& migrations
				.iter()
				.take(target.len())
				.zip(target)
				.all(|(m, (name, sum))| m.name == *name && m.checksum == *sum),
		"target migration prefix is not present in this checkout"
	);
	let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
	let port = listener.local_addr()?.port();
	drop(listener);
	let child = Command::new("ssh")
		.args([
			"-o",
			"BatchMode=yes",
			"-o",
			"ExitOnForwardFailure=yes",
			"-N",
			"-L",
			&format!("127.0.0.1:{port}:{private}:5432"),
			&format!("root@{ip}"),
		])
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()?;
	struct Tunnel(std::process::Child);
	impl Drop for Tunnel {
		fn drop(&mut self) {
			let _ = self.0.kill();
			let _ = self.0.wait();
		}
	}
	let _tunnel = Tunnel(child);
	let password = std::env::var("DATABASE_PASSWORD").context("set DATABASE_PASSWORD")?;
	let mut url = url::Url::parse(&format!(
		"postgres://leptoskit@127.0.0.1:{port}/leptoskit?sslmode=disable"
	))?;
	url.set_password(Some(&password))
		.map_err(|()| anyhow::anyhow!("invalid database password"))?;
	tokio::runtime::Runtime::new()?.block_on(async {
		let mut connection = None;
		for _ in 0..50 {
			match leptoskit_db::Db::connect(url.as_str()).await {
				Ok(db) => {
					connection = Some(db);
					break;
				}
				Err(_) => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
			}
		}
		let db = connection.context("database tunnel did not become ready")?;
		let applied = db.applied_migrations().await?;
		ensure!(
			applied.len() >= target.len(),
			"database is older than target schema"
		);
		if applied.len() > target.len() {
			db.rollback(&migrations, applied.len() - target.len())
				.await?;
		}
		anyhow::Ok(())
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	fn args() -> ReleaseArgs {
		ReleaseArgs {
			version: None,
			rollback: None,
			setup: false,
			maintenance: false,
			recover: false,
			rollback_schema: false,
			yes: false,
		}
	}

	fn recorded(version: &str, migrations: &str) -> Release {
		Release {
			version: version.into(),
			migrations: migrations.into(),
			schema: Vec::new(),
			image_digest: Some(format!("sha256:{}", "a".repeat(64))),
			completed_at: None,
		}
	}

	fn infra() -> InfraConfig {
		let raw = format!(
			"[app]\nname = 'notes'\n[db]\ndialect = 'postgres'\n[infra]\nname = \
			 'notes-production'\ndomain = 'app.example.com'\nlocation = 'hel1'\nserver_type = \
			 'cx23'\nssh_key_ids = [123]\nssh_cidrs = ['192.0.2.1/32']\naccount_id = \
			 '{}'\nzone_id = '{}'\nimage = 'ghcr.io/team/notes'\nregistry = \
			 'ghcr.io'\nregistry_username = 'team'\nupdates_domain = \
			 'updates.example.com'\nupdates_bucket = 'notes-releases'\nbackup_bucket = \
			 'notes-backups'",
			"a".repeat(32),
			"b".repeat(32)
		);
		LeptoskitConfig::from_toml(&raw, Path::new("."))
			.unwrap()
			.infra
			.unwrap()
	}

	#[test]
	fn versions_come_from_either_flag_and_are_validated() {
		let mut flags = args();
		flags.version = Some("v1".into());
		assert_eq!(release_version(&flags).unwrap(), "v1");
		flags.version = None;
		flags.rollback = Some("v0".into());
		assert_eq!(release_version(&flags).unwrap(), "v0");
		flags.rollback = None;
		assert!(
			release_version(&flags)
				.unwrap_err()
				.to_string()
				.contains("pass --version or --rollback")
		);
		flags.version = Some("latest".into());
		assert!(
			release_version(&flags)
				.unwrap_err()
				.to_string()
				.contains("use a unique immutable image tag")
		);
		flags.version = Some("v/1".into());
		assert!(release_version(&flags).is_err());
	}

	#[test]
	fn a_new_tag_must_beat_setup_history_and_schema_checks() {
		let mut flags = args();
		flags.version = Some("v1".into());
		assert!(
			target_release(&flags, &[], "v1", "m1")
				.unwrap_err()
				.to_string()
				.contains("first release requires --setup")
		);
		flags.setup = true;
		assert!(target_release(&flags, &[], "v1", "m1").unwrap().is_none());

		let history = [recorded("v1", "m1")];
		assert!(
			target_release(&flags, &history, "v1", "m1")
				.unwrap_err()
				.to_string()
				.contains("tag already recorded")
		);
		flags.setup = false;
		assert!(
			target_release(&flags, &history, "v2", "m2")
				.unwrap_err()
				.to_string()
				.contains("schema changes require --maintenance")
		);
		flags.maintenance = true;
		assert!(
			target_release(&flags, &history, "v2", "m2")
				.unwrap()
				.is_none()
		);
		flags.version = Some("v2".into());
		assert!(
			target_release(&flags, &history, "v2", "m2")
				.unwrap()
				.is_none()
		);
	}

	#[test]
	fn rollbacks_and_recoveries_must_name_a_recorded_version() {
		let history = [recorded("v1", "m1")];
		let mut flags = args();
		flags.rollback = Some("v1".into());
		let target = target_release(&flags, &history, "v1", "m1")
			.unwrap()
			.unwrap();
		assert_eq!(target.version, "v1");
		flags.rollback = Some("v9".into());
		assert!(
			target_release(&flags, &history, "v9", "m1")
				.unwrap_err()
				.to_string()
				.contains("version is absent from shared release history")
		);
		flags.rollback = None;
		flags.recover = true;
		flags.version = Some("v1".into());
		assert_eq!(
			target_release(&flags, &history, "v1", "m1")
				.unwrap()
				.unwrap()
				.version,
			"v1"
		);
	}

	#[test]
	fn images_are_prefixed_with_the_registry_exactly_once() {
		let mut config = infra();
		config.image = "team/notes".into();
		assert_eq!(image(&config), "ghcr.io/team/notes");
		config.image = "ghcr.io/team/notes".into();
		assert_eq!(image(&config), "ghcr.io/team/notes");
		config.image = "ghcr.io/team/other".into();
		assert_eq!(image(&config), "ghcr.io/team/other");
	}
}
