//! Physical coordinates for logical flex alignment and line reversal.
#![allow(clippy::float_cmp)]

use leptoskit_yoga::Align;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Direction;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::FlexWrap;
use leptoskit_yoga::Layout;
use leptoskit_yoga::Rect;
use leptoskit_yoga::Size;
use leptoskit_yoga::Style;
use leptoskit_yoga::Yoga;

fn frames(parent_style: Style, children: impl IntoIterator<Item = Style>) -> Vec<Layout> {
	let mut yoga = Yoga::new();
	let root = yoga.create_node();
	yoga.set_style(root, parent_style);
	let children = children
		.into_iter()
		.map(|style| {
			let child = yoga.create_node();
			yoga.set_style(child, style);
			yoga.insert_child(root, child, yoga.children(root).len());
			child
		})
		.collect::<Vec<_>>();
	yoga.calculate_layout(
		root,
		Size::new(
			AvailableSpace::Definite(300.0),
			AvailableSpace::Definite(300.0),
		),
	);
	children
		.into_iter()
		.map(|child| *yoga.layout(child))
		.collect()
}

fn box_style(width: f32, height: f32) -> Style {
	Style {
		width: Dimension::Points(width),
		height: Dimension::Points(height),
		..Style::default()
	}
}

#[test]
fn rtl_columns_align_from_the_right_with_physical_margins_and_decorations() {
	for (align, expected_x) in [
		(Align::FlexStart, 73.0),
		(Align::FlexEnd, 14.0),
		(Align::Center, 43.5),
	] {
		let parent = Style {
			direction: Direction::Rtl,
			align_items: align,
			padding: Rect::all_edges(
				Dimension::Points(5.0),
				Dimension::Points(13.0),
				Dimension::Points(0.0),
				Dimension::Points(7.0),
			),
			border: Rect::all_edges(0.0, 5.0, 0.0, 3.0),
			..box_style(120.0, 100.0)
		};
		let child = Style {
			margin: Rect::all_edges(
				Dimension::Points(2.0),
				Dimension::Points(9.0),
				Dimension::Points(0.0),
				Dimension::Points(4.0),
			),
			..box_style(20.0, 10.0)
		};
		let result = frames(parent, [child]);
		assert_eq!((result[0].x, result[0].y), (expected_x, 7.0), "{align:?}");
	}
}

#[test]
fn rtl_column_reverse_reverses_both_logical_axes() {
	let result = frames(
		Style {
			direction: Direction::Rtl,
			flex_direction: FlexDirection::ColumnReverse,
			align_items: Align::FlexStart,
			..box_style(100.0, 100.0)
		},
		[box_style(20.0, 10.0)],
	);
	assert_eq!((result[0].x, result[0].y), (80.0, 90.0));
}

#[test]
fn wrapped_columns_mirror_line_order_and_start_alignment_together() {
	for (direction, wrap, expected) in [
		(Direction::Ltr, FlexWrap::Wrap, [0.0, 30.0, 60.0]),
		(Direction::Rtl, FlexWrap::Wrap, [80.0, 50.0, 20.0]),
		(Direction::Ltr, FlexWrap::WrapReverse, [80.0, 50.0, 20.0]),
		(Direction::Rtl, FlexWrap::WrapReverse, [0.0, 30.0, 60.0]),
	] {
		let parent = Style {
			direction,
			flex_wrap: wrap,
			align_items: Align::FlexStart,
			align_content: Align::FlexStart,
			gap: Rect::all_edges(
				Dimension::Auto,
				Dimension::Auto,
				Dimension::Auto,
				Dimension::Points(10.0),
			),
			..box_style(100.0, 40.0)
		};
		let result = frames(parent, (0..3).map(|_| box_style(20.0, 30.0)));
		assert_eq!(
			result.iter().map(|layout| layout.x).collect::<Vec<_>>(),
			expected,
			"{direction:?} {wrap:?}"
		);
	}
}

#[test]
fn a_wrapped_line_counts_only_the_gaps_between_its_own_items() {
	// Two 45-point items and the 10-point gap between them fill a 100-point
	// row exactly, so four items make two full rows: the item that starts the
	// second row has no gap before it.
	let result = frames(
		Style {
			flex_direction: FlexDirection::Row,
			flex_wrap: FlexWrap::Wrap,
			align_items: Align::FlexStart,
			align_content: Align::FlexStart,
			gap: Rect::all_edges(
				Dimension::Auto,
				Dimension::Auto,
				Dimension::Auto,
				Dimension::Points(10.0),
			),
			..box_style(100.0, 100.0)
		},
		(0..4).map(|_| box_style(45.0, 20.0)),
	);
	assert_eq!(
		result
			.iter()
			.map(|layout| (layout.x, layout.y))
			.collect::<Vec<_>>(),
		[(0.0, 0.0), (55.0, 0.0), (0.0, 20.0), (55.0, 20.0)]
	);
}

#[test]
fn wrap_reverse_starts_rows_at_the_bottom_without_losing_free_space() {
	let result = frames(
		Style {
			flex_direction: FlexDirection::Row,
			flex_wrap: FlexWrap::WrapReverse,
			align_items: Align::FlexStart,
			align_content: Align::FlexStart,
			gap: Rect::all_edges(
				Dimension::Points(10.0),
				Dimension::Auto,
				Dimension::Auto,
				Dimension::Auto,
			),
			..box_style(40.0, 100.0)
		},
		(0..3).map(|_| box_style(30.0, 20.0)),
	);
	assert_eq!(
		result.iter().map(|layout| layout.y).collect::<Vec<_>>(),
		[80.0, 50.0, 20.0]
	);
}

#[test]
fn wrap_reverse_line_end_uses_the_physical_top_even_with_one_line() {
	let result = frames(
		Style {
			flex_direction: FlexDirection::Row,
			flex_wrap: FlexWrap::WrapReverse,
			align_content: Align::FlexEnd,
			..box_style(100.0, 100.0)
		},
		[box_style(20.0, 20.0)],
	);
	assert_eq!(result[0].y, 0.0);
}

#[test]
fn rtl_stretch_anchors_a_max_constrained_child_to_cross_start() {
	let result = frames(
		Style {
			direction: Direction::Rtl,
			..box_style(100.0, 100.0)
		},
		[Style {
			max_width: Dimension::Points(40.0),
			height: Dimension::Points(20.0),
			..Style::default()
		}],
	);
	assert_eq!((result[0].x, result[0].width), (60.0, 40.0));
}
