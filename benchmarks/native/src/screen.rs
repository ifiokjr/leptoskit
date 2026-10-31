//! The screen layer: the arena's screens, rendered by a Rust guest through the
//! element-tree adapter a shipped app uses.
//!
//! One frame of the ticker rewrites 300 labels, and creating the `mount` cards
//! adds 1,000. Both are dominated by interpreted work, so each scenario
//! reports the interpreter instructions the entry points consumed alongside
//! its wall time: instructions decide a device's frame budget, and they do not
//! depend on the machine or its load.

use leptoskit_wasm::EventKind;
use leptoskit_wasm::WasmHost;

use crate::report::Config;
use crate::report::Report;
use crate::view::VIEWPORT;

/// Root wire id the guest's own tree assigns to the screen's root.
const ROOT_WIRE_ID: i64 = 1;

type Host = WasmHost<leptoskit_native::HeadlessBackend>;

/// Runs the screen scenarios into `report`, or records why not.
pub fn run(report: &mut Report, config: Config, guest: Option<&[u8]>) {
	let Some(bytes) = guest else {
		report.note(
			"screen layer skipped: no guest embedded (build with LEPTOSKIT_BENCH_SCREEN_WASM set)",
		);
		return;
	};

	let mut mounted = || {
		let mut host = Host::headless(bytes).expect("the embedded screen guest instantiates");
		host.start(VIEWPORT.0, VIEWPORT.1)
			.expect("the screen guest starts");

		host
	};

	let frame = measure(config, &mut mounted, |host| step(host, EventKind::Press));
	report.duration(
		"screen.ticker-frame",
		"one ticker frame: 300 labels rewritten through the element adapter",
		frame.wall.clone(),
	);
	report.count(
		"screen.ticker-frame-fuel",
		frame.fuel.clone(),
		"interpreter instructions for one ticker frame",
	);

	let mutations = {
		let mut host = mounted();
		// The mount's own mutations are not a frame's.
		host.surface_mut().backend_mut().take_log();
		host.dispatch_wire(ROOT_WIRE_ID, EventKind::Press, None)
			.expect("the ticker ticks");

		host.surface_mut().backend_mut().take_log().len() as f64
	};
	report.count(
		"screen.ticker-mutations",
		vec![mutations],
		"backend mutations queued by one ticker frame",
	);

	let create = measure(config, &mut mounted, |host| step(host, EventKind::Change));
	report.duration(
		"screen.cards-create-1000",
		"create 1,000 cards (2,000 labels) through the element adapter",
		create.wall.clone(),
	);
	report.count(
		"screen.cards-create-fuel",
		create.fuel.clone(),
		"interpreter instructions to create 1,000 cards",
	);

	let update = measure(config, &mut mounted, |host| {
		step(host, EventKind::Change) + step(host, EventKind::Submit)
	});
	report.duration(
		"screen.cards-update-1000",
		"create 1,000 cards, then mark every tenth",
		update.wall.clone(),
	);
	report.count(
		"screen.cards-update-fuel",
		update.fuel.clone(),
		"interpreter instructions to create and mark 1,000 cards",
	);

	let clear = measure(config, &mut mounted, |host| {
		step(host, EventKind::Change) + step(host, EventKind::Focus)
	});
	report.duration(
		"screen.cards-clear-1000",
		"create 1,000 cards, then clear them",
		clear.wall.clone(),
	);
	report.count(
		"screen.cards-clear-fuel",
		clear.fuel.clone(),
		"interpreter instructions to create and clear 1,000 cards",
	);
}

/// Delivers one step event and returns the instructions it consumed.
fn step(host: &mut Host, kind: EventKind) -> u64 {
	host.dispatch_wire(ROOT_WIRE_ID, kind, None)
		.expect("the screen guest handles the step");

	host.last_fuel_used()
}

/// One scenario's wall time and interpreter instructions.
struct Fuel {
	wall: Vec<f64>,
	fuel: Vec<f64>,
}

/// Times `timed` on a fresh host per sample and records the fuel it consumed,
/// so setup work stays out of both numbers.
fn measure(
	config: Config,
	setup: &mut impl FnMut() -> Host,
	timed: impl Fn(&mut Host) -> u64,
) -> Fuel {
	let mut wall = Vec::with_capacity(config.runs);
	let mut fuel = Vec::with_capacity(config.runs);

	for iteration in 0..(config.warmup + config.runs) {
		let mut host = setup();
		let start = std::time::Instant::now();

		let used = timed(&mut host);

		let elapsed = start.elapsed().as_secs_f64() * 1000.0;
		drop(host);

		if iteration >= config.warmup {
			wall.push(elapsed);
			fuel.push(used as f64);
		}
	}

	Fuel { wall, fuel }
}
