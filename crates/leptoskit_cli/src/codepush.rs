//! `leptoskit codepush`: releases and serves over-the-air updates.

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use semver::Version;

use crate::cli::CodepushCommand;

/// Runs a code push subcommand.
pub fn run(command: CodepushCommand) -> Result<()> {
	match command {
		CodepushCommand::Release {
			from,
			store,
			channel,
			mandatory,
		} => release(&from, &store, &channel, mandatory),
		CodepushCommand::Serve { store, port } => serve(&store, port),
		CodepushCommand::Check {
			store,
			channel,
			version,
		} => check(&store, &channel, &version),
	}
}

fn release(from: &str, store: &str, channel: &str, mandatory: bool) -> Result<()> {
	let (config, root) =
		leptoskit_config::LeptoskitConfig::load().context("no leptoskit project found")?;
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
	);
	let record = release_store
		.ingest_directory(
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
		"released {} {}+{} on {channel} ({delta_note}, {:.1} KiB)",
		config.app.name,
		record.manifest.version,
		record.manifest.build,
		record.container.len() as f64 / 1024.0
	);
	Ok(())
}

fn next_version(
	store: &leptoskit_codepush::ReleaseStore,
	app: &str,
	channel: &str,
	project_version: &Version,
) -> (Version, u64) {
	let latest = store.latest(app, channel).ok().flatten();
	match latest {
		Some(record) if record.manifest.version == *project_version => {
			(project_version.clone(), record.manifest.build + 1)
		}
		_ => (project_version.clone(), 1),
	}
}

fn serve(store: &str, port: Option<u16>) -> Result<()> {
	let (_config, root) =
		leptoskit_config::LeptoskitConfig::load().context("no leptoskit project found")?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	let store_dir = root.join(store);
	let release_store = leptoskit_codepush::ReleaseStore::open(store_dir.as_std_path())
		.map_err(|error| anyhow::anyhow!("could not open the release store: {error}"))?;
	let port = port.unwrap_or(4100);
	println!("serving code push updates on http://127.0.0.1:{port}");
	let runtime = tokio::runtime::Runtime::new().context("could not start the async runtime")?;
	let options =
		leptoskit_axum::ServeOptions::new(root.join("dist").as_std_path(), "127.0.0.1", port)
			.with_release_store(release_store);
	runtime.block_on(async {
		leptoskit_axum::serve(options)
			.await
			.map_err(|error| anyhow::anyhow!("{error}"))
	})
}

fn check(store: &str, channel: &str, version: &str) -> Result<()> {
	let (config, root) =
		leptoskit_config::LeptoskitConfig::load().context("no leptoskit project found")?;
	let root = Utf8PathBuf::from_path_buf(root)
		.map_err(|_| anyhow::anyhow!("project path is not UTF-8"))?;
	let store_dir = root.join(store);
	let release_store = leptoskit_codepush::ReleaseStore::open(store_dir.as_std_path())
		.map_err(|error| anyhow::anyhow!("could not open the release store: {error}"))?;
	let current = Version::parse(version).context("could not parse the version")?;
	let plan = release_store
		.plan_update(&config.app.name, channel, &current, 1)
		.map_err(|error| anyhow::anyhow!("could not plan the update: {error}"))?;
	match plan {
		leptoskit_codepush::UpdatePlan::UpToDate => {
			println!("{} {current}+1 on {channel}: up to date", config.app.name);
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
