//! One optional deployment service, reusing the existing infrastructure engine.
mod client;
mod hosted_worker;
mod runner;

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_deploy_console::control::Project;
use leptoskit_deploy_console::control::Scope;
use leptoskit_deploy_console::control::Service;
use leptoskit_deploy_console::control::Store;

#[derive(Debug, clap::Subcommand)]
pub enum ConsoleCommand {
	/// Serves the dashboard, durable jobs and analytics history for trusted
	/// repositories.
	Serve {
		#[arg(long, required = true)]
		project: Vec<PathBuf>,
		#[arg(long)]
		data: Option<PathBuf>,
		#[arg(long, default_value = "127.0.0.1:4700")]
		bind: std::net::SocketAddr,
		/// Public HTTPS origin, or localhost HTTP for local use.
		#[arg(long, default_value = "http://localhost:4700")]
		url: String,
	},
	/// Links this app to a service. Reads `LEPTOSKIT_CONTROL_KEY`; stores it
	/// outside the repository.
	Connect {
		#[arg(long)]
		url: String,
	},
	/// Reads projects and recent operations from the connected service.
	Status,
	/// Runs a dedicated outbound worker paired with the hosted `LeptosKit` app.
	Worker {
		#[arg(long)]
		url: String,
		#[arg(long)]
		project: PathBuf,
		#[arg(long)]
		data: PathBuf,
	},
	/// Issues a project-scoped key; prints its value once. Store it as a
	/// secret.
	Token {
		#[arg(long)]
		project: String,
		#[arg(long, value_enum)]
		scope: TokenScope,
	},
	/// Makes a consistent local backup of the management database.
	Backup {
		#[arg(long)]
		data: Option<PathBuf>,
		#[arg(long)]
		out: PathBuf,
	},
	/// Restores a verified management backup offline and revokes restored
	/// tokens.
	Restore {
		#[arg(long)]
		data: Option<PathBuf>,
		#[arg(long)]
		from: PathBuf,
		#[arg(long)]
		previous: PathBuf,
		#[arg(long)]
		yes: bool,
	},
	/// Reverses the management schema offline, after making a new backup.
	/// Deletes its history.
	RollbackDatabase {
		#[arg(long)]
		data: Option<PathBuf>,
		#[arg(long)]
		backup: PathBuf,
		#[arg(long)]
		yes: bool,
	},
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum TokenScope {
	Read,
	Deploy,
	Ingest,
}

pub fn run(command: ConsoleCommand) -> Result<()> {
	match command {
		ConsoleCommand::Serve {
			project,
			data,
			bind,
			url,
		} => {
			let data = data_directory(data)?;
			let lock = directory_lock(&data)?;
			let store = Store::open(&data.join("console.sqlite3"))?;
			for root in project {
				let root = root
					.canonicalize()
					.context("registered repository does not exist")?;
				let config = LeptoskitConfig::load_from(&root)?;
				let infra = config.infra.context("registered app needs [infra]")?;
				let environment = infra
					.plugins
					.analytics
					.map_or_else(|| "production".into(), |a| a.environment);
				store.register(&Project {
					id: infra.name,
					environment,
					root,
					revision: 0,
				})?;
			}
			let key = std::env::var("LEPTOSKIT_CONTROL_KEY").map_err(|_| {
				anyhow::anyhow!(
					"set LEPTOSKIT_CONTROL_KEY to a random owner credential of at least 32 \
					 characters"
				)
			})?;
			let service = Service::new(store, &key, &url)?;
			let runtime = tokio::runtime::Builder::new_multi_thread()
				.worker_threads(4)
				.enable_all()
				.build()?;
			runtime.block_on(async {
				let mut worker = service.worker(Arc::new(runner::CommandRunner::new(
					data,
					std::env::current_exe()?,
					service.clone(),
				)))?;
				let listener = tokio::net::TcpListener::bind(bind).await?;
				println!("LeptosKit deployment service: {url}");
				let result=tokio::select!{
					result=leptoskit_axum::serve_listener(listener, service.router())=>result,
					_=&mut worker=>anyhow::bail!("deployment worker stopped; restart the service and reconcile interrupted operations"),
				};
				worker.abort();
				let _ = worker.await;
				result?;
				Ok::<_, anyhow::Error>(())
			})?;
			drop(lock);
			Ok(())
		}
		ConsoleCommand::Connect { url } => client::connect(&url),
		ConsoleCommand::Status => client::status(),
		ConsoleCommand::Worker { url, project, data } => hosted_worker::run(&url, &project, &data),
		ConsoleCommand::Token { project, scope } => {
			client::token(
				&project,
				match scope {
					TokenScope::Read => Scope::Read,
					TokenScope::Deploy => Scope::Deploy,
					TokenScope::Ingest => Scope::Ingest,
				},
			)
		}
		ConsoleCommand::Backup { data, out } => {
			let data = data_directory(data)?;
			Store::open(&data.join("console.sqlite3"))?.backup(&out)
		}
		ConsoleCommand::Restore {
			data,
			from,
			previous,
			yes,
		} => {
			ensure!(
				yes,
				"restoring replaces the management journal; pass --yes with a new \
				 previous-database backup path"
			);
			let data = data_directory(data)?;
			let _lock = directory_lock(&data)?;
			Store::restore(&data.join("console.sqlite3"), &from, &previous)?;
			println!(
				"Restored the management database and revoked restored tokens. Rotate the owner \
				 key and redeploy apps to refresh their ingestion keys."
			);
			Ok(())
		}
		ConsoleCommand::RollbackDatabase { data, backup, yes } => {
			ensure!(
				yes,
				"management schema rollback deletes its history; pass --yes with a new backup path"
			);
			let data = data_directory(data)?;
			let _lock = directory_lock(&data)?;
			Store::open(&data.join("console.sqlite3"))?.rollback(&backup)
		}
	}
}

fn data_directory(explicit: Option<PathBuf>) -> Result<PathBuf> {
	if let Some(path) = explicit {
		return Ok(path);
	}
	Ok(
		PathBuf::from(std::env::var_os("HOME").context("set HOME or pass --data")?)
			.join(".local/share/leptoskit/console"),
	)
}

fn directory_lock(path: &Path) -> Result<std::fs::File> {
	#[cfg(unix)]
	let existed = path.exists();
	std::fs::create_dir_all(path)?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		if !existed {
			std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
		}
		ensure!(
			std::fs::metadata(path)?
				.permissions()
				.mode()
				.trailing_zeros()
				>= 6,
			"use a dedicated private management data directory"
		);
	}
	let file = std::fs::OpenOptions::new()
		.create(true)
		.truncate(false)
		.write(true)
		.open(path.join("service.lock"))?;
	fs4::FileExt::try_lock(&file)
		.context("the deployment service is running; stop it before offline maintenance")?;
	Ok(file)
}

pub fn deploy(args: &crate::infra::ReleaseArgs) -> Result<()> {
	if let Some(connection) = client::Connection::for_project()? {
		return client::deploy(&connection, args);
	}
	crate::infra::release(args)
}

pub fn infra(command: crate::infra::InfraCommand) -> Result<()> {
	if matches!(
		command,
		crate::infra::InfraCommand::Plan
			| crate::infra::InfraCommand::Apply {
				offline: false,
				yes: false
			}
			| crate::infra::InfraCommand::Status
			| crate::infra::InfraCommand::Analytics
	) && let Some(connection) = client::Connection::for_project()?
	{
		match command {
			crate::infra::InfraCommand::Plan => return client::plan(&connection),
			crate::infra::InfraCommand::Apply {
				offline: false,
				yes: false,
			} => return client::apply(&connection),
			crate::infra::InfraCommand::Status => return client::status(),
			crate::infra::InfraCommand::Analytics => return client::analytics(&connection),
			_ => {}
		}
	}
	crate::infra::run(command)
}
