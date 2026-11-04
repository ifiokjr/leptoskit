//! Operator entry points share the same primary-host reservation as releases.

use std::path::Path;

use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::infra::InfraConfig;
use serde_json::Value;
use serde_json::json;

use super::BackupCommand;
use super::applied_bindings;
use super::lock;
use super::output;
use super::recovery;
use super::releases;
use super::remote;
use super::server_ip;

pub(super) fn backups(dir: &Path, c: &InfraConfig, command: BackupCommand) -> Result<()> {
	ensure!(
		c.backups.enabled,
		"enable infra.backups.enabled before using scheduled backups"
	);
	applied_bindings(dir, c)?;
	let ip = server_ip(dir)?;
	let _local = lock(dir)?;
	let reservation = remote::Reservation::acquire(&ip, &c.name)?;
	let result = match command {
		BackupCommand::Install => {
			recovery::install(&ip, c, &output(dir, "backup_endpoint")?, &reservation)?;
			json!({"installed": true})
		}
		BackupCommand::Run => {
			recovery::invoke(&ip, c, "backup", "latest", false, Some(&reservation.owner))?
		}
		BackupCommand::List => {
			recovery::invoke(
				&ip,
				c,
				"snapshots",
				"latest",
				false,
				Some(&reservation.owner),
			)?
		}
		BackupCommand::Verify { snapshot } => {
			recovery::invoke(&ip, c, "verify", &snapshot, false, Some(&reservation.owner))?
		}
		BackupCommand::Status => {
			recovery::invoke(&ip, c, "status", "latest", false, Some(&reservation.owner))?
		}
	};
	println!("{}", serde_json::to_string_pretty(&result)?);
	if matches!(result.get("healthy"), Some(Value::Bool(false))) {
		anyhow::bail!("backup is unhealthy");
	}
	Ok(())
}

pub(super) fn restore(dir: &Path, c: &InfraConfig, snapshot: &str, yes: bool) -> Result<()> {
	ensure!(
		yes,
		"restore replaces the current database and stops all app writers; pass --yes to confirm"
	);
	ensure!(
		c.backups.enabled,
		"restore requires the configured backup worker"
	);
	applied_bindings(dir, c)?;
	let _local = lock(dir)?;
	let ip = server_ip(dir)?;
	let reservation = remote::Reservation::acquire(&ip, &c.name)?;
	// Verify before stopping traffic; the worker repeats the check before the
	// swap.
	recovery::invoke(&ip, c, "verify", snapshot, false, Some(&reservation.owner))?;
	releases::stop(&releases::hosts(dir)?, c)?;
	let result = recovery::invoke(&ip, c, "restore", snapshot, true, Some(&reservation.owner))?;
	println!("{}", serde_json::to_string_pretty(&result)?);
	println!(
		"Application is stopped. Inspect `lk infra releases`, then `lk deploy --recover --version \
		 <recorded-tag>`."
	);
	Ok(())
}

pub(super) fn status(dir: &Path, c: &InfraConfig) -> Result<()> {
	let hosts = releases::hosts(dir)?;
	let mut reports = Vec::new();
	for ip in &hosts {
		let services = if ip == &hosts[0] {
			format!(
				"db {} {}",
				if c.services.cache { "cache" } else { "" },
				if c.services.relay { "relay" } else { "" }
			)
		} else {
			String::new()
		};
		let command = format!(
			"python3 -c {} {} {}",
			remote::quote(include_str!("status.py")),
			remote::quote(&c.name),
			services
		);
		let report: Value = match remote::execute(ip, &command, None)
			.and_then(|bytes| Ok(serde_json::from_slice(&bytes)?))
		{
			Ok(value) => value,
			Err(_) => json!({"healthy": false, "error": "host health query failed"}),
		};
		reports.push(json!({"ip": ip, "health": report}));
	}
	let ip = &hosts[0];
	let backups = if c.backups.enabled {
		recovery::invoke(ip, c, "status", "latest", false, None)
			.unwrap_or_else(|_| json!({"healthy": false, "error": "backup worker unavailable"}))
	} else {
		json!({"enabled": false})
	};
	let operation: Value = serde_json::from_slice(&remote::read(ip, &c.name, "operation.json")?)?;
	let owner = remote::execute(
		ip,
		&format!(
			"if [ -f {base}/operation.lock ]; then cat {base}/operation.lock; fi",
			base = remote::quote(&remote::base(&c.name))
		),
		None,
	)?;
	let db = remote::execute(
		ip,
		&format!(
			"docker exec {}-db pg_isready -U leptoskit -d leptoskit >/dev/null",
			c.name
		),
		None,
	)
	.is_ok();
	let operation_ok = !matches!(
		operation["phase"].as_str(),
		Some("failed" | "verification-failed-app-stopped")
	);
	let restore: Value =
		serde_json::from_slice(&remote::read(ip, &c.name, "restore-status.json")?)?;
	let reset: Value =
		serde_json::from_slice(&remote::read(ip, &c.name, "derived-reset-required.json")?)?;
	let healthy = reset.is_array()
		&& operation_ok
		&& db
		&& reports.iter().all(|r| r["health"]["healthy"] == true)
		&& (!c.backups.enabled || backups["healthy"] == true);
	println!(
		"{}",
		serde_json::to_string_pretty(
			&json!({"healthy": healthy, "hosts": reports, "database_ready": db,
		"backups": backups, "operation": operation, "restore": restore, "derived_reset_required": reset, "reservation_owner": String::from_utf8(owner)?,
		"persistent_services_high_availability": false})
		)?
	);
	ensure!(healthy, "environment is unhealthy; inspect the JSON status");
	Ok(())
}
