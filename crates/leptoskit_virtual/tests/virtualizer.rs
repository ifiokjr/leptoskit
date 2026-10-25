//! Windowing math: ranges, estimates, measurements, and scroll targets.
//!
//! Offsets are exact sums of dyadic sizes, so equality asserts them directly.
#![allow(clippy::float_cmp)]

use leptoskit_virtual::Align;
use leptoskit_virtual::Estimate;
use leptoskit_virtual::MeasureError;
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
	// Scrolling to the last page still ends at the final item, and scrolling
	// past everything renders nothing.
	let virtualizer = mounted(10, 50.0, 400.0, 100.0);
	assert_eq!(indexes(&virtualizer), (2..10).collect::<Vec<_>>());

	let virtualizer = mounted(10, 50.0, 400.0, 1_000.0);
	assert_eq!(indexes(&virtualizer), Vec::<usize>::new());
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
	let estimate = Estimate::PerItem(std::sync::Arc::new(
		|index: usize| {
			if index == 4 { 0.0 } else { 50.0 }
		},
	));
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
		Err(OutOfRange {
			index: 10,
			count: 10
		})
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
	let estimate = Estimate::PerItem(std::sync::Arc::new(|_| f64::NAN));
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
	// An invalid estimate leaves an aligned target where it was.
	virtualizer.scroll_to(77.0);
	assert_eq!(virtualizer.offset_for_index(1, Align::Center), Ok(77.0));

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
