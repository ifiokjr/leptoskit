//! Layout engine integration tests, in layout units (logical pixels).

// Layout outputs are exact floats: the expected values are whole numbers
// produced by the same arithmetic the engine runs, not measured quantities.
#![allow(clippy::float_cmp)]

use leptoskit_yoga::Align;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Display;
use leptoskit_yoga::GridAutoFlow;
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

	// The min bound holds the track open even though the content is small, so
	// the second item starts at the track edge rather than hugging the first.
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
