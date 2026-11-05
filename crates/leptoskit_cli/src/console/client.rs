use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::Intent;
use leptoskit_deploy_console::control::Operation;
use leptoskit_deploy_console::control::Overview;
use leptoskit_deploy_console::control::Phase;
use leptoskit_deploy_console::control::Scope;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;

pub struct Connection {
	url: String,
	key: String,
	project: String,
	root: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Credential {
	key: String,
}

#[derive(Debug)]
struct Rejected(&'static str);
impl std::fmt::Display for Rejected {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "deployment service rejected the request: {}", self.0)
	}
}
impl std::error::Error for Rejected {}

impl Connection {
	pub fn for_project() -> Result<Option<Self>> {
		if std::env::var("LEPTOSKIT_CONTROL_LOCAL").as_deref() == Ok("1") {
			return Ok(None);
		}
		let (config, root) = LeptoskitConfig::load()?;
		let Some(infra) = config.infra else {
			return Ok(None);
		};
		let Some(url) = infra.control_url else {
			return Ok(None);
		};
		let url = origin(&url)?;
		let key = key(&url, &infra.name)?;
		Ok(Some(Self {
			url,
			key,
			project: infra.name,
			root,
		}))
	}

	fn request(&self, path: &str, body: Option<&serde_json::Value>) -> Result<serde_json::Value> {
		let agent: ureq::Agent = ureq::Agent::config_builder()
			.timeout_global(Some(std::time::Duration::from_secs(15)))
			.max_redirects(0)
			.http_status_as_error(false)
			.build()
			.into();
		let url = format!("{}{}", self.url, path);
		let response = if let Some(body) = body {
			agent
				.post(&url)
				.header("Authorization", format!("Bearer {}", self.key))
				.send_json(body)
		} else {
			agent
				.get(&url)
				.header("Authorization", format!("Bearer {}", self.key))
				.call()
		};
		let mut response = response.map_err(|_| {
			anyhow::anyhow!(
				"deployment service request failed; check the connection and scoped credential"
			)
		})?;
		let status = response.status();
		let text = response
			.body_mut()
			.with_config()
			.limit(16 * 1024 * 1024)
			.read_to_string()?;
		if !status.is_success() {
			let problem = serde_json::from_str::<serde_json::Value>(&text).unwrap_or_default();
			let code = match problem["error"].as_str() {
				Some("revision_conflict") => "revision_conflict",
				Some("operation_in_progress") => "operation_in_progress",
				Some("plan_is_not_current") => "plan_is_not_current",
				Some("idempotency_conflict") => "idempotency_conflict",
				Some("authentication_required") => "authentication_required",
				Some("scope_or_origin_denied") => "scope_or_origin_denied",
				_ => "request_rejected",
			};
			return Err(Rejected(code).into());
		}
		if text.is_empty() {
			Ok(serde_json::Value::Null)
		} else {
			serde_json::from_str(&text).context("invalid deployment service response")
		}
	}
}

pub(super) fn origin(value: &str) -> Result<String> {
	let url = url::Url::parse(value).context("invalid deployment service URL")?;
	let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
	ensure!(
		(url.scheme() == "https" || (local && url.scheme() == "http"))
			&& url.path() == "/"
			&& url.username().is_empty()
			&& url.password().is_none()
			&& url.query().is_none()
			&& url.fragment().is_none(),
		"deployment service URL needs an HTTPS origin or localhost HTTP"
	);
	Ok(url.origin().ascii_serialization())
}

fn credential_path(url: &str, project: &str) -> Result<PathBuf> {
	let root = match std::env::var_os("XDG_CONFIG_HOME") {
		Some(root) => PathBuf::from(root),
		None => {
			PathBuf::from(std::env::var_os("HOME").context("set HOME or XDG_CONFIG_HOME")?)
				.join(".config")
		}
	};
	Ok(root.join("leptoskit/control").join(format!(
		"{:x}.json",
		sha2::Sha256::digest(format!("{url}/{project}").as_bytes())
	)))
}

fn key(url: &str, project: &str) -> Result<String> {
	let key = if let Ok(key) = std::env::var("LEPTOSKIT_CONTROL_KEY") {
		key
	} else {
		let path = credential_path(url, project)?;
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			ensure!(
				std::fs::metadata(&path)
					.context("run lk console connect with LEPTOSKIT_CONTROL_KEY set")?
					.permissions()
					.mode()
					.trailing_zeros()
					>= 6,
				"deployment credential file must be private"
			);
		}
		serde_json::from_slice::<Credential>(
			&std::fs::read(path)
				.context("run lk console connect with LEPTOSKIT_CONTROL_KEY set")?,
		)?
		.key
	};
	ensure!(
		(32..=256).contains(&key.len()) && key.bytes().all(|b| (33..127).contains(&b)),
		"invalid deployment service credential"
	);
	Ok(key)
}

pub(super) fn private_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
	let parent = path.parent().context("file needs a parent")?;
	std::fs::create_dir_all(parent)?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
	}
	use std::io::Write as _;
	let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
	temporary.write_all(bytes)?;
	temporary.as_file().sync_all()?;
	temporary.persist(path)?;
	Ok(())
}

pub fn connect(url: &str) -> Result<()> {
	let url = origin(url)?;
	let (config, root) = LeptoskitConfig::load()?;
	let infra = config.infra.context("add [infra] before linking the app")?;
	let connection = Connection {
		key: key(&url, &infra.name)?,
		url: url.clone(),
		project: infra.name,
		root: root.clone(),
	};
	let overview: Overview = serde_json::from_value(connection.request("/api/v1/overview", None)?)?;
	ensure!(
		overview.projects.iter().any(|p| p.id == connection.project),
		"this repository is not registered on the deployment service"
	);
	private_write(
		&credential_path(&url, &connection.project)?,
		&serde_json::to_vec(&Credential {
			key: connection.key,
		})?,
	)?;
	let path = root.join(leptoskit_config::CONFIG_FILE);
	let mut document = std::fs::read_to_string(&path)?.parse::<toml_edit::DocumentMut>()?;
	document["infra"]["control_url"] = toml_edit::value(&url);
	let text = document.to_string();
	LeptoskitConfig::from_toml(&text, &root)?;
	std::fs::write(path, text)?;
	println!("Connected to {url}. Use lk deploy --version <immutable-image-tag>.");
	Ok(())
}

fn connected() -> Result<Connection> {
	Connection::for_project()?.context("run lk console connect first")
}
pub fn status() -> Result<()> {
	let connection = connected()?;
	println!(
		"{}",
		serde_json::to_string_pretty(&connection.request("/api/v1/overview", None)?)?
	);
	Ok(())
}
pub fn analytics(connection: &Connection) -> Result<()> {
	println!(
		"{}",
		serde_json::to_string_pretty(&connection.request(
			&format!("/api/v1/projects/{}/analytics", connection.project),
			None
		)?)?
	);
	Ok(())
}
pub fn token(project: &str, scope: Scope) -> Result<()> {
	let connection = connected()?;
	let value = connection.request(
		"/api/v1/tokens",
		Some(&serde_json::json!({"scope":scope,"project":project})),
	)?;
	println!(
		"{}",
		value["key"]
			.as_str()
			.context("service did not issue a key")?
	);
	Ok(())
}

pub fn deploy(connection: &Connection, args: &crate::infra::ReleaseArgs) -> Result<()> {
	ensure!(
		!args.recover && !args.rollback_schema,
		"perform disaster recovery and database reversal on the trusted worker using the explicit \
		 local recovery commands"
	);
	let action = if let Some(version) = &args.rollback {
		Action::Rollback {
			version: version.clone(),
		}
	} else {
		let version = if let Some(version) = &args.version {
			version.clone()
		} else {
			let output = crate::git::command(&connection.root)
				.args(["rev-parse", "--verify", "HEAD"])
				.output()?;
			ensure!(
				output.status.success(),
				"pass --version for an app outside Git"
			);
			String::from_utf8(output.stdout)?.trim().to_owned()
		};
		Action::Deploy {
			version,
			setup: args.setup,
			maintenance: args.maintenance,
		}
	};
	submit(connection, action)?;
	Ok(())
}

pub fn plan(connection: &Connection) -> Result<()> {
	let operation = submit(connection, Action::Plan)?;
	private_write(
		&connection.root.join(".leptoskit/control-plan.json"),
		&serde_json::to_vec(&serde_json::json!({"url":connection.url,"id":operation.id}))?,
	)
}
pub fn apply(connection: &Connection) -> Result<()> {
	let plan: serde_json::Value = serde_json::from_slice(
		&std::fs::read(connection.root.join(".leptoskit/control-plan.json"))
			.context("run lk infra plan and review it before applying")?,
	)?;
	ensure!(
		plan["url"] == connection.url,
		"saved plan belongs to another service"
	);
	submit(
		connection,
		Action::Apply {
			plan_id: plan["id"].as_str().context("invalid saved plan")?.into(),
		},
	)?;
	Ok(())
}

fn submit(connection: &Connection, action: Action) -> Result<Operation> {
	let pending_path = connection.root.join(".leptoskit/control-request.json");
	let action_value = serde_json::to_value(&action)?;
	let intent = if pending_path.exists() {
		let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&pending_path)?)?;
		ensure!(
			saved["url"] == connection.url
				&& saved["project"] == connection.project
				&& saved["intent"]["action"] == action_value,
			"a previous request is pending; repeat it to reconnect before starting another \
			 operation"
		);
		serde_json::from_value::<Intent>(saved["intent"].clone())?
	} else {
		let overview: Overview =
			serde_json::from_value(connection.request("/api/v1/overview", None)?)?;
		let project = overview
			.projects
			.iter()
			.find(|p| p.id == connection.project)
			.context("project is unavailable on this service")?;
		let intent = Intent {
			expected_revision: project.revision,
			idempotency_key: uuid::Uuid::new_v4().to_string(),
			action,
		};
		private_write(
			&pending_path,
			&serde_json::to_vec(
				&serde_json::json!({"url":connection.url,"project":connection.project,"intent":intent}),
			)?,
		)?;
		intent
	};
	let response = connection.request(
		&format!("/api/v1/projects/{}/operations", connection.project),
		Some(&serde_json::to_value(intent)?),
	);
	if response
		.as_ref()
		.is_err_and(|error| error.downcast_ref::<Rejected>().is_some())
	{
		std::fs::remove_file(&pending_path)?;
	}
	let mut operation: Operation = serde_json::from_value(response?)?;
	println!("Operation {} · {}", operation.id, connection.url);
	while matches!(operation.phase, Phase::Queued | Phase::Running) {
		std::thread::sleep(std::time::Duration::from_secs(1));
		operation = serde_json::from_value(
			connection.request(&format!("/api/v1/operations/{}", operation.id), None)?,
		)?;
	}
	std::fs::remove_file(pending_path)?;
	println!("{}", serde_json::to_string_pretty(&operation)?);
	ensure!(
		operation.phase == Phase::Succeeded,
		"operation did not succeed; inspect its durable status before retrying"
	);
	Ok(operation)
}

#[cfg(test)]
mod tests;
