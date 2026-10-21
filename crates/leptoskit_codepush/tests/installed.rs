//! Native installed-release lifecycle: signed bytes, durable trial markers,
//! stale callbacks, compatibility, and offline recovery.

use std::fs;

use leptoskit_codepush::ChannelIndex;
use leptoskit_codepush::CodePushClient;
use leptoskit_codepush::CodePushError;
use leptoskit_codepush::ReleaseSigningKey;
use leptoskit_codepush::Transport;
use leptoskit_codepush::TrustedKeys;
use leptoskit_codepush::bundle_directory_as;
use leptoskit_codepush::installed::InstalledBundles;
use leptoskit_codepush::installed::RuntimeContract;
use leptoskit_codepush::installed::UpdateConfiguration;
use leptoskit_codepush::installed::UpdateStatus;
use leptoskit_codepush::layout::INDEX_FORMAT_VERSION;
use leptoskit_codepush::read_container;
use semver::Version;

// Two valid modules: the replacement has an authenticated custom section.
const BASELINE: &[u8] = b"\0asm\x01\0\0\0";
const REPLACEMENT: &[u8] = b"\0asm\x01\0\0\0\0\x03\x01v2";

fn publisher() -> ReleaseSigningKey {
	ReleaseSigningKey::from_seed(&[19; 32])
}

fn contract() -> RuntimeContract {
	RuntimeContract {
		abi: 1,
		shell: "counter-shell-v1".to_owned(),
		plugins: vec!["clipboard".to_owned()],
	}
}

fn configuration() -> UpdateConfiguration {
	UpdateConfiguration {
		enabled: true,
		app: "counter".to_owned(),
		channel: "production".to_owned(),
		server: "https://updates.example".to_owned(),
		public_keys: vec![publisher().public_key().to_hex()],
		runtime: contract(),
		allow_http: false,
	}
}

fn bundle(build: u64, wasm: Option<&[u8]>, runtime: Option<&[u8]>) -> Vec<u8> {
	let temporary = tempfile::tempdir().expect("bundle directory");
	let source = temporary.path().join("source");
	let output = temporary.path().join("app.lkb");
	fs::create_dir(&source).expect("source");

	if let Some(wasm) = wasm {
		fs::write(source.join("app.wasm"), wasm).expect("module");
	}

	if let Some(runtime) = runtime {
		fs::write(source.join("runtime.json"), runtime).expect("descriptor");
	}

	bundle_directory_as(
		&source,
		&output,
		"counter",
		"production",
		Version::new(1, 0, 0),
		build,
	)
	.expect("bundle");
	fs::read(output).expect("container")
}

fn native_bundle(build: u64, wasm: &[u8]) -> Vec<u8> {
	bundle(
		build,
		Some(wasm),
		Some(&serde_json::to_vec(&contract()).expect("runtime")),
	)
}

struct Registry {
	index: ChannelIndex,
	container: Vec<u8>,
	interrupt: bool,
}

impl Registry {
	fn new(container: Vec<u8>) -> Self {
		let parsed = read_container(&container).expect("container");
		let signature = publisher().sign(&parsed.manifest);
		Self {
			index: ChannelIndex {
				format_version: INDEX_FORMAT_VERSION,
				manifest: parsed.manifest,
				signature: Some(signature),
			},
			container,
			interrupt: false,
		}
	}
}

impl Transport for Registry {
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
		if url.ends_with("/index.json") {
			return Ok(serde_json::to_vec(&self.index)?);
		}

		if self.interrupt {
			return Err(CodePushError::Transport("interrupted download".to_owned()));
		}

		Ok(self.container.clone())
	}
}

#[test]
fn confirmed_candidate_survives_offline_restart_with_original_signature() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let replacement = native_bundle(1, REPLACEMENT);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	let original = installed.launch().expect("baseline");
	assert_eq!(original.wasm, BASELINE);
	assert!(!original.metadata.candidate);
	let report = installed
		.sync_with(&mut Registry::new(replacement))
		.expect("download");
	assert_eq!(report.status, UpdateStatus::Staged);
	// A late healthy baseline callback must not discard the newly downloaded
	// trial.
	installed
		.confirm(&original.metadata.receipt)
		.expect("baseline ready");
	let candidate = installed.launch().expect("trial");
	assert_eq!(candidate.wasm, REPLACEMENT);
	assert!(candidate.metadata.candidate);
	installed
		.confirm(&candidate.metadata.receipt)
		.expect("healthy first frame");
	installed
		.confirm(&candidate.metadata.receipt)
		.expect("idempotent confirmation");
	drop(installed);

	let mut offline =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("offline open");
	let launch = offline.launch().expect("offline launch");
	assert_eq!(launch.wasm, REPLACEMENT);
	assert_eq!(launch.metadata.build, 1);
	assert!(!launch.metadata.candidate);
}

#[test]
fn ready_survives_restart_but_unconfirmed_trial_recovers_and_replay_fails() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let replacement = native_bundle(1, REPLACEMENT);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	installed
		.sync_with(&mut Registry::new(replacement.clone()))
		.expect("stage");
	drop(installed);

	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("ready reopen");
	let trial = installed.launch().expect("ready trial");
	assert!(trial.metadata.candidate);
	assert_eq!(trial.wasm, REPLACEMENT);
	drop(installed);

	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("crash reopen");
	assert!(
		installed
			.recovery_diagnostic()
			.expect("recovery reason")
			.contains("unconfirmed")
	);
	assert_eq!(installed.launch().expect("fallback").wasm, BASELINE);
	assert!(installed.confirm(&trial.metadata.receipt).is_err());
	assert!(
		installed
			.sync_with(&mut Registry::new(replacement))
			.expect_err("failed bundle replay")
			.to_string()
			.contains("previously failed")
	);
	installed
		.sync_with(&mut Registry::new(native_bundle(2, REPLACEMENT)))
		.expect("a fixed release may follow");
}

#[test]
fn explicit_rejection_preserves_last_confirmed_download_and_stale_callbacks_fail() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	installed
		.sync_with(&mut Registry::new(native_bundle(1, REPLACEMENT)))
		.expect("stage first");
	let working = installed.launch().expect("first trial");
	installed
		.confirm(&working.metadata.receipt)
		.expect("first ready");
	installed
		.sync_with(&mut Registry::new(native_bundle(2, BASELINE)))
		.expect("stage bad startup");
	let bad = installed.launch().expect("second trial");
	assert!(installed.confirm(&working.metadata.receipt).is_err());
	installed
		.reject(&bad.metadata.receipt)
		.expect("native host reports startup failure");
	installed
		.reject(&bad.metadata.receipt)
		.expect("idempotent reject");
	assert!(installed.confirm(&bad.metadata.receipt).is_err());
	let fallback = installed.launch().expect("working fallback");
	assert_eq!(fallback.wasm, REPLACEMENT);
	assert_eq!(fallback.metadata.build, 1);
	assert!(!fallback.metadata.candidate);
	assert!(
		installed.reject(&fallback.metadata.receipt).is_err(),
		"working baseline failure must surface"
	);
}

#[test]
fn incompatible_or_missing_native_contract_and_entry_never_become_ready() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	let mut incompatible_abi = contract();
	incompatible_abi.abi = 2;
	let mut incompatible_shell = contract();
	incompatible_shell.shell = "other-shell".to_owned();
	let mut missing_plugin = contract();
	missing_plugin.plugins.push("camera".to_owned());
	let descriptors = [
		serde_json::to_vec(&incompatible_abi).expect("abi"),
		serde_json::to_vec(&incompatible_shell).expect("shell"),
		serde_json::to_vec(&missing_plugin).expect("plugin"),
		br#"{"abi":1,"shell":"counter-shell-v1","plugins":[],"future":true}"#.to_vec(),
	];

	for descriptor in descriptors {
		let bytes = bundle(1, Some(REPLACEMENT), Some(&descriptor));
		assert!(installed.sync_with(&mut Registry::new(bytes)).is_err());
		assert_eq!(
			installed.launch().expect("unchanged baseline").wasm,
			BASELINE
		);
	}

	let runtime = serde_json::to_vec(&contract()).expect("runtime");

	for bytes in [
		bundle(1, Some(REPLACEMENT), None),
		bundle(1, None, Some(&runtime)),
		bundle(1, Some(b"not wasm"), Some(&runtime)),
	] {
		assert!(installed.sync_with(&mut Registry::new(bytes)).is_err());
		assert_eq!(
			installed
				.launch()
				.expect("unchanged baseline")
				.metadata
				.build,
			0
		);
	}
}

#[test]
fn interrupted_download_and_invalid_signature_leave_no_trial() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	let mut interrupted = Registry::new(native_bundle(1, REPLACEMENT));
	interrupted.interrupt = true;
	assert!(installed.sync_with(&mut interrupted).is_err());
	let mut forged = Registry::new(native_bundle(1, REPLACEMENT));
	forged
		.index
		.signature
		.as_mut()
		.expect("signature")
		.signature = "00".repeat(64);
	assert!(matches!(
		installed.sync_with(&mut forged),
		Err(CodePushError::BadSignature(_))
	));
	let mut changed = Registry::new(native_bundle(1, REPLACEMENT));
	changed.container = native_bundle(2, REPLACEMENT);
	assert!(matches!(
		installed.sync_with(&mut changed),
		Err(CodePushError::HashMismatch { .. })
	));
	drop(installed);

	let mut reopened =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reopen");
	assert_eq!(reopened.launch().expect("baseline").metadata.build, 0);
}

#[test]
fn offline_tampering_and_unknown_selector_recover_with_diagnostic() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let replacement = native_bundle(1, REPLACEMENT);
	let id = read_container(&replacement)
		.expect("parse")
		.manifest
		.bundle_hash();
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	installed
		.sync_with(&mut Registry::new(replacement))
		.expect("stage");
	let trial = installed.launch().expect("trial");
	installed.confirm(&trial.metadata.receipt).expect("confirm");
	drop(installed);
	fs::write(
		directory
			.path()
			.join("releases")
			.join(id)
			.join("release.sig"),
		b"{}",
	)
	.expect("tamper");
	let mut recovered =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("fallback");
	assert!(recovered.recovery_diagnostic().is_some());
	assert_eq!(recovered.launch().expect("baseline").wasm, BASELINE);
	drop(recovered);
	fs::write(directory.path().join("state.json"), br#"{"format":123}"#).expect("unknown selector");
	let mut recovered = InstalledBundles::open(directory.path(), &baseline, configuration())
		.expect("selector recovery");
	assert!(
		recovered
			.recovery_diagnostic()
			.expect("diagnostic")
			.contains("invalid installed selector")
	);
	assert_eq!(recovered.launch().expect("baseline").metadata.build, 0);
}

#[test]
fn newer_store_baseline_supersedes_confirmed_and_ready_downloads() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	installed
		.sync_with(&mut Registry::new(native_bundle(1, REPLACEMENT)))
		.expect("stage");
	let launch = installed.launch().expect("trial");
	installed
		.confirm(&launch.metadata.receipt)
		.expect("confirm");
	installed
		.sync_with(&mut Registry::new(native_bundle(2, REPLACEMENT)))
		.expect("stage next");
	drop(installed);
	let newer = native_bundle(3, BASELINE);
	let mut upgraded =
		InstalledBundles::open(directory.path(), &newer, configuration()).expect("store upgrade");
	let launch = upgraded.launch().expect("new store baseline");
	assert_eq!(launch.wasm, BASELINE);
	assert_eq!(launch.metadata.build, 3);
	assert!(!launch.metadata.candidate);
}

#[test]
fn disabled_updates_need_no_keys_and_make_no_transport_calls() {
	struct Unreachable;
	impl Transport for Unreachable {
		fn fetch(&mut self, _: &str) -> Result<Vec<u8>, CodePushError> {
			panic!("disabled transport")
		}
	}
	let directory = tempfile::tempdir().expect("installed");
	let mut config = configuration();
	config.enabled = false;
	config.public_keys.clear();
	let mut installed =
		InstalledBundles::open(directory.path(), &native_bundle(0, BASELINE), config)
			.expect("disabled baseline");
	assert_eq!(
		installed
			.sync_with(&mut Unreachable)
			.expect("disabled check")
			.status,
		UpdateStatus::Current
	);
}

#[test]
fn download_verification_does_not_advance_client_running_version() {
	let mut client = CodePushClient::new(
		Registry::new(native_bundle(1, REPLACEMENT)),
		"https://updates.example",
		"counter",
		"production",
		Version::new(1, 0, 0),
		0,
		native_bundle(0, BASELINE),
		TrustedKeys::new([publisher().public_key()]).expect("keys"),
	);
	assert_eq!(
		client
			.download_verified()
			.expect("verified")
			.expect("new release")
			.manifest
			.build,
		1
	);
	assert_eq!(client.current_version(), &Version::new(1, 0, 0));
	// Asking again must still request build 0 and accept the same signed offer.
	assert!(client.download_verified().expect("check again").is_some());
}

#[cfg(all(feature = "http", feature = "server"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_axum_tcp_download_activation_and_offline_restart() {
	let temporary = tempfile::tempdir().expect("fixture");
	let source = temporary.path().join("replacement");
	fs::create_dir(&source).expect("source");
	fs::write(source.join("app.wasm"), REPLACEMENT).expect("module");
	fs::write(
		source.join("runtime.json"),
		serde_json::to_vec(&contract()).expect("descriptor"),
	)
	.expect("runtime");
	let store =
		leptoskit_codepush::ReleaseStore::open(temporary.path().join("registry")).expect("store");
	store
		.ingest_directory(
			&publisher(),
			"counter",
			"production",
			&Version::new(1, 0, 0),
			1,
			&source,
			false,
		)
		.expect("publish");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
		.await
		.expect("bind");
	let address = listener.local_addr().expect("address");
	let server = tokio::spawn(async move {
		axum::serve(listener, leptoskit_codepush::codepush_router(store))
			.await
			.expect("serve");
	});
	let installed_path = temporary.path().join("installed");
	let mut config = configuration();
	config.server = format!("http://{address}");
	config.allow_http = true;
	let (baseline, config) = tokio::task::spawn_blocking(move || {
		let baseline = native_bundle(0, BASELINE);
		let mut installed =
			InstalledBundles::open(&installed_path, &baseline, config.clone()).expect("open");
		assert_eq!(
			installed.sync().expect("real HTTP stage").status,
			UpdateStatus::Staged
		);
		let launch = installed.launch().expect("activate");
		assert_eq!(launch.wasm, REPLACEMENT);
		installed
			.confirm(&launch.metadata.receipt)
			.expect("first frame");
		(baseline, config)
	})
	.await
	.expect("worker");
	server.abort();
	let mut offline =
		InstalledBundles::open(&temporary.path().join("installed"), &baseline, config)
			.expect("offline reopen");
	let launch = offline.launch().expect("offline launch");
	assert_eq!(launch.wasm, REPLACEMENT);
	assert!(!launch.metadata.candidate);
}

#[cfg(feature = "http")]
#[test]
fn production_http_transport_rejects_cleartext_and_non_http_schemes() {
	let mut transport = leptoskit_codepush::installed::HttpTransport::new(false);

	for url in [
		"http://127.0.0.1:1",
		"file:///private/app.lkb",
		"ftp://example.com",
	] {
		assert!(
			transport
				.fetch(url)
				.expect_err("scheme")
				.to_string()
				.contains("HTTPS")
		);
	}
}

#[test]
fn failure_while_confirm_result_is_in_flight_restores_previous_selection() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	installed
		.sync_with(&mut Registry::new(native_bundle(1, REPLACEMENT)))
		.expect("stage");
	let trial = installed.launch().expect("trial");
	installed
		.confirm(&trial.metadata.receipt)
		.expect("disk confirmation wins worker race");
	installed
		.reject(&trial.metadata.receipt)
		.expect("UI refused handoff before confirmation callback");
	assert!(installed.confirm(&trial.metadata.receipt).is_err());
	drop(installed);
	let mut reopened =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("restart");
	assert_eq!(
		reopened.launch().expect("working baseline").metadata.build,
		0
	);
}

#[test]
fn previous_confirmation_cannot_be_rejected_after_another_launch_or_restart() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	installed
		.sync_with(&mut Registry::new(native_bundle(1, REPLACEMENT)))
		.expect("stage");
	let trial = installed.launch().expect("trial");
	installed.confirm(&trial.metadata.receipt).expect("confirm");
	drop(installed);
	let mut reopened =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("restart");
	assert!(
		reopened.reject(&trial.metadata.receipt).is_err(),
		"live rollback marker never persists"
	);
	let next = reopened.launch().expect("next launch");
	assert_ne!(trial.metadata.receipt, next.metadata.receipt);
	assert!(reopened.reject(&trial.metadata.receipt).is_err());
	assert_eq!(next.metadata.build, 1);
}
