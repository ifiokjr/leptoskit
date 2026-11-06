//! Reproducible backend builds. A bundle's source is the exact tree of a Git
//! commit; its dependencies are what `cargo vendor` resolves from the
//! lockfile; its executable is compiled in a digest-pinned builder image with
//! networking disabled. `reproduce` repeats each step and compares bytes, so
//! anyone can check a bundle without trusting whoever built it.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::BufRead as _;
use std::io::BufReader;
use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use anyhow::ensure;
use sha2::Digest as _;
use sha2::Sha256;

use super::BuildRecipe;
use super::Entry;

/// Identifies the build steps in provenance; bump it when they change.
const BUILD_TYPE: &str = "https://leptoskit.dev/verify/backend-build/v1";

/// Identifies this builder in provenance.
const BUILDER_ID: &str = "https://leptoskit.dev/verify/container-builder/v1";

/// `cargo vendor`'s source replacement, kept in `dependencies/`. Cargo skips
/// dot-files in a vendored directory, so it is never read as a crate.
const VENDOR_CONFIG: &str = ".leptoskit-vendor.toml";

/// Every verified build sees this date (1 January 1980, the earliest a zip
/// archive can record), so a date embedded by a build is the same anywhere.
const SOURCE_DATE_EPOCH: &str = "315532800";

/// Where the container sees each input.
const SOURCE_MOUNT: &str = "/build/source";
const DEPENDENCIES_MOUNT: &str = "/build/dependencies";
const WORK_MOUNT: &str = "/build/work";

/// Builds a bundle from a Git commit.
#[derive(Debug, clap::Args)]
pub struct BuildArgs {
	/// The build recipe: the JSON a bundle keeps as build.json.
	#[arg(long)]
	recipe: PathBuf,
	/// A local clone holding the recipe's commit. Without it, the commit is
	/// fetched from the recipe's repository without credentials.
	#[arg(long)]
	checkout: Option<PathBuf>,
	/// The name of an environment variable the backend reads a secret from;
	/// repeat for each. Values are never read.
	#[arg(long = "secret", value_name = "NAME")]
	secrets: Vec<String>,
	/// A new directory for `bundle/`, `inventory.json`, and `provenance.json`.
	#[arg(long)]
	out: PathBuf,
	/// The container engine. Defaults to docker, then podman, whichever
	/// responds.
	#[arg(long, value_enum)]
	engine: Option<Engine>,
}

/// Rebuilds a pinned bundle and compares it with its sources.
#[derive(Debug, clap::Args)]
pub struct ReproduceArgs {
	/// The extracted backend bundle directory.
	#[arg(long)]
	bundle: PathBuf,
	/// The inventory written with the bundle.
	#[arg(long)]
	manifest: PathBuf,
	/// The expected inventory digest, obtained independently of the bundle.
	#[arg(long)]
	sha256: String,
	/// Compare the source with this local clone instead of fetching the
	/// recipe's commit from its repository.
	#[arg(long)]
	checkout: Option<PathBuf>,
	/// Skip the steps that need the network: fetching the commit and
	/// re-resolving dependencies. Only the executable is rebuilt.
	#[arg(long)]
	offline: bool,
	/// The container engine. Defaults to docker, then podman, whichever
	/// responds.
	#[arg(long, value_enum)]
	engine: Option<Engine>,
}

/// A container engine with a docker-compatible command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Engine {
	/// Docker.
	Docker,
	/// Podman.
	Podman,
}

impl Engine {
	const fn program(self) -> &'static str {
		match self {
			Self::Docker => "docker",
			Self::Podman => "podman",
		}
	}

	fn detect(choice: Option<Self>) -> Result<Self> {
		if let Some(engine) = choice {
			return Ok(engine);
		}

		[Self::Docker, Self::Podman]
			.into_iter()
			.find(|engine| {
				Command::new(engine.program())
					.arg("info")
					.stdin(Stdio::null())
					.stdout(Stdio::null())
					.stderr(Stdio::null())
					.status()
					.is_ok_and(|status| status.success())
			})
			.context("verified builds need a running docker or podman engine")
	}
}

/// The container platform that runs a target's compiler natively.
pub(crate) fn platform(target: &str) -> Result<&'static str> {
	Ok(match target {
		"x86_64-unknown-linux-gnu" | "x86_64-unknown-linux-musl" => "linux/amd64",
		"aarch64-unknown-linux-gnu" | "aarch64-unknown-linux-musl" => "linux/arm64",
		_ => {
			bail!("verified builds support x86_64 and aarch64 Linux targets, not {target:?}")
		}
	})
}

/// One container invocation. Nothing from the host environment is passed
/// in; every variable and mount is listed here.
#[derive(Debug)]
pub(crate) struct Run {
	pub(crate) image: String,
	pub(crate) platform: &'static str,
	pub(crate) network: bool,
	/// Host directory, container path, and whether it is read-only.
	pub(crate) mounts: Vec<(PathBuf, &'static str, bool)>,
	pub(crate) env: Vec<(&'static str, String)>,
	pub(crate) user: Option<String>,
	pub(crate) command: Vec<String>,
}

impl Run {
	pub(crate) fn arguments(&self) -> Vec<OsString> {
		let mut arguments: Vec<OsString> = vec![
			"run".into(),
			"--rm".into(),
			"--pull=never".into(),
			"--platform".into(),
			self.platform.into(),
			"--read-only".into(),
			"--tmpfs".into(),
			"/tmp".into(),
			"--cap-drop".into(),
			"ALL".into(),
			"--security-opt".into(),
			"no-new-privileges".into(),
			"--workdir".into(),
			SOURCE_MOUNT.into(),
		];

		if !self.network {
			arguments.extend(["--network".into(), "none".into()]);
		}

		if let Some(user) = &self.user {
			arguments.extend(["--user".into(), user.into()]);
		}

		for (host, container, read_only) in &self.mounts {
			let mut mount = OsString::from("type=bind,source=");
			mount.push(host);
			mount.push(format!(",target={container}"));

			if *read_only {
				mount.push(",readonly=true");
			}

			arguments.extend(["--mount".into(), mount]);
		}

		for (name, value) in &self.env {
			arguments.extend(["--env".into(), format!("{name}={value}").into()]);
		}

		arguments.push(self.image.clone().into());
		arguments.extend(self.command.iter().map(OsString::from));
		arguments
	}
}

/// Runs containers for one recipe.
struct Builder<'a> {
	engine: Engine,
	recipe: &'a BuildRecipe,
	platform: &'static str,
}

/// A bind mount of an existing host directory. `--mount` separates its
/// options with commas, so a path containing one is refused.
fn mount(
	host: &Path,
	container: &'static str,
	read_only: bool,
) -> Result<(PathBuf, &'static str, bool)> {
	let host = host.canonicalize()?;
	ensure!(
		!host.to_string_lossy().contains(','),
		"container mounts cannot use a path containing a comma: {}",
		host.display()
	);
	Ok((host, container, read_only))
}

impl<'a> Builder<'a> {
	/// Pulls the builder image unless `offline`, in which case it must
	/// already be present.
	fn new(engine: Engine, recipe: &'a BuildRecipe, offline: bool) -> Result<Self> {
		let platform = platform(&recipe.target)?;

		if !offline {
			let status = Command::new(engine.program())
				.args(["pull", "--quiet", "--platform", platform])
				.arg(&recipe.builder_image)
				.stdin(Stdio::null())
				.stdout(Stdio::null())
				.status()
				.context("could not run the container engine")?;
			ensure!(
				status.success(),
				"could not pull the builder image {}",
				recipe.builder_image
			);
		}

		Ok(Self {
			engine,
			recipe,
			platform,
		})
	}

	/// A run with the work directory mounted and the builder's fixed
	/// environment.
	fn run(&self, work: &Path, network: bool, command: Vec<String>) -> Result<Run> {
		for directory in ["cargo-home", "target", "tmp"] {
			fs::create_dir_all(work.join(directory))?;
		}

		let mut env = vec![
			("CARGO_HOME", format!("{WORK_MOUNT}/cargo-home")),
			("CARGO_TARGET_DIR", format!("{WORK_MOUNT}/target")),
			("TMPDIR", format!("{WORK_MOUNT}/tmp")),
			("RUSTUP_AUTO_INSTALL", "0".to_owned()),
			("CARGO_TERM_COLOR", "never".to_owned()),
			("CARGO_INCREMENTAL", "0".to_owned()),
			("SOURCE_DATE_EPOCH", SOURCE_DATE_EPOCH.to_owned()),
			("TZ", "UTC".to_owned()),
			("LC_ALL", "C".to_owned()),
		];

		if !network {
			env.push(("CARGO_NET_OFFLINE", "true".to_owned()));
		}

		Ok(Run {
			image: self.recipe.builder_image.clone(),
			platform: self.platform,
			network,
			mounts: vec![mount(work, WORK_MOUNT, false)?],
			env,
			#[cfg(unix)]
			user: self.user(work)?,
			#[cfg(not(unix))]
			user: None,
			command,
		})
	}

	/// Docker runs containers as root; build as the owner of the work
	/// directory so its files stay removable. Rootless podman already maps
	/// root to the calling user.
	#[cfg(unix)]
	fn user(&self, work: &Path) -> Result<Option<String>> {
		use std::os::unix::fs::MetadataExt as _;

		if self.engine != Engine::Docker {
			return Ok(None);
		}

		let metadata = fs::metadata(work)?;
		Ok(Some(format!("{}:{}", metadata.uid(), metadata.gid())))
	}

	/// Runs a container. A captured run returns its standard output and shows
	/// its diagnostics only if it fails; otherwise the build's output streams
	/// through.
	fn execute(&self, run: &Run, capture: bool) -> Result<Vec<u8>> {
		let mut command = Command::new(self.engine.program());
		command.args(run.arguments()).stdin(Stdio::null());
		let output = if capture {
			command.output()?
		} else {
			let status = command.stdout(Stdio::inherit()).status()?;
			std::process::Output {
				status,
				stdout: Vec::new(),
				stderr: Vec::new(),
			}
		};

		if !output.status.success() {
			std::io::stderr().write_all(&output.stderr)?;
			bail!(
				"`{}` failed in the builder container",
				run.command.join(" ")
			);
		}

		Ok(output.stdout)
	}

	/// Vendors every locked dependency into `dependencies`, with the source
	/// replacement Cargo needs to use them offline.
	fn vendor(&self, source: &Path, dependencies: &Path, work: &Path) -> Result<()> {
		let config = self.execute(&self.vendor_run(source, dependencies, work)?, true)?;
		fs::write(dependencies.join(VENDOR_CONFIG), config)?;
		Ok(())
	}

	/// The only step with network access: the source is read-only, and only
	/// `dependencies` and scratch space are writable.
	fn vendor_run(&self, source: &Path, dependencies: &Path, work: &Path) -> Result<Run> {
		let mut run = self.run(
			work,
			true,
			vec![
				"cargo".into(),
				"vendor".into(),
				"--locked".into(),
				"--versioned-dirs".into(),
				DEPENDENCIES_MOUNT.into(),
			],
		)?;
		run.mounts.push(mount(source, SOURCE_MOUNT, true)?);
		run.mounts
			.push(mount(dependencies, DEPENDENCIES_MOUNT, false)?);
		Ok(run)
	}

	/// The compiler and Cargo versions the source's toolchain resolves to.
	fn toolchain(&self, source: &Path, work: &Path) -> Result<String> {
		let mut report = String::new();

		for command in [["rustc", "-vV"], ["cargo", "-V"]] {
			let mut run = self.run(work, false, command.map(String::from).to_vec())?;
			run.mounts.push(mount(source, SOURCE_MOUNT, true)?);
			report.push_str(&String::from_utf8(self.execute(&run, true)?)?);
		}

		Ok(report)
	}

	/// Compiles the recipe's binary without network access and returns the
	/// executable's path under `work`.
	fn compile(&self, source: &Path, dependencies: &Path, work: &Path) -> Result<PathBuf> {
		self.execute(&self.compile_run(source, dependencies, work)?, false)?;
		let executable = work
			.join("target")
			.join(&self.recipe.target)
			.join("release")
			.join(&self.recipe.binary);
		ensure!(
			executable.is_file(),
			"the build finished without producing {}",
			self.recipe.binary
		);
		Ok(executable)
	}

	fn compile_run(&self, source: &Path, dependencies: &Path, work: &Path) -> Result<Run> {
		let recipe = self.recipe;
		let mut command = vec![
			"cargo".to_owned(),
			"build".to_owned(),
			"--release".to_owned(),
			"--frozen".to_owned(),
			"--target".to_owned(),
			recipe.target.clone(),
			"--package".to_owned(),
			recipe.package.clone(),
			"--bin".to_owned(),
			recipe.binary.clone(),
			"--config".to_owned(),
			format!("{DEPENDENCIES_MOUNT}/{VENDOR_CONFIG}"),
		];

		if !recipe.features.is_empty() {
			command.extend(["--features".to_owned(), recipe.features.join(",")]);
		}

		if recipe.no_default_features {
			command.push("--no-default-features".to_owned());
		}

		let mut run = self.run(work, false, command)?;
		// The recipe's list replaces any rustflags in the source's Cargo
		// configuration, so the recipe is the whole story.
		run.env
			.push(("CARGO_ENCODED_RUSTFLAGS", recipe.rustflags.join("\u{1f}")));
		run.mounts.push(mount(source, SOURCE_MOUNT, true)?);
		run.mounts
			.push(mount(dependencies, DEPENDENCIES_MOUNT, true)?);
		Ok(run)
	}
}

/// `git` with no user or system configuration, credentials, or prompts, so
/// configured URL rewrites and helpers cannot change what is read.
fn git(repository: &Path) -> Command {
	let mut command = crate::git::command(repository);

	#[cfg(unix)]
	command.env("GIT_CONFIG_GLOBAL", "/dev/null");

	command
		.env("GIT_CONFIG_NOSYSTEM", "1")
		.env("GIT_TERMINAL_PROMPT", "0")
		.args(["-c", "credential.helper="]);
	command
}

fn git_output(command: &mut Command) -> Result<Vec<u8>> {
	let output = command.stdin(Stdio::null()).output()?;
	ensure!(
		output.status.success(),
		"git failed: {}",
		String::from_utf8_lossy(&output.stderr).trim()
	);
	Ok(output.stdout)
}

/// Fetches one commit from a public repository into a new bare repository.
fn fetch(repository: &str, commit: &str) -> Result<tempfile::TempDir> {
	let directory = tempfile::tempdir()?;
	git_output(git(directory.path()).args(["init", "--quiet", "--bare"]))?;
	git_output(git(directory.path()).args([
		"fetch",
		"--quiet",
		"--depth=1",
		"--no-tags",
		"--",
		repository,
		commit,
	]))
	.with_context(|| format!("could not fetch {commit} from {repository} without credentials"))?;
	Ok(directory)
}

/// Writes the tree of `commit` (or its `prefix` subdirectory) into `dest`
/// exactly: every file Git tracks, its bytes, and its executable bit. Export
/// attributes do not apply, and submodules and symbolic links are rejected.
pub(crate) fn export_tree(
	repository: &Path,
	commit: &str,
	prefix: Option<&str>,
	dest: &Path,
) -> Result<()> {
	let resolved = git_output(git(repository).args([
		"rev-parse",
		"--verify",
		"--quiet",
		&format!("{commit}^{{commit}}"),
	]))
	.with_context(|| format!("commit {commit} is not in {}", repository.display()))?;
	ensure!(
		String::from_utf8_lossy(&resolved).trim() == commit,
		"{commit} must name a commit by its full ID"
	);

	let mut list = git(repository);
	list.args(["ls-tree", "-r", "-z", "--full-tree", commit]);

	if let Some(prefix) = prefix {
		list.args(["--", prefix]);
	}

	let listing = git_output(&mut list)?;
	let mut files = Vec::new();

	for record in listing.split(|byte| *byte == 0).filter(|r| !r.is_empty()) {
		let record = std::str::from_utf8(record).context("repository paths must be UTF-8")?;
		let (meta, path) = record
			.split_once('\t')
			.context("unexpected git ls-tree output")?;
		let mut fields = meta.split(' ');
		let (Some(mode), Some(_), Some(object)) = (fields.next(), fields.next(), fields.next())
		else {
			bail!("unexpected git ls-tree output");
		};
		let executable = match mode {
			"100644" => false,
			"100755" => true,
			"160000" => {
				bail!("{path} is a submodule; verified builds need every source in the commit")
			}
			"120000" => bail!("{path} is a symbolic link, which bundles cannot hold"),
			_ => bail!("{path} has unsupported Git mode {mode}"),
		};
		let relative = match prefix {
			Some(prefix) => {
				path.strip_prefix(prefix)
					.and_then(|rest| rest.strip_prefix('/'))
					.with_context(|| format!("{prefix} must be a directory in the commit"))?
			}
			None => path,
		};
		ensure!(
			relative
				.split('/')
				.all(|part| !matches!(part, "" | "." | "..")),
			"unsafe repository path {path:?}"
		);
		files.push((object.to_owned(), relative.to_owned(), executable));
	}

	if let Some(prefix) = prefix {
		ensure!(!files.is_empty(), "{prefix} is not a directory in {commit}");
	}

	fs::create_dir_all(dest)?;
	let mut batch = git(repository)
		.args(["cat-file", "--batch"])
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()?;
	let mut input = batch.stdin.take().context("git cat-file stdin")?;
	let objects: Vec<String> = files.iter().map(|(object, ..)| object.clone()).collect();
	// Written from a thread so a full output pipe cannot stall the input.
	let writer = std::thread::spawn(move || -> std::io::Result<()> {
		for object in objects {
			writeln!(input, "{object}")?;
		}
		Ok(())
	});
	let mut output = BufReader::new(batch.stdout.take().context("git cat-file stdout")?);

	for (object, relative, executable) in &files {
		let mut header = String::new();
		output.read_line(&mut header)?;
		let mut fields = header.trim_end().split(' ');
		ensure!(
			fields.next() == Some(object.as_str()) && fields.next() == Some("blob"),
			"git returned an unexpected object for {relative}"
		);
		let size: u64 = fields
			.next()
			.context("git cat-file header")?
			.parse()
			.context("git cat-file size")?;
		let path = dest.join(relative);
		fs::create_dir_all(path.parent().context("file paths have a parent")?)?;
		let mut file = fs::File::options()
			.write(true)
			.create_new(true)
			.open(&path)?;
		let copied = std::io::copy(&mut (&mut output).take(size), &mut file)?;
		ensure!(copied == size, "git ended {relative} early");
		let mut newline = [0_u8];
		output.read_exact(&mut newline)?;
		#[cfg(unix)]
		set_mode(&path, if *executable { 0o755 } else { 0o644 })?;
		#[cfg(not(unix))]
		let _ = executable;
	}

	writer
		.join()
		.map_err(|_| anyhow::anyhow!("git cat-file writer panicked"))??;
	ensure!(batch.wait()?.success(), "git cat-file failed");
	Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
	use std::os::unix::fs::PermissionsExt as _;

	fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
	Ok(())
}

/// Requires `actual` to hold exactly the entries of `expected`.
pub(crate) fn compare(expected: &Path, actual: &Path, what: &str) -> Result<()> {
	let expected: BTreeMap<String, Entry> = super::walk(expected)?;
	let actual = super::walk(actual)?;

	for (path, entry) in &expected {
		ensure!(
			actual.get(path) == Some(entry),
			"{what}: {path} is missing or differs"
		);
	}

	for path in actual.keys() {
		ensure!(expected.contains_key(path), "{what}: {path} is extra");
	}

	Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
	let mut hasher = Sha256::new();
	std::io::copy(&mut fs::File::open(path)?, &mut hasher)?;
	Ok(hex::encode(hasher.finalize()))
}

/// The commit's tree in `source/` and its infrastructure directory in
/// `infra/`, both new directories under `bundle`.
fn export_sources(repository: &Path, recipe: &BuildRecipe, bundle: &Path) -> Result<()> {
	export_tree(repository, &recipe.commit, None, &bundle.join("source"))?;

	match &recipe.infra {
		Some(infra) => {
			export_tree(
				repository,
				&recipe.commit,
				Some(infra),
				&bundle.join("infra"),
			)
		}
		None => Ok(fs::create_dir(bundle.join("infra"))?),
	}
}

/// An in-toto statement with a SLSA provenance predicate. It describes the
/// build; it is not signed and proves nothing by itself.
pub(crate) fn provenance(
	recipe: &BuildRecipe,
	executable_sha256: &str,
	inventory_sha256: &str,
	lockfile_sha256: &str,
	toolchain: &str,
) -> serde_json::Value {
	serde_json::json!({
		"_type": "https://in-toto.io/Statement/v1",
		"subject": [
			{ "name": "artifacts/backend", "digest": { "sha256": executable_sha256 } },
			{ "name": "inventory.json", "digest": { "sha256": inventory_sha256 } },
		],
		"predicateType": "https://slsa.dev/provenance/v1",
		"predicate": {
			"buildDefinition": {
				"buildType": BUILD_TYPE,
				"externalParameters": { "recipe": recipe },
				"internalParameters": {
					"network": "none",
					"source_date_epoch": SOURCE_DATE_EPOCH,
					"identical_builds": 2,
				},
				"resolvedDependencies": [
					{
						"uri": format!("git+{}@{}", recipe.repository, recipe.commit),
						"digest": { "gitCommit": recipe.commit },
					},
					{
						"uri": format!("oci://{}", recipe.builder_image),
						"digest": {
							"sha256": recipe
								.builder_image
								.rsplit_once("@sha256:")
								.map_or("", |(_, digest)| digest),
						},
					},
					{ "name": "source/Cargo.lock", "digest": { "sha256": lockfile_sha256 } },
				],
			},
			"runDetails": {
				"builder": {
					"id": BUILDER_ID,
					"version": { "leptoskit": env!("CARGO_PKG_VERSION") },
				},
				"metadata": { "toolchain": toolchain },
			},
		},
	})
}

/// `leptoskit verify build`.
pub(super) fn build(args: &BuildArgs) -> Result<()> {
	let recipe: BuildRecipe = super::read_json(&args.recipe)?;
	super::validate(&recipe)?;
	super::validate_secrets(&args.secrets)?;
	let engine = Engine::detect(args.engine)?;

	let fetched;
	let repository = if let Some(path) = &args.checkout {
		path.as_path()
	} else {
		println!("fetching {} from {}", recipe.commit, recipe.repository);
		fetched = fetch(&recipe.repository, &recipe.commit)?;
		fetched.path()
	};

	fs::create_dir(&args.out)
		.with_context(|| format!("could not create a new directory at {}", args.out.display()))?;
	let out = args.out.canonicalize()?;
	let bundle = out.join("bundle");
	fs::create_dir(&bundle)?;
	export_sources(repository, &recipe, &bundle)?;
	let source = bundle.join("source");

	let builder = Builder::new(engine, &recipe, false)?;
	let scratch = tempfile::tempdir()?;
	let dependencies = bundle.join("dependencies");
	fs::create_dir(&dependencies)?;
	println!("vendoring dependencies from source/Cargo.lock");
	builder.vendor(&source, &dependencies, &scratch.path().join("vendor"))?;
	let toolchain = builder.toolchain(&source, &scratch.path().join("toolchain"))?;

	let compile = |round: &str| -> Result<(PathBuf, String)> {
		println!("compiling {} ({round} build, no network)", recipe.binary);
		let executable = builder.compile(&source, &dependencies, &scratch.path().join(round))?;
		let digest = sha256_file(&executable)?;
		Ok((executable, digest))
	};
	let (executable, first) = compile("first")?;
	let (_, second) = compile("second")?;
	ensure!(
		first == second,
		"the two builds differ (SHA-256 {first} and {second}); the build is not reproducible"
	);

	let artifacts = bundle.join("artifacts");
	fs::create_dir(&artifacts)?;
	fs::copy(&executable, artifacts.join("backend"))?;
	#[cfg(unix)]
	set_mode(&artifacts.join("backend"), 0o755)?;
	fs::write(
		bundle.join("build.json"),
		serde_json::to_vec_pretty(&recipe)?,
	)?;
	fs::write(
		bundle.join("secrets.json"),
		serde_json::to_vec_pretty(&args.secrets)?,
	)?;

	let inventory = super::record(&bundle, &out.join("inventory.json"))?;
	let statement = provenance(
		&recipe,
		&first,
		&inventory,
		&sha256_file(&source.join("Cargo.lock"))?,
		&toolchain,
	);
	fs::write(
		out.join("provenance.json"),
		serde_json::to_vec_pretty(&statement)?,
	)?;

	println!(
		"built {} twice with identical bytes: SHA-256 {first}",
		recipe.binary
	);
	println!("inventory SHA-256: {inventory}");
	println!(
		"Wrote bundle/, inventory.json, and provenance.json to {}. Publish the inventory SHA-256 \
		 where people can authenticate it; nothing here is signed, and no one else has reproduced \
		 the build yet.",
		out.display()
	);
	Ok(())
}

/// `leptoskit verify reproduce`.
pub(super) fn reproduce(args: &ReproduceArgs) -> Result<()> {
	let count = super::check(&args.bundle, &args.manifest, &args.sha256)?;
	println!("bundle matches the pinned inventory ({count} entries)");
	let bundle = args.bundle.canonicalize()?;
	let recipe: BuildRecipe = super::read_json(&bundle.join("build.json"))?;
	let engine = Engine::detect(args.engine)?;
	let scratch = tempfile::tempdir()?;

	let fetched;
	let repository = match (&args.checkout, args.offline) {
		(Some(path), _) => Some(path.as_path()),
		(None, false) => {
			fetched = fetch(&recipe.repository, &recipe.commit)?;
			Some(fetched.path())
		}
		(None, true) => None,
	};

	match repository {
		Some(repository) => {
			let expected = scratch.path().join("expected");
			fs::create_dir(&expected)?;
			export_sources(repository, &recipe, &expected)?;
			let origin = match &args.checkout {
				Some(path) => {
					format!(
						"commit {} in the clone at {} (not fetched from {})",
						recipe.commit,
						path.display(),
						recipe.repository
					)
				}
				None => format!("{} at commit {}", recipe.repository, recipe.commit),
			};
			compare(&expected.join("source"), &bundle.join("source"), "source/")
				.with_context(|| format!("source/ is not {origin}"))?;
			compare(&expected.join("infra"), &bundle.join("infra"), "infra/")
				.with_context(|| format!("infra/ is not {origin}"))?;
			println!("source/ and infra/ are exactly {origin}");
		}
		None => {
			println!(
				"Not checked (--offline): source/ against {}",
				recipe.repository
			);
		}
	}

	let builder = Builder::new(engine, &recipe, args.offline)?;

	if args.offline {
		println!("Not checked (--offline): dependencies/ against source/Cargo.lock");
	} else {
		let dependencies = scratch.path().join("dependencies");
		fs::create_dir(&dependencies)?;
		builder.vendor(
			&bundle.join("source"),
			&dependencies,
			&scratch.path().join("vendor"),
		)?;
		compare(&dependencies, &bundle.join("dependencies"), "dependencies/")
			.context("dependencies/ is not what source/Cargo.lock resolves to")?;
		println!("dependencies/ is exactly what source/Cargo.lock resolves to");
	}

	println!("compiling {} (no network)", recipe.binary);
	let executable = builder.compile(
		&bundle.join("source"),
		&bundle.join("dependencies"),
		&scratch.path().join("build"),
	)?;
	let rebuilt = sha256_file(&executable)?;
	let published = sha256_file(&bundle.join("artifacts/backend"))?;
	ensure!(
		rebuilt == published,
		"artifacts/backend (SHA-256 {published}) was not reproduced: the rebuild is {rebuilt}"
	);
	println!(
		"artifacts/backend was rebuilt byte for byte from source/ and dependencies/ in {}",
		recipe.builder_image
	);
	println!("Not verified: which executable a live server runs, or whether the code is safe.");
	Ok(())
}

#[cfg(test)]
mod tests;
