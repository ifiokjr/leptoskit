//! CSS box-model contracts shared by the flex, grid, and positioning paths.
#![allow(clippy::float_cmp)]

use approx::assert_abs_diff_eq;
use leptoskit_yoga::*;

fn add(tree: &mut Yoga, parent: NodeId, style: Style) -> NodeId {
	let child = tree.create_node();
	tree.set_style(child, style);
	tree.insert_child(parent, child, tree.children(parent).len());
	child
}

fn layout(tree: &mut Yoga, root: NodeId, width: f32, height: f32) {
	tree.calculate_layout(
		root,
		Size::new(
			AvailableSpace::Definite(width),
			AvailableSpace::Definite(height),
		),
	);
}

fn decorated() -> Style {
	Style {
		box_sizing: BoxSizing::ContentBox,
		padding: Rect::new(|| Dimension::Points(10.0)),
		border: Rect::new(|| 2.0),
		..Style::default()
	}
}

#[test]
fn content_box_lengths_and_percentages_include_padding_and_border() {
	for display in [Display::Flex, Display::Grid] {
		let mut tree = Yoga::new();
		let root = tree.create_node();
		tree.set_style(
			root,
			Style {
				display,
				width: Dimension::Points(200.0),
				height: Dimension::Points(100.0),
				grid: Some(GridStyle {
					template_columns: vec![TrackSizing::Fraction(1.0)],
					..GridStyle::default()
				}),
				..Style::default()
			},
		);
		let child = add(
			&mut tree,
			root,
			Style {
				width: Dimension::Percent(0.5),
				height: Dimension::Points(20.0),
				..decorated()
			},
		);

		layout(&mut tree, root, 200.0, 100.0);

		assert_eq!(
			(tree.layout(child).width, tree.layout(child).height),
			(124.0, 44.0),
			"{display:?}"
		);
	}
}

#[test]
fn border_box_cannot_be_smaller_than_its_padding_and_border() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: Dimension::Points(4.0),
			height: Dimension::Points(4.0),
			box_sizing: BoxSizing::BorderBox,
			..decorated()
		},
	);

	layout(&mut tree, root, 100.0, 100.0);

	assert_eq!(
		(tree.layout(root).width, tree.layout(root).height),
		(24.0, 24.0)
	);
}

#[test]
fn content_box_minimums_and_maximums_constrain_the_matching_box() {
	for display in [Display::Flex, Display::Grid] {
		let mut tree = Yoga::new();
		let root = tree.create_node();
		tree.set_style(
			root,
			Style {
				display,
				width: Dimension::Points(200.0),
				height: Dimension::Points(100.0),
				..Style::default()
			},
		);
		let child = add(
			&mut tree,
			root,
			Style {
				width: Dimension::Points(200.0),
				max_width: Dimension::Points(50.0),
				height: Dimension::Points(1.0),
				min_height: Dimension::Points(30.0),
				..decorated()
			},
		);

		layout(&mut tree, root, 200.0, 100.0);

		assert_eq!(
			(tree.layout(child).width, tree.layout(child).height),
			(74.0, 54.0),
			"{display:?}"
		);
	}
}

#[test]
fn content_box_maximum_limits_the_measured_content_space() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			max_width: Dimension::Points(50.0),
			..decorated()
		},
	);
	tree.set_measure_func(
		root,
		Some(std::sync::Arc::new(|available| {
			Size::new(available.width.definite().unwrap_or(200.0), 10.0)
		})),
	);

	layout(&mut tree, root, 200.0, 100.0);

	assert_eq!(
		(tree.layout(root).width, tree.layout(root).height),
		(74.0, 34.0)
	);
}

#[test]
fn parent_forced_content_box_sizes_are_not_decorated_twice() {
	for display in [Display::Flex, Display::Grid] {
		let mut tree = Yoga::new();
		let root = tree.create_node();
		tree.set_style(
			root,
			Style {
				display,
				width: Dimension::Points(200.0),
				height: Dimension::Points(100.0),
				grid: Some(GridStyle {
					template_columns: vec![TrackSizing::Fraction(1.0)],
					..GridStyle::default()
				}),
				..Style::default()
			},
		);
		let child = add(
			&mut tree,
			root,
			Style {
				flex_grow: 1.0,
				..decorated()
			},
		);
		let grandchild = add(
			&mut tree,
			child,
			Style {
				width: Dimension::Percent(1.0),
				height: Dimension::Percent(1.0),
				..Style::default()
			},
		);

		layout(&mut tree, root, 200.0, 100.0);

		assert_eq!(
			(tree.layout(child).width, tree.layout(child).height),
			(200.0, 100.0),
			"{display:?}"
		);
		assert_eq!(
			(
				tree.layout(grandchild).x,
				tree.layout(grandchild).y,
				tree.layout(grandchild).width,
				tree.layout(grandchild).height
			),
			(12.0, 12.0, 176.0, 76.0),
			"{display:?}"
		);
	}
}

#[test]
fn flex_basis_uses_the_selected_sizing_box() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: Dimension::Points(300.0),
			height: Dimension::Points(100.0),
			..Style::row()
		},
	);
	let first = add(
		&mut tree,
		root,
		Style {
			flex_basis: Dimension::Points(50.0),
			width: Dimension::Points(200.0),
			..decorated()
		},
	);
	let second = add(
		&mut tree,
		root,
		Style {
			width: Dimension::Points(10.0),
			..Style::default()
		},
	);

	layout(&mut tree, root, 300.0, 100.0);

	assert_eq!(tree.layout(first).width, 74.0);
	assert_eq!(tree.layout(second).x, 74.0);
}

#[test]
fn intrinsic_grid_tracks_include_content_box_decorations() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			display: Display::Grid,
			..Style::default()
		},
	);
	let child = add(
		&mut tree,
		root,
		Style {
			width: Dimension::Points(50.0),
			height: Dimension::Points(20.0),
			..decorated()
		},
	);

	tree.calculate_layout(
		root,
		Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
	);

	assert_eq!(
		(tree.layout(root).width, tree.layout(root).height),
		(74.0, 44.0)
	);
	assert_eq!(
		(tree.layout(child).width, tree.layout(child).height),
		(74.0, 44.0)
	);
}

#[test]
fn intrinsic_containers_use_constrained_child_boxes() {
	for display in [Display::Flex, Display::Grid] {
		let mut tree = Yoga::new();
		let root = tree.create_node();
		tree.set_style(
			root,
			Style {
				display,
				..Style::row()
			},
		);
		let child = add(
			&mut tree,
			root,
			Style {
				width: Dimension::Points(200.0),
				max_width: Dimension::Points(50.0),
				height: Dimension::Points(1.0),
				min_height: Dimension::Points(30.0),
				..decorated()
			},
		);

		tree.calculate_layout(
			root,
			Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
		);

		assert_eq!(
			(tree.layout(root).width, tree.layout(root).height),
			(74.0, 54.0),
			"{display:?}"
		);
		assert_eq!(
			(tree.layout(child).width, tree.layout(child).height),
			(74.0, 54.0),
			"{display:?}"
		);
	}
}

#[test]
fn content_box_aspect_ratio_is_applied_before_constraints() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: Dimension::Points(100.0),
			aspect_ratio: Some(2.0),
			min_height: Dimension::Points(70.0),
			..decorated()
		},
	);

	layout(&mut tree, root, 300.0, 200.0);

	assert_eq!(
		(tree.layout(root).width, tree.layout(root).height),
		(124.0, 94.0)
	);
}

#[test]
fn aspect_ratio_transfers_the_constrained_definite_size() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: Dimension::Points(100.0),
			max_width: Dimension::Points(50.0),
			aspect_ratio: Some(2.0),
			..decorated()
		},
	);

	layout(&mut tree, root, 300.0, 200.0);

	assert_eq!(
		(tree.layout(root).width, tree.layout(root).height),
		(74.0, 49.0)
	);
}

#[test]
fn percentage_padding_uses_containing_width_for_both_axes() {
	for display in [Display::Flex, Display::Grid] {
		let mut tree = Yoga::new();
		let root = tree.create_node();
		tree.set_style(
			root,
			Style {
				display,
				width: Dimension::Points(200.0),
				height: Dimension::Points(100.0),
				grid: Some(GridStyle {
					template_columns: vec![TrackSizing::Fraction(1.0)],
					..GridStyle::default()
				}),
				..Style::default()
			},
		);
		let child = add(
			&mut tree,
			root,
			Style {
				width: Dimension::Percent(0.5),
				height: Dimension::Points(20.0),
				box_sizing: BoxSizing::ContentBox,
				padding: Rect::new(|| Dimension::Percent(0.1)),
				..Style::default()
			},
		);

		layout(&mut tree, root, 200.0, 100.0);

		assert_eq!(
			(tree.layout(child).width, tree.layout(child).height),
			(140.0, 60.0),
			"{display:?}"
		);
	}
}

#[test]
fn signed_margins_move_flex_and_grid_items() {
	for display in [Display::Flex, Display::Grid] {
		let mut tree = Yoga::new();
		let root = tree.create_node();
		tree.set_style(
			root,
			Style {
				display,
				width: Dimension::Points(200.0),
				height: Dimension::Points(100.0),
				grid: Some(GridStyle {
					template_columns: vec![TrackSizing::Fraction(1.0)],
					..GridStyle::default()
				}),
				..Style::default()
			},
		);
		let child = add(
			&mut tree,
			root,
			Style {
				width: Dimension::Points(40.0),
				height: Dimension::Points(20.0),
				margin: Rect::all_edges(
					Dimension::Percent(-0.1),
					Dimension::Points(0.0),
					Dimension::Points(0.0),
					Dimension::Points(-10.0),
				),
				..Style::default()
			},
		);

		layout(&mut tree, root, 200.0, 100.0);

		assert_eq!(
			(tree.layout(child).x, tree.layout(child).y),
			(-10.0, -20.0),
			"{display:?}"
		);
	}
}

#[test]
fn relative_insets_choose_the_leading_edge_without_moving_siblings() {
	for display in [Display::Flex, Display::Grid] {
		for direction in [Direction::Ltr, Direction::Rtl] {
			let mut tree = Yoga::new();
			let root = tree.create_node();
			tree.set_style(
				root,
				Style {
					display,
					direction,
					width: Dimension::Points(200.0),
					height: Dimension::Points(100.0),
					grid: Some(GridStyle {
						template_columns: vec![TrackSizing::Points(200.0)],
						template_rows: vec![TrackSizing::Points(20.0); 2],
						..GridStyle::default()
					}),
					..Style::default()
				},
			);
			let child = add(
				&mut tree,
				root,
				Style {
					width: Dimension::Points(40.0),
					height: Dimension::Points(20.0),
					inset: Rect::all_edges(
						Dimension::Points(-5.0),
						Dimension::Points(9.0),
						Dimension::Points(13.0),
						Dimension::Points(7.0),
					),
					..Style::default()
				},
			);
			let sibling = add(
				&mut tree,
				root,
				Style {
					width: Dimension::Points(40.0),
					height: Dimension::Points(20.0),
					..Style::default()
				},
			);

			layout(&mut tree, root, 200.0, 100.0);

			let base_x = if display == Display::Flex && direction == Direction::Rtl {
				160.0
			} else {
				0.0
			};

			assert_eq!(
				(tree.layout(child).x, tree.layout(child).y),
				(
					if direction == Direction::Ltr {
						7.0
					} else {
						base_x - 9.0
					},
					-5.0
				),
				"{display:?} {direction:?}"
			);
			assert_eq!(
				(tree.layout(sibling).x, tree.layout(sibling).y),
				(base_x, 20.0)
			);
		}
	}
}

#[test]
fn grid_relative_percentages_use_the_grid_area() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			display: Display::Grid,
			width: Dimension::Points(200.0),
			height: Dimension::Points(100.0),
			grid: Some(GridStyle {
				template_columns: vec![TrackSizing::Points(100.0); 2],
				template_rows: vec![TrackSizing::Points(40.0)],
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	let child = add(
		&mut tree,
		root,
		Style {
			inset: Rect::all_edges(
				Dimension::Percent(-0.5),
				Dimension::Auto,
				Dimension::Auto,
				Dimension::Percent(0.25),
			),
			..Style::default()
		},
	);

	layout(&mut tree, root, 200.0, 100.0);

	assert_eq!((tree.layout(child).x, tree.layout(child).y), (25.0, -20.0));
}

#[test]
fn absolute_explicit_sizes_win_over_opposing_insets() {
	for direction in [Direction::Ltr, Direction::Rtl] {
		let mut tree = Yoga::new();
		let root = tree.create_node();
		tree.set_style(
			root,
			Style {
				direction,
				width: Dimension::Points(200.0),
				height: Dimension::Points(100.0),
				..Style::default()
			},
		);
		let child = add(
			&mut tree,
			root,
			Style {
				position: PositionType::Absolute,
				width: Dimension::Points(50.0),
				height: Dimension::Points(20.0),
				inset: Rect::all_edges(
					Dimension::Points(-5.0),
					Dimension::Points(20.0),
					Dimension::Points(10.0),
					Dimension::Points(10.0),
				),
				..decorated()
			},
		);

		layout(&mut tree, root, 200.0, 100.0);

		assert_eq!(
			(tree.layout(child).width, tree.layout(child).height),
			(74.0, 44.0)
		);
		assert_eq!(
			(tree.layout(child).x, tree.layout(child).y),
			(
				if direction == Direction::Ltr {
					10.0
				} else {
					106.0
				},
				-5.0
			)
		);
	}
}

#[test]
fn absolute_auto_content_box_size_fills_between_signed_insets() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: Dimension::Points(200.0),
			height: Dimension::Points(100.0),
			..Style::default()
		},
	);
	let child = add(
		&mut tree,
		root,
		Style {
			position: PositionType::Absolute,
			inset: Rect::all_edges(
				Dimension::Points(-5.0),
				Dimension::Points(20.0),
				Dimension::Points(10.0),
				Dimension::Points(-10.0),
			),
			..decorated()
		},
	);

	layout(&mut tree, root, 200.0, 100.0);

	assert_eq!(
		(
			tree.layout(child).x,
			tree.layout(child).y,
			tree.layout(child).width,
			tree.layout(child).height
		),
		(-10.0, -5.0, 190.0, 95.0)
	);
}

#[test]
fn row_reverse_and_rtl_cancel_each_other() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: Dimension::Points(100.0),
			direction: Direction::Rtl,
			flex_direction: FlexDirection::RowReverse,
			..Style::default()
		},
	);
	let first = add(
		&mut tree,
		root,
		Style {
			width: Dimension::Points(20.0),
			..Style::default()
		},
	);
	let second = add(
		&mut tree,
		root,
		Style {
			width: Dimension::Points(30.0),
			..Style::default()
		},
	);

	layout(&mut tree, root, 100.0, 100.0);

	assert_eq!((tree.layout(first).x, tree.layout(second).x), (0.0, 20.0));
}

#[test]
fn fractional_flex_grow_leaves_its_unrequested_free_space() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: Dimension::Points(100.0),
			..Style::row()
		},
	);
	let child = add(
		&mut tree,
		root,
		Style {
			width: Dimension::Points(20.0),
			flex_grow: 0.5,
			..Style::default()
		},
	);

	layout(&mut tree, root, 100.0, 100.0);

	assert_eq!(tree.layout(child).width, 60.0);
}

#[test]
fn fractional_flex_grow_keeps_the_initial_request_after_a_sibling_freezes() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: Dimension::Points(200.0),
			..Style::row()
		},
	);
	let first = add(
		&mut tree,
		root,
		Style {
			flex_basis: Dimension::Points(0.0),
			flex_grow: 0.5,
			max_width: Dimension::Points(20.0),
			..Style::default()
		},
	);
	let second = add(
		&mut tree,
		root,
		Style {
			flex_basis: Dimension::Points(0.0),
			flex_grow: 0.25,
			..Style::default()
		},
	);

	layout(&mut tree, root, 200.0, 100.0);

	assert_eq!(
		(tree.layout(first).width, tree.layout(second).width),
		(20.0, 50.0)
	);
}

#[test]
fn fractional_flex_shrink_can_leave_overflow() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: Dimension::Points(100.0),
			..Style::row()
		},
	);
	let first = add(
		&mut tree,
		root,
		Style {
			width: Dimension::Points(100.0),
			flex_shrink: 0.25,
			..Style::default()
		},
	);
	let second = add(
		&mut tree,
		root,
		Style {
			width: Dimension::Points(100.0),
			flex_shrink: 0.25,
			..Style::default()
		},
	);

	layout(&mut tree, root, 100.0, 100.0);

	assert_eq!(
		(tree.layout(first).width, tree.layout(second).width),
		(75.0, 75.0)
	);
}

#[test]
fn opposing_flex_violations_freeze_only_the_responsible_items() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: Dimension::Points(100.0),
			..Style::row()
		},
	);
	let first = add(
		&mut tree,
		root,
		Style {
			flex_basis: Dimension::Points(0.0),
			flex_grow: 1.0,
			min_width: Dimension::Points(80.0),
			..Style::default()
		},
	);
	let second = add(
		&mut tree,
		root,
		Style {
			flex_basis: Dimension::Points(0.0),
			flex_grow: 1.0,
			max_width: Dimension::Points(10.0),
			..Style::default()
		},
	);

	layout(&mut tree, root, 100.0, 100.0);

	assert_eq!(
		(tree.layout(first).width, tree.layout(second).width),
		(90.0, 10.0)
	);
}

#[test]
fn flex_shrink_weights_exclude_margins_and_box_decorations() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			width: Dimension::Points(220.0),
			..Style::row()
		},
	);
	let first = add(
		&mut tree,
		root,
		Style {
			width: Dimension::Points(100.0),
			flex_shrink: 1.0,
			margin: Rect::all_edges(
				Dimension::Points(0.0),
				Dimension::Points(20.0),
				Dimension::Points(0.0),
				Dimension::Points(0.0),
			),
			..decorated()
		},
	);
	let second = add(
		&mut tree,
		root,
		Style {
			width: Dimension::Points(100.0),
			flex_shrink: 1.0,
			..Style::default()
		},
	);

	layout(&mut tree, root, 220.0, 100.0);

	assert_eq!(
		(
			tree.layout(first).width,
			tree.layout(second).width,
			tree.layout(second).x
		),
		(112.0, 88.0, 132.0)
	);
}

#[test]
fn minmax_fraction_tracks_share_space_independently_of_item_content() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	let fraction = |factor| {
		TrackSizing::MinMax(Box::new(MinMax {
			min: TrackSizing::Points(0.0),
			max: TrackSizing::Fraction(factor),
		}))
	};
	tree.set_style(
		root,
		Style {
			display: Display::Grid,
			width: Dimension::Points(310.0),
			gap: Rect::all_edges(
				Dimension::Points(0.0),
				Dimension::Points(0.0),
				Dimension::Points(0.0),
				Dimension::Points(10.0),
			),
			grid: Some(GridStyle {
				template_columns: vec![fraction(1.0), fraction(2.0)],
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	let first = add(&mut tree, root, Style { ..Style::default() });
	tree.set_measure_func(first, Some(std::sync::Arc::new(|_| Size::new(400.0, 10.0))));
	let second = add(&mut tree, root, Style::default());

	layout(&mut tree, root, 310.0, 100.0);

	assert_eq!(
		(
			tree.layout(first).width,
			tree.layout(second).x,
			tree.layout(second).width
		),
		(100.0, 110.0, 200.0)
	);
}

#[test]
fn fractional_grid_minimums_freeze_before_remaining_space_is_shared() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	let track = |minimum| {
		TrackSizing::MinMax(Box::new(MinMax {
			min: TrackSizing::Points(minimum),
			max: TrackSizing::Fraction(1.0),
		}))
	};
	tree.set_style(
		root,
		Style {
			display: Display::Grid,
			width: Dimension::Points(300.0),
			grid: Some(GridStyle {
				template_columns: vec![track(200.0), track(0.0)],
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	let first = add(&mut tree, root, Style::default());
	let second = add(&mut tree, root, Style::default());

	layout(&mut tree, root, 300.0, 100.0);

	assert_eq!(
		(tree.layout(first).width, tree.layout(second).width),
		(200.0, 100.0)
	);
}

#[test]
fn grid_fraction_totals_below_one_leave_unclaimed_space() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			display: Display::Grid,
			width: Dimension::Points(200.0),
			grid: Some(GridStyle {
				template_columns: vec![TrackSizing::Fraction(0.25), TrackSizing::Fraction(0.25)],
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	let first = add(&mut tree, root, Style::default());
	let second = add(&mut tree, root, Style::default());

	layout(&mut tree, root, 200.0, 100.0);

	assert_abs_diff_eq!(tree.layout(first).width, 50.0);
	assert_abs_diff_eq!(tree.layout(second).x, 50.0);
	assert_abs_diff_eq!(tree.layout(second).width, 50.0);
}

#[test]
fn fractional_grid_rows_include_gaps_and_minimums() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	let fraction = |factor| {
		TrackSizing::MinMax(Box::new(MinMax {
			min: TrackSizing::Points(0.0),
			max: TrackSizing::Fraction(factor),
		}))
	};
	tree.set_style(
		root,
		Style {
			display: Display::Grid,
			width: Dimension::Points(100.0),
			height: Dimension::Points(310.0),
			gap: Rect::all_edges(
				Dimension::Points(10.0),
				Dimension::Points(0.0),
				Dimension::Points(0.0),
				Dimension::Points(0.0),
			),
			grid: Some(GridStyle {
				template_columns: vec![TrackSizing::Points(100.0)],
				template_rows: vec![fraction(1.0), fraction(2.0)],
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	let first = add(&mut tree, root, Style::default());
	let second = add(&mut tree, root, Style::default());

	layout(&mut tree, root, 100.0, 310.0);

	assert_eq!(
		(
			tree.layout(first).height,
			tree.layout(second).y,
			tree.layout(second).height
		),
		(100.0, 110.0, 200.0)
	);
}

#[test]
fn max_content_fraction_tracks_keep_their_proportions() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	let fraction = |factor| {
		TrackSizing::MinMax(Box::new(MinMax {
			min: TrackSizing::Points(0.0),
			max: TrackSizing::Fraction(factor),
		}))
	};
	tree.set_style(
		root,
		Style {
			display: Display::Grid,
			grid: Some(GridStyle {
				template_columns: vec![fraction(1.0), fraction(2.0)],
				..GridStyle::default()
			}),
			..Style::default()
		},
	);
	let first = add(&mut tree, root, Style::default());
	tree.set_measure_func(first, Some(std::sync::Arc::new(|_| Size::new(50.0, 10.0))));
	let second = add(&mut tree, root, Style::default());
	tree.set_measure_func(second, Some(std::sync::Arc::new(|_| Size::new(20.0, 10.0))));

	tree.calculate_layout(
		root,
		Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
	);

	assert_eq!(
		(
			tree.layout(root).width,
			tree.layout(first).width,
			tree.layout(second).width
		),
		(150.0, 50.0, 100.0)
	);
}

#[test]
fn repeated_spanning_contributions_do_not_grow_satisfied_tracks() {
	let mut tree = Yoga::new();
	let root = tree.create_node();
	tree.set_style(
		root,
		Style {
			display: Display::Grid,
			grid: Some(GridStyle {
				template_columns: vec![TrackSizing::Auto; 2],
				..GridStyle::default()
			}),
			..Style::default()
		},
	);

	for _ in 0..2 {
		add(
			&mut tree,
			root,
			Style {
				width: Dimension::Points(100.0),
				height: Dimension::Points(10.0),
				grid: Some(GridStyle {
					column: GridPlacement::span(2),
					..GridStyle::default()
				}),
				..Style::default()
			},
		);
	}

	tree.calculate_layout(
		root,
		Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
	);

	assert_eq!(
		(tree.layout(root).width, tree.layout(root).height),
		(100.0, 20.0)
	);
}

#[test]
fn border_box_flex_bases_are_not_floored_to_their_decorations() {
	for direction in [FlexDirection::Row, FlexDirection::Column] {
		let mut tree = Yoga::new();
		let root = tree.create_node();
		tree.set_style(
			root,
			Style {
				width: Dimension::Points(100.0),
				height: Dimension::Points(100.0),
				flex_direction: direction,
				..Style::default()
			},
		);
		let first = add(
			&mut tree,
			root,
			Style {
				flex_basis: Dimension::Points(0.0),
				flex_grow: 1.0,
				padding: Rect::new(|| Dimension::Points(10.0)),
				..Style::default()
			},
		);
		let second = add(
			&mut tree,
			root,
			Style {
				flex_basis: Dimension::Points(0.0),
				flex_grow: 1.0,
				..Style::default()
			},
		);

		layout(&mut tree, root, 100.0, 100.0);

		let main = |id| {
			if direction.is_row() {
				tree.layout(id).width
			} else {
				tree.layout(id).height
			}
		};
		assert_eq!((main(first), main(second)), (50.0, 50.0), "{direction:?}");
	}
}

#[test]
fn intrinsic_fractional_spans_use_the_combined_fraction_factors() {
	for columns in [true, false] {
		let mut tree = Yoga::new();
		let root = tree.create_node();
		let track = |factor| {
			TrackSizing::MinMax(Box::new(MinMax {
				min: TrackSizing::Points(0.0),
				max: TrackSizing::Fraction(factor),
			}))
		};
		let tracks = vec![track(1.0), track(2.0)];
		tree.set_style(
			root,
			Style {
				display: Display::Grid,
				grid: Some(if columns {
					GridStyle {
						template_columns: tracks,
						..GridStyle::default()
					}
				} else {
					GridStyle {
						template_rows: tracks,
						..GridStyle::default()
					}
				}),
				..Style::default()
			},
		);
		add(
			&mut tree,
			root,
			Style {
				width: Dimension::Points(90.0),
				height: Dimension::Points(90.0),
				grid: Some(if columns {
					GridStyle {
						column: GridPlacement::span(2),
						..GridStyle::default()
					}
				} else {
					GridStyle {
						row: GridPlacement::span(2),
						..GridStyle::default()
					}
				}),
				..Style::default()
			},
		);

		tree.calculate_layout(
			root,
			Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
		);

		let size = if columns {
			tree.layout(root).width
		} else {
			tree.layout(root).height
		};

		assert_eq!(size, 90.0, "columns: {columns}");
	}
}

#[test]
fn intrinsic_fractional_tracks_are_recomputed_for_container_minimums() {
	for columns in [true, false] {
		let mut tree = Yoga::new();
		let root = tree.create_node();
		let track = |factor| {
			TrackSizing::MinMax(Box::new(MinMax {
				min: TrackSizing::Points(0.0),
				max: TrackSizing::Fraction(factor),
			}))
		};
		let tracks = vec![track(1.0), track(2.0)];
		tree.set_style(
			root,
			Style {
				display: Display::Grid,
				min_width: if columns {
					Dimension::Points(300.0)
				} else {
					Dimension::Auto
				},
				min_height: if columns {
					Dimension::Auto
				} else {
					Dimension::Points(300.0)
				},
				grid: Some(if columns {
					GridStyle {
						template_columns: tracks,
						..GridStyle::default()
					}
				} else {
					GridStyle {
						template_rows: tracks,
						..GridStyle::default()
					}
				}),
				..Style::default()
			},
		);
		let first = add(&mut tree, root, Style::default());
		let second = add(
			&mut tree,
			root,
			Style {
				grid: Some(if columns {
					GridStyle {
						column: GridPlacement::line(2, 3),
						..GridStyle::default()
					}
				} else {
					GridStyle {
						row: GridPlacement::line(2, 3),
						..GridStyle::default()
					}
				}),
				..Style::default()
			},
		);

		tree.calculate_layout(
			root,
			Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
		);

		let size = |id| {
			if columns {
				tree.layout(id).width
			} else {
				tree.layout(id).height
			}
		};
		assert_eq!(
			(size(root), size(first), size(second)),
			(300.0, 100.0, 200.0),
			"columns: {columns}"
		);
	}
}
