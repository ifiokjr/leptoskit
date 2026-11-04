//! Encrypted host-side backup lifecycle, isolated restore checks, and recovery.

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::infra::InfraConfig;
use serde_json::Value;
use serde_json::json;

use super::remote::Reservation;
use super::remote::{self};

pub(super) fn credentials() -> Result<Value> {
	let required = |name| {
		std::env::var(name)
			.ok()
			.filter(|s| !s.is_empty())
			.with_context(|| {
				format!(
					"set {name} in the operator environment; keep backup credentials separate \
					 from publisher credentials"
				)
			})
	};
	Ok(
		json!({"RESTIC_PASSWORD": required("LEPTOSKIT_BACKUP_PASSWORD")?,
		"AWS_ACCESS_KEY_ID": required("LEPTOSKIT_BACKUP_ACCESS_KEY_ID")?,
		"AWS_SECRET_ACCESS_KEY": required("LEPTOSKIT_BACKUP_SECRET_ACCESS_KEY")?}),
	)
}

pub(super) fn install(
	ip: &str,
	c: &InfraConfig,
	endpoint: &str,
	reservation: &Reservation,
) -> Result<()> {
	reservation.check()?;
	let base = remote::base(&c.name);
	let mut secrets = credentials()?;
	secrets["AWS_DEFAULT_REGION"] = json!(c.b2.as_ref().map_or("auto", |b2| b2.region.as_str()));
	let config = json!({"name": c.name, "services": c.services, "repository": format!("s3:{endpoint}/{}/{}", c.backup_bucket, c.name),
		"daily": c.backups.daily, "weekly": c.backups.weekly, "monthly": c.backups.monthly, "max_age_hours": c.backups.max_age_hours});
	remote::execute(
		ip,
		&format!(
			"systemd-analyze calendar {} >/dev/null; command -v restic >/dev/null; command -v \
			 python3 >/dev/null",
			remote::quote(&c.backups.schedule)
		),
		None,
	)?;
	remote::write(
		ip,
		&format!("{base}/backup-secrets.json"),
		&serde_json::to_vec(&secrets)?,
	)?;
	remote::write(
		ip,
		&format!("{base}/recovery.json"),
		&serde_json::to_vec(&config)?,
	)?;
	remote::write(
		ip,
		&format!("{base}/recovery.py"),
		include_bytes!("recovery.py"),
	)?;
	let unit = format!("leptoskit-{}-backup", c.name);
	let service = format!(
		"[Unit]\nDescription=Encrypted verified database backup for {}\nAfter=docker.service \
		 network-online.target\nWants=network-online.target\n[Service]\nType=oneshot\nUMask=0077\\
		 nExecStart=/usr/bin/python3 {base}/recovery.py \
		 backup\nNoNewPrivileges=true\nPrivateTmp=true\nProtectSystem=strict\\
		 nReadWritePaths={base} /run/docker.sock\n",
		c.name
	);
	let timer = format!(
		"[Unit]\nDescription=Database backup schedule for \
		 {}\n[Timer]\nOnCalendar={}\nPersistent=true\nRandomizedDelaySec=300\n[Install]\\
		 nWantedBy=timers.target\n",
		c.name, c.backups.schedule
	);
	remote::write(
		ip,
		&format!("/etc/systemd/system/{unit}.service"),
		service.as_bytes(),
	)?;
	remote::write(
		ip,
		&format!("/etc/systemd/system/{unit}.timer"),
		timer.as_bytes(),
	)?;
	invoke(ip, c, "init", "latest", false, Some(&reservation.owner))?;
	remote::execute(
		ip,
		&format!(
			"systemctl daemon-reload; systemctl enable --now {}.timer",
			remote::quote(&unit)
		),
		None,
	)?;
	Ok(())
}

pub(super) fn invoke(
	ip: &str,
	c: &InfraConfig,
	action: &str,
	snapshot: &str,
	yes: bool,
	owner: Option<&str>,
) -> Result<Value> {
	ensure!(
		["init", "backup", "verify", "snapshots", "restore", "status"].contains(&action),
		"unsupported backup action"
	);
	ensure!(
		snapshot == "latest"
			|| ((8..=64).contains(&snapshot.len())
				&& snapshot
					.bytes()
					.all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())),
		"snapshot must be latest or an 8–64 character hexadecimal id"
	);
	let reservation = match owner {
		Some(value) => {
			ensure!(
				uuid::Uuid::parse_str(value).is_ok(),
				"invalid reservation owner"
			);
			format!(" --reservation {}", remote::quote(value))
		}
		None => String::new(),
	};
	let script = format!(
		"python3 {}/recovery.py {} --snapshot {}{}{}",
		remote::quote(&remote::base(&c.name)),
		action,
		remote::quote(snapshot),
		reservation,
		if yes { " --yes" } else { "" }
	);
	let bytes = remote::execute(ip, &script, None)?;
	serde_json::from_slice(&bytes).context("backup worker returned invalid JSON")
}
