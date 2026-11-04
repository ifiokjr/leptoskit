//! Native installed-release lifecycle: signed bytes, durable trial markers,
//! stale callbacks, compatibility, and offline recovery.

use std::fs;

use leptoskit_codepush::ChannelIndex;
use leptoskit_codepush::CodePushClient;
use leptoskit_codepush::CodePushError;
use leptoskit_codepush::ProvenanceConfiguration;
use leptoskit_codepush::ReleaseApprovals;
use leptoskit_codepush::ReleaseSigningKey;
use leptoskit_codepush::SignedApproval;
use leptoskit_codepush::SourceApproval;
use leptoskit_codepush::Transport;
use leptoskit_codepush::TrustedKeys;
use leptoskit_codepush::bundle_directory_as;
use leptoskit_codepush::installed::InstalledBundles;
use leptoskit_codepush::installed::MAX_BUNDLE_BYTES;
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
		provenance: None,
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
	/// The release's approvals file; a 404 without one.
	approvals: Option<Vec<u8>>,
	interrupt: bool,
	/// Every URL fetched, in order.
	fetched: Vec<String>,
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
			approvals: None,
			interrupt: false,
			fetched: Vec::new(),
		}
	}
}

impl Transport for Registry {
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
		self.fetched.push(url.to_owned());

		if url.ends_with("/index.json") {
			return Ok(serde_json::to_vec(&self.index)?);
		}

		if url.contains("/approvals/") {
			return self
				.approvals
				.clone()
				.ok_or_else(|| CodePushError::NotFound(url.to_owned()));
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
fn replacement_owner_cannot_interrupt_a_live_update_or_erase_its_confirmation() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	let (started, waiting) = std::sync::mpsc::channel();
	let (resume, proceed) = std::sync::mpsc::channel();

	struct DelayedRegistry {
		registry: Registry,
		started: std::sync::mpsc::Sender<()>,
		proceed: std::sync::mpsc::Receiver<()>,
		paused: bool,
	}

	impl Transport for DelayedRegistry {
		fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
			if !self.paused && !url.ends_with("/index.json") {
				self.paused = true;
				self.started.send(()).expect("download started");
				self.proceed.recv().expect("resume download");
			}

			self.registry.fetch(url)
		}
	}

	let registry = DelayedRegistry {
		registry: Registry::new(native_bundle(1, REPLACEMENT)),
		started,
		proceed,
		paused: false,
	};
	let worker = std::thread::spawn(move || {
		let mut registry = registry;
		installed.sync_with(&mut registry).expect("stage");
		let trial = installed.launch().expect("trial");
		(installed, trial)
	});
	waiting.recv().expect("download paused");
	let during_download = InstalledBundles::open(directory.path(), &baseline, configuration());
	// Always unblock and join the worker, including when the regression fails.
	resume.send(()).expect("resume");
	let (mut installed, trial) = worker.join().expect("update worker");
	assert!(
		matches!(during_download, Err(CodePushError::Activation(ref message)) if message.contains("already owned"))
	);
	let state = fs::read(directory.path().join("state.json")).expect("trial state");

	assert!(matches!(
		InstalledBundles::open(directory.path(), &baseline, configuration()),
		Err(CodePushError::Activation(message)) if message.contains("already owned")
	));
	assert_eq!(
		fs::read(directory.path().join("state.json")).unwrap(),
		state
	);
	installed.confirm(&trial.metadata.receipt).expect("confirm");
	drop(installed);

	let mut replacement =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("replacement");
	let launch = replacement.launch().expect("confirmed release");
	assert_eq!(launch.wasm, REPLACEMENT);
	assert!(!launch.metadata.candidate);
}

#[test]
fn failed_open_releases_the_directory_for_retry() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let selector = directory.path().join("state.json");
	fs::create_dir(&selector).expect("prevent selector reads");
	assert!(matches!(
		InstalledBundles::open(directory.path(), &baseline, configuration()),
		Err(CodePushError::Io(_))
	));
	fs::remove_dir(selector).expect("repair storage");
	let _retry =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("retry");
}

#[cfg(unix)]
#[test]
fn directory_aliases_share_ownership_but_other_stores_remain_independent() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let path = directory.path().join("store");
	let alias = directory.path().join("alias");
	let owner = InstalledBundles::open(&path, &baseline, configuration()).expect("owner");
	std::os::unix::fs::symlink(&path, &alias).expect("alias");
	assert!(matches!(
		InstalledBundles::open(&alias, &baseline, configuration()),
		Err(CodePushError::Activation(message)) if message.contains("already owned")
	));
	let _independent =
		InstalledBundles::open(&directory.path().join("other"), &baseline, configuration())
			.expect("independent");
	drop(owner);
	let _replacement =
		InstalledBundles::open(&alias, &baseline, configuration()).expect("replacement");
}

#[test]
fn ready_survives_restart_but_an_unconfirmed_trial_recovers_and_stays_refused() {
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
	// The registry still offers the failed release: the device stays on its
	// working release without downloading it again.
	let mut replay = Registry::new(replacement);
	let report = installed
		.sync_with(&mut replay)
		.expect("failed bundle replay");
	assert_eq!((report.status, report.build), (UpdateStatus::Current, 0));

	assert_eq!(
		replay.fetched,
		["https://updates.example/counter/production/index.json"],
		"only the index is fetched"
	);
	assert_eq!(
		installed
			.sync_with(&mut Registry::new(native_bundle(2, REPLACEMENT)))
			.expect("a fixed release may follow")
			.status,
		UpdateStatus::Staged
	);
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
fn an_incompatible_release_is_downloaded_once() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	let mut incompatible = contract();
	incompatible.shell = "other-shell".to_owned();
	let runtime = serde_json::to_vec(&incompatible).expect("runtime");
	let mut registry = Registry::new(bundle(1, Some(REPLACEMENT), Some(&runtime)));

	assert!(installed.sync_with(&mut registry).is_err());
	// The signed container fails the same way every time, so later checks
	// read only the index instead of downloading it again.
	for _ in 0..2 {
		let report = installed.sync_with(&mut registry).expect("current");
		assert_eq!(report.status, UpdateStatus::Current);
	}
	drop(installed);
	let mut reopened =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reopen");
	reopened.sync_with(&mut registry).expect("current");
	let downloads = registry
		.fetched
		.iter()
		.filter(|url| url.ends_with("/release.lkb"))
		.count();
	assert_eq!(downloads, 1, "{:?}", registry.fetched);

	// A new store build may support it, so the record does not outlive the
	// build that rejected it.
	let upgraded = native_bundle(0, b"\0asm\x01\0\0\0\0\x03\x01v0");
	drop(reopened);
	let mut upgraded =
		InstalledBundles::open(directory.path(), &upgraded, configuration()).expect("upgrade");
	assert!(upgraded.sync_with(&mut registry).is_err());
	let downloads = registry
		.fetched
		.iter()
		.filter(|url| url.ends_with("/release.lkb"))
		.count();
	assert_eq!(downloads, 2, "{:?}", registry.fetched);
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

/// Stages and confirms `container`, the way a host does after a healthy frame.
fn install(installed: &mut InstalledBundles, container: Vec<u8>) {
	installed
		.sync_with(&mut Registry::new(container))
		.expect("stage");
	let trial = installed.launch().expect("trial");
	installed.confirm(&trial.metadata.receipt).expect("confirm");
}

fn release_entries(directory: &std::path::Path) -> usize {
	fs::read_dir(directory.join("releases"))
		.expect("releases")
		.count()
}

fn edit_state(directory: &std::path::Path, edit: impl FnOnce(&mut serde_json::Value)) {
	let path = directory.join("state.json");
	let mut state: serde_json::Value =
		serde_json::from_slice(&fs::read(&path).expect("state")).expect("state json");
	edit(&mut state);
	fs::write(path, serde_json::to_vec(&state).expect("state json")).expect("write state");
}

#[test]
fn truncated_download_stores_nothing_and_keeps_the_working_release() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	let full = native_bundle(1, REPLACEMENT);

	for cut in [0, 8, full.len() / 2, full.len() - 1] {
		let mut truncated = Registry::new(full.clone());
		truncated.container.truncate(cut);

		assert!(
			installed.sync_with(&mut truncated).is_err(),
			"a container cut at {cut} bytes must not stage"
		);
	}

	assert_eq!(release_entries(directory.path()), 0);
	let launch = installed.launch().expect("working release");
	assert_eq!(launch.wasm, BASELINE);
	assert!(!launch.metadata.candidate);
	assert_eq!(
		installed
			.sync_with(&mut Registry::new(full))
			.expect("a complete retry")
			.status,
		UpdateStatus::Staged
	);
}

#[test]
fn signed_downgrades_and_replays_are_up_to_date() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	install(&mut installed, native_bundle(2, REPLACEMENT));

	for older in [native_bundle(1, BASELINE), native_bundle(2, REPLACEMENT)] {
		let report = installed
			.sync_with(&mut Registry::new(older))
			.expect("validly signed older release");
		assert_eq!(report.status, UpdateStatus::Current);

		assert_eq!(report.build, 2);
	}

	let launch = installed.launch().expect("still the newer release");
	assert_eq!(launch.metadata.build, 2);
	assert!(!launch.metadata.candidate);
}

/// The stored release's id and directory.
fn release_directory(directory: &std::path::Path, container: &[u8]) -> std::path::PathBuf {
	let id = read_container(container)
		.expect("parse")
		.manifest
		.bundle_hash();
	directory.join("releases").join(id)
}

/// Makes the stored container unreadable, as an I/O error rather than
/// damaged bytes: reading a directory fails on every platform.
fn make_unreadable(release: &std::path::Path) {
	let container = release.join("release.lkb");
	fs::remove_file(&container).expect("container");
	fs::create_dir(&container).expect("unreadable container");
}

fn assert_refused(installed: &mut InstalledBundles, older: Vec<u8>) {
	let report = installed
		.sync_with(&mut Registry::new(older))
		.expect("validly signed older release");
	assert_eq!(
		(report.status, report.build),
		(UpdateStatus::Current, 0),
		"an older release must not be staged"
	);
}

#[test]
fn an_unreadable_confirmed_release_keeps_the_version_floor() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let confirmed = native_bundle(2, REPLACEMENT);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	install(&mut installed, confirmed.clone());
	drop(installed);

	make_unreadable(&release_directory(directory.path(), &confirmed));
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reopen");
	assert_eq!(installed.launch().expect("store baseline").wasm, BASELINE);
	// Still validly signed, and newer than the store baseline that runs now.
	assert_refused(&mut installed, native_bundle(1, BASELINE));
	assert_eq!(
		installed
			.sync_with(&mut Registry::new(native_bundle(3, REPLACEMENT)))
			.expect("a newer release")
			.status,
		UpdateStatus::Staged
	);
}

#[test]
fn the_version_floor_survives_restarts_without_the_confirmed_release() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let confirmed = native_bundle(2, REPLACEMENT);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	install(&mut installed, confirmed.clone());
	drop(installed);

	// A damaged release is dropped and removed at the next open, so later
	// restarts have no confirmed release left to read.
	fs::write(
		release_directory(directory.path(), &confirmed).join("release.lkb"),
		b"damaged",
	)
	.expect("damage");

	for restart in 0..3 {
		let mut installed =
			InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reopen");
		assert_eq!(release_entries(directory.path()), 0, "restart {restart}");
		assert_eq!(installed.launch().expect("store baseline").wasm, BASELINE);
		assert_refused(&mut installed, native_bundle(1, BASELINE));
	}

	// Either copy alone keeps the floor.
	edit_state(directory.path(), |state| {
		state["floor"] = serde_json::Value::Null;
	});
	let mut installed = InstalledBundles::open(directory.path(), &baseline, configuration())
		.expect("floor file only");
	assert_refused(&mut installed, native_bundle(1, BASELINE));
	drop(installed);
	fs::remove_file(directory.path().join("floor.json")).expect("floor file");
	let mut installed = InstalledBundles::open(directory.path(), &baseline, configuration())
		.expect("selector copy only");
	assert_refused(&mut installed, native_bundle(1, BASELINE));
}

#[test]
fn an_older_layout_derives_its_floor_from_the_confirmed_release() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	install(&mut installed, native_bundle(2, REPLACEMENT));
	drop(installed);

	// Stores written before the floor existed have neither copy.
	fs::remove_file(directory.path().join("floor.json")).expect("floor file");
	edit_state(directory.path(), |state| {
		state.as_object_mut().expect("selector").remove("floor");
	});
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("upgrade");
	assert!(installed.recovery_diagnostic().is_none());
	let floor: serde_json::Value =
		serde_json::from_slice(&fs::read(directory.path().join("floor.json")).expect("floor"))
			.expect("floor json");
	assert_eq!(floor["build"], 2);
	let report = installed
		.sync_with(&mut Registry::new(native_bundle(1, BASELINE)))
		.expect("validly signed older release");
	assert_eq!((report.status, report.build), (UpdateStatus::Current, 2));
}

#[test]
fn a_new_store_build_starts_a_new_version_floor() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	install(&mut installed, native_bundle(5, REPLACEMENT));
	drop(installed);

	// The new store build ships at build 1, below the release confirmed on
	// the previous one; releases made for it start above its own baseline.
	let store_build = native_bundle(1, REPLACEMENT);
	let mut upgraded =
		InstalledBundles::open(directory.path(), &store_build, configuration()).expect("upgrade");
	assert_eq!(upgraded.launch().expect("store build").metadata.build, 1);
	assert_eq!(
		upgraded
			.sync_with(&mut Registry::new(native_bundle(3, BASELINE)))
			.expect("a release for the new store build")
			.status,
		UpdateStatus::Staged
	);
}

#[test]
fn a_corrupt_version_floor_still_refuses_downgrades() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let confirmed = native_bundle(2, REPLACEMENT);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	install(&mut installed, confirmed.clone());
	drop(installed);
	let floor = directory.path().join("floor.json");

	// The floor file is torn and the confirmed release can't be read: the
	// copy in the selector still holds the floor.
	fs::write(&floor, b"{\"format\":1,\"emb").expect("torn floor");
	make_unreadable(&release_directory(directory.path(), &confirmed));
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reopen");
	assert!(
		installed
			.recovery_diagnostic()
			.expect("diagnostic")
			.contains("installed release rejected")
	);
	assert_refused(&mut installed, native_bundle(1, BASELINE));
	drop(installed);
	let repaired: serde_json::Value =
		serde_json::from_slice(&fs::read(&floor).expect("floor")).expect("rewritten floor");
	assert_eq!(repaired["build"], 2);

	// A floor from an unknown format is no floor to trust either, and the
	// confirmed release still bounds what installs.
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reopen");
	install(&mut installed, native_bundle(3, BASELINE));
	drop(installed);
	fs::write(&floor, br#"{"format":2}"#).expect("unknown floor");
	edit_state(directory.path(), |state| {
		state["floor"] = serde_json::Value::Null;
	});
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reopen");
	assert!(
		installed
			.recovery_diagnostic()
			.expect("diagnostic")
			.contains("invalid version floor")
	);
	let report = installed
		.sync_with(&mut Registry::new(native_bundle(2, REPLACEMENT)))
		.expect("validly signed older release");
	assert_eq!((report.status, report.build), (UpdateStatus::Current, 3));
}

#[cfg(unix)]
#[test]
fn disk_full_while_staging_keeps_the_working_release() {
	use std::os::unix::fs::PermissionsExt;

	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	install(&mut installed, native_bundle(1, REPLACEMENT));
	let releases = directory.path().join("releases");
	// Refusing writes to the release store fails the same calls a full disk
	// does: creating the release directory and its files.
	fs::set_permissions(&releases, fs::Permissions::from_mode(0o555)).expect("read only");
	let failed = installed.sync_with(&mut Registry::new(native_bundle(2, BASELINE)));

	fs::set_permissions(&releases, fs::Permissions::from_mode(0o755)).expect("writable");
	assert!(matches!(failed, Err(CodePushError::Io(_))), "{failed:?}");

	let launch = installed.launch().expect("working release");
	assert_eq!(launch.metadata.build, 1);
	assert!(!launch.metadata.candidate);
	drop(installed);
	let mut reopened =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reopen");
	assert_eq!(reopened.launch().expect("working").metadata.build, 1);
	assert_eq!(
		reopened
			.sync_with(&mut Registry::new(native_bundle(2, BASELINE)))
			.expect("space is back")
			.status,
		UpdateStatus::Staged
	);
}

#[test]
fn power_loss_at_each_persistence_step_recovers_the_working_release() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	install(&mut installed, native_bundle(1, REPLACEMENT));
	let before = fs::read(directory.path().join("state.json")).expect("selector");

	// Power fails after the release files are durable but before the selector
	// names them: the orphan is ignored, and the same release stages again.
	installed
		.sync_with(&mut Registry::new(native_bundle(2, BASELINE)))
		.expect("stage");
	drop(installed);
	fs::write(directory.path().join("state.json"), &before).expect("lose selector write");
	// An interrupted selector write leaves only its temporary file behind,
	// which the next open clears away.
	let torn = directory.path().join(".state.json.4242.0.0.tmp");
	fs::write(&torn, b"{\"torn").expect("torn write");
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reopen");
	assert!(!torn.exists());

	let launch = installed.launch().expect("working release");
	assert_eq!(launch.metadata.build, 1);
	assert!(!launch.metadata.candidate);
	installed
		.sync_with(&mut Registry::new(native_bundle(2, BASELINE)))
		.expect("restage over the orphan");

	// Power fails between activation and the host's confirmation.
	let trial = installed.launch().expect("trial");
	assert!(trial.metadata.candidate);
	drop(installed);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reboot");
	assert!(installed.confirm(&trial.metadata.receipt).is_err());
	let launch = installed.launch().expect("working release");
	assert_eq!(launch.metadata.build, 1);

	assert!(!launch.metadata.candidate);
}

#[test]
fn a_torn_staged_release_is_rejected_with_a_diagnostic() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	let staged = native_bundle(1, REPLACEMENT);
	let id = read_container(&staged)
		.expect("parse")
		.manifest
		.bundle_hash();
	installed
		.sync_with(&mut Registry::new(staged.clone()))
		.expect("stage");
	drop(installed);

	let file = directory
		.path()
		.join("releases")
		.join(&id)
		.join("release.lkb");
	fs::write(&file, &staged[..staged.len() / 2]).expect("torn container");

	let mut recovered =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("recover");
	assert!(
		recovered
			.recovery_diagnostic()
			.expect("diagnostic")
			.contains("staged release rejected")
	);
	let launch = recovered.launch().expect("baseline");
	assert_eq!(launch.wasm, BASELINE);
	assert!(!launch.metadata.candidate);
	assert_eq!(
		recovered
			.sync_with(&mut Registry::new(staged))
			.expect("a release that failed on disk stays refused")
			.status,
		UpdateStatus::Current
	);
}

#[test]
fn startup_trap_rolls_back_and_a_second_update_still_installs() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	install(&mut installed, native_bundle(1, REPLACEMENT));

	installed
		.sync_with(&mut Registry::new(native_bundle(2, BASELINE)))
		.expect("stage the trapping release");
	let trapped = installed.launch().expect("trial");
	installed.reject(&trapped.metadata.receipt).expect("trap");
	let fallback = installed.launch().expect("rollback");
	assert_eq!(fallback.metadata.build, 1);
	assert!(!fallback.metadata.candidate);
	installed
		.confirm(&fallback.metadata.receipt)
		.expect("working release renders");

	install(&mut installed, native_bundle(3, REPLACEMENT));
	drop(installed);
	let mut reopened =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reopen");
	let launch = reopened.launch().expect("second update");
	assert_eq!(launch.metadata.build, 3);
	assert!(!launch.metadata.candidate);
}

#[test]
fn staged_and_trying_states_gate_further_checks() {
	struct Unreachable;
	impl Transport for Unreachable {
		fn fetch(&mut self, _: &str) -> Result<Vec<u8>, CodePushError> {
			panic!("a staged release needs no network")
		}
	}
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	installed
		.sync_with(&mut Registry::new(native_bundle(1, REPLACEMENT)))
		.expect("stage");
	let staged = installed.sync_with(&mut Unreachable).expect("still staged");
	assert_eq!((staged.status, staged.build), (UpdateStatus::Staged, 1));

	installed.launch().expect("trial");
	assert!(
		installed
			.sync_with(&mut Unreachable)
			.expect_err("outstanding trial")
			.to_string()
			.contains("outstanding trial")
	);
}

#[test]
fn hostile_selectors_never_escape_the_release_store() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	let release = native_bundle(1, REPLACEMENT);
	let confirmed = read_container(&release)
		.expect("parse")
		.manifest
		.bundle_hash();
	install(&mut installed, release);
	drop(installed);

	// A genuine signed release filed under another release's identifier.
	let impostor = "f".repeat(64);
	let releases = directory.path().join("releases");
	fs::create_dir(releases.join(&impostor)).expect("impostor");

	for file in ["release.lkb", "release.sig"] {
		fs::copy(
			releases.join(&confirmed).join(file),
			releases.join(&impostor).join(file),
		)
		.expect("copy");
	}

	edit_state(directory.path(), |state| {
		state["confirmed"] = impostor.clone().into();
	});
	let mut recovered =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("impostor");

	assert!(
		recovered
			.recovery_diagnostic()
			.expect("diagnostic")
			.contains("does not match")
	);
	assert_eq!(recovered.launch().expect("baseline").metadata.build, 0);
	drop(recovered);

	edit_state(directory.path(), |state| {
		state["confirmed"] = "../../../etc".into();
	});
	let mut recovered =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("traversal");
	assert!(
		recovered
			.recovery_diagnostic()
			.expect("diagnostic")
			.contains("invalid installed release identifier")
	);
	assert_eq!(recovered.launch().expect("baseline").metadata.build, 0);
}

#[test]
fn store_baselines_must_be_for_this_app_and_within_size_limits() {
	let directory = tempfile::tempdir().expect("installed");
	let mut other = configuration();
	other.app = "other".to_owned();
	assert!(matches!(
		InstalledBundles::open(directory.path(), &native_bundle(0, BASELINE), other),
		Err(CodePushError::WrongTarget { .. })
	));

	let oversized = vec![0; usize::try_from(MAX_BUNDLE_BYTES).expect("fits") + 1];
	assert!(
		InstalledBundles::open(directory.path(), &oversized, configuration())
			.err()
			.expect("oversized container")
			.to_string()
			.contains("container exceeds")
	);

	// Declared expansion is rejected before anything is decompressed.
	let container = native_bundle(0, BASELINE);
	let manifest_len = usize::try_from(u64::from_le_bytes(
		container[4..12].try_into().expect("length"),
	))
	.expect("fits");
	let mut manifest: serde_json::Value =
		serde_json::from_slice(&container[12..12 + manifest_len]).expect("manifest");
	manifest["files"][0]["size"] = (MAX_BUNDLE_BYTES + 1).into();
	let manifest = serde_json::to_vec(&manifest).expect("manifest");
	let mut bomb = container[..4].to_vec();
	bomb.extend((manifest.len() as u64).to_le_bytes());

	bomb.extend(&manifest);
	bomb.extend(&container[12 + manifest_len..]);
	assert!(
		InstalledBundles::open(directory.path(), &bomb, configuration())
			.err()
			.expect("declared bomb")
			.to_string()
			.contains("expanded bundle exceeds")
	);
}

#[cfg(all(feature = "http", feature = "server"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_statuses_classify_missing_failed_and_redirected_responses() {
	use axum::http::StatusCode;
	use axum::response::Redirect;
	use axum::routing::get;
	use leptoskit_codepush::installed::HttpTransport;

	let router = axum::Router::new()
		.route("/ok", get(|| async { "body" }))
		.route(
			"/broken",
			get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
		)
		.route("/moved", get(|| async { Redirect::temporary("/ok") }));
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
		.await
		.expect("bind");
	let base = format!("http://{}", listener.local_addr().expect("address"));
	let server = tokio::spawn(async move { axum::serve(listener, router).await });
	let results = tokio::task::spawn_blocking(move || {
		let mut transport = HttpTransport::new(true);
		["ok", "missing", "broken", "moved"].map(|path| transport.fetch(&format!("{base}/{path}")))
	})
	.await
	.expect("worker");
	server.abort();

	let [ok, missing, broken, moved] = results;
	assert_eq!(ok.expect("success"), b"body");
	assert!(
		matches!(missing, Err(CodePushError::NotFound(_))),
		"{missing:?}"
	);
	assert!(
		matches!(&broken, Err(CodePushError::Transport(message)) if message.contains("500")),
		"{broken:?}"
	);
	assert!(
		matches!(&moved, Err(CodePushError::Transport(message)) if message.contains("307")),
		"redirects are never followed: {moved:?}"
	);
}

#[cfg(all(feature = "http", feature = "server"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_reads_stop_at_the_requested_bound() {
	use axum::routing::get;
	use leptoskit_codepush::installed::HttpTransport;

	let router = axum::Router::new().route("/big", get(|| async { vec![b'x'; 3 * 1024 * 1024] }));
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
		.await
		.expect("bind");
	let base = format!("http://{}", listener.local_addr().expect("address"));
	let server = tokio::spawn(async move { axum::serve(listener, router).await });
	let (bounded, unbounded) = tokio::task::spawn_blocking(move || {
		let mut transport = HttpTransport::new(true);
		let url = format!("{base}/big");
		(
			transport.fetch_within(&url, 2 * 1024 * 1024),
			transport.fetch(&url),
		)
	})
	.await
	.expect("worker");
	server.abort();

	assert!(
		matches!(&bounded, Err(CodePushError::Transport(_))),
		"{:?}",
		bounded.as_ref().map(Vec::len)
	);
	assert_eq!(
		unbounded.expect("within the bundle limit").len(),
		3 * 1024 * 1024
	);
}

#[cfg(all(feature = "http", feature = "server"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_channel_without_releases_is_up_to_date_over_real_http() {
	let temporary = tempfile::tempdir().expect("fixture");
	let store =
		leptoskit_codepush::ReleaseStore::open(temporary.path().join("registry")).expect("store");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
		.await
		.expect("bind");
	let address = listener.local_addr().expect("address");
	let server = tokio::spawn(async move {
		axum::serve(listener, leptoskit_codepush::codepush_router(store)).await
	});
	let installed_path = temporary.path().join("installed");
	let mut config = configuration();

	config.server = format!("http://{address}");
	config.allow_http = true;
	let report = tokio::task::spawn_blocking(move || {
		InstalledBundles::open(&installed_path, &native_bundle(0, BASELINE), config)
			.expect("open")
			.sync()
	})
	.await
	.expect("worker");
	server.abort();
	assert_eq!(report.expect("empty channel").status, UpdateStatus::Current);
}

#[test]
fn a_new_store_build_supersedes_downloads_even_at_the_same_version() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	install(&mut installed, native_bundle(2, REPLACEMENT));
	installed
		.sync_with(&mut Registry::new(native_bundle(3, BASELINE)))
		.expect("stage");
	drop(installed);

	// Same app version and build number, different store-shipped code.
	let store_fix = native_bundle(0, REPLACEMENT);
	let mut upgraded =
		InstalledBundles::open(directory.path(), &store_fix, configuration()).expect("upgrade");
	let launch = upgraded.launch().expect("store build");
	assert_eq!(launch.wasm, REPLACEMENT);
	assert_eq!(launch.metadata.build, 0);
	assert!(!launch.metadata.candidate);
	assert_eq!(
		release_entries(directory.path()),
		0,
		"superseded downloads are removed"
	);
}

#[test]
fn only_the_working_and_staged_releases_stay_on_disk() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");

	for build in 1..=3 {
		install(&mut installed, native_bundle(build, REPLACEMENT));
	}

	installed
		.sync_with(&mut Registry::new(native_bundle(4, BASELINE)))
		.expect("stage the trapping release");
	let trapped = installed.launch().expect("trial");

	installed.reject(&trapped.metadata.receipt).expect("trap");
	installed
		.sync_with(&mut Registry::new(native_bundle(5, REPLACEMENT)))
		.expect("stage the next release");
	assert_eq!(
		release_entries(directory.path()),
		2,
		"the confirmed release and the staged one"
	);
	drop(installed);

	let mut reopened =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reopen");
	assert_eq!(release_entries(directory.path()), 2);
	let trial = reopened.launch().expect("trial");
	reopened.confirm(&trial.metadata.receipt).expect("confirm");
	drop(reopened);
	InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reopen");
	assert_eq!(release_entries(directory.path()), 1);
}

#[test]
fn deltas_cannot_rebuild_more_than_the_client_allows() {
	struct DeltaRegistry {
		index: ChannelIndex,
		delta: Vec<u8>,
		container: Vec<u8>,
	}
	impl Transport for DeltaRegistry {
		fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
			if url.ends_with("/index.json") {
				return Ok(serde_json::to_vec(&self.index)?);
			}

			if url.contains("/deltas/") {
				return Ok(self.delta.clone());
			}

			Ok(self.container.clone())
		}
	}

	let running = native_bundle(0, BASELINE);
	let target = native_bundle(1, REPLACEMENT);
	let honest = leptoskit_codepush::create_delta(&running, &target);
	// Copies the running container over and over: far more output than the
	// container it claims to rebuild.
	let hostile = leptoskit_codepush::Delta {
		ops: vec![
			leptoskit_codepush::DeltaOp::Copy {
				offset: 0,
				len: running.len() as u64,
			};
			target.len() / running.len() + 2
		],
		..honest.clone()
	};
	let client = |delta: &leptoskit_codepush::Delta, limit: u64| {
		CodePushClient::new(
			DeltaRegistry {
				index: Registry::new(target.clone()).index,
				delta: serde_json::to_vec(delta).expect("delta"),
				container: target.clone(),
			},
			"https://updates.example",
			"counter",
			"production",
			Version::new(1, 0, 0),
			0,
			running.clone(),
			TrustedKeys::new([publisher().public_key()]).expect("keys"),
		)
		.with_max_container_bytes(limit)
	};

	let generous = client(&honest, MAX_BUNDLE_BYTES)
		.download_verified()
		.expect("download")
		.expect("offer");
	assert!(generous.used_delta);
	// A delta that would rebuild more than the cap is refused before it
	// allocates, and the full download, within the cap, replaces it.
	let capped = client(&hostile, target.len() as u64)
		.download_verified()
		.expect("download")
		.expect("offer");
	assert!(!capped.used_delta);
	assert_eq!(capped.container, target);
	assert!(format!("{capped:?}").contains("container_len"));
	// A container over the cap is refused whichever way it would arrive.
	let result = client(&honest, target.len() as u64 - 1).download_verified();
	assert!(
		matches!(&result, Err(CodePushError::Malformed(message)) if message.contains("limit")),
		"{result:?}"
	);
}

#[test]
fn every_download_is_bounded_by_its_own_object_limit() {
	struct Bounded {
		index: ChannelIndex,
		delta: Vec<u8>,
		container: Vec<u8>,
		bounds: Vec<(String, u64)>,
	}
	impl Transport for Bounded {
		fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
			self.fetch_within(url, u64::MAX)
		}

		fn fetch_within(&mut self, url: &str, max_bytes: u64) -> Result<Vec<u8>, CodePushError> {
			self.bounds.push((url.to_owned(), max_bytes));

			if url.ends_with("/index.json") {
				return Ok(serde_json::to_vec(&self.index)?);
			}

			if url.contains("/deltas/") {
				return Ok(self.delta.clone());
			}

			Ok(self.container.clone())
		}
	}

	let running = native_bundle(0, BASELINE);
	let target = native_bundle(1, REPLACEMENT);
	let mut transport = Bounded {
		index: Registry::new(target.clone()).index,
		delta: b"not a delta".to_vec(),
		container: target.clone(),
		bounds: Vec::new(),
	};
	let mut client = CodePushClient::new(
		&mut transport,
		"https://updates.example",
		"counter",
		"production",
		Version::new(1, 0, 0),
		0,
		running,
		TrustedKeys::new([publisher().public_key()]).expect("keys"),
	);
	let download = client
		.download_verified()
		.expect("download")
		.expect("offer");
	assert_eq!(download.container, target);
	drop(client);
	// The unsigned index is read on every check and has the tightest bound;
	// without a configured cap, a delta or release can never be larger than
	// the biggest container a client reads.
	let bounds = &transport.bounds;
	assert_eq!(bounds.len(), 3, "{bounds:?}");
	for (url, max_bytes) in bounds {
		let expected = if url.ends_with("/index.json") {
			2 * 1024 * 1024
		} else {
			MAX_BUNDLE_BYTES
		};
		assert_eq!(*max_bytes, expected, "{url}");
	}
}

#[test]
fn a_failed_selector_write_leaves_no_temporary_file() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	// The new selector is written and synced, but cannot replace the old one.
	let selector = directory.path().join("state.json");
	fs::remove_file(&selector).expect("selector");
	fs::create_dir(&selector).expect("blocking directory");
	fs::write(selector.join("keep"), b"").expect("non-empty");

	assert!(matches!(installed.launch(), Err(CodePushError::Io(_))));
	let mut entries: Vec<_> = fs::read_dir(directory.path())
		.expect("installed")
		.map(|entry| entry.expect("entry").file_name())
		.collect();
	entries.sort();
	assert_eq!(
		entries,
		["floor.json", "installed.lock", "releases", "state.json"]
	);
}

#[test]
fn launch_falls_back_past_releases_damaged_while_running() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	let confirmed = native_bundle(1, REPLACEMENT);
	let confirmed_id = read_container(&confirmed)
		.expect("parse")
		.manifest
		.bundle_hash();
	install(&mut installed, confirmed.clone());
	let staged = native_bundle(2, BASELINE);

	let staged_id = read_container(&staged)
		.expect("parse")
		.manifest
		.bundle_hash();
	installed
		.sync_with(&mut Registry::new(staged.clone()))
		.expect("stage");
	let releases = directory.path().join("releases");

	// The staged release is damaged after it was verified: the confirmed one
	// runs, and the damaged release is refused from then on.
	fs::write(releases.join(&staged_id).join("release.lkb"), b"damaged").expect("damage");
	let launch = installed.launch().expect("confirmed release");
	assert_eq!(launch.metadata.build, 1);
	assert!(!launch.metadata.candidate);
	assert!(
		installed
			.recovery_diagnostic()
			.expect("diagnostic")
			.contains("installed release rejected")
	);
	installed
		.confirm(&launch.metadata.receipt)
		.expect("renders");
	let replay = installed
		.sync_with(&mut Registry::new(staged))
		.expect("damaged release replay");
	assert_eq!((replay.status, replay.build), (UpdateStatus::Current, 1));

	// The confirmed release is damaged too: the store baseline runs.
	fs::write(releases.join(&confirmed_id).join("release.lkb"), b"damaged").expect("damage");
	let launch = installed.launch().expect("store baseline");
	assert_eq!(launch.wasm, BASELINE);
	assert_eq!(launch.metadata.build, 0);
}

#[test]
fn the_store_entry_module_is_verified_without_an_update_store() {
	let container = native_bundle(0, REPLACEMENT);
	assert_eq!(
		leptoskit_codepush::installed::entry_module(&container).expect("entry"),
		REPLACEMENT
	);
	let mut tampered = container.clone();
	let last = tampered.len() - 1;
	tampered[last] ^= 0xff;
	assert!(leptoskit_codepush::installed::entry_module(&tampered).is_err());
	assert!(
		leptoskit_codepush::installed::entry_module(&bundle(0, Some(b"not wasm"), None))
			.expect_err("not a module")
			.to_string()
			.contains("WebAssembly")
	);
	assert!(
		leptoskit_codepush::installed::entry_module(&bundle(0, None, None))
			.expect_err("no entry")
			.to_string()
			.contains("entry is missing")
	);
}

#[test]
fn a_second_launch_abandons_an_unconfirmed_trial() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	let staged = native_bundle(1, REPLACEMENT);
	installed
		.sync_with(&mut Registry::new(staged.clone()))
		.expect("stage");
	let trial = installed.launch().expect("trial");
	assert!(trial.metadata.candidate);

	// The host launched again without reporting the trial's outcome.
	let fallback = installed.launch().expect("working release");
	assert_eq!(fallback.wasm, BASELINE);
	assert!(!fallback.metadata.candidate);
	assert!(
		installed
			.recovery_diagnostic()
			.expect("diagnostic")
			.contains("unconfirmed trial")
	);
	assert!(installed.confirm(&trial.metadata.receipt).is_err());
	assert_eq!(
		installed
			.sync_with(&mut Registry::new(staged))
			.expect("abandoned release replay")
			.status,
		UpdateStatus::Current
	);
}

// Source approvals: with a provenance policy, a host installs only releases
// that enough of its approvers rebuilt, file for file, from one commit of the
// policy's repository.

const REPOSITORY: &str = "https://github.com/example/counter";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER_COMMIT: &str = "89abcdef0123456789abcdef0123456789abcdef";

fn approver(seed: u8) -> ReleaseSigningKey {
	ReleaseSigningKey::from_seed(&[seed; 32])
}

/// The app's configuration, requiring `threshold` of two approvers.
fn approving_configuration(threshold: usize) -> UpdateConfiguration {
	UpdateConfiguration {
		provenance: Some(ProvenanceConfiguration {
			repository: REPOSITORY.to_owned(),
			approvers: vec![
				approver(31).public_key().to_hex(),
				approver(32).public_key().to_hex(),
			],
			threshold,
		}),
		..configuration()
	}
}

fn approval(
	signer: &ReleaseSigningKey,
	container: &[u8],
	repository: &str,
	commit: &str,
) -> SignedApproval {
	let manifest = read_container(container).expect("container").manifest;
	let statement = SourceApproval::new(&manifest, repository, commit).expect("statement");
	SignedApproval::sign(&statement, signer).expect("sign")
}

fn approvals_file(approvals: Vec<SignedApproval>) -> Vec<u8> {
	let mut file = ReleaseApprovals::empty();
	file.approvals = approvals;
	file.to_vec().expect("approvals file")
}

/// A registry serving `container` with approvals from the approvers seeded
/// `signers`, all naming [`COMMIT`].
fn approved_registry(container: Vec<u8>, signers: &[u8]) -> Registry {
	let approvals = signers
		.iter()
		.map(|seed| approval(&approver(*seed), &container, REPOSITORY, COMMIT))
		.collect();
	let mut registry = Registry::new(container);
	registry.approvals = Some(approvals_file(approvals));
	registry
}

fn stored_approvals(directory: &std::path::Path, container: &[u8]) -> std::path::PathBuf {
	let id = read_container(container)
		.expect("container")
		.manifest
		.bundle_hash();
	directory.join("releases").join(id).join("approvals.json")
}

#[test]
fn approved_releases_install_and_the_store_baseline_needs_no_approvals() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let replacement = native_bundle(1, REPLACEMENT);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, approving_configuration(2))
			.expect("open");
	// The store build is part of the installed host: nothing approves it.
	let original = installed.launch().expect("baseline");
	assert_eq!(original.wasm, BASELINE);
	installed.confirm(&original.metadata.receipt).expect("ok");

	let report = installed
		.sync_with(&mut approved_registry(replacement.clone(), &[31, 32]))
		.expect("approved release");
	assert_eq!(report.status, UpdateStatus::Staged);
	assert!(stored_approvals(directory.path(), &replacement).is_file());
	let trial = installed.launch().expect("trial");
	assert_eq!(trial.wasm, REPLACEMENT);
	installed.confirm(&trial.metadata.receipt).expect("healthy");
	drop(installed);

	// Offline, the stored approvals verify again.
	let mut offline =
		InstalledBundles::open(directory.path(), &baseline, approving_configuration(2))
			.expect("offline open");
	assert_eq!(offline.recovery_diagnostic(), None);
	assert_eq!(offline.launch().expect("offline").wasm, REPLACEMENT);
}

/// Syncs `registry` under a policy of two approvers, threshold `threshold`,
/// and checks that it fails without downloading or storing the container.
fn refused(threshold: usize, mut registry: Registry) -> CodePushError {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed = InstalledBundles::open(
		directory.path(),
		&baseline,
		approving_configuration(threshold),
	)
	.expect("open");
	let error = installed
		.sync_with(&mut registry)
		.expect_err("an unapproved release must not stage");
	assert!(
		registry
			.fetched
			.iter()
			.all(|url| !url.ends_with("release.lkb") && !url.contains("/deltas/")),
		"nothing is downloaded before approvals verify: {:?}",
		registry.fetched
	);
	assert_eq!(release_entries(directory.path()), 0, "nothing is stored");
	assert_eq!(installed.launch().expect("working").wasm, BASELINE);
	error
}

fn assert_unapproved(error: &CodePushError, approvals: usize, threshold: usize) {
	assert!(
		matches!(
			error,
			CodePushError::Unapproved { approvals: counted, threshold: required, .. }
				if *counted == approvals && *required == threshold
		),
		"expected {approvals} of {threshold} approvals, got {error}"
	);
}

#[test]
fn releases_without_enough_trusted_approvals_never_download() {
	let release = native_bundle(1, REPLACEMENT);

	// No approvals file at all.
	assert_unapproved(&refused(1, Registry::new(release.clone())), 0, 1);
	// One of the two required.
	assert_unapproved(&refused(2, approved_registry(release.clone(), &[31])), 1, 2);
	// One approver, twice, is still one approver.
	assert_unapproved(
		&refused(2, approved_registry(release.clone(), &[31, 31])),
		1,
		2,
	);

	let with = |approvals: Vec<SignedApproval>| {
		let mut registry = Registry::new(release.clone());
		registry.approvals = Some(approvals_file(approvals));
		registry
	};
	// The publisher can sign an approval, but it is not an approver.
	let by_publisher = approval(&publisher(), &release, REPOSITORY, COMMIT);
	assert_unapproved(&refused(1, with(vec![by_publisher])), 0, 1);
	// Built from somewhere else.
	let fork = approval(
		&approver(31),
		&release,
		"https://github.com/example/fork",
		COMMIT,
	);
	assert_unapproved(&refused(1, with(vec![fork])), 0, 1);
	// Approvals of another build, and of a release with other files.
	let other_build = approval(
		&approver(31),
		&native_bundle(2, REPLACEMENT),
		REPOSITORY,
		COMMIT,
	);
	let other_files = approval(
		&approver(32),
		&native_bundle(1, BASELINE),
		REPOSITORY,
		COMMIT,
	);
	assert_unapproved(&refused(1, with(vec![other_build, other_files])), 0, 1);
	// Statement bytes changed after signing.
	let mut tampered = approval(&approver(31), &release, REPOSITORY, COMMIT);
	tampered.statement = tampered.statement.replace(COMMIT, OTHER_COMMIT);
	assert_unapproved(&refused(1, with(vec![tampered])), 0, 1);
	// The publisher's release signature presented as an approval.
	let manifest = read_container(&release).expect("container").manifest;
	let release_signature = approver(31).sign(&manifest);
	let replayed = SignedApproval {
		statement: serde_json::to_string(
			&SourceApproval::new(&manifest, REPOSITORY, COMMIT).expect("statement"),
		)
		.expect("statement"),
		public_key: release_signature.public_key,
		signature: release_signature.signature,
	};
	assert_unapproved(&refused(1, with(vec![replayed])), 0, 1);
	// An approvals file that does not parse is an error too.
	let mut garbage = Registry::new(release);
	garbage.approvals = Some(b"not json".to_vec());
	refused(1, garbage);
}

#[test]
fn approvers_naming_different_commits_refuse_the_release() {
	let release = native_bundle(1, REPLACEMENT);
	let mut registry = Registry::new(release.clone());
	registry.approvals = Some(approvals_file(vec![
		approval(&approver(31), &release, REPOSITORY, COMMIT),
		approval(&approver(32), &release, REPOSITORY, OTHER_COMMIT),
	]));
	let error = refused(1, registry);
	assert!(
		matches!(&error, CodePushError::ConflictingApprovals { .. }),
		"{error}"
	);
}

#[test]
fn approval_files_inside_the_bundle_count_for_nothing() {
	let temporary = tempfile::tempdir().expect("bundle directory");
	let source = temporary.path().join("source");
	fs::create_dir_all(source.join("approvals")).expect("source");
	fs::write(source.join("app.wasm"), REPLACEMENT).expect("module");
	fs::write(
		source.join("runtime.json"),
		serde_json::to_vec(&contract()).expect("runtime"),
	)
	.expect("descriptor");
	// Real approvals of another release, under the names a host might look
	// for. Only the registry's approvals file counts.
	let approved = native_bundle(1, REPLACEMENT);
	let inside = approvals_file(vec![
		approval(&approver(31), &approved, REPOSITORY, COMMIT),
		approval(&approver(32), &approved, REPOSITORY, COMMIT),
	]);
	fs::write(source.join("approvals.json"), &inside).expect("approvals");
	fs::write(source.join("approvals/release.json"), &inside).expect("approvals");
	let output = temporary.path().join("app.lkb");
	bundle_directory_as(
		&source,
		&output,
		"counter",
		"production",
		Version::new(1, 0, 0),
		1,
	)
	.expect("bundle");
	let release = fs::read(output).expect("container");

	assert_unapproved(&refused(1, Registry::new(release.clone())), 0, 1);
	let mut registry = Registry::new(release);
	registry.approvals = Some(inside);
	assert_unapproved(&refused(1, registry), 0, 1);
}

/// A change to a staged release's stored approvals, and its name.
type StoredEdit = (&'static str, fn(&std::path::Path));

#[test]
fn a_staged_release_whose_stored_approvals_change_never_launches() {
	let baseline = native_bundle(0, BASELINE);
	let release = native_bundle(1, REPLACEMENT);
	let edits: [StoredEdit; 3] = [
		("altered", |path| {
			let altered = String::from_utf8(fs::read(path).expect("approvals"))
				.expect("utf-8")
				.replace(COMMIT, OTHER_COMMIT);
			fs::write(path, altered).expect("alter");
		}),
		("removed", |path| fs::remove_file(path).expect("remove")),
		("oversized", |path| {
			fs::write(path, vec![b' '; 2 * 1024 * 1024]).expect("grow");
		}),
	];

	for (edit, apply) in edits {
		let directory = tempfile::tempdir().expect("installed");
		let mut installed =
			InstalledBundles::open(directory.path(), &baseline, approving_configuration(2))
				.expect("open");
		installed
			.sync_with(&mut approved_registry(release.clone(), &[31, 32]))
			.expect("stage");
		apply(&stored_approvals(directory.path(), &release));

		// On this run, and on the next.
		let launch = installed.launch().expect("working release");
		assert_eq!(launch.wasm, BASELINE, "{edit}");
		assert!(!launch.metadata.candidate, "{edit}");
		assert!(installed.recovery_diagnostic().is_some(), "{edit}");
		drop(installed);
		let mut reopened =
			InstalledBundles::open(directory.path(), &baseline, approving_configuration(2))
				.expect("reopen");
		assert_eq!(
			reopened.launch().expect("baseline").wasm,
			BASELINE,
			"{edit}"
		);
	}

	// A confirmed release whose approvals change falls back to the baseline
	// at the next open.
	let directory = tempfile::tempdir().expect("installed");
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, approving_configuration(2))
			.expect("open");
	installed
		.sync_with(&mut approved_registry(release.clone(), &[31, 32]))
		.expect("stage");
	let trial = installed.launch().expect("trial");
	installed.confirm(&trial.metadata.receipt).expect("confirm");
	drop(installed);
	let path = stored_approvals(directory.path(), &release);
	let mut approvals =
		ReleaseApprovals::from_slice(&fs::read(&path).expect("approvals")).expect("parse");
	approvals.approvals.pop();
	fs::write(&path, approvals.to_vec().expect("bytes")).expect("drop one approval");
	let mut reopened =
		InstalledBundles::open(directory.path(), &baseline, approving_configuration(2))
			.expect("reopen");
	assert!(
		reopened
			.recovery_diagnostic()
			.expect("diagnostic")
			.contains("source approvals")
	);
	assert_eq!(reopened.launch().expect("baseline").wasm, BASELINE);
}

#[test]
fn a_store_build_that_adds_a_policy_drops_unapproved_downloads() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	install(&mut installed, native_bundle(1, REPLACEMENT));
	drop(installed);

	let mut approving =
		InstalledBundles::open(directory.path(), &baseline, approving_configuration(1))
			.expect("open with a policy");
	assert!(approving.recovery_diagnostic().is_some());
	assert_eq!(approving.launch().expect("baseline").wasm, BASELINE);
}

#[test]
fn without_a_policy_no_approvals_are_fetched_or_stored() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let release = native_bundle(1, REPLACEMENT);
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("open");
	let mut registry = Registry::new(release.clone());
	assert_eq!(
		installed.sync_with(&mut registry).expect("stage").status,
		UpdateStatus::Staged
	);
	assert!(
		registry
			.fetched
			.iter()
			.all(|url| !url.contains("/approvals/")),
		"{:?}",
		registry.fetched
	);
	assert!(!stored_approvals(directory.path(), &release).exists());
	assert_eq!(installed.launch().expect("trial").wasm, REPLACEMENT);
}

#[test]
fn policies_must_be_valid_and_independent_of_the_publisher() {
	let directory = tempfile::tempdir().expect("installed");
	let baseline = native_bundle(0, BASELINE);
	let invalid = |edit: fn(&mut ProvenanceConfiguration)| {
		let mut config = approving_configuration(1);
		edit(config.provenance.as_mut().expect("policy"));
		config
	};

	for (what, config) in [
		("zero threshold", invalid(|policy| policy.threshold = 0)),
		(
			"unreachable threshold",
			invalid(|policy| policy.threshold = 3),
		),
		("no approvers", invalid(|policy| policy.approvers.clear())),
		(
			"bad approver",
			invalid(|policy| policy.approvers.push("zz".to_owned())),
		),
		(
			"repeated approver",
			invalid(|policy| policy.approvers.push(policy.approvers[0].clone())),
		),
		(
			"not GitHub",
			invalid(|policy| policy.repository = "https://example.com/a/b".to_owned()),
		),
		(
			"publisher as approver",
			invalid(|policy| policy.approvers.push(publisher().public_key().to_hex())),
		),
	] {
		assert!(
			InstalledBundles::open(directory.path(), &baseline, config).is_err(),
			"{what} must not open"
		);
	}

	// Hosts get the policy as JSON, which rejects unknown fields.
	let mut json = serde_json::to_value(approving_configuration(1)).expect("json");
	json["provenance"]["threshold_hint"] = 1.into();
	assert!(serde_json::from_value::<UpdateConfiguration>(json).is_err());
	let mut json = serde_json::to_value(configuration()).expect("json");
	assert!(json.get("provenance").is_none(), "absent, not null");
	json["provenance"] = serde_json::Value::Null;
	assert!(
		serde_json::from_value::<UpdateConfiguration>(json)
			.expect("null policy")
			.provenance
			.is_none()
	);
}
