//! `leptoskit dev` for extensions: an unpacked Chromium build, loaded into
//! Chrome for Testing, and reloaded after every rebuild.
//!
//! Chrome 137 and later ignore `--load-extension`; Chrome for Testing, the
//! build of Chrome meant for automation, still honours it. The development
//! worker long-polls a small server here for the build it runs, and reloads
//! the extension when a newer one is ready.

use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use axum::extract::Query;
use axum::extract::State;
use axum::routing::get;
use serde::Deserialize;
use sha2::Digest;
use tokio::sync::watch;

use super::manifest::DevManifest;

/// The environment variable that names a Chrome for Testing executable.
pub const CHROME_ENV: &str = "LEPTOSKIT_CHROME";

/// How long one reload poll waits for a new build.
const POLL: Duration = Duration::from_secs(25);

/// Chrome for Testing: `LEPTOSKIT_CHROME`, or the newest one the Chrome for
/// Testing installer (`npx @puppeteer/browsers install chrome@stable`) put
/// in its cache.
#[must_use]
pub fn chrome() -> Option<PathBuf> {
	if let Some(path) = std::env::var_os(CHROME_ENV) {
		return Some(PathBuf::from(path)).filter(|path| path.is_file());
	}
	let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
	let cache = PathBuf::from(home).join(".cache/puppeteer/chrome");
	let mut installs: Vec<PathBuf> = std::fs::read_dir(cache)
		.ok()?
		.filter_map(Result::ok)
		.map(|entry| entry.path())
		.collect();
	installs.sort();
	installs.into_iter().rev().find_map(|install| {
		[
			"chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for \
			 Testing",
			"chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
			"chrome-linux64/chrome",
			"chrome-win64/chrome.exe",
		]
		.into_iter()
		.map(|relative| install.join(relative))
		.find(|path| path.is_file())
	})
}

/// Why this machine can't run an extension in development, if it can't.
///
/// # Errors
///
/// A sentence naming what to install.
pub fn support() -> Result<(), String> {
	chrome().map(drop).ok_or_else(|| {
		format!(
			"extensions run in Chrome for Testing: install it with `npx @puppeteer/browsers \
			 install chrome@stable --path ~/.cache/puppeteer`, or set {CHROME_ENV} to its \
			 executable"
		)
	})
}

/// The ID Chromium gives an unpacked extension loaded from `dir`: the first
/// 32 hex digits of the SHA-256 of its absolute path, written with the
/// letters `a` to `p`.
#[must_use]
pub fn unpacked_id(dir: &Path) -> String {
	let path = dir.to_string_lossy();
	let bytes: Vec<u8> = if cfg!(windows) {
		path.encode_utf16().flat_map(u16::to_le_bytes).collect()
	} else {
		path.as_bytes().to_vec()
	};
	let digest = sha2::Sha256::digest(bytes);
	digest[..16]
		.iter()
		.flat_map(|byte| [byte >> 4, byte & 0x0f])
		.map(|nibble| char::from(b'a' + nibble))
		.collect()
}

#[derive(Deserialize)]
struct After {
	#[serde(default)]
	after: String,
}

/// Answers with the current build, or waits for the next one when the
/// caller already has it.
async fn build(
	State(builds): State<watch::Receiver<String>>,
	Query(query): Query<After>,
) -> String {
	let mut builds = builds.clone();
	let current = builds.borrow_and_update().clone();
	if current != query.after {
		return current;
	}
	match tokio::time::timeout(POLL, builds.changed()).await {
		Ok(Ok(())) => builds.borrow().clone(),
		_ => current,
	}
}

/// The extension half of `leptoskit dev`.
pub struct ExtensionDev {
	builds: watch::Sender<String>,
	server: String,
	dir: PathBuf,
	chrome: Option<Child>,
	generation: u64,
}

impl ExtensionDev {
	/// Starts the reload server, builds the extension, and opens it in
	/// Chrome for Testing.
	///
	/// # Errors
	///
	/// When Chrome for Testing is missing, or the first build fails.
	pub fn start(root: &Path, runtime: &tokio::runtime::Runtime) -> Result<Self> {
		let chrome = chrome().context(support().err().unwrap_or_default())?;
		let (builds, receiver) = watch::channel(String::new());
		let listener = std::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
			.context("could not start the extension's reload server")?;
		listener.set_nonblocking(true)?;
		let server = format!("http://{}", listener.local_addr()?);
		let listener = {
			let _guard = runtime.enter();
			tokio::net::TcpListener::from_std(listener)?
		};
		let router = axum::Router::new()
			.route("/_leptoskit/extension/build", get(build))
			.with_state(receiver);
		runtime.spawn(async move {
			if let Err(error) = axum::serve(listener, router).await {
				eprintln!("[extension] reload server stopped: {error}");
			}
		});

		let mut dev = Self {
			builds,
			server,
			dir: PathBuf::new(),
			chrome: None,
			generation: 0,
		};
		dev.rebuild_or_fail()?;
		let profile = root.join(super::OUTPUT).join("profile");
		std::fs::create_dir_all(&profile)?;
		let child = Command::new(&chrome)
			.arg(format!("--user-data-dir={}", profile.display()))
			.arg(format!("--load-extension={}", dev.dir.display()))
			.arg(format!("--disable-extensions-except={}", dev.dir.display()))
			.args(["--no-first-run", "--no-default-browser-check"])
			.arg(format!("chrome-extension://{}/", unpacked_id(&dev.dir)))
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.with_context(|| format!("could not start {}", chrome.display()))?;
		dev.chrome = Some(child);
		println!(
			"[extension] loaded {} into Chrome for Testing; it reloads after each rebuild, \
			 announced at {}",
			dev.dir.display(),
			dev.server
		);
		Ok(dev)
	}

	fn rebuild_or_fail(&mut self) -> Result<()> {
		let manifest = DevManifest {
			key: None,
			server: self.server.clone(),
		};
		let built = super::build(false, Some(&manifest))?;
		let package = built
			.packages
			.into_iter()
			.next()
			.context("the development build made no package")?;
		self.dir = package.dir.canonicalize().unwrap_or(package.dir);
		self.generation += 1;
		let _ = self.builds.send(self.generation.to_string());
		Ok(())
	}

	/// Rebuilds after a change; the extension reloads itself once the build
	/// is ready.
	pub fn rebuild(&mut self) {
		match self.rebuild_or_fail() {
			Ok(()) => println!("[extension] rebuilt; reloading the extension"),
			Err(error) => eprintln!("[extension] rebuild failed: {error:#}"),
		}
	}
}

impl Drop for ExtensionDev {
	fn drop(&mut self) {
		if let Some(child) = &mut self.chrome {
			let _ = child.kill();
			let _ = child.wait();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn unpacked_ids_follow_chromium_s_rule() {
		let id = unpacked_id(Path::new("/tmp/extension"));
		assert_eq!(id.len(), 32);
		assert!(id.bytes().all(|byte| (b'a'..=b'p').contains(&byte)));
		assert_eq!(id, unpacked_id(Path::new("/tmp/extension")));
		assert_ne!(id, unpacked_id(Path::new("/tmp/other")));
	}

	#[tokio::test]
	async fn the_reload_poll_answers_at_once_or_on_the_next_build() {
		let (sender, receiver) = watch::channel("1".to_owned());
		let first = build(
			State(receiver.clone()),
			Query(After {
				after: String::new(),
			}),
		)
		.await;
		assert_eq!(first, "1");
		let waiting = tokio::spawn(build(State(receiver), Query(After { after: "1".into() })));
		tokio::time::sleep(Duration::from_millis(20)).await;
		sender.send("2".into()).unwrap();
		assert_eq!(waiting.await.unwrap(), "2");
	}
}
