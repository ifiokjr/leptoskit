//! `leptoskit codepush`: releases, publishes, pulls, and serves
//! over-the-air updates.

use std::io::Write;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use leptoskit_codepush::ReleaseSigningKey;
use leptoskit_codepush::installed::RuntimeContract;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;
use semver::Version;

use crate::cli::CodepushCommand;
use crate::native::config as native_config;
use crate::s3::S3Bucket;
use crate::s3::S3Location;

/// The port `leptoskit codepush serve` uses without `--port`, distinct from
/// the app's `serve.port` so both can run side by side.
pub const DEFAULT_CODEPUSH_PORT: u16 = 4100;

/// The environment variable `leptoskit codepush release` reads the hex
/// signing key from when `--signing-key` is not given, for CI secrets.
pub const SIGNING_KEY_ENV: &str = "LEPTOSKIT_CODEPUSH_SIGNING_KEY";

/// Runs a code push subcommand.
pub fn run(command: CodepushCommand) -> Result<()> {
	match command {
		CodepushCommand::Release {
			from,
			store,
			channel,
			platform,
			mandatory,
			signing_key,
		} => {
			release(
				&from,
				&store,
				&channel,
				platform,
				mandatory,
				signing_key.as_deref(),
			)
		}
		CodepushCommand::Keygen { out } => keygen(&out),
		CodepushCommand::Publish {
			store,
			channel,
			platform,
			location,
		} => publish(&store, &channel, platform, &location),
		CodepushCommand::Pull {
			store,
			channel,
			platform,
			location,
		} => pull(&store, &channel, platform, &location),
		CodepushCommand::Serve { store, dir, port } => serve(&store, &dir, port),
		CodepushCommand::Check {
			store,
			channel,
			platform,
			version,
			build,
		} => check(&store, &channel, platform, &version, build),
	}
}

fn release(
	from: &str,
	store: &str,
	channel: &str,
	platform: Option<Platform>,
	mandatory: bool,
	signing_key: Option<&str>,
) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let channel = selected_channel(&root, &config, channel, platform)?;
	let source = root.join(from);
	anyhow::ensure!(
		source.is_dir(),
		"{} does not exist; build first",
		source.display()
	);
	validate_release_platform(&root, &source, &config, platform)?;
	let signer = load_signing_key(signing_key)?;
	let public_key = signer.public_key().to_hex();
	let trusted = &config.codepush.public_keys;
	anyhow::ensure!(
		trusted.is_empty()
			|| trusted
				.iter()
				.any(|key| key.eq_ignore_ascii_case(&public_key)),
		"the signing key's public key {public_key} is not in codepush.public_keys, so no app \
		 would install this release"
	);
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	let store_dir = root.join(store);
	let release_store = leptoskit_codepush::ReleaseStore::open(store_dir.as_std_path())
		.map_err(|error| anyhow::anyhow!("could not open the release store: {error}"))?;

	let (version, build) = next_version(
		&release_store,
		&config.app.name,
		&channel,
		&config.app.version,
	)?;
	let ingested = release_store
		.ingest_directory(
			&signer,
			&config.app.name,
			&channel,
			&version,
			build,
			&source,
			mandatory,
		)
		.map_err(|error| anyhow::anyhow!("could not ingest the release: {error}"))?;
	let delta_note = ingested.delta_from_previous.as_ref().map_or_else(
		|| "full bundle".to_owned(),
		|delta| format!("{} ops", delta.ops.len()),
	);
	println!(
		"released {} {}+{} on {channel} ({delta_note}, {:.1} KiB), signed by {public_key}",
		config.app.name,
		ingested.record.manifest.version,
		ingested.record.manifest.build,
		ingested.record.container.len() as f64 / 1024.0
	);
	Ok(())
}

/// Copies a channel's newest release from an S3-compatible bucket into the
/// store, after checking it is signed by a key in `codepush.public_keys`.
fn pull(
	store: &str,
	channel: &str,
	platform: Option<Platform>,
	location: &S3Location,
) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let channel = selected_channel(&root, &config, channel, platform)?;
	anyhow::ensure!(
		!config.codepush.public_keys.is_empty(),
		"pulling verifies the bucket's release against codepush.public_keys; add your publisher \
		 public key there"
	);
	let trusted = leptoskit_codepush::TrustedKeys::from_hex(&config.codepush.public_keys)
		.map_err(|error| anyhow::anyhow!("codepush.public_keys: {error}"))?;
	let release_store = leptoskit_codepush::ReleaseStore::open(root.join(store))
		.map_err(|error| anyhow::anyhow!("could not open the release store: {error}"))?;
	let mut bucket = S3Bucket::from_env(location)?;
	let app = &config.app.name;
	let report =
		leptoskit_codepush::pull_channel(&release_store, app, &channel, &mut bucket, &trusted)
			.map_err(|error| anyhow::anyhow!("could not pull {app}/{channel}: {error}"))?;
	match report {
		leptoskit_codepush::PullReport::Empty => {
			println!(
				"{} has no {app} releases on {channel} yet; the next release starts fresh",
				location.bucket
			);
		}
		leptoskit_codepush::PullReport::Pulled {
			version,
			build,
			imported,
		} => {
			let note = if imported {
				"pulled into the store"
			} else {
				"already in the store"
			};
			println!(
				"{app} {version}+{build} on {channel} from {}: {note}",
				location.bucket
			);
		}
	}
	Ok(())
}

/// Uploads a channel's releases from the store to an S3-compatible bucket,
/// `index.json` last.
fn publish(
	store: &str,
	channel: &str,
	platform: Option<Platform>,
	location: &S3Location,
) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let channel = selected_channel(&root, &config, channel, platform)?;
	let store_dir = root.join(store);
	let release_store = leptoskit_codepush::ReleaseStore::open(&store_dir)
		.map_err(|error| anyhow::anyhow!("could not open the release store: {error}"))?;
	let mut bucket = S3Bucket::from_env(location)?;
	let app = &config.app.name;
	let report = leptoskit_codepush::publish_channel(&release_store, app, &channel, &mut bucket)
		.map_err(|error| anyhow::anyhow!("could not publish {app}/{channel}: {error}"))?;
	let uploaded = if report.releases.is_empty() {
		"no new releases".to_owned()
	} else {
		format!("uploaded {}", report.releases.join(", "))
	};
	println!(
		"published {app} {}+{} on {channel} to {}: {uploaded}; {} objects written, {} already \
		 present",
		report.version, report.build, location.bucket, report.written, report.already_present
	);
	Ok(())
}

/// Reads the signing key from `path`, or from [`SIGNING_KEY_ENV`] without
/// one. Errors never quote the key.
fn load_signing_key(path: Option<&str>) -> Result<ReleaseSigningKey> {
	let encoded = match path {
		Some(path) => {
			std::fs::read_to_string(path)
				.with_context(|| format!("could not read the signing key {path}"))?
		}
		None => {
			std::env::var(SIGNING_KEY_ENV).map_err(|_| {
				anyhow::anyhow!(
					"releases must be signed: pass --signing-key <PATH> or set {SIGNING_KEY_ENV}; \
					 create a key with `leptoskit codepush keygen --out <PATH>`"
				)
			})?
		}
	};
	ReleaseSigningKey::from_hex(&encoded)
		.map_err(|error| anyhow::anyhow!("could not load the signing key: {error}"))
}

/// Creates a signing key at `out`, readable only by its owner, and prints the
/// public key. The secret is never printed.
fn keygen(out: &str) -> Result<()> {
	let mut seed = [0u8; 32];
	getrandom::fill(&mut seed)
		.map_err(|error| anyhow::anyhow!("could not gather randomness: {error}"))?;
	let key = ReleaseSigningKey::from_seed(&seed);

	let mut options = std::fs::OpenOptions::new();
	options.write(true).create_new(true);
	#[cfg(unix)]
	std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
	let mut file = options
		.open(out)
		.with_context(|| format!("could not create {out}; an existing key is never overwritten"))?;
	file.write_all(format!("{}\n", key.to_hex()).as_bytes())
		.and_then(|()| file.sync_all())
		.with_context(|| format!("could not write {out}"))?;

	println!("wrote a code push signing key to {out}; keep it secret and out of version control");
	println!(
		"\ntrust it in leptoskit.toml:\n\n[codepush]\npublic_keys = [\"{}\"]",
		key.public_key().to_hex()
	);
	Ok(())
}

/// The version and build the next release of `project_version` takes: the
/// next build of the latest release when the versions match, build 1 of a
/// newer version, and an error for a version older than the latest release,
/// which clients would never update to.
fn next_version(
	store: &leptoskit_codepush::ReleaseStore,
	app: &str,
	channel: &str,
	project_version: &Version,
) -> Result<(Version, u64)> {
	let latest = store
		.latest(app, channel)
		.map_err(|error| anyhow::anyhow!("could not read the latest release: {error}"))?;
	let Some(record) = latest else {
		return Ok((project_version.clone(), 1));
	};
	let latest_version = &record.manifest.version;
	match project_version.cmp(latest_version) {
		std::cmp::Ordering::Equal => Ok((project_version.clone(), record.manifest.build + 1)),
		std::cmp::Ordering::Greater => Ok((project_version.clone(), 1)),
		std::cmp::Ordering::Less => {
			anyhow::bail!(
				"app.version {project_version} is older than the latest {channel} release \
				 {latest_version}+{}; bump app.version in leptoskit.toml",
				record.manifest.build
			)
		}
	}
}

fn serve(store: &str, dir: &str, port: Option<u16>) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	let store_dir = root.join(store);
	let release_store = leptoskit_codepush::ReleaseStore::open(store_dir.as_std_path())
		.map_err(|error| anyhow::anyhow!("could not open the release store: {error}"))?;
	let port = port.unwrap_or(DEFAULT_CODEPUSH_PORT);
	let address = &config.serve.address;
	println!("serving code push updates on http://{address}:{port}/api/codepush");
	let runtime = tokio::runtime::Runtime::new().context("could not start the async runtime")?;
	let options =
		leptoskit_axum::ServeOptions::new(root.join(dir).as_std_path(), address.clone(), port)
			.with_release_store(release_store);
	runtime.block_on(async {
		leptoskit_axum::serve(options)
			.await
			.map_err(|error| anyhow::anyhow!("{error}"))
	})
}

fn check(
	store: &str,
	channel: &str,
	platform: Option<Platform>,
	version: &str,
	build: u64,
) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let channel = selected_channel(&root, &config, channel, platform)?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	let store_dir = root.join(store);
	let release_store = leptoskit_codepush::ReleaseStore::open(store_dir.as_std_path())
		.map_err(|error| anyhow::anyhow!("could not open the release store: {error}"))?;
	let current = Version::parse(version).context("could not parse the version")?;
	let plan = release_store
		.plan_update(&config.app.name, &channel, &current, build)
		.map_err(|error| anyhow::anyhow!("could not plan the update: {error}"))?;
	match plan {
		leptoskit_codepush::UpdatePlan::UpToDate => {
			println!(
				"{} {current}+{build} on {channel}: up to date",
				config.app.name
			);
		}
		leptoskit_codepush::UpdatePlan::Available(offer) => {
			println!(
				"update available: {}+{} ({}, delta: {})",
				offer.manifest.version,
				offer.manifest.build,
				offer.manifest.bundle_hash(),
				offer.delta_url.is_some()
			);
		}
	}
	Ok(())
}

/// Explicit destinations select the same namespace embedded in native hosts.
fn selected_channel(
	root: &Path,
	config: &LeptoskitConfig,
	base: &str,
	platform: Option<Platform>,
) -> Result<String> {
	let Some(platform) = platform else {
		return Ok(base.to_owned());
	};
	anyhow::ensure!(
		platform.feature().is_some(),
		"code push --platform requires a concrete application platform, such as web, ios, or \
		 android; `{platform}` is an aggregate build target"
	);
	config.ensure_platform_enabled(platform)?;

	Ok(native_config::channel(root, base, platform))
}

/// A bundle's signed native requirements must agree with its release channel.
fn validate_release_platform(
	root: &Path,
	source: &Path,
	config: &LeptoskitConfig,
	platform: Option<Platform>,
) -> Result<()> {
	if !native_config::universal(root) {
		return Ok(());
	}

	let descriptor = source.join("runtime.json");
	let native_platform = platform.filter(|platform| platform.is_native());

	if !descriptor.is_file() && native_platform.is_none() {
		return Ok(());
	}

	let platform = native_platform.context(
		"native Wasm releases require --platform ios, android, macos, windows, or linux; use the \
		 same platform as `leptoskit native bundle --target`",
	)?;
	let runtime: RuntimeContract =
		serde_json::from_slice(&std::fs::read(&descriptor).with_context(|| {
			format!(
				"native releases need {}; build with `leptoskit native bundle --target \
				 {platform}` first",
				descriptor.display()
			)
		})?)
		.with_context(|| format!("invalid native runtime descriptor {}", descriptor.display()))?;
	let expected_shell = native_config::shell(root, config, platform);
	anyhow::ensure!(
		runtime.shell == expected_shell,
		"native runtime shell `{}` does not match --platform {platform} (expected \
		 `{expected_shell}`); release the bundle built for this platform",
		runtime.shell
	);

	Ok(())
}
