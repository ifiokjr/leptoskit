//! The tap a person feels: pressing the starter counter's button until the
//! host has committed the new count and encoded the changes for the shell,
//! with the counter guest built from the same components and guest runtime
//! as a new app.
//!
//! Besides the comparison against the base branch, a tap has a hard budget:
//! it must land within one 60 Hz frame, or the run fails.

use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_wasm::WasmHost;
use leptoskit_yoga::NodeId;

use crate::report::Config;
use crate::report::Report;
use crate::report::sample;
use crate::view::VIEWPORT;

/// Taps per sample: enough that a tap's cost dominates the timer.
const TAPS: usize = 100;

/// One frame at 60 Hz. A slower tap shows as lag on every platform.
const FRAME_BUDGET_MS: f64 = 1000.0 / 60.0;

/// Runs the tap scenarios into `report`, or records why not.
pub fn run(report: &mut Report, config: Config, guest: Option<&[u8]>) -> Result<(), String> {
	let Some(bytes) = guest else {
		report.note(
			"tap scenarios skipped: no counter guest embedded (build with \
			 LEPTOSKIT_BENCH_COUNTER_WASM set)",
		);
		return Ok(());
	};

	let started = || -> (WasmHost<HeadlessBackend>, NodeId) {
		let mut host = WasmHost::headless(bytes).expect("the counter guest instantiates");
		host.start(VIEWPORT.0, VIEWPORT.1)
			.expect("the counter guest starts");
		host.take_mutations();
		let button = button(&host).expect("the counter has a button");
		(host, button)
	};

	{
		let (mut host, button) = started();
		for _ in 0..3 {
			tap(&mut host, button);
		}
		verify_count(&host, 3)?;
	}

	let taps = sample(config, started, |(host, button)| {
		for _ in 0..TAPS {
			tap(host, *button);
		}
	});
	let per_tap = median(&taps) / TAPS as f64;
	report.duration(
		"tap.counter-100",
		"tap: 100 taps on the starter counter, each committed and encoded for the shell",
		taps.clone(),
	);
	report.throughput(
		"tap.counter-throughput",
		"tap: starter counter taps committed per second",
		TAPS as f64,
		&taps,
	);

	if per_tap > FRAME_BUDGET_MS {
		return Err(format!(
			"a tap on the starter counter takes {per_tap:.2} ms, over the {FRAME_BUDGET_MS:.2} ms \
			 frame budget"
		));
	}

	Ok(())
}

/// Presses the button, lets the guest re-render, and encodes the committed
/// changes the way the shell receives them.
fn tap(host: &mut WasmHost<HeadlessBackend>, button: NodeId) {
	host.dispatch(button, &NativeEvent::Press)
		.expect("the counter handles a press");
	std::hint::black_box(
		serde_json::to_vec(&host.take_mutations()).expect("mutations encode as JSON"),
	);
}

fn nodes(host: &WasmHost<HeadlessBackend>) -> Vec<NodeId> {
	let surface = host.surface();
	let mut found = Vec::new();
	let mut stack: Vec<NodeId> = surface.root().into_iter().collect();

	while let Some(node) = stack.pop() {
		found.push(node);
		stack.extend(surface.children(node).iter().copied());
	}

	found
}

fn button(host: &WasmHost<HeadlessBackend>) -> Option<NodeId> {
	nodes(host)
		.into_iter()
		.find(|node| host.surface().kind(*node) == NativeKind::Button)
}

fn verify_count(host: &WasmHost<HeadlessBackend>, count: usize) -> Result<(), String> {
	let expected = format!("Count: {count}");
	let shown = nodes(host)
		.into_iter()
		.any(|node| host.surface().props(node).text.as_deref() == Some(expected.as_str()));

	if shown {
		Ok(())
	} else {
		Err(format!(
			"the counter does not show {expected:?} after {count} taps"
		))
	}
}

fn median(values: &[f64]) -> f64 {
	let mut sorted = values.to_vec();
	sorted.sort_by(f64::total_cmp);
	sorted.get(sorted.len() / 2).copied().unwrap_or(0.0)
}
