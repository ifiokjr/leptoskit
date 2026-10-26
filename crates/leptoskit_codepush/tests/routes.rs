//! The axum routes serve the store's static layout byte for byte, so a client
//! cannot tell `leptoskit codepush serve` from a bucket behind a CDN.
#![cfg(feature = "server")]

use std::path::PathBuf;

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use leptoskit_codepush::ObjectKey;
use leptoskit_codepush::ReleaseSigningKey;
use leptoskit_codepush::ReleaseStore;
use leptoskit_codepush::codepush_router;
use leptoskit_codepush::hash_bytes;
use semver::Version;
use tower::util::ServiceExt;

struct Scratch(PathBuf);

impl Scratch {
	fn new(tag: &str) -> Self {
		let path = std::env::temp_dir().join(format!(
			"leptoskit_codepush_routes_{tag}_{}",
			std::process::id()
		));
		let _ = std::fs::remove_dir_all(&path);
		std::fs::create_dir_all(&path).expect("scratch");
		Self(path)
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.0);
	}
}

/// A store with `1.0.0+abc` build 1, then `1.1.0` build 2 patched from it.
fn store(scratch: &Scratch) -> (ReleaseStore, Vec<u8>) {
	let store = ReleaseStore::open(scratch.0.join("releases")).expect("open store");
	let signer = ReleaseSigningKey::from_seed(&[0x11; 32]);
	// A module that compresses without repeating itself, shared by both
	// releases, so the second one is worth a delta: only `marker.txt` differs.
	let mut state = 0x9e37_79b9_u64;
	let wasm: Vec<u8> = (0..64 * 1024)
		.map(|_| {
			state ^= state << 13;
			state ^= state >> 7;
			state ^= state << 17;
			b"\0asm local.get call"[(state % 19) as usize]
		})
		.collect();
	let mut v1 = Vec::new();
	for (version, build, marker) in [("1.0.0+abc", 1, "one"), ("1.1.0", 2, "two")] {
		let dir = scratch.0.join(marker);
		std::fs::create_dir_all(&dir).expect("mkdir");
		std::fs::write(dir.join("app.wasm"), &wasm).expect("write");
		std::fs::write(dir.join("marker.txt"), marker).expect("write");
		let ingested = store
			.ingest_directory(
				&signer,
				"app",
				"production",
				&Version::parse(version).expect("version"),
				build,
				&dir,
				false,
			)
			.expect("ingest");
		if v1.is_empty() {
			v1 = ingested.record.container;
		}
	}
	(store, v1)
}

async fn get(store: &ReleaseStore, path: &str) -> (StatusCode, Option<String>, Vec<u8>) {
	let request = Request::builder()
		.uri(path)
		.body(Body::empty())
		.expect("request");
	let response = codepush_router(store.clone())
		.oneshot(request)
		.await
		.expect("response");
	let status = response.status();
	let cache_control = response
		.headers()
		.get(header::CACHE_CONTROL)
		.map(|value| value.to_str().expect("ascii").to_owned());
	let body = axum::body::to_bytes(response.into_body(), usize::MAX)
		.await
		.expect("body");
	(status, cache_control, body.to_vec())
}

#[tokio::test]
async fn static_routes_serve_the_files_a_bucket_would_hold() {
	let scratch = Scratch::new("static");
	let (store, v1) = store(&scratch);
	let v1_1 = Version::new(1, 1, 0);
	let index = ObjectKey::index("app", "production");
	let keys = [
		index.clone(),
		ObjectKey::release("app", "production", &v1_1, 2),
		ObjectKey::signature("app", "production", &v1_1, 2),
		ObjectKey::delta("app", "production", &v1_1, 2, &hash_bytes(&v1)).expect("hash"),
		ObjectKey::release(
			"app",
			"production",
			&Version::parse("1.0.0+abc").expect("version"),
			1,
		),
	];
	for key in keys {
		let (status, cache_control, body) = get(&store, &format!("/{}", key.url_path())).await;
		assert_eq!(status, StatusCode::OK, "{}", key.as_str());
		let expected = std::fs::read(scratch.0.join("releases").join(key.as_str())).expect("file");
		assert_eq!(body, expected, "{}", key.as_str());
		let expected_cache = if key == index {
			"no-cache"
		} else {
			key.kind().cache_control()
		};
		assert_eq!(cache_control.as_deref(), Some(expected_cache));
	}
}

#[tokio::test]
async fn static_routes_answer_404_for_anything_outside_the_layout() {
	let scratch = Scratch::new("missing");
	let (store, _) = store(&scratch);
	let missing_delta = format!(
		"/app/production/1.1.0%2B2/deltas/{}.jsondelta",
		"0".repeat(64)
	);
	for path in [
		"/app/staging/index.json",
		"/app/production/9.0.0%2B1/release.lkb",
		"/app/production/1.1.0%2B02/release.lkb",
		"/app/production/..%2F..%2Fsecret/release.lkb",
		"/app/production/1.1.0%2B2/deltas/..%2F..%2Findex.json",
		"/app/production/1.1.0%2B2/deltas/not-a-hash.jsondelta",
		missing_delta.as_str(),
	] {
		let (status, ..) = get(&store, path).await;
		assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
	}
}

#[tokio::test]
async fn the_dynamic_routes_still_offer_deltas() {
	let scratch = Scratch::new("dynamic");
	let (store, _) = store(&scratch);
	let (status, _, body) = get(&store, "/app/production/check?version=1.0.0%2Babc&build=1").await;
	assert_eq!(status, StatusCode::OK);
	let plan: serde_json::Value = serde_json::from_slice(&body).expect("plan");
	let delta_url = plan["delta_url"].as_str().expect("a delta is offered");
	let (status, ..) = get(&store, delta_url).await;
	assert_eq!(status, StatusCode::OK, "{delta_url}");
	let download_url = plan["download_url"].as_str().expect("download url");
	let (status, _, body) = get(&store, download_url).await;
	assert_eq!(status, StatusCode::OK, "{download_url}");
	let release = ObjectKey::release("app", "production", &Version::new(1, 1, 0), 2);
	let expected = std::fs::read(scratch.0.join("releases").join(release.as_str())).expect("file");
	assert_eq!(
		body, expected,
		"the download is the release file, byte for byte"
	);
}

#[tokio::test]
async fn storage_failures_do_not_describe_the_store() {
	let scratch = Scratch::new("storage-failure");
	let (store, _) = store(&scratch);
	// A release file that cannot be read: here a directory where the
	// container should be.
	let release = ObjectKey::release("app", "production", &Version::new(5, 0, 0), 1);
	std::fs::create_dir_all(scratch.0.join("releases").join(release.as_str())).expect("mkdir");
	for path in [
		format!("/{}", release.url_path()),
		"/app/production/download/5.0.0/1".to_owned(),
		"/app/production/delta/5.0.0/1".to_owned(),
	] {
		let (status, _, body) = get(&store, &path).await;
		assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{path}");
		let body = String::from_utf8(body).expect("utf-8");
		assert!(!body.is_empty(), "{path} explains the failure");
		assert!(
			!body.contains(&scratch.0.display().to_string()) && !body.contains("releases"),
			"{path} must not leak the store layout: {body:?}"
		);
	}
}

#[tokio::test]
async fn the_dynamic_routes_reject_bad_versions_and_unknown_releases() {
	let scratch = Scratch::new("dynamic-errors");
	let (store, _) = store(&scratch);
	for (path, expected) in [
		(
			"/app/production/check?version=nope&build=1",
			StatusCode::BAD_REQUEST,
		),
		("/app/production/download/nope/1", StatusCode::BAD_REQUEST),
		("/app/production/download/9.9.9/1", StatusCode::NOT_FOUND),
		("/app/production/delta/nope/1", StatusCode::BAD_REQUEST),
		("/app/staging/delta/1.0.0%2Babc/1", StatusCode::NOT_FOUND),
		("/app/production/delta/9.9.9/1", StatusCode::NOT_FOUND),
		// The newest release has no delta from itself.
		("/app/production/delta/1.1.0/2", StatusCode::NOT_FOUND),
	] {
		let (status, _, body) = get(&store, path).await;
		assert_eq!(status, expected, "{path}");
		assert!(!body.is_empty(), "{path} explains the error");
	}
}
