//! Operator entrypoint. No customer build, shell, or provider credential runs
//! here.
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use clap::Parser;
use clap::Subcommand;
use leptoskit_deploy_console::hosted::Billing;
use leptoskit_deploy_console::hosted::Hosted;
use leptoskit_deploy_console::hosted::Ownership;

#[derive(Parser)]
struct Args {
	#[arg(long, default_value = ".leptoskit-cloud")]
	data: PathBuf,
	#[arg(long, default_value = "http://localhost:4701")]
	url: String,
	#[command(subcommand)]
	command: Command,
}
#[derive(Subcommand)]
enum Command {
	/// Restore all journals into a new data directory and revoke restored
	/// access.
	Restore {
		#[arg(long)]
		from: PathBuf,
	},
	/// Reverse the hosted schema offline, deleting hosted metadata after
	/// backup.
	RollbackSchema {
		#[arg(long)]
		backup: PathBuf,
		#[arg(long)]
		yes: bool,
	},
	Serve {
		#[arg(long, default_value = "127.0.0.1:4701")]
		bind: std::net::SocketAddr,
		#[arg(long)]
		assets: PathBuf,
		/// The documentation book; defaults to the docs directory in --assets.
		#[arg(long)]
		docs: Option<PathBuf>,
	},
	/// Enroll a private-pilot account using an existing Stripe customer ID.
	Account {
		#[arg(long)]
		id: String,
		#[arg(long)]
		customer: String,
	},
	RotateKey {
		#[arg(long)]
		account: String,
	},
	/// Install a dedicated, operator-owned `DigitalOcean` worker after an
	/// approved quote.
	Managed {
		#[arg(long)]
		account: String,
		#[arg(long)]
		project: String,
		#[arg(long, default_value = "production")]
		environment: String,
	},
	/// Reconcile a worker operation after inspecting actual provider/release
	/// state.
	Interrupt {
		#[arg(long)]
		account: String,
		#[arg(long)]
		operation: String,
	},
	Backup {
		#[arg(long)]
		out: PathBuf,
	},
}
#[tokio::main]
async fn main() -> Result<()> {
	let args = Args::parse();
	let secret = std::env::var("LEPTOSKIT_HOSTED_KEY").context(
		"set LEPTOSKIT_HOSTED_KEY to a persistent random 32+ character encryption key; back it up \
		 separately",
	)?;
	if let Command::Restore { from } = &args.command {
		return Hosted::restore_new(from, &args.data, secret);
	}
	let billing = match std::env::var("STRIPE_SECRET_KEY") {
		Ok(key) => {
			Some(Billing::new(
				key,
				std::env::var("STRIPE_WEBHOOK_SECRET")?,
				std::env::var("STRIPE_CONTROL_PRICE")?,
				std::env::var("STRIPE_TEAM_PRICE")?,
			)?)
		}
		Err(_) => None,
	};
	if matches!(args.command, Command::Serve { .. }) {
		ensure!(
			billing.is_some(),
			"configure Stripe keys and both recurring prices before serving hosted subscriptions"
		);
	}
	let host = Hosted::open(&args.data, secret, &args.url, billing)?;
	// One broker process per data directory; SQLite handles request-level
	// serialization.
	let lock = std::fs::OpenOptions::new()
		.create(true)
		.truncate(false)
		.write(true)
		.open(args.data.join("broker.lock"))?;

	if matches!(
		args.command,
		Command::Backup { .. } | Command::RollbackSchema { .. }
	) {
		lock.try_lock()
			.context("stop the broker before offline maintenance")?;
	} else {
		lock.try_lock_shared()
			.context("offline maintenance uses this directory")?;
	}

	match args.command {
		Command::Serve { bind, assets, docs } => {
			let serving = std::fs::OpenOptions::new()
				.create(true)
				.truncate(false)
				.write(true)
				.open(args.data.join("server.lock"))?;
			serving
				.try_lock()
				.context("hosted broker already running")?;
			let docs = docs.unwrap_or_else(|| assets.join("docs"));
			let app = leptoskit_app::site::router(host.router(), &assets, &docs, &args.url)?;
			let listener = tokio::net::TcpListener::bind(bind).await?;
			println!("LeptosKit hosted broker: {}", args.url);
			axum::serve(listener, app).await?;
		}
		Command::Account { id, customer } => {
			println!("{}", host.accounts().create(&id, &customer)?);
		}
		Command::RotateKey { account } => {
			println!("{}", host.accounts().rotate(&account)?);
		}
		Command::Managed {
			account,
			project,
			environment,
		} => {
			host.accounts().enable_managed(&account, true)?;
			host.register(
				&account,
				&project,
				&environment,
				Ownership::LeptoskitDigitalocean,
			)?;
			println!("{}", host.accounts().pairing(&account, &project)?);
		}
		Command::Interrupt { account, operation } => {
			host.interrupt(&account, &operation)?;
			println!(
				"Marked interrupted; the worker's pending marker must be reconciled separately."
			);
		}
		Command::Backup { out } => host.backup(&out)?,
		Command::RollbackSchema { backup, yes } => {
			ensure!(
				yes,
				"schema rollback deletes hosted account/state history; pass --yes with a new \
				 backup directory"
			);
			host.rollback_schema(&backup)?;
		}
		Command::Restore { .. } => unreachable!("restore handled before opening data"),
	}
	drop(lock);
	Ok(())
}
