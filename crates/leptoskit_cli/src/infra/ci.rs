//! A reviewed pipeline in the app repository; runner setup stays explicit.
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use serde_json::Value;
use serde_json::json;

pub(super) fn workflow(
	config: &LeptoskitConfig,
	project: &crate::deploy::Project,
) -> Result<Value> {
	let c = super::settings(config)?;
	ensure!(
		!c.hosted_state || c.control_url.is_some(),
		"connect the hosted service before generating a hosted-state CI pipeline"
	);
	ensure!(
		c.state.is_some() || c.hosted_state,
		"CI requires infra.state: use shared, locking remote state instead of a runner-local \
		 state file"
	);
	ensure!(
		c.registry == "ghcr.io",
		"the built-in GitHub pipeline uses ghcr.io; export Docker files for another registry"
	);

	let mut names: BTreeSet<String> = project
		.secrets
		.iter()
		.chain(&c.secrets)
		.chain(&project.urls)
		.cloned()
		.collect();
	names.remove(&config.db.as_ref().expect("validated Postgres").url_env);
	if let Some(relay) = &config.rpc.live.relay_url_env {
		names.remove(relay);
	}
	names.extend(
		[
			"POSTGRES_PASSWORD",
			"DATABASE_PASSWORD",
			"AWS_ACCESS_KEY_ID",
			"AWS_SECRET_ACCESS_KEY",
			"AWS_SESSION_TOKEN",
			"CLOUDFLARE_API_TOKEN",
		]
		.map(str::to_owned),
	);
	names.insert(
		match c.compute {
			leptoskit_config::infra::ComputeProvider::Hetzner => "HCLOUD_TOKEN",
			leptoskit_config::infra::ComputeProvider::Digitalocean => "DIGITALOCEAN_TOKEN",
		}
		.into(),
	);
	if c.services.cache {
		names.insert("VALKEY_PASSWORD".into());
	}
	if c.services.relay {
		names.insert("NATS_TOKEN".into());
	}
	if let Some(plugin) = &c.plugins.analytics {
		names.insert("LEPTOSKIT_ANALYTICS_READ_KEY".into());
		if plugin.otlp_endpoint.is_some() {
			names.insert("LEPTOSKIT_ANALYTICS_WRITE_KEY".into());
		}
	}
	if c.backups.enabled {
		names.extend(
			[
				"LEPTOSKIT_BACKUP_PASSWORD",
				"LEPTOSKIT_BACKUP_ACCESS_KEY_ID",
				"LEPTOSKIT_BACKUP_SECRET_ACCESS_KEY",
			]
			.map(str::to_owned),
		);
	}
	let mut env = serde_json::Map::new();
	for name in names {
		env.insert(name.clone(), json!(format!("${{{{ secrets.{name} }}}}")));
	}
	env.insert(
		"KAMAL_REGISTRY_PASSWORD".into(),
		json!("${{ secrets.KAMAL_REGISTRY_PASSWORD }}"),
	);
	env.insert("SETUP".into(), json!("${{ inputs.setup || false }}"));
	env.insert(
		"MAINTENANCE".into(),
		json!("${{ inputs.maintenance || false }}"),
	);
	if c.control_url.is_some() {
		env.clear();
		env.insert(
			"LEPTOSKIT_CONTROL_KEY".into(),
			json!("${{ secrets.LEPTOSKIT_CONTROL_KEY }}"),
		);
		env.insert("SETUP".into(), json!("${{ inputs.setup || false }}"));
		env.insert(
			"MAINTENANCE".into(),
			json!("${{ inputs.maintenance || false }}"),
		);
	}
	let deploy_script = if c.control_url.is_some() {
		"set -eu\nargs=()\nif [ \"$SETUP\" = true ]; then args+=(--setup); fi\nif [ \
		 \"$MAINTENANCE\" = true ]; then args+=(--maintenance); fi\nlk deploy --version \
		 \"$GITHUB_SHA\" \"${args[@]}\"\nlk infra status\n"
			.into()
	} else {
		format!(
			"set -eu\ntofu -chdir=infra/{} init -input=false\nargs=()\nif [ \"$SETUP\" = true ]; \
			 then args+=(--setup); fi\nif [ \"$MAINTENANCE\" = true ]; then \
			 args+=(--maintenance); fi\nlk deploy --version \"$GITHUB_SHA\" \"${{args[@]}}\"\nlk \
			 infra status\n",
			c.name
		)
	};
	Ok(json!({"name": format!("Deploy {}", c.name),
		"on": {"push": {"branches": ["main"]}, "workflow_dispatch": {"inputs": {
			"setup": {"description": "First release: bootstrap accessories and verified backups", "type": "boolean", "default": false},
			"maintenance": {"description": "Stop writers for a reviewed schema-changing release", "type": "boolean", "default": false}}}},
		"permissions": {"contents": "read"}, "concurrency": {"group": format!("leptoskit-image-{}", super::digest(format!("{}/{}", c.registry, c.image.strip_prefix(&format!("{}/", c.registry)).unwrap_or(&c.image)).as_bytes())), "cancel-in-progress": false},
		"jobs": {
			"build": {"runs-on": ["self-hosted", "linux", "leptoskit"], "permissions": {"contents": "read", "packages": "write"},
				"steps": [{"uses": "actions/checkout@11d5960a326750d5838078e36cf38b85af677262", "with": {"persist-credentials": false}},
					{"name": "Build and push immutable server and website image", "env": {"KAMAL_REGISTRY_PASSWORD": "", "REGISTRY_USER": "${{ github.actor }}", "REGISTRY_PASSWORD": "${{ secrets.GITHUB_TOKEN }}"},
					 "run": "set -eu\nDOCKER_CONFIG=$(mktemp -d)\nexport DOCKER_CONFIG\ntrap 'rm -rf \"$DOCKER_CONFIG\"' EXIT\nprintf '%s' \"$REGISTRY_PASSWORD\" | docker login ghcr.io -u \"$REGISTRY_USER\" --password-stdin\nlk infra build --version \"$GITHUB_SHA\" --push --reuse-existing\n"}]},
			"deploy": {"needs": "build", "runs-on": ["self-hosted", "linux", "leptoskit"], "environment": c.name,
				"permissions": {"contents": "read", "packages": "read"}, "steps": [
					{"uses": "actions/checkout@11d5960a326750d5838078e36cf38b85af677262", "with": {"persist-credentials": false}},
					{"name": "Release through private origin and verify health", "env": env,
					"run": deploy_script, "shell": "bash"}]}}}))
}

pub(super) fn generate(root: &Path, config: &LeptoskitConfig, force: bool) -> Result<()> {
	let project = crate::deploy::Project::read(root, config)?;
	let text = serde_json::to_string_pretty(&workflow(config, &project)?)? + "\n";
	let path = root.join(".github/workflows/leptoskit-deploy.yml");
	if path.exists() {
		ensure!(
			force || fs::read_to_string(&path)? == text,
			"workflow differs; review before regenerating with --force"
		);
	}
	fs::create_dir_all(path.parent().expect("workflow parent"))?;
	fs::write(&path, text)?;
	println!(
		"Wrote {}. Configure its GitHub environment, scoped secrets, and the prepared runner \
		 before enabling it. Infrastructure apply remains an explicit reviewed operation.",
		path.display()
	);
	Ok(())
}
