//! Observable composition behavior through shared reconciliation and layout.
#![allow(clippy::float_cmp)]

use std::num::NonZeroU8;

use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::FontWeight;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_native::TextAlign;
use leptoskit_native::TextStyle;
use leptoskit_native::components::*;
use leptoskit_yoga::Align as BoxAlign;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Direction;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::NodeId;
use leptoskit_yoga::Rect;

#[test]
fn flex_components_place_children_on_their_declared_axes() {
	for (tree, expected) in [
		(Row(), [(0.0, 0.0), (40.0, 0.0)]),
		(Column(), [(0.0, 0.0), (0.0, 20.0)]),
		(
			Flex(FlexDirection::RowReverse),
			[(140.0, 0.0), (100.0, 0.0)],
		),
	] {
		let surface = mount(
			&tree
				.class("w-[180px] h-20")
				.children([box_at(40.0, 20.0), box_at(40.0, 20.0)]),
		);
		let children = surface.children(root(&surface));

		for (child, position) in children.iter().zip(expected) {
			assert_eq!(position_of(&surface, *child), position);
		}
	}
}

#[test]
fn wrap_creates_new_runs_and_respects_gaps() {
	let surface = mount(&Wrap().class("w-[100px] gap-2").children([
		box_at(60.0, 20.0),
		box_at(60.0, 20.0),
		box_at(20.0, 20.0),
	]));
	let children = surface.children(root(&surface));
	assert_eq!(position_of(&surface, children[0]), (0.0, 0.0));
	assert_eq!(position_of(&surface, children[1]), (0.0, 28.0));
	assert_eq!(position_of(&surface, children[2]), (68.0, 28.0));
	assert_eq!(surface.layout(root(&surface)).height, 48.0);
}

#[test]
fn grid_uses_equal_columns_and_content_sized_rows() {
	let surface = mount(&Grid(columns(2)).class("w-[180px] gap-2").children([
		View().class("h-5"),
		View().class("h-10"),
		View().class("h-5"),
	]));
	let children = surface.children(root(&surface));
	assert_eq!(surface.layout(children[0]).width, 86.0);
	assert_eq!(position_of(&surface, children[1]), (94.0, 0.0));
	assert_eq!(position_of(&surface, children[2]), (0.0, 48.0));
	assert_eq!(surface.layout(root(&surface)).height, 68.0);
}

#[test]
fn padding_and_sized_box_follow_the_shared_box_model() {
	let surface = mount(
		&Padding(Rect::all_edges(
			points(3.0),
			points(5.0),
			points(7.0),
			points(10.0),
		))
		.child(SizedBox(points(20.0), points(12.0)))
		.with_style(|style| style.align_items = BoxAlign::FlexStart),
	);
	let child = surface.children(root(&surface))[0];
	assert_eq!(position_of(&surface, child), (10.0, 3.0));
	assert_eq!(size_of(&surface, child), (20.0, 12.0));
	assert_eq!(surface.layout(root(&surface)).height, 22.0);
}

#[test]
fn validated_constraints_clamp_the_wrapper_without_rewriting_children() {
	let bounds = BoxConstraints::new(40.0, Some(100.0), 60.0, None).expect("valid bounds");
	let surface = mount(
		&ConstrainedBox(bounds)
			.class("w-[300px] h-[30px]")
			.child(box_at(20.0, 10.0)),
	);
	let child = surface.children(root(&surface))[0];
	assert_eq!(size_of(&surface, root(&surface)), (100.0, 60.0));
	assert_eq!(size_of(&surface, child), (20.0, 10.0));
	assert_eq!(bounds.min_width(), 40.0);
	assert_eq!(bounds.max_width(), Some(100.0));
	assert_eq!(bounds.min_height(), 60.0);
	assert_eq!(bounds.max_height(), None);

	let tight = mount(&ConstrainedBox(
		BoxConstraints::tight(35.0, 25.0).expect("tight"),
	));
	assert_eq!(size_of(&tight, root(&tight)), (35.0, 25.0));
}

#[test]
fn aspect_ratio_derives_an_automatic_axis_and_defers_to_explicit_sizes() {
	let surface = mount(&AspectRatio(16.0 / 9.0).expect("ratio").class("w-40"));
	assert_eq!(size_of(&surface, root(&surface)), (160.0, 90.0));

	let explicit = mount(&AspectRatio(2.0).expect("ratio").class("w-40 h-10"));
	assert_eq!(size_of(&explicit, root(&explicit)), (160.0, 40.0));
}

#[test]
fn supported_aspect_ratio_boundaries_produce_finite_frames() {
	for ratio in [1e-6, 1e6] {
		for tree in [
			AspectRatio(ratio).expect("supported ratio").class("w-40"),
			AspectRatio(ratio).expect("supported ratio").class("h-40"),
		] {
			let surface = mount(&tree);
			let (width, height) = size_of(&surface, root(&surface));
			assert!(width.is_finite() && height.is_finite());
			assert!(width > 0.0 && height > 0.0);
		}
	}
}

#[test]
fn all_nine_alignments_position_content_inside_the_allocated_box() {
	for (alignment, expected) in [
		(Alignment::TopStart, (0.0, 0.0)),
		(Alignment::TopCenter, (40.0, 0.0)),
		(Alignment::TopEnd, (80.0, 0.0)),
		(Alignment::CenterStart, (0.0, 30.0)),
		(Alignment::Center, (40.0, 30.0)),
		(Alignment::CenterEnd, (80.0, 30.0)),
		(Alignment::BottomStart, (0.0, 60.0)),
		(Alignment::BottomCenter, (40.0, 60.0)),
		(Alignment::BottomEnd, (80.0, 60.0)),
	] {
		let surface = mount(
			&Align(alignment)
				.class("w-[100px] h-20")
				.child(box_at(20.0, 20.0)),
		);
		let child = surface.children(root(&surface))[0];
		assert_eq!(position_of(&surface, child), expected, "{alignment:?}");
	}

	let centered = mount(&Center().class("w-[100px] h-20").child(box_at(20.0, 20.0)));
	assert_eq!(
		position_of(&centered, centered.children(root(&centered))[0]),
		(40.0, 30.0)
	);
}

#[test]
fn alignment_start_and_end_follow_the_wrappers_rtl_direction() {
	for (alignment, expected_x) in [(Alignment::TopStart, 80.0), (Alignment::BottomEnd, 0.0)] {
		let surface = mount(
			&Align(alignment)
				.class("w-[100px] h-20")
				.with_style(|style| style.direction = Direction::Rtl)
				.child(box_at(20.0, 20.0)),
		);
		let child = surface.children(root(&surface))[0];
		assert_eq!(surface.layout(child).x, expected_x);
	}
}

#[test]
fn expanded_edits_parent_data_directly_and_shares_space_with_spacer() {
	let expanded = Expanded(Text("content"));
	assert_eq!(expanded.kind, NativeKind::Text);
	assert!(expanded.children.is_empty());
	let surface =
		mount(
			&Row()
				.class("w-[300px] h-10")
				.children([box_at(50.0, 20.0), expanded, Spacer()]),
		);
	let children = surface.children(root(&surface));
	assert_eq!(surface.layout(children[1]).width, 125.0);
	assert_eq!(surface.layout(children[2]).width, 125.0);
	assert_eq!(surface.layout(children[2]).x, 175.0);
}

#[test]
fn stack_intrinsic_size_includes_all_non_positioned_children() {
	let surface = mount(
		&Column()
			.with_style(|style| style.align_items = BoxAlign::FlexStart)
			.child(Stack([
				box_at(80.0, 30.0),
				box_at(40.0, 60.0),
				Positioned(Rect::default(), box_at(200.0, 200.0)),
			])),
	);
	let stack = surface.children(root(&surface))[0];
	assert_eq!(size_of(&surface, stack), (80.0, 60.0));
	let children = surface.children(stack);
	assert_eq!(position_of(&surface, children[0]), (0.0, 0.0));
	assert_eq!(position_of(&surface, children[1]), (0.0, 0.0));
}

#[test]
fn stack_hits_the_last_painted_child_and_preserves_node_identity() {
	let tree = || {
		Stack([
			Button("back").class("w-20 h-20"),
			Button("front").class("w-10 h-10"),
		])
	};
	let mut surface = mount(&tree());
	let stack = root(&surface);
	let children = surface.children(stack).to_vec();
	assert_eq!(surface.hit_test(10.0, 10.0), Some(children[1]));
	assert_eq!(surface.hit_test(60.0, 60.0), Some(children[0]));
	surface.set_root(&tree().class("w-[100px]"));
	assert_eq!(surface.children(stack), children);
}

#[test]
fn stack_positioned_children_keep_their_cell_when_classes_restore_normal_flow() {
	let surface = mount(
		&Column()
			.with_style(|style| style.align_items = BoxAlign::FlexStart)
			.child(Stack([
				box_at(80.0, 30.0),
				Positioned(Rect::default(), box_at(40.0, 60.0)).class("relative"),
			])),
	);
	let stack = surface.children(root(&surface))[0];
	assert_eq!(size_of(&surface, stack), (80.0, 60.0));
	let children = surface.children(stack);
	assert_eq!(position_of(&surface, children[0]), (0.0, 0.0));
	assert_eq!(position_of(&surface, children[1]), (0.0, 0.0));
}

#[test]
fn stack_keeps_responsive_absolute_and_relative_children_in_one_cell() {
	for child in [
		Positioned(Rect::default(), box_at(40.0, 60.0)).class("md:relative"),
		box_at(40.0, 60.0).class("absolute md:relative"),
	] {
		let mut surface = mount(
			&Column()
				.with_style(|style| style.align_items = BoxAlign::FlexStart)
				.child(Stack([box_at(80.0, 30.0), child])),
		);
		let stack = surface.children(root(&surface))[0];
		let children = surface.children(stack).to_vec();
		assert_eq!(size_of(&surface, stack), (80.0, 30.0));

		surface.set_viewport(900.0, 300.0);
		assert_eq!(surface.children(stack), children);
		assert_eq!(size_of(&surface, stack), (80.0, 60.0));
		assert_eq!(position_of(&surface, children[1]), (0.0, 0.0));

		surface.set_viewport(400.0, 300.0);
		assert_eq!(size_of(&surface, stack), (80.0, 30.0));
	}
}

#[test]
fn positioned_edits_the_child_and_resolves_opposing_insets() {
	let child = Positioned(
		Rect::all_edges(points(4.0), points(6.0), Dimension::Auto, points(8.0)),
		Button("overlay").class("h-5"),
	);
	assert_eq!(child.kind, NativeKind::Button);
	assert!(child.children.is_empty());
	let surface = mount(&Stack([child]).class("w-[100px] h-20"));
	let child = surface.children(root(&surface))[0];
	assert_eq!(position_of(&surface, child), (8.0, 4.0));
	assert_eq!(surface.layout(child).width, 86.0);
}

#[test]
fn positioned_stack_child_uses_the_grid_cell_as_its_containing_block() {
	let surface = mount(
		&Column()
			.with_style(|style| style.align_items = BoxAlign::FlexStart)
			.child(
				Stack([
					box_at(80.0, 60.0),
					Positioned(
						Rect::all_edges(points(4.0), points(6.0), Dimension::Auto, points(8.0)),
						View().class("h-5"),
					),
				])
				.class("p-4"),
			),
	);
	let stack = surface.children(root(&surface))[0];
	let overlay = surface.children(stack)[1];
	assert_eq!(size_of(&surface, stack), (112.0, 92.0));
	assert_eq!(position_of(&surface, overlay), (24.0, 20.0));
	assert_eq!(size_of(&surface, overlay), (66.0, 20.0));
}

#[test]
fn clip_components_clip_overflow_and_preserve_the_rounded_contract() {
	for clip in [ClipRect(), ClipRRect(8.0).expect("radius")] {
		let surface = mount(&clip.class("w-10 h-10").child(Positioned(
			Rect::all_edges(points(0.0), Dimension::Auto, Dimension::Auto, points(50.0)),
			Button("outside").class("w-5 h-5"),
		)));
		assert_eq!(surface.props(root(&surface)).clip, Some(true));
		assert_eq!(surface.hit_test(55.0, 5.0), None);
	}

	let rounded = mount(&ClipRRect(8.0).expect("radius"));
	assert_eq!(surface_radius(&rounded), Some(8.0));
}

#[test]
fn transparent_opacity_preserves_layout_and_input() {
	let surface = mount(
		&Opacity(0.0)
			.expect("opacity")
			.class("w-20 h-10")
			.child(Button("target").class("w-10 h-5")),
	);
	let child = surface.children(root(&surface))[0];
	assert_eq!(size_of(&surface, root(&surface)), (80.0, 40.0));
	assert_eq!(surface.props(root(&surface)).opacity, Some(0.0));
	assert_eq!(surface.hit_test(5.0, 5.0), Some(child));
}

#[test]
fn default_text_style_is_inherited_and_children_override_it() {
	let text_style = TextStyle {
		font_size: 20.0,
		font_weight: FontWeight::Bold,
		text_align: TextAlign::Center,
	};
	let surface = mount(
		&DefaultTextStyle(text_style)
			.color(Color::rgba(1, 2, 3, 255))
			.children([
				Text("default"),
				Text("override").class("text-sm font-medium text-red-500 text-start"),
			]),
	);
	let children = surface.children(root(&surface));
	assert_eq!(surface.props(children[0]).text_style(), text_style);
	assert_eq!(
		surface.props(children[0]).color,
		Some(Color::rgba(1, 2, 3, 255))
	);
	assert_eq!(surface.props(children[1]).font_size, Some(14.0));
	assert_eq!(
		surface.props(children[1]).font_weight,
		Some(FontWeight::Medium)
	);
	assert_eq!(
		surface.props(children[1]).text_align,
		Some(TextAlign::Start)
	);
}

#[test]
fn visibility_collapses_layout_and_blocks_input_without_replacing_nodes() {
	let tree = |visible| {
		Column().children([
			Visibility(visible).child(Button("target").class("w-10 h-10").on_press(|| {})),
			box_at(20.0, 20.0),
		])
	};
	let mut surface = mount(&tree(true));
	let container = surface.children(root(&surface))[0];
	let button = surface.children(container)[0];
	let sibling = surface.children(root(&surface))[1];
	assert_eq!(surface.layout(sibling).y, 40.0);
	assert_eq!(
		surface.dispatch_event(button, &NativeEvent::Press),
		EventOutcome::Handled
	);
	surface.set_root(&tree(false));
	assert_eq!(surface.layout(sibling).y, 0.0);
	assert_eq!(
		surface.dispatch_event(button, &NativeEvent::Press),
		EventOutcome::Unhandled
	);
	assert_eq!(surface.props(container).hidden, Some(true));
	surface.set_root(&tree(true));
	assert_eq!(surface.children(container), [button]);
	assert_eq!(surface.layout(sibling).y, 40.0);
	assert_eq!(
		surface.dispatch_event(button, &NativeEvent::Press),
		EventOutcome::Handled
	);
}

#[test]
fn separators_stretch_on_their_cross_axis_and_take_one_pixel() {
	let horizontal = mount(
		&Column()
			.class("w-[100px]")
			.children([box_at(20.0, 10.0), Divider()]),
	);
	let divider = horizontal.children(root(&horizontal))[1];
	assert_eq!(size_of(&horizontal, divider), (100.0, 1.0));
	assert_eq!(horizontal.layout(divider).y, 10.0);

	let vertical = mount(
		&Row()
			.class("h-20")
			.children([box_at(20.0, 10.0), VerticalDivider()]),
	);
	let divider = vertical.children(root(&vertical))[1];
	assert_eq!(size_of(&vertical, divider), (1.0, 80.0));
	assert_eq!(vertical.layout(divider).x, 20.0);
}

#[test]
fn surfaces_and_compact_labels_paint_and_measure_their_content() {
	let color = Color::rgba(10, 20, 30, 255);
	let colored = mount(&ColoredBox(color).child(box_at(20.0, 10.0)));
	assert_eq!(colored.props(root(&colored)).background, Some(color));

	let card = mount(
		&Card()
			.with_style(|style| style.align_items = BoxAlign::FlexStart)
			.child(box_at(20.0, 10.0)),
	);
	let child = card.children(root(&card))[0];
	assert_eq!(position_of(&card, child), (16.0, 16.0));
	assert_eq!(card.layout(root(&card)).height, 42.0);
	assert_eq!(surface_radius(&card), Some(12.0));

	for (label, minimum_height) in [(Badge("7"), 20.0), (Chip("Ready"), 32.0)] {
		let surface = mount(
			&Column()
				.with_style(|style| style.align_items = BoxAlign::FlexStart)
				.child(label),
		);
		let capsule = surface.children(root(&surface))[0];
		let text = surface.children(capsule)[0];
		assert!(surface.layout(capsule).height >= minimum_height);
		assert!(surface.layout(capsule).width > surface.layout(text).width);
		assert!(surface.props(text).color.is_some());
	}
}

#[test]
fn list_tile_allocates_leading_content_and_trailing_slots() {
	let surface = mount(
		&ListTile(
			Some(box_at(24.0, 24.0)),
			Text("Title"),
			Some(box_at(20.0, 20.0)),
		)
		.class("w-[300px]"),
	);
	let children = surface.children(root(&surface));
	assert_eq!(children.len(), 3);
	assert_eq!(surface.kind(children[1]), NativeKind::Text);
	assert_eq!(surface.layout(children[0]).x, 12.0);
	assert_eq!(surface.layout(children[1]).x, 48.0);
	assert_eq!(surface.layout(children[1]).width, 208.0);
	assert_eq!(surface.layout(children[2]).x, 268.0);
	assert_eq!(surface.layout(root(&surface)).height, 48.0);
}

#[test]
fn scaffold_reserves_header_and_footer_and_grows_the_body() {
	let surface = mount(
		&Scaffold(
			Some(AppBar(Text("Title")).child(Button("Action").class("w-16 h-6"))),
			View(),
			Some(box_at(100.0, 20.0)),
		)
		.class("w-[300px] h-[220px]"),
	);
	let children = surface.children(root(&surface));
	assert_eq!(size_of(&surface, children[0]), (300.0, 56.0));
	assert_eq!(size_of(&surface, children[1]), (300.0, 144.0));
	assert_eq!(surface.layout(children[1]).y, 56.0);
	assert_eq!(surface.layout(children[2]).y, 200.0);
	let header = surface.children(children[0]);
	assert_eq!(surface.kind(header[0]), NativeKind::Text);
	assert_eq!(surface.layout(header[1]).x, 224.0);
}

#[test]
fn eager_collections_fit_between_header_and_footer_and_keep_overflowing_items() {
	for collection in [
		ListView().children((0..5).map(|_| View().class("h-10"))),
		GridView(columns(2)).children((0..8).map(|_| View().class("h-10"))),
	] {
		let surface = mount(&Column().class("w-[200px] h-[160px]").children([
			View().class("h-5"),
			collection,
			View().class("h-5"),
		]));
		let children = surface.children(root(&surface));
		let collection = children[1];
		assert_eq!(surface.kind(collection), NativeKind::ScrollView);
		assert_eq!(position_of(&surface, collection), (0.0, 20.0));
		assert_eq!(size_of(&surface, collection), (200.0, 120.0));
		assert_eq!(surface.layout(children[2]).y, 140.0);
		let items = surface.children(collection);
		let last = surface.layout(*items.last().expect("eager items"));
		assert!(last.y + last.height > surface.layout(collection).height);
		assert_eq!(surface.props(collection).clip, Some(true));
	}
}

#[test]
fn tailwind_overrides_composition_defaults() {
	let surface = mount(
		&Row()
			.class("flex-col bg-red-500 p-2")
			.children([box_at(10.0, 10.0), box_at(10.0, 10.0)]),
	);
	let children = surface.children(root(&surface));
	assert_eq!(position_of(&surface, children[0]), (8.0, 8.0));
	assert_eq!(position_of(&surface, children[1]), (8.0, 18.0));
	assert!(surface.props(root(&surface)).background.is_some());

	let visible = mount(&Visibility(false).class("flex").child(box_at(10.0, 10.0)));
	assert_ne!(visible.props(root(&visible)).hidden, Some(true));
	assert_eq!(visible.layout(root(&visible)).height, 10.0);
}

#[test]
fn numeric_boundaries_reject_invalid_values_without_constructing_bad_elements() {
	for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
		assert!(BoxConstraints::new(invalid, None, 0.0, None).is_err());
		assert!(BoxConstraints::new(0.0, Some(invalid), 0.0, None).is_err());
		assert!(BoxConstraints::new(0.0, None, invalid, None).is_err());
		assert!(BoxConstraints::new(0.0, None, 0.0, Some(invalid)).is_err());
		assert!(AspectRatio(invalid).is_err());
		assert!(ClipRRect(invalid).is_err());
		assert!(Opacity(invalid).is_err());
	}

	assert_eq!(
		BoxConstraints::new(20.0, Some(10.0), 0.0, None),
		Err(LayoutError::InvertedConstraints("width"))
	);
	assert_eq!(
		BoxConstraints::new(0.0, None, 20.0, Some(10.0)),
		Err(LayoutError::InvertedConstraints("height"))
	);
	assert!(AspectRatio(0.0).is_err());
	assert!(AspectRatio(f32::MIN_POSITIVE).is_err());
	assert!(AspectRatio(f32::MAX).is_err());
	assert!(Opacity(1.01).is_err());
	assert!(ClipRRect(0.0).is_ok());
	assert!(Opacity(1.0).is_ok());
	assert_eq!(BoxConstraints::default().max_width(), None);
}

#[cfg(all(feature = "web", not(target_arch = "wasm32")))]
#[test]
fn compositions_emit_scoped_grid_clipping_and_scroll_css_on_the_web() {
	use leptos::prelude::RenderHtml;

	let tree = Column().children([
		Stack([Text("back"), Text("front")]),
		GridView(columns(2)).children([Text("one"), Text("two")]),
		ClipRRect(8.0).expect("radius").child(Text("clipped")),
		Align(Alignment::TopStart)
			.with_style(|style| style.direction = Direction::Rtl)
			.child(Text("RTL")),
	]);
	let html = leptoskit_native::web::render(tree).to_html();
	assert!(
		html.contains("grid-column-start:1;grid-column-end:2;grid-row-start:1;grid-row-end:2;")
	);
	assert!(html.contains("grid-template-columns:minmax(0px,1fr) minmax(0px,1fr);"));
	assert!(html.contains("overflow:auto;"));
	assert!(html.contains("border-radius:8px;"));
	assert!(html.contains("direction:rtl;"));
}

fn mount(tree: &Element) -> NativeSurface<HeadlessBackend> {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(400.0, 300.0);
	surface.set_root(tree);
	surface
}

fn root(surface: &NativeSurface<HeadlessBackend>) -> NodeId {
	surface.root().expect("committed root")
}

fn position_of(surface: &NativeSurface<HeadlessBackend>, id: NodeId) -> (f32, f32) {
	let layout = surface.layout(id);
	(layout.x, layout.y)
}

fn size_of(surface: &NativeSurface<HeadlessBackend>, id: NodeId) -> (f32, f32) {
	let layout = surface.layout(id);
	(layout.width, layout.height)
}

fn surface_radius(surface: &NativeSurface<HeadlessBackend>) -> Option<f32> {
	surface.props(root(surface)).corner_radius
}

fn points(value: f32) -> Dimension {
	Dimension::Points(value)
}

fn box_at(width: f32, height: f32) -> Element {
	SizedBox(points(width), points(height))
}

fn columns(count: u8) -> NonZeroU8 {
	NonZeroU8::new(count).expect("positive column count")
}
