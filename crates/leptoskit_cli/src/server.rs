//! The app's own server: the binary a build produced, and running it.
//!
//! In a server-rendering mode the app's server binary is what serves the
//! site, not leptoskit's static server: it renders every page and serves the
//! built files next to them. leptos reads where those files are from
//! `LEPTOS_*` environment variables, so `leptoskit dev` and `leptoskit serve`
//! run the binary with them set from `leptoskit.toml`.

use std::io::BufRead;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;

use crate::cache::hash_bytes;
use crate::wasm::ClientInfo;

/// Resolves the backend from the configuration and Cargo targets for both
/// building and serving. Rendering does not determine whether a CSR app
/// has an API server.
pub(crate) fn backend_package<'a>(
	config: &leptoskit_config::LeptoskitConfig,
	metadata: &'a crate::wasm::CargoMetadata,
	platform: leptoskit_config::Platform,
) -> Result<Option<&'a crate::wasm::MetadataPackage>> {
	let package = metadata
		.package(&config.app.name)
		.filter(|package| package.bin_target().is_some());
	let enabled = config.backend_enabled(package.is_some())?;
	anyhow::ensure!(
		platform != leptoskit_config::Platform::Server || enabled,
		"the server platform requires a backend; set build.backend = true and name its package in \
		 app.name"
	);
	Ok(package.filter(|_| enabled))
}

/// The directory under the site root that holds the wasm-bindgen output.
pub const PKG_DIR: &str = "pkg";

/// The leptos environment the server runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeptosEnv {
	/// `leptoskit dev`.
	Dev,
	/// `leptoskit serve`.
	Prod,
}

/// Everything leptos needs to find the built site.
#[derive(Debug, Clone)]
pub struct SiteEnv {
	/// The client's library name: the stem of `pkg/<name>.js`.
	pub output_name: String,
	/// The built site directory, such as `dist/`.
	pub site_root: PathBuf,
	/// Where the server listens.
	pub address: SocketAddr,
	/// The leptos environment.
	pub env: LeptosEnv,
}

impl SiteEnv {
	/// The variables `leptos_config::get_configuration` reads.
	///
	/// `LEPTOS_OUTPUT_NAME` is set at run time only. leptos checks at compile
	/// time whether it was set then, and when it was not, `HydrationScripts`
	/// loads `pkg/<name>_bg.wasm`: exactly the file leptoskit writes.
	#[must_use]
	pub fn vars(&self) -> Vec<(&'static str, String)> {
		vec![
			("LEPTOS_OUTPUT_NAME", self.output_name.clone()),
			("LEPTOS_SITE_ROOT", self.site_root.display().to_string()),
			("LEPTOS_SITE_PKG_DIR", PKG_DIR.to_owned()),
			("LEPTOS_SITE_ADDR", self.address.to_string()),
			(
				"LEPTOS_ENV",
				match self.env {
					LeptosEnv::Dev => "DEV",
					LeptosEnv::Prod => "PROD",
				}
				.to_owned(),
			),
		]
	}
}

/// The `LEPTOS_OUTPUT_NAME` for a build: the client's library name, which
/// names its `pkg/` files, or the app name when there is no client to load.
#[must_use]
pub fn output_name(client: Option<&ClientInfo>, app_name: &str) -> String {
	client.map_or_else(
		|| app_name.replace('-', "_"),
		|client| client.lib_name.clone(),
	)
}

/// Starts `binary` with the site environment, inheriting stdio.
pub fn spawn(binary: &Path, site: &SiteEnv) -> Result<Child> {
	spawn_with(binary, site, &[])
}

/// Starts `binary` like [`spawn`], with `env` set too.
pub fn spawn_with(binary: &Path, site: &SiteEnv, env: &[(String, String)]) -> Result<Child> {
	Command::new(binary)
		.envs(site.vars())
		.envs(env.iter().map(|(name, value)| (name, value)))
		.spawn()
		.with_context(|| format!("could not start the server {}", binary.display()))
}

/// How long `leptoskit serve` gives a process between `SIGTERM` and
/// `SIGKILL`: the runtime's default drain delay (5 s) and grace period
/// (20 s), so a server stops the way it does in production.
pub const SERVE_STOP_GRACE: Duration = Duration::from_secs(25);

/// How long `leptoskit dev` waits instead: nothing sits behind a load
/// balancer there, and services skip their drain delay, so a rebuild
/// doesn't wait on it.
pub const DEV_STOP_GRACE: Duration = Duration::from_secs(5);

/// Fails when something already listens on `address`, naming `what` would
/// have listened there. A process an earlier `leptoskit dev` or `leptoskit
/// serve` left running would otherwise answer in its place: a stale service
/// passes the readiness check, and the new one exits on the bind error.
///
/// # Errors
///
/// When the address is taken.
pub fn ensure_port_free(address: SocketAddr, what: &str) -> Result<()> {
	let hint = || {
		format!(
			"{what} can't listen on {address}: something is already listening there, perhaps a \
			 process an earlier `leptoskit dev` or `leptoskit serve` left running. Stop it (for \
			 example, find it with `lsof -i :{port}`), or give {what} another port",
			port = address.port()
		)
	};

	// A listener on the wildcard address can let a more specific bind
	// through, so a connection that's answered counts as taken too.
	if std::net::TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() {
		anyhow::bail!(hint());
	}

	std::net::TcpListener::bind(address)
		.map(drop)
		.map_err(|error| anyhow::anyhow!("{}: {error}", hint()))
}

/// Stops `children`: `SIGTERM` to each, so they drain the way they do in a
/// deploy, then `SIGKILL` to any still running after `grace`. Each is
/// reaped. On Windows, which has no `SIGTERM`, they're killed at once.
pub fn stop_gracefully(children: &mut [&mut Child], grace: Duration) {
	for child in children.iter_mut() {
		terminate(child);
	}

	let deadline = Instant::now() + grace;

	loop {
		let mut running = 0;

		for child in children.iter_mut() {
			if matches!(child.try_wait(), Ok(None)) {
				running += 1;
			}
		}

		if running == 0 {
			break;
		}

		if Instant::now() >= deadline {
			for child in children.iter_mut() {
				if matches!(child.try_wait(), Ok(None)) {
					let _ = child.kill();
				}
			}

			break;
		}

		std::thread::sleep(Duration::from_millis(50));
	}

	for child in children.iter_mut() {
		let _ = child.wait();
	}
}

/// Asks `child` to stop: `SIGTERM` on Unix, a kill elsewhere.
fn terminate(child: &mut Child) {
	#[cfg(unix)]
	{
		use rustix::process::Pid;
		use rustix::process::Signal;
		use rustix::process::kill_process;

		if let Some(pid) = i32::try_from(child.id()).ok().and_then(Pid::from_raw) {
			let _ = kill_process(pid, Signal::TERM);
		}
	}
	#[cfg(not(unix))]
	{
		let _ = child.kill();
	}
}

/// The cargo messages `cargo build --message-format=json` prints that the
/// build reads: compiled artifacts, and whether they are executables.
#[derive(Debug, Deserialize)]
struct ArtifactMessage {
	reason: String,
	target: Option<ArtifactTarget>,
	executable: Option<PathBuf>,
	/// The `Cargo.toml` of the package the artifact belongs to.
	manifest_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct ArtifactTarget {
	name: String,
	kind: Vec<String>,
}

/// The binary targets in cargo's JSON message stream, as `(target name,
/// executable)` pairs. Lines that are not artifact messages are skipped:
/// cargo interleaves other reasons, such as build script output.
pub fn binaries_in_messages(messages: impl BufRead) -> Result<Vec<(String, PathBuf)>> {
	Ok(packaged_binaries_in_messages(messages)?
		.into_iter()
		.map(|binary| (binary.target, binary.executable))
		.collect())
}

/// One binary cargo built, with the package it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagedBinary {
	/// The `Cargo.toml` of the package that declares it.
	pub manifest_path: PathBuf,
	/// The binary target's name.
	pub target: String,
	/// The executable.
	pub executable: PathBuf,
}

/// The binary targets in cargo's JSON message stream, with the package each
/// belongs to, for a build of several packages at once.
pub fn packaged_binaries_in_messages(messages: impl BufRead) -> Result<Vec<PackagedBinary>> {
	let mut binaries = Vec::new();

	for line in messages.lines() {
		let line = line.context("could not read cargo's output")?;
		let Ok(message) = serde_json::from_str::<ArtifactMessage>(&line) else {
			continue;
		};

		if message.reason != "compiler-artifact" {
			continue;
		}

		if let (Some(target), Some(executable)) = (message.target, message.executable)
			&& target.kind.iter().any(|kind| kind == "bin")
		{
			binaries.push(PackagedBinary {
				manifest_path: message.manifest_path.unwrap_or_default(),
				target: target.name,
				executable,
			});
		}
	}

	Ok(binaries)
}

/// The server among a package's built binaries: the one named after the
/// package, or the only one.
#[must_use]
pub fn pick_server(package: &str, binaries: Vec<(String, PathBuf)>) -> Option<PathBuf> {
	let named = binaries
		.iter()
		.find(|(name, _)| name == package)
		.map(|(_, path)| path.clone());

	if named.is_some() {
		return named;
	}

	let mut binaries = binaries.into_iter();

	match (binaries.next(), binaries.next()) {
		(Some((_, only)), None) => Some(only),
		_ => None,
	}
}

/// The record of the last server binary a project's build produced, kept
/// beside the build cache so `leptoskit serve` runs exactly what
/// `leptoskit build` built.
///
/// A target directory can be shared, through a workspace or a global
/// `CARGO_TARGET_DIR`, so each project's record is keyed by its root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltServer {
	/// The server executable.
	pub binary: PathBuf,
}

impl BuiltServer {
	fn path(target_dir: &Path, project_root: &Path) -> PathBuf {
		let key = hash_bytes(project_root.as_os_str().as_encoded_bytes());
		target_dir
			.join("leptoskit")
			.join(format!("server-{}.json", &key[..16]))
	}

	/// Records this server as the latest build's for the project at
	/// `project_root`.
	pub fn save(&self, target_dir: &Path, project_root: &Path) -> Result<()> {
		let path = Self::path(target_dir, project_root);

		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)
				.with_context(|| format!("could not create {}", parent.display()))?;
		}

		let json = serde_json::to_vec_pretty(self).context("could not encode the server record")?;
		std::fs::write(&path, json).with_context(|| format!("could not write {}", path.display()))
	}

	/// The latest build's server for the project at `project_root`.
	///
	/// # Errors
	///
	/// Returns an error when no build of the project has recorded a server,
	/// or the binary it recorded is gone.
	pub fn load(target_dir: &Path, project_root: &Path) -> Result<Self> {
		let path = Self::path(target_dir, project_root);
		let json =
			std::fs::read(&path).context("no server build found; run `leptoskit build` first")?;
		let record: Self = serde_json::from_slice(&json)
			.with_context(|| format!("could not parse {}", path.display()))?;
		anyhow::ensure!(
			record.binary.is_file(),
			"the last build's server {} is gone; run `leptoskit build` again",
			record.binary.display()
		);
		Ok(record)
	}
}
