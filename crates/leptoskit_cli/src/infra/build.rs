//! Image metadata comes from the same migration files used for releases.

use std::io::Write as _;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;

pub(super) const SCHEMA_LABEL: &str = "io.leptoskit.migrations";

pub(super) fn fingerprint(root: &Path, config: &LeptoskitConfig) -> Result<String> {
	let db = config
		.db
		.as_ref()
		.context("configure the database before building a managed image")?;
	let migrations = leptoskit_db::load_migrations(&root.join(&db.migrations))?;
	super::history_fingerprint(
		migrations
			.iter()
			.map(|m| (m.name.to_string(), m.checksum.to_string()))
			.collect(),
	)
}

pub(super) fn validate_version(version: &str) -> Result<()> {
	ensure!(
		!version.is_empty()
			&& version.len() <= 128
			&& version != "latest"
			&& !version.starts_with(['-', '.'])
			&& version
				.bytes()
				.all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
		"use a unique immutable image tag"
	);
	Ok(())
}

fn docker(root: &Path, private: Option<&tempfile::TempDir>) -> Command {
	let mut command = Command::new("docker");
	command.current_dir(root);
	if let Some(directory) = private {
		command.env("DOCKER_CONFIG", directory.path());
	}
	command
}

fn credentials(
	root: &Path,
	c: &leptoskit_config::infra::InfraConfig,
) -> Result<Option<tempfile::TempDir>> {
	let Some(password) = std::env::var("KAMAL_REGISTRY_PASSWORD")
		.ok()
		.filter(|s| !s.is_empty())
	else {
		return Ok(None);
	};
	let directory = tempfile::tempdir()?;
	let mut login = docker(root, Some(&directory))
		.args([
			"login",
			&c.registry,
			"--username",
			&c.registry_username,
			"--password-stdin",
		])
		.stdin(Stdio::piped())
		.spawn()?;
	let written = login
		.stdin
		.take()
		.context("registry login stdin unavailable")?
		.write_all(password.as_bytes());
	let status = login.wait()?;
	written?;
	ensure!(status.success(), "registry login failed");
	Ok(Some(directory))
}

fn exists(root: &Path, private: Option<&tempfile::TempDir>, tag: &str) -> Result<bool> {
	let result = docker(root, private)
		.args(["manifest", "inspect", tag])
		.output()?;
	if result.status.success() {
		return Ok(true);
	}
	let error = String::from_utf8_lossy(&result.stderr).to_ascii_lowercase();
	ensure!(
		error.contains("manifest unknown") || error.contains("no such manifest"),
		"couldn't verify registry tag absence; publication is blocked (check registry credentials \
		 and network)"
	);
	Ok(false)
}

pub(super) fn run(
	root: &Path,
	config: &LeptoskitConfig,
	version: &str,
	push: bool,
	reuse_existing: bool,
) -> Result<()> {
	validate_version(version)?;
	let c = super::settings(config)?;
	// Generation excludes state, plans, and runtime secrets from Docker's
	// context.
	super::run(super::InfraCommand::Generate { force: false })?;
	let dir = super::directory(root, c);
	let _lock = super::lock(&dir)?;
	let image = if c.image.starts_with(&format!("{}/", c.registry)) {
		c.image.clone()
	} else {
		format!("{}/{}", c.registry, c.image)
	};
	let tag = format!("{image}:{version}");
	let schema = fingerprint(root, config)?;
	let private = if push { credentials(root, c)? } else { None };
	if push && exists(root, private.as_ref(), &tag)? {
		ensure!(
			reuse_existing,
			"image tag already exists; choose a new tag, deploy the existing artifact, or \
			 explicitly --reuse-existing"
		);
		ensure!(
			docker(root, private.as_ref())
				.args(["pull", &tag])
				.status()?
				.success(),
			"couldn't pull the existing artifact"
		);
		let result = docker(root, private.as_ref())
			.args([
				"image",
				"inspect",
				"--format",
				&format!("{{{{index .Config.Labels \"{SCHEMA_LABEL}\"}}}}"),
				&tag,
			])
			.output()?;
		ensure!(
			result.status.success() && String::from_utf8_lossy(&result.stdout).trim() == schema,
			"existing artifact has different migration metadata; refusing reuse"
		);
		println!("Reused existing artifact {tag}; no image was rebuilt or overwritten");
		return Ok(());
	}
	let label = format!("{SCHEMA_LABEL}={schema}");
	let status = docker(root, private.as_ref())
		.args([
			"build",
			"--platform",
			"linux/amd64",
			"--label",
			&label,
			"--tag",
			&tag,
			".",
		])
		.status()
		.context("couldn't build the managed image")?;
	ensure!(status.success(), "managed image build failed");
	ensure!(
		fingerprint(root, config)? == schema,
		"migration files changed during the build; rebuild before pushing"
	);
	if push {
		ensure!(
			!exists(root, private.as_ref(), &tag)?,
			"another publisher created this tag during the build; refusing overwrite"
		);
		ensure!(
			docker(root, private.as_ref())
				.args(["push", &tag])
				.status()?
				.success(),
			"managed image push failed"
		);
	}
	println!("Built {tag} with migration metadata");
	Ok(())
}
