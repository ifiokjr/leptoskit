//! The command surface: parsing and dispatch.

use std::ffi::OsStr;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use clap::Parser;
use clap::Subcommand;
use clap_complete::Shell;
use leptoskit_fmt::cli::Outcome;

use crate::build;
use crate::codepush;
use crate::dev;
use crate::doctor;
use crate::rpc;
use crate::generate;
use crate::new_project;
use crate::routes;
use crate::s3::S3Location;
use crate::serve_cmd;

/// Build, serve, and ship leptos apps: web, native, and over the air.
#[derive(Debug, Parser)]
#[command(name = "leptoskit", version, about, propagate_version = true)]
pub struct Cli {
	#[command(subcommand)]
	pub command: Command,
}

/// RPC subcommands.
///
/// The RPC is described by the server binary itself: unless `--from` names a
/// spec file or URL, these commands build the server with `leptoskit build`'s
/// settings and run it with `LEPTOSKIT_RPC_SPEC_OUT` set, which
/// `leptoskit_rpc::spec::export_if_requested()` in `main` answers by writing
/// the spec and exiting.
#[derive(Debug, Subcommand)]
pub enum RpcCommand {
	/// Exports the RPC specification.
	Spec {
		/// Write to this file (relative to the project root) instead of stdout.
		#[arg(long)]
		out: Option<String>,
		/// The document to write.
		#[arg(long, value_enum, default_value_t = SpecFormat::Json)]
		format: SpecFormat,
		/// Read the spec from a file or URL instead of running the server.
		#[arg(long)]
		from: Option<String>,
		/// Include `#[api(internal)]` functions.
		#[arg(long)]
		include_internal: bool,
	},
	/// Lists the published functions with their routes.
	List {
		/// Read the spec from a file or URL instead of running the server.
		#[arg(long)]
		from: Option<String>,
		/// Include `#[api(internal)]` functions.
		#[arg(long)]
		include_internal: bool,
	},
	/// Generates typed clients that speak the RPC's own protocol.
	///
	/// Targets: `typescript`, `python`, `go`, `rust`, `dart`, `cli` (a command
	/// line with one command per function), and `openapi`. Without `--target`,
	/// the `rpc.targets` from leptoskit.toml are generated, or every target
	/// when none is configured.
	Generate {
		/// A target to generate; repeatable.
		#[arg(long)]
		target: Vec<String>,
		/// The output directory, relative to the project root; only with a
		/// single `--target`.
		#[arg(long)]
		out: Option<String>,
		/// The package name (npm, PyPI, Go module, crate, or Dart).
		#[arg(long)]
		package: Option<String>,
		/// The origin the client points at by default.
		#[arg(long)]
		server_url: Option<String>,
		/// Read the spec from a file or URL instead of running the server.
		#[arg(long)]
		from: Option<String>,
		/// Include `#[api(internal)]` functions.
		#[arg(long)]
		include_internal: bool,
		/// How much to write: `create` writes generated code always and
		/// scaffolding (manifests, READMEs) only where missing; `full` writes
		/// everything, overwriting; `files` writes generated code only and
		/// leaves the package definition to you. Overrides
		/// `rpc.targets[].scaffold`.
		#[arg(long)]
		scaffold: Option<String>,
		/// Also `cargo check` the generated Rust and CLI crates.
		#[arg(long)]
		build: bool,
	},
}

/// What `leptoskit rpc spec` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SpecFormat {
	/// The leptoskit specification, which `--from` reads back.
	Json,
	/// The OpenAPI 3.1 document as JSON.
	Openapi,
	/// The OpenAPI 3.1 document as YAML.
	OpenapiYaml,
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
	/// Describes the published RPC and generates clients for it.
	Rpc {
		#[command(subcommand)]
		command: RpcCommand,
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
		/// Generate a native app for one platform, e.g. harmonyos (requires
		/// --platform native).
		#[arg(long)]
		native_target: Option<leptoskit_config::NativeTarget>,
		/// The app name; defaults to the directory name.
		#[arg(long)]
		name: Option<String>,
		/// A local leptoskit repository whose crates the project depends on by
		/// path instead of the published release.
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
		/// How hard to optimize a release client for size, `fast` or
		/// `smallest`; defaults to `bundle.optimize`. Needs `--release`.
		#[arg(long, requires = "release")]
		optimize: Option<leptoskit_config::Optimize>,
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
	/// Writes the typed `Route` module from the routes directory.
	///
	/// Lists the routes after writing. `dev`, `build` and `native` write the
	/// module on their own; run this after adding a route file, or with
	/// `--check` in CI.
	Routes {
		/// Fail instead of writing when the committed module is out of date.
		#[arg(long)]
		check: bool,
	},
	/// Manages code push releases.
	Codepush {
		#[command(subcommand)]
		command: CodepushCommand,
	},
	/// Checks the local environment for missing tooling; exits non-zero when
	/// a required check fails.
	Doctor,
	/// Formats Rust, and the markup in `view!` macros, with the project's
	/// `rustfmt.toml`.
	Format(leptoskit_fmt::cli::FormatArgs),
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
		/// Generate a native app for one platform, e.g. harmonyos (requires
		/// --platform native).
		#[arg(long)]
		native_target: Option<leptoskit_config::NativeTarget>,
		/// Package name, otherwise taken from the directory.
		#[arg(long)]
		name: Option<String>,
		/// A local leptoskit repository whose crates the project depends on by
		/// path instead of the published release.
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
		Command::Rpc { command } => rpc::run(command),
		Command::Generate { kind } => {
			match kind {
				GenerateCommand::App {
					path,
					platform,
					native_target,
					name,
					sdk_path,
				} => {
					if let Some(target) = native_target {
						anyhow::ensure!(
							platform == leptoskit_config::Platform::Native,
							"--native-target requires --platform native"
						);
						return crate::native::scaffold::run_for_target(
							&path,
							name.as_deref(),
							sdk_path.as_deref(),
							Some(target),
						);
					}
					new_project::run_with_sdk(&path, platform, name.as_deref(), sdk_path.as_deref())
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
			native_target,
			name,
			sdk_path,
		} => {
			run(Cli {
				command: Command::Generate {
					kind: GenerateCommand::App {
						path,
						platform,
						native_target,
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
			optimize,
			out,
		} => build::command(release, platform, optimize, &out),
		Command::Serve { dir, port } => serve_cmd::run(&dir, port),
		Command::Routes { check } => routes::command(check),
		Command::Codepush { command } => codepush::run(command),
		Command::Doctor => doctor::run(),
		Command::Format(args) => {
			match leptoskit_fmt::cli::execute(&args) {
				Outcome::Formatted => Ok(()),
				Outcome::WouldFormat(files) => {
					anyhow::bail!(
						"{files} file(s) would be reformatted; run `{} format`",
						invoked_name()
					)
				}
				Outcome::Failed => anyhow::bail!("formatting failed"),
			}
		}
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
