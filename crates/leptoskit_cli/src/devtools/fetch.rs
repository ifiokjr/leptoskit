//! Downloading the `DevTools` interface for exactly this CLI's version.
//!
//! Each release builds the interface in `devtools/` once and uploads
//! `leptoskit-devtools-<version>.tar.gz` to the GitHub release `v<version>`.
//! The archive's SHA-256 is compiled into the CLI (see `build.rs`), so the
//! download is checked against the release the binary itself came from. The
//! CLI never falls back to another version, because the protocol and the
//! interface change together, and a hash mismatch is an error that is never
//! retried silently.

use std::fs;
use std::io::Read;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use flate2::read::GzDecoder;
use sha2::Digest;
use sha2::Sha256;

use super::ui;

/// Where releases are downloaded from.
pub const RELEASES: &str = "https://github.com/ifiokjr/leptoskit/releases/download";

/// Replaces [`RELEASES`], for a mirror. The archive is still checked against
/// the hash built into the CLI, so a mirror can't change what is served.
pub const RELEASES_ENV: &str = "LEPTOSKIT_DEVTOOLS_RELEASES";

/// The largest archive the CLI downloads.
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;

/// The most the archive may hold once unpacked.
const MAX_UNPACKED: u64 = 256 * 1024 * 1024;

/// The SHA-256 of the interface archive released with this CLI, when it was
/// built by the release workflow or installed from crates.io.
#[must_use]
pub fn expected_hash() -> Option<&'static str> {
	option_env!("LEPTOSKIT_DEVTOOLS_UI_HASH")
}

/// One release's interface archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
	/// The base URL releases are downloaded from.
	pub base: String,
	/// The CLI version, without a `v`.
	pub version: String,
	/// The archive's expected SHA-256, in hex.
	pub sha256: String,
}

impl Release {
	/// This CLI's release, when it knows its archive's hash.
	#[must_use]
	pub fn current() -> Option<Self> {
		expected_hash().map(|sha256| {
			Self {
				base: std::env::var(RELEASES_ENV)
					.ok()
					.filter(|base| !base.is_empty())
					.unwrap_or_else(|| RELEASES.to_owned()),
				version: env!("CARGO_PKG_VERSION").to_owned(),
				sha256: sha256.to_owned(),
			}
		})
	}

	/// The archive's file name.
	#[must_use]
	pub fn archive(&self) -> String {
		format!("leptoskit-devtools-{}.tar.gz", self.version)
	}

	/// The archive's address.
	#[must_use]
	pub fn url(&self) -> String {
		format!(
			"{}/v{}/{}",
			self.base.trim_end_matches('/'),
			self.version,
			self.archive()
		)
	}

	/// Downloads the archive, checks its hash, and unpacks it into `dir`,
	/// replacing whatever was there only once the new build is complete.
	///
	/// # Errors
	///
	/// When the release has no archive, the download fails, the hash does
	/// not match, or the archive is not a build of the interface.
	pub fn install(&self, dir: &Path) -> Result<()> {
		let bytes = self.download()?;
		let actual = hex::encode(Sha256::digest(&bytes));

		anyhow::ensure!(
			actual.eq_ignore_ascii_case(&self.sha256),
			"the DevTools interface downloaded from {} has SHA-256 {actual}, but leptoskit {} \
			 expects {}. It was not used. The download may have been tampered with or corrupted; \
			 check your network or proxy, then run `leptoskit devtools fetch` again.",
			self.url(),
			self.version,
			self.sha256,
		);

		unpack_into(&bytes, dir)
	}

	fn download(&self) -> Result<Vec<u8>> {
		let url = self.url();
		let agent: ureq::Agent = ureq::Agent::config_builder()
			.timeout_global(Some(Duration::from_secs(120)))
			.http_status_as_error(false)
			.build()
			.into();
		let mut response = agent.get(&url).call().with_context(|| {
			format!(
				"could not reach {url}; check your connection, then run `leptoskit devtools fetch`"
			)
		})?;

		match response.status().as_u16() {
			200 => {}
			404 => {
				anyhow::bail!(
					"leptoskit {} has no DevTools interface at {url}. Each release publishes the \
					 interface for its own version only, and leptoskit never uses another \
					 version's.",
					self.version
				)
			}
			status => anyhow::bail!("{url} answered {status}"),
		}

		let mut bytes = Vec::new();
		response
			.body_mut()
			.with_config()
			.limit(MAX_ARCHIVE + 1)
			.reader()
			.take(MAX_ARCHIVE + 1)
			.read_to_end(&mut bytes)
			.with_context(|| format!("could not download {url}"))?;
		anyhow::ensure!(
			bytes.len() as u64 <= MAX_ARCHIVE,
			"{url} is larger than {MAX_ARCHIVE} bytes"
		);
		Ok(bytes)
	}
}

/// Unpacks a checked archive next to `dir`, then moves it into place.
fn unpack_into(archive: &[u8], dir: &Path) -> Result<()> {
	let parent = dir
		.parent()
		.context("the interface's directory has no parent")?;
	fs::create_dir_all(parent).with_context(|| format!("could not create {}", parent.display()))?;
	let staging = tempfile::Builder::new()
		.prefix(".devtools-")
		.tempdir_in(parent)
		.with_context(|| format!("could not create a directory in {}", parent.display()))?;
	unpack(archive, staging.path())?;
	anyhow::ensure!(
		ui::has_build(staging.path()),
		"the downloaded archive has no index.html, so it is not a build of the DevTools interface"
	);

	// Move the old build aside rather than deleting it first, so the hub
	// never sees a half-written directory.
	let old = parent.join(format!(
		".devtools-old-{}",
		staging
			.path()
			.file_name()
			.and_then(|name| name.to_str())
			.unwrap_or_default()
	));
	let had_old = dir.exists();

	if had_old {
		fs::rename(dir, &old).with_context(|| format!("could not replace {}", dir.display()))?;
	}

	let staged = staging.keep();

	if let Err(error) = fs::rename(&staged, dir) {
		let _ = fs::remove_dir_all(&staged);

		if had_old {
			let _ = fs::rename(&old, dir);
		}

		// Another process may have finished the same download first.
		if ui::has_build(dir) {
			return Ok(());
		}

		return Err(error)
			.with_context(|| format!("could not move the interface into {}", dir.display()));
	}

	if had_old {
		let _ = fs::remove_dir_all(&old);
	}

	Ok(())
}

/// Unpacks only plain files and directories that stay inside `into`.
fn unpack(archive: &[u8], into: &Path) -> Result<()> {
	let mut archive = tar::Archive::new(GzDecoder::new(archive));
	let mut total = 0_u64;

	for entry in archive
		.entries()
		.context("the downloaded archive is not a tar.gz")?
	{
		let mut entry = entry.context("the downloaded archive is damaged")?;
		let path = entry
			.path()
			.context("the archive names a file badly")?
			.into_owned();
		let relative = inside(&path)
			.with_context(|| format!("the archive's entry {} leaves its folder", path.display()))?;
		let kind = entry.header().entry_type();

		if relative.as_os_str().is_empty() {
			continue;
		}

		if kind.is_dir() {
			fs::create_dir_all(into.join(&relative))?;
			continue;
		}

		anyhow::ensure!(
			kind.is_file(),
			"the archive's entry {} is not a plain file",
			path.display()
		);
		total = total.saturating_add(entry.size());
		anyhow::ensure!(
			total <= MAX_UNPACKED,
			"the archive unpacks to more than {MAX_UNPACKED} bytes"
		);
		let target = into.join(&relative);

		if let Some(parent) = target.parent() {
			fs::create_dir_all(parent)?;
		}

		let mut file = fs::File::create(&target)
			.with_context(|| format!("could not write {}", target.display()))?;
		std::io::copy(&mut entry, &mut file)?;
	}

	Ok(())
}

/// The entry's path without `.`, when it stays inside the archive's folder.
fn inside(path: &Path) -> Option<PathBuf> {
	let mut clean = PathBuf::new();

	for component in path.components() {
		match component {
			Component::Normal(part) => clean.push(part),
			Component::CurDir => {}
			_ => return None,
		}
	}

	Some(clean)
}

/// Runs `leptoskit devtools fetch`.
///
/// # Errors
///
/// When this CLI knows no archive hash, or the download fails.
pub fn run(force: bool) -> Result<()> {
	let version = env!("CARGO_PKG_VERSION");
	let Some(release) = Release::current() else {
		anyhow::bail!(
			"this leptoskit {version} was built from source, so it doesn't know which DevTools \
			 interface belongs to it and won't download one. In a checkout of leptoskit, build \
			 the interface with `devenv shell devtools:build` and set {} to its output; a \
			 released leptoskit (from GitHub, `cargo binstall`, or `cargo install`) downloads it.",
			ui::UI_ENV
		);
	};
	let dir = ui::version_dir(version).context(
		"neither XDG_CACHE_HOME nor HOME is set, so there is nowhere to keep the interface",
	)?;

	if ui::has_build(&dir) && !force {
		crate::ui::success(format_args!(
			"the DevTools interface for leptoskit {version} is already in {}; pass --force to \
			 download it again",
			dir.display()
		));

		return Ok(());
	}

	println!("downloading {}", release.url());
	release.install(&dir)?;
	crate::ui::success(format_args!(
		"the DevTools interface for leptoskit {version} is in {}",
		dir.display()
	));
	Ok(())
}

/// Downloads the interface in the background the first time `leptoskit dev`
/// runs without it. A failure leaves the hub running with its plain page,
/// which explains how to fetch the interface.
pub fn on_first_use(sources: &ui::Sources) {
	if sources.local.is_some() {
		return;
	}

	let (Some(release), Some(dir)) = (Release::current(), sources.cache.clone()) else {
		return;
	};

	if ui::has_build(&dir) {
		return;
	}

	std::thread::spawn(move || {
		println!(
			"[devtools] downloading the interface for leptoskit {}",
			release.version
		);
		match release.install(&dir) {
			Ok(()) => println!("[devtools] the interface is ready; reload the hub page"),
			Err(error) => {
				eprintln!(
					"[devtools] could not download the interface: {error:#}\n[devtools] the hub \
					 still runs; `leptoskit devtools fetch` tries again"
				);
			}
		}
	});
}

#[cfg(test)]
mod tests {
	use std::io::Write;
	use std::net::TcpListener;
	use std::sync::Arc;
	use std::sync::Mutex;

	use flate2::Compression;
	use flate2::write::GzEncoder;

	use super::*;

	/// A tar.gz holding `files`.
	fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
		let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));

		for (path, contents) in files {
			let mut header = tar::Header::new_gnu();
			header.set_size(contents.len() as u64);
			header.set_mode(0o644);
			header.set_entry_type(tar::EntryType::Regular);
			header.set_cksum();
			builder.append_data(&mut header, path, *contents).unwrap();
		}

		builder.into_inner().unwrap().finish().unwrap()
	}

	fn interface(version: &str) -> Vec<u8> {
		archive(&[
			("./index.html", format!("<html>{version}</html>").as_bytes()),
			("./pkg/leptoskit_devtools_ui.js", b"export {}"),
			("./pkg/leptoskit_devtools_ui_bg.wasm", b"\0asm"),
		])
	}

	fn sha(bytes: &[u8]) -> String {
		hex::encode(Sha256::digest(bytes))
	}

	/// A release server that answers `files` by path and records each
	/// request path.
	fn server(files: Vec<(String, Vec<u8>)>) -> (String, Arc<Mutex<Vec<String>>>) {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let base = format!("http://{}", listener.local_addr().unwrap());
		let seen = Arc::new(Mutex::new(Vec::new()));
		let record = Arc::clone(&seen);
		std::thread::spawn(move || {
			for stream in listener.incoming() {
				let Ok(mut stream) = stream else {
					continue;
				};
				let mut buffer = [0_u8; 4096];
				let read = stream.read(&mut buffer).unwrap_or(0);
				let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
				let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
				record.lock().unwrap().push(path.clone());
				let found = files.iter().find(|(file, _)| *file == path);
				let (status, body) = match found {
					Some((_, body)) => ("200 OK", body.clone()),
					None => ("404 Not Found", b"Not Found".to_vec()),
				};

				let head = format!(
					"HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
					body.len()
				);
				let _ = stream.write_all(head.as_bytes());
				let _ = stream.write_all(&body);
			}
		});
		(base, seen)
	}

	fn release(base: &str, version: &str, sha256: &str) -> Release {
		Release {
			base: format!("{base}/releases/download"),
			version: version.to_owned(),
			sha256: sha256.to_owned(),
		}
	}

	#[test]
	fn the_matching_archive_is_installed() {
		let bytes = interface("0.4.0");
		let (base, seen) = server(vec![(
			"/releases/download/v0.4.0/leptoskit-devtools-0.4.0.tar.gz".into(),
			bytes.clone(),
		)]);
		let cache = tempfile::tempdir().unwrap();
		let dir = cache.path().join("devtools/0.4.0");
		release(&base, "0.4.0", &sha(&bytes)).install(&dir).unwrap();
		assert_eq!(
			fs::read_to_string(dir.join("index.html")).unwrap(),
			"<html>0.4.0</html>"
		);
		assert!(dir.join("pkg/leptoskit_devtools_ui_bg.wasm").is_file());
		assert_eq!(
			*seen.lock().unwrap(),
			["/releases/download/v0.4.0/leptoskit-devtools-0.4.0.tar.gz"]
		);

		// Only the build is left behind.
		assert_eq!(fs::read_dir(dir.parent().unwrap()).unwrap().count(), 1);

		// Installing again replaces the build in place.
		let newer = interface("0.4.0 again");
		let (base, _) = server(vec![(
			"/releases/download/v0.4.0/leptoskit-devtools-0.4.0.tar.gz".into(),
			newer.clone(),
		)]);
		release(&base, "0.4.0", &sha(&newer)).install(&dir).unwrap();
		assert_eq!(
			fs::read_to_string(dir.join("index.html")).unwrap(),
			"<html>0.4.0 again</html>"
		);
		assert_eq!(fs::read_dir(dir.parent().unwrap()).unwrap().count(), 1);
	}

	#[test]
	fn a_wrong_hash_is_refused_and_nothing_is_installed() {
		let bytes = interface("0.4.0");
		let (base, seen) = server(vec![(
			"/releases/download/v0.4.0/leptoskit-devtools-0.4.0.tar.gz".into(),
			bytes,
		)]);
		let cache = tempfile::tempdir().unwrap();
		let dir = cache.path().join("0.4.0");
		let error = release(&base, "0.4.0", &"0".repeat(64))
			.install(&dir)
			.unwrap_err()
			.to_string();
		assert!(error.contains("SHA-256"), "{error}");
		assert!(error.contains(&"0".repeat(64)), "{error}");
		assert!(!dir.exists());

		// The mismatch is reported, not retried.
		assert_eq!(seen.lock().unwrap().len(), 1);
	}

	#[test]
	fn a_missing_release_is_an_error_naming_the_version() {
		let (base, _) = server(Vec::new());
		let cache = tempfile::tempdir().unwrap();
		let dir = cache.path().join("0.4.0");
		let error = release(&base, "0.4.0", &"0".repeat(64))
			.install(&dir)
			.unwrap_err()
			.to_string();
		assert!(
			error.contains("leptoskit 0.4.0 has no DevTools interface"),
			"{error}"
		);
		assert!(!dir.exists());
	}

	#[test]
	fn another_versions_archive_is_refused() {
		// The server has only 0.3.0's archive, and serves it under 0.4.0's
		// name as well: the hash 0.4.0 expects still refuses it.
		let older = interface("0.3.0");
		let expected = sha(&interface("0.4.0"));
		let (base, seen) = server(vec![
			(
				"/releases/download/v0.3.0/leptoskit-devtools-0.3.0.tar.gz".into(),
				older.clone(),
			),
			(
				"/releases/download/v0.4.0/leptoskit-devtools-0.4.0.tar.gz".into(),
				older,
			),
		]);
		let cache = tempfile::tempdir().unwrap();
		let dir = cache.path().join("0.4.0");
		let error = release(&base, "0.4.0", &expected)
			.install(&dir)
			.unwrap_err()
			.to_string();
		assert!(error.contains("SHA-256"), "{error}");
		assert!(!dir.exists());

		// Only this version's address was asked for: no fallback.
		assert_eq!(
			*seen.lock().unwrap(),
			["/releases/download/v0.4.0/leptoskit-devtools-0.4.0.tar.gz"]
		);
	}

	#[test]
	fn an_archive_without_the_interface_is_refused() {
		let bytes = archive(&[("readme.txt", b"hello")]);
		let (base, _) = server(vec![(
			"/releases/download/v0.4.0/leptoskit-devtools-0.4.0.tar.gz".into(),
			bytes.clone(),
		)]);
		let cache = tempfile::tempdir().unwrap();
		let dir = cache.path().join("0.4.0");
		let error = release(&base, "0.4.0", &sha(&bytes))
			.install(&dir)
			.unwrap_err()
			.to_string();
		assert!(error.contains("no index.html"), "{error}");
		assert!(!dir.exists());
		assert_eq!(fs::read_dir(cache.path()).unwrap().count(), 0);
	}

	#[test]
	fn entries_that_leave_the_folder_are_refused() {
		assert_eq!(
			inside(Path::new("./pkg/a.js")),
			Some(PathBuf::from("pkg/a.js"))
		);
		assert_eq!(inside(Path::new("../a.js")), None);
		assert_eq!(inside(Path::new("/etc/passwd")), None);
		assert_eq!(inside(Path::new("pkg/../../a.js")), None);
	}

	#[test]
	fn release_addresses_name_the_exact_version() {
		let release = release("https://example.test", "1.2.3", "ab");
		assert_eq!(
			release.url(),
			"https://example.test/releases/download/v1.2.3/leptoskit-devtools-1.2.3.tar.gz"
		);
	}
}
