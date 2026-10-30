//! `autoscroll` and `busy`: the feed scrolled by app logic, and scrolled by
//! the platform while app logic blocks.

use leptoskit::native::ScrollOffset;

use super::elapsed_ms;
use crate::host::Host;
use crate::report::Report;
use crate::routes::feed::ITEM_HEIGHT;
use crate::services::Arena;
use crate::services::Feed;
use crate::services::Phase;
use crate::services::Scenario;
use crate::services::Screen;

/// How long `autoscroll` runs.
const AUTOSCROLL_MS: f64 = 5_000.0;
/// `autoscroll`'s speed, in points per millisecond.
const SPEED: f64 = 1.5;
/// Frames the last offset may take to come back as a scroll event.
const SETTLE_FRAMES: usize = 60;
/// The item `busy` scrolls to.
const BUSY_INDEX: f64 = 1_500.0;
/// How long `busy` blocks the guest each time.
const BLOCK_MS: f64 = 40.0;
/// How often `busy` blocks the guest.
const BLOCK_EVERY_MS: f64 = 100.0;
/// How long `busy` waits for the scroll to arrive.
const BUSY_TIMEOUT_MS: f64 = 15_000.0;

/// `autoscroll`: on every frame for 5,000 ms, scroll without animation to
/// `offset += 1.5 × dt`, then wait for the list to report the last offset.
pub async fn autoscroll(host: Host, arena: &Arena) -> Result<Report, String> {
	show_feed(host, arena).await?;

	let start = host.next_frame().await?;
	let mut previous = start;
	let mut frame = start;
	let mut frames = 0_u64;
	let mut offset = 0.0;

	loop {
		offset += SPEED * elapsed_ms(previous, frame);
		previous = frame;
		frames += 1;
		let done = elapsed_ms(start, frame) >= AUTOSCROLL_MS;
		arena.update(|model| {
			model.phase = if done { Phase::Done } else { Phase::Running };
			if let Screen::Feed(feed) = &mut model.screen {
				// The window moves with the target in the same commit.
				feed.offset = offset;
				feed.target = Some(ScrollOffset::new(0.0, offset as f32));
			}
		});

		if done {
			break;
		}

		frame = host.next_frame().await?;
	}

	settle(host, arena, offset).await?;
	let mut report = Report::new(Scenario::Autoscroll);
	report.metric("frames_count", frames as f64);
	report.metric("final_offset", (offset * 1_000.0).round() / 1_000.0);
	report.note(
		"the guest sets the list's scroll offset on each leptoskit::guest::next_frame, with the \
		 window for that offset, in one commit",
	);

	Ok(report)
}

/// `busy`: the platform's animated scroll to item 1,500 while the guest
/// spins for 40 ms of every 100 ms.
pub async fn busy(host: Host, arena: &Arena) -> Result<Report, String> {
	show_feed(host, arena).await?;

	let target = BUSY_INDEX * ITEM_HEIGHT;
	let spins_per_ms = calibrate(host).await?;
	let start = host.now_ms().await?;
	let mut next_block = start + BLOCK_EVERY_MS;
	let mut blocks = 0_u64;
	let mut blocked_ms = 0.0;
	arena.update(|model| {
		model.phase = Phase::Running;
		if let Screen::Feed(feed) = &mut model.screen {
			feed.target = Some(ScrollOffset::new(0.0, target as f32).animated());
		}
	});

	while (reported(arena)? - target).abs() > 1.0 {
		host.next_frame().await?;
		let now = host.now_ms().await?;

		if now - start > BUSY_TIMEOUT_MS {
			return Err(format!(
				"the scroll stopped at {}, expected {target}",
				reported(arena)?
			));
		}

		if now >= next_block {
			spin((BLOCK_MS * spins_per_ms) as u64);
			blocked_ms += host.now_ms().await? - now;
			blocks += 1;
			next_block += BLOCK_EVERY_MS;
		}
	}

	arena.update(|model| model.phase = Phase::Done);
	host.next_frame().await?;
	let mut report = Report::new(Scenario::Busy);
	report.metric("blocks_count", blocks as f64);
	report.note(format!(
		"leptoskit runs app logic in the guest on the UI thread, so each block stops the \
		 platform's scroll animation too; the guest spun a calibrated loop for {:.1} ms per block",
		if blocks == 0 {
			0.0
		} else {
			blocked_ms / blocks as f64
		}
	));

	Ok(report)
}

/// Shows the feed at the top, and lets its first window mount before the
/// clock starts.
async fn show_feed(host: Host, arena: &Arena) -> Result<(), String> {
	arena.update(|model| model.screen = Screen::Feed(Feed::default()));
	host.next_frame().await?;
	host.next_frame().await?;
	Ok(())
}

/// Waits for the list to report `offset`.
async fn settle(host: Host, arena: &Arena, offset: f64) -> Result<(), String> {
	for _ in 0..SETTLE_FRAMES {
		if (reported(arena)? - offset).abs() <= 1.0 {
			return Ok(());
		}
		host.next_frame().await?;
	}

	Err(format!(
		"scrolled to {}, expected {offset}",
		reported(arena)?
	))
}

/// The offset the list last reported.
fn reported(arena: &Arena) -> Result<f64, String> {
	match &arena.read().screen {
		Screen::Feed(feed) => Ok(feed.reported),
		_ => Err("the feed is gone".to_owned()),
	}
}

/// Iterations of [`spin`] per millisecond. The guest has no synchronous
/// clock, so a block is a loop sized against the host clock beforehand.
async fn calibrate(host: Host) -> Result<f64, String> {
	const SAMPLE: u64 = 200_000;
	let start = host.now_ms().await?;
	spin(SAMPLE);
	let elapsed = host.now_ms().await? - start;

	Ok(SAMPLE as f64 / elapsed.max(0.001))
}

/// Busy work the optimiser cannot remove.
fn spin(iterations: u64) {
	let mut value = 0_u64;

	for index in 0..iterations {
		value = std::hint::black_box(value.wrapping_mul(31).wrapping_add(index));
	}

	std::hint::black_box(value);
}
