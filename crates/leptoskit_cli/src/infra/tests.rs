use super::*;
#[cfg(unix)]
use crate::jobs::postgres;

fn config() -> InfraConfig {
	InfraConfig {
		external: None,
		control_url: None,
		hosted_state: false,
		compute: leptoskit_config::infra::ComputeProvider::default(),
		instances: 1,
		services: leptoskit_config::infra::InfraServices::default(),
		postgres: leptoskit_config::infra::InfraPostgres::default(),
		plugins: leptoskit_config::infra::InfraPlugins::default(),
		backups: leptoskit_config::infra::InfraBackups::default(),
		b2: None,
		budget: None,
		name: "notes-production".into(),
		domain: "app.example.com".into(),
		location: "hel1".into(),
		server_type: "cx23".into(),
		ssh_key_ids: vec![123],
		ssh_cidrs: vec!["192.0.2.1/32".into()],
		account_id: "a".repeat(32),
		zone_id: "b".repeat(32),
		updates_domain: "updates.example.com".into(),
		backup_bucket: "notes-backups".into(),
		updates_bucket: "notes-releases".into(),
		image: "ghcr.io/team/notes".into(),
		registry: "ghcr.io".into(),
		registry_username: "team".into(),
		secrets: vec!["AUTH_KEY".into()],
		state: None,
	}
}

fn external_config() -> InfraConfig {
	let mut config = config();
	config.external = Some(leptoskit_config::infra::InfraExternal { origin_port: 4701 });
	config.compute = leptoskit_config::infra::ComputeProvider::Digitalocean;
	config.location = "lon1".into();
	config.server_type = "s-1vcpu-1gb".into();
	config.backups.enabled = false;
	config.updates_domain.clear();
	config.updates_bucket.clear();
	config.backup_bucket.clear();
	config.image.clear();
	config.registry.clear();
	config.registry_username.clear();
	config.secrets.clear();
	config
}

#[test]
fn external_hosts_reuse_the_provider_and_edge_resources_without_a_managed_database() {
	let infra = external_config();
	assert!(infra.problem().is_none());
	let value = resources(&infra);
	assert_eq!(
		value["resource"]["digitalocean_droplet"]["app"]["lifecycle"]["prevent_destroy"],
		true
	);
	assert_eq!(
		value["resource"]["digitalocean_droplet"]["app"]["size"],
		"s-1vcpu-1gb"
	);
	assert!(
		!value["resource"]
			.as_object()
			.unwrap()
			.contains_key("hcloud_server")
	);
	assert!(value["resource"]["cloudflare_r2_bucket"].is_null());
	assert!(value["resource"]["cloudflare_r2_custom_domain"].is_null());
	assert!(value["output"]["updates_bucket"].is_null());
	assert!(value["output"]["backup_bucket"].is_null());
	assert_eq!(
		value["resource"]["cloudflare_zero_trust_tunnel_cloudflared_config"]["app"]["config"]
			["ingress"][0]["service"],
		"http://127.0.0.1:4701"
	);
	assert_eq!(value["output"]["tunnel_token"]["sensitive"], true);
	assert_eq!(
		value["resource"]["digitalocean_firewall"]["app"]["inbound_rule"]
			.as_array()
			.unwrap()
			.len(),
		1
	);
	let cloud_init = value["resource"]["digitalocean_droplet"]["app"]["user_data"]
		.as_str()
		.unwrap();
	assert!(cloud_init.contains("PermitRootLogin prohibit-password"));
	assert!(!cloud_init.contains("docker"));
	assert!(cloud_init.contains("  - python3\n"));
	let config = LeptoskitConfig {
		infra: Some(infra),
		..LeptoskitConfig::default()
	};
	assert!(settings(&config).is_ok());
	assert!(managed_settings(&config).is_err());
	assert!(InfraCommand::Plan.supports_external());
	assert!(!InfraCommand::Prepare.supports_external());
}

#[test]
fn external_hosts_reject_runtime_features_instead_of_silently_ignoring_them() {
	let mut config = external_config();
	config.external.as_mut().unwrap().origin_port = 0;
	assert!(config.problem().is_some());
	config.external.as_mut().unwrap().origin_port = 4701;
	config.backups.enabled = true;
	assert!(config.problem().is_some());
	config.backups.enabled = false;
	config.services.cache = true;
	assert!(config.problem().is_some());
	config.services.cache = false;
	config.image = "ghcr.io/team/app".into();
	assert!(config.problem().is_some());
	config.image.clear();
	config.hosted_state = true;
	assert!(config.problem().is_some());
	config.hosted_state = false;
	config.backup_bucket = "pilot-backups".into();
	assert!(config.problem().is_none());
	let value = resources(&config);
	assert_eq!(
		value["resource"]["cloudflare_r2_bucket"]["backups"]["name"],
		"pilot-backups"
	);
	assert!(value["resource"]["cloudflare_r2_bucket"]["updates"].is_null());
	assert!(value["output"]["backup_endpoint"].is_object());
}

#[test]
fn external_hetzner_hosts_do_not_emit_empty_digitalocean_resource_blocks() {
	let mut config = external_config();
	config.compute = leptoskit_config::infra::ComputeProvider::Hetzner;
	config.location = "hel1".into();
	config.server_type = "cx23".into();
	assert!(config.problem().is_none());
	let value = resources(&config);
	let resources = value["resource"].as_object().unwrap();
	assert!(!resources.contains_key("digitalocean_firewall"));
	assert!(!resources.contains_key("digitalocean_droplet"));
	assert!(resources.values().all(serde_json::Value::is_object));
	assert_eq!(
		resources["hcloud_server"]["app"]["lifecycle"]["prevent_destroy"],
		true
	);
}

#[test]
fn the_worker_origin_cannot_be_redirected_by_repository_connection_metadata() {
	let mut config = config();
	config.plugins.analytics = Some(leptoskit_config::infra::InfraAnalytics::default());
	config.control_url = Some("https://another-console.example".into());
	let (endpoint, protocol) =
		runtime::analytics_sink(&config, Some("https://trusted-worker.example")).unwrap();
	assert_eq!(
		endpoint,
		"https://trusted-worker.example/api/v1/projects/notes-production/analytics/ingest"
	);
	assert_eq!(protocol, "leptoskit");
}

#[test]
fn resources_are_pinned_and_protected() {
	let c = config();
	let resources = resources(&c);
	insta::assert_json_snapshot!("managed_resources", resources);
	assert_eq!(
		resources["resource"]["hcloud_server"]["app"]["lifecycle"]["prevent_destroy"],
		true
	);
	assert_eq!(resources["output"]["tunnel_token"]["sensitive"], true);
}

#[test]
fn digitalocean_and_b2_preserve_data_and_use_the_selected_providers() {
	let mut c = config();
	c.compute = leptoskit_config::infra::ComputeProvider::Digitalocean;
	c.location = "ams3".into();
	c.server_type = "s-1vcpu-2gb".into();
	c.instances = 2;
	c.services.relay = true;
	c.b2 = Some(leptoskit_config::infra::InfraB2 {
		region: "us-west-004".into(),
	});
	let value = resources(&c);
	assert!(value["resource"]["hcloud_server"].is_null());
	assert_eq!(
		value["resource"]["digitalocean_droplet"]["app"]["lifecycle"]["prevent_destroy"],
		true
	);
	assert_eq!(
		value["resource"]["b2_bucket"]["backups"]["bucket_type"],
		"allPrivate"
	);
	assert!(value["resource"]["cloudflare_r2_bucket"]["backups"].is_null());
	insta::assert_json_snapshot!("managed_digitalocean_b2", value);
}

#[test]
fn multi_host_runtime_has_shared_services_and_sequential_rollouts() {
	let root = tempfile::tempdir().unwrap();
	fs::create_dir(root.path().join("src")).unwrap();
	fs::write(
		root.path().join("Cargo.toml"),
		"[package]\nname='notes'\nversion='0.1.0'\n",
	)
	.unwrap();
	fs::write(
		root.path().join("src/main.rs"),
		"fn main() { /* readyz */ }",
	)
	.unwrap();
	let config = LeptoskitConfig::from_toml(
		"[app]\nname='notes'\nrender='ssr'\n[serve]\nprobes=true\n[db]\ndialect='postgres'\n[rpc.\
		 live]\nrelay_url_env='NATS_URL'\n",
		root.path(),
	)
	.unwrap();
	let project = crate::deploy::Project::read(root.path(), &config).unwrap();
	let mut c = self::config();
	c.instances = 2;
	c.services.relay = true;
	c.services.cache = true;
	let (text, secrets) = runtime::kamal(
		&c,
		&project,
		&["192.0.2.10".into(), "192.0.2.11".into()],
		"10.42.0.10",
	)
	.unwrap();
	let value: serde_json::Value = serde_json::from_str(&text).unwrap();
	assert_eq!(value["boot"]["limit"], 1);
	assert_eq!(
		value["servers"]["web"]["hosts"].as_array().unwrap().len(),
		2
	);
	assert_eq!(
		value["accessories"]["relay"]["port"],
		"10.42.0.10:4222:4222"
	);
	assert_eq!(
		value["accessories"]["cache"]["port"],
		"10.42.0.10:6379:6379"
	);
	assert!(secrets.contains("NATS_TOKEN=$NATS_TOKEN"));
	insta::assert_snapshot!("managed_multi_host", text);
}

#[test]
fn ci_workflow_builds_before_deploying_and_uses_shared_state() {
	let root = tempfile::tempdir().unwrap();
	fs::create_dir(root.path().join("src")).unwrap();
	fs::write(
		root.path().join("Cargo.toml"),
		"[package]\nname='notes'\nversion='0.1.0'\n",
	)
	.unwrap();
	fs::write(
		root.path().join("src/main.rs"),
		"fn main() { /* readyz */ }",
	)
	.unwrap();
	let mut config = LeptoskitConfig::from_toml(
		"[app]\nname='notes'\nrender='ssr'\n[serve]\nprobes=true\n[db]\ndialect='postgres'\n",
		root.path(),
	)
	.unwrap();
	let mut infra = self::config();
	infra.state = Some(leptoskit_config::infra::InfraState {
		bucket: "independent-state".into(),
		key: "production.tfstate".into(),
		region: "eu-west-1".into(),
	});
	config.infra = Some(infra);
	let project = crate::deploy::Project::read(root.path(), &config).unwrap();
	insta::assert_json_snapshot!("managed_ci", ci::workflow(&config, &project).unwrap());
	config.infra.as_mut().unwrap().plugins.analytics =
		Some(leptoskit_config::infra::InfraAnalytics::default());
	let workflow = ci::workflow(&config, &project).unwrap();
	assert_eq!(
		workflow["jobs"]["deploy"]["steps"][1]["env"]["LEPTOSKIT_ANALYTICS_READ_KEY"],
		"${{ secrets.LEPTOSKIT_ANALYTICS_READ_KEY }}"
	);
	assert!(workflow["jobs"]["build"]["steps"][1]["env"]["LEPTOSKIT_ANALYTICS_READ_KEY"].is_null());
}

#[test]
fn unsafe_access_and_shared_storage_are_rejected() {
	let mut c = config();
	c.ssh_cidrs = vec!["0.0.0.0/0".into()];
	assert!(c.problem().is_some());
	c.ssh_cidrs = vec!["::/0".into()];
	assert!(c.problem().is_some());
	c.ssh_cidrs = vec!["2001:db8::/64".into()];
	assert!(c.problem().is_none());
	c.updates_bucket.clone_from(&c.backup_bucket);
	assert!(c.problem().is_some());
}

#[test]
fn generated_edits_are_never_silently_overwritten() {
	let root = tempfile::tempdir().unwrap();
	let dir = root.path();
	let mut c = config();
	generate(dir, &c, false).unwrap();
	generate(dir, &c, false).unwrap();
	fs::write(dir.join("main.tf.json"), "edited").unwrap();
	assert!(generate(dir, &c, false).is_err());
	generate(dir, &c, true).unwrap();
	let resources_before = fs::read(dir.join("main.tf.json")).unwrap();
	fs::write(dir.join("plugins.json"), "edited").unwrap();
	assert!(generate(dir, &c, false).is_err());
	assert_eq!(
		fs::read(dir.join("main.tf.json")).unwrap(),
		resources_before
	);
	generate(dir, &c, true).unwrap();
	c.state = Some(leptoskit_config::infra::InfraState {
		bucket: "independent-state".into(),
		key: "prod/state".into(),
		region: "eu-west-1".into(),
	});
	assert_eq!(
		resources(&c)["terraform"]["backend"]["s3"]["use_lockfile"],
		true
	);
}

#[test]
fn saved_plans_detect_tampering_and_configuration_changes() {
	let root = tempfile::tempdir().unwrap();
	generate(root.path(), &config(), false).unwrap();
	assert!(plan_fingerprint(root.path()).is_err());
	fs::write(root.path().join("leptoskit.plan"), "plan").unwrap();
	fs::write(root.path().join(".terraform.lock.hcl"), "lock").unwrap();
	let first = plan_fingerprint(root.path()).unwrap();
	fs::write(root.path().join("plugins.json"), "edited plugin bindings").unwrap();
	assert_ne!(first, plan_fingerprint(root.path()).unwrap());
	fs::write(root.path().join("leptoskit.plan"), "modified").unwrap();
	assert_ne!(first, plan_fingerprint(root.path()).unwrap());
	let _held = lock(root.path()).unwrap();
	assert!(lock(root.path()).is_err());
}

#[test]
fn release_configuration_uses_private_services_and_secret_references() {
	let root = tempfile::tempdir().unwrap();
	fs::create_dir(root.path().join("src")).unwrap();
	fs::write(
		root.path().join("Cargo.toml"),
		"[package]\nname = 'notes'\nversion = '0.1.0'\n",
	)
	.unwrap();
	fs::write(
		root.path().join("src/main.rs"),
		"fn main() { /* /readyz /healthz */ }",
	)
	.unwrap();
	let config = LeptoskitConfig::from_toml(
		"[app]\nname='notes'\nrender='ssr'\n[serve]\nprobes=true\n[db]\ndialect='postgres'\n",
		root.path(),
	)
	.unwrap();
	let project = crate::deploy::Project::read(root.path(), &config).unwrap();
	let (yaml, secrets) = runtime::kamal(
		&self::config(),
		&project,
		&["192.0.2.10".into()],
		"10.42.0.10",
	)
	.unwrap();
	insta::assert_snapshot!("managed_kamal", yaml);
	insta::assert_snapshot!("managed_secret_references", secrets);
	assert!(yaml.contains("10.42.0.10:5432:5432"));
	assert!(secrets.contains("DATABASE_URL=$DATABASE_URL"));
	let mut c = self::config();
	c.plugins.analytics = Some(leptoskit_config::infra::InfraAnalytics {
		browser_routes: vec!["/notes/{id}".into()],
		..Default::default()
	});
	let (analytics_yaml, references) =
		runtime::kamal(&c, &project, &["192.0.2.10".into()], "10.42.0.10").unwrap();
	insta::assert_snapshot!("managed_analytics", analytics_yaml);
	assert!(references.contains("LEPTOSKIT_ANALYTICS_READ_KEY=$LEPTOSKIT_ANALYTICS_READ_KEY"));
	assert!(references.contains("LEPTOSKIT_RELEASE=$LEPTOSKIT_RELEASE"));
	assert_eq!(resources(&c), resources(&self::config())); // No new cloud resources.
	let manifest = plugins::manifest(&c);
	assert_eq!(manifest["plugins"][0]["enabled"], true);
	assert_eq!(manifest["plugins"][0]["additional_containers"], 0);
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(unix)]
async fn database_bootstrap_gives_the_app_ownership_without_superuser() {
	use leptoskit_db::Connection as _;
	use leptoskit_db::Read as _;
	let Some(cluster) = postgres::PostgresCluster::start() else {
		return;
	};
	let mut url = url::Url::parse(&cluster.database()).unwrap();
	let admin = leptoskit_db::Db::connect(url.as_str()).await.unwrap();
	admin
		.execute(leptoskit_db::Statement {
			sql: "CREATE DATABASE leptoskit".into(),
			args: vec![],
		})
		.await
		.unwrap();
	url.set_path("/leptoskit");
	let directory = tempfile::tempdir().unwrap();
	let script = directory.path().join("init.sh");
	fs::write(&script, runtime::DATABASE_INIT).unwrap();
	let result = Command::new("bash")
		.arg(script)
		.env("POSTGRES_USER", "postgres")
		.env("POSTGRES_DB", url.as_str())
		.env("DATABASE_PASSWORD", "fixture's-password")
		.output()
		.unwrap();
	assert!(
		result.status.success(),
		"{}",
		String::from_utf8_lossy(&result.stderr)
	);
	url.set_username("leptoskit").unwrap();
	url.set_password(Some("fixture's-password")).unwrap();
	let app = leptoskit_db::Db::connect(url.as_str()).await.unwrap();
	let rows = app
		.fetch(leptoskit_db::Statement {
			sql: "SELECT rolsuper FROM pg_roles WHERE rolname = current_user".into(),
			args: vec![],
		})
		.await
		.unwrap();
	assert_eq!(rows[0][0], leptoskit_db::Value::Bool(false));
	let migrations =
		[
			leptoskit_db::Migration::new("first", "CREATE TABLE owned (id INTEGER PRIMARY KEY)")
				.with_down("DROP TABLE owned"),
		];
	app.migrate(&migrations).await.unwrap();
	app.rollback(&migrations, 1).await.unwrap();
}

#[test]
#[cfg(unix)]
fn encrypted_archives_restore_real_postgres_and_preserve_the_previous_database() {
	if Command::new("restic").arg("version").output().is_err() {
		assert!(
			std::env::var_os("LEPTOSKIT_REQUIRE_RESTIC").is_none(),
			"install restic to run required recovery tests"
		);
		return;
	}
	let Some(cluster) = postgres::PostgresCluster::start() else {
		return;
	};
	let directory = tempfile::tempdir().unwrap();
	fs::create_dir(directory.path().join("control")).unwrap();
	let worker = directory.path().join("control/recovery.py");
	let test = directory.path().join("test.py");
	fs::write(&worker, include_bytes!("recovery.py")).unwrap();
	fs::write(&test, include_bytes!("recovery_tests.py")).unwrap();
	let output = Command::new("python3")
		.arg(test)
		.arg(worker)
		.arg(cluster.database())
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"{}\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}

/// The services fixture with a server that answers the probes.
fn services_project(root: &Path, trust: &str) -> crate::deploy::Project {
	let config = crate::deploy::tests::project_with_services(
		root,
		crate::deploy::tests::Db::Postgres,
		trust,
	);
	crate::deploy::Project::read(root, &config).unwrap()
}

#[test]
fn managed_services_run_as_roles_with_their_own_secrets() {
	let root = tempfile::tempdir().unwrap();
	let project = services_project(root.path(), "first-party");
	let (yaml, secrets) =
		runtime::kamal(&config(), &project, &["192.0.2.10".into()], "10.42.0.10").unwrap();
	insta::assert_snapshot!("managed_services", yaml);
	insta::assert_snapshot!("managed_services_secret_references", secrets);
	let value: serde_json::Value = serde_json::from_str(&yaml).unwrap();

	// No service inherits the app's environment: it's the web role's alone.
	assert!(value.get("env").is_none());
	let web = &value["servers"]["web"];
	assert_eq!(web["options"]["network-alias"], "notes-production-app");
	let web_secrets = web["env"]["secret"].as_array().unwrap();
	assert!(web_secrets.contains(&serde_json::json!("DATABASE_URL")));
	assert!(web_secrets.contains(&serde_json::json!("LEPTOSKIT_SERVICE_ANALYTICS_TOKEN")));
	assert!(web_secrets.contains(&serde_json::json!("ANALYTICS_AUTHORIZATION")));
	assert_eq!(
		web["env"]["clear"]["LEPTOSKIT_SERVICE_ANALYTICS_URL"],
		"http://notes-production-analytics:3100"
	);

	// Each service gets its own token and database under the contract's
	// names, and nothing of the app's.
	let analytics = &value["servers"]["service-analytics"];
	assert_eq!(analytics["cmd"], "/app/services/analytics");
	assert_eq!(analytics["hosts"], serde_json::json!(["192.0.2.10"]));
	assert_eq!(
		analytics["env"]["secret"],
		serde_json::json!([
			"LEPTOSKIT_SERVICE_TOKEN:LEPTOSKIT_SERVICE_ANALYTICS_TOKEN",
			"DATABASE_URL:LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_URL",
			"ANALYTICS_GEOIP_KEY"
		])
	);
	assert_eq!(
		analytics["proxy"]["path_prefix"],
		"/_leptoskit/services/analytics,/_analytics"
	);
	assert_eq!(analytics["proxy"]["strip_path_prefix"], false);
	assert_eq!(
		analytics["options"]["volume"],
		serde_json::json!([
			"/var/lib/leptoskit/notes-production/services/analytics/exports:/volumes/exports"
		])
	);
	let cards = &value["servers"]["service-cards"];
	assert!(cards.get("proxy").is_none());
	assert_eq!(cards["options"]["network-alias"], "notes-production-cards");

	// The app's token-bearing binding is written in terms of the token, and
	// every secret is listed once.
	assert!(
		secrets.contains("ANALYTICS_AUTHORIZATION=Bearer ${LEPTOSKIT_SERVICE_ANALYTICS_TOKEN}\n")
	);
	assert!(!secrets.contains("ANALYTICS_AUTHORIZATION=$ANALYTICS_AUTHORIZATION"));
	assert!(secrets.contains(
		"LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_URL=$LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_URL"
	));
	let lines: Vec<&str> = secrets.lines().collect();
	let unique: std::collections::BTreeSet<&str> = lines
		.iter()
		.map(|line| line.split('=').next().unwrap())
		.collect();
	assert_eq!(lines.len(), unique.len(), "{secrets}");
}

#[test]
fn managed_services_refuse_what_the_hosts_cannot_do() {
	let root = tempfile::tempdir().unwrap();

	let isolated = services_project(root.path(), "isolated");
	let error = services::check(&config(), &isolated).unwrap_err();
	assert!(error.to_string().contains("services.analytics"), "{error}");
	assert!(error.to_string().contains("first-party"), "{error}");

	let project = services_project(root.path(), "first-party");
	let mut several = config();
	several.instances = 2;
	let error = services::check(&several, &project).unwrap_err();
	assert!(error.to_string().contains("instances = 1"), "{error}");

	let mut tight = config();
	tight.postgres.max_connections = 30;
	let error = services::check(&tight, &project).unwrap_err();
	assert!(
		error.to_string().contains("max_connections = 30"),
		"{error}"
	);
	assert!(error.to_string().contains("services.analytics"), "{error}");

	services::check(&config(), &project).unwrap();
}

#[test]
fn a_raised_connection_limit_starts_postgres_with_it() {
	let root = tempfile::tempdir().unwrap();
	let project = services_project(root.path(), "first-party");
	let mut c = config();
	c.postgres.max_connections = 200;
	let (yaml, _) = runtime::kamal(&c, &project, &["192.0.2.10".into()], "10.42.0.10").unwrap();
	let value: serde_json::Value = serde_json::from_str(&yaml).unwrap();
	assert_eq!(
		value["accessories"]["db"]["cmd"],
		"postgres -c max_connections=200"
	);
}

#[test]
fn service_database_urls_bind_their_passwords_on_the_private_address() {
	let root = tempfile::tempdir().unwrap();
	let project = services_project(root.path(), "first-party");
	let environment = |name: &str| {
		match name {
			"LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_PASSWORD" => Some("p@ss word".to_owned()),
			"LEPTOSKIT_SERVICE_ANALYTICS_TOKEN"
			| "LEPTOSKIT_SERVICE_BILLING_TOKEN"
			| "LEPTOSKIT_SERVICE_CARDS_TOKEN"
			| "ANALYTICS_GEOIP_KEY" => Some("fixture".to_owned()),
			_ => None,
		}
	};
	let values = services::values_with(&project, "10.42.0.10", environment).unwrap();
	assert_eq!(
		values["LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_URL"],
		"postgres://leptoskit_service_analytics:p%40ss%20word@10.42.0.10:5432/\
		 leptoskit_service_analytics?sslmode=disable"
	);
	assert_eq!(values.len(), 1);
	let error = services::values_with(&project, "10.42.0.10", |_| None).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("LEPTOSKIT_SERVICE_ANALYTICS_TOKEN"),
		"{error}"
	);
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(unix)]
async fn service_databases_are_their_roles_alone() {
	use leptoskit_db::Read as _;
	let Some(cluster) = postgres::PostgresCluster::start() else {
		return;
	};
	let root = tempfile::tempdir().unwrap();
	let project = services_project(root.path(), "first-party");
	let url = url::Url::parse(&cluster.database()).unwrap();
	let mut admin = url.clone();
	admin.set_path("/postgres");
	let admin_sql = |sql: &str| {
		let output = Command::new("psql")
			.arg("-X")
			.arg(admin.as_str())
			.args(["-v", "ON_ERROR_STOP=1", "-q", "-c", sql])
			.output()
			.unwrap();
		assert!(
			output.status.success(),
			"{}",
			String::from_utf8_lossy(&output.stderr)
		);
	};
	admin_sql("CREATE ROLE leptoskit LOGIN PASSWORD 'app'");
	admin_sql("CREATE DATABASE leptoskit OWNER leptoskit");
	let sql = crate::deploy::services::postgres_setup_sql(&project.services, "leptoskit");

	// Twice: a release runs it every time, and the second run changes only
	// the password.
	for password in ["first", "second's"] {
		// The setup holds `\\gexec` meta-commands, so it goes in on standard
		// input, as a release sends it.
		let mut child = Command::new("psql")
			.arg("-X")
			.arg(admin.as_str())
			.args(["-v", "ON_ERROR_STOP=1", "-q"])
			.arg(format!(
				"--set=LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_PASSWORD={password}"
			))
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.unwrap();
		std::io::Write::write_all(&mut child.stdin.take().unwrap(), sql.as_bytes()).unwrap();
		let output = child.wait_with_output().unwrap();
		assert!(
			output.status.success(),
			"{}",
			String::from_utf8_lossy(&output.stderr)
		);
	}

	// A pool connects on its first query, so reaching a database means
	// running one there.
	let reaches = |user: &str, database: &str| {
		let mut url = url.clone();
		url.set_username(user).unwrap();
		url.set_path(&format!("/{database}"));
		let url = url.to_string();
		async move {
			let Ok(db) = leptoskit_db::Db::connect(&url).await else {
				return None;
			};
			db.fetch(leptoskit_db::Statement {
				sql: "SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname = \
				      current_database()"
					.into(),
				args: vec![],
			})
			.await
			.ok()
			.map(|rows| rows[0][0].clone())
		}
	};
	assert_eq!(
		reaches("leptoskit_service_analytics", "leptoskit_service_analytics").await,
		Some(leptoskit_db::Value::Text(
			"leptoskit_service_analytics".into()
		)),
		"the service reaches its own database, which its role owns"
	);
	assert_eq!(
		reaches("leptoskit_service_analytics", "leptoskit").await,
		None,
		"a service's role must not reach the app's database"
	);
	assert_eq!(
		reaches("leptoskit", "leptoskit_service_analytics").await,
		None,
		"the app's role must not reach a service's database"
	);
	assert_eq!(
		reaches("leptoskit", "leptoskit").await,
		Some(leptoskit_db::Value::Text("leptoskit".into())),
		"the app still reaches its own database"
	);
	// The second run set the role's current password.
	let output = Command::new("psql")
		.arg("-X")
		.arg(admin.as_str())
		.args([
			"-At",
			"-c",
			"SELECT rolpassword IS NOT NULL FROM pg_authid WHERE rolname = \
			 'leptoskit_service_analytics'",
		])
		.output()
		.unwrap();
	assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "t");
}

/// The script Compose's Postgres runs on its first start, as the image's
/// entrypoint would: as the superuser, with the passwords in its
/// environment.
#[test]
#[cfg(unix)]
fn the_compose_database_script_gives_each_service_its_database() {
	let Some(cluster) = postgres::PostgresCluster::start() else {
		return;
	};
	let root = tempfile::tempdir().unwrap();
	let project = services_project(root.path(), "first-party");
	let url = cluster.database();
	let database = url::Url::parse(&url)
		.unwrap()
		.path()
		.trim_start_matches('/')
		.to_owned();
	let script = root.path().join("10-services.sh");
	// Compose's file doubles each `$`; the script it hands Postgres has one.
	fs::write(
		&script,
		crate::deploy::postgres_services_script(&project, false)
			.replace(&format!("\"{}\"", "notes"), &format!("\"{database}\"")),
	)
	.unwrap();
	let parsed = url::Url::parse(&url).unwrap();
	let output = Command::new("bash")
		.arg(&script)
		.env("POSTGRES_USER", "postgres")
		.env("POSTGRES_DB", &database)
		.env("PGHOST", parsed.host_str().unwrap())
		.env("PGPORT", parsed.port().unwrap().to_string())
		.env(
			"LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_PASSWORD",
			"it's a secret",
		)
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let check = Command::new("psql")
		.arg("-X")
		.arg(&url)
		.args([
			"-At",
			"-c",
			"SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname = \
			 'notes_service_analytics'",
		])
		.output()
		.unwrap();
	assert_eq!(
		String::from_utf8_lossy(&check.stdout).trim(),
		"notes_service_analytics"
	);
}

/// Writes a Postgres migration for the fixture's analytics service.
fn service_migration(root: &Path, name: &str, sql: &str) {
	let directory = root.join("analytics_service/migrations-postgres");
	fs::create_dir_all(directory.join(name)).unwrap();
	fs::write(directory.join(name).join("migration.sql"), sql).unwrap();
	let registry = directory.join("registry.txt");
	let mut text = fs::read_to_string(&registry).unwrap_or_default();
	text.push_str(name);
	text.push('\n');
	fs::write(registry, text).unwrap();
}

#[test]
fn releases_record_each_service_databases_schema() {
	let root = tempfile::tempdir().unwrap();
	let project = services_project(root.path(), "first-party");

	// Only analytics has a database: cards has none, and billing is remote.
	let schemas = service_schemas(&project.services).unwrap();
	assert_eq!(schemas.keys().collect::<Vec<_>>(), ["analytics"]);
	assert_eq!(schemas["analytics"].schema, Vec::<(String, String)>::new());
	let before = services_fingerprint(&schemas).unwrap();
	assert!(before.is_some());
	assert_eq!(
		services_fingerprint(&std::collections::BTreeMap::new()).unwrap(),
		None
	);

	service_migration(
		root.path(),
		"20260101T000000_init",
		"CREATE TABLE events (id INTEGER PRIMARY KEY);",
	);
	let schemas = service_schemas(&project.services).unwrap();
	assert_eq!(schemas["analytics"].schema.len(), 1);
	assert_eq!(schemas["analytics"].schema[0].0, "20260101T000000_init");
	assert_ne!(services_fingerprint(&schemas).unwrap(), before);

	// A record from before services were tracked loads with none, and one
	// without services is written exactly as before.
	let old: Release = serde_json::from_str(
		r#"{"version":"v1","migrations":"abc","schema":[],"image_digest":null,"completed_at":null}"#,
	)
	.unwrap();
	assert!(old.services.is_empty());
	assert!(!serde_json::to_string(&old).unwrap().contains("services"));
	let new = Release {
		services: schemas.clone(),
		..old
	};
	let json = serde_json::to_string(&new).unwrap();
	let read: Release = serde_json::from_str(&json).unwrap();
	assert_eq!(read.services, schemas);
}

/// The history script a release runs on the primary, against a real
/// Postgres through a `docker` that runs `psql` here: `[]` before the
/// service's database or its history table exists, then its migrations.
#[test]
#[cfg(unix)]
fn a_service_databases_history_is_read_before_and_after_it_migrates() {
	use std::os::unix::fs::PermissionsExt as _;

	let Some(cluster) = postgres::PostgresCluster::start() else {
		return;
	};
	let url = url::Url::parse(&cluster.database()).unwrap();
	let directory = tempfile::tempdir().unwrap();
	let docker = directory.path().join("docker");
	// `docker exec <container> psql …` becomes `psql …`.
	fs::write(&docker, "#!/bin/sh\nshift 2\nexec \"$@\"\n").unwrap();
	fs::set_permissions(&docker, fs::Permissions::from_mode(0o755)).unwrap();
	let psql = Command::new("sh")
		.args(["-c", "command -v psql"])
		.output()
		.unwrap();
	let psql_dir = Path::new(String::from_utf8_lossy(&psql.stdout).trim())
		.parent()
		.map(Path::to_path_buf);
	let path = std::env::join_paths(
		[directory.path().to_path_buf()]
			.into_iter()
			.chain(psql_dir)
			.chain(std::env::split_paths(
				&std::env::var_os("PATH").unwrap_or_default(),
			)),
	)
	.unwrap();
	let history = |database: &str| -> Vec<(String, String)> {
		let script = service_history_script("notes", database).unwrap();
		let output = Command::new("sh")
			.args(["-c", &script])
			.env("PATH", &path)
			.env("PGHOST", url.host_str().unwrap())
			.env("PGPORT", url.port().unwrap().to_string())
			.output()
			.unwrap();
		assert!(
			output.status.success(),
			"{}",
			String::from_utf8_lossy(&output.stderr)
		);
		serde_json::from_slice(&output.stdout).unwrap()
	};
	let admin = |sql: &str| {
		let mut admin = url.clone();
		admin.set_path("/postgres");
		let output = Command::new("psql")
			.arg("-X")
			.arg(admin.as_str())
			.args(["-v", "ON_ERROR_STOP=1", "-q", "-c", sql])
			.output()
			.unwrap();
		assert!(
			output.status.success(),
			"{}",
			String::from_utf8_lossy(&output.stderr)
		);
	};
	admin("CREATE ROLE leptoskit_admin SUPERUSER LOGIN");

	assert_eq!(
		history("leptoskit_service_analytics"),
		Vec::<(String, String)>::new()
	);
	admin("CREATE DATABASE leptoskit_service_analytics");
	assert_eq!(
		history("leptoskit_service_analytics"),
		Vec::<(String, String)>::new()
	);

	let mut service = url.clone();
	service.set_path("/leptoskit_service_analytics");
	let migrations = [leptoskit_db::Migration::new(
		"20260101T000000_init",
		"CREATE TABLE events (id INTEGER PRIMARY KEY)",
	)];
	tokio::runtime::Runtime::new().unwrap().block_on(async {
		let db = leptoskit_db::Db::connect(service.as_str()).await.unwrap();
		db.migrate(&migrations).await.unwrap();
	});
	let read = history("leptoskit_service_analytics");
	assert_eq!(
		read,
		[(
			"20260101T000000_init".to_owned(),
			migrations[0].checksum.to_string()
		)]
	);
	assert!(service_history_script("notes", "x; rm -rf /").is_err());
}

/// A rollback checks every database before any down migration: a service's
/// missing password, or its recorded migrations missing here, stops it
/// while the app's database is still untouched.
#[test]
fn a_rollback_checks_every_service_before_the_apps_down_migrations() {
	let root = tempfile::tempdir().unwrap();
	let project = services_project(root.path(), "first-party");
	let config = crate::deploy::tests::project_with_services(
		root.path(),
		crate::deploy::tests::Db::Postgres,
		"first-party",
	);
	fs::create_dir_all(root.path().join("migrations")).unwrap();
	fs::write(root.path().join("migrations/registry.txt"), "").unwrap();
	service_migration(
		root.path(),
		"20260101T000000_init",
		"CREATE TABLE events (id INTEGER PRIMARY KEY);",
	);
	let schemas = service_schemas(&project.services).unwrap();
	let target = Release {
		authorization: None,
		evidence: None,
		version: "v1".into(),
		migrations: history_fingerprint(Vec::new()).unwrap(),
		schema: Vec::new(),
		image_digest: None,
		completed_at: None,
		services: schemas.clone(),
	};
	let plan = |lookup: &dyn Fn(&str) -> Option<String>, target: &Release| {
		releases::plan_rollbacks(root.path(), &config, &project, target, lookup)
	};
	let every = |name: &str| Some(format!("{name}-value"));

	let planned = plan(&every, &target).unwrap();
	assert_eq!(
		planned.len(),
		2,
		"the app's database, then the analytics service's"
	);

	let no_service_password = |name: &str| {
		(name != "LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_PASSWORD")
			.then(|| every(name))
			.flatten()
	};
	let error = plan(&no_service_password, &target).err().unwrap();
	assert_eq!(
		error.to_string(),
		"set LEPTOSKIT_SERVICE_ANALYTICS_DATABASE_PASSWORD"
	);

	let mut missing = target.clone();
	missing
		.services
		.get_mut("analytics")
		.unwrap()
		.schema
		.push(("20260202T000000_gone".into(), "sum".into()));
	let error = plan(&every, &missing).err().unwrap();
	assert!(
		error.to_string().contains("leptoskit_service_analytics"),
		"{error}"
	);

	let mut gone = target;
	gone.services
		.insert("search".into(), schemas["analytics"].clone());
	let error = plan(&every, &gone).err().unwrap();
	assert!(
		error
			.to_string()
			.starts_with("services.search is in the target release"),
		"{error}"
	);
}
