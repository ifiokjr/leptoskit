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

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;

use crate::cache::hash_bytes;
use crate::wasm::ClientInfo;

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

/// The cargo messages `cargo build --message-format=json` prints that the
/// build reads: compiled artifacts, and whether they are executables.
#[derive(Debug, Deserialize)]
struct ArtifactMessage {
	reason: String,
	target: Option<ArtifactTarget>,
	executable: Option<PathBuf>,
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
			binaries.push((target.name, executable));
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
