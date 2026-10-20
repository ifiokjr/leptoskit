//! Code push tests: containers, deltas, the store, the static layout, the
//! update client, and publishing to a bucket.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_codepush::Bucket;
use leptoskit_codepush::ChannelIndex;
use leptoskit_codepush::CodePushClient;
use leptoskit_codepush::CodePushError;
use leptoskit_codepush::FileTransport;
use leptoskit_codepush::ObjectKey;
use leptoskit_codepush::ObjectKind;
use leptoskit_codepush::ReleaseSigningKey;
use leptoskit_codepush::ReleaseStore;
use leptoskit_codepush::StaticObject;
use leptoskit_codepush::Transport;
use leptoskit_codepush::TrustedKeys;
use leptoskit_codepush::UpdatePlan;
use leptoskit_codepush::apply_delta;
use leptoskit_codepush::bundle_directory;
use leptoskit_codepush::create_delta;
use leptoskit_codepush::hash_bytes;
use leptoskit_codepush::publish_channel;
use leptoskit_codepush::read_container;
use semver::Version;

/// The publisher key every test release is signed with.
fn publisher() -> ReleaseSigningKey {
	ReleaseSigningKey::from_seed(&[0x11; 32])
}

/// The trust set of a client that installs the publisher's releases.
fn trusted() -> TrustedKeys {
	TrustedKeys::new([publisher().public_key()]).expect("trusted keys")
}

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
			&publisher(),
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
			&publisher(),
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

/// A transport over a fixed map of URL to body, answering anything else with
/// a 404, so the map pins the exact wire URLs.
struct MemoryTransport {
	responses: HashMap<String, Vec<u8>>,
}

impl Transport for MemoryTransport {
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
		self.responses
			.get(url)
			.cloned()
			.ok_or_else(|| CodePushError::NotFound(url.to_owned()))
	}
}

/// An in-memory object store: what a bucket holds, and every write in order.
#[derive(Default)]
struct MemoryBucket {
	objects: HashMap<String, StaticObject>,
	/// The keys written, in order.
	puts: Vec<String>,
	/// Fails every write once this many have succeeded, like a publish that
	/// loses its connection.
	fail_after: Option<usize>,
}

impl Bucket for MemoryBucket {
	fn get(&mut self, key: &ObjectKey) -> Result<Option<Vec<u8>>, CodePushError> {
		Ok(self
			.objects
			.get(key.as_str())
			.map(|object| object.bytes.clone()))
	}

	fn put(&mut self, object: &StaticObject) -> Result<(), CodePushError> {
		if self.fail_after == Some(self.puts.len()) {
			return Err(CodePushError::Transport("connection reset".to_owned()));
		}
		self.puts.push(object.key.as_str().to_owned());
		self.objects
			.insert(object.key.as_str().to_owned(), object.clone());
		Ok(())
	}
}

impl MemoryBucket {
	/// What a CDN in front of the bucket answers at [`REGISTRY`].
	fn responses(&self) -> HashMap<String, Vec<u8>> {
		self.objects
			.values()
			.map(|object| {
				(
					format!("{REGISTRY}/{}", object.key.url_path()),
					object.bytes.clone(),
				)
			})
			.collect()
	}
}

/// Publishes a channel of `store` to a fresh in-memory bucket.
fn published(store: &ReleaseStore, channel: &str) -> MemoryBucket {
	let mut bucket = MemoryBucket::default();
	publish_channel(store, "app", channel, &mut bucket).expect("publish");
	bucket
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
			&publisher(),
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
			&publisher(),
			"app",
			"staging",
			&Version::new(1, 0, 0),
			2,
			&write_bundle(scratch, "v2", "v2 marker with new trailing content"),
			false,
		)
		.expect("ingest v2");
	(store, v1.record.container)
}

/// The URL a client fetches a file of the layout from.
fn registry_url(key: &ObjectKey) -> String {
	format!("{REGISTRY}/{}", key.url_path())
}

/// The URL of the delta from `0.9.0+1` to `1.0.0+2` on `staging`.
fn staging_delta_url(store: &ReleaseStore) -> String {
	let v1 = store
		.find("app", "staging", &Version::new(0, 9, 0), 1)
		.expect("find")
		.expect("v1 exists");
	registry_url(
		&ObjectKey::delta(
			"app",
			"staging",
			&Version::new(1, 0, 0),
			2,
			&hash_bytes(&v1.container),
		)
		.expect("hash"),
	)
}

/// What a CDN serves once the staging channel is published to a bucket,
/// with the delta from `0.9.0+1` replaced by `delta_body`, or missing.
fn staging_responses(
	store: &ReleaseStore,
	delta_body: Option<Vec<u8>>,
) -> HashMap<String, Vec<u8>> {
	let mut responses = published(store, "staging").responses();
	let delta_url = staging_delta_url(store);
	assert!(
		responses.remove(&delta_url).is_some(),
		"the bucket holds the delta from v1"
	);
	if let Some(body) = delta_body {
		responses.insert(delta_url, body);
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
		trusted(),
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
		.delta(
			"app",
			"staging",
			&Version::new(1, 0, 0),
			2,
			&hash_bytes(&v1_container),
		)
		.expect("read delta")
		.expect("v2 has a delta from v1");
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
fn client_percent_encodes_build_metadata_in_static_urls() {
	let scratch = Scratch::new("static-encode");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("open store");
	store
		.ingest_directory(
			&publisher(),
			"app",
			"production",
			&Version::parse("1.0.0+abc").expect("version"),
			1,
			&write_bundle(&scratch, "v1", "v1"),
			false,
		)
		.expect("ingest");
	let mut responses = HashMap::new();
	responses.insert(
		format!("{REGISTRY}/app/production/index.json"),
		serde_json::to_vec(&store.index("app", "production").expect("index")).expect("json"),
	);
	let mut client = CodePushClient::new(
		MemoryTransport { responses },
		REGISTRY,
		"app",
		"production",
		Version::new(0, 1, 0),
		1,
		b"the running container".to_vec(),
		trusted(),
	);
	let UpdatePlan::Available(offer) = client.check().expect("check") else {
		panic!("expected an update offer");
	};
	// A literal `+` in a URL path reaches some object stores as a space.
	assert_eq!(
		offer.download_url,
		"/app/production/1.0.0%2Babc%2B1/release.lkb"
	);
	assert_eq!(
		offer.delta_url,
		Some(format!(
			"/app/production/1.0.0%2Babc%2B1/deltas/{}.jsondelta",
			hash_bytes(b"the running container")
		))
	);
}

#[test]
fn offers_percent_encode_versions_in_download_urls() {
	let scratch = Scratch::new("encode");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("open store");
	store
		.ingest_directory(
			&publisher(),
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
				&publisher(),
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
	assert_eq!(
		offer.delta_base_hash,
		Some(hash_bytes(&v1.record.container))
	);
}

#[test]
fn reingesting_an_existing_release_fails_without_overwriting() {
	let scratch = Scratch::new("reingest");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("open store");
	let version = Version::new(1, 0, 0);
	let first = store
		.ingest_directory(
			&publisher(),
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
			&publisher(),
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
	assert_eq!(stored.container, first.record.container);
}

#[test]
fn list_skips_interrupted_ingests() {
	let scratch = Scratch::new("interrupted");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("open store");
	store
		.ingest_directory(
			&publisher(),
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
			&publisher(),
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
						&publisher(),
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
	assert_eq!(records[0].container, published[0].record.container);
}

#[test]
fn list_skips_releases_in_a_newer_container_format() {
	let scratch = Scratch::new("newer-format");
	let releases = scratch.path.join("releases");
	let store = ReleaseStore::open(&releases).expect("open store");
	store
		.ingest_directory(
			&publisher(),
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
			&publisher(),
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

/// Runs a sync for a client on `running` against a published staging
/// channel whose `index.json` — naming the genuine, signed `1.0.0+2` — has
/// been changed by `tamper`, as a hostile bucket or CDN could.
///
/// A rejected sync must leave the staging directory untouched.
fn sync_offer(
	running: (Version, u64),
	tamper: impl FnOnce(&mut ChannelIndex),
) -> Result<leptoskit_codepush::SyncReport, CodePushError> {
	let scratch = Scratch::new("offer");
	let (store, v1_container) = staging_history(&scratch);
	let mut index = store
		.index("app", "staging")
		.expect("read index")
		.expect("the channel has an index");
	tamper(&mut index);

	let (version, build) = running;
	let mut responses = published(&store, "staging").responses();
	responses.insert(
		registry_url(&ObjectKey::index("app", "staging")),
		serde_json::to_vec(&index).expect("index json"),
	);
	let mut client = CodePushClient::new(
		MemoryTransport { responses },
		REGISTRY,
		"app",
		"staging",
		version,
		build,
		v1_container,
		trusted(),
	);
	let staging = scratch.path.join("client_staged");
	let result = client.sync(&staging);
	if result.is_err() {
		assert!(!staging.exists(), "a rejected release must stage nothing");
	}
	result
}

#[test]
fn a_genuine_signed_offer_installs() {
	let report = sync_offer((Version::new(0, 9, 0), 1), |offer| {
		trusted()
			.verify(&offer.manifest, offer.signature.as_ref())
			.expect("the store signed the release");
	})
	.expect("sync");
	assert!(report.updated);
	assert_eq!(report.version, Some(Version::new(1, 0, 0)));
}

#[test]
fn unsigned_offers_are_rejected() {
	let result = sync_offer((Version::new(0, 9, 0), 1), |offer| {
		offer.signature = None;
	});
	assert!(
		matches!(result, Err(CodePushError::Unsigned(_))),
		"got {result:?}"
	);
}

#[test]
fn offers_signed_by_an_untrusted_key_are_rejected() {
	let attacker = ReleaseSigningKey::from_seed(&[0x66; 32]);
	let result = sync_offer((Version::new(0, 9, 0), 1), |offer| {
		offer.signature = Some(attacker.sign(&offer.manifest));
	});
	assert!(
		matches!(&result, Err(CodePushError::UntrustedKey(key)) if *key == attacker.public_key().to_hex()),
		"got {result:?}"
	);
}

#[test]
fn signatures_forged_under_a_trusted_key_are_rejected() {
	for forge in [
		// Another key's signature, relabelled as the publisher's.
		|offer: &mut ChannelIndex| {
			let mut signature = ReleaseSigningKey::from_seed(&[0x66; 32]).sign(&offer.manifest);
			signature.public_key = publisher().public_key().to_hex();
			offer.signature = Some(signature);
		},
		// Garbage that is not even a signature.
		|offer: &mut ChannelIndex| {
			if let Some(signature) = &mut offer.signature {
				signature.signature = "not hex".to_owned();
			}
		},
	] {
		let result = sync_offer((Version::new(0, 9, 0), 1), forge);
		assert!(
			matches!(result, Err(CodePushError::BadSignature(_))),
			"got {result:?}"
		);
	}
}

#[test]
fn manifests_tampered_after_signing_are_rejected() {
	for tamper in [
		|offer: &mut ChannelIndex| offer.manifest.mandatory = !offer.manifest.mandatory,
		|offer: &mut ChannelIndex| offer.manifest.files[0].hash = hash_bytes(b"attacker wasm"),
		|offer: &mut ChannelIndex| offer.manifest.build += 1,
	] {
		let result = sync_offer((Version::new(0, 9, 0), 1), tamper);
		assert!(
			matches!(result, Err(CodePushError::BadSignature(_))),
			"got {result:?}"
		);
	}
}

#[test]
fn a_container_that_differs_from_the_signed_manifest_is_not_staged() {
	let scratch = Scratch::new("swapped");
	let (store, v1_container) = staging_history(&scratch);
	let mut responses = staging_responses(&store, None);
	// The registry offers the signed 1.0.0+2 but serves a different (also
	// genuine) container in its place.
	let download = registry_url(&ObjectKey::release(
		"app",
		"staging",
		&Version::new(1, 0, 0),
		2,
	));
	assert!(responses.contains_key(&download));
	responses.insert(download, v1_container.clone());
	let mut client = staging_client(responses, v1_container);

	let staging = scratch.path.join("client_staged");
	let result = client.sync(&staging);
	assert!(
		matches!(result, Err(CodePushError::HashMismatch { .. })),
		"got {result:?}"
	);
	assert!(
		!staging.exists(),
		"a mismatched container must stage nothing"
	);
}

#[test]
fn signed_releases_for_another_app_or_channel_are_rejected() {
	for retarget in [
		|offer: &mut ChannelIndex| offer.manifest.channel = "production".to_owned(),
		|offer: &mut ChannelIndex| offer.manifest.app = "other_app".to_owned(),
	] {
		// The publisher really did sign these, just not for this client.
		let result = sync_offer((Version::new(0, 9, 0), 1), |offer| {
			retarget(offer);
			offer.signature = Some(publisher().sign(&offer.manifest));
		});
		assert!(
			matches!(result, Err(CodePushError::WrongTarget { .. })),
			"got {result:?}"
		);
	}
}

#[test]
fn old_signed_releases_are_never_installed() {
	// The index names the genuine, signed 1.0.0+2: a replay to a client
	// already on it, and a downgrade for clients on anything newer — or just
	// a CDN still serving the index from before the client updated.
	for running in [
		(Version::new(1, 0, 0), 2),
		(Version::new(1, 0, 0), 3),
		(Version::new(1, 1, 0), 1),
		(Version::new(2, 0, 0), 0),
	] {
		let report = sync_offer(running.clone(), |_| {})
			.unwrap_or_else(|error| panic!("{running:?}: {error}"));
		assert!(!report.updated, "{running:?} must stay where it is");
	}
}

#[test]
fn offers_that_are_not_newer_are_rejected() {
	let scratch = Scratch::new("not-newer");
	let (store, _) = staging_history(&scratch);
	let UpdatePlan::Available(offer) = store
		.plan_update("app", "staging", &Version::new(0, 9, 0), 1)
		.expect("plan")
	else {
		panic!("expected an update offer");
	};
	let client = CodePushClient::new(
		MemoryTransport {
			responses: HashMap::new(),
		},
		REGISTRY,
		"app",
		"staging",
		Version::new(1, 0, 0),
		2,
		Vec::new(),
		trusted(),
	);
	assert!(matches!(
		client.verify_offer(&offer),
		Err(CodePushError::NotNewer { .. })
	));
}

#[test]
fn releases_without_a_stored_signature_are_offered_unsigned_and_rejected() {
	let scratch = Scratch::new("missing-signature");
	let (store, v1_container) = staging_history(&scratch);
	// A release published before signing existed, or mid-ingest.
	std::fs::remove_file(
		scratch
			.path
			.join("releases/app/staging/1.0.0+2/release.sig"),
	)
	.expect("remove signature");
	let latest = store
		.latest("app", "staging")
		.expect("latest")
		.expect("v2 still listed");
	assert_eq!(latest.signature, None);

	let UpdatePlan::Available(offer) = store
		.plan_update("app", "staging", &Version::new(0, 9, 0), 1)
		.expect("plan")
	else {
		panic!("expected an update offer");
	};
	assert_eq!(offer.signature, None);
	let client = staging_client(HashMap::new(), v1_container);
	assert!(
		matches!(client.verify_offer(&offer), Err(CodePushError::Unsigned(_))),
		"the dynamic offer is rejected"
	);
	let mut bucket = MemoryBucket::default();
	assert!(
		matches!(
			publish_channel(&store, "app", "staging", &mut bucket),
			Err(CodePushError::Unsigned(_))
		),
		"an unsigned release is never published"
	);
	assert!(bucket.puts.is_empty());
}

#[test]
fn the_store_directory_is_the_static_layout() {
	let scratch = Scratch::new("layout");
	let (store, v1_container) = staging_history(&scratch);
	let channel = scratch.path.join("releases/app/staging");
	for path in [
		"0.9.0+1/release.lkb",
		"0.9.0+1/release.sig",
		"1.0.0+2/release.lkb",
		"1.0.0+2/release.sig",
		&format!("1.0.0+2/deltas/{}.jsondelta", hash_bytes(&v1_container)),
	] {
		assert!(channel.join(path).is_file(), "{path} is missing");
	}
	assert!(
		!channel.join("0.9.0+1/deltas").exists(),
		"the first release has nothing to patch from"
	);

	let index: ChannelIndex =
		serde_json::from_slice(&std::fs::read(channel.join("index.json")).expect("read index"))
			.expect("parse index");
	assert_eq!(index.format_version, 1);
	assert_eq!(
		(index.manifest.version.clone(), index.manifest.build),
		(Version::new(1, 0, 0), 2)
	);
	trusted()
		.verify(&index.manifest, index.signature.as_ref())
		.expect("the index carries the publisher signature");
	assert_eq!(
		Some(index),
		store.index("app", "staging").expect("read index")
	);
}

#[test]
fn a_store_directory_serves_clients_as_plain_files() {
	let scratch = Scratch::new("file-registry");
	let (_, v1_container) = staging_history(&scratch);
	let mut client = CodePushClient::new(
		FileTransport,
		format!("file://{}", scratch.path.join("releases").display()),
		"app",
		"staging",
		Version::new(0, 9, 0),
		1,
		v1_container,
		trusted(),
	);
	let staging = scratch.path.join("client_staged");
	let report = client.sync(&staging).expect("sync");
	assert!(report.updated);
	assert!(report.used_delta, "the delta is found by the v1 hash");
	assert_staged_v2(&staging);
}

#[test]
fn a_client_on_a_release_without_a_delta_downloads_the_full_container() {
	let scratch = Scratch::new("delta-miss");
	let (store, _) = staging_history(&scratch);
	let responses = published(&store, "staging").responses();
	let foreign = b"a store build that was never released through code push".to_vec();
	let mut client = staging_client(responses, foreign);

	let staging = scratch.path.join("client_staged");
	let report = client.sync(&staging).expect("sync");
	assert!(report.updated);
	assert!(!report.used_delta, "no delta exists from that container");
	assert_staged_v2(&staging);
}

#[test]
fn a_channel_without_an_index_is_up_to_date() {
	let mut client = staging_client(HashMap::new(), Vec::new());
	assert_eq!(client.check().expect("check"), UpdatePlan::UpToDate);
}

#[test]
fn transport_failures_other_than_not_found_are_errors() {
	struct Offline;
	impl Transport for Offline {
		fn fetch(&mut self, _: &str) -> Result<Vec<u8>, CodePushError> {
			Err(CodePushError::Transport("offline".to_owned()))
		}
	}
	let mut client = CodePushClient::new(
		Offline,
		REGISTRY,
		"app",
		"staging",
		Version::new(0, 9, 0),
		1,
		Vec::new(),
		trusted(),
	);
	assert!(matches!(client.check(), Err(CodePushError::Transport(_))));
}

#[test]
fn an_index_in_a_newer_format_is_rejected() {
	let result = sync_offer((Version::new(0, 9, 0), 1), |index| {
		index.format_version = 2;
	});
	assert!(
		matches!(result, Err(CodePushError::UnsupportedFormatVersion(2))),
		"got {result:?}"
	);
}

#[test]
fn the_index_names_the_newest_release_whatever_the_ingest_order() {
	let scratch = Scratch::new("index-order");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("open store");
	for version in [1, 3, 2] {
		store
			.ingest_directory(
				&publisher(),
				"app",
				"production",
				&Version::new(version, 0, 0),
				1,
				&write_bundle(&scratch, &format!("v{version}"), &format!("v{version}")),
				false,
			)
			.expect("ingest");
	}
	let index = store
		.index("app", "production")
		.expect("read index")
		.expect("index exists");
	assert_eq!(index.manifest.version, Version::new(3, 0, 0));
}

#[test]
fn concurrent_ingests_of_different_releases_leave_the_index_on_the_newest() {
	let scratch = Scratch::new("index-race");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("open store");
	let bundles: Vec<_> = (1..=4)
		.map(|version| write_bundle(&scratch, &format!("v{version}"), &format!("v{version}")))
		.collect();
	std::thread::scope(|scope| {
		for (version, bundle) in (1..).zip(&bundles) {
			let store = store.clone();
			scope.spawn(move || {
				store
					.ingest_directory(
						&publisher(),
						"app",
						"production",
						&Version::new(version, 0, 0),
						1,
						bundle,
						false,
					)
					.expect("ingest");
			});
		}
	});
	let index = store
		.index("app", "production")
		.expect("read index")
		.expect("index exists");
	assert_eq!(index.manifest.version, Version::new(4, 0, 0));
}

#[test]
fn publishing_writes_every_release_file_before_the_index() {
	let scratch = Scratch::new("publish-order");
	let (store, v1_container) = staging_history(&scratch);
	let bucket = published(&store, "staging");

	let v1_delta = format!(
		"app/staging/1.0.0+2/deltas/{}.jsondelta",
		hash_bytes(&v1_container)
	);
	assert_eq!(
		bucket.puts,
		[
			"app/staging/0.9.0+1/release.lkb",
			"app/staging/0.9.0+1/release.sig",
			"app/staging/1.0.0+2/release.lkb",
			"app/staging/1.0.0+2/release.sig",
			v1_delta.as_str(),
			"app/staging/index.json",
		]
	);
	for object in bucket.objects.values() {
		let expected = std::fs::read(scratch.path.join("releases").join(object.key.as_str()))
			.expect("the bucket mirrors the store");
		assert_eq!(object.bytes, expected, "{}", object.key.as_str());
	}
	let index = &bucket.objects["app/staging/index.json"];
	assert_eq!(index.key.kind(), ObjectKind::Index);
	assert_eq!(index.key.kind().cache_control(), "public, max-age=60");
	assert_eq!(
		bucket.objects["app/staging/1.0.0+2/release.lkb"]
			.key
			.kind()
			.cache_control(),
		"public, max-age=31536000, immutable"
	);
}

#[test]
fn an_interrupted_publish_leaves_the_channel_on_its_old_release_and_resumes() {
	let scratch = Scratch::new("publish-resume");
	let (store, v1_container) = staging_history(&scratch);
	let mut bucket = MemoryBucket {
		fail_after: Some(3),
		..MemoryBucket::default()
	};
	let error = publish_channel(&store, "app", "staging", &mut bucket)
		.expect_err("the connection drops mid-publish");
	assert!(matches!(error, CodePushError::Transport(_)), "{error:?}");
	assert!(
		!bucket.objects.contains_key("app/staging/index.json"),
		"no device may learn of a release whose files are missing"
	);
	let mut client = staging_client(bucket.responses(), v1_container);
	assert_eq!(client.check().expect("check"), UpdatePlan::UpToDate);

	bucket.fail_after = None;
	let report = publish_channel(&store, "app", "staging", &mut bucket).expect("resume");
	assert_eq!(report.already_present, 3);
	assert_eq!(report.written, 3);
	assert_eq!(
		bucket.puts.last().map(String::as_str),
		Some("app/staging/index.json")
	);
	assert_eq!((report.version, report.build), (Version::new(1, 0, 0), 2));
}

#[test]
fn publishing_again_uploads_only_new_releases() {
	let scratch = Scratch::new("publish-incremental");
	let (store, _) = staging_history(&scratch);
	let mut bucket = published(&store, "staging");

	let report = publish_channel(&store, "app", "staging", &mut bucket).expect("republish");
	assert_eq!(report.written, 0, "nothing new to publish");
	assert!(report.releases.is_empty());

	store
		.ingest_directory(
			&publisher(),
			"app",
			"staging",
			&Version::new(1, 1, 0),
			1,
			&write_bundle(&scratch, "v3", "v3 marker"),
			false,
		)
		.expect("ingest v3");
	bucket.puts.clear();
	let report = publish_channel(&store, "app", "staging", &mut bucket).expect("publish v3");
	assert_eq!(report.releases, ["1.1.0+1"]);
	assert!(
		bucket
			.puts
			.iter()
			.all(|key| !key.contains("/0.9.0+1/") && !key.contains("/1.0.0+2/")),
		"published releases are not uploaded again: {:?}",
		bucket.puts
	);
	assert_eq!(
		bucket.puts.last().map(String::as_str),
		Some("app/staging/index.json")
	);
}

#[test]
fn publishing_never_rolls_a_channel_back() {
	let scratch = Scratch::new("publish-ahead");
	let (store, _) = staging_history(&scratch);
	let mut bucket = published(&store, "staging");

	// Another machine's store that only has the first release.
	let behind = ReleaseStore::open(scratch.path.join("behind")).expect("open store");
	behind
		.ingest_directory(
			&publisher(),
			"app",
			"staging",
			&Version::new(0, 9, 0),
			1,
			&write_bundle(&scratch, "v1", "v1 marker"),
			false,
		)
		.expect("ingest");
	bucket.puts.clear();
	let error = publish_channel(&behind, "app", "staging", &mut bucket).expect_err("behind");
	assert!(
		matches!(error, CodePushError::RegistryAhead { .. }),
		"{error:?}"
	);
	assert!(bucket.puts.is_empty());
}

#[test]
fn publishing_refuses_to_replace_a_different_release() {
	let scratch = Scratch::new("publish-conflict");
	let (store, _) = staging_history(&scratch);
	let mut bucket = published(&store, "staging");

	// Another store released different contents as the same 1.0.0+2.
	let other = ReleaseStore::open(scratch.path.join("other")).expect("open store");
	other
		.ingest_directory(
			&publisher(),
			"app",
			"staging",
			&Version::new(1, 0, 0),
			2,
			&write_bundle(&scratch, "impostor", "impostor"),
			false,
		)
		.expect("ingest");
	bucket.puts.clear();
	let error = publish_channel(&other, "app", "staging", &mut bucket).expect_err("conflict");
	assert!(
		matches!(error, CodePushError::RegistryConflict(_)),
		"{error:?}"
	);
	assert!(bucket.puts.is_empty());

	// The same, for a release file the bucket already holds from an
	// interrupted publish by another store.
	let mut bucket = MemoryBucket::default();
	let lkb = ObjectKey::release("app", "staging", &Version::new(1, 0, 0), 2);
	bucket
		.put(&StaticObject {
			key: lkb,
			bytes: b"someone else's container".to_vec(),
		})
		.expect("put");
	let error = publish_channel(&other, "app", "staging", &mut bucket).expect_err("conflict");
	assert!(
		matches!(error, CodePushError::RegistryConflict(_)),
		"{error:?}"
	);
	assert!(!bucket.objects.contains_key("app/staging/index.json"));
}

#[test]
fn publishing_an_empty_channel_is_an_error() {
	let scratch = Scratch::new("publish-empty");
	let store = ReleaseStore::open(scratch.path.join("releases")).expect("open store");
	let mut bucket = MemoryBucket::default();
	assert!(matches!(
		publish_channel(&store, "app", "production", &mut bucket),
		Err(CodePushError::NoReleases(_))
	));
}
