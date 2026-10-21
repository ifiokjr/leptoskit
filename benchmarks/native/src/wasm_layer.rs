//! The wasm layer: `leptoskit_wasm`'s host running the Rust-compiled guest in
//! wasmi, the interpreter leptoskit ships because iOS forbids JIT compilation.
//!
//! Every scenario goes through the real rendering ABI — JSON props written by
//! the guest, decoded and assembled by the host, committed to the surface —
//! and verifies the surface afterwards, so a guest that stopped rendering
//! cannot post a fast number.

use leptoskit_wasm::EventKind;
use leptoskit_wasm::WasmHost;

use crate::report::Config;
use crate::report::Report;
use crate::report::sample;
use crate::view::ROWS;
use crate::view::VIEWPORT;

/// Events per round-trip sample: enough that the per-event cost dominates
/// the measurement overhead, few enough to keep device runs short.
pub const ROUNDTRIP_EVENTS: usize = 100;

const ROOT_WIRE_ID: i64 = 1;
const FIRST_ROW_WIRE_ID: i64 = 2;

/// Runs every wasm-layer scenario into `report`, or records why not.
pub fn run(report: &mut Report, config: Config, guest: Option<&[u8]>) -> Result<(), String> {
	let Some(bytes) = guest else {
		report.note(
			"wasm layer skipped: no guest embedded (build with LEPTOSKIT_BENCH_GUEST_WASM set)",
		);
		return Ok(());
	};

	let instantiate = sample(
		config,
		|| (),
		|()| {
			std::hint::black_box(
				WasmHost::headless(bytes).expect("the embedded guest instantiates"),
			);
		},
	);
	report.duration(
		"wasm.instantiate",
		"wasm: compile and instantiate the guest module",
		instantiate,
	);

	let start = sample(
		config,
		|| WasmHost::headless(bytes).expect("the embedded guest instantiates"),
		|host| {
			host.start(VIEWPORT.0, VIEWPORT.1)
				.expect("the guest starts");
		},
	);
	report.duration(
		"wasm.start-1000",
		"wasm: guest builds 1,000 rows through the ABI and the host commits them",
		start,
	);

	let mut started = || {
		let mut host = WasmHost::headless(bytes).expect("the embedded guest instantiates");
		host.start(VIEWPORT.0, VIEWPORT.1)
			.expect("the guest starts");
		host
	};
	verify_rows(&started(), ROWS)?;

	let roundtrip = sample(config, &mut started, |host| {
		for _ in 0..ROUNDTRIP_EVENTS {
			host.dispatch_wire(ROOT_WIRE_ID, EventKind::Press, None)
				.expect("the guest handles a press");
		}
	});
	report.throughput(
		"wasm.event-throughput",
		"wasm: press → guest update → commit round trips per second",
		ROUNDTRIP_EVENTS as f64,
		&roundtrip,
	);
	report.duration(
		"wasm.event-roundtrip-100",
		"wasm: 100 press round trips, one label updated each",
		roundtrip,
	);
	{
		let mut host = started();
		host.dispatch_wire(ROOT_WIRE_ID, EventKind::Press, None)
			.map_err(|error| format!("wasm press: {error}"))?;
		verify_label(&host, 0, "Row 0 · 1")?;
	}

	let rerender = sample(config, &mut started, |host| {
		host.dispatch_wire(FIRST_ROW_WIRE_ID, EventKind::Press, None)
			.expect("the guest re-renders");
	});
	report.duration(
		"wasm.rerender-1000",
		"wasm: guest rewrites all 1,000 labels and the host diffs them",
		rerender,
	);
	{
		let mut host = started();
		host.dispatch_wire(FIRST_ROW_WIRE_ID, EventKind::Press, None)
			.map_err(|error| format!("wasm re-render: {error}"))?;
		verify_label(&host, ROWS - 1, &format!("Row {} · gen 1", ROWS - 1))?;
	}
	Ok(())
}

fn verify_rows(
	host: &WasmHost<leptoskit_native::HeadlessBackend>,
	expected: usize,
) -> Result<(), String> {
	let surface = host.surface();
	let root = surface.root().ok_or("wasm guest committed no root")?;
	let rows = surface.children(root).len();
	if rows != expected {
		return Err(format!(
			"wasm guest committed {rows} rows, expected {expected}"
		));
	}
	Ok(())
}

fn verify_label(
	host: &WasmHost<leptoskit_native::HeadlessBackend>,
	row: usize,
	expected: &str,
) -> Result<(), String> {
	let surface = host.surface();
	let root = surface.root().ok_or("wasm guest committed no root")?;
	let row_node = *surface
		.children(root)
		.get(row)
		.ok_or_else(|| format!("wasm guest has no row {row}"))?;
	let label = *surface
		.children(row_node)
		.first()
		.ok_or_else(|| format!("wasm row {row} has no label"))?;
	let text = surface.props(label).text.clone().unwrap_or_default();
	if text != expected {
		return Err(format!(
			"wasm row {row} reads {text:?}, expected {expected:?}"
		));
	}
	Ok(())
}
