//! Resolve development secrets before starting the app's processes.

use std::process::Command;

use anyhow::Context as _;

const RESOLVED: &str = "LEPTOSKIT_SECRETS_RESOLVED";

/// Re-executes only development through Monosecret. Build and scaffold commands
/// never need credentials. Existing projects opt in by adding a manifest.
pub(crate) fn development(arguments: &[String], disabled: bool) -> anyhow::Result<Option<i32>> {
	if disabled || std::env::var_os(RESOLVED).is_some() {
		return Ok(None);
	}
	let (_, root) = leptoskit_config::LeptoskitConfig::load()?;
	if !root.join("monosecret.toml").is_file() {
		return Ok(None);
	}
	let executable = std::env::current_exe().context("locate the leptoskit executable")?;
	let status = Command::new("monosecret")
		.arg("--file")
		.arg(root.join("monosecret.toml"))
		.arg("run")
		.arg("--")
		.arg(executable)
		.args(arguments)
		.env(RESOLVED, "1")
		.env(
			"MONOSECRET_REASON",
			std::env::var_os("MONOSECRET_REASON")
				.unwrap_or_else(|| "Start leptoskit development processes".into()),
		)
		.status()
		.context(
			"install monosecret to resolve monosecret.toml, or use `leptoskit dev --no-secrets` \
			 for an environment supplied by your host",
		)?;
	Ok(Some(status.code().unwrap_or(1)))
}

#[cfg(test)]
mod tests {
	#[test]
	fn supplied_environment_bypasses_resolution() {
		assert_eq!(super::development(&[], true).unwrap(), None);
	}
}
