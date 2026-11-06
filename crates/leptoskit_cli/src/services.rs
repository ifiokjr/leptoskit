//! Services: other processes the app runs beside its server, declared in
//! `[services.<name>]` and described by each service crate's
//! `[package.metadata.leptoskit.service]`.
//!
//! This module finds each service's code (a workspace package, a crate
//! cargo fetches, or nothing for a remote service), merges its manifest with
//! its table, and builds its binary: workspace services with the app's
//! server in one cargo invocation, installed ones on their own. [`run`]
//! starts them with the runtime contract's environment, and [`proxy`] puts
//! their public paths on the app's origin.

pub mod env;
pub mod proxy;
pub mod run;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use anyhow::Context as _;
use anyhow::Result;
use leptoskit_config::DbDialect;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::ResolvedService;
use leptoskit_config::ServiceManifest;
use leptoskit_config::ServiceSource;
use leptoskit_config::services::GitReference;
use leptoskit_config::services::check_resolved;
use serde::Deserialize;
use serde::Serialize;

use crate::cache::hash_bytes;
use crate::wasm::CargoMetadata;
use crate::wasm::CargoOptions;

/// The file at the project root that pins the registry versions and git
/// commits of installed services: the `Cargo.lock` of the manifest that
/// fetches them.
pub const LOCK_FILE: &str = "services.lock";

/// A service: its merged settings, and where its code is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
	/// The settings, once its manifest and `leptoskit.toml` are merged.
	pub settings: ResolvedService,
	/// Where cargo builds it from.
	pub code: Code,
}

/// Where a service's code is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Code {
	/// A member of the app's Cargo workspace, built with the app's server.
	Workspace {
		/// The package name.
		package: String,
		/// Its `Cargo.toml`.
		manifest: PathBuf,
	},
	/// A crate outside the workspace, built from its own manifest.
	External {
		/// The package name.
		package: String,
		/// Its `Cargo.toml`, wherever cargo put the source.
		manifest: PathBuf,
	},
	/// Nothing: the service runs elsewhere.
	Remote,
}

impl Service {
	/// The service's name.
	#[must_use]
	pub fn name(&self) -> &str {
		&self.settings.name
	}
}

/// Finds every service's code and manifest, merges them with the
/// `[services]` tables, and checks the result. `locked` fetches installed
/// services exactly as `services.lock` pins them, failing rather than
/// updating it, as a release build does.
///
/// # Errors
///
/// When a package isn't in the workspace, cargo can't fetch an installed
/// crate, a manifest has an unknown key, or the merged services collide.
pub fn resolve(
	config: &LeptoskitConfig,
	root: &Path,
	metadata: &CargoMetadata,
	locked: bool,
) -> Result<Vec<Service>> {
	if config.services.is_empty() {
		return Ok(Vec::new());
	}

	let fetched = fetch(config, root, metadata, locked)?;
	let mut services = Vec::new();

	for (name, table) in &config.services {
		let source = table.source(name).map_err(anyhow::Error::msg)?;
		let (code, manifest) = match fetched.get(name) {
			Some((package, manifest_path, manifest)) => {
				(
					Code::External {
						package: package.clone(),
						manifest: manifest_path.clone(),
					},
					manifest.clone(),
				)
			}
			None => locate(name, &source, root, metadata)?,
		};
		let settings =
			ResolvedService::merge(name, table, manifest.as_ref()).map_err(anyhow::Error::msg)?;
		services.push(Service { settings, code });
	}

	let settings: Vec<ResolvedService> = services
		.iter()
		.map(|service| service.settings.clone())
		.collect();
	check_resolved(&settings, config.serve.port)
		.map_err(|problem| anyhow::anyhow!("invalid [services]: {problem}"))?;
	Ok(services)
}

/// Where a workspace, path, or remote `source`'s code is, and the manifest
/// it declares.
fn locate(
	name: &str,
	source: &ServiceSource,
	root: &Path,
	metadata: &CargoMetadata,
) -> Result<(Code, Option<ServiceManifest>)> {
	match source {
		ServiceSource::Package { name: package } => {
			let found = metadata.package(package).with_context(|| {
				format!(
					"services.{name}.package names `{package}`, which isn't a member of the cargo \
					 workspace"
				)
			})?;
			let manifest = manifest_of(name, package, &found.metadata)?;
			Ok((
				Code::Workspace {
					package: package.clone(),
					manifest: found.manifest_path.clone(),
				},
				manifest,
			))
		}

		ServiceSource::Path { path } => {
			let directory = root.join(path);
			let manifest_path = directory.join("Cargo.toml");
			let canonical = manifest_path.canonicalize().with_context(|| {
				format!(
					"services.{name}.path: there's no Cargo.toml at {}",
					directory.display()
				)
			})?;

			// A path inside the app's workspace is built like `package`.
			if let Some(member) = metadata
				.packages
				.iter()
				.find(|package| same_file(&package.manifest_path, &canonical))
			{
				let manifest = manifest_of(name, &member.name, &member.metadata)?;
				return Ok((
					Code::Workspace {
						package: member.name.clone(),
						manifest: member.manifest_path.clone(),
					},
					manifest,
				));
			}

			let outside = cargo_metadata(&canonical, &["--no-deps"])?;
			let package = outside
				.packages
				.iter()
				.find(|package| same_file(&package.manifest_path, &canonical))
				.with_context(|| {
					format!(
						"services.{name}.path: {} holds no package",
						canonical.display()
					)
				})?;
			let manifest = manifest_of(name, &package.name, &package.metadata)?;
			Ok((
				Code::External {
					package: package.name.clone(),
					manifest: canonical,
				},
				manifest,
			))
		}

		ServiceSource::Crate { .. } | ServiceSource::Git { .. } => {
			anyhow::bail!("services.{name} wasn't fetched")
		}

		ServiceSource::Remote { .. } => Ok((Code::Remote, None)),
	}
}

/// Parses a package's `metadata.leptoskit.service`, if it has one.
fn manifest_of(
	name: &str,
	package: &str,
	metadata: &serde_json::Value,
) -> Result<Option<ServiceManifest>> {
	let Some(value) = metadata.pointer("/leptoskit/service") else {
		return Ok(None);
	};

	serde_json::from_value(value.clone())
		.map(Some)
		.with_context(|| {
			format!(
				"services.{name}: `{package}`'s [package.metadata.leptoskit.service] can't be read"
			)
		})
}

/// A fetched service: its package, its manifest, and its service manifest.
type Fetched = (String, PathBuf, Option<ServiceManifest>);

/// Fetches every registry and git service through one generated manifest
/// that depends on them all, whose `Cargo.lock` is kept at the project root
/// as `services.lock`, so two builds of one commit fetch the same code.
fn fetch(
	config: &LeptoskitConfig,
	root: &Path,
	metadata: &CargoMetadata,
	locked: bool,
) -> Result<BTreeMap<String, Fetched>> {
	let mut wanted = Vec::new();

	for (name, table) in &config.services {
		let source = table.source(name).map_err(anyhow::Error::msg)?;

		if matches!(
			source,
			ServiceSource::Crate { .. } | ServiceSource::Git { .. }
		) {
			let (package, dependency) = dependency_of(&source)?;
			wanted.push((name.clone(), package, dependency, source));
		}
	}

	if wanted.is_empty() {
		return Ok(BTreeMap::new());
	}

	let directory = fetch_dir(&metadata.target_directory);
	std::fs::create_dir_all(&directory)
		.with_context(|| format!("could not create {}", directory.display()))?;
	let stub = stub_manifest(
		&wanted
			.iter()
			.map(|(name, package, dependency, _)| {
				(name.as_str(), package.as_str(), dependency.as_str())
			})
			.collect::<Vec<_>>(),
	);
	write_if_changed(&directory.join("Cargo.toml"), &stub)?;
	write_if_changed(&directory.join("lib.rs"), "")?;
	let pinned = root.join(LOCK_FILE);
	let lock = directory.join("Cargo.lock");

	match std::fs::read_to_string(&pinned) {
		Ok(text) => write_if_changed(&lock, &text)?,
		Err(_) if locked => {
			anyhow::bail!(
				"{LOCK_FILE} is missing, so the installed services aren't pinned; run `leptoskit \
				 build` (not --release) or `leptoskit services` once and commit {LOCK_FILE}"
			)
		}
		Err(_) => {
			let _ = std::fs::remove_file(&lock);
		}
	}

	let manifest = directory.join("Cargo.toml");
	let resolved = cargo_metadata(&manifest, if locked { &["--locked"] } else { &[] })
		.context("cargo couldn't fetch the installed services")?;

	if !locked
		&& let Ok(text) = std::fs::read_to_string(&lock)
		&& std::fs::read_to_string(&pinned).ok().as_deref() != Some(text.as_str())
	{
		std::fs::write(&pinned, text)
			.with_context(|| format!("could not write {}", pinned.display()))?;
		crate::ui::tagged(
			"services",
			format_args!("pinned the installed services in {LOCK_FILE}"),
		);
	}

	let mut fetched = BTreeMap::new();

	for (name, package, _, source) in wanted {
		let found = resolved
			.packages
			.iter()
			.filter(|candidate| {
				candidate.name == package && !same_file(&candidate.manifest_path, &manifest)
			})
			.find(|candidate| {
				let origin = candidate.source.as_deref().unwrap_or_default();
				match &source {
					ServiceSource::Git { url, .. } => {
						origin.starts_with("git+") && origin.contains(url.trim_end_matches(".git"))
					}
					_ => origin.starts_with("registry+") || origin.starts_with("sparse+"),
				}
			})
			.with_context(|| {
				format!("services.{name}: cargo fetched no package named `{package}`")
			})?;
		let service = manifest_of(&name, &package, &found.metadata)?;
		fetched.insert(name, (package, found.manifest_path.clone(), service));
	}

	Ok(fetched)
}

/// Where the manifest that fetches installed services lives.
fn fetch_dir(target_directory: &Path) -> PathBuf {
	target_directory
		.join("leptoskit")
		.join("services")
		.join("fetch")
}

fn write_if_changed(path: &Path, contents: &str) -> Result<()> {
	if std::fs::read_to_string(path).ok().as_deref() != Some(contents) {
		std::fs::write(path, contents)
			.with_context(|| format!("could not write {}", path.display()))?;
	}

	Ok(())
}

/// The package a fetched source builds, and its dependency entry as TOML.
fn dependency_of(source: &ServiceSource) -> Result<(String, String)> {
	let quote = |value: &str| toml::Value::String(value.to_owned()).to_string();

	match source {
		ServiceSource::Crate { name, version } => {
			Ok((
				name.clone(),
				format!(
					"{{ package = {}, version = {} }}",
					quote(name),
					quote(version.as_deref().unwrap_or("*"))
				),
			))
		}

		ServiceSource::Git {
			url,
			package,
			reference,
		} => {
			let reference = match reference {
				Some(GitReference::Rev(rev)) => format!(", rev = {}", quote(rev)),
				Some(GitReference::Branch(branch)) => format!(", branch = {}", quote(branch)),
				Some(GitReference::Tag(tag)) => format!(", tag = {}", quote(tag)),
				None => String::new(),
			};
			Ok((
				package.clone(),
				format!(
					"{{ package = {}, git = {}{reference} }}",
					quote(package),
					quote(url)
				),
			))
		}

		_ => anyhow::bail!("only registry and git services are fetched"),
	}
}

/// The manifest that makes cargo fetch the installed services: one
/// dependency per service, named after it so two services can't collide.
fn stub_manifest(services: &[(&str, &str, &str)]) -> String {
	let mut manifest = String::from(
		"# Written by leptoskit to fetch the installed services; don't edit.\n[package]\nname = \
		 \"leptoskit-services-fetch\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = \
		 false\n\n[lib]\npath = \"lib.rs\"\n\n[workspace]\n\n[dependencies]\n",
	);

	for (name, _, dependency) in services {
		let _ = writeln!(manifest, "service-{name} = {dependency}");
	}

	manifest
}

/// One package in `cargo metadata`'s output, with what fetching reads.
#[derive(Debug, Deserialize)]
struct FetchedPackage {
	name: String,
	manifest_path: PathBuf,
	#[serde(default)]
	source: Option<String>,
	#[serde(default)]
	metadata: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct FetchedMetadata {
	packages: Vec<FetchedPackage>,
}

/// `cargo metadata` for `manifest`, with `args`.
fn cargo_metadata(manifest: &Path, args: &[&str]) -> Result<FetchedMetadata> {
	let output = Command::new("cargo")
		.args(["metadata", "--format-version", "1", "--manifest-path"])
		.arg(manifest)
		.args(args)
		.output()
		.context("could not run cargo metadata")?;
	anyhow::ensure!(
		output.status.success(),
		"cargo metadata failed: {}",
		String::from_utf8_lossy(&output.stderr).trim()
	);
	serde_json::from_slice(&output.stdout).context("could not parse cargo metadata")
}

/// A service and the binary a build made for it: `None` for a remote
/// service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltService {
	/// The service.
	pub service: Service,
	/// Its executable.
	pub binary: Option<PathBuf>,
}

/// What [`build`] made: the app's server, when it was asked to build one,
/// and every service.
#[derive(Debug, Clone, Default)]
pub struct Built {
	/// The app's server executable.
	pub app: Option<PathBuf>,
	/// Every service, in name order.
	pub services: Vec<BuiltService>,
}

/// The app's server package and the options its build takes.
pub struct AppBuild<'a> {
	/// The package.
	pub package: &'a str,
	/// Its cargo options, with the render mode's features.
	pub options: &'a CargoOptions,
}

/// Builds the app's server (when `app` is given) and every workspace
/// service in one cargo invocation, then each installed service in its own.
///
/// One invocation means the app's server and its services share every
/// dependency's compiled artifacts. Cargo unifies features across it, so a
/// service's `service` feature is on in the app's server too; that's safe
/// because `leptoskit_rpc` serves an operation only in its own service and
/// routes every call to it there.
///
/// # Errors
///
/// When a build fails or makes no binary to run.
pub fn build(
	root: &Path,
	metadata: &CargoMetadata,
	app: Option<&AppBuild<'_>>,
	services: &[Service],
	release: bool,
) -> Result<Built> {
	let workspace: Vec<&Service> = services
		.iter()
		.filter(|service| matches!(service.code, Code::Workspace { .. }))
		.collect();
	let mut built = Built::default();
	let mut binaries = Vec::new();

	if app.is_some() || !workspace.is_empty() {
		let base = app.map_or_else(
			|| {
				CargoOptions {
					release,
					..CargoOptions::default()
				}
			},
			|app| app.options.clone(),
		);
		let mut features: Vec<String> = match app {
			Some(app) => {
				base.features
					.iter()
					.map(|feature| qualify(app.package, feature))
					.collect()
			}
			None => Vec::new(),
		};
		let mut packages: Vec<&str> = app.map(|app| app.package).into_iter().collect();

		for service in &workspace {
			let Code::Workspace { package, .. } = &service.code else {
				continue;
			};

			if !packages.contains(&package.as_str()) {
				packages.push(package);
			}

			for feature in &service.settings.features {
				let feature = qualify(package, feature);

				if !features.contains(&feature) {
					features.push(feature);
				}
			}
		}

		let options = CargoOptions { features, ..base };
		let (first, rest) = packages.split_first().context("nothing to build")?;
		let mut command = crate::wasm::cargo_build_command(root, first, &options);

		for package in rest {
			command.arg("--package").arg(package);
		}

		if !workspace.is_empty() {
			let names: Vec<&str> = workspace.iter().map(|service| service.name()).collect();
			crate::ui::tagged(
				"services",
				format_args!("building {} with the app", names.join(", ")),
			);
		}

		binaries = run_cargo(command, "the app and its services")?;

		if let Some(app) = app {
			let manifest = package_manifest(metadata, app.package);
			built.app = crate::server::pick_server(
				app.package,
				binaries
					.iter()
					.filter(|binary| {
						manifest
							.as_deref()
							.is_none_or(|path| same_file(path, &binary.manifest_path))
					})
					.map(|binary| (binary.target.clone(), binary.executable.clone()))
					.collect(),
			);
		}
	}

	for service in services {
		let binary = match &service.code {
			Code::Remote => None,
			Code::Workspace { package, manifest } => {
				Some(pick(
					service,
					package,
					binaries
						.iter()
						.filter(|binary| same_file(manifest, &binary.manifest_path))
						.cloned()
						.collect(),
				)?)
			}
			Code::External { package, manifest } => {
				Some(build_external(
					service, package, manifest, metadata, release,
				)?)
			}
		};

		built.services.push(BuiltService {
			service: service.clone(),
			binary,
		});
	}

	Ok(built)
}

/// `feature` for `package` in a build of several packages: a plain name is
/// the package's own, and `dep/feature` stays as written.
fn qualify(package: &str, feature: &str) -> String {
	if feature.contains('/') {
		feature.to_owned()
	} else {
		format!("{package}/{feature}")
	}
}

fn package_manifest(metadata: &CargoMetadata, package: &str) -> Option<PathBuf> {
	metadata
		.package(package)
		.map(|package| package.manifest_path.clone())
}

fn same_file(a: &Path, b: &Path) -> bool {
	a == b
		|| match (a.canonicalize(), b.canonicalize()) {
			(Ok(a), Ok(b)) => a == b,
			_ => false,
		}
}

/// Runs a cargo build and returns the binaries it reported.
fn run_cargo(mut command: Command, what: &str) -> Result<Vec<crate::server::PackagedBinary>> {
	let mut child = command
		.arg("--message-format=json-render-diagnostics")
		.stdout(Stdio::piped())
		.spawn()
		.context("could not run cargo")?;
	let messages = child
		.stdout
		.take()
		.context("cargo's output was not captured")?;
	let binaries = crate::server::packaged_binaries_in_messages(BufReader::new(messages))?;
	let status = child.wait().context("could not wait for cargo")?;
	anyhow::ensure!(status.success(), "cargo build of {what} failed");
	Ok(binaries)
}

/// The service's binary among those its package built: `bin`, or the one
/// named after the package, or the only one.
fn pick(
	service: &Service,
	package: &str,
	binaries: Vec<crate::server::PackagedBinary>,
) -> Result<PathBuf> {
	let name = service.name();
	let binary = match &service.settings.bin {
		Some(bin) => {
			binaries
				.into_iter()
				.find(|binary| &binary.target == bin)
				.map(|binary| binary.executable)
		}
		None => {
			crate::server::pick_server(
				package,
				binaries
					.into_iter()
					.map(|binary| (binary.target, binary.executable))
					.collect(),
			)
		}
	};

	binary.with_context(|| {
		format!(
			"services.{name}: `{package}` built no binary to run, or several and none named after \
			 it; set `bin`, and check the features its binary needs (`features`)"
		)
	})
}

/// Builds an installed service from its own manifest, into a target
/// directory of its own, pinned by its own `Cargo.lock` in a release.
fn build_external(
	service: &Service,
	package: &str,
	manifest: &Path,
	metadata: &CargoMetadata,
	release: bool,
) -> Result<PathBuf> {
	let target = metadata
		.target_directory
		.join("leptoskit")
		.join("services")
		.join("target");
	let mut command = Command::new("cargo");
	command
		.arg("build")
		.arg("--manifest-path")
		.arg(manifest)
		.arg("--package")
		.arg(package)
		.arg("--target-dir")
		.arg(&target);

	if release {
		command.arg("--release");

		if lock_beside(manifest).is_some() {
			command.arg("--locked");
		} else {
			crate::ui::warning(format_args!(
				"services.{}: `{package}` ships no Cargo.lock, so its dependencies aren't pinned",
				service.name()
			));
		}
	}

	let features: Vec<String> = service
		.settings
		.features
		.iter()
		.map(|feature| qualify(package, feature))
		.collect();

	if !features.is_empty() {
		command.arg("--features").arg(features.join(","));
	}

	if let Some(bin) = &service.settings.bin {
		command.arg("--bin").arg(bin);
	}

	crate::ui::tagged(service.name(), format_args!("building `{package}`"));
	let binaries = run_cargo(command, &format!("services.{}", service.name()))?;
	pick(service, package, binaries)
}

/// The `Cargo.lock` that governs the package at `manifest`: beside it, or in
/// an ancestor workspace.
fn lock_beside(manifest: &Path) -> Option<PathBuf> {
	manifest
		.ancestors()
		.skip(1)
		.map(|directory| directory.join("Cargo.lock"))
		.find(|lock| lock.is_file())
}

/// The record of the last build's services, kept beside the build cache so
/// `leptoskit serve` runs exactly what `leptoskit build` built.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltServices {
	/// Every service, in name order.
	pub services: Vec<BuiltService>,
}

impl BuiltServices {
	fn path(target_dir: &Path, project_root: &Path) -> PathBuf {
		let key = hash_bytes(project_root.as_os_str().as_encoded_bytes());
		target_dir
			.join("leptoskit")
			.join(format!("services-{}.json", &key[..16]))
	}

	/// Records these as the latest build's services for the project at
	/// `project_root`.
	///
	/// # Errors
	///
	/// When the record can't be written.
	pub fn save(&self, target_dir: &Path, project_root: &Path) -> Result<()> {
		let path = Self::path(target_dir, project_root);

		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)
				.with_context(|| format!("could not create {}", parent.display()))?;
		}

		let json =
			serde_json::to_vec_pretty(self).context("could not encode the services record")?;
		std::fs::write(&path, json).with_context(|| format!("could not write {}", path.display()))
	}

	/// The latest build's services for the project at `project_root`: none
	/// when no build recorded any.
	///
	/// # Errors
	///
	/// When the record can't be parsed, or a binary it names is gone.
	pub fn load(target_dir: &Path, project_root: &Path) -> Result<Self> {
		let path = Self::path(target_dir, project_root);
		let Ok(json) = std::fs::read(&path) else {
			return Ok(Self::default());
		};
		let record: Self = serde_json::from_slice(&json)
			.with_context(|| format!("could not parse {}", path.display()))?;

		for built in &record.services {
			if let Some(binary) = &built.binary {
				anyhow::ensure!(
					binary.is_file(),
					"the last build's `{}` service {} is gone; run `leptoskit build` again",
					built.service.name(),
					binary.display()
				);
			}
		}

		Ok(record)
	}
}

/// Where a service's dev files live: `.leptoskit/services/<name>/`.
#[must_use]
pub fn dev_dir(root: &Path, service: &str) -> PathBuf {
	root.join(".leptoskit").join("services").join(service)
}

/// The SQLite file a service's database is in development.
#[must_use]
pub fn sqlite_path(root: &Path, service: &str) -> PathBuf {
	dev_dir(root, service).join(format!("{service}.sqlite3"))
}

/// The name of a service's Postgres database, and of the role that owns
/// it, beside the app's database `app`: `<app>_service_<service>`.
///
/// The `_service_` segment keeps services out of the names leptoskit uses
/// itself (the managed `leptoskit` database, the `leptoskit_admin`
/// superuser, `app__<branch>` branch databases), so no service name can take
/// one over. A name past Postgres's 63 bytes is shortened with a hash of the
/// service's name, the way branch databases are.
#[must_use]
pub fn postgres_database(app: &str, service: &str) -> String {
	const MAX_NAME: usize = 63;
	let name = format!("{app}_service_{}", service.replace('-', "_"));

	if name.len() <= MAX_NAME {
		return name;
	}

	let hash = &leptoskit_db::checksum(service)[..8];
	let mut end = MAX_NAME - hash.len() - 1;

	while !name.is_char_boundary(end) {
		end -= 1;
	}

	format!("{}_{hash}", &name[..end])
}

/// `leptoskit services`: lists the resolved services.
///
/// # Errors
///
/// When the configuration or a service can't be resolved.
pub fn command(json: bool) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let metadata = crate::wasm::cargo_metadata(&root)?;
	let services = resolve(&config, &root, &metadata, false)?;
	let listing = listing(&services, &config, &root);

	if json {
		println!("{}", serde_json::to_string_pretty(&listing)?);
		return Ok(());
	}

	if services.is_empty() {
		println!("no services: add a [services.<name>] table to leptoskit.toml");
		return Ok(());
	}

	for entry in &listing.services {
		print_entry(entry);
	}

	Ok(())
}

fn print_entry(entry: &Entry) {
	use crate::ui::BOLD;
	use crate::ui::DIM;

	let port = entry
		.port
		.map_or_else(String::new, |port| format!("  port {port}"));
	anstream::println!(
		"{BOLD}{}{BOLD:#}  {} {}{port}  {DIM}{}{DIM:#}",
		entry.name,
		entry.source.kind,
		entry.source.detail,
		entry.trust
	);

	if !entry.public_paths.is_empty() {
		anstream::println!("  {DIM}public{DIM:#}    {}", entry.public_paths.join(", "));
	}

	if let Some(database) = &entry.database {
		let pool = database
			.pool
			.map_or_else(String::new, |pool| format!(", pool {pool}"));
		anstream::println!(
			"  {DIM}database{DIM:#}  {} {}{pool}",
			database.engine,
			database.location
		);
	}

	for volume in &entry.volumes {
		anstream::println!("  {DIM}volume{DIM:#}    {} → {}", volume.name, volume.path);
	}

	anstream::println!("  {DIM}app gets{DIM:#}  {}", entry.bindings.join(", "));
}

/// What `leptoskit services --json` prints.
#[derive(Debug, Serialize)]
pub struct Listing {
	/// Every service, in name order.
	pub services: Vec<Entry>,
}

/// One service, as listed. Never holds a token or a password.
#[derive(Debug, Serialize)]
pub struct Entry {
	/// Its name.
	pub name: String,
	/// Where it comes from.
	pub source: EntrySource,
	/// Its port; `None` for a remote service.
	pub port: Option<u16>,
	/// `first-party` or `isolated`.
	pub trust: &'static str,
	/// The paths browsers reach it at on the app's origin.
	pub public_paths: Vec<String>,
	/// Its database, when it has one.
	pub database: Option<EntryDatabase>,
	/// Its volumes.
	pub volumes: Vec<EntryVolume>,
	/// The variables the app receives for it.
	pub bindings: Vec<String>,
	/// The cargo features its binary is built with.
	pub features: Vec<String>,
}

/// A source, as listed.
#[derive(Debug, Serialize)]
pub struct EntrySource {
	/// `package`, `path`, `crate`, `git`, or `remote`.
	pub kind: &'static str,
	/// The package, path, crate, repository, or URL.
	pub detail: String,
}

/// A database, as listed.
#[derive(Debug, Serialize)]
pub struct EntryDatabase {
	/// `sqlite` or `postgres`.
	pub engine: &'static str,
	/// The file, relative to the project, or the database's name.
	pub location: String,
	/// The most connections its pool opens, when set.
	pub pool: Option<u32>,
}

/// A volume, as listed.
#[derive(Debug, Serialize)]
pub struct EntryVolume {
	/// Its name.
	pub name: String,
	/// Its directory in development, relative to the project.
	pub path: String,
}

/// The listing for `services`.
#[must_use]
pub fn listing(services: &[Service], config: &LeptoskitConfig, root: &Path) -> Listing {
	let relative = |path: PathBuf| {
		path.strip_prefix(root)
			.unwrap_or(&path)
			.display()
			.to_string()
	};
	let services = services
		.iter()
		.map(|service| {
			let settings = &service.settings;
			let name = settings.name.clone();
			let detail = match &settings.source {
				ServiceSource::Package { name } => name.clone(),
				ServiceSource::Path { path } => path.display().to_string(),
				ServiceSource::Crate { name, version } => {
					format!("{name} {}", version.as_deref().unwrap_or("*"))
				}
				ServiceSource::Git {
					url,
					package,
					reference,
				} => {
					let reference = match reference {
						Some(GitReference::Rev(rev)) => format!(" rev {rev}"),
						Some(GitReference::Branch(branch)) => format!(" branch {branch}"),
						Some(GitReference::Tag(tag)) => format!(" tag {tag}"),
						None => String::new(),
					};
					format!("{package} from {url}{reference}")
				}
				ServiceSource::Remote { url } => url.clone(),
			};
			let database = settings.database.then(|| {
				match config.db.as_ref().map(|db| db.dialect) {
					Some(DbDialect::Postgres) => {
						EntryDatabase {
							engine: "postgres",
							location: format!(
								"{} on the app's server",
								postgres_database("<app database>", &name)
							),
							pool: settings.pool,
						}
					}
					Some(DbDialect::Sqlite) | None => {
						EntryDatabase {
							engine: "sqlite",
							location: relative(sqlite_path(root, &name)),
							pool: settings.pool,
						}
					}
				}
			});
			let volumes = settings
				.volumes
				.iter()
				.map(|volume| {
					EntryVolume {
						name: volume.clone(),
						path: relative(dev_dir(root, &name).join(volume)),
					}
				})
				.collect();
			let bindings = [env::url(&name), env::token(&name)]
				.into_iter()
				.chain(settings.bind.keys().cloned())
				.collect();

			Entry {
				source: EntrySource {
					kind: settings.source.kind(),
					detail,
				},
				port: settings.port,
				trust: settings.trust.as_str(),
				public_paths: settings.public_prefixes(),
				database,
				volumes,
				bindings,
				features: settings.features.clone(),
				name,
			}
		})
		.collect();

	Listing { services }
}
