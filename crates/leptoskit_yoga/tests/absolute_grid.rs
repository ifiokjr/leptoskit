//! Absolute grid containing blocks follow CSS Grid §9.1 after track sizing.
#![allow(clippy::float_cmp)]

use leptoskit_yoga::Align;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Direction;
use leptoskit_yoga::Display;
use leptoskit_yoga::GridLine;
use leptoskit_yoga::GridPlacement;
use leptoskit_yoga::GridStyle;
use leptoskit_yoga::JustifyContent;
use leptoskit_yoga::NodeId;
use leptoskit_yoga::PositionType;
use leptoskit_yoga::Rect;
use leptoskit_yoga::Size;
use leptoskit_yoga::Style;
use leptoskit_yoga::TrackSizing;
use leptoskit_yoga::Yoga;

#[test]
fn explicit_grid_area_excludes_padding_borders_and_adjacent_gaps() {
	let (mut yoga, root) = grid();
	let first = child(
		&mut yoga,
		root,
		GridPlacement::line(1, 2),
		GridPlacement::line(1, 2),
	);
	let second = child(
		&mut yoga,
		root,
		GridPlacement::line(2, 3),
		GridPlacement::line(2, 3),
	);
	calculate(&mut yoga, root);
	assert_frame(&yoga, first, (15.0, 7.0, 60.0, 30.0));
	assert_frame(&yoga, second, (85.0, 47.0, 80.0, 50.0));
}

#[test]
fn auto_edges_use_the_corresponding_padding_edge_independently() {
	for (placement, expected_x, expected_width) in [
		(GridPlacement::AUTO, 2.0, 196.0),
		(
			GridPlacement {
				start: GridLine::Line(2),
				end: GridLine::Auto,
			},
			85.0,
			113.0,
		),
		(
			GridPlacement {
				start: GridLine::Auto,
				end: GridLine::Line(2),
			},
			2.0,
			73.0,
		),
	] {
		let (mut yoga, root) = grid();
		let item = child(&mut yoga, root, placement, GridPlacement::AUTO);
		calculate(&mut yoga, root);
		assert_frame(&yoga, item, (expected_x, 2.0, expected_width, 136.0));
	}
}

#[test]
fn spans_resolve_from_an_anchor_and_unanchored_spans_use_padding_edges() {
	for placement in [
		GridPlacement {
			start: GridLine::Line(2),
			end: GridLine::Span(1),
		},
		GridPlacement {
			start: GridLine::Span(1),
			end: GridLine::Line(3),
		},
		GridPlacement::line(3, 2),
		GridPlacement::line(2, 2),
	] {
		let (mut yoga, root) = grid();
		let item = child(&mut yoga, root, placement, GridPlacement::line(2, 3));
		calculate(&mut yoga, root);
		assert_frame(&yoga, item, (85.0, 47.0, 80.0, 50.0));
	}

	for placement in [
		GridPlacement::span(2),
		GridPlacement {
			start: GridLine::Span(2),
			end: GridLine::Auto,
		},
		GridPlacement {
			start: GridLine::Span(2),
			end: GridLine::Span(1),
		},
	] {
		let (mut yoga, root) = grid();
		let item = child(&mut yoga, root, placement, placement);
		calculate(&mut yoga, root);
		assert_frame(&yoga, item, (2.0, 2.0, 196.0, 136.0));
	}
}

#[test]
fn nonexistent_lines_and_spans_fall_back_without_creating_tracks() {
	for placement in [
		GridPlacement::line(100, 101),
		GridPlacement::line(-100, -99),
	] {
		let (mut yoga, root) = grid();
		let item = child(&mut yoga, root, placement, placement);
		calculate(&mut yoga, root);
		assert_frame(&yoga, item, (2.0, 2.0, 196.0, 136.0));
		assert_eq!(yoga.layout(root).content_width, 200.0);
	}

	for (placement, expected_x, expected_width) in [
		(
			GridPlacement {
				start: GridLine::Line(2),
				end: GridLine::Span(10),
			},
			85.0,
			113.0,
		),
		(
			GridPlacement {
				start: GridLine::Span(10),
				end: GridLine::Line(2),
			},
			2.0,
			73.0,
		),
		(GridPlacement::line(3, 3), 165.0, 33.0),
		(
			GridPlacement {
				start: GridLine::Line(-5),
				end: GridLine::Span(4),
			},
			2.0,
			163.0,
		),
		(
			GridPlacement {
				start: GridLine::Span(4),
				end: GridLine::Line(5),
			},
			15.0,
			183.0,
		),
	] {
		let (mut yoga, root) = grid();
		let item = child(&mut yoga, root, placement, GridPlacement::AUTO);
		calculate(&mut yoga, root);
		assert_frame(&yoga, item, (expected_x, 2.0, expected_width, 136.0));
	}
}

#[test]
fn existing_implicit_lines_are_valid_but_negative_lines_anchor_to_explicit_tracks() {
	let (mut yoga, root) = grid();
	let in_flow = yoga.create_node();
	yoga.set_style(
		in_flow,
		Style {
			grid: Some(GridStyle {
				column: GridPlacement::line(3, 4),
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	yoga.insert_child(root, in_flow, 0);
	let implicit = child(
		&mut yoga,
		root,
		GridPlacement::line(3, 4),
		GridPlacement::line(1, 2),
	);
	let negative = child(
		&mut yoga,
		root,
		GridPlacement::line(-2, -1),
		GridPlacement::line(-2, -1),
	);
	calculate(&mut yoga, root);
	assert_frame(&yoga, implicit, (175.0, 7.0, 25.0, 30.0));
	assert_frame(&yoga, negative, (85.0, 47.0, 80.0, 50.0));
}

#[test]
fn percentages_and_opposing_insets_resolve_against_the_grid_area() {
	let (mut yoga, root) = grid();
	let item = child(
		&mut yoga,
		root,
		GridPlacement::line(2, 3),
		GridPlacement::line(2, 3),
	);
	let mut style = yoga.style(item).clone();
	style.inset = Rect::all_edges(
		Dimension::Percent(0.1),
		Dimension::Percent(0.1),
		Dimension::Auto,
		Dimension::Percent(0.1),
	);
	style.height = Dimension::Percent(0.5);
	yoga.set_style(item, style);
	calculate(&mut yoga, root);
	assert_frame(&yoga, item, (93.0, 52.0, 64.0, 25.0));
}

#[test]
fn grid_alignment_and_distributed_spacing_move_absolute_containing_blocks() {
	for (justify, align, expected) in [
		(
			JustifyContent::Center,
			Align::Center,
			(98.0, 62.0, 80.0, 50.0),
		),
		(
			JustifyContent::SpaceBetween,
			Align::SpaceBetween,
			(111.0, 77.0, 80.0, 50.0),
		),
	] {
		let (mut yoga, root) = grid();
		let mut style = yoga.style(root).clone();
		style.justify_content = justify;
		style.align_content = align;
		yoga.set_style(root, style);
		let item = child(
			&mut yoga,
			root,
			GridPlacement::line(2, 3),
			GridPlacement::line(2, 3),
		);
		let span = child(
			&mut yoga,
			root,
			GridPlacement::line(1, 3),
			GridPlacement::line(1, 3),
		);
		calculate(&mut yoga, root);
		assert_frame(&yoga, item, expected);
		let span_layout = yoga.layout(span);
		assert_eq!(
			span_layout.width,
			if justify == JustifyContent::Center {
				150.0
			} else {
				176.0
			}
		);
		assert_eq!(
			span_layout.height,
			if align == Align::Center { 90.0 } else { 120.0 }
		);
	}
}

#[test]
fn rtl_grid_lines_are_logical_and_insets_remain_physical() {
	let (mut yoga, root) = grid();
	let mut root_style = yoga.style(root).clone();
	root_style.direction = Direction::Rtl;
	yoga.set_style(root, root_style);
	let item = child(
		&mut yoga,
		root,
		GridPlacement::line(2, 3),
		GridPlacement::line(2, 3),
	);
	let mut style = yoga.style(item).clone();
	style.inset.left = Dimension::Points(4.0);
	style.inset.right = Dimension::Points(6.0);
	yoga.set_style(item, style);
	calculate(&mut yoga, root);
	assert_frame(&yoga, item, (45.0, 47.0, 70.0, 50.0));
}

#[test]
fn absolute_grid_children_do_not_contribute_to_intrinsic_container_size() {
	let (mut yoga, root) = grid();
	let mut style = yoga.style(root).clone();
	style.width = Dimension::Auto;
	style.height = Dimension::Auto;
	yoga.set_style(root, style);
	let item = child(
		&mut yoga,
		root,
		GridPlacement::line(1, 2),
		GridPlacement::line(1, 2),
	);
	let mut style = yoga.style(item).clone();
	style.width = Dimension::Points(500.0);
	style.height = Dimension::Points(500.0);
	yoga.set_style(item, style);
	calculate(&mut yoga, root);
	assert_eq!(
		(yoga.layout(root).width, yoga.layout(root).height),
		(174.0, 110.0)
	);
}

#[test]
fn self_alignment_positions_definite_boxes_with_static_or_zero_insets() {
	for direction in [Direction::Ltr, Direction::Rtl] {
		for insets in [
			Rect::default(),
			Rect::all_edges(
				Dimension::Points(0.0),
				Dimension::Points(0.0),
				Dimension::Points(0.0),
				Dimension::Points(0.0),
			),
		] {
			for (alignment, ltr_x, rtl_x, y) in [
				(Align::FlexStart, 85.0, 101.0, 47.0),
				(Align::Center, 115.0, 71.0, 67.0),
				(Align::FlexEnd, 145.0, 41.0, 87.0),
			] {
				let (mut yoga, root) = grid();
				let mut parent = yoga.style(root).clone();
				parent.direction = direction;
				yoga.set_style(root, parent);
				let item = child(
					&mut yoga,
					root,
					GridPlacement::line(2, 3),
					GridPlacement::line(2, 3),
				);
				let mut style = yoga.style(item).clone();
				style.width = Dimension::Points(20.0);
				style.height = Dimension::Points(10.0);
				style.inset = insets;
				style.align_self = alignment;
				style.grid.as_mut().expect("placement").justify_self = alignment;
				yoga.set_style(item, style);
				calculate(&mut yoga, root);
				assert_frame(
					&yoga,
					item,
					(
						if direction == Direction::Ltr {
							ltr_x
						} else {
							rtl_x
						},
						y,
						20.0,
						10.0,
					),
				);
			}
		}
	}
}

#[test]
fn static_alignment_resolves_logical_start_and_explicit_margins() {
	for (direction, alignment, expected_x, expected_y) in [
		(Direction::Ltr, Align::Auto, 94.0, 50.0),
		(Direction::Rtl, Align::Auto, 96.0, 50.0),
		(Direction::Ltr, Align::Center, 117.0, 65.0),
		(Direction::Rtl, Align::Center, 73.0, 65.0),
		(Direction::Ltr, Align::FlexEnd, 140.0, 80.0),
		(Direction::Rtl, Align::FlexEnd, 50.0, 80.0),
	] {
		let (mut yoga, root) = grid();
		let mut parent = yoga.style(root).clone();
		parent.direction = direction;
		yoga.set_style(root, parent);
		let item = child(
			&mut yoga,
			root,
			GridPlacement::line(2, 3),
			GridPlacement::line(2, 3),
		);
		let mut style = yoga.style(item).clone();
		style.width = Dimension::Points(20.0);
		style.height = Dimension::Points(10.0);
		style.inset = Rect::default();
		style.margin = Rect::all_edges(
			Dimension::Points(3.0),
			Dimension::Points(5.0),
			Dimension::Points(7.0),
			Dimension::Points(9.0),
		);
		style.align_self = alignment;
		style.grid.as_mut().expect("placement").justify_self = alignment;
		yoga.set_style(item, style);
		calculate(&mut yoga, root);
		assert_frame(&yoga, item, (expected_x, expected_y, 20.0, 10.0));
	}
}

#[test]
fn oversized_positioned_boxes_keep_negative_free_space_when_aligned() {
	for (direction, alignment, x, y) in [
		(Direction::Ltr, Align::Center, 75.0, 37.0),
		(Direction::Ltr, Align::FlexEnd, 65.0, 27.0),
		(Direction::Rtl, Align::Center, 31.0, 37.0),
		(Direction::Rtl, Align::FlexEnd, 41.0, 27.0),
	] {
		for insets in [
			Rect::default(),
			Rect::all_edges(
				Dimension::Points(0.0),
				Dimension::Points(0.0),
				Dimension::Points(0.0),
				Dimension::Points(0.0),
			),
		] {
			let (mut yoga, root) = grid();
			let mut parent = yoga.style(root).clone();
			parent.direction = direction;
			yoga.set_style(root, parent);
			let item = child(
				&mut yoga,
				root,
				GridPlacement::line(2, 3),
				GridPlacement::line(2, 3),
			);
			let mut style = yoga.style(item).clone();
			style.width = Dimension::Points(100.0);
			style.height = Dimension::Points(70.0);
			style.inset = insets;
			style.align_self = alignment;
			style.grid.as_mut().expect("placement").justify_self = alignment;
			yoga.set_style(item, style);
			calculate(&mut yoga, root);
			assert_frame(&yoga, item, (x, y, 100.0, 70.0));
		}
	}
}

#[test]
fn self_alignment_keeps_auto_dimensions_fitted_to_content() {
	let (mut yoga, root) = grid();
	let item = child(
		&mut yoga,
		root,
		GridPlacement::line(2, 3),
		GridPlacement::line(2, 3),
	);
	let mut style = yoga.style(item).clone();
	style.align_self = Align::Center;
	style.grid.as_mut().expect("placement").justify_self = Align::Center;
	yoga.set_style(item, style);
	let content = yoga.create_node();
	yoga.set_style(
		content,
		Style {
			width: Dimension::Points(20.0),
			height: Dimension::Points(10.0),
			..Style::default()
		},
	);
	yoga.insert_child(item, content, 0);
	calculate(&mut yoga, root);
	assert_frame(&yoga, item, (115.0, 67.0, 20.0, 10.0));
}

#[test]
fn one_auto_inset_retains_physical_position_regardless_of_self_alignment() {
	let (mut yoga, root) = grid();
	let item = child(
		&mut yoga,
		root,
		GridPlacement::line(2, 3),
		GridPlacement::line(2, 3),
	);
	let mut style = yoga.style(item).clone();
	style.width = Dimension::Points(20.0);
	style.height = Dimension::Points(10.0);
	style.inset = Rect::all_edges(
		Dimension::Points(6.0),
		Dimension::Auto,
		Dimension::Auto,
		Dimension::Points(4.0),
	);
	style.align_self = Align::Center;
	style.grid.as_mut().expect("placement").justify_self = Align::Center;
	yoga.set_style(item, style);
	calculate(&mut yoga, root);
	assert_frame(&yoga, item, (89.0, 53.0, 20.0, 10.0));
}

#[test]
fn auto_grid_edges_use_the_padding_box_for_static_position_and_percentages() {
	let (mut yoga, root) = grid();
	let item = child(&mut yoga, root, GridPlacement::AUTO, GridPlacement::AUTO);
	let mut style = yoga.style(item).clone();
	style.width = Dimension::Percent(0.5);
	style.height = Dimension::Points(10.0);
	style.inset = Rect::default();
	yoga.set_style(item, style);
	calculate(&mut yoga, root);
	assert_frame(&yoga, item, (2.0, 2.0, 98.0, 10.0));
}

#[test]
fn fractional_intrinsic_grid_does_not_fill_a_bounded_nonstretch_parent() {
	let mut yoga = Yoga::new();
	let root = yoga.create_node();
	yoga.set_style(
		root,
		Style {
			width: Dimension::Points(400.0),
			height: Dimension::Points(300.0),
			align_items: Align::FlexStart,
			..Style::default()
		},
	);
	let grid = yoga.create_node();
	let track = TrackSizing::MinMax(Box::new(leptoskit_yoga::MinMax {
		min: TrackSizing::Points(0.0),
		max: TrackSizing::Fraction(1.0),
	}));
	yoga.set_style(
		grid,
		Style {
			display: Display::Grid,
			grid: Some(GridStyle {
				template_columns: vec![track.clone()],
				template_rows: vec![track],
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	yoga.insert_child(root, grid, 0);
	for (width, height) in [(80.0, 30.0), (40.0, 60.0)] {
		let node = yoga.create_node();
		yoga.set_style(
			node,
			Style {
				width: Dimension::Points(width),
				height: Dimension::Points(height),
				grid: Some(GridStyle {
					column: GridPlacement::line(1, 2),
					row: GridPlacement::line(1, 2),
					..GridStyle::default()
				}),
				..Style::default()
			},
		);
		yoga.insert_child(grid, node, yoga.children(grid).len());
	}
	calculate(&mut yoga, root);
	assert_frame(&yoga, grid, (0.0, 0.0, 80.0, 60.0));
}

#[test]
fn changing_an_in_flow_item_to_absolute_clears_its_old_flow_order() {
	let (mut yoga, root) = grid();
	for _ in 0..2 {
		let node = yoga.create_node();
		yoga.insert_child(root, node, yoga.children(root).len());
	}
	let node = yoga.children(root)[1];
	calculate(&mut yoga, root);
	assert_eq!(yoga.layout(node).order, 1);
	let mut style = yoga.style(node).clone();
	style.position = PositionType::Absolute;
	yoga.set_style(node, style);
	calculate(&mut yoga, root);
	assert_eq!(yoga.layout(node).order, 0);
}

#[test]
fn intrinsic_percentage_rows_measure_content_before_resolving_used_tracks() {
	for bounded_parent in [false, true] {
		let mut yoga = Yoga::new();
		let grid = yoga.create_node();
		yoga.set_style(
			grid,
			Style {
				display: Display::Grid,
				width: Dimension::Points(80.0),
				grid: Some(GridStyle {
					template_rows: vec![TrackSizing::Percent(0.5)],
					..GridStyle::default()
				}),
				..Style::default()
			},
		);
		let content = yoga.create_node();
		yoga.set_style(
			content,
			Style {
				width: Dimension::Points(20.0),
				height: Dimension::Points(40.0),
				..Style::default()
			},
		);
		yoga.insert_child(grid, content, 0);
		let marker = child(
			&mut yoga,
			grid,
			GridPlacement::line(1, 2),
			GridPlacement::line(1, 2),
		);
		let root = if bounded_parent {
			let parent = yoga.create_node();
			yoga.set_style(
				parent,
				Style {
					width: Dimension::Points(400.0),
					height: Dimension::Points(300.0),
					align_items: Align::FlexStart,
					..Style::default()
				},
			);
			yoga.insert_child(parent, grid, 0);
			parent
		} else {
			grid
		};
		calculate(&mut yoga, root);
		assert_frame(&yoga, grid, (0.0, 0.0, 80.0, 40.0));
		assert_eq!(yoga.layout(content).height, 40.0);
		assert_eq!(yoga.layout(marker).height, 20.0);
	}
}

#[test]
fn intrinsic_percentage_columns_and_fractional_tracks_resolve_after_container_size() {
	let mut yoga = Yoga::new();
	let root = yoga.create_node();
	yoga.set_style(
		root,
		Style {
			display: Display::Grid,
			grid: Some(GridStyle {
				template_columns: vec![
					TrackSizing::Percent(0.9),
					TrackSizing::MinMax(Box::new(leptoskit_yoga::MinMax {
						min: TrackSizing::Points(0.0),
						max: TrackSizing::Fraction(1.0),
					})),
				],
				template_rows: vec![TrackSizing::Points(20.0)],
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	for column in [GridPlacement::line(1, 2), GridPlacement::line(2, 3)] {
		let content = yoga.create_node();
		yoga.set_style(
			content,
			Style {
				width: Dimension::Points(80.0),
				height: Dimension::Points(20.0),
				grid: Some(GridStyle {
					column,
					..GridStyle::default()
				}),
				..Style::default()
			},
		);
		yoga.insert_child(root, content, yoga.children(root).len());
	}
	let first = child(
		&mut yoga,
		root,
		GridPlacement::line(1, 2),
		GridPlacement::line(1, 2),
	);
	let second = child(
		&mut yoga,
		root,
		GridPlacement::line(2, 3),
		GridPlacement::line(1, 2),
	);
	calculate(&mut yoga, root);
	assert_frame(&yoga, root, (0.0, 0.0, 160.0, 20.0));
	assert_eq!(yoga.layout(first).width, 144.0);
	assert_eq!(yoga.layout(second).width, 16.0);
}

#[test]
fn intrinsic_minmax_percentages_keep_auto_minimums_and_auto_maximums() {
	for (min, max, intrinsic_height, used_height) in [
		(
			TrackSizing::Points(10.0),
			TrackSizing::Percent(0.5),
			40.0,
			20.0,
		),
		(
			TrackSizing::Percent(0.5),
			TrackSizing::Points(50.0),
			50.0,
			50.0,
		),
		(TrackSizing::Auto, TrackSizing::Percent(0.5), 40.0, 40.0),
		(TrackSizing::Percent(0.5), TrackSizing::Auto, 40.0, 40.0),
	] {
		let mut yoga = Yoga::new();
		let root = yoga.create_node();
		yoga.set_style(
			root,
			Style {
				display: Display::Grid,
				width: Dimension::Points(80.0),
				grid: Some(GridStyle {
					template_rows: vec![TrackSizing::MinMax(Box::new(leptoskit_yoga::MinMax {
						min,
						max,
					}))],
					..GridStyle::default()
				}),
				..Style::default()
			},
		);
		let content = yoga.create_node();
		yoga.set_style(
			content,
			Style {
				width: Dimension::Points(20.0),
				height: Dimension::Points(40.0),
				..Style::default()
			},
		);
		yoga.insert_child(root, content, 0);
		let marker = child(
			&mut yoga,
			root,
			GridPlacement::line(1, 2),
			GridPlacement::line(1, 2),
		);
		calculate(&mut yoga, root);
		assert_eq!(yoga.layout(root).height, intrinsic_height);
		assert_eq!(yoga.layout(marker).height, used_height);
	}
}

#[test]
fn intrinsic_percentage_tracks_resolve_against_the_settled_content_box() {
	let (mut yoga, root) = grid();
	let mut style = yoga.style(root).clone();
	style.height = Dimension::Auto;
	style.grid = Some(GridStyle {
		template_rows: vec![TrackSizing::Percent(0.5)],
		..GridStyle::default()
	});
	style.gap = Rect::default();
	yoga.set_style(root, style);
	let content = yoga.create_node();
	yoga.set_style(
		content,
		Style {
			width: Dimension::Points(20.0),
			height: Dimension::Points(40.0),
			..Style::default()
		},
	);
	yoga.insert_child(root, content, 0);
	let marker = child(
		&mut yoga,
		root,
		GridPlacement::line(1, 2),
		GridPlacement::line(1, 2),
	);
	calculate(&mut yoga, root);
	assert_eq!(yoga.layout(root).height, 60.0);
	assert_eq!(yoga.layout(marker).height, 20.0);
	assert_eq!(yoga.layout(marker).y, 7.0);
}

fn grid() -> (Yoga, NodeId) {
	let mut yoga = Yoga::new();
	let root = yoga.create_node();
	yoga.set_style(
		root,
		Style {
			display: Display::Grid,
			width: Dimension::Points(200.0),
			height: Dimension::Points(140.0),
			padding: Rect::all_edges(
				Dimension::Points(5.0),
				Dimension::Points(7.0),
				Dimension::Points(11.0),
				Dimension::Points(13.0),
			),
			border: Rect::all_edges(2.0, 2.0, 2.0, 2.0),
			gap: Rect::all_edges(
				Dimension::Points(10.0),
				Dimension::Points(10.0),
				Dimension::Points(10.0),
				Dimension::Points(10.0),
			),
			grid: Some(GridStyle {
				template_columns: vec![TrackSizing::Points(60.0), TrackSizing::Points(80.0)],
				template_rows: vec![TrackSizing::Points(30.0), TrackSizing::Points(50.0)],
				auto_columns: TrackSizing::Points(25.0),
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	(yoga, root)
}

fn child(yoga: &mut Yoga, parent: NodeId, column: GridPlacement, row: GridPlacement) -> NodeId {
	let node = yoga.create_node();
	yoga.set_style(
		node,
		Style {
			position: PositionType::Absolute,
			inset: Rect::all_edges(
				Dimension::Points(0.0),
				Dimension::Points(0.0),
				Dimension::Points(0.0),
				Dimension::Points(0.0),
			),
			grid: Some(GridStyle {
				column,
				row,
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	yoga.insert_child(parent, node, yoga.children(parent).len());
	node
}

fn calculate(yoga: &mut Yoga, root: NodeId) {
	yoga.calculate_layout(
		root,
		Size::new(
			AvailableSpace::Definite(400.0),
			AvailableSpace::Definite(300.0),
		),
	);
}

fn assert_frame(yoga: &Yoga, node: NodeId, expected: (f32, f32, f32, f32)) {
	let layout = yoga.layout(node);
	assert_eq!((layout.x, layout.y, layout.width, layout.height), expected);
}
