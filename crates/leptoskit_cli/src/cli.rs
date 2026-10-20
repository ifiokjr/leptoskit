//! The command surface: parsing and dispatch.

use std::ffi::OsStr;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use clap::Parser;
use clap::Subcommand;
use clap_complete::Shell;

use crate::build;
use crate::codepush;
use crate::dev;
use crate::doctor;
use crate::generate;
use crate::new_project;
use crate::s3::S3Location;
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
	/// Builds native application hosts and generates plugin registration.
	Native {
		#[command(subcommand)]
		command: crate::native::NativeCommand,
	},
	/// Runs component and device tests with retained failure artifacts.
	Test {
		#[command(subcommand)]
		command: crate::testing::TestCommand,
	},
	/// Generates an application or an independent runtime plugin crate.
	Generate {
		#[command(subcommand)]
		kind: GenerateCommand,
	},
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
		/// Local SDK repository for native application development.
		#[arg(long)]
		sdk_path: Option<String>,
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

/// Project generators share the existing application's scaffold implementation.
#[derive(Debug, Subcommand)]
pub enum GenerateCommand {
	/// Generates an application (also available as `leptoskit new`).
	App {
		/// Destination directory.
		path: String,
		/// Default build platform.
		#[arg(long, default_value = "web")]
		platform: leptoskit_config::Platform,
		/// Package name, otherwise taken from the directory.
		#[arg(long)]
		name: Option<String>,
		/// Local SDK repository for native applications before SDK publication.
		#[arg(long)]
		sdk_path: Option<String>,
	},
	/// Generates an independently publishable runtime plugin with an echo
	/// method.
	Plugin {
		/// Destination directory.
		path: String,
		/// Plugin name, otherwise taken from the directory.
		#[arg(long)]
		name: Option<String>,
		/// Local `leptoskit_bridge` crate for SDK development before
		/// publication.
		#[arg(long)]
		bridge_path: Option<String>,
	},
}

/// Code push subcommands.
#[derive(Debug, Subcommand)]
pub enum CodepushCommand {
	/// Packs a directory into a versioned release signed with the publisher
	/// key.
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
		/// The file holding the publisher signing key. Without it, the key is
		/// read from the `LEPTOSKIT_CODEPUSH_SIGNING_KEY` environment variable.
		#[arg(long, value_name = "PATH")]
		signing_key: Option<String>,
	},
	/// Creates a publisher signing key and prints its public key for
	/// `codepush.public_keys`.
	Keygen {
		/// Where to write the secret key; never overwritten. Keep it out of
		/// version control.
		#[arg(long, value_name = "PATH")]
		out: String,
	},
	/// Uploads a channel's releases to an S3-compatible bucket (Backblaze
	/// B2, Cloudflare R2, AWS S3), so a CDN can serve updates with no server.
	///
	/// Credentials come from `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and
	/// optionally `AWS_SESSION_TOKEN`. Release files are uploaded before the
	/// channel's `index.json`, so devices never see a half-published release.
	Publish {
		/// The release store directory; defaults to `releases/`.
		#[arg(long, default_value = "releases")]
		store: String,
		/// The channel to publish.
		#[arg(long, default_value = "production")]
		channel: String,
		#[command(flatten)]
		location: S3Location,
	},
	/// Copies a channel's newest release from an S3-compatible bucket into
	/// the store, verified against `codepush.public_keys`, so `release` can
	/// number the next build and delta from it on a runner that starts empty.
	///
	/// Uses the same credentials as `publish`.
	Pull {
		/// The release store directory; defaults to `releases/`.
		#[arg(long, default_value = "releases")]
		store: String,
		/// The channel to pull.
		#[arg(long, default_value = "production")]
		channel: String,
		#[command(flatten)]
		location: S3Location,
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

/// The binary entry point, shared by the `leptoskit` and `lk` targets: both
/// are one-line wrappers over this.
pub fn main() {
	let cli = Cli::parse();
	if let Err(error) = run(cli) {
		eprintln!("error: {error:#}");
		std::process::exit(1);
	}
}

/// Runs a parsed command.
///
/// # Errors
///
/// Returns an error when any step of the command fails.
pub fn run(cli: Cli) -> anyhow::Result<()> {
	match cli.command {
		Command::Native { command } => crate::native::run(command),
		Command::Test { command } => crate::testing::run(command),
		Command::Generate { kind } => {
			match kind {
				GenerateCommand::App {
					path,
					platform,
					name,
					sdk_path,
				} => {
					if platform == leptoskit_config::Platform::Native {
						crate::native::scaffold::run(&path, name.as_deref(), sdk_path.as_deref())
					} else {
						anyhow::ensure!(
							sdk_path.is_none(),
							"--sdk-path is only supported for native apps"
						);
						new_project::run(&path, platform, name.as_deref())
					}
				}
				GenerateCommand::Plugin {
					path,
					name,
					bridge_path,
				} => generate::plugin(&path, name.as_deref(), bridge_path.as_deref()),
			}
		}
		Command::New {
			path,
			platform,
			name,
			sdk_path,
		} => {
			run(Cli {
				command: Command::Generate {
					kind: GenerateCommand::App {
						path,
						platform,
						name,
						sdk_path,
					},
				},
			})
		}
		Command::Dev { open, port } => dev::run(open, port),
		Command::Build {
			release,
			platform,
			out,
		} => build::command(release, platform, &out),
		Command::Serve { dir, port } => serve_cmd::run(&dir, port),
		Command::Codepush { command } => codepush::run(command),
		Command::Doctor => doctor::run(),
		Command::Completions { shell } => {
			let mut command = Cli::command();
			// Completions follow the invoked name, so `lk completions zsh`
			// completes `lk` rather than the canonical command.
			let name = invoked_name().to_owned();
			clap_complete::generate(shell, &mut command, name, &mut std::io::stdout());
			Ok(())
		}
	}
}

/// The name the user invoked the binary as: `lk` when run through the alias
/// binary, `leptoskit` otherwise (including library callers, whose `argv[0]`
/// is some test binary). Output that echoes the command — completion scripts,
/// the scaffold's next-step hint — uses it so it matches what the user typed;
/// clap already derives usage strings from `argv[0]` on its own.
pub fn invoked_name() -> &'static str {
	static INVOKED: LazyLock<&'static str> = LazyLock::new(|| {
		let invoked_as_lk = std::env::args_os()
			.next()
			.as_deref()
			.and_then(|argv0| Path::new(argv0).file_stem())
			.is_some_and(|stem| stem == OsStr::new("lk"));
		if invoked_as_lk { "lk" } else { "leptoskit" }
	});
	&INVOKED
}

/// The debounce window for rebuilds in `leptoskit dev`.
pub const REBUILD_DEBOUNCE: Duration = Duration::from_millis(150);

use clap::CommandFactory;
