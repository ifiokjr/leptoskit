//! `leptoskit codepush`: releases and serves over-the-air updates.

use std::io::Write;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use leptoskit_codepush::ReleaseSigningKey;
use semver::Version;

use crate::cli::CodepushCommand;

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
			mandatory,
			signing_key,
		} => release(&from, &store, &channel, mandatory, signing_key.as_deref()),
		CodepushCommand::Keygen { out } => keygen(&out),
		CodepushCommand::Serve { store, dir, port } => serve(&store, &dir, port),
		CodepushCommand::Check {
			store,
			channel,
			version,
			build,
		} => check(&store, &channel, &version, build),
	}
}

fn release(
	from: &str,
	store: &str,
	channel: &str,
	mandatory: bool,
	signing_key: Option<&str>,
) -> Result<()> {
	let (config, root) = leptoskit_config::LeptoskitConfig::load()?;
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
	let source = root.join(from);
	anyhow::ensure!(source.is_dir(), "{source} does not exist; build first");
	let store_dir = root.join(store);
	let release_store = leptoskit_codepush::ReleaseStore::open(store_dir.as_std_path())
		.map_err(|error| anyhow::anyhow!("could not open the release store: {error}"))?;

	let (version, build) = next_version(
		&release_store,
		&config.app.name,
		channel,
		&config.app.version,
	)?;
	let record = release_store
		.ingest_directory(
			&signer,
			&config.app.name,
			channel,
			&version,
			build,
			source.as_std_path(),
			mandatory,
		)
		.map_err(|error| anyhow::anyhow!("could not ingest the release: {error}"))?;
	let delta_note = record.delta_from_previous.as_ref().map_or_else(
		|| "full bundle".to_owned(),
		|delta| format!("{} ops", delta.ops.len()),
	);
	println!(
		"released {} {}+{} on {channel} ({delta_note}, {:.1} KiB), signed by {public_key}",
		config.app.name,
		record.manifest.version,
		record.manifest.build,
		record.container.len() as f64 / 1024.0
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
	let (config, root) = leptoskit_config::LeptoskitConfig::load()?;
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

fn check(store: &str, channel: &str, version: &str, build: u64) -> Result<()> {
	let (config, root) = leptoskit_config::LeptoskitConfig::load()?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	let store_dir = root.join(store);
	let release_store = leptoskit_codepush::ReleaseStore::open(store_dir.as_std_path())
		.map_err(|error| anyhow::anyhow!("could not open the release store: {error}"))?;
	let current = Version::parse(version).context("could not parse the version")?;
	let plan = release_store
		.plan_update(&config.app.name, channel, &current, build)
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
