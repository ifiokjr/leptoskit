use std::collections::BTreeSet;
use std::fmt::Write as _;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::infra::InfraConfig;
use serde_json::json;

pub(super) fn kamal(
	c: &InfraConfig,
	project: &crate::deploy::Project,
	hosts: &[String],
	private: &str,
) -> Result<(String, String)> {
	let database = project.database.as_ref().context("Postgres is required")?;
	let primary = hosts.first().context("no application hosts")?;
	super::services::check(c, project)?;
	let mut names: BTreeSet<String> = project.secrets.iter().chain(&c.secrets).cloned().collect();
	names.extend(project.urls.iter().cloned());
	names.insert(database.url_env.clone());
	if c.services.cache {
		names.insert("VALKEY_URL".into());
	}
	if c.services.relay {
		names.insert(
			project
				.urls
				.first()
				.cloned()
				.unwrap_or_else(|| "NATS_URL".into()),
		);
	}
	if let Some(plugin) = &c.plugins.analytics {
		names.insert("LEPTOSKIT_ANALYTICS_READ_KEY".into());
		names.insert("LEPTOSKIT_RELEASE".into());
		if plugin.otlp_endpoint.is_none()
			&& (c.control_url.is_some() || std::env::var_os("LEPTOSKIT_CONTROL_ORIGIN").is_some())
			|| std::env::var_os("LEPTOSKIT_ANALYTICS_WRITE_KEY").is_some()
		{
			names.insert("LEPTOSKIT_ANALYTICS_WRITE_KEY".into());
		}
	}
	for name in &names {
		ensure!(
			leptoskit_config::infra::env_name(name),
			"invalid secret name {name}"
		);
	}
	let image = c
		.image
		.strip_prefix(&format!("{}/", c.registry))
		.unwrap_or(&c.image);
	let mut config = json!({"service": c.name, "image": image, "servers": {"web": {"hosts": hosts,
		"options": {"read-only": true, "tmpfs": "/tmp:rw,nosuid,nodev,size=64m", "cap-drop": "ALL", "security-opt": "no-new-privileges:true"}}},
	"registry": {"server": c.registry, "username": c.registry_username, "password": ["KAMAL_REGISTRY_PASSWORD"]},
	"builder": {"arch": "amd64"}, "boot": {"limit": 1, "wait": 2},
	"proxy": {"host": c.domain, "app_port": project.port, "ssl": false, "forward_headers": true,
		"healthcheck": {"path": "/readyz", "interval": 2, "timeout": 5}},
	"deploy_timeout": 90, "drain_timeout": 30, "retain_containers": 5,
	"env": {"clear": {"LEPTOS_SITE_ADDR": format!("0.0.0.0:{}", project.port), "LEPTOS_SITE_ROOT": "/app/dist",
		"LEPTOS_OUTPUT_NAME": project.output_name, "LEPTOS_ENV": "PROD"}, "secret": names},
	"accessories": {
		"db": {"image": "postgres:17", "host": primary, "port": format!("{private}:5432:5432"),
			"env": {"clear": {"POSTGRES_USER": "leptoskit_admin", "POSTGRES_DB": "leptoskit"}, "secret": ["POSTGRES_PASSWORD", "DATABASE_PASSWORD"]},
			"files": ["db-init.sh:/docker-entrypoint-initdb.d/01-leptoskit.sh:ro"],
			"directories": [format!("/var/lib/leptoskit/{}/postgres:/var/lib/postgresql/data", c.name)], "options": {"network": "kamal"}},
		"tunnel": {"image": "cloudflare/cloudflared:latest", "hosts": hosts, "cmd": "tunnel --no-autoupdate run",
			"env": {"secret": ["TUNNEL_TOKEN"]}, "options": {"network": "kamal"}}
	}});
	if let Some(plugin) = &c.plugins.analytics {
		let sink = analytics_sink(c, std::env::var("LEPTOSKIT_CONTROL_ORIGIN").ok().as_deref());
		if let Some((endpoint, protocol)) = sink {
			config["env"]["clear"]["LEPTOSKIT_ANALYTICS_EXPORT_ENDPOINT"] = json!(endpoint);
			config["env"]["clear"]["LEPTOSKIT_ANALYTICS_EXPORT_PROTOCOL"] = json!(protocol);
		}
		for (name, value) in [
			("LEPTOSKIT_ANALYTICS", "1".to_owned()),
			("LEPTOSKIT_ANALYTICS_SERVICE", c.name.clone()),
			(
				"LEPTOSKIT_ANALYTICS_ENVIRONMENT",
				plugin.environment.clone(),
			),
			(
				"LEPTOSKIT_ANALYTICS_ORIGIN",
				format!("https://{}", c.domain),
			),
			(
				"LEPTOSKIT_ANALYTICS_WINDOWS",
				plugin.retained_windows.to_string(),
			),
			(
				"LEPTOSKIT_ANALYTICS_MAX_SERIES",
				plugin.max_series.to_string(),
			),
			(
				"LEPTOSKIT_ANALYTICS_ROUTES",
				plugin.browser_routes.join(","),
			),
		] {
			config["env"]["clear"][name] = json!(value);
		}
	}
	if c.services.cache {
		config["accessories"]["cache"] = json!({"image": "valkey/valkey:8-alpine", "host": primary,
			"port": format!("{private}:6379:6379"), "cmd": "sh -ec 'exec valkey-server --requirepass \"$VALKEY_PASSWORD\" --maxmemory 128mb --maxmemory-policy allkeys-lru --save \"\"'",
			"env": {"secret": ["VALKEY_PASSWORD"]}, "options": {"network": "kamal"}});
		names.insert("VALKEY_PASSWORD".into());
	}
	if c.services.relay {
		config["accessories"]["relay"] = json!({"image": "nats:2-alpine", "host": primary, "port": format!("{private}:4222:4222"),
			"cmd": "-c /etc/nats/leptoskit.conf", "env": {"secret": ["NATS_TOKEN"]},
			"files": ["nats.conf:/etc/nats/leptoskit.conf:ro"],
			"directories": [format!("/var/lib/leptoskit/{}/nats:/data", c.name)], "options": {"network": "kamal", "user": "1000:1000"}});
		names.insert("NATS_TOKEN".into());
	}
	if c.postgres.max_connections != leptoskit_config::infra::InfraPostgres::DEFAULT_MAX_CONNECTIONS
	{
		config["accessories"]["db"]["cmd"] = json!(format!(
			"postgres -c max_connections={}",
			c.postgres.max_connections
		));
	}
	let services = super::services::kamal(&mut config, c, project, primary);
	super::services::keep_app_env_to_itself(&mut config, project);
	names.extend(services.names);
	let custom: BTreeSet<&str> = services
		.lines
		.iter()
		.map(|(name, _)| name.as_str())
		.collect();
	names.extend([
		"KAMAL_REGISTRY_PASSWORD".into(),
		"POSTGRES_PASSWORD".into(),
		"DATABASE_PASSWORD".into(),
		"TUNNEL_TOKEN".into(),
	]);
	let mut secrets = String::new();
	for name in names.iter().filter(|name| !custom.contains(name.as_str())) {
		writeln!(secrets, "{name}=${name}")?;
	}
	for (name, value) in &services.lines {
		ensure!(
			leptoskit_config::infra::env_name(name),
			"invalid secret name {name}"
		);
		writeln!(secrets, "{name}={value}")?;
	}
	// JSON is a YAML subset understood by Kamal, avoiding hand-escaped YAML.
	Ok((serde_json::to_string_pretty(&config)? + "\n", secrets))
}

pub(super) fn analytics_sink(
	c: &InfraConfig,
	worker_origin: Option<&str>,
) -> Option<(String, &'static str)> {
	let plugin = c.plugins.analytics.as_ref()?;
	if let Some(endpoint) = &plugin.otlp_endpoint {
		return Some((endpoint.clone(), "otlp"));
	}
	let origin = worker_origin.or(c.control_url.as_deref())?;
	Some((
		format!(
			"{}/api/v1/projects/{}/analytics/ingest",
			origin.trim_end_matches('/'),
			c.name
		),
		"leptoskit",
	))
}

pub(super) const NATS_CONFIG: &str = "port: 4222\nauthorization { token: $NATS_TOKEN }\njetstream \
                                      { store_dir: /data, max_memory_store: 64MB, max_file_store: \
                                      1GB }\n";

pub(super) const DATABASE_INIT: &str = r#"#!/bin/bash
set -euo pipefail
psql --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" --set=ON_ERROR_STOP=1 --set=app_password="$DATABASE_PASSWORD" <<'SQL'
CREATE ROLE leptoskit LOGIN PASSWORD :'app_password';
ALTER DATABASE leptoskit OWNER TO leptoskit;
GRANT ALL ON SCHEMA public TO leptoskit;
SQL
"#;
