//! The command surface: parsing and dispatch.

use std::time::Duration;

use clap::Parser;
use clap::Subcommand;
use clap_complete::Shell;

use crate::build;
use crate::codepush;
use crate::dev;
use crate::doctor;
use crate::new_project;
use crate::serve_cmd;

/// Build, serve, and ship leptos apps: web, native, and over the air.
#[derive(Debug, Parser)]
#[command(name = "leptoskit", version, about, propagate_version = true)]
pub struct Cli {
	#[command(subcommand)]
	pub command: Command,
}

/// The subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
	/// Scaffolds a new project.
	New {
		/// The project directory; created when missing. Existing files are
		/// never overwritten.
		path: String,
		/// The platform to target by default.
		#[arg(long, default_value = "web")]
		platform: leptoskit_config::Platform,
		/// The app name; defaults to the directory name.
		#[arg(long)]
		name: Option<String>,
	},
	/// Builds into `dist/`, serves it, and rebuilds on change.
	Dev {
		/// Open the browser after the server starts.
		#[arg(long)]
		open: bool,
		/// Serve on a different port than the configuration says.
		#[arg(long)]
		port: Option<u16>,
	},
	/// Builds the project for a platform.
	Build {
		/// Build in release mode.
		#[arg(long)]
		release: bool,
		/// The platform to build; defaults to the configuration's.
		#[arg(long)]
		platform: Option<leptoskit_config::Platform>,
		/// The output directory, relative to the project root.
		#[arg(long, default_value = "dist")]
		out: String,
	},
	/// Serves the built output.
	Serve {
		/// The directory to serve; defaults to `dist/`.
		#[arg(long, default_value = "dist")]
		dir: String,
		/// Serve on a different port than the configuration says.
		#[arg(long)]
		port: Option<u16>,
	},
	/// Manages code push releases.
	Codepush {
		#[command(subcommand)]
		command: CodepushCommand,
	},
	/// Checks the local environment for missing tooling; exits non-zero when
	/// a required check fails.
	Doctor,
	/// Generates shell completions.
	Completions {
		/// The shell to generate for.
		shell: Shell,
	},
}

/// Code push subcommands.
#[derive(Debug, Subcommand)]
pub enum CodepushCommand {
	/// Packs a directory into a versioned release.
	Release {
		/// The directory to release; defaults to `dist/`.
		#[arg(long, default_value = "dist")]
		from: String,
		/// The release store directory; defaults to `releases/`.
		#[arg(long, default_value = "releases")]
		store: String,
		/// The release channel.
		#[arg(long, default_value = "production")]
		channel: String,
		/// Whether the release is mandatory for clients.
		#[arg(long)]
		mandatory: bool,
	},
	/// Serves a release store for clients to update from, on the configured
	/// `serve.address`.
	Serve {
		/// The release store directory; defaults to `releases/`.
		#[arg(long, default_value = "releases")]
		store: String,
		/// The static directory served next to the update API.
		#[arg(long, default_value = "dist")]
		dir: String,
		/// The port to serve on; defaults to 4100.
		#[arg(long)]
		port: Option<u16>,
	},
	/// Checks for updates against a store, like a client would.
	// `--version` names the client's version here, so the auto-generated
	// `--version` flag that `propagate_version` adds must step aside.
	#[command(disable_version_flag = true)]
	Check {
		/// The release store directory; defaults to `releases/`.
		#[arg(long, default_value = "releases")]
		store: String,
		/// The channel to check.
		#[arg(long, default_value = "production")]
		channel: String,
		/// The version a hypothetical client runs.
		#[arg(long, default_value = "0.1.0")]
		version: String,
		/// The build number a hypothetical client runs.
		#[arg(long, default_value_t = 1)]
		build: u64,
	},
}

/// Runs a parsed command.
///
/// # Errors
///
/// Returns an error when any step of the command fails.
pub fn run(cli: Cli) -> anyhow::Result<()> {
	match cli.command {
		Command::New {
			path,
			platform,
			name,
		} => new_project::run(&path, platform, name.as_deref()),
		Command::Dev { open, port } => dev::run(open, port),
		Command::Build {
			release,
			platform,
			out,
		} => build::run(release, platform, &out),
		Command::Serve { dir, port } => serve_cmd::run(&dir, port),
		Command::Codepush { command } => codepush::run(command),
		Command::Doctor => doctor::run(),
		Command::Completions { shell } => {
			let mut command = Cli::command();
			let name = command.get_name().to_owned();
			clap_complete::generate(shell, &mut command, name, &mut std::io::stdout());
			Ok(())
		}
	}
}

/// The debounce window for rebuilds in `leptoskit dev`.
pub const REBUILD_DEBOUNCE: Duration = Duration::from_millis(150);

use clap::CommandFactory;
