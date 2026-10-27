//! The platform probe: one binary that proves the leptoskit stack runs on a
//! target OS — the layout engine, the native renderer's diff and commit
//! pipeline, the on-device wasm runtime, and a full code push update over the
//! real HTTP surface.
//!
//! The same `run_all` executes on the host (desktop validation), the iOS
//! simulator, and an Android emulator, so every platform a release claims to
//! support has the same evidence. Each probe prints `PASS <name>` or
//! `FAIL <name>: <error>` and the process exits non-zero on any failure.

// Layout outputs are exact floats: expected values are whole numbers the
// engine computes with the same arithmetic, not measured quantities.
#![allow(clippy::float_cmp)]

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering as AtomicOrdering;

use leptoskit::prelude::Button;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;
use leptoskit_codepush::CodePushClient;
use leptoskit_codepush::CodePushError;
use leptoskit_codepush::ReleaseSigningKey;
use leptoskit_codepush::ReleaseStore;
use leptoskit_codepush::Transport;
use leptoskit_codepush::TrustedKeys;
use leptoskit_codepush::codepush_router;
use leptoskit_native::EventOutcome;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeSurface;
use leptoskit_wasm::EventKind;
use leptoskit_wasm::WasmHost;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Display;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::NodeId;
use leptoskit_yoga::Size;
use leptoskit_yoga::Style;
use semver::Version;

/// A counter app in WebAssembly text format, implementing the leptoskit ABI:
/// a view holding a label and an increment button. Pressing the button via
/// the host rewrites the label and commits, exactly as a code-pushed bundle
/// would.
const COUNTER_WAT: &str = r#"
(module
  (import "leptoskit" "node_create" (func $node_create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $node_set_props (param i64 i32 i32)))
  (import "leptoskit" "node_insert" (func $node_insert (param i64 i64 i32)))
  (import "leptoskit" "node_remove" (func $node_remove (param i64 i64)))
  (import "leptoskit" "node_delete" (func $node_delete (param i64)))
  (import "leptoskit" "commit" (func $commit))
  (global $count (mut i64) (i64.const 0))
  (memory (export "memory") 1)
  ;; `{"text":"Count: 0"}` — 19 bytes; the digit sits at offset 80.
  (data (i32.const 64) "{\"text\":\"Count: 0\"}")
  (func $build
    (call $node_create (i64.const 1) (i32.const 0))
    (call $node_create (i64.const 2) (i32.const 1))
    (call $node_create (i64.const 3) (i32.const 6))
    (call $node_set_props (i64.const 2) (i32.const 64) (i32.const 19))
    (call $node_insert (i64.const 1) (i64.const 2) (i32.const 0))
    (call $node_insert (i64.const 1) (i64.const 3) (i32.const 1))
    (call $commit))
  (func (export "leptoskit_main") (call $build))
  (func (export "leptoskit_event")
    (param $id i64) (param $kind i32) (param $ptr i32) (param $len i32) (result i32)
    (if (i32.and
          (i64.eq (local.get $id) (i64.const 3))
          (i32.eq (local.get $kind) (i32.const 0)))
      (then
        (global.set $count (i64.add (global.get $count) (i64.const 1)))
        (i32.store8
          (i32.const 80)
          (i32.wrap_i64 (i64.add (i64.const 48) (global.get $count))))
        (call $node_set_props (i64.const 2) (i32.const 64) (i32.const 19))
        (call $commit)
        (return (i32.const 1))))
    (i32.const 0))
)
"#;

/// A scratch directory unique to this run, removed when dropped.
///
/// The process id separates concurrent probe processes, and the timestamp
/// separates runs within one process (the test harness may call `run_all`
/// more than once).
struct StateDir(PathBuf);

impl StateDir {
	fn create() -> Result<Self, String> {
		let nanos = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map_err(|error| format!("clock before the epoch: {error}"))?
			.as_nanos();
		let path = std::env::temp_dir().join(format!(
			"leptoskit_platform_probe-{}-{nanos}",
			std::process::id()
		));
		std::fs::create_dir_all(&path)
			.map_err(|error| format!("state dir {}: {error}", path.display()))?;
		Ok(Self(path))
	}

	fn path(&self) -> &Path {
		&self.0
	}
}

impl Drop for StateDir {
	fn drop(&mut self) {
		// Best-effort cleanup of temporary files; a leftover directory in the
		// OS temp dir is harmless and must not turn a pass into a failure.
		let _ = std::fs::remove_dir_all(&self.0);
	}
}

/// One probe: `Ok` on success, or a message saying what went wrong.
type Probe = fn() -> Result<(), String>;

/// Runs every probe, printing one `PASS` or `FAIL` line per probe, and returns
/// the number of probes that failed.
///
/// A probe that panics is reported as a `FAIL` line like any other failure,
/// so one broken probe never hides the results of the rest.
pub fn run_all() -> usize {
	let probes: [(&str, Probe); 5] = [
		("yoga:flexbox", yoga_flexbox),
		("yoga:grid", yoga_grid),
		("native:diff-and-events", native_diff_and_events),
		("wasm:abi-runtime", wasm_abi_runtime),
		("codepush:delta-update", codepush_delta_update),
	];
	probes
		.into_iter()
		.filter(|(name, body)| !probe(name, *body))
		.count()
}

/// Runs one probe and prints its outcome, returning whether it passed.
fn probe(name: &str, body: Probe) -> bool {
	let outcome = std::panic::catch_unwind(body).unwrap_or_else(|panic| {
		let message = panic
			.downcast_ref::<&str>()
			.map(ToString::to_string)
			.or_else(|| panic.downcast_ref::<String>().cloned())
			.unwrap_or_else(|| "non-string panic payload".to_owned());
		Err(format!("panicked: {message}"))
	});
	match outcome {
		Ok(()) => {
			println!("PASS {name}");
			true
		}
		Err(error) => {
			println!("FAIL {name}: {error}");
			false
		}
	}
}

/// The child at `index` in `children`, or a probe failure naming the parent
/// (`what`) and how many children it actually has.
fn nth_child(children: &[NodeId], index: usize, what: &str) -> Result<NodeId, String> {
	children.get(index).copied().ok_or_else(|| {
		format!(
			"{what}: expected a child at {index}, found {} children",
			children.len()
		)
	})
}

fn yoga_flexbox() -> Result<(), String> {
	let mut tree = leptoskit_yoga::Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			flex_direction: FlexDirection::Row,
			width: Dimension::Points(100.0),
			..Style::default()
		},
	);
	for grow in [1.0, 3.0] {
		let child = tree.create_node();
		tree.set_style(
			child,
			Style {
				flex_grow: grow,
				..Style::default()
			},
		);
		tree.insert_child(root, child, tree.children(root).len());
	}
	tree.calculate_layout(
		root,
		Size {
			width: AvailableSpace::Definite(100.0),
			height: AvailableSpace::Definite(50.0),
		},
	);
	let first = tree.layout(nth_child(tree.children(root), 0, "flex root")?);
	let second = tree.layout(nth_child(tree.children(root), 1, "flex root")?);
	if first.width != 25.0 || second.width != 75.0 || second.x != 25.0 {
		return Err(format!(
			"flex grow split wrong: {}@{}, {} wide",
			first.width, second.x, second.width
		));
	}
	Ok(())
}

fn yoga_grid() -> Result<(), String> {
	let mut tree = leptoskit_yoga::Yoga::new();
	let root = tree.create_node();
	let mut style = Style {
		display: Display::Grid,
		width: Dimension::Points(300.0),
		..Style::default()
	};
	style.grid = Some(leptoskit_yoga::GridStyle {
		template_columns: vec![
			leptoskit_yoga::TrackSizing::Fraction(1.0),
			leptoskit_yoga::TrackSizing::Fraction(2.0),
		],
		..leptoskit_yoga::GridStyle::default()
	});
	tree.set_style(root, style);
	for _ in 0..2 {
		let child = tree.create_node();
		tree.set_style(child, Style::default());
		tree.insert_child(root, child, tree.children(root).len());
	}
	tree.calculate_layout(
		root,
		Size {
			width: AvailableSpace::Definite(300.0),
			height: AvailableSpace::Definite(100.0),
		},
	);
	let second = tree.layout(nth_child(tree.children(root), 1, "grid root")?);
	if second.x != 100.0 || second.width != 200.0 {
		return Err(format!(
			"fr tracks wrong: x={}, w={}",
			second.x, second.width
		));
	}
	Ok(())
}

fn native_diff_and_events() -> Result<(), String> {
	let presses = Arc::new(AtomicUsize::new(0));
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(390.0, 844.0);

	let build = |count: u32, presses: Arc<AtomicUsize>| {
		View()
			.class("flex-col items-stretch")
			.child(Text(format!("Count: {count}")))
			.child(
				Button("Increment").on(leptoskit_native::NativeEventSlot::Press, move |_| {
					presses.fetch_add(1, AtomicOrdering::SeqCst);
					EventOutcome::Handled
				}),
			)
	};

	surface.set_root(&build(0, Arc::clone(&presses)));
	surface.backend_mut().clear();
	surface.set_root(&build(1, Arc::clone(&presses)));

	let log = surface.backend().log();
	let creates = log
		.iter()
		.filter(|mutation| matches!(mutation, leptoskit_native::Mutation::Create { .. }))
		.count();
	if creates != 0 {
		return Err(format!("rebuild recreated {creates} widgets; diff failed"));
	}

	let root = surface.root().ok_or("no root committed")?;
	let button = nth_child(surface.children(root), 1, "native root")?;
	let outcome = surface.dispatch_event(button, &NativeEvent::Press);
	if outcome != EventOutcome::Handled {
		return Err("press event was not handled".to_owned());
	}
	if presses.load(AtomicOrdering::SeqCst) != 1 {
		return Err("press handler did not run".to_owned());
	}
	Ok(())
}

fn wasm_abi_runtime() -> Result<(), String> {
	// Compiling wat here keeps the probe self-contained: the bundle the host
	// runs is produced on-device, like a freshly downloaded code push.
	let bytes = wat::parse_str(COUNTER_WAT).map_err(|error| format!("wat compile: {error}"))?;

	let mut host = WasmHost::headless(&bytes).map_err(|error| format!("host: {error}"))?;
	host.start(320.0, 480.0)
		.map_err(|error| format!("start: {error}"))?;

	let root = {
		let surface = host.surface();
		surface.root().ok_or("no root after start")?
	};
	let label = nth_child(host.surface().children(root), 0, "wasm root")?;
	if host.surface().props(label).text.as_deref() != Some("Count: 0") {
		return Err("initial label wrong".to_owned());
	}

	let outcome = host
		.dispatch(label, &NativeEvent::Press)
		.map_err(|error| format!("dispatch: {error}"))?;
	if outcome != EventOutcome::Unhandled {
		return Err("the label must not consume a press".to_owned());
	}

	let outcome = host
		.dispatch_wire(3, EventKind::Press, None)
		.map_err(|error| format!("dispatch_wire: {error}"))?;
	if outcome != EventOutcome::Handled {
		return Err("the module did not handle the button press".to_owned());
	}
	if host.commits() != 2 {
		return Err(format!("expected 2 commits, saw {}", host.commits()));
	}
	if host.surface().props(label).text.as_deref() != Some("Count: 1") {
		return Err("label did not update after the wasm-side rebuild".to_owned());
	}
	Ok(())
}

/// Drives the client's HTTP fetches through the real axum router on a
/// dedicated thread: the probe runs outside tokio, and `Transport::fetch` is
/// synchronous.
#[derive(Clone)]
struct RouterTransport {
	router: axum::Router,
}

impl Transport for RouterTransport {
	fn fetch(&mut self, url: &str) -> Result<Vec<u8>, CodePushError> {
		std::thread::scope(|scope| {
			let router = self.router.clone();
			let url = url.to_owned();
			scope
				.spawn(move || {
					let runtime = tokio::runtime::Runtime::new()
						.map_err(|error| CodePushError::Transport(format!("runtime: {error}")))?;
					runtime.block_on(async move {
						use tower::util::ServiceExt;
						// Absolute URLs carry a host the in-process router
						// does not route on; split the path-and-query off
						// and send only that.
						let after_scheme =
							url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
						let path_and_query = after_scheme
							.split_once('/')
							.map_or(after_scheme, |(_, path)| path);
						let request = axum::http::Request::builder()
							.uri(format!("/{path_and_query}"))
							.body(axum::body::Body::empty())
							.map_err(|error| {
								CodePushError::Transport(format!("request: {error}"))
							})?;
						let response = router.oneshot(request).await.map_err(|error| {
							CodePushError::Transport(format!("router: {error}"))
						})?;
						let status = response.status();
						let bytes = http_body_util::BodyExt::collect(response.into_body())
							.await
							.map_err(|error| CodePushError::Transport(format!("body: {error}")))?
							.to_bytes();
						if status == axum::http::StatusCode::NOT_FOUND {
							return Err(CodePushError::NotFound(url));
						}
						if !status.is_success() {
							return Err(CodePushError::Transport(format!("{url}: {status}")));
						}
						Ok(bytes.to_vec())
					})
				})
				.join()
				.map_err(|_| CodePushError::Transport("the fetch thread panicked".to_owned()))?
		})
	}
}

/// Writes a bundle whose module starts with `marker` and continues with the
/// same 256 KiB in every version: varied enough that it compresses like real
/// code, not like one repeated byte, so the compressed chunks after the
/// marker are identical between versions and the delta copies them.
fn write_bundle(dir: &Path, marker: &str) -> Result<(), String> {
	let io = |error: std::io::Error| format!("bundle {}: {error}", dir.display());
	std::fs::create_dir_all(dir).map_err(io)?;
	let mut state = 0x9e37_79b9_u64;
	let mut wasm = marker.as_bytes().to_vec();
	wasm.extend((0..256 * 1024).map(|_| {
		state ^= state << 13;
		state ^= state >> 7;
		state ^= state << 17;
		b"\0asm local.get call"[(state % 19) as usize]
	}));
	std::fs::write(dir.join("app.wasm"), &wasm).map_err(io)?;
	std::fs::write(dir.join("index.txt"), marker).map_err(io)
}

fn codepush_delta_update() -> Result<(), String> {
	let state_dir = StateDir::create()?;
	let state = state_dir.path();

	let store = ReleaseStore::open(state.join("releases")).map_err(|e| e.to_string())?;
	let base_dir = state.join("v0.9.0");
	let next_dir = state.join("v1.0.0");
	write_bundle(&base_dir, "probe-base-marker")?;
	write_bundle(&next_dir, "probe-next-marker-with-changes")?;
	// The probe plays both publisher and device, so it holds both halves of
	// one key.
	let signer = ReleaseSigningKey::from_seed(&[0x50; 32]);
	let trusted = TrustedKeys::new([signer.public_key()]).map_err(|e| e.to_string())?;

	store
		.ingest_directory(
			&signer,
			"probe",
			"production",
			&Version::new(0, 9, 0),
			1,
			&base_dir,
			false,
		)
		.map_err(|e| e.to_string())?;
	let base = store
		.latest("probe", "production")
		.map_err(|e| e.to_string())?
		.ok_or("base release missing")?;

	store
		.ingest_directory(
			&signer,
			"probe",
			"production",
			&Version::new(1, 0, 0),
			2,
			&next_dir,
			false,
		)
		.map_err(|e| e.to_string())?;

	let router = codepush_router(store.clone());
	let mut client = CodePushClient::new(
		RouterTransport { router },
		"https://updates.leptoskit.test",
		"probe",
		"production",
		Version::new(0, 9, 0),
		1,
		base.container,
		trusted,
	);

	let report = client
		.sync(&state.join("staged"))
		.map_err(|error| format!("sync: {error}"))?;
	if !report.updated {
		return Err("the update did not install".to_owned());
	}
	if !report.used_delta {
		return Err("the update arrived as a full bundle, not a delta".to_owned());
	}
	if report.version != Some(Version::new(1, 0, 0)) {
		return Err(format!(
			"updated to the wrong version: {:?}",
			report.version
		));
	}

	let staged = std::fs::read(state.join("staged").join("app.wasm")).map_err(|e| e.to_string())?;
	if !staged.starts_with(b"probe-next-marker-with-changes") {
		return Err("the staged bytes are not the v1.0.0 bundle".to_owned());
	}
	Ok(())
}
