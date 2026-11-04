//! Backend bundle verification. An inventory pinned by a separately trusted
//! digest proves local file integrity; [`build`] rebuilds a bundle from a
//! public commit in a container without network access and compares the
//! executable byte for byte. Neither proves what a remote server is running.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;
use sha2::Sha256;

mod build;
mod deployment;

const FORMAT_VERSION: u32 = 1;
const KIND: &str = "leptoskit-backend-bundle-inventory";
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

/// Backend bundle checks, with no claim about a live deployment.
#[derive(Debug, clap::Subcommand)]
pub enum VerifyCommand {
	/// Builds a backend bundle from a Git commit in a container without network
	/// access, twice, and requires identical executables.
	Build(build::BuildArgs),
	/// Rebuilds a pinned bundle from its own source and dependencies and
	/// compares the executable; also compares the source with its Git commit.
	Reproduce(build::ReproduceArgs),
	/// Writes a new deployment signing key and prints its public key.
	Keygen(deployment::KeygenArgs),
	/// Signs a deployment policy naming the origins and releases a
	/// deployment serves.
	Sign(deployment::SignArgs),
	/// Checks an origin's signed deployment policy against a key you trust.
	/// Does not show what its server runs.
	Deployment(deployment::DeploymentArgs),
	/// Records the files in a prepared backend bundle. This does not build,
	/// sign, or attest the bundle.
	Record {
		/// The prepared bundle directory; see the backend bundle guide.
		#[arg(long)]
		bundle: PathBuf,
		/// A new inventory file outside the bundle. Prints its SHA-256.
		#[arg(long)]
		out: PathBuf,
	},
	/// Checks every bundle entry against an inventory with a separately
	/// trusted SHA-256. Does not contact or verify a running server.
	Check {
		/// The extracted backend bundle directory.
		#[arg(long)]
		bundle: PathBuf,
		/// The inventory written by `verify record`.
		#[arg(long)]
		manifest: PathBuf,
		/// The expected inventory digest, obtained independently of the bundle.
		#[arg(long)]
		sha256: String,
	},
}

/// A declared build recipe. Recording it does not execute or attest it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BuildRecipe {
	pub(crate) repository: String,
	pub(crate) commit: String,
	pub(crate) builder_image: String,
	pub(crate) target: String,
	pub(crate) package: String,
	pub(crate) binary: String,
	pub(crate) features: Vec<String>,
	pub(crate) no_default_features: bool,
	/// The complete rustflags list; Cargo configuration rustflags in the
	/// source do not apply to a verified build.
	pub(crate) rustflags: Vec<String>,
	/// A directory in the commit copied into `infra/`, if any.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub(crate) infra: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Inventory {
	format_version: u32,
	kind: String,
	pub(crate) entries: BTreeMap<String, Entry>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Entry {
	Directory,
	File {
		sha256: String,
		size: u64,
		// None on hosts without Unix permission metadata. A check on such a
		// host cannot silently accept an inventory carrying Unix permissions.
		unix_mode: Option<u32>,
	},
}

/// Runs an offline inventory command.
pub fn run(command: VerifyCommand) -> Result<()> {
	match command {
		VerifyCommand::Build(args) => build::build(&args)?,
		VerifyCommand::Reproduce(args) => build::reproduce(&args)?,
		VerifyCommand::Keygen(args) => deployment::keygen(&args)?,
		VerifyCommand::Sign(args) => deployment::sign_file(&args)?,
		VerifyCommand::Deployment(args) => deployment::deployment(&args)?,
		VerifyCommand::Record { bundle, out } => {
			let root = bundle
				.canonicalize()
				.context("could not open the bundle directory")?;
			let parent = out
				.parent()
				.filter(|path| !path.as_os_str().is_empty())
				.unwrap_or(Path::new("."));
			let parent = parent
				.canonicalize()
				.context("the inventory output directory must exist")?;
			anyhow::ensure!(
				!parent.starts_with(&root),
				"write the inventory outside the bundle so it cannot inventory itself"
			);
			let digest = record(&root, &out)?;
			println!("inventory SHA-256: {digest}");
			println!(
				"Recorded local files only; build provenance and live deployment were not \
				 verified."
			);
		}
		VerifyCommand::Check {
			bundle,
			manifest,
			sha256,
		} => {
			let count = check(&bundle, &manifest, &sha256)?;
			println!("backend bundle matches the pinned inventory ({count} entries)");
			println!("Build provenance and live deployment were not verified.");
		}
	}
	Ok(())
}

/// Writes a new inventory of `root` at `out`, returning its SHA-256.
fn record(root: &Path, out: &Path) -> Result<String> {
	let bytes = serde_json::to_vec_pretty(&inventory(root)?)?;
	anyhow::ensure!(
		bytes.len() as u64 <= MAX_MANIFEST_BYTES,
		"the bundle inventory is too large"
	);
	let mut file = File::options()
		.write(true)
		.create_new(true)
		.open(out)
		.with_context(|| format!("could not create a new inventory at {}", out.display()))?;
	file.write_all(&bytes)?;
	Ok(hex::encode(Sha256::digest(&bytes)))
}

fn digest_is_valid(value: &str) -> bool {
	value.len() == 64
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
	let mut bytes = Vec::new();
	File::open(path)?
		.take(MAX_MANIFEST_BYTES + 1)
		.read_to_end(&mut bytes)?;
	anyhow::ensure!(
		bytes.len() as u64 <= MAX_MANIFEST_BYTES,
		"bundle metadata is too large"
	);
	// Serde's detailed errors can quote a supplied string. In particular,
	// malformed secrets.json must not echo an accidentally supplied value.
	serde_json::from_slice(&bytes).map_err(|error| {
		anyhow::anyhow!(
			"invalid bundle metadata at {} (line {}, column {})",
			path.display(),
			error.line(),
			error.column()
		)
	})
}

/// A non-empty word of ASCII letters, digits, and `extra`, which cannot be
/// read as a command-line option.
fn is_name(value: &str, extra: &[u8]) -> bool {
	!value.is_empty()
		&& !value.starts_with('-')
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || extra.contains(&byte))
}

/// A full lowercase Git object ID, SHA-1 or SHA-256.
fn is_commit(value: &str) -> bool {
	matches!(value.len(), 40 | 64)
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Exactly `https://github.com/<owner>/<repository>`, so two spellings of
/// one repository cannot compare unequal.
pub(crate) fn validate_repository(value: &str) -> Result<()> {
	let canonical = value
		.strip_prefix("https://github.com/")
		.and_then(|path| path.split_once('/'))
		.is_some_and(|(owner, repository)| {
			is_name(owner, b"-")
				&& is_name(repository, b"-_.")
				&& !matches!(repository, "." | "..")
				&& repository
					.rsplit_once('.')
					.is_none_or(|(_, extension)| !extension.eq_ignore_ascii_case("git"))
		});
	anyhow::ensure!(
		canonical,
		"{value:?} must be a GitHub repository written as https://github.com/<owner>/<repository>"
	);
	Ok(())
}

/// Checks a recipe's declared values. This validates their form only.
pub(crate) fn validate(recipe: &BuildRecipe) -> Result<()> {
	validate_repository(&recipe.repository)?;
	anyhow::ensure!(
		is_commit(&recipe.commit),
		"build.json must name a full lowercase Git commit ID"
	);
	let pinned = recipe
		.builder_image
		.rsplit_once("@sha256:")
		.is_some_and(|(name, digest)| {
			!name.is_empty()
				&& name
					.bytes()
					.all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte))
				&& digest_is_valid(digest)
		});
	anyhow::ensure!(
		pinned,
		"build.json builder_image must be pinned by @sha256:<64 lowercase hex digits>"
	);
	anyhow::ensure!(
		is_name(&recipe.target, b"-_.")
			&& is_name(&recipe.package, b"-_")
			&& is_name(&recipe.binary, b"-_"),
		"build.json must name a target, package, and binary"
	);
	// Features, no_default_features, and rustflags are deliberately required,
	// including empty lists and false: omitting a build input must not
	// silently select a different default.
	anyhow::ensure!(
		recipe
			.features
			.iter()
			.all(|feature| is_name(feature, b"-_/+.")),
		"build.json features must be plain feature names"
	);
	anyhow::ensure!(
		recipe
			.rustflags
			.iter()
			.all(|flag| !flag.is_empty() && !flag.contains('\u{1f}')),
		"build.json rustflags cannot contain empty or unit-separator entries"
	);
	if let Some(infra) = &recipe.infra {
		anyhow::ensure!(
			is_name(infra, b"-_./")
				&& infra
					.split('/')
					.all(|part| !matches!(part, "" | "." | "..")),
			"build.json infra must be a relative directory in the repository"
		);
	}
	Ok(())
}

fn validate_recipe(root: &Path) -> Result<()> {
	validate(&read_json(&root.join("build.json"))?)?;
	validate_secrets(&read_json::<Vec<String>>(&root.join("secrets.json"))?)
}

/// Secret declarations are unique environment variable names, never values.
pub(crate) fn validate_secrets(names: &[String]) -> Result<()> {
	let mut unique = BTreeSet::new();
	for name in names {
		anyhow::ensure!(
			!name.is_empty()
				&& name.bytes().enumerate().all(|(index, byte)| {
					byte == b'_'
						|| byte.is_ascii_alphabetic()
						|| (index > 0 && byte.is_ascii_digit())
				})
				&& unique.insert(name),
			"secrets.json must contain unique environment variable names, never secret values or \
			 assignments"
		);
	}
	Ok(())
}

/// Every directory and regular file under `root`, by `/`-separated path.
/// Links and special files are rejected.
pub(crate) fn walk(root: &Path) -> Result<BTreeMap<String, Entry>> {
	anyhow::ensure!(root.is_dir(), "{} must be a directory", root.display());
	let mut entries = BTreeMap::new();
	for entry in walkdir::WalkDir::new(root).min_depth(1).follow_links(false) {
		let entry = entry?;
		let relative = entry.path().strip_prefix(root)?;
		let name = relative
			.to_str()
			.context("bundle paths must be UTF-8")?
			.replace(std::path::MAIN_SEPARATOR, "/");
		anyhow::ensure!(
			!name.contains('\\') && !name.chars().any(char::is_control),
			"bundle paths cannot contain backslashes or control characters"
		);
		let kind = entry.file_type();
		let value = if kind.is_dir() {
			Entry::Directory
		} else {
			anyhow::ensure!(
				kind.is_file(),
				"bundle entry {name:?} must be a regular file or directory; links are not \
				 supported"
			);
			let mut file = File::open(entry.path())?;
			let metadata = file.metadata()?;
			let mut hasher = Sha256::new();
			let size = std::io::copy(&mut file, &mut hasher)?;
			#[cfg(unix)]
			let unix_mode = {
				use std::os::unix::fs::PermissionsExt as _;
				Some(metadata.permissions().mode() & 0o7777)
			};
			#[cfg(not(unix))]
			let unix_mode = {
				let _ = metadata;
				None
			};
			Entry::File {
				sha256: hex::encode(hasher.finalize()),
				size,
				unix_mode,
			}
		};
		entries.insert(name, value);
		anyhow::ensure!(
			entries.len() <= MAX_ENTRIES,
			"the bundle contains too many entries"
		);
	}
	Ok(entries)
}

fn inventory(root: &Path) -> Result<Inventory> {
	anyhow::ensure!(root.is_dir(), "the bundle must be a directory");
	let entries = walk(root)?;
	for name in entries.keys() {
		let top = name.split('/').next().unwrap_or_default();
		anyhow::ensure!(
			matches!(
				top,
				"source" | "dependencies" | "infra" | "artifacts" | "build.json" | "secrets.json"
			),
			"unexpected top-level bundle entry {top:?}"
		);
	}
	for name in ["source", "dependencies", "infra", "artifacts"] {
		anyhow::ensure!(
			entries.get(name) == Some(&Entry::Directory),
			"the bundle needs a {name}/ directory"
		);
	}
	for name in [
		"source/Cargo.toml",
		"source/Cargo.lock",
		"build.json",
		"secrets.json",
		"artifacts/backend",
	] {
		anyhow::ensure!(
			matches!(entries.get(name), Some(Entry::File { .. })),
			"the bundle needs a regular file at {name}"
		);
	}
	validate_recipe(root)?;
	Ok(Inventory {
		format_version: FORMAT_VERSION,
		kind: KIND.to_owned(),
		entries,
	})
}

pub(crate) fn check(root: &Path, manifest: &Path, expected_digest: &str) -> Result<usize> {
	anyhow::ensure!(
		digest_is_valid(expected_digest),
		"--sha256 must be 64 lowercase hex digits from a separately trusted source"
	);
	let mut bytes = Vec::new();
	File::open(manifest)?
		.take(MAX_MANIFEST_BYTES + 1)
		.read_to_end(&mut bytes)?;
	anyhow::ensure!(
		bytes.len() as u64 <= MAX_MANIFEST_BYTES,
		"the bundle inventory is too large"
	);
	anyhow::ensure!(
		hex::encode(Sha256::digest(&bytes)) == expected_digest,
		"inventory SHA-256 does not match the independently pinned digest"
	);
	let expected: Inventory = serde_json::from_slice(&bytes).context("invalid bundle inventory")?;
	anyhow::ensure!(
		expected.format_version == FORMAT_VERSION && expected.kind == KIND,
		"unsupported bundle inventory format or kind"
	);
	let actual = inventory(root)?;
	for (path, entry) in &expected.entries {
		anyhow::ensure!(
			actual.entries.get(path) == Some(entry),
			"bundle entry {path:?} is missing or changed"
		);
	}
	for path in actual.entries.keys() {
		anyhow::ensure!(
			expected.entries.contains_key(path),
			"unexpected bundle entry {path:?}"
		);
	}
	Ok(actual.entries.len())
}

#[cfg(test)]
mod tests;
