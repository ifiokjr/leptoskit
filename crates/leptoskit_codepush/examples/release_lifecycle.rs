//! A release's whole life, in one process and without a network: sign and
//! store releases, serve them as static files, stage and confirm one on a
//! device, ship the next as a delta, refuse a release that crashes, and
//! mirror the channel to a bucket and back.
//!
//! ```sh
//! cargo run -p leptoskit_codepush --example release_lifecycle
//! ```
//!
//! Every step prints what happened and checks it, so the run doubles as a
//! test: `cargo test -p leptoskit_codepush --example release_lifecycle`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use leptoskit_codepush::Bucket;
use leptoskit_codepush::CodePushClient;
use leptoskit_codepush::CodePushError;
use leptoskit_codepush::FileTransport;
use leptoskit_codepush::ObjectKey;
use leptoskit_codepush::PullReport;
use leptoskit_codepush::ReleaseSigningKey;
use leptoskit_codepush::ReleaseStore;
use leptoskit_codepush::StaticObject;
use leptoskit_codepush::Transport;
use leptoskit_codepush::TrustedKeys;
use leptoskit_codepush::bundle_directory_as;
use leptoskit_codepush::installed::InstalledBundles;
use leptoskit_codepush::installed::RuntimeContract;
use leptoskit_codepush::installed::UpdateConfiguration;
use leptoskit_codepush::installed::UpdateStatus;
use leptoskit_codepush::publish_channel;
use leptoskit_codepush::pull_channel;
use semver::Version;

const APP: &str = "lighthouse";
const CHANNEL: &str = "production";

type Outcome = Result<(), Box<dyn Error>>;

fn main() -> Outcome {
	run(&mut std::io::stdout())
}

/// Runs every step, narrating to `out`.
fn run(out: &mut impl Write) -> Outcome {
	let work = tempfile::tempdir()?;
	let work = work.path();

	// The publisher signs every release with one key, from a fixed seed so
	// every run signs the same way. Apps pin its public half.
	let publisher = ReleaseSigningKey::from_seed(&[0x1b; 32]);
	let trusted = TrustedKeys::new([publisher.public_key()])?;

	// The registry is a directory of static files: any file server or bucket
	// can serve it, and a `file://` URL reads it directly.
	let store = ReleaseStore::open(work.join("registry"))?;
	let server = format!("file://{}", store.root().display());

	// 1. The store build: what the app ships with, build 0.
	let baseline = bundle(work, Version::new(1, 0, 0), 0, &module("store build", 0))?;
	let mut device = InstalledBundles::open(
		&work.join("device"),
		&baseline,
		configuration(&server, &publisher),
	)?;
	let launch = device.launch()?;
	device.confirm(&launch.metadata.receipt)?;
	writeln!(
		out,
		"1. launched the store build {}+{}",
		launch.metadata.version, launch.metadata.build
	)?;

	// 2. A release, staged on the device as a trial and confirmed when its
	//    first frame is healthy.
	release(
		&store,
		&publisher,
		work,
		&Version::new(1, 0, 1),
		1,
		&module("lantern", 0),
	)?;
	let staged = device.sync_with(&mut FileTransport)?;
	ensure(staged.status == UpdateStatus::Staged, "1.0.1 is staged")?;
	let trial = device.launch()?;
	ensure(trial.metadata.candidate, "1.0.1 launches as a trial")?;
	device.confirm(&trial.metadata.receipt)?;
	writeln!(
		out,
		"2. staged, launched, and confirmed {}+{} on the device",
		trial.metadata.version, trial.metadata.build
	)?;

	// 3. The next release changes the middle of the module. A desktop client on
	//    1.0.1 downloads only the delta from it.
	let previous = store.latest(APP, CHANNEL)?.ok_or("the store holds 1.0.1")?;
	release(
		&store,
		&publisher,
		work,
		&Version::new(1, 0, 2),
		2,
		&module("lantern", 1),
	)?;
	let fetched = Rc::new(RefCell::new(Fetched::default()));
	let mut desktop = CodePushClient::new(
		Counting::new(FileTransport, &fetched),
		server.clone(),
		APP,
		CHANNEL,
		Version::new(1, 0, 1),
		1,
		previous.container.clone(),
		trusted.clone(),
	);
	let synced = desktop.sync(&work.join("desktop"))?;
	ensure(synced.used_delta, "1.0.2 arrives as a delta")?;
	let downloaded = fetched.borrow().bytes;
	writeln!(
		out,
		"3. a desktop client moved to 1.0.2 fetching {} KiB instead of a {} KiB container",
		downloaded / 1024,
		previous.container.len() / 1024
	)?;
	ensure(
		downloaded * 4 < previous.container.len(),
		"the delta is a fraction of the container",
	)?;
	let next = device.sync_with(&mut FileTransport)?;
	ensure(next.status == UpdateStatus::Staged, "1.0.2 is staged")?;
	let trial = device.launch()?;
	device.confirm(&trial.metadata.receipt)?;

	// 4. A release that crashes: its trial is rejected, the device falls back
	//    to the release that worked, and never downloads the bad one again.
	release(
		&store,
		&publisher,
		work,
		&Version::new(1, 0, 3),
		3,
		&module("broken", 1),
	)?;
	device.sync_with(&mut FileTransport)?;
	let trial = device.launch()?;
	ensure(trial.metadata.build == 3, "1.0.3 launches as a trial")?;
	device.reject(&trial.metadata.receipt)?;
	let fallback = device.launch()?;
	ensure(
		fallback.metadata.build == 2,
		"the device falls back to 1.0.2",
	)?;
	device.confirm(&fallback.metadata.receipt)?;

	let checked = Rc::new(RefCell::new(Fetched::default()));
	let after = device.sync_with(&mut Counting::new(FileTransport, &checked))?;
	ensure(after.status == UpdateStatus::Current, "1.0.3 is refused")?;
	let checked = checked.borrow();
	ensure(
		checked.urls.iter().all(|url| url.ends_with("/index.json")),
		"only the channel index is fetched",
	)?;
	writeln!(
		out,
		"4. rejected the crashing 1.0.3, fell back to {}+{}, and checked again with {} request",
		fallback.metadata.version,
		fallback.metadata.build,
		checked.urls.len()
	)?;

	// 5. Mirror the channel to a bucket, then pull it into a fresh store, as a
	//    CI runner does before it builds the next release.
	let mut bucket = MemoryBucket::default();
	let mirrored = publish_channel(&store, APP, CHANNEL, &mut bucket)?;
	let runner = ReleaseStore::open(work.join("runner"))?;
	let pulled = pull_channel(&runner, APP, CHANNEL, &mut bucket, &trusted)?;
	ensure(
		matches!(
			pulled,
			PullReport::Pulled {
				build: 3,
				imported: true,
				..
			}
		),
		"the runner holds the newest release",
	)?;
	writeln!(
		out,
		"5. published {} objects to a bucket and pulled {}+{} into a fresh store",
		mirrored.written, mirrored.version, mirrored.build
	)?;
	Ok(())
}

/// What the device's store build supports: one guest ABI, one shell, no
/// plugins. A release that needs more is never staged.
fn contract() -> RuntimeContract {
	RuntimeContract {
		abi: 1,
		shell: "lighthouse-shell-v1".to_owned(),
		plugins: Vec::new(),
	}
}

fn configuration(server: &str, publisher: &ReleaseSigningKey) -> UpdateConfiguration {
	UpdateConfiguration {
		enabled: true,
		app: APP.to_owned(),
		channel: CHANNEL.to_owned(),
		server: server.to_owned(),
		public_keys: vec![publisher.public_key().to_hex()],
		runtime: contract(),
		allow_http: false,
	}
}

/// A 1 MiB module that varies like compiled code, with `marker` at the
/// start and an edit `edits` 4 KiB lines into its middle.
fn module(marker: &str, edits: usize) -> Vec<u8> {
	const CODE: &[u8] = b"\0asm local.get i32.add call br_if";
	let mut state = 0x2545_f491_4f6c_dd1d_u64;
	let mut wasm = b"\0asm\x01\0\0\0".to_vec();
	wasm.extend_from_slice(marker.as_bytes());
	wasm.extend((0..1024 * 1024).map(|_| {
		state ^= state << 13;
		state ^= state >> 7;
		state ^= state << 17;
		CODE[(state % CODE.len() as u64) as usize]
	}));
	let middle = wasm.len() / 2;

	for edit in 0..edits {
		wasm[middle + edit * 4096..][..12].copy_from_slice(b"patched code");
	}

	wasm
}

/// Packs a module and the runtime contract into a container, as the CLI
/// does for a store build.
fn bundle(
	work: &Path,
	version: Version,
	build: u64,
	wasm: &[u8],
) -> Result<Vec<u8>, Box<dyn Error>> {
	let source = source_dir(work, &version, build, wasm)?;
	let output = work.join(format!("{version}+{build}.lkb"));
	bundle_directory_as(&source, &output, APP, CHANNEL, version, build)?;
	Ok(fs::read(output)?)
}

/// Signs a module into the store as a release.
fn release(
	store: &ReleaseStore,
	publisher: &ReleaseSigningKey,
	work: &Path,
	version: &Version,
	build: u64,
	wasm: &[u8],
) -> Outcome {
	let source = source_dir(work, version, build, wasm)?;
	store.ingest_directory(publisher, APP, CHANNEL, version, build, &source, false)?;
	Ok(())
}

fn source_dir(
	work: &Path,
	version: &Version,
	build: u64,
	wasm: &[u8],
) -> Result<PathBuf, Box<dyn Error>> {
	let source = work.join(format!("source-{version}+{build}"));
	fs::create_dir_all(&source)?;
	fs::write(source.join("app.wasm"), wasm)?;
	fs::write(
		source.join("runtime.json"),
		serde_json::to_vec(&contract())?,
	)?;
	Ok(source)
}

fn ensure(condition: bool, claim: &str) -> Outcome {
	if condition {
		Ok(())
	} else {
		Err(format!("expected: {claim}").into())
	}
}

/// What a [`Counting`] transport fetched.
#[derive(Default)]
struct Fetched {
	urls: Vec<String>,
	bytes: usize,
}

/// A transport that records what it fetched where the caller can read it,
/// since a client owns its transport.
struct Counting<T> {
	inner: T,
	fetched: Rc<RefCell<Fetched>>,
}

impl<T> Counting<T> {
	fn new(inner: T, fetched: &Rc<RefCell<Fetched>>) -> Self {
		Self {
			inner,
			fetched: Rc::clone(fetched),
		}
	}
}

impl<T: Transport> Transport for Counting<T> {
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
		let body = self.inner.fetch(url)?;
		let mut fetched = self.fetched.borrow_mut();
		fetched.urls.push(url.to_owned());
		fetched.bytes += body.len();
		Ok(body)
	}
}

/// A bucket in memory, standing in for S3 or any object store.
#[derive(Default)]
struct MemoryBucket {
	objects: BTreeMap<String, Vec<u8>>,
}

impl Bucket for MemoryBucket {
	fn get(&mut self, key: &ObjectKey) -> Result<Option<Vec<u8>>, CodePushError> {
		Ok(self.objects.get(key.as_str()).cloned())
	}

	fn put(&mut self, object: &StaticObject) -> Result<(), CodePushError> {
		self.objects
			.insert(object.key.as_str().to_owned(), object.bytes.clone());
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn the_lifecycle_runs_from_store_build_to_mirror() {
		let mut narration = Vec::new();
		super::run(&mut narration).expect("every step holds");
		let narration = String::from_utf8(narration).expect("utf-8");
		assert_eq!(narration.lines().count(), 5, "{narration}");
	}
}
