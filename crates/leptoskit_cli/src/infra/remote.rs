//! SSH transport and a durable, fail-closed reservation shared by operators.

use std::io::Write as _;
use std::process::Command;
use std::process::Stdio;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;

pub(super) fn base(name: &str) -> String {
	format!("/var/lib/leptoskit/{name}/control")
}

pub(super) fn quote(text: &str) -> String {
	format!("'{}'", text.replace('\'', "'\\''"))
}

pub(super) fn ssh(ip: &str) -> Command {
	let mut command = Command::new("ssh");
	command.args([
		"-o",
		"BatchMode=yes",
		"-o",
		"ConnectTimeout=15",
		"-o",
		"ServerAliveInterval=15",
		"-o",
		"ServerAliveCountMax=3",
		"--",
		&format!("root@{ip}"),
	]);
	command
}

pub(super) fn execute(ip: &str, script: &str, input: Option<&[u8]>) -> Result<Vec<u8>> {
	let mut child = ssh(ip)
		.arg(script)
		.stdin(if input.is_some() {
			Stdio::piped()
		} else {
			Stdio::null()
		})
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.context("couldn't run SSH")?;
	if let Some(input) = input {
		child
			.stdin
			.take()
			.context("SSH stdin unavailable")?
			.write_all(input)?;
	}
	let result = child.wait_with_output()?;
	ensure!(
		result.status.success(),
		"remote operation failed ({}); secrets and remote output were withheld; inspect the \
		 host's operator logs",
		result.status
	);
	ensure!(
		result.stdout.len() <= 4 * 1024 * 1024,
		"remote control response exceeds 4 MiB"
	);
	Ok(result.stdout)
}

pub(super) fn write(ip: &str, path: &str, bytes: &[u8]) -> Result<()> {
	let temp = format!("{path}.{}", uuid::Uuid::new_v4());
	execute(
		ip,
		&format!(
			"set -eu; umask 077; cat > {}; chmod 600 {}; mv -f {} {}",
			quote(&temp),
			quote(&temp),
			quote(&temp),
			quote(path)
		),
		Some(bytes),
	)?;
	Ok(())
}

/// No automatic expiry: losing a runner cannot let another runner mutate an
/// environment while an earlier remote command might still be finishing.
pub(super) struct Reservation {
	ip: String,
	directory: String,
	pub owner: String,
}

impl Reservation {
	pub fn acquire(ip: &str, name: &str) -> Result<Self> {
		let directory = base(name);
		let owner = uuid::Uuid::new_v4().to_string();
		let candidate = format!("{directory}/{owner}.owner");
		let lock = format!("{directory}/operation.lock");
		let script = format!(
			"set -eu; umask 077; mkdir -p {dir}; chmod 700 {dir}; printf '%s' {owner} > \
			 {candidate}; if ln {candidate} {lock} 2>/dev/null; then rm {candidate}; else rm \
			 {candidate}; exit 73; fi",
			dir = quote(&directory),
			owner = quote(&owner),
			candidate = quote(&candidate),
			lock = quote(&lock)
		);
		execute(ip, &script, None).with_context(|| {
			format!(
				"environment is reserved; inspect `lk infra status` before recovering its owner \
				 lock (attempt {owner})"
			)
		})?;
		Ok(Self {
			ip: ip.into(),
			directory,
			owner,
		})
	}

	pub fn check(&self) -> Result<()> {
		let value = execute(
			&self.ip,
			&format!(
				"cat {}",
				quote(&format!("{}/operation.lock", self.directory))
			),
			None,
		)?;
		ensure!(
			value == self.owner.as_bytes(),
			"environment reservation changed; aborting operation"
		);
		Ok(())
	}
}

impl Drop for Reservation {
	fn drop(&mut self) {
		if unlock(&self.ip, &self.directory, &self.owner).is_err() {
			eprintln!(
				"Environment remains reserved by {}; inspect it before `lk infra unlock --owner \
				 {} --yes`",
				self.owner, self.owner
			);
		}
	}
}

pub(super) fn unlock(ip: &str, directory: &str, owner: &str) -> Result<()> {
	ensure!(
		uuid::Uuid::parse_str(owner).is_ok(),
		"lock owner must be the UUID shown by status"
	);
	let path = quote(&format!("{directory}/operation.lock"));
	execute(
		ip,
		&format!(
			"set -eu; test \"$(cat {path})\" = {}; rm {path}",
			quote(owner)
		),
		None,
	)?;
	Ok(())
}

pub(super) fn read(ip: &str, name: &str, file: &str) -> Result<Vec<u8>> {
	let path = quote(&format!("{}/{file}", base(name)));
	execute(
		ip,
		&format!("set -eu; if [ -f {path} ]; then cat {path}; else printf '[]'; fi"),
		None,
	)
}
