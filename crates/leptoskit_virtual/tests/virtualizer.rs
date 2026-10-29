//! Windowing math: ranges, estimates, measurements, and scroll targets.
//!
//! Offsets are exact sums of dyadic sizes, so equality asserts them directly.
#![allow(clippy::float_cmp)]

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_virtual::Align;
use leptoskit_virtual::Estimate;
use leptoskit_virtual::InvalidEstimate;
use leptoskit_virtual::MeasureError;
use leptoskit_virtual::OffsetError;
use leptoskit_virtual::OutOfRange;
use leptoskit_virtual::VirtualOptions;
use leptoskit_virtual::Virtualizer;

fn mounted(count: usize, size: f64, viewport: f64, offset: f64) -> Virtualizer {
	let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(count, size));
	virtualizer.set_viewport(viewport);
	virtualizer.scroll_to(offset);
	virtualizer
}

fn indexes(virtualizer: &Virtualizer) -> Vec<usize> {
	virtualizer
		.items()
		.expect("a fixed estimate is valid")
		.iter()
		.map(|item| item.index)
		.collect()
}

#[test]
fn a_window_covers_exactly_the_viewport() {
	let virtualizer = mounted(1000, 50.0, 400.0, 0.0);
	assert_eq!(indexes(&virtualizer), (0..8).collect::<Vec<_>>());

	let virtualizer = mounted(1000, 50.0, 400.0, 10_000.0);
	assert_eq!(indexes(&virtualizer), (200..208).collect::<Vec<_>>());
	assert_eq!(virtualizer.total_size(), Ok(50_000.0));
}

#[test]
fn the_last_window_clamps_to_the_items() {
	// Scrolling to the last page ends at the final item, and an offset past
	// the end resolves as the last page rather than an empty window.
	let virtualizer = mounted(10, 50.0, 400.0, 100.0);
	assert_eq!(indexes(&virtualizer), (2..10).collect::<Vec<_>>());

	let virtualizer = mounted(10, 50.0, 400.0, 1_000.0);
	assert_eq!(indexes(&virtualizer), (2..10).collect::<Vec<_>>());
}

#[test]
fn overscan_widens_both_sides_without_leaving_the_items() {
	let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(1000, 50.0).with_overscan(3));
	virtualizer.set_viewport(400.0);
	virtualizer.scroll_to(10_000.0);
	assert_eq!(indexes(&virtualizer), (197..211).collect::<Vec<_>>());

	// The window never reaches past the first or last item.
	virtualizer.scroll_to(0.0);
	assert_eq!(indexes(&virtualizer), (0..11).collect::<Vec<_>>());
	virtualizer.scroll_to(49_600.0);
	assert_eq!(indexes(&virtualizer), (989..1000).collect::<Vec<_>>());
}

#[test]
fn an_unknown_viewport_renders_nothing() {
	let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(10, 50.0));
	virtualizer.scroll_to(0.0);
	assert_eq!(indexes(&virtualizer), Vec::<usize>::new());
	assert_eq!(virtualizer.total_size(), Ok(500.0));
}

#[test]
fn a_measurement_shifts_every_item_after_it() {
	let mut virtualizer = mounted(10, 50.0, 400.0, 0.0);
	virtualizer
		.measure(0, 80.0)
		.expect("index 0 with a finite size is in range");

	let items = virtualizer.items().expect("a fixed estimate is valid");
	assert_eq!(items[0].size, 80.0);
	assert_eq!(items[1].index, 1);
	assert_eq!(items[1].start, 80.0);
	assert_eq!(items[2].start, 130.0);
	assert_eq!(virtualizer.total_size(), Ok(530.0));
}

#[test]
fn measurements_set_count_and_errors_are_named() {
	let mut virtualizer = mounted(10, 50.0, 400.0, 0.0);

	assert_eq!(
		virtualizer.measure(10, 50.0),
		Err(MeasureError::OutOfRange(OutOfRange {
			index: 10,
			count: 10
		}))
	);
	assert_eq!(
		virtualizer.measure(3, 0.0),
		Err(MeasureError::InvalidSize {
			index: 3,
			value: 0.0
		})
	);

	// A shrinking count drops measurements that no longer apply.
	virtualizer.measure(9, 20.0).expect("index 9 is in range");
	virtualizer.set_count(9);
	assert_eq!(
		virtualizer.measure(9, 20.0),
		Err(MeasureError::OutOfRange(OutOfRange { index: 9, count: 9 }))
	);
	assert_eq!(indexes(&virtualizer), (0..8).collect::<Vec<_>>());
	assert_eq!(virtualizer.total_size(), Ok(450.0));
}

#[test]
fn a_bad_estimate_names_its_index() {
	let estimate = Estimate::PerItem(Arc::new(|index: usize| if index == 4 { 0.0 } else { 50.0 }));
	let options = VirtualOptions {
		count: 10,
		estimate,
		overscan: 0,
		padding_start: 0.0,
		padding_end: 0.0,
	};
	let mut virtualizer = Virtualizer::new(options);
	virtualizer.set_viewport(400.0);

	let error = virtualizer.items().unwrap_err();
	assert_eq!(error.index, 4);
	assert_eq!(error.value, 0.0);
	assert!(virtualizer.total_size().is_err());

	// Measuring the offending item resolves the table again.
	virtualizer.measure(4, 75.0).expect("index 4 is in range");
	let items = virtualizer.items().expect("the estimate is repaired");
	assert_eq!(items[4].start, 200.0);
	assert_eq!(items[5].start, 275.0);
}

#[test]
fn aligned_targets_clamp_to_the_scrollable_range() {
	let virtualizer = mounted(10, 50.0, 200.0, 0.0);

	assert_eq!(virtualizer.offset_for_index(3, Align::Start), Ok(150.0));
	assert_eq!(virtualizer.offset_for_index(3, Align::End), Ok(0.0));
	assert_eq!(virtualizer.offset_for_index(4, Align::Center), Ok(125.0));

	// The last item cannot scroll past the end of the content.
	assert_eq!(virtualizer.offset_for_index(9, Align::Start), Ok(300.0));
	assert_eq!(virtualizer.offset_for_index(9, Align::End), Ok(300.0));

	assert_eq!(
		virtualizer.offset_for_index(10, Align::Start),
		Err(OffsetError::OutOfRange(OutOfRange {
			index: 10,
			count: 10
		}))
	);
}

#[test]
fn auto_alignment_moves_only_when_the_item_is_hidden() {
	let mut virtualizer = mounted(100, 50.0, 200.0, 0.0);

	// Item 2 is fully inside the viewport: the offset stays.
	assert_eq!(virtualizer.offset_for_index(2, Align::Auto), Ok(0.0));

	// Item 6 ends past the viewport: align its end to the viewport's end.
	assert_eq!(virtualizer.offset_for_index(6, Align::Auto), Ok(150.0));

	// An item before the viewport aligns its start to the viewport's start.
	virtualizer.scroll_to(400.0);
	assert_eq!(virtualizer.offset_for_index(2, Align::Auto), Ok(100.0));
	// Item 9 is inside this viewport: the offset stays.
	assert_eq!(virtualizer.offset_for_index(9, Align::Auto), Ok(400.0));
}

#[test]
fn padding_becomes_part_of_the_content() {
	let options = VirtualOptions {
		count: 4,
		estimate: Estimate::Fixed(50.0),
		overscan: 0,
		padding_start: 40.0,
		padding_end: 60.0,
	};
	let mut virtualizer = Virtualizer::new(options);
	virtualizer.set_viewport(200.0);

	let items = virtualizer.items().expect("a fixed estimate is valid");
	assert_eq!(items[0].start, 40.0);
	assert_eq!(virtualizer.total_size(), Ok(300.0));
	assert_eq!(virtualizer.offset_for_index(0, Align::Start), Ok(40.0));
}

#[test]
fn getters_report_what_was_recorded() {
	let mut virtualizer = mounted(10, 50.0, 400.0, 0.0);
	assert_eq!(virtualizer.count(), 10);
	assert_eq!(virtualizer.scroll_offset(), 0.0);
	virtualizer.scroll_to(120.0);
	assert_eq!(virtualizer.scroll_offset(), 120.0);

	// Recording the same values keeps the resolved table as it was.
	let before = indexes(&virtualizer);
	virtualizer.set_count(10);
	let after = indexes(&virtualizer);
	assert_eq!(before, after);

	// Measuring a size the estimate already holds keeps the window put: the
	// list was scrolled to 120, so the window starts at item 2 either way.
	virtualizer.measure(0, 50.0).expect("index 0 is in range");
	assert_eq!(indexes(&virtualizer), (2..10).collect::<Vec<_>>());
}

#[test]
fn an_empty_list_has_padding_for_content() {
	let options = VirtualOptions {
		count: 0,
		estimate: Estimate::Fixed(50.0),
		overscan: 0,
		padding_start: 40.0,
		padding_end: 60.0,
	};
	let mut virtualizer = Virtualizer::new(options);
	virtualizer.set_viewport(400.0);
	assert_eq!(indexes(&virtualizer), Vec::<usize>::new());
	assert_eq!(virtualizer.total_size(), Ok(100.0));
}

#[test]
fn errors_read_with_their_indexes_and_values() {
	let estimate = Estimate::PerItem(Arc::new(|_| f64::NAN));
	let options = VirtualOptions {
		count: 3,
		estimate,
		overscan: 0,
		padding_start: 0.0,
		padding_end: 0.0,
	};
	let mut virtualizer = Virtualizer::new(options);
	virtualizer.set_viewport(400.0);

	let error = virtualizer.items().unwrap_err();
	assert_eq!(
		error.to_string(),
		"the size estimate for item 0 is NaN, not a finite positive number of layout units"
	);
	// An invalid estimate has no offset to scroll to, and says why.
	virtualizer.scroll_to(77.0);
	let error = virtualizer.offset_for_index(1, Align::Center).unwrap_err();
	assert!(
		matches!(error, OffsetError::InvalidEstimate(InvalidEstimate { index: 0, value }) if value.is_nan())
	);
	assert!(
		error
			.to_string()
			.starts_with("the size estimate for item 0 is NaN")
	);
	assert!(std::error::Error::source(&error).is_some());

	let out_of_range = OutOfRange { index: 9, count: 3 };
	assert_eq!(out_of_range.to_string(), "index 9 is outside 0..3");

	let mut measured = mounted(3, 50.0, 0.0, 0.0);
	assert_eq!(
		measured.measure(3, 50.0).unwrap_err().to_string(),
		"index 3 is outside 0..3"
	);
	assert_eq!(
		measured.measure(2, -1.0).unwrap_err().to_string(),
		"the measured size for item 2 is -1, not a finite positive number of layout units"
	);
}

#[test]
fn a_huge_overscan_saturates_instead_of_overflowing() {
	let mut virtualizer =
		Virtualizer::new(VirtualOptions::fixed(10, 50.0).with_overscan(usize::MAX));
	virtualizer.set_viewport(100.0);
	virtualizer.scroll_to(200.0);
	assert_eq!(indexes(&virtualizer), (0..10).collect::<Vec<_>>());
}

#[test]
fn an_elastic_overscroll_renders_the_edge_window() {
	// iOS bounces past both edges; the window stays on the real content.
	let mut virtualizer = mounted(100, 50.0, 200.0, -80.0);
	assert_eq!(indexes(&virtualizer), [0, 1, 2, 3]);
	virtualizer.scroll_to(9_000.0);
	assert_eq!(indexes(&virtualizer), [96, 97, 98, 99]);
	// The recorded offset is what the surface reported.
	assert_eq!(virtualizer.scroll_offset(), 9_000.0);
}

#[test]
fn a_shrunk_count_under_the_offset_shows_the_last_window() {
	let mut virtualizer = mounted(1_000, 50.0, 200.0, 40_000.0);
	assert_eq!(indexes(&virtualizer), [800, 801, 802, 803]);
	virtualizer.set_count(10);
	assert_eq!(indexes(&virtualizer), [6, 7, 8, 9]);
}

#[test]
fn a_growing_count_keeps_earlier_measurements() {
	let mut virtualizer = mounted(3, 50.0, 1_000.0, 0.0);
	virtualizer.measure(1, 80.0).expect("index 1 is in range");
	virtualizer.set_count(5);

	let items = virtualizer.items().expect("a fixed estimate is valid");
	assert_eq!(items.len(), 5);
	assert_eq!(items[1].size, 80.0);
	assert_eq!(items[4].start, 230.0);
	assert_eq!(virtualizer.total_size(), Ok(280.0));
}

#[test]
fn a_measurement_resolves_only_the_items_after_it() {
	let calls = Arc::new(AtomicUsize::new(0));
	let estimate = Estimate::PerItem(Arc::new({
		let calls = Arc::clone(&calls);
		move |_| {
			calls.fetch_add(1, Ordering::Relaxed);
			50.0
		}
	}));
	let mut virtualizer = Virtualizer::new(VirtualOptions {
		estimate,
		..VirtualOptions::fixed(1_000, 0.0)
	});
	virtualizer.set_viewport(200.0);

	virtualizer.items().expect("the estimate is valid");
	assert_eq!(calls.load(Ordering::Relaxed), 1_000);

	// A measurement near the end re-resolves the ten items from it on, not
	// the whole list again; repeating it changes nothing.
	virtualizer
		.measure(990, 75.0)
		.expect("index 990 is in range");
	virtualizer.items().expect("the estimate is valid");
	assert_eq!(calls.load(Ordering::Relaxed), 1_009);
	virtualizer
		.measure(990, 75.0)
		.expect("index 990 is in range");
	virtualizer.items().expect("the estimate is valid");
	assert_eq!(calls.load(Ordering::Relaxed), 1_009);
	assert_eq!(virtualizer.total_size(), Ok(50_025.0));
}

#[test]
fn an_item_taller_than_the_viewport_aligns_by_its_start() {
	let estimate = Estimate::PerItem(Arc::new(|index| if index == 5 { 600.0 } else { 50.0 }));
	let mut virtualizer = Virtualizer::new(VirtualOptions {
		estimate,
		..VirtualOptions::fixed(20, 0.0)
	});
	virtualizer.set_viewport(200.0);

	// Item 5 starts at 250 and ends at 850: it cannot be fully visible, so
	// Auto scrolls the least distance that brings its end into view; Start
	// shows its top.
	assert_eq!(virtualizer.offset_for_index(5, Align::Auto), Ok(650.0));
	assert_eq!(virtualizer.offset_for_index(5, Align::Start), Ok(250.0));

	// Inside it, the window is that one item.
	virtualizer.scroll_to(400.0);
	assert_eq!(indexes(&virtualizer), [5]);
}

#[test]
fn a_window_inside_the_leading_padding_starts_at_the_first_item() {
	let mut virtualizer = Virtualizer::new(VirtualOptions {
		padding_start: 120.0,
		..VirtualOptions::fixed(100, 50.0)
	});
	virtualizer.set_viewport(100.0);
	assert_eq!(indexes(&virtualizer), Vec::<usize>::new());
	virtualizer.scroll_to(60.0);
	// The viewport covers 60..160: the padding, then item 0 from 120.
	assert_eq!(indexes(&virtualizer), [0]);
}

#[test]
fn aligned_targets_follow_measurements() {
	let mut virtualizer = mounted(100, 50.0, 200.0, 0.0);
	assert_eq!(virtualizer.offset_for_index(10, Align::Start), Ok(500.0));
	virtualizer.measure(3, 150.0).expect("index 3 is in range");
	assert_eq!(virtualizer.offset_for_index(10, Align::Start), Ok(600.0));
}

#[test]
fn a_zero_viewport_aligns_starts_and_ends_to_one_point() {
	let virtualizer = mounted(10, 50.0, 0.0, 0.0);
	assert_eq!(virtualizer.offset_for_index(4, Align::Start), Ok(200.0));
	assert_eq!(virtualizer.offset_for_index(4, Align::End), Ok(250.0));
	assert_eq!(virtualizer.offset_for_index(4, Align::Center), Ok(225.0));
}

#[test]
fn per_item_estimates_resolve_exact_boundaries() {
	// Sizes 10, 20, 30, …: item n starts at 5·n·(n+1).
	let estimate = Estimate::PerItem(Arc::new(|index| 10.0 * (index as f64 + 1.0)));
	let mut virtualizer = Virtualizer::new(VirtualOptions {
		estimate,
		..VirtualOptions::fixed(50, 0.0)
	});
	virtualizer.set_viewport(130.0);

	// The viewport 150..280 starts exactly where item 4 ends and ends exactly
	// where item 7 starts: an item that only touches an edge is out.
	virtualizer.scroll_to(150.0);
	assert_eq!(indexes(&virtualizer), [5, 6]);
	let items = virtualizer.items().expect("the estimate is valid");
	assert_eq!(items[0].start, 150.0);
	assert_eq!(items[1].start + items[1].size, 280.0);
}
