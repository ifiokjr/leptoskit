//! Code push tests: containers, deltas, the store, and the update client.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_codepush::CodePushClient;
use leptoskit_codepush::CodePushError;
use leptoskit_codepush::ReleaseStore;
use leptoskit_codepush::Transport;
use leptoskit_codepush::UpdatePlan;
use leptoskit_codepush::apply_delta;
use leptoskit_codepush::bundle_directory;
use leptoskit_codepush::create_delta;
use leptoskit_codepush::hash_bytes;
use leptoskit_codepush::read_container;
use semver::Version;

struct Scratch {
	path: PathBuf,
}

impl Scratch {
	fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_codepush_test_{tag}_{}_{}",
			std::process::id(),
			unique
		));
		std::fs::create_dir_all(&path).expect("scratch");
		Self { path }
	}

	fn write(&self, path: &str, content: &[u8]) -> PathBuf {
		let target = self.path.join(path);
		if let Some(parent) = target.parent() {
			std::fs::create_dir_all(parent).expect("parent");
		}
		std::fs::write(&target, content).expect("write");
		target
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.path);
	}
}

#[test]
fn container_round_trips_with_hash_verification() {
	let scratch = Scratch::new("roundtrip");
	scratch.write("app.wasm", b"\0asm pseudo module bytes");
	scratch.write("assets/logo.png", b"not really a png");

	let container_path = scratch.path.join("bundle.lkb");
	let manifest = bundle_directory(&scratch.path, &container_path).expect("bundle");
	// The .lkb output is written after the walk, so it is not in the bundle.
	assert_eq!(manifest.files.len(), 2);

	let bytes = std::fs::read(&container_path).expect("read container");
	let parsed = read_container(&bytes).expect("parse");
	assert_eq!(parsed.manifest.bundle_hash(), manifest.bundle_hash());
	assert!(
		parsed
			.files
			.iter()
			.any(|(path, content)| path == "app.wasm" && content == b"\0asm pseudo module bytes")
	);
}

#[test]
fn corrupt_container_fails_the_hash_check() {
	let scratch = Scratch::new("corrupt");
	scratch.write("app.wasm", b"\0asm pseudo module bytes");
	let container_path = scratch.path.join("bundle.lkb");
	bundle_directory(&scratch.path, &container_path).expect("bundle");

	let mut bytes = std::fs::read(&container_path).expect("read");
	*bytes.last_mut().expect("nonempty") ^= 0xff;
	assert!(
		read_container(&bytes).is_err(),
		"corruption must be detected"
	);
}

#[test]
fn delta_round_trips_and_shrinks_shared_content() {
	let base = vec![7u8; 64 * 1024];
	let mut target = base.clone();
	target.extend_from_slice(b"tail addition");
	target[100] = 42;

	let delta = create_delta(&base, &target);
	let applied = apply_delta(&base, &delta).expect("apply");
	assert_eq!(applied, target);

	// The delta instructions are far smaller than the target.
	let serialized = serde_json::to_vec(&delta).expect("serialize delta");
	assert!(
		serialized.len() < target.len() / 2,
		"delta {} should be well under half of {}",
		serialized.len(),
		target.len()
	);
}

#[test]
fn delta_rejects_the_wrong_base() {
	let base = vec![1u8; 4096];
	let other = vec![2u8; 4096];
	let delta = create_delta(&base, &base);
	let error = apply_delta(&other, &delta).unwrap_err();
	assert!(error.to_string().contains("mismatch"), "got {error:?}");
}

#[test]
fn store_versions_and_serves_deltas() {
	let scratch = Scratch::new("store");
	let store_root = scratch.path.join("releases");
	let store = ReleaseStore::open(&store_root).expect("open store");

	let v1_dir = scratch.path.join("v1");
	std::fs::create_dir_all(&v1_dir).expect("mkdir");
	std::fs::write(v1_dir.join("app.wasm"), vec![9u8; 32 * 1024]).expect("write");
	store
		.ingest_directory(
			"app",
			"production",
			&Version::new(1, 0, 0),
			1,
			&v1_dir,
			false,
		)
		.expect("ingest v1");

	let v2_dir = scratch.path.join("v2");
	std::fs::create_dir_all(&v2_dir).expect("mkdir");
	let mut v2_bytes = vec![9u8; 32 * 1024];
	v2_bytes.extend_from_slice(b"new section");
	std::fs::write(v2_dir.join("app.wasm"), &v2_bytes).expect("write");
	let v2 = store
		.ingest_directory(
			"app",
			"production",
			&Version::new(1, 1, 0),
			2,
			&v2_dir,
			false,
		)
		.expect("ingest v2");

	assert!(
		v2.delta_from_previous.is_some(),
		"second release gets a delta"
	);

	// The client on v1 sees the v2 offer with a delta URL.
	let plan = store
		.plan_update("app", "production", &Version::new(1, 0, 0), 1)
		.expect("plan");
	let UpdatePlan::Available(offer) = plan else {
		panic!("expected an update offer");
	};
	assert_eq!(offer.manifest.version, Version::new(1, 1, 0));
	assert!(offer.delta_url.is_some());

	// The client on v2 is up to date.
	let plan = store
		.plan_update("app", "production", &Version::new(1, 1, 0), 2)
		.expect("plan");
	assert_eq!(plan, UpdatePlan::UpToDate);
}

/// The registry base the in-memory transport answers for.
const REGISTRY: &str = "mem://registry";

/// A transport over a fixed map of URL to body: any URL the client builds
/// that the test did not register fails, so the map pins the exact wire URLs.
struct MemoryTransport {
	responses: HashMap<String, Vec<u8>>,
}

impl Transport for MemoryTransport {
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
		self.responses
			.get(url)
			.cloned()
			.ok_or_else(|| CodePushError::Transport(format!("404 {url}")))
	}
}

/// Writes a bundle source directory with enough shared bulk that a delta
/// between two of them does real work.
fn write_bundle(scratch: &Scratch, name: &str, marker: &str) -> PathBuf {
	let mut wasm = vec![5u8; 48 * 1024];
	wasm.extend_from_slice(marker.as_bytes());
	scratch.write(&format!("{name}/app.wasm"), &wasm);
	scratch.write(&format!("{name}/assets/extra.txt"), marker.as_bytes());
	scratch.path.join(name)
}

/// A store with `0.9.0+1` then `1.0.0+2` on the `staging` channel, and the
/// `0.9.0+1` container a client on that release runs.
fn staging_history(scratch: &Scratch) -> (ReleaseStore, Vec<u8>) {
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("open store");
	let v1 = store
		.ingest_directory(
			"app",
			"staging",
			&Version::new(0, 9, 0),
			1,
			&write_bundle(scratch, "v1", "v1 marker"),
			false,
		)
		.expect("ingest v1");
	store
		.ingest_directory(
			"app",
			"staging",
			&Version::new(1, 0, 0),
			2,
			&write_bundle(scratch, "v2", "v2 marker with new trailing content"),
			false,
		)
		.expect("ingest v2");
	(store, v1.container)
}

/// Serves what the routes would for a client on `0.9.0+1`: the check answer,
/// the full download, and — when given — the delta body.
fn staging_responses(
	store: &ReleaseStore,
	delta_body: Option<Vec<u8>>,
) -> HashMap<String, Vec<u8>> {
	let plan = store
		.plan_update("app", "staging", &Version::new(0, 9, 0), 1)
		.expect("plan");
	let UpdatePlan::Available(offer) = &plan else {
		panic!("expected an update offer");
	};
	let latest = store
		.latest("app", "staging")
		.expect("latest")
		.expect("v2 exists");
	let mut responses = HashMap::new();
	responses.insert(
		format!("{REGISTRY}/app/staging/check?version=0.9.0&build=1"),
		serde_json::to_vec(&plan).expect("plan json"),
	);
	responses.insert(
		format!("{REGISTRY}{}", offer.download_url),
		latest.container,
	);
	if let Some(body) = delta_body {
		let delta_url = offer.delta_url.as_ref().expect("v2 offers a delta to v1");
		responses.insert(format!("{REGISTRY}{delta_url}"), body);
	}
	responses
}

fn staging_client(
	responses: HashMap<String, Vec<u8>>,
	v1_container: Vec<u8>,
) -> CodePushClient<MemoryTransport> {
	CodePushClient::new(
		MemoryTransport { responses },
		REGISTRY,
		"app",
		"staging",
		Version::new(0, 9, 0),
		1,
		v1_container,
	)
}

fn assert_staged_v2(staging: &Path) {
	let wasm = std::fs::read(staging.join("app.wasm")).expect("staged wasm");
	assert!(wasm.ends_with(b"v2 marker with new trailing content"));
	let extra = std::fs::read(staging.join("assets").join("extra.txt")).expect("staged extra");
	assert_eq!(extra, b"v2 marker with new trailing content");
}

#[test]
fn client_syncs_via_delta_and_stages_files() {
	let scratch = Scratch::new("sync");
	let (store, v1_container) = staging_history(&scratch);
	let delta = store
		.latest("app", "staging")
		.expect("latest")
		.and_then(|record| record.delta_from_previous)
		.expect("v2 has a delta");
	let responses = staging_responses(&store, Some(serde_json::to_vec(&delta).expect("delta")));
	let mut client = staging_client(responses, v1_container);

	let staging = scratch.path.join("client_staged");
	let report = client.sync(&staging).expect("sync");
	assert!(report.updated);
	assert!(
		report.used_delta,
		"a client on v1 should patch, not download"
	);
	assert_eq!(report.version, Some(Version::new(1, 0, 0)));
	assert_eq!(client.current_version(), &Version::new(1, 0, 0));
	assert_staged_v2(&staging);
}

#[test]
fn client_falls_back_to_the_full_download_when_the_delta_fails() {
	for delta_body in [None, Some(b"not a delta".to_vec())] {
		let scratch = Scratch::new("fallback");
		let (store, v1_container) = staging_history(&scratch);
		let responses = staging_responses(&store, delta_body.clone());
		let mut client = staging_client(responses, v1_container);

		let staging = scratch.path.join("client_staged");
		let report = client
			.sync(&staging)
			.unwrap_or_else(|error| panic!("sync with delta {delta_body:?}: {error}"));
		assert!(report.updated);
		assert!(!report.used_delta, "the broken delta must not be used");
		assert_eq!(report.version, Some(Version::new(1, 0, 0)));
		assert_staged_v2(&staging);
	}
}

#[test]
fn client_percent_encodes_build_metadata_in_check_urls() {
	let mut responses = HashMap::new();
	responses.insert(
		format!("{REGISTRY}/app/production/check?version=1.0.0%2Babc&build=1"),
		serde_json::to_vec(&UpdatePlan::UpToDate).expect("plan json"),
	);
	let mut client = CodePushClient::new(
		MemoryTransport { responses },
		REGISTRY,
		"app",
		"production",
		Version::parse("1.0.0+abc").expect("version"),
		1,
		Vec::new(),
	);
	assert_eq!(client.check().expect("check"), UpdatePlan::UpToDate);
}

#[test]
fn offers_percent_encode_versions_in_download_urls() {
	let scratch = Scratch::new("encode");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("open store");
	store
		.ingest_directory(
			"app",
			"production",
			&Version::parse("1.0.0+abc").expect("version"),
			1,
			&write_bundle(&scratch, "v1", "v1"),
			false,
		)
		.expect("ingest");
	let plan = store
		.plan_update("app", "production", &Version::new(0, 1, 0), 1)
		.expect("plan");
	let UpdatePlan::Available(offer) = plan else {
		panic!("expected an update offer");
	};
	assert_eq!(offer.download_url, "/app/production/download/1.0.0%2Babc/1");
}

#[test]
fn plan_offers_a_delta_only_to_the_release_it_was_computed_against() {
	// Ingesting out of version order makes the delta's base something other
	// than the second-newest release: 3.0.0 is patched from 1.0.0, while
	// 2.0.0 arrives later and sorts between them.
	let scratch = Scratch::new("delta_base");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("open store");
	let ingest = |version: u64, marker: &str| {
		store
			.ingest_directory(
				"app",
				"production",
				&Version::new(version, 0, 0),
				version,
				&write_bundle(&scratch, marker, marker),
				false,
			)
			.expect("ingest")
	};
	let v1 = ingest(1, "one");
	ingest(3, "three");
	ingest(2, "two");

	let plan = store
		.plan_update("app", "production", &Version::new(2, 0, 0), 2)
		.expect("plan");
	let UpdatePlan::Available(offer) = plan else {
		panic!("2.0.0 should be offered 3.0.0");
	};
	assert_eq!(
		offer.delta_url, None,
		"3.0.0's delta does not apply to 2.0.0"
	);

	let plan = store
		.plan_update("app", "production", &Version::new(1, 0, 0), 1)
		.expect("plan");
	let UpdatePlan::Available(offer) = plan else {
		panic!("1.0.0 should be offered 3.0.0");
	};
	assert_eq!(
		offer.delta_url.as_deref(),
		Some("/app/production/delta/1.0.0/1")
	);
	assert_eq!(offer.delta_base_hash, Some(hash_bytes(&v1.container)));
}

#[test]
fn reingesting_an_existing_release_fails_without_overwriting() {
	let scratch = Scratch::new("reingest");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("open store");
	let version = Version::new(1, 0, 0);
	let first = store
		.ingest_directory(
			"app",
			"production",
			&version,
			1,
			&write_bundle(&scratch, "first", "first"),
			false,
		)
		.expect("first ingest");
	let error = store
		.ingest_directory(
			"app",
			"production",
			&version,
			1,
			&write_bundle(&scratch, "second", "second"),
			false,
		)
		.expect_err("the release already exists");
	assert!(
		matches!(error, CodePushError::ReleaseExists(_)),
		"got {error:?}"
	);
	let stored = store
		.find("app", "production", &version, 1)
		.expect("find")
		.expect("release present");
	assert_eq!(stored.container, first.container);
}

#[test]
fn list_skips_interrupted_ingests() {
	let scratch = Scratch::new("interrupted");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("open store");
	store
		.ingest_directory(
			"app",
			"production",
			&Version::new(1, 0, 0),
			1,
			&write_bundle(&scratch, "v1", "v1"),
			false,
		)
		.expect("ingest v1");
	// An ingest that died before its container was renamed into place.
	scratch.write(
		"releases/app/production/2.0.0+2/.release.lkb.tmp",
		b"partial",
	);

	let records = store.list("app", "production").expect("list");
	assert_eq!(records.len(), 1);
	assert_eq!(records[0].manifest.version, Version::new(1, 0, 0));

	// Re-running the interrupted ingest completes it.
	store
		.ingest_directory(
			"app",
			"production",
			&Version::new(2, 0, 0),
			2,
			&write_bundle(&scratch, "v2", "v2"),
			false,
		)
		.expect("ingest v2");
	let latest = store
		.latest("app", "production")
		.expect("latest")
		.expect("v2 present");
	assert_eq!(latest.manifest.version, Version::new(2, 0, 0));
}

#[test]
fn concurrent_ingests_of_one_release_publish_exactly_once() {
	let scratch = Scratch::new("concurrent");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("open store");
	let bundles = [
		write_bundle(&scratch, "first", "first"),
		write_bundle(&scratch, "second", "second"),
	];

	let results: Vec<_> = std::thread::scope(|scope| {
		let handles: Vec<_> = bundles
			.iter()
			.map(|bundle| {
				let store = store.clone();
				scope.spawn(move || {
					store.ingest_directory(
						"app",
						"production",
						&Version::new(1, 0, 0),
						1,
						bundle,
						false,
					)
				})
			})
			.collect();
		handles
			.into_iter()
			.map(|handle| handle.join().expect("ingest thread"))
			.collect()
	});

	let published: Vec<_> = results
		.iter()
		.filter_map(|result| result.as_ref().ok())
		.collect();
	assert_eq!(published.len(), 1, "exactly one ingest wins: {results:?}");
	assert!(
		results
			.iter()
			.any(|result| matches!(result, Err(CodePushError::ReleaseExists(_)))),
		"the other ingest is told the release exists: {results:?}"
	);
	let records = store.list("app", "production").expect("list");
	assert_eq!(records.len(), 1);
	assert_eq!(records[0].container, published[0].container);
}

#[test]
fn list_skips_releases_in_a_newer_container_format() {
	let scratch = Scratch::new("newer-format");
	let releases = scratch.path.join("releases");
	let store = ReleaseStore::open(&releases).expect("open store");
	store
		.ingest_directory(
			"app",
			"production",
			&Version::new(1, 0, 0),
			1,
			&write_bundle(&scratch, "v1", "v1"),
			false,
		)
		.expect("ingest v1");
	store
		.ingest_directory(
			"app",
			"production",
			&Version::new(2, 0, 0),
			1,
			&write_bundle(&scratch, "v2", "v2"),
			false,
		)
		.expect("ingest v2");

	// Rewrite v2 as if a newer leptoskit had published it.
	let v2 = releases.join("app/production/2.0.0+1/release.lkb");
	let mut bytes = std::fs::read(&v2).expect("read v2");
	let field = b"\"format_version\":1";
	let at = bytes
		.windows(field.len())
		.position(|window| window == field)
		.expect("compact manifest names its format version");
	bytes[at + field.len() - 1] = b'9';
	std::fs::write(&v2, bytes).expect("write v2");

	let records = store.list("app", "production").expect("list survives");
	assert_eq!(records.len(), 1);
	assert_eq!(records[0].manifest.version, Version::new(1, 0, 0));
}
