use std::path::PathBuf;

use leptoskit_deploy_console::control::*;
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;

pub struct CommandRunner {
	data: PathBuf,
	binary: PathBuf,
	service: Service,
	pub hosted_bindings: Option<(String, String)>,
	pub hosted_state: Option<(String, String)>,
}
impl CommandRunner {
	pub fn new(data: PathBuf, binary: PathBuf, service: Service) -> Self {
		Self {
			data,
			binary,
			service,
			hosted_bindings: None,
			hosted_state: None,
		}
	}
}

impl Executor for CommandRunner {
	fn run(
		&self,
		project: Project,
		operation: Operation,
	) -> std::pin::Pin<Box<dyn Future<Output = RunResult> + Send>> {
		let data = self.data.clone();
		let binary = self.binary.clone();
		let service = self.service.clone();
		let bindings = self.hosted_bindings.clone();
		let state = self.hosted_state.clone();
		Box::pin(async move {
			let checkout = checkout(&data, &project, &operation).await;
			let result = match &checkout {
				Ok(source) => {
					let environment = Environment {
						data: &data,
						binary: &binary,
						service: &service,
						bindings: bindings.as_ref(),
						backend: state.as_ref(),
					};
					execute(
						&environment,
						source.as_ref().unwrap_or(&project),
						&operation,
					)
					.await
				}
				Err(error) => {
					Err(anyhow::anyhow!(
						"could not prepare the selected source: {error}"
					))
				}
			};
			let workspace = data.join("workspaces").join(&operation.id);
			if workspace.exists() {
				let _ = tokio::process::Command::from(crate::git::command(&project.root))
					.args(["worktree", "remove", "--force"])
					.arg(&workspace)
					.stdout(std::process::Stdio::null())
					.stderr(std::process::Stdio::null())
					.status()
					.await;
			}
			match result {
				Ok(result) => result,
				Err(_) => {
					RunResult {
						succeeded: false,
						message: "Worker failed before completing the operation. Inspect its \
						          private run log and reconcile the app's shared operation status \
						          before retrying."
							.into(),
						details: serde_json::json!({"code":"worker_failed"}),
					}
				}
			}
		})
	}
}

#[derive(Clone, Copy)]
struct Environment<'a> {
	data: &'a std::path::Path,
	binary: &'a std::path::Path,
	service: &'a Service,
	bindings: Option<&'a (String, String)>,
	backend: Option<&'a (String, String)>,
}
async fn execute(
	environment: &Environment<'_>,
	project: &Project,
	operation: &Operation,
) -> anyhow::Result<RunResult> {
	if let Action::Deploy { version, .. } = &operation.intent.action
		&& git_commit(version)
	{
		let prebuilt = Operation {
			intent: Intent {
				action: Action::Status,
				..operation.intent.clone()
			},
			..operation.clone()
		};
		let args = vec![
			"infra".to_owned(),
			"build".into(),
			"--version".into(),
			version.clone(),
			"--push".into(),
			"--reuse-existing".into(),
		];
		let built = run(environment, project, &prebuilt, &args, "build").await?;
		if !built.succeeded {
			return Ok(built);
		}
	}
	run(
		environment,
		project,
		operation,
		&arguments(&operation.intent.action),
		"run",
	)
	.await
}

async fn run(
	environment: &Environment<'_>,
	project: &Project,
	operation: &Operation,
	args: &[String],
	stage: &str,
) -> anyhow::Result<RunResult> {
	let Environment {
		data,
		binary,
		service,
		bindings,
		backend,
	} = *environment;
	let directory = data.join("logs");
	service.progress(
		&operation.id,
		if stage == "build" {
			"Building the selected Git commit or reusing its verified image"
		} else {
			"Running the requested operation through the existing deployment engine"
		},
	)?;
	std::fs::create_dir_all(&directory)?;
	prune_logs(&directory)?;
	let config = leptoskit_config::LeptoskitConfig::load_from(&project.root)?;
	let plugin = config
		.infra
		.as_ref()
		.and_then(|c| c.plugins.analytics.as_ref());
	let mut command = tokio::process::Command::new(binary);
	command
		.args(args)
		.current_dir(&project.root)
		.env("LEPTOSKIT_CONTROL_ORIGIN", service.origin());
	if let Some((address, key)) = backend {
		command
			.env("TF_HTTP_ADDRESS", address)
			.env("TF_HTTP_LOCK_ADDRESS", address)
			.env("TF_HTTP_UNLOCK_ADDRESS", address)
			.env("TF_HTTP_USERNAME", "worker")
			.env("TF_HTTP_PASSWORD", key)
			.env("TF_HTTP_LOCK_METHOD", "LOCK")
			.env("TF_HTTP_UNLOCK_METHOD", "UNLOCK");
	}
	if let Some(plugin) = plugin {
		command.env(
			"LEPTOSKIT_ANALYTICS_READ_KEY",
			bindings
				.map(|b| b.0.clone())
				.map_or_else(|| service.managed_app_key(&project.id, false), Ok)?,
		);
		if plugin.otlp_endpoint.is_none() {
			command.env(
				"LEPTOSKIT_ANALYTICS_WRITE_KEY",
				bindings
					.map(|b| b.1.clone())
					.map_or_else(|| service.managed_app_key(&project.id, true), Ok)?,
			);
		}
	}
	command
		.env_remove("LEPTOSKIT_PAIRING_CODE")
		.env_remove("LEPTOSKIT_HOSTED_KEY");
	let stdout_file = log_file(directory.join(format!("{}.{stage}.out", operation.id)))?;
	let stderr_file = log_file(directory.join(format!("{}.{stage}.err", operation.id)))?;
	let mut child = command
		.env_remove("LEPTOSKIT_CONTROL_KEY")
		.env("LEPTOSKIT_CONTROL_LOCAL", "1")
		.stdin(std::process::Stdio::null())
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped())
		.kill_on_drop(true)
		.spawn()?;
	let stdout = child
		.stdout
		.take()
		.ok_or_else(|| anyhow::anyhow!("missing runner output"))?;
	let stderr = child
		.stderr
		.take()
		.ok_or_else(|| anyhow::anyhow!("missing runner error stream"))?;
	let (out, err, status) = tokio::join!(
		capture(stdout, stdout_file),
		capture(stderr, stderr_file),
		child.wait()
	);
	out?;
	err?;
	let succeeded = status?.success();
	let mut details = serde_json::json!({"code":if succeeded{"completed"}else{"command_failed"}});
	if succeeded && matches!(operation.intent.action, Action::Plan) {
		let config = leptoskit_config::LeptoskitConfig::load_from(&project.root)?;
		let infra = config
			.infra
			.ok_or_else(|| anyhow::anyhow!("missing infra"))?;
		let output = tokio::process::Command::new("tofu")
			.args(["show", "-json", "leptoskit.plan"])
			.current_dir(project.root.join("infra").join(infra.name))
			.stdin(std::process::Stdio::null())
			.output()
			.await?;
		anyhow::ensure!(
			output.status.success() && output.stdout.len() <= 32 * 1024 * 1024,
			"could not inspect the saved plan"
		);
		let plan: serde_json::Value = serde_json::from_slice(&output.stdout)?;
		let changes = plan["resource_changes"]
			.as_array()
			.into_iter()
			.flatten()
			.filter(|change| change["change"]["actions"] != serde_json::json!(["no-op"]))
			.map(
				|change| serde_json::json!({"address":change["address"],"actions":change["change"]["actions"]}),
			)
			.collect::<Vec<_>>();
		details["changes"] = serde_json::json!(changes);
	}
	Ok(RunResult {
		succeeded,
		message: if succeeded {
			"Completed. The existing release engine verified its normal safety checks."
		} else {
			"Operation failed. Inspect the worker's private log and shared app status before \
			 retrying; credentials and raw tool output are kept off the API."
		}
		.into(),
		details,
	})
}

fn git_commit(version: &str) -> bool {
	matches!(version.len(), 40 | 64) && version.bytes().all(|b| b.is_ascii_hexdigit())
}

async fn checkout(
	data: &std::path::Path,
	project: &Project,
	operation: &Operation,
) -> anyhow::Result<Option<Project>> {
	let Action::Deploy { version, .. } = &operation.intent.action else {
		return Ok(None);
	};
	if !git_commit(version) {
		return Ok(None);
	}
	let exists = tokio::process::Command::from(crate::git::command(&project.root))
		.args(["cat-file", "-e", &format!("{version}^{{commit}}")])
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null())
		.status()
		.await?;
	if !exists.success() {
		let fetched = tokio::process::Command::from(crate::git::command(&project.root))
			.args(["fetch", "--no-tags", "origin", version])
			.stdout(std::process::Stdio::null())
			.stderr(std::process::Stdio::null())
			.status()
			.await?;
		anyhow::ensure!(
			fetched.success(),
			"selected commit is unavailable in the registered repository"
		);
	}
	let workspaces = data.join("workspaces");
	std::fs::create_dir_all(&workspaces)?;
	let path = workspaces.join(&operation.id);
	let added = tokio::process::Command::from(crate::git::command(&project.root))
		.args(["worktree", "add", "--detach"])
		.arg(&path)
		.arg(version)
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null())
		.status()
		.await?;
	anyhow::ensure!(
		added.success(),
		"could not create an isolated source checkout"
	);
	let config = leptoskit_config::LeptoskitConfig::load_from(&path)?;
	let original = leptoskit_config::LeptoskitConfig::load_from(&project.root)?;
	let infra = config
		.infra
		.as_ref()
		.ok_or_else(|| anyhow::anyhow!("source has no managed infrastructure"))?;
	let mut proposed = config.infra.clone();
	let mut policy = original.infra.clone();
	if let Some(infra) = &mut proposed {
		infra.control_url = None;
	}
	if let Some(infra) = &mut policy {
		infra.control_url = None;
	}
	anyhow::ensure!(
		infra.name == project.id && proposed == policy,
		"source infrastructure differs from the registered policy; update and re-plan the trusted \
		 repository first"
	);
	let shared = project.root.join("infra").join(&project.id);
	std::fs::create_dir_all(&shared)?;
	let parent = path.join("infra");
	std::fs::create_dir_all(&parent)?;
	let destination = parent.join(&project.id);
	if destination.exists() {
		std::fs::remove_dir_all(&destination)?;
	}
	#[cfg(unix)]
	{
		std::os::unix::fs::symlink(shared, destination)?;
		Ok(Some(Project {
			root: path,
			..project.clone()
		}))
	}
	#[cfg(not(unix))]
	anyhow::bail!("managed source workspaces currently require a Unix worker");
}

fn arguments(action: &Action) -> Vec<String> {
	let text = match action {
		Action::Plan => vec!["infra", "plan"],
		Action::Apply { .. } => vec!["infra", "apply"],
		Action::Deploy {
			version,
			setup,
			maintenance,
		} => {
			let mut args = vec!["deploy", "--version", version.as_str()];
			if *setup {
				args.push("--setup");
			}
			if *maintenance {
				args.push("--maintenance");
			}
			args
		}
		Action::Rollback { version } => vec!["deploy", "--rollback", version.as_str()],
		Action::Status => vec!["infra", "status"],
		Action::Backup => vec!["infra", "backups", "verify"],
	};
	text.into_iter().map(str::to_owned).collect()
}

fn log_file(path: PathBuf) -> std::io::Result<tokio::fs::File> {
	let mut options = std::fs::OpenOptions::new();
	options.write(true).create_new(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt as _;
		options.mode(0o600);
	}
	Ok(tokio::fs::File::from_std(options.open(path)?))
}

async fn capture(
	mut stream: impl tokio::io::AsyncRead + Unpin,
	mut file: tokio::fs::File,
) -> anyhow::Result<()> {
	let mut remaining = 4 * 1024 * 1024;
	let mut truncated = false;
	let mut buffer = vec![0; 8192];
	loop {
		let length = stream.read(&mut buffer).await?;
		if length == 0 {
			break;
		}
		let saved = remaining.min(length);
		file.write_all(&buffer[..saved]).await?;
		remaining -= saved;
		if saved < length && !truncated {
			file.write_all(b"\n[output truncated at 4 MiB]\n").await?;
			truncated = true;
		}
	}
	file.flush().await?;
	Ok(())
}

fn prune_logs(directory: &std::path::Path) -> anyhow::Result<()> {
	let mut files = std::fs::read_dir(directory)?
		.filter_map(Result::ok)
		.filter(|entry| {
			matches!(
				entry.path().extension().and_then(std::ffi::OsStr::to_str),
				Some("out" | "err")
			)
		})
		.map(|entry| Ok((entry.metadata()?.modified()?, entry.path())))
		.collect::<anyhow::Result<Vec<_>>>()?;
	files.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
	for (_, path) in files.into_iter().skip(64) {
		std::fs::remove_file(path)?;
	}
	Ok(())
}

#[cfg(test)]
mod tests;
