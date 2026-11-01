//! End-to-end code push integration test: a real axum router serves a real
//! release store, and a real client — the same `CodePushClient` a device
//! shell runs — checks, downloads, hash-verifies, and stages an update over
//! the same HTTP surface production uses.
//!
//! The only test double is the network itself: requests are driven through
//! the router in-process, so the wire format, query parsing, status codes,
//! and byte-for-byte payloads are all the real ones.

use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::Uri;
use http_body_util::BodyExt;
use leptoskit_axum::ServeOptions;
use leptoskit_axum::dev_router;
use leptoskit_codepush::CodePushClient;
use leptoskit_codepush::CodePushError;
use leptoskit_codepush::ReleaseSigningKey;
use leptoskit_codepush::ReleaseStore;
use leptoskit_codepush::TrustedKeys;
use leptoskit_codepush::UpdatePlan;
use leptoskit_codepush::read_container;
use semver::Version;
use tower::util::ServiceExt;

/// The publisher key the test releases are signed with.
fn publisher() -> ReleaseSigningKey {
	ReleaseSigningKey::from_seed(&[0x11; 32])
}

struct Scratch {
	path: PathBuf,
}

impl Scratch {
	fn new(tag: &str) -> Self {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
		let path = std::env::temp_dir().join(format!(
			"leptoskit_e2e_{tag}_{}_{}",
			std::process::id(),
			unique
		));
		std::fs::create_dir_all(&path).expect("scratch");
		Self { path }
	}

	/// Writes one bundle source directory: a module that compresses without
	/// repeating itself, the same in every version but for the marker it
	/// starts with, so the delta between two versions does real work: the
	/// compressed chunk holding the marker changes, and the rest copies.
	fn write_version(&self, version: &str, marker: &str, extra: &str) -> PathBuf {
		let dir = self.path.join(version);
		std::fs::create_dir_all(dir.join("assets")).expect("version dir");
		let mut state = 0x9e37_79b9_u64;
		let mut wasm = marker.as_bytes().to_vec();
		wasm.extend((0..256 * 1024).map(|_| {
			state ^= state << 13;
			state ^= state >> 7;
			state ^= state << 17;
			b"\0asm local.get call"[(state % 19) as usize]
		}));
		std::fs::write(dir.join("app.wasm"), &wasm).expect("write wasm");
		std::fs::write(dir.join("assets").join("extra.txt"), extra).expect("write extra");
		dir
	}

	/// Ingests a two-release history and returns the store.
	fn store_with_history(&self) -> ReleaseStore {
		let store = ReleaseStore::open(self.path.join("releases")).expect("open store");
		store
			.ingest_directory(
				&publisher(),
				"app",
				"production",
				&Version::new(0, 9, 0),
				1,
				&self.write_version("0.9.0", "v1-marker", "v1 extra\n"),
				false,
			)
			.expect("ingest v0.9.0");
		store
			.ingest_directory(
				&publisher(),
				"app",
				"production",
				&Version::new(1, 0, 0),
				2,
				&self.write_version("1.0.0", "v2-marker-with-more-content", "v2 extra\n"),
				true,
			)
			.expect("ingest v1.0.0");
		store
	}

	fn router(&self, store: ReleaseStore) -> axum::Router {
		let options =
			ServeOptions::new(self.path.join("dist"), "127.0.0.1", 0).with_release_store(store);
		dev_router(&options)
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.path);
	}
}

/// Drives requests through the real router in-process: the client sees the
/// exact bytes, status codes, and JSON the HTTP surface produces.
///
/// `Transport::fetch` is synchronous, and the tests run inside a tokio
/// runtime, so each fetch runs the router on its own thread with its own
/// runtime rather than nesting `block_on`.
#[derive(Clone)]
struct RouterTransport {
	router: axum::Router,
}

impl leptoskit_codepush::Transport for RouterTransport {
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
		std::thread::scope(|scope| {
			let router = self.router.clone();
			let url = url.to_owned();
			scope
				.spawn(move || {
					tokio::runtime::Runtime::new()
						.expect("runtime")
						.block_on(async move {
							let (method, target) = url.split_once(' ').unwrap_or(("GET", &url));
							let parsed: Uri = target.parse().map_err(|error| {
								CodePushError::Transport(format!("invalid url: {error}"))
							})?;
							let request = Request::builder()
								.method(method)
								.uri(parsed)
								.body(Body::empty())
								.expect("request builds");
							let response = router.oneshot(request).await.map_err(|error| {
								CodePushError::Transport(format!("router: {error}"))
							})?;
							let status = response.status();
							let bytes = response
								.into_body()
								.collect()
								.await
								.expect("body collects")
								.to_bytes();
							if status != StatusCode::OK {
								return Err(CodePushError::Transport(format!(
									"{target} returned {status}"
								)));
							}

							Ok(bytes.to_vec())
						})
				})
				.join()
				.expect("fetch thread panics propagate")
		})
	}
}

#[tokio::test]
async fn check_endpoint_reports_the_latest_release() {
	let scratch = Scratch::new("check");
	let store = scratch.store_with_history();
	let router = scratch.router(store);

	let response = router
		.clone()
		.oneshot(
			Request::get("/api/codepush/app/production/check?version=0.9.0&build=1")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.expect("request");
	assert_eq!(response.status(), StatusCode::OK);
	let body = response
		.into_body()
		.collect()
		.await
		.expect("body")
		.to_bytes();
	let plan: UpdatePlan = serde_json::from_slice(&body).expect("plan");
	let UpdatePlan::Available(offer) = plan else {
		panic!("expected an update offer");
	};
	assert_eq!(offer.manifest.version, Version::new(1, 0, 0));
	assert!(offer.manifest.mandatory, "the v1.0.0 release is mandatory");

	assert!(
		offer.delta_url.is_some(),
		"a client on the previous release must be offered the delta"
	);
}

#[tokio::test]
async fn download_endpoint_serves_the_full_container() {
	let scratch = Scratch::new("download");
	let store = scratch.store_with_history();
	let router = scratch.router(store);

	let response = router
		.clone()
		.oneshot(
			Request::get("/api/codepush/app/production/download/1.0.0/2")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.expect("request");
	assert_eq!(response.status(), StatusCode::OK);
	let body = response
		.into_body()
		.collect()
		.await
		.expect("body")
		.to_bytes();
	let container = read_container(&body).expect("container verifies");
	assert_eq!(container.manifest.version, Version::new(1, 0, 0));
	assert!(container.files.iter().any(|(path, content)| {
		path == "app.wasm" && content.starts_with(b"v2-marker-with-more-content")
	}));
}

#[tokio::test]
async fn client_syncs_over_the_wire_with_a_delta() {
	let scratch = Scratch::new("sync");
	let store = scratch.store_with_history();
	let base = store
		.find("app", "production", &Version::new(0, 9, 0), 1)
		.expect("find base")
		.expect("base release present");
	let router = scratch.router(store);

	let mut client = CodePushClient::new(
		RouterTransport { router },
		"https://updates.example.test/api/codepush",
		"app",
		"production",
		Version::new(0, 9, 0),
		1,
		base.container,
		TrustedKeys::new([publisher().public_key()]).expect("trusted keys"),
	);

	let report = client.sync(&scratch.path.join("staged")).expect("sync");
	assert!(report.updated, "the client should install v1.0.0");
	assert!(
		report.used_delta,
		"a client on v0.9.0 should receive the delta, not the full bundle"
	);
	assert_eq!(report.version, Some(Version::new(1, 0, 0)));

	// The staged tree is the real v1.0.0 bundle: files present, bytes right.
	let staged = scratch.path.join("staged");
	let wasm = std::fs::read(staged.join("app.wasm")).expect("staged wasm");
	assert!(
		wasm.starts_with(b"v2-marker-with-more-content"),
		"the staged wasm must be the v1.0.0 bytes"
	);
	let extra = std::fs::read(staged.join("assets").join("extra.txt")).expect("staged extra");
	assert_eq!(extra, b"v2 extra\n");

	// The client is now up to date over the same wire.
	let plan = client.check().expect("check");
	assert_eq!(plan, UpdatePlan::UpToDate);
}

#[tokio::test]
async fn a_tampered_container_fails_the_hash_check() {
	// Verification is the client's contract with the registry: a container
	// whose bytes do not hash to what the manifest promised must never stage.
	let scratch = Scratch::new("tamper");
	let dir = scratch.write_version("0.9.0", "v1-marker", "v1 extra\n");
	let container_path = scratch.path.join("tampered.lkb");
	let manifest = leptoskit_codepush::bundle_directory(&dir, &container_path).expect("bundle");
	let mut bytes = std::fs::read(&container_path).expect("read container");
	*bytes.last_mut().expect("non-empty") ^= 0xff;
	assert!(
		read_container(&bytes).is_err(),
		"corruption must be detected before anything stages"
	);
	let _ = manifest;
}

/// The probe path used on devices also works from plain files: the same
/// client, fed the store over the filesystem transport.
#[tokio::test]
async fn file_transport_paths_still_work() {
	let scratch = Scratch::new("file-transport");
	let dir = scratch.write_version("0.9.0", "v1-marker", "v1 extra\n");
	let container_path = scratch.path.join("file.lkb");
	leptoskit_codepush::bundle_directory(&dir, &container_path).expect("bundle");
	let mut transport = leptoskit_codepush::FileTransport;
	let body = leptoskit_codepush::Transport::fetch(
		&mut transport,
		&format!("file://{}", container_path.display()),
	)
	.expect("fetch");
	let container = read_container(&body).expect("container verifies");
	assert_eq!(container.manifest.version, Version::new(0, 1, 0));
}
