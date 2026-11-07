//! Managed deployment: `OpenTofu` resources, Kamal releases, and existing code
//! push.
//!
//! No credentials are generated into configuration, plans or logs by this
//! module. `OpenTofu`'s own state/plan files can contain secrets and are
//! private.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::infra::InfraConfig;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

mod analytics;
mod build;
mod ci;
mod operations;
mod plugins;
mod pricing;
mod recovery;
mod releases;
mod remote;
mod resources;
mod runtime;
mod verified;
use resources::resources;

/// Managed infrastructure operations. `lk tofu` is an alias.
#[derive(Debug, clap::Subcommand)]
pub enum InfraCommand {
	/// Reproduce and package a backend without rebuilding it in the image.
	BuildVerified(verified::Args),
	/// Queries read-only JSON evidence from one app instance, not a fleet
	/// total.
	Analytics,
	/// Prints the versioned capability catalog, enabled plugins and bindings as
	/// JSON.
	Plugins,
	/// Builds the server/website image with migration metadata; optionally
	/// pushes it.
	#[command(disable_version_flag = true)]
	Build {
		/// Unique immutable image tag.
		#[arg(long)]
		version: String,
		/// Push using Docker credentials, or `KAMAL_REGISTRY_PASSWORD` when
		/// set.
		#[arg(long)]
		push: bool,
		/// Reuse an existing tag only after checking its migration metadata.
		#[arg(long, requires = "push")]
		reuse_existing: bool,
	},
	/// Writes inspectable main.tf.json and deployment instructions.
	Generate {
		/// Replace an edited generated configuration.
		#[arg(long)]
		force: bool,
	},
	/// Initializes `OpenTofu` and saves an infrastructure plan. Does not apply
	/// it.
	Plan,
	/// Applies the saved plan, refusing configuration changes since planning.
	Apply {
		/// Recover infrastructure when the old primary is lost. Skips its
		/// application reservation; the provider state lock still applies.
		#[arg(long, requires = "yes")]
		offline: bool,
		/// Confirm the old primary and its deployment runners are stopped.
		#[arg(long)]
		yes: bool,
	},
	/// Prepares private services and the backup worker without starting an
	/// app or taking an initial backup. Use before disaster recovery.
	Prepare,
	/// Prints public endpoints and the server address as JSON. Omits secrets.
	Output,
	/// Lists successful releases from the primary host's shared history.
	Releases,
	/// Reports application containers, the database, backups, and operation
	/// owner.
	Status,
	/// Lists current x86 offers from providers with configured credentials,
	/// sorted by currency and base monthly price. Never provisions resources.
	Offers {
		#[arg(long, default_value_t = 2.0)]
		min_memory_gb: f64,
	},
	/// Writes a push-to-deploy GitHub Actions workflow for a prepared Linux
	/// runner with Docker, Rust, lk, `OpenTofu`, Kamal, and an SSH agent.
	Ci {
		#[arg(long)]
		force: bool,
	},
	/// Encrypted scheduled backups and isolated restore checks.
	Backups {
		#[command(subcommand)]
		command: BackupCommand,
	},
	/// Restores a verified snapshot, retaining the previous database and
	/// leaving all app containers stopped. Recover its recorded image
	/// afterward.
	Restore {
		#[arg(long, default_value = "latest")]
		snapshot: String,
		#[arg(long)]
		yes: bool,
	},
	/// Releases a stale operation lock after checking that its runner has
	/// stopped.
	Unlock {
		#[arg(long)]
		owner: String,
		#[arg(long)]
		yes: bool,
	},
	/// Stops app containers and their workers, retaining Postgres and the
	/// tunnel. Use before explicitly rolling back the database.
	Stop,
	/// Publishes an already signed code-push release to the provisioned bucket.
	Publish {
		/// Platform whose signed release to publish.
		#[arg(long)]
		platform: leptoskit_config::Platform,
		/// Local release store.
		#[arg(long, default_value = "releases")]
		store: String,
		/// Release channel.
		#[arg(long, default_value = "production")]
		channel: String,
	},
	/// Writes a consistent Postgres custom-format backup. Copy it off-site.
	Backup {
		/// New output file; never overwrites an existing backup.
		#[arg(long)]
		out: PathBuf,
		/// Also upload the completed archive to the private backup bucket.
		/// Uses `AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY` scoped to that
		/// bucket.
		#[arg(long)]
		upload: bool,
	},
}

impl InfraCommand {
	fn supports_external(&self) -> bool {
		matches!(
			self,
			Self::Generate { .. }
				| Self::Plan
				| Self::Apply { .. }
				| Self::Output
				| Self::Status
				| Self::Unlock { .. }
				| Self::Plugins
				| Self::Offers { .. }
		)
	}
}

/// Host-side encrypted backup operations.
#[derive(Debug, clap::Subcommand)]
pub enum BackupCommand {
	/// Installs or updates the worker, credentials, schedule, and retention.
	Install,
	/// Takes, encrypts, uploads, restores for verification, and retains a
	/// backup.
	Run,
	/// Lists remote encrypted snapshots.
	List,
	/// Restores a snapshot into an isolated temporary Postgres container.
	Verify {
		#[arg(long, default_value = "latest")]
		snapshot: String,
	},
	/// Reports verification, age, and the last backup failure.
	Status,
}

/// Releases prebuilt images; compilation/pushing remains in the build pipeline.
#[derive(Debug, clap::Args)]
#[allow(
	clippy::struct_excessive_bools,
	reason = "clap flags have explicit conflicts and requirements"
)]
pub struct ReleaseArgs {
	/// Packaging descriptor whose digest was independently authenticated.
	#[arg(long, requires = "descriptor_sha256")]
	pub verified_descriptor: Option<PathBuf>,
	/// Independently authenticated SHA-256 of the packaging descriptor.
	#[arg(long, requires = "verified_descriptor")]
	pub descriptor_sha256: Option<String>,
	/// Immutable image tag, or a Git commit for the connected deployment
	/// service.
	#[arg(long, conflicts_with = "rollback")]
	pub version: Option<String>,
	/// Previously recorded version to roll back to. Schema history must match.
	#[arg(long, conflicts_with = "version")]
	pub rollback: Option<String>,
	/// First deploy: prepare the server and boot Postgres and the tunnel.
	#[arg(long, conflicts_with = "rollback")]
	pub setup: bool,
	/// Stop the current app before a schema-changing release. On failure it
	/// stays stopped until an operator recovers the schema and application.
	#[arg(long, conflicts_with = "rollback")]
	pub maintenance: bool,
	/// Redeploy a recorded version after recovery without reusing it as a new
	/// release.
	#[arg(long, requires = "version", conflicts_with_all = ["rollback", "setup"])]
	pub recover: bool,
	/// Stop writers and run checked down migrations to the rollback target.
	#[arg(long, requires_all = ["rollback", "yes"])]
	pub rollback_schema: bool,
	/// Confirm an explicitly requested database rollback.
	#[arg(long)]
	pub yes: bool,
}

/// Runs a managed infrastructure command from the nearest project.
pub fn run(command: InfraCommand) -> Result<()> {
	if let InfraCommand::BuildVerified(args) = &command {
		return verified::run(args);
	}
	if let InfraCommand::Offers { min_memory_gb } = &command {
		return pricing::print(*min_memory_gb);
	}
	let (config, root) = LeptoskitConfig::load()?;
	let infra = settings(&config)?;
	ensure!(
		infra.external.is_none() || command.supports_external(),
		"infra.external manages provisioning only; use the service's own build, release and \
		 backup procedures"
	);
	let dir = directory(&root, infra);
	match command {
		InfraCommand::BuildVerified(args) => verified::run(&args),
		InfraCommand::Analytics => analytics::query(infra),
		InfraCommand::Plugins => {
			println!(
				"{}",
				serde_json::to_string_pretty(&plugins::manifest(infra))?
			);
			Ok(())
		}
		InfraCommand::Generate { force } => generate_project(&root, &config, infra, &dir, force),
		InfraCommand::Build {
			version,
			push,
			reuse_existing,
		} => build::run(&root, &config, &version, push, reuse_existing),
		InfraCommand::Plan => {
			generate(&dir, infra, false)?;
			let _lock = lock(&dir)?;
			// A failed new plan must never leave an older plan applyable.
			if let Err(error) = fs::remove_file(dir.join("plan.sha256"))
				&& error.kind() != std::io::ErrorKind::NotFound
			{
				return Err(error).context("couldn't invalidate the previous plan");
			}
			pricing::guard(infra)?;
			run_tool("tofu", &dir, &["init", "-input=false"])?;
			run_tool("tofu", &dir, &["validate"])?;
			run_tool(
				"tofu",
				&dir,
				&["plan", "-input=false", "-out=leptoskit.plan"],
			)?;
			protect_plan(&dir)?;
			fs::write(dir.join("plan.sha256"), plan_fingerprint(&dir)?)?;
			println!("Saved plan. Run `lk infra apply` to apply these exact changes.");
			Ok(())
		}
		InfraCommand::Apply { offline, yes } => {
			ensure!(
				!offline || yes,
				"offline recovery requires --yes after stopping the old primary and its runners"
			);
			generate(&dir, infra, false)?;
			let _lock = lock(&dir)?;
			pricing::guard(infra)?;
			let expected =
				fs::read_to_string(dir.join("plan.sha256")).context("run `lk infra plan` first")?;
			ensure!(
				expected == plan_fingerprint(&dir)?,
				"the configuration or plan changed; run `lk infra plan` again"
			);
			protect_plan(&dir)?;
			let primary = applied_primary(&dir)?;
			let _reservation = primary
				.filter(|_| !offline)
				.as_deref()
				.map(|ip| remote::Reservation::acquire(ip, &infra.name))
				.transpose()?;
			// Consume before running: an interrupted/partial apply needs a
			// fresh plan.
			fs::remove_file(dir.join("plan.sha256"))?;
			run_tool("tofu", &dir, &["apply", "-input=false", "leptoskit.plan"])
		}
		InfraCommand::Prepare => releases::prepare(),
		InfraCommand::Output => print_output(&dir, infra),
		InfraCommand::Releases => {
			println!(
				"{}",
				serde_json::to_string_pretty(&releases::history(&server_ip(&dir)?, infra)?)?
			);
			Ok(())
		}
		InfraCommand::Status => status(&dir, infra),
		InfraCommand::Offers { min_memory_gb } => pricing::print(min_memory_gb),
		InfraCommand::Ci { force } => ci::generate(&root, &config, force),
		InfraCommand::Backups { command } => operations::backups(&dir, infra, command),
		InfraCommand::Restore { snapshot, yes } => operations::restore(&dir, infra, &snapshot, yes),
		InfraCommand::Unlock { owner, yes } => {
			ensure!(
				yes,
				"inspect `lk infra status` and confirm the old runner has stopped, then pass --yes"
			);
			remote::unlock(&server_ip(&dir)?, &remote::base(&infra.name), &owner)
		}
		InfraCommand::Stop => {
			let ip = server_ip(&dir)?;
			let _lock = lock(&dir)?;
			let _reservation = remote::Reservation::acquire(&ip, &infra.name)?;
			releases::stop(&releases::hosts(&dir)?, infra)?;
			println!("Stopped app containers; Postgres and the tunnel remain running.");
			Ok(())
		}
		InfraCommand::Publish {
			platform,
			store,
			channel,
		} => {
			applied_bindings(&dir, infra)?;
			ensure!(
				matches!(
					platform,
					leptoskit_config::Platform::Ios | leptoskit_config::Platform::Android
				),
				"the managed publisher supports iOS and Android code push"
			);
			ensure!(
				config.codepush.server.trim_end_matches('/')
					== format!("https://{}", infra.updates_domain),
				"codepush.server must match the provisioned updates hostname"
			);
			crate::codepush::run(crate::cli::CodepushCommand::Publish {
				store,
				channel,
				platform: Some(platform),
				location: crate::s3::S3Location {
					endpoint: format!("https://{}.r2.cloudflarestorage.com", infra.account_id),
					bucket: infra.updates_bucket.clone(),
					region: "auto".into(),
					prefix: None,
					path_style: false,
				},
			})
		}
		InfraCommand::Backup { out, upload } => backup(&dir, infra, &out, upload),
	}
}

fn generate_project(
	root: &Path,
	config: &LeptoskitConfig,
	infra: &InfraConfig,
	dir: &Path,
	force: bool,
) -> Result<()> {
	generate(dir, infra, force)?;
	if infra.external.is_some() {
		println!(
			"Infrastructure written to {}; run `lk infra plan`.",
			dir.display()
		);
		return Ok(());
	}
	// Reuse the existing container recipe rather than another build
	// system.
	let report = crate::deploy::generate(root, config, crate::deploy::Target::Docker, false)?;
	let ignore = root.join(".dockerignore");
	let mut text = fs::read_to_string(&ignore)?;
	if !text.lines().any(|line| line.trim() == "/infra") {
		text.push_str("\n# Managed state, plans, and runtime secret bindings\n/infra\n/.kamal\n");
		fs::write(ignore, text)?;
	}
	println!("{report}\nInfrastructure written to {}", dir.display());
	Ok(())
}

fn print_output(dir: &Path, infra: &InfraConfig) -> Result<()> {
	if infra.external.is_some() {
		let mut value = json!({"server_ip": server_ip(dir)?,
			"server_ips": releases::hosts(dir)?, "app_url": output(dir, "app_url")?,
			"runtime": "external"});
		if !infra.backup_bucket.is_empty() {
			value["backup_bucket"] = json!(output(dir, "backup_bucket")?);
			value["backup_endpoint"] = json!(output(dir, "backup_endpoint")?);
		}
		println!("{}", serde_json::to_string_pretty(&value)?);
		return Ok(());
	}
	let output = json!({"server_ip": server_ip(dir)?,
		"server_ips": releases::hosts(dir)?,
		"app_url": output(dir, "app_url")?, "updates_url": output(dir, "updates_url")?,
		"updates_bucket": output(dir, "updates_bucket")?, "backup_bucket": output(dir, "backup_bucket")?,
		"s3_endpoint": output(dir, "s3_endpoint")?, "backup_endpoint": output(dir, "backup_endpoint")?});
	println!("{}", serde_json::to_string_pretty(&output)?);
	Ok(())
}

fn status(dir: &Path, infra: &InfraConfig) -> Result<()> {
	if infra.external.is_some() {
		let ip = server_ip(dir)?;
		let path = remote::quote(&format!("{}/operation.lock", remote::base(&infra.name)));
		let owner = remote::execute(&ip, &format!("if [ -f {path} ]; then cat {path}; fi"), None)?;
		println!(
			"{}",
			serde_json::to_string_pretty(&json!({"runtime": "external", "server_ip": ip,
				"reservation_owner": String::from_utf8(owner)?,
				"application_health": "inspect through the service supervisor"}))?
		);
		Ok(())
	} else {
		operations::status(dir, infra)
	}
}

fn backup(dir: &Path, infra: &InfraConfig, out: &Path, upload: bool) -> Result<()> {
	applied_bindings(dir, infra)?;
	// Validate credentials before taking a dump, but never log them.
	let bucket = upload
		.then(|| {
			crate::s3::S3Bucket::from_env(&crate::s3::S3Location {
				endpoint: backup_endpoint(infra),
				bucket: infra.backup_bucket.clone(),
				region: infra
					.b2
					.as_ref()
					.map_or_else(|| "auto".into(), |b2| b2.region.clone()),
				prefix: None,
				path_style: false,
			})
		})
		.transpose()?;
	let ip = server_ip(dir)?;
	let _reservation = remote::Reservation::acquire(&ip, &infra.name)?;
	let mut temporary = tempfile::NamedTempFile::new_in(
		out.parent()
			.filter(|p| !p.as_os_str().is_empty())
			.unwrap_or(Path::new(".")),
	)?;
	let status = Command::new("ssh")
		.args([
			"-o",
			"BatchMode=yes",
			"--",
			&format!("root@{ip}"),
			&format!(
				"docker exec {}-db pg_dump -U leptoskit -d leptoskit --format=custom",
				infra.name
			),
		])
		.stdout(Stdio::from(temporary.as_file().try_clone()?))
		.status()
		.context("couldn't run ssh for backup")?;
	ensure!(
		status.success(),
		"backup failed; no backup file was published"
	);
	use std::io::Read as _;
	use std::io::Seek as _;
	temporary.as_file_mut().rewind()?;
	let mut magic = [0; 5];
	temporary.as_file_mut().read_exact(&mut magic)?;
	ensure!(
		&magic == b"PGDMP",
		"backup did not produce a Postgres custom archive"
	);
	temporary
		.persist_noclobber(out)
		.context("couldn't save backup without overwriting")?;
	if let Some(bucket) = bucket {
		let name = format!(
			"{}-{}.dump",
			infra.name,
			chrono::Utc::now().format("%Y%m%dT%H%M%S%.9fZ")
		);
		bucket.upload_backup(&name, out)?;
		println!("Uploaded backup to private bucket {}", infra.backup_bucket);
	}
	println!(
		"Backup saved to {}. Store it off-site with encryption and verify a restore.",
		out.display()
	);
	Ok(())
}

fn settings(config: &LeptoskitConfig) -> Result<&InfraConfig> {
	let infra = config
		.infra
		.as_ref()
		.context("add [infra] to leptoskit.toml; see the managed deployment guide")?;
	if let Some(problem) = infra.problem() {
		anyhow::bail!(problem);
	}
	if infra.external.is_some() {
		return Ok(infra);
	}
	ensure!(
		config
			.db
			.as_ref()
			.is_some_and(|db| db.dialect == leptoskit_config::DbDialect::Postgres),
		"the managed release preset requires [db] dialect = 'postgres'; other deploy generators \
		 still support SQLite"
	);
	ensure!(
		infra.services.relay == config.rpc.live.relay_url_env.is_some(),
		"configure rpc.live.relay_url_env together with infra.services.relay; build the app with \
		 rpc-relay when enabled"
	);
	Ok(infra)
}

fn managed_settings(config: &LeptoskitConfig) -> Result<&InfraConfig> {
	let infra = settings(config)?;
	ensure!(
		infra.external.is_none(),
		"infra.external manages provisioning only; deploy through the service supervisor"
	);
	Ok(infra)
}

fn directory(root: &Path, config: &InfraConfig) -> PathBuf {
	root.join("infra").join(&config.name)
}

fn lock(dir: &Path) -> Result<fs::File> {
	let file = fs::OpenOptions::new()
		.create(true)
		.truncate(false)
		.write(true)
		.open(dir.join("operation.lock"))?;
	fs4::FileExt::try_lock(&file).context("another infrastructure/release operation is running")?;
	Ok(file)
}

fn run_tool(tool: &str, dir: &Path, args: &[&str]) -> Result<()> {
	let status = Command::new(tool)
		.current_dir(dir)
		.args(args)
		.status()
		.with_context(|| format!("couldn't run {tool}; install it on the operator/CI machine"))?;
	ensure!(
		status.success(),
		"{tool} failed ({status}); inspect its output before retrying"
	);
	Ok(())
}

fn output(dir: &Path, name: &str) -> Result<String> {
	let result = Command::new("tofu")
		.current_dir(dir)
		.args(["output", "-raw", name])
		.output()
		.context("couldn't read OpenTofu outputs; run `lk infra plan` and `apply` first")?;
	ensure!(
		result.status.success(),
		"couldn't read infrastructure output `{name}`; no output values were logged"
	);
	Ok(String::from_utf8(result.stdout)?.trim().to_owned())
}

fn server_ip(dir: &Path) -> Result<String> {
	let ip = output(dir, "server_ip")?;
	ensure!(
		ip.parse::<std::net::Ipv4Addr>().is_ok(),
		"invalid server_ip output"
	);
	Ok(ip)
}

fn applied_primary(dir: &Path) -> Result<Option<String>> {
	let result = Command::new("tofu")
		.current_dir(dir)
		.args(["output", "-json"])
		.output()?;
	ensure!(
		result.status.success(),
		"couldn't inspect applied state before reserving the environment"
	);
	let values: serde_json::Value = serde_json::from_slice(&result.stdout)?;
	let ip = values
		.get("server_ip")
		.map(|v| v["value"].as_str().context("invalid primary output"))
		.transpose()?;
	if let Some(ip) = ip {
		ensure!(
			ip.parse::<std::net::Ipv4Addr>().is_ok(),
			"invalid primary IP in applied state"
		);
	}
	Ok(ip.map(str::to_owned))
}

fn protect_plan(dir: &Path) -> Result<()> {
	let result = Command::new("tofu")
		.current_dir(dir)
		.args(["show", "-json", "leptoskit.plan"])
		.output()?;
	ensure!(
		result.status.success(),
		"couldn't inspect the saved plan for protected data resources"
	);
	let plan: serde_json::Value = serde_json::from_slice(&result.stdout)?;
	ensure!(
		plan["format_version"]
			.as_str()
			.is_some_and(|s| s.starts_with("1.")),
		"unsupported plan JSON format"
	);
	for change in plan["resource_changes"].as_array().into_iter().flatten() {
		let kind = change["type"]
			.as_str()
			.context("invalid planned resource type")?;
		let name = change["name"]
			.as_str()
			.context("invalid planned resource name")?;
		let persistent = matches!(
			kind,
			"cloudflare_r2_bucket" | "b2_bucket" | "hcloud_network"
		) || (name == "app"
			&& matches!(kind, "hcloud_server" | "digitalocean_droplet"));
		let actions = change["change"]["actions"]
			.as_array()
			.context("invalid planned resource actions")?;
		ensure!(
			!persistent || !actions.iter().any(|a| a == "delete"),
			"plan would delete or replace protected persistent infrastructure; create and restore \
			 a separate environment, or migrate resources/state explicitly before continuing"
		);
	}
	Ok(())
}

fn applied_bindings(dir: &Path, config: &InfraConfig) -> Result<()> {
	for (key, expected) in [
		("app_url", format!("https://{}", config.domain)),
		("updates_url", format!("https://{}", config.updates_domain)),
		("updates_bucket", config.updates_bucket.clone()),
		("backup_bucket", config.backup_bucket.clone()),
		("backup_endpoint", backup_endpoint(config)),
		(
			"s3_endpoint",
			format!("https://{}.r2.cloudflarestorage.com", config.account_id),
		),
	] {
		ensure!(
			output(dir, key)? == expected,
			"{key} differs from applied infrastructure; plan and apply before using changed \
			 bindings"
		);
	}
	Ok(())
}

fn backup_endpoint(config: &InfraConfig) -> String {
	config.b2.as_ref().map_or_else(
		|| format!("https://{}.r2.cloudflarestorage.com", config.account_id),
		|b2| format!("https://s3.{}.backblazeb2.com", b2.region),
	)
}

fn digest(bytes: &[u8]) -> String {
	leptoskit_db::checksum(&String::from_utf8_lossy(bytes))
}

fn plan_fingerprint(dir: &Path) -> Result<String> {
	use sha2::Digest as _;
	let mut hash = sha2::Sha256::new();
	let mut files = vec![
		"main.tf.json".to_owned(),
		"plugins.json".into(),
		".terraform.lock.hcl".into(),
		"leptoskit.plan".into(),
	];
	for entry in fs::read_dir(dir)? {
		let name = entry?.file_name().to_string_lossy().into_owned();
		if [
			".tf",
			".tofu",
			".tf.json",
			".tofu.json",
			".tfvars",
			".tfvars.json",
		]
		.iter()
		.any(|suffix| name.ends_with(suffix))
			&& !files.contains(&name)
		{
			files.push(name);
		}
	}
	files.sort();
	for file in files {
		hash.update(file.as_bytes());
		hash.update([0]);
		hash.update(
			fs::read(dir.join(&file)).with_context(|| format!("missing {file}; plan again"))?,
		);
	}
	Ok(hex::encode(hash.finalize()))
}

/// Writes a deterministic provider configuration without evaluating
/// credentials.
pub fn generate(dir: &Path, config: &InfraConfig, force: bool) -> Result<()> {
	if let Some(problem) = config.problem() {
		anyhow::bail!(problem);
	}
	let text = serde_json::to_string_pretty(&resources(config))? + "\n";
	fs::create_dir_all(dir)?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
	}
	let plugin_text = serde_json::to_string_pretty(&plugins::manifest(config))? + "\n";
	// Check both before writing either, so an edited manifest cannot leave a
	// partially updated resource configuration.
	for (name, content) in [("main.tf.json", &text), ("plugins.json", &plugin_text)] {
		let path = dir.join(name);
		if path.exists() {
			ensure!(
				fs::read_to_string(&path)? == *content || force,
				"{} differs; review it, then use `lk infra generate --force`",
				path.display()
			);
		}
	}
	fs::write(dir.join("main.tf.json"), text)?;
	fs::write(dir.join("plugins.json"), plugin_text)?;
	let ignore = dir.join(".gitignore");
	if !ignore.exists() {
		fs::write(
			ignore,
			".terraform/\n*.tfstate*\n*.plan\nplan.sha256\noperation.lock\nreleases.json\nrelease.\
			 yml\n.kamal/\n",
		)?;
	}
	Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Release {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	evidence: Option<verified::Descriptor>,
	version: String,
	migrations: String,
	#[serde(default)]
	schema: Vec<(String, String)>,
	#[serde(default)]
	image_digest: Option<String>,
	#[serde(default)]
	completed_at: Option<String>,
}

fn history_fingerprint(mut pairs: Vec<(String, String)>) -> Result<String> {
	pairs.sort();
	Ok(digest(serde_json::to_string(&pairs)?.as_bytes()))
}

fn remote_history(ip: &str, name: &str) -> Result<String> {
	// Read inside the private database container, with no public database port.
	let sql = "SELECT COALESCE(json_agg(json_build_array(name, checksum)), '[]'::json) FROM \
	           _leptoskit_migrations";
	let remote = format!("docker exec {name}-db psql -U leptoskit -d leptoskit -At -c \"{sql}\"");
	let result = Command::new("ssh")
		.args(["-o", "BatchMode=yes", "--", &format!("root@{ip}"), &remote])
		.output()
		.context("couldn't read remote migration history")?;
	ensure!(
		result.status.success(),
		"couldn't verify remote migration history; rollback is blocked"
	);
	history_fingerprint(
		serde_json::from_slice(&result.stdout).context("invalid remote migration history")?,
	)
}

/// Deploys a prebuilt immutable tag. Rollback refuses a different migration
/// history; database rollback is explicit and never runs under live writers.
pub fn release(args: &ReleaseArgs) -> Result<()> {
	releases::run(args)
}

#[cfg(test)]
mod tests;
