//! The scenario drivers. Each runs on the app's own clock and returns one
//! report; nothing here waits on UI automation.

mod echo;

use leptoskit::prelude::Element;

use crate::host::Frame;
use crate::host::Host;
use crate::host::PLATFORM;
use crate::report::Report;
use crate::routes;
use crate::services::Arena;
use crate::services::Card;
use crate::services::Phase;
use crate::services::Scenario;
use crate::services::Screen;

/// `mount` rounds.
const MOUNT_ROUNDS: usize = 5;
/// Cards created per `mount` round.
const MOUNT_CARDS: usize = 1_000;
/// How long `ticker` runs.
const TICKER_MS: f64 = 5_000.0;

/// Reads the launch scenario and runs it, or shows the home screen.
pub async fn launch(arena: Arena) {
	let host = match Host::new() {
		Ok(host) => host,
		Err(error) => return arena.update(|model| fail(model, error)),
	};

	match host.scenario().await {
		Ok(None) => arena.update(|model| model.screen = Screen::Home),
		Ok(Some(id)) => {
			match Scenario::parse(&id) {
				Some(scenario) => run(host, arena, scenario).await,
				None => arena.update(|model| fail(model, format!("unknown scenario `{id}`"))),
			}
		}
		Err(error) => arena.update(|model| fail(model, error)),
	}
}

/// Runs one scenario and writes its report to the system log.
pub async fn run(host: Host, arena: Arena, scenario: Scenario) {
	arena.update(|model| {
		model.scenario = Some(scenario);
		model.phase = Phase::Idle;
		model.screen = Screen::Starting;
	});

	let report = match scenario {
		Scenario::Cold => cold(host, &arena).await,
		Scenario::Mount => mount(host, &arena).await,
		Scenario::Echo => echo::run(host, &arena).await,
		Scenario::Ticker => ticker(host, &arena).await,
		Scenario::Autoscroll | Scenario::Busy => Err(UNSUPPORTED_SCROLL.to_owned()),
	};
	let report = report.unwrap_or_else(|error| {
		let reason = error.clone();
		arena.update(move |model| {
			model.phase = Phase::Failed;
			model.screen = Screen::Unsupported(reason);
		});
		Report::new(scenario).fail(error)
	});

	// Nowhere else to send a failure to write the report: the log is the
	// only channel, so the failure shows on screen instead.
	if let Err(error) = host.report(&report.lines(PLATFORM)).await {
		arena.update(|model| fail(model, error));
	}
}

/// Why `autoscroll` and `busy` cannot run.
const UNSUPPORTED_SCROLL: &str = "leptoskit cannot set a scroll view's offset, animated or not, \
                                  on any platform, and the generated iOS, Android, and macOS apps \
                                  report no scroll events to the guest";

fn fail(model: &mut crate::services::Model, error: String) {
	model.phase = Phase::Failed;
	model.screen = Screen::Unsupported(error);
}

/// `cold`: the ready screen and the beacon's done colour in one commit.
async fn cold(host: Host, arena: &Arena) -> Result<Report, String> {
	arena.update(|model| {
		model.screen = Screen::Ready;
		model.phase = Phase::Done;
	});
	host.next_frame().await?;

	let mut report = Report::new(Scenario::Cold);
	report.note(
		"leptoskit exposes no first-frame time relative to process start; the harness measures \
		 launch to presented frame",
	);

	if cfg!(feature = "native") {
		report.note(
			"the guest learns its scenario through an asynchronous plugin call, so the ready \
			 screen is its second commit, after an empty header",
		);
	}

	Ok(report)
}

/// `mount`: create, update, and clear 1,000 cards, five times.
async fn mount(host: Host, arena: &Arena) -> Result<Report, String> {
	let mut create = Vec::new();
	let mut update = Vec::new();
	let mut clear = Vec::new();

	arena.update(|model| model.screen = Screen::Cards(Vec::new()));
	host.next_frame().await?;

	for _ in 0..MOUNT_ROUNDS {
		create.push(
			step(host, arena, |cards| {
				*cards = (1..=MOUNT_CARDS)
					.map(|number| {
						Card {
							number,
							marked: false,
						}
					})
					.collect();
			})
			.await?,
		);
		update.push(
			step(host, arena, |cards| {
				for card in cards.iter_mut().filter(|card| card.number % 10 == 0) {
					card.marked = true;
				}
			})
			.await?,
		);
		clear.push(step(host, arena, Vec::clear).await?);
	}

	let mut report = Report::new(Scenario::Mount);
	for (key, values) in [
		("create_ms", create),
		("update_ms", update),
		("clear_ms", clear),
	] {
		report.metric(key, crate::report::median(&values));
		report.samples(key, &values);
	}
	report.note(
		"timed from the state change to the next display-frame callback after the native widgets \
		 were updated (CADisplayLink, Choreographer, or requestAnimationFrame), read back on the \
		 host clock; leptoskit has no after-commit or after-frame hook of its own",
	);
	report.note(
		"the row check reads the element tree the guest committed; the guest cannot read native \
		 widgets back",
	);

	Ok(report)
}

/// One `mount` step: a frame of the running beacon alone, then the change and
/// the done beacon in one commit, so the recording's running-to-done window
/// is the change's time to the screen. The tree is checked after the frame
/// that shows it; a mismatch fails the report.
async fn step(
	host: Host,
	arena: &Arena,
	change: impl FnOnce(&mut Vec<Card>),
) -> Result<f64, String> {
	arena.update(|model| model.phase = Phase::Running);
	host.next_frame().await?;

	let start = host.now_ms().await?;
	arena.update(|model| {
		model.phase = Phase::Done;
		if let Screen::Cards(cards) = &mut model.screen {
			change(cards);
		}
	});
	host.next_frame().await?;
	let elapsed = host.now_ms().await? - start;

	check_cards(arena)?;
	arena.update(|model| model.phase = Phase::Idle);
	host.next_frame().await?;

	Ok(elapsed)
}

/// The committed rows match the model: their count, and the first and last
/// titles.
fn check_cards(arena: &Arena) -> Result<(), String> {
	let expected = match &arena.read().screen {
		Screen::Cards(cards) => cards.iter().map(routes::cards::title).collect::<Vec<_>>(),
		_ => return Err("mount lost its cards".to_owned()),
	};
	let titles: Vec<String> = texts(&routes::screen(arena))
		.into_iter()
		.filter(|text| text.starts_with("Card "))
		.collect();

	if titles.len() != expected.len()
		|| titles.first() != expected.first()
		|| titles.last() != expected.last()
	{
		return Err(format!(
			"rendered {} rows ({:?} … {:?}), expected {} ({:?} … {:?})",
			titles.len(),
			titles.first(),
			titles.last(),
			expected.len(),
			expected.first(),
			expected.last()
		));
	}

	Ok(())
}

/// Every text in the tree, in document order.
fn texts(element: &Element) -> Vec<String> {
	let mut found = Vec::new();
	let mut stack = vec![element];

	while let Some(element) = stack.pop() {
		if let Some(text) = &element.props.text {
			found.push(text.clone());
		}
		stack.extend(element.children.iter().rev());
	}

	found
}

/// `ticker`: rewrite all 300 tiles on every frame for five seconds.
async fn ticker(host: Host, arena: &Arena) -> Result<Report, String> {
	arena.update(|model| model.screen = Screen::Ticker(0));
	let first = host.next_frame().await?;
	let mut previous = first;
	let mut intervals = Vec::new();
	let mut frames = 0_u64;
	let mut skipped = 0_u64;

	loop {
		frames += 1;
		let last = elapsed_ms(first, previous) >= TICKER_MS;
		arena.update(|model| {
			model.screen = Screen::Ticker(frames);
			model.phase = if last { Phase::Done } else { Phase::Running };
		});

		if last {
			break;
		}

		let frame = host.next_frame().await?;
		let interval = elapsed_ms(previous, frame);
		let period = if frame.period_ns > 0 {
			frame.period_ns as f64 / 1e6
		} else {
			// The browser reports no period; take the typical interval.
			intervals.push(interval);
			crate::report::median(&intervals)
		};

		if interval > 1.5 * period {
			skipped += 1;
		}

		previous = frame;
	}

	let shown = texts(&routes::screen(arena))
		.into_iter()
		.nth(1)
		.ok_or("the ticker shows no tiles")?;

	if shown != frames.to_string() {
		return Err(format!("tile 0 shows {shown}, expected {frames}"));
	}

	let mut report = Report::new(Scenario::Ticker);
	report.metric("frames_count", frames as f64);
	report.metric("skipped_count", skipped as f64);
	report.metric("final_frame", frames as f64);

	if cfg!(feature = "native") {
		report.note(
			"frames come from leptoskit::guest::next_frame, which the host resolves from \
			 CADisplayLink or Choreographer and renders within the same callback",
		);
	}

	Ok(report)
}

fn elapsed_ms(from: Frame, to: Frame) -> f64 {
	to.timestamp_ns.saturating_sub(from.timestamp_ns) as f64 / 1e6
}
