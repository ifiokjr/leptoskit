//! Layout engine integration tests, in layout units (logical pixels).

// Layout outputs are exact floats: the expected values are whole numbers
// produced by the same arithmetic the engine runs, not measured quantities.
#![allow(clippy::float_cmp)]

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_yoga::Align;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Display;
use leptoskit_yoga::GridAutoFlow;
use leptoskit_yoga::GridLine;
use leptoskit_yoga::GridPlacement;
use leptoskit_yoga::GridStyle;
use leptoskit_yoga::NodeId;
use leptoskit_yoga::PositionType;
use leptoskit_yoga::Size;
use leptoskit_yoga::Style;
use leptoskit_yoga::TrackSizing;
use leptoskit_yoga::Yoga;

fn definite(width: f32, height: f32) -> Size<AvailableSpace> {
	Size::new(
		AvailableSpace::Definite(width),
		AvailableSpace::Definite(height),
	)
}

fn points(value: f32) -> Dimension {
	Dimension::Points(value)
}

fn simple_child(tree: &mut Yoga, parent: NodeId, style: Style) -> NodeId {
	let child = tree.create_node();
	tree.set_style(child, style);
	tree.insert_child(parent, child, tree.children(parent).len());
	child
}

#[test]
fn row_places_children_horizontally() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			flex_direction: leptoskit_yoga::FlexDirection::Row,
			width: points(100.0),
			height: points(50.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(30.0),
			height: points(20.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(40.0),
			height: points(20.0),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(100.0, 50.0));

	let first = tree.children(root)[0];
	let second = tree.children(root)[1];
	assert_eq!(
		(tree.layout(first).x, tree.layout(first).width),
		(0.0, 30.0)
	);
	assert_eq!(
		(tree.layout(second).x, tree.layout(second).width),
		(30.0, 40.0)
	);
}

#[test]
fn column_stretches_children_to_full_width() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(120.0),
			height: points(80.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			height: points(40.0),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(120.0, 80.0));

	let child = tree.children(root)[0];
	assert_eq!(
		(tree.layout(child).width, tree.layout(child).height),
		(120.0, 40.0)
	);
}

#[test]
fn flex_grow_splits_free_space() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			flex_direction: leptoskit_yoga::FlexDirection::Row,
			width: points(100.0),
			height: points(50.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			flex_grow: 1.0,
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			flex_grow: 3.0,
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(100.0, 50.0));

	let first = tree.children(root)[0];
	let second = tree.children(root)[1];
	assert_eq!(tree.layout(first).width, 25.0);
	assert_eq!(tree.layout(second).x, 25.0);
	assert_eq!(tree.layout(second).width, 75.0);
}

#[test]
fn flex_shrink_respects_min_size() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			flex_direction: leptoskit_yoga::FlexDirection::Row,
			width: points(100.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			flex_shrink: 1.0,
			min_width: points(40.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			flex_shrink: 1.0,
			min_width: points(40.0),
			..Style::default()
		},
	);
	// Both children want the content width of 0 but the container is empty, so
	// only the min sizes hold the line.
	let _ = tree.style(root).clone();
	tree.calculate_layout(root, definite(100.0, 50.0));

	let first = tree.children(root)[0];
	let second = tree.children(root)[1];
	assert_eq!(tree.layout(first).width, 40.0);
	assert_eq!(tree.layout(second).width, 40.0);
	assert_eq!(tree.layout(second).x, 40.0);
}

#[test]
fn justify_content_center_offsets_items() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			flex_direction: leptoskit_yoga::FlexDirection::Row,
			justify_content: leptoskit_yoga::JustifyContent::Center,
			width: points(100.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(20.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(30.0),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(100.0, 50.0));

	let first = tree.children(root)[0];
	let second = tree.children(root)[1];
	assert_eq!(tree.layout(first).x, 25.0);
	assert_eq!(tree.layout(second).x, 45.0);
}

#[test]
fn gap_separates_items() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			flex_direction: leptoskit_yoga::FlexDirection::Row,
			gap: leptoskit_yoga::Rect {
				left: Dimension::Points(10.0),
				..Default::default()
			},
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(20.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(20.0),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(200.0, 50.0));

	let second = tree.children(root)[1];
	assert_eq!(tree.layout(second).x, 30.0);
}

#[test]
fn wrapping_breaks_lines_and_stacks_them() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			flex_direction: leptoskit_yoga::FlexDirection::Row,
			flex_wrap: leptoskit_yoga::FlexWrap::Wrap,
			width: points(100.0),
			..Style::default()
		},
	);
	for _ in 0..3 {
		simple_child(
			&mut tree,
			root,
			Style {
				width: points(40.0),
				height: points(10.0),
				..Style::default()
			},
		);
	}
	tree.calculate_layout(root, definite(100.0, 50.0));

	let first = tree.children(root)[0];
	let second = tree.children(root)[1];
	let third = tree.children(root)[2];
	// Two per line, third wraps.
	assert_eq!((tree.layout(first).x, tree.layout(first).y), (0.0, 0.0));
	assert_eq!((tree.layout(second).x, tree.layout(second).y), (40.0, 0.0));
	assert_eq!((tree.layout(third).x, tree.layout(third).y), (0.0, 10.0));
	assert_eq!(tree.layout(root).height, 20.0);
}

#[test]
fn padding_and_border_offset_children() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			padding: leptoskit_yoga::Rect::new(|| Dimension::Points(10.0)),
			border: leptoskit_yoga::Rect {
				top: 5.0,
				right: 5.0,
				bottom: 5.0,
				left: 5.0,
			},
			..Style::default()
		},
	);
	simple_child(&mut tree, root, Style::default());
	tree.calculate_layout(root, definite(200.0, 200.0));

	let child = tree.children(root)[0];
	assert_eq!((tree.layout(child).x, tree.layout(child).y), (15.0, 15.0));
}

#[test]
fn absolute_child_positions_against_padding_box() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(200.0),
			height: points(200.0),
			border: leptoskit_yoga::Rect {
				top: 2.0,
				right: 2.0,
				bottom: 2.0,
				left: 2.0,
			},
			..Style::default()
		},
	);
	let child = tree.create_node();
	tree.set_style(
		child,
		Style {
			position: PositionType::Absolute,
			inset: leptoskit_yoga::Rect {
				top: Dimension::Points(8.0),
				left: Dimension::Points(12.0),
				..Default::default()
			},
			width: points(50.0),
			height: points(20.0),
			..Style::default()
		},
	);
	tree.insert_child(root, child, 0);
	tree.calculate_layout(root, definite(200.0, 200.0));

	let layout = tree.layout(child);
	assert_eq!((layout.x, layout.y), (2.0 + 12.0, 2.0 + 8.0));
}

#[test]
fn absolute_child_with_both_insets_stretches() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(200.0),
			height: points(100.0),
			..Style::default()
		},
	);
	let child = tree.create_node();
	tree.set_style(
		child,
		Style {
			position: PositionType::Absolute,
			inset: leptoskit_yoga::Rect {
				top: Dimension::Points(10.0),
				bottom: Dimension::Points(20.0),
				..Default::default()
			},
			..Style::default()
		},
	);
	tree.insert_child(root, child, 0);
	tree.calculate_layout(root, definite(200.0, 100.0));

	let layout = tree.layout(child);
	assert_eq!(layout.height, 70.0);
}

#[test]
fn percentage_width_resolves_against_parent() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(200.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: Dimension::Percent(0.5),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(200.0, 100.0));

	let child = tree.children(root)[0];
	assert_eq!(tree.layout(child).width, 100.0);
}

#[test]
fn aspect_ratio_derives_height() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			flex_direction: leptoskit_yoga::FlexDirection::Row,
			width: points(200.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(100.0),
			aspect_ratio: Some(2.0),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(200.0, 100.0));

	let child = tree.children(root)[0];
	assert_eq!(tree.layout(child).height, 50.0);
}

/// A text-like leaf: 200 units of content in 10-unit lines, wrapped to the
/// available width.
fn paragraph_leaf(tree: &mut Yoga) -> NodeId {
	let leaf = tree.create_node();
	tree.set_measure_func(
		leaf,
		Some(std::sync::Arc::new(|available: Size<AvailableSpace>| {
			let width = available.width.definite().map_or(200.0, |w| w.min(200.0));
			let lines = if width > 0.0 {
				(200.0 / width).ceil()
			} else {
				1.0
			};
			Size::new(width, lines * 10.0)
		})),
	);
	leaf
}

#[test]
fn text_in_a_column_wraps_at_the_column_width() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(100.0),
			height: points(300.0),
			..Style::default()
		},
	);
	// An auto-sized column between the root and the text: its width comes
	// from stretching, and the text must still wrap at it.
	let column = simple_child(&mut tree, root, Style::default());
	let text = paragraph_leaf(&mut tree);
	tree.insert_child(column, text, 0);
	tree.calculate_layout(root, definite(100.0, 300.0));

	let layout = tree.layout(text);
	assert_eq!((layout.width, layout.height), (100.0, 20.0));
	assert_eq!(tree.layout(column).height, 20.0);
}

#[test]
fn max_width_bounds_the_space_content_wraps_in() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(300.0),
			height: points(300.0),
			align_items: Align::FlexEnd,
			..Style::default()
		},
	);
	// A chat bubble: shrink-to-fit, capped at half the width, with padding.
	let bubble = simple_child(
		&mut tree,
		root,
		Style {
			max_width: Dimension::Percent(0.5),
			padding: leptoskit_yoga::Rect::all_edges(
				points(5.0),
				points(10.0),
				points(5.0),
				points(10.0),
			),
			..Style::default()
		},
	);
	let text = paragraph_leaf(&mut tree);
	tree.insert_child(bubble, text, 0);
	tree.calculate_layout(root, definite(300.0, 300.0));

	// 150 wide minus 20 of padding leaves 130 for two lines of text.
	let bubble_layout = tree.layout(bubble);
	assert_eq!((bubble_layout.width, bubble_layout.height), (150.0, 30.0));
	assert_eq!(bubble_layout.x, 150.0);
	let text_layout = tree.layout(text);
	assert_eq!((text_layout.width, text_layout.height), (130.0, 20.0));
}

#[test]
fn percentages_inside_a_stretched_column_resolve_while_measuring() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(300.0),
			height: points(300.0),
			..Style::default()
		},
	);
	// An auto-height column whose width only comes from stretching, holding
	// a bubble capped at half of that width.
	let column = simple_child(&mut tree, root, Style::default());
	let bubble = simple_child(
		&mut tree,
		column,
		Style {
			max_width: Dimension::Percent(0.5),
			align_self: Align::FlexStart,
			..Style::default()
		},
	);
	let text = paragraph_leaf(&mut tree);
	tree.insert_child(bubble, text, 0);
	tree.calculate_layout(root, definite(300.0, 300.0));

	// The text wraps at 150 into two lines, and the column is measured with
	// that wrapping, so it contains the bubble.
	assert_eq!(tree.layout(text).height, 20.0);
	assert_eq!(tree.layout(bubble).height, 20.0);
	assert_eq!(tree.layout(column).height, 20.0);
}

#[test]
fn aspect_ratio_derives_the_cross_size_of_a_flexed_item() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(200.0),
			..Style::default()
		},
	);
	let row = simple_child(
		&mut tree,
		root,
		Style {
			flex_direction: leptoskit_yoga::FlexDirection::Row,
			align_items: Align::FlexStart,
			..Style::default()
		},
	);
	let tile = Style {
		flex_grow: 1.0,
		flex_basis: points(0.0),
		aspect_ratio: Some(1.0),
		..Style::default()
	};
	let first = simple_child(&mut tree, row, tile.clone());
	let second = simple_child(&mut tree, row, tile);
	tree.calculate_layout(root, definite(200.0, 400.0));

	for tile in [first, second] {
		let layout = tree.layout(tile);
		assert_eq!((layout.width, layout.height), (100.0, 100.0));
	}
	assert_eq!(tree.layout(row).height, 100.0);
}

#[test]
fn measure_func_sizes_text_leaf() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			flex_direction: leptoskit_yoga::FlexDirection::Row,
			width: points(500.0),
			..Style::default()
		},
	);
	let text = tree.create_node();
	tree.set_measure_func(text, Some(std::sync::Arc::new(|_| Size::new(80.0, 18.0))));
	tree.insert_child(root, text, 0);
	tree.calculate_layout(root, definite(500.0, 100.0));

	let layout = tree.layout(text);
	assert_eq!((layout.width, layout.height), (80.0, 18.0));
}

#[test]
fn display_none_removes_the_subtree() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			flex_direction: leptoskit_yoga::FlexDirection::Row,
			..Style::default()
		},
	);
	let child = simple_child(
		&mut tree,
		root,
		Style {
			width: points(40.0),
			..Style::default()
		},
	);
	let mut hidden = Style {
		width: points(60.0),
		..Style::default()
	};
	hidden.display = Display::None;
	simple_child(&mut tree, root, hidden);
	tree.calculate_layout(root, definite(500.0, 100.0));

	let layout = tree.layout(child);
	assert_eq!(tree.layout(root).width, 40.0);
	assert_eq!(layout.width, 40.0);
}

#[test]
fn auto_margins_absorb_free_space() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			flex_direction: leptoskit_yoga::FlexDirection::Row,
			width: points(100.0),
			..Style::default()
		},
	);
	let pushed = tree.create_node();
	tree.set_style(
		pushed,
		Style {
			width: points(20.0),
			margin: leptoskit_yoga::Rect {
				left: Dimension::Auto,
				top: Dimension::Points(0.0),
				right: Dimension::Points(0.0),
				bottom: Dimension::Points(0.0),
			},
			..Style::default()
		},
	);
	tree.insert_child(root, pushed, 0);
	tree.calculate_layout(root, definite(100.0, 50.0));

	assert_eq!(tree.layout(pushed).x, 80.0);
}

#[test]
fn auto_sized_row_wraps_its_content() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(root, Style::row());
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(30.0),
			height: points(12.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(45.0),
			height: points(12.0),
			..Style::default()
		},
	);
	tree.calculate_layout(root, max_content());

	assert_eq!(tree.layout(root).width, 75.0);
}

/// Max-content sizing on both axes.
fn max_content() -> Size<AvailableSpace> {
	Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent)
}

#[test]
fn rtl_row_lays_out_from_the_right_edge() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			flex_direction: leptoskit_yoga::FlexDirection::Row,
			direction: leptoskit_yoga::Direction::Rtl,
			width: points(100.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(30.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(40.0),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(100.0, 50.0));

	let first = tree.children(root)[0];
	let second = tree.children(root)[1];
	assert_eq!(tree.layout(first).x, 70.0);
	assert_eq!(tree.layout(second).x, 30.0);
}

// --- grid ---

fn grid_style(columns: Vec<TrackSizing>, rows: Vec<TrackSizing>) -> Style {
	Style {
		display: Display::Grid,
		width: points(300.0),
		height: points(200.0),
		grid: Some(GridStyle {
			template_columns: columns,
			template_rows: rows,
			..GridStyle::default()
		}),
		..Style::default()
	}
}

#[test]
fn grid_fixed_tracks_place_items_in_cells() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		grid_style(
			vec![TrackSizing::Points(100.0), TrackSizing::Points(200.0)],
			vec![TrackSizing::Points(50.0), TrackSizing::Points(150.0)],
		),
	);
	for _ in 0..4 {
		simple_child(&mut tree, root, Style::default());
	}
	tree.calculate_layout(root, definite(300.0, 200.0));

	let [a, b, c, d] = [0, 1, 2, 3].map(|index| tree.children(root)[index]);
	assert_eq!((tree.layout(a).x, tree.layout(a).y), (0.0, 0.0));
	assert_eq!((tree.layout(b).x, tree.layout(b).y), (100.0, 0.0));
	assert_eq!((tree.layout(c).x, tree.layout(c).y), (0.0, 50.0));
	assert_eq!((tree.layout(d).x, tree.layout(d).y), (100.0, 50.0));
	// Auto items stretch to fill their area by default.
	assert_eq!((tree.layout(a).width, tree.layout(a).height), (100.0, 50.0));
}

#[test]
fn grid_fr_tracks_share_free_space() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		grid_style(
			vec![TrackSizing::Fraction(1.0), TrackSizing::Fraction(2.0)],
			vec![TrackSizing::Points(100.0)],
		),
	);
	simple_child(&mut tree, root, Style::default());
	simple_child(&mut tree, root, Style::default());
	tree.calculate_layout(root, definite(300.0, 200.0));

	let second = tree.children(root)[1];
	assert_eq!(tree.layout(second).x, 100.0);
	assert_eq!(tree.layout(second).width, 200.0);
}

#[test]
fn grid_span_crosses_tracks() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		grid_style(
			vec![TrackSizing::Points(100.0), TrackSizing::Points(100.0)],
			vec![TrackSizing::Points(100.0), TrackSizing::Points(100.0)],
		),
	);
	simple_child(
		&mut tree,
		root,
		Style {
			grid: Some(GridStyle {
				column: GridPlacement::line(1, 3),
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	simple_child(&mut tree, root, Style::default());
	tree.calculate_layout(root, definite(300.0, 200.0));

	let first = tree.children(root)[0];
	let second = tree.children(root)[1];
	assert_eq!(tree.layout(first).width, 200.0);
	// The spanning item occupies both columns of row one, so the next auto item
	// lands on row two.
	assert_eq!(tree.layout(second).y, 100.0);
}

#[test]
fn grid_gap_separates_cells() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	let mut style = grid_style(
		vec![TrackSizing::Points(100.0), TrackSizing::Points(100.0)],
		vec![TrackSizing::Points(100.0)],
	);
	style.gap = leptoskit_yoga::Rect {
		top: Dimension::Points(10.0),
		left: Dimension::Points(10.0),
		..Default::default()
	};
	tree.set_style(root, style);
	simple_child(&mut tree, root, Style::default());
	simple_child(&mut tree, root, Style::default());
	tree.calculate_layout(root, definite(300.0, 200.0));

	let second = tree.children(root)[1];
	assert_eq!(tree.layout(second).x, 110.0);
}

#[test]
fn grid_auto_tracks_size_from_content() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	let mut style = grid_style(vec![TrackSizing::Auto, TrackSizing::Points(100.0)], vec![]);
	// Without stretch, the auto track keeps its content size.
	style.justify_content = leptoskit_yoga::JustifyContent::FlexStart;
	tree.set_style(root, style);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(80.0),
			..Style::default()
		},
	);
	simple_child(&mut tree, root, Style::default());
	tree.calculate_layout(root, definite(300.0, 200.0));

	let first = tree.children(root)[0];
	let second = tree.children(root)[1];
	assert_eq!(tree.layout(first).width, 80.0);
	assert_eq!(tree.layout(second).x, 80.0);
}

#[test]
fn grid_auto_tracks_stretch_to_fill_the_container() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	// `justify_content` defaults to packing at the start without stretching.
	tree.set_style(
		root,
		grid_style(vec![TrackSizing::Auto, TrackSizing::Points(100.0)], vec![]),
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(80.0),
			..Style::default()
		},
	);
	simple_child(&mut tree, root, Style::default());
	tree.calculate_layout(root, definite(300.0, 200.0));

	// The block axis (`align_content`) defaults to stretch, so the auto row
	// fills the container height.
	assert_eq!(tree.layout(tree.children(root)[0]).height, 200.0);
}

#[test]
fn grid_minmax_bounds_the_track() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		grid_style(
			vec![
				leptoskit_yoga::minmax(50.0, 120.0),
				TrackSizing::Points(100.0),
			],
			vec![],
		),
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(30.0),
			..Style::default()
		},
	);
	simple_child(&mut tree, root, Style::default());
	tree.calculate_layout(root, definite(300.0, 200.0));

	// Free space grows the track, but only up to its maximum.
	assert_eq!(tree.layout(tree.children(root)[1]).x, 120.0);

	// Without free space the minimum holds the track open even though the
	// content is small, so the second item starts at the track edge rather
	// than hugging the first.
	let mut narrow = grid_style(
		vec![
			leptoskit_yoga::minmax(50.0, 120.0),
			TrackSizing::Points(100.0),
		],
		vec![],
	);
	narrow.width = points(150.0);
	tree.set_style(root, narrow);
	tree.calculate_layout(root, definite(300.0, 200.0));
	assert_eq!(tree.layout(tree.children(root)[1]).x, 50.0);
}

#[test]
fn grid_column_flow_walks_down_first() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	let mut style = grid_style(
		vec![TrackSizing::Points(100.0), TrackSizing::Points(100.0)],
		vec![],
	);
	style.grid = Some(GridStyle {
		auto_flow: GridAutoFlow::Column,
		..GridStyle::default()
	});
	style.align_content = Align::FlexStart;
	tree.set_style(root, style);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(40.0),
			height: points(20.0),
			..Style::default()
		},
	);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(40.0),
			height: points(20.0),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(300.0, 200.0));

	// Column flow fills the first column top-to-bottom before moving right.
	let first = tree.children(root)[0];
	let second = tree.children(root)[1];
	assert_eq!((tree.layout(first).x, tree.layout(first).y), (0.0, 0.0));
	assert_eq!((tree.layout(second).x, tree.layout(second).y), (0.0, 20.0));
}

#[test]
fn grid_percent_track_resolves_against_container() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		grid_style(
			vec![TrackSizing::Percent(0.5), TrackSizing::Points(50.0)],
			vec![],
		),
	);
	simple_child(&mut tree, root, Style::default());
	simple_child(&mut tree, root, Style::default());
	tree.calculate_layout(root, definite(300.0, 200.0));

	let second = tree.children(root)[1];
	assert_eq!(tree.layout(second).x, 150.0);
}

#[test]
fn grid_item_alignment_centers_within_area() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	let mut style = grid_style(vec![TrackSizing::Points(200.0)], vec![]);
	style.align_items = Align::Center;
	style.grid = Some(GridStyle {
		template_columns: vec![TrackSizing::Points(200.0)],
		justify_items: Align::Center,
		..GridStyle::default()
	});
	tree.set_style(root, style);
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(40.0),
			height: points(20.0),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(300.0, 200.0));

	let child = tree.children(root)[0];
	let layout = tree.layout(child);
	assert_eq!((layout.x, layout.y), (80.0, 90.0));
}

// --- regressions ---

/// A leaf whose measure function counts its invocations and wraps a 200-wide
/// run of text into 10-high lines at the available width.
fn wrapping_text_leaf(tree: &mut Yoga, calls: &std::sync::Arc<AtomicUsize>) -> NodeId {
	let leaf = tree.create_node();
	let calls = std::sync::Arc::clone(calls);
	tree.set_measure_func(
		leaf,
		Some(std::sync::Arc::new(
			move |available: Size<AvailableSpace>| {
				calls.fetch_add(1, Ordering::Relaxed);
				let width = available.width.definite().map_or(200.0, |w| w.min(200.0));
				let lines = if width > 0.0 {
					(200.0 / width).ceil()
				} else {
					1.0
				};
				Size::new(width, lines * 10.0)
			},
		)),
	);
	leaf
}

/// Builds a chain of auto-sized containers alternating row and column, each
/// optionally holding a fixed-size sibling next to the next level, and returns
/// the measured leaf at the bottom.
fn nested_chain(
	tree: &mut Yoga,
	root: NodeId,
	depth: usize,
	siblings: bool,
	calls: &std::sync::Arc<AtomicUsize>,
) -> NodeId {
	let mut current = root;
	for level in 0..depth {
		let style = if level % 2 == 0 {
			Style::row()
		} else {
			Style::default()
		};
		let next = simple_child(tree, current, style);
		if siblings {
			simple_child(
				tree,
				current,
				Style {
					width: points(5.0),
					height: points(5.0),
					..Style::default()
				},
			);
		}
		current = next;
	}
	let leaf = wrapping_text_leaf(tree, calls);
	tree.insert_child(current, leaf, 0);
	leaf
}

#[test]
fn deep_nesting_measures_each_leaf_a_bounded_number_of_times() {
	const DEPTH: usize = 40;

	// Without a per-node cache every auto-sized level lays its subtree out
	// three times, so the leaf would be measured on the order of 3^40
	// times.
	let mut tree = Yoga::new();
	let root = tree.create_node();
	let calls = std::sync::Arc::new(AtomicUsize::new(0));
	let leaf = nested_chain(&mut tree, root, DEPTH, false, &calls);
	tree.calculate_layout(root, definite(300.0, 300.0));
	let measured = calls.load(Ordering::Relaxed);
	assert!(measured <= 8, "leaf measured {measured} times");
	assert_eq!(tree.layout(leaf).height, 10.0);

	// Siblings make every level's content size distinct, so a node is asked
	// for a number of sizes that grows with depth; the work must stay linear.
	let mut tree = Yoga::new();
	let root = tree.create_node();
	let calls = std::sync::Arc::new(AtomicUsize::new(0));
	nested_chain(&mut tree, root, DEPTH, true, &calls);
	tree.calculate_layout(root, definite(300.0, 300.0));
	let measured = calls.load(Ordering::Relaxed);
	assert!(measured <= 2 * DEPTH, "leaf measured {measured} times");

	// An unchanged relayout is answered from the cache.
	tree.calculate_layout(root, definite(300.0, 300.0));
	assert_eq!(calls.load(Ordering::Relaxed), measured);
}

#[test]
fn cached_layout_reflects_changes_below_clean_ancestors() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: Dimension::Percent(1.0),
			..Style::default()
		},
	);
	let branch = simple_child(&mut tree, root, Style::row());
	let other = simple_child(&mut tree, root, Style::row());
	let leaf = simple_child(
		&mut tree,
		branch,
		Style {
			width: points(10.0),
			height: points(10.0),
			..Style::default()
		},
	);
	let sibling = simple_child(
		&mut tree,
		other,
		Style {
			width: points(30.0),
			height: points(30.0),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(300.0, 300.0));
	tree.calculate_layout(root, definite(200.0, 300.0));
	assert_eq!(tree.layout(branch).width, 200.0);
	tree.calculate_layout(root, definite(300.0, 300.0));
	assert_eq!(tree.layout(branch).width, 300.0);
	assert_eq!(tree.layout(other).y, 10.0);

	tree.set_style(
		leaf,
		Style {
			width: points(10.0),
			height: points(50.0),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(300.0, 300.0));
	assert_eq!(tree.layout(branch).height, 50.0);
	assert_eq!(tree.layout(other).y, 50.0);
	assert_eq!(
		(tree.layout(sibling).width, tree.layout(sibling).height),
		(30.0, 30.0)
	);
}

#[test]
fn flex_free_space_subtracts_padding_once() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(200.0),
			padding: leptoskit_yoga::Rect::new(|| points(10.0)),
			..Style::row()
		},
	);
	let child = simple_child(
		&mut tree,
		root,
		Style {
			flex_grow: 1.0,
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(200.0, 200.0));

	assert_eq!(tree.layout(child).width, 180.0);
}

#[test]
fn flex_free_space_uses_the_container_own_size() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(400.0),
			..Style::default()
		},
	);
	let row = simple_child(
		&mut tree,
		root,
		Style {
			width: points(100.0),
			padding: leptoskit_yoga::Rect::new(|| points(5.0)),
			align_self: Align::FlexStart,
			..Style::row()
		},
	);
	let grow = simple_child(
		&mut tree,
		row,
		Style {
			flex_grow: 1.0,
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(400.0, 400.0));

	assert_eq!(tree.layout(row).width, 100.0);
	assert_eq!(tree.layout(grow).width, 90.0);
}

#[test]
fn percentages_resolve_against_the_flex_container() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(400.0),
			height: points(400.0),
			..Style::row()
		},
	);
	let row = simple_child(
		&mut tree,
		root,
		Style {
			width: points(200.0),
			..Style::row()
		},
	);
	let half = simple_child(
		&mut tree,
		row,
		Style {
			width: Dimension::Percent(0.5),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(400.0, 400.0));

	assert_eq!(tree.layout(half).width, 100.0);
}

#[test]
fn nowrap_line_fills_the_container_and_ignores_align_content() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(300.0),
			height: points(300.0),
			align_content: Align::FlexEnd,
			align_items: Align::FlexStart,
			..Style::row()
		},
	);
	let packed = simple_child(
		&mut tree,
		root,
		Style {
			width: points(10.0),
			height: points(10.0),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(300.0, 300.0));
	assert_eq!(tree.layout(packed).y, 0.0);

	let column = tree.create_node();
	tree.set_style(
		column,
		Style {
			width: points(300.0),
			align_content: Align::FlexStart,
			..Style::default()
		},
	);
	let stretched = simple_child(
		&mut tree,
		column,
		Style {
			height: points(10.0),
			..Style::default()
		},
	);
	tree.calculate_layout(column, definite(300.0, 300.0));
	assert_eq!(tree.layout(stretched).width, 300.0);
}

#[test]
fn reversed_main_axes_lead_with_the_main_start_margin() {
	let fixed = |margin: leptoskit_yoga::Rect<Dimension>| {
		Style {
			width: points(10.0),
			height: points(10.0),
			margin,
			..Style::default()
		}
	};
	let zero = points(0.0);

	// Row-reverse packs at the right; the left margin trails.
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(100.0),
			height: points(100.0),
			flex_direction: leptoskit_yoga::FlexDirection::RowReverse,
			..Style::default()
		},
	);
	let child = simple_child(
		&mut tree,
		root,
		fixed(leptoskit_yoga::Rect::all_edges(
			zero,
			zero,
			zero,
			points(5.0),
		)),
	);
	tree.calculate_layout(root, definite(100.0, 100.0));
	assert_eq!(tree.layout(child).x, 90.0);

	// RTL rows lead with the right margin.
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(100.0),
			direction: leptoskit_yoga::Direction::Rtl,
			..Style::row()
		},
	);
	let first = simple_child(
		&mut tree,
		root,
		fixed(leptoskit_yoga::Rect::all_edges(
			zero,
			points(5.0),
			zero,
			points(3.0),
		)),
	);
	let second = simple_child(&mut tree, root, fixed(leptoskit_yoga::Rect::new(|| zero)));
	tree.calculate_layout(root, definite(100.0, 100.0));
	assert_eq!(tree.layout(first).x, 85.0);
	assert_eq!(tree.layout(second).x, 72.0);

	// Column-reverse packs at the bottom; the top margin trails.
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(100.0),
			height: points(100.0),
			flex_direction: leptoskit_yoga::FlexDirection::ColumnReverse,
			..Style::default()
		},
	);
	let child = simple_child(
		&mut tree,
		root,
		fixed(leptoskit_yoga::Rect::all_edges(
			points(5.0),
			zero,
			zero,
			zero,
		)),
	);
	tree.calculate_layout(root, definite(100.0, 100.0));
	assert_eq!(tree.layout(child).y, 90.0);
}

#[test]
fn layout_order_is_the_in_flow_index() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: points(100.0),
			flex_wrap: leptoskit_yoga::FlexWrap::Wrap,
			..Style::row()
		},
	);
	let cells: Vec<NodeId> = (0..3)
		.map(|_| {
			simple_child(
				&mut tree,
				root,
				Style {
					width: points(60.0),
					height: points(10.0),
					..Style::default()
				},
			)
		})
		.collect();
	tree.calculate_layout(root, definite(100.0, 100.0));
	let orders: Vec<u32> = cells.iter().map(|&cell| tree.layout(cell).order).collect();
	assert_eq!(orders, vec![0, 1, 2]);

	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		grid_style(vec![TrackSizing::Points(100.0); 2], vec![]),
	);
	let cells: Vec<NodeId> = (0..3)
		.map(|_| simple_child(&mut tree, root, Style::default()))
		.collect();
	tree.calculate_layout(root, definite(300.0, 200.0));
	let orders: Vec<u32> = cells.iter().map(|&cell| tree.layout(cell).order).collect();
	assert_eq!(orders, vec![0, 1, 2]);
}

#[test]
#[should_panic(expected = "cycle")]
fn inserting_a_node_under_itself_panics() {
	let mut tree = Yoga::new();
	let node = tree.create_node();
	tree.insert_child(node, node, 0);
}

#[test]
#[should_panic(expected = "cycle")]
fn inserting_an_ancestor_under_its_descendant_panics() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	let child = simple_child(&mut tree, root, Style::default());
	let grandchild = simple_child(&mut tree, child, Style::default());
	tree.insert_child(grandchild, root, 0);
}

#[test]
#[should_panic(expected = "cycle")]
fn set_children_rejects_an_ancestor() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	let child = simple_child(&mut tree, root, Style::default());
	tree.set_children(child, [root]);
}

#[test]
fn grid_partly_auto_item_keeps_its_definite_row() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			display: Display::Grid,
			grid: Some(GridStyle {
				template_columns: vec![TrackSizing::Points(100.0); 2],
				template_rows: vec![TrackSizing::Points(20.0); 3],
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	simple_child(&mut tree, root, Style::default());
	let row_locked = simple_child(
		&mut tree,
		root,
		Style {
			grid: Some(GridStyle {
				row: GridPlacement {
					start: GridLine::Line(3),
					end: GridLine::Auto,
				},
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	let column_locked = simple_child(
		&mut tree,
		root,
		Style {
			grid: Some(GridStyle {
				column: GridPlacement {
					start: GridLine::Line(1),
					end: GridLine::Auto,
				},
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(300.0, 300.0));

	assert_eq!(
		(tree.layout(row_locked).x, tree.layout(row_locked).y),
		(0.0, 40.0)
	);
	// The column-locked item stays in column one, below the first auto item.
	assert_eq!(
		(tree.layout(column_locked).x, tree.layout(column_locked).y),
		(0.0, 20.0)
	);
}

#[test]
fn grid_rows_measure_items_at_their_column_width() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			display: Display::Grid,
			align_content: Align::FlexStart,
			grid: Some(GridStyle {
				template_columns: vec![TrackSizing::Points(80.0)],
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	let calls = std::sync::Arc::new(AtomicUsize::new(0));
	let text = wrapping_text_leaf(&mut tree, &calls);
	tree.insert_child(root, text, 0);
	tree.calculate_layout(root, definite(300.0, 300.0));

	// 200 units of text wrap onto three lines in an 80-wide column.
	assert_eq!(tree.layout(text).height, 30.0);
	assert_eq!(tree.layout(root).height, 30.0);
}

#[test]
fn grid_minmax_grows_to_its_maximum() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			display: Display::Grid,
			width: points(300.0),
			grid: Some(GridStyle {
				template_columns: vec![leptoskit_yoga::minmax(100.0, 200.0)],
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	let item = simple_child(&mut tree, root, Style::default());
	tree.calculate_layout(root, definite(300.0, 300.0));

	assert_eq!(tree.layout(item).width, 200.0);
}

#[test]
fn grid_flattens_display_contents_children() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			display: Display::Grid,
			grid: Some(GridStyle {
				template_columns: vec![TrackSizing::Points(50.0); 2],
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	let first = simple_child(
		&mut tree,
		root,
		Style {
			height: points(50.0),
			..Style::default()
		},
	);
	let contents = simple_child(
		&mut tree,
		root,
		Style {
			display: Display::Contents,
			..Style::default()
		},
	);
	let nested = simple_child(
		&mut tree,
		contents,
		Style {
			height: points(50.0),
			..Style::default()
		},
	);
	tree.calculate_layout(root, max_content());

	assert_eq!(tree.layout(first).x, 0.0);
	assert_eq!(
		(tree.layout(nested).x, tree.layout(nested).width),
		(50.0, 50.0)
	);
	assert_eq!(
		(tree.layout(root).width, tree.layout(root).height),
		(100.0, 50.0)
	);
}

#[test]
fn grid_huge_line_numbers_are_clamped() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			display: Display::Grid,
			grid: Some(GridStyle {
				auto_columns: TrackSizing::Points(1.0),
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	let item = simple_child(
		&mut tree,
		root,
		Style {
			grid: Some(GridStyle {
				column: GridPlacement::line(50_000_000, 50_000_001),
				row: GridPlacement::line(1, 2),
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	tree.calculate_layout(root, max_content());

	// Line numbers clamp to 10 000, so the item sits in the last allowed track.
	assert_eq!(tree.layout(item).x, 9_999.0);
}

#[test]
fn grid_negative_line_counts_from_the_last_explicit_line() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		grid_style(vec![TrackSizing::Points(100.0); 2], vec![]),
	);
	let item = simple_child(
		&mut tree,
		root,
		Style {
			grid: Some(GridStyle {
				column: GridPlacement::line(1, -1),
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	tree.calculate_layout(root, definite(300.0, 200.0));

	assert_eq!(tree.layout(item).width, 200.0);
}

#[test]
fn end_and_center_justification_overflow_the_start_like_css() {
	use leptoskit_yoga::JustifyContent;

	for (justify, expected) in [
		(JustifyContent::FlexEnd, -50.0),
		(JustifyContent::Center, -25.0),
		(JustifyContent::SpaceBetween, 0.0),
		(JustifyContent::SpaceAround, 0.0),
	] {
		let mut tree = Yoga::new();
		let root = tree.create_node();
		tree.set_style(
			root,
			Style {
				height: points(100.0),
				justify_content: justify,
				..Style::default()
			},
		);
		let child = simple_child(
			&mut tree,
			root,
			Style {
				height: points(150.0),
				flex_shrink: 0.0,
				..Style::default()
			},
		);
		tree.calculate_layout(root, definite(100.0, 100.0));
		assert_eq!(tree.layout(child).y, expected, "{justify:?}");
	}
}

/// A page root: an `auto` width fills the available width less its margins,
/// within its min and max widths, while its height follows its content.
#[test]
fn a_filling_root_takes_the_available_width_within_its_constraints() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	simple_child(
		&mut tree,
		root,
		Style {
			width: points(40.0),
			height: points(30.0),
			..Style::default()
		},
	);

	tree.calculate_layout_filling_width(root, definite(320.0, 480.0));
	let layout = tree.layout(root);
	assert_eq!((layout.width, layout.height), (320.0, 30.0));

	tree.calculate_layout(root, definite(320.0, 480.0));
	assert_eq!(
		tree.layout(root).width,
		40.0,
		"the ordinary root wraps its content"
	);

	tree.set_style(
		root,
		Style {
			margin: leptoskit_yoga::Rect {
				top: points(0.0),
				right: points(10.0),
				bottom: points(0.0),
				left: points(20.0),
			},
			max_width: points(200.0),
			..Style::default()
		},
	);
	tree.calculate_layout_filling_width(root, definite(320.0, 480.0));
	assert_eq!(
		tree.layout(root).width,
		200.0,
		"margins come off first, then the maximum caps the width"
	);

	tree.set_style(
		root,
		Style {
			width: points(100.0),
			..Style::default()
		},
	);
	tree.calculate_layout_filling_width(root, definite(320.0, 480.0));
	assert_eq!(tree.layout(root).width, 100.0, "an explicit width wins");
}
