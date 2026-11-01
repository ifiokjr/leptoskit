//! Timing probes for the host's commit path, run on demand:
//! `cargo test -p leptoskit_native --release --test relayout_probe -- --ignored
//! --nocapture`. They mirror the arena's scenarios so a change to diffing or
//! layout can be judged natively before it is measured on a device.

use std::time::Instant;

use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;

/// The arena's ticker wall at `frame`: 20 rows of 15 tiles, tile `i` showing
/// `frame + i`.
fn ticker(frame: u64) -> Element {
	View().class("flex-1").children((0..20).map(|row| {
		View()
			.class("h-8 flex-row")
			.children((0..15).map(move |column| {
				View()
					.class("flex-1 items-center justify-center")
					.child(Text((frame + row * 15 + column).to_string()).class("text-[10px]"))
			}))
	}))
}

fn per_frame(label: &str, frames: u64, mut run: impl FnMut(u64)) {
	let start = Instant::now();

	for frame in 1..=frames {
		run(frame);
	}

	println!(
		"{label:>36}: {:>8.3} ms per frame",
		start.elapsed().as_secs_f64() * 1000.0 / frames as f64
	);
}

#[test]
#[ignore = "a timing probe, not a check"]
fn ticker_commit() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(390.0, 844.0);
	surface.set_root(&ticker(0));
	per_frame(
		"ticker: every label changes",
		std::env::var("PROBE_FRAMES")
			.ok()
			.and_then(|frames| frames.parse().ok())
			.unwrap_or(200),
		|frame| {
			surface.set_root(&ticker(frame));
			surface.backend_mut().clear();
		},
	);
	per_frame("ticker: nothing changes", 200, |_| {
		surface.set_root(&ticker(200));
		surface.backend_mut().clear();
	});
}
