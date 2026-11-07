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
