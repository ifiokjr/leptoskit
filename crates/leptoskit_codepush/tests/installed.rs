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
	// An interrupted selector write leaves only its temporary file behind.
	fs::write(directory.path().join("state.json.pending"), b"{\"torn").expect("torn write");
	let mut installed =
		InstalledBundles::open(directory.path(), &baseline, configuration()).expect("reopen");
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
	assert!(
		recovered
			.sync_with(&mut Registry::new(staged))
			.expect_err("a release that failed on disk stays refused")
			.to_string()
			.contains("previously failed")
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
	let registry = || {
		let mut registry = Registry::new(target.clone());
		DeltaRegistry {
			index: std::mem::replace(
				&mut registry.index,
				ChannelIndex {
					format_version: INDEX_FORMAT_VERSION,
					manifest: read_container(&target).expect("target").manifest,
					signature: None,
				},
			),
			delta: serde_json::to_vec(&leptoskit_codepush::create_delta(&running, &target))
				.expect("delta"),
			container: target.clone(),
		}
	};
	let client = |limit: u64| {
		CodePushClient::new(
			registry(),
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

	let generous = client(MAX_BUNDLE_BYTES)
		.download_verified()
		.expect("download")
		.expect("offer");
	assert!(generous.used_delta);
	// A delta that would rebuild more than the cap falls back to the full,
	// transport-bounded download instead of allocating.
	let capped = client(target.len() as u64 - 1)
		.download_verified()
		.expect("download")
		.expect("offer");
	assert!(!capped.used_delta);
	assert_eq!(capped.container, target);
	assert!(format!("{capped:?}").contains("container_len"));
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
	assert!(!directory.path().join("state.json.pending").exists());
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
	assert!(
		installed
			.sync_with(&mut Registry::new(staged))
			.expect_err("damaged release replay")
			.to_string()
			.contains("previously failed")
	);

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
	assert!(
		installed
			.sync_with(&mut Registry::new(staged))
			.expect_err("abandoned release replay")
			.to_string()
			.contains("previously failed")
	);
}
