//! `leptoskit codepush approve` and `attach`: source approvals from
//! independent rebuilders, and attaching them to stored releases.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use leptoskit_codepush::CodePushError;
use leptoskit_codepush::ProvenancePolicy;
use leptoskit_codepush::ReleaseSigningKey;
use leptoskit_codepush::ReleaseStore;
use leptoskit_codepush::SignedApproval;
use leptoskit_codepush::SourceApproval;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;
use semver::Version;

use super::APPROVER_KEY_ENV;
use super::load_key;
use super::selected_channel;

/// The release `approve` checks a rebuild against.
pub(super) enum Target {
	/// A release container file.
	Container(String),
	/// A release in the project's store.
	Store {
		store: String,
		channel: String,
		platform: Option<Platform>,
		version: String,
		build: u64,
	},
}

/// Signs a source approval for a release after checking that `rebuilt`
/// holds exactly its files. Nothing is written when anything differs.
pub(super) fn approve(
	target: &Target,
	rebuilt: &Path,
	repository: &str,
	commit: &str,
	approver_key: Option<&str>,
	out: &Path,
) -> Result<()> {
	// The statement is checked first, so a typo fails before any work.
	leptoskit_codepush::provenance::validate_repository(repository)
		.map_err(|error| anyhow::anyhow!("--repository: {error}"))?;
	leptoskit_codepush::provenance::validate_commit(commit)
		.map_err(|error| anyhow::anyhow!("--commit: {error}"))?;
	anyhow::ensure!(
		rebuilt.is_dir(),
		"{} is not a directory; build the release yourself first",
		rebuilt.display()
	);
	let release = leptoskit_codepush::read_container(&release_container(target)?)
		.map_err(|error| anyhow::anyhow!("could not read the release: {error}"))?;
	let manifest = &release.manifest;
	let name = format!(
		"{} {}+{} on {}",
		manifest.app, manifest.version, manifest.build, manifest.channel
	);

	if let Some(difference) = first_difference(&release.files, rebuilt)? {
		anyhow::bail!(
			"not approving {name}: {difference}; the rebuilt directory must hold exactly the \
			 release's files, byte for byte"
		);
	}

	let approver = load_approver_key(approver_key)?;
	let public_key = approver.public_key().to_hex();
	ensure_not_a_publisher(&public_key)?;
	let statement = SourceApproval::new(manifest, repository, commit)
		.map_err(|error| anyhow::anyhow!("{error}"))?;
	let approval = SignedApproval::sign(&statement, &approver)
		.map_err(|error| anyhow::anyhow!("could not sign the approval: {error}"))?;
	std::fs::write(out, serde_json::to_vec_pretty(&approval)?)
		.with_context(|| format!("could not write {}", out.display()))?;

	println!(
		"approved {name}: all {} files match {repository} at {commit}; signed by {public_key}, \
		 wrote {}",
		release.files.len(),
		out.display()
	);
	Ok(())
}

/// Reads the approver key from `path`, or from [`APPROVER_KEY_ENV`] without
/// one. Errors never quote the key.
fn load_approver_key(path: Option<&str>) -> Result<ReleaseSigningKey> {
	load_key(path, APPROVER_KEY_ENV, "approver key", || {
		format!(
			"approvals must be signed: pass --approver-key <PATH> or set {APPROVER_KEY_ENV}; \
			 create a key with `leptoskit codepush keygen --approver --out <PATH>`"
		)
	})
}

/// The container bytes of the release `target` names.
fn release_container(target: &Target) -> Result<Vec<u8>> {
	match target {
		Target::Container(path) => {
			std::fs::read(path).with_context(|| format!("could not read the release {path}"))
		}
		Target::Store {
			store,
			channel,
			platform,
			version,
			build,
		} => stored_container(store, channel, *platform, version, *build),
	}
}

/// The container bytes of a release in the project's store.
fn stored_container(
	store: &str,
	channel: &str,
	platform: Option<Platform>,
	version: &str,
	build: u64,
) -> Result<Vec<u8>> {
	let (config, root) = LeptoskitConfig::load()?;
	let channel = selected_channel(&root, &config, channel, platform)?;
	let version = Version::parse(version).context("could not parse the version")?;
	let release_store = ReleaseStore::open(root.join(store))
		.map_err(|error| anyhow::anyhow!("could not open the release store: {error}"))?;
	let app = &config.app.name;

	Ok(release_store
		.find(app, &channel, &version, build)
		.map_err(|error| anyhow::anyhow!("could not read the release store: {error}"))?
		.with_context(|| format!("the store has no {app} {version}+{build} on {channel}"))?
		.container)
}

/// An approver must be someone other than the publisher. A directory with no
/// project, as an outside rebuilder's often is, has no publisher keys to
/// compare.
fn ensure_not_a_publisher(public_key: &str) -> Result<()> {
	let Ok((config, _)) = LeptoskitConfig::load() else {
		return Ok(());
	};
	let publisher = config
		.codepush
		.public_keys
		.iter()
		.any(|key| key.eq_ignore_ascii_case(public_key));
	anyhow::ensure!(
		!publisher,
		"the approver key {public_key} is a publisher key in codepush.public_keys; approvals must \
		 come from someone other than the publisher"
	);
	Ok(())
}

/// The first difference, in path order, between a release's files and the
/// regular files under `rebuilt`, or `None` when they are the same files
/// with the same bytes.
fn first_difference(release: &[(String, Vec<u8>)], rebuilt: &Path) -> Result<Option<String>> {
	let built = regular_files(rebuilt)?;
	let released: BTreeMap<&str, &[u8]> = release
		.iter()
		.map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
		.collect();
	let paths: BTreeSet<&str> = released
		.keys()
		.copied()
		.chain(built.keys().map(String::as_str))
		.collect();

	for path in paths {
		let difference = describe_difference(
			path,
			released.get(path).copied(),
			built.get(path).map(PathBuf::as_path),
		)?;

		if difference.is_some() {
			return Ok(difference);
		}
	}

	Ok(None)
}

/// Every regular file under `directory`, by its `/`-separated relative path.
/// A symbolic link or other special file is an error: a release only ever
/// holds regular files.
fn regular_files(directory: &Path) -> Result<BTreeMap<String, PathBuf>> {
	let mut files = BTreeMap::new();

	for entry in walkdir::WalkDir::new(directory).min_depth(1) {
		let entry = entry.with_context(|| format!("could not read {}", directory.display()))?;

		if entry.file_type().is_dir() {
			continue;
		}

		let relative = entry
			.path()
			.strip_prefix(directory)
			.expect("walkdir entries are under the root")
			.to_string_lossy()
			.replace('\\', "/");
		anyhow::ensure!(
			entry.file_type().is_file(),
			"{relative} in {} is not a regular file; a release holds only regular files",
			directory.display()
		);
		files.insert(relative, entry.into_path());
	}

	Ok(files)
}

/// How the release's copy of `path` and the rebuilt one differ, or `None`
/// when both exist with the same bytes.
fn describe_difference(
	path: &str,
	released: Option<&[u8]>,
	built: Option<&Path>,
) -> Result<Option<String>> {
	let (released, built) = match (released, built) {
		(Some(released), Some(built)) => (released, built),
		(Some(_), None) => {
			return Ok(Some(format!(
				"{path} is in the release but not in the rebuilt directory"
			)));
		}
		(None, _) => {
			return Ok(Some(format!(
				"{path} is in the rebuilt directory but not in the release"
			)));
		}
	};
	let rebuilt =
		std::fs::read(built).with_context(|| format!("could not read {}", built.display()))?;

	Ok((rebuilt != released)
		.then(|| format!("{path} differs between the release and the rebuilt directory")))
}

/// Attaches approval files to the releases they approve in the store.
pub(super) fn attach(store: &str, files: &[String]) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let release_store = ReleaseStore::open(root.join(store))
		.map_err(|error| anyhow::anyhow!("could not open the release store: {error}"))?;
	let policy = project_policy(&config)?;
	let approvals = files
		.iter()
		.map(|file| read_approval(file, policy.as_ref()))
		.collect::<Result<Vec<_>>>()?;
	let attached = release_store
		.attach_approvals(&approvals)
		.map_err(|error| anyhow::anyhow!("could not attach the approvals: {error}"))?;

	for release in attached {
		println!(
			"attached {} new approval(s) to {}; it has {}",
			release.added, release.release, release.total
		);
	}

	if let Some(policy) = &policy {
		report_approvals(&release_store, &approvals, policy)?;
	}

	Ok(())
}

/// The project's `codepush.provenance`, as the policy its apps enforce.
fn project_policy(config: &LeptoskitConfig) -> Result<Option<ProvenancePolicy>> {
	let Some(provenance) = &config.codepush.provenance else {
		return Ok(None);
	};
	let policy = leptoskit_codepush::TrustedKeys::from_hex(&provenance.approvers)
		.and_then(|approvers| {
			ProvenancePolicy::new(
				provenance.repository.clone(),
				approvers,
				provenance.threshold as usize,
			)
		})
		.map_err(|error| anyhow::anyhow!("codepush.provenance: {error}"))?;

	Ok(Some(policy))
}

/// Reads one approval file and checks its signature. With the project's
/// `policy`, also refuses an approval no app of the project would count.
fn read_approval(file: &str, policy: Option<&ProvenancePolicy>) -> Result<SignedApproval> {
	let bytes = std::fs::read(file).with_context(|| format!("could not read {file}"))?;
	let approval =
		SignedApproval::from_slice(&bytes).map_err(|error| anyhow::anyhow!("{file}: {error}"))?;
	let (key, statement) = approval
		.open()
		.map_err(|error| anyhow::anyhow!("{file}: {error}"))?;
	let Some(policy) = policy else {
		return Ok(approval);
	};

	anyhow::ensure!(
		policy.approvers().contains(&key),
		"{file} is signed by {}, which codepush.provenance.approvers doesn't list, so no app \
		 would count it",
		key.to_hex()
	);
	anyhow::ensure!(
		statement
			.repository
			.eq_ignore_ascii_case(policy.repository()),
		"{file} approves a build of {}, not codepush.provenance.repository {}",
		statement.repository,
		policy.repository()
	);
	Ok(approval)
}

/// Says, for each release `approvals` name, whether its attached approvals
/// now satisfy `policy`.
fn report_approvals(
	store: &ReleaseStore,
	approvals: &[SignedApproval],
	policy: &ProvenancePolicy,
) -> Result<()> {
	let mut reported = BTreeSet::new();

	for approval in approvals {
		let (_, statement) = approval
			.open()
			.map_err(|error| anyhow::anyhow!("{error}"))?;

		if reported.insert(statement.bundle_hash.clone()) {
			report_release(store, &statement, policy)?;
		}
	}

	Ok(())
}

/// Says whether the release `statement` names has enough approvals.
fn report_release(
	store: &ReleaseStore,
	statement: &SourceApproval,
	policy: &ProvenancePolicy,
) -> Result<()> {
	let release = statement.release();
	let manifest = store
		.find(
			&statement.app,
			&statement.channel,
			&statement.version,
			statement.build,
		)
		.map_err(|error| anyhow::anyhow!("could not read the release store: {error}"))?
		.with_context(|| format!("the store has no {release}"))?
		.manifest;
	let attached = store
		.approvals(&manifest)
		.map_err(|error| anyhow::anyhow!("could not read the approvals: {error}"))?
		.map(|approvals| approvals.approvals)
		.unwrap_or_default();

	match policy.verify(&manifest, &attached) {
		Ok(source) => {
			println!(
				"{release} meets codepush.provenance: {} of {} approvers, commit {}",
				source.approvers.len(),
				policy.threshold(),
				source.commit
			);
		}
		Err(CodePushError::Unapproved {
			approvals,
			threshold,
			..
		}) => {
			println!(
				"{release} has {approvals} of the {threshold} approvals codepush.provenance \
				 requires; apps won't install it yet"
			);
		}
		Err(error) => anyhow::bail!("{error}; apps won't install {release}"),
	}

	Ok(())
}
