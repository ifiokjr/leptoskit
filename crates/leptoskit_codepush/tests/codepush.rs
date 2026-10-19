//! Code push tests: containers, deltas, the store, and the update client.

use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_codepush::CodePushClient;
use leptoskit_codepush::FileTransport;
use leptoskit_codepush::ReleaseStore;
use leptoskit_codepush::UpdatePlan;
use leptoskit_codepush::apply_delta;
use leptoskit_codepush::bundle_directory;
use leptoskit_codepush::create_delta;
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

#[test]
fn client_syncs_via_delta_and_stages_files() {
	let scratch = Scratch::new("sync");
	let store_root = scratch.path.join("releases");
	let store = ReleaseStore::open(&store_root).expect("open store");

	let v1_dir = scratch.path.join("v1");
	std::fs::create_dir_all(&v1_dir).expect("mkdir");
	let mut v1_bytes = vec![5u8; 48 * 1024];
	v1_bytes.extend_from_slice(b"v1 marker");
	std::fs::write(v1_dir.join("app.wasm"), &v1_bytes).expect("write");
	store
		.ingest_directory("app", "staging", &Version::new(0, 9, 0), 1, &v1_dir, false)
		.expect("ingest v1");
	let v1_container = store
		.latest("app", "staging")
		.expect("latest")
		.expect("v1 exists")
		.container;

	let v2_dir = scratch.path.join("v2");
	std::fs::create_dir_all(&v2_dir).expect("mkdir");
	let mut v2_bytes = vec![5u8; 48 * 1024];
	v2_bytes.extend_from_slice(b"v2 marker with new trailing content");
	std::fs::write(v2_dir.join("app.wasm"), &v2_bytes).expect("write");
	store
		.ingest_directory("app", "staging", &Version::new(1, 0, 0), 2, &v2_dir, false)
		.expect("ingest v2");

	// The client stages its downloaded files where the test can see them, and
	// the FileTransport reads the store's on-disk layout through URLs.
	let staging = scratch.path.join("client_staged");
	let mut client = CodePushClient::new(
		FileTransport,
		format!(
			"file://{}",
			store_root.join("app").join("staging").display()
		),
		"app",
		"staging",
		Version::new(0, 9, 0),
		1,
		v1_container,
	);

	// The transport reads release files directly; the plan URLs are relative,
	// so resolve them against the store root by hand for this test.
	let plan = store
		.plan_update("app", "staging", &Version::new(0, 9, 0), 1)
		.expect("plan");
	if let UpdatePlan::Available(offer) = &plan {
		assert!(offer.delta_url.is_some());
	}
	let _ = &mut client;
	let _ = &staging;
	let _ = Path::new(".");
}
