//! Hit testing prunes subtrees with cached bounds; these tests pin its results
//! to an exhaustive walk of the committed tree across overflow, clipping,
//! visibility, and structural changes.
#![allow(clippy::float_cmp)]

use std::sync::Arc;

use leptoskit_native::ApproxTextMeasurer;
use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::*;
use leptoskit_yoga::NodeId;

const VIEWPORT: (f32, f32) = (320.0, 240.0);

/// The unpruned hit test: visits every node, using only the public surface.
fn exhaustive_hit_test(surface: &NativeSurface<HeadlessBackend>, x: f32, y: f32) -> Option<NodeId> {
	fn walk(
		surface: &NativeSurface<HeadlessBackend>,
		id: NodeId,
		origin: (f32, f32),
		point: (f32, f32),
		best: &mut Option<NodeId>,
	) {
		let (x, y) = point;
		let props = surface.props(id);
		let kind = surface.kind(id);
		if props.hidden == Some(true)
			|| (kind == NativeKind::ActivityIndicator && props.running == Some(false))
		{
			return;
		}
		let layout = surface.layout(id);
		let left = origin.0 + layout.x;
		let top = origin.1 + layout.y;
		let mut contains =
			x >= left && x <= left + layout.width && y >= top && y <= top + layout.height;
		if contains && props.clip == Some(true) {
			let radius = props
				.corner_radius
				.unwrap_or_default()
				.max(0.0)
				.min(layout.width.min(layout.height) * 0.5);
			let dx = x - x.clamp(left + radius, left + layout.width - radius);
			let dy = y - y.clamp(top + radius, top + layout.height - radius);
			contains = dx * dx + dy * dy <= radius * radius;
		}
		if contains {
			*best = Some(id);
		}
		if kind == NativeKind::Pressable {
			return;
		}
		if contains || props.clip != Some(true) {
			for &child in surface.children(id) {
				walk(surface, child, (left, top), point, best);
			}
		}
	}

	let mut best = None;
	walk(surface, surface.root()?, (0.0, 0.0), (x, y), &mut best);
	best
}

/// Points on a grid past the viewport, plus every committed frame's corners,
/// edge midpoints, and the points just outside them, where pruning must agree
/// with the exhaustive walk to the last bit.
fn probe_points(surface: &NativeSurface<HeadlessBackend>) -> Vec<(f32, f32)> {
	let mut points = Vec::new();
	let mut y = -20.0;
	while y <= VIEWPORT.1 + 20.0 {
		let mut x = -20.0;
		while x <= VIEWPORT.0 + 20.0 {
			points.push((x, y));
			x += 3.5;
		}
		y += 3.5;
	}

	let mut pending: Vec<_> = surface
		.root()
		.into_iter()
		.map(|root| (root, 0.0_f32, 0.0_f32))
		.collect();
	while let Some((id, origin_x, origin_y)) = pending.pop() {
		let layout = surface.layout(id);
		let left = origin_x + layout.x;
		let top = origin_y + layout.y;
		let right = left + layout.width;
		let bottom = top + layout.height;
		for x in [left, left.midpoint(right), right] {
			for y in [top, top.midpoint(bottom), bottom] {
				points.push((x, y));
				points.push((x.next_down(), y.next_down()));
				points.push((x.next_up(), y.next_up()));
			}
		}
		pending.extend(surface.children(id).iter().map(|&child| (child, left, top)));
	}
	points
}

#[track_caller]
fn assert_matches_exhaustive(surface: &NativeSurface<HeadlessBackend>) {
	for (x, y) in probe_points(surface) {
		assert_eq!(
			surface.hit_test(x, y),
			exhaustive_hit_test(surface, x, y),
			"hit test at ({x}, {y})"
		);
	}
}

fn surface_with(tree: &Element) -> NativeSurface<HeadlessBackend> {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(VIEWPORT.0, VIEWPORT.1);
	surface.set_root(tree);
	surface
}

fn hidden(element: Element, hidden: bool) -> Element {
	element.with_props(|props| props.hidden = Some(hidden))
}

/// A row of overflowing descendants three levels deep, so a hit can land well
/// outside every ancestor's frame.
fn overflowing_tree(container_classes: &str) -> Element {
	View().class("w-full h-full").children([
		View().class(container_classes).child(
			View()
				.class("absolute left-[100px] top-4 w-10 h-10")
				.child(Button("deep").class("absolute left-[120px] top-[120px] w-12 h-12")),
		),
		View().class("w-16 h-16"),
	])
}

#[test]
fn overflowing_descendants_are_hit_outside_every_ancestor() {
	let surface = surface_with(&overflowing_tree("w-20 h-20"));
	let root = surface.root().unwrap();
	let container = surface.children(root)[0];
	let middle = surface.children(container)[0];
	let deep = surface.children(middle)[0];

	assert_eq!(surface.hit_test(250.0, 150.0), Some(deep));
	assert_matches_exhaustive(&surface);
}

#[test]
fn clipping_containers_exclude_their_overflow() {
	let surface = surface_with(&overflowing_tree("w-20 h-20 overflow-hidden"));
	let root = surface.root().unwrap();

	assert_eq!(surface.hit_test(250.0, 150.0), Some(root));
	assert_matches_exhaustive(&surface);
}

#[test]
fn rounded_clipping_matches_the_painted_shape() {
	let surface = surface_with(&View().class("w-full h-full").child(
		View().class("w-40 h-40 rounded-full").children([
			Button("fill").class("w-40 h-40"),
			Button("overflow").class("absolute left-[200px] w-10 h-10"),
		]),
	));
	let root = surface.root().unwrap();
	let circle = surface.children(root)[0];
	let fill = surface.children(circle)[0];

	assert_eq!(surface.hit_test(1.0, 1.0), Some(root));
	assert_eq!(surface.hit_test(80.0, 80.0), Some(fill));
	assert_eq!(surface.hit_test(210.0, 10.0), Some(root));
	assert_matches_exhaustive(&surface);
}

#[test]
fn hidden_subtrees_and_stopped_indicators_are_never_hit() {
	let tree = |hide: bool, running: bool| {
		View().class("w-full h-full").children([
			hidden(
				View()
					.class("w-20 h-20")
					.child(Button("overflow").class("absolute left-[150px] w-10 h-10")),
				hide,
			),
			ActivityIndicator(running).class("absolute left-[200px] top-[150px]"),
		])
	};
	let mut surface = surface_with(&tree(true, false));
	let root = surface.root().unwrap();
	let container = surface.children(root)[0];
	let button = surface.children(container)[0];
	let indicator = surface.children(root)[1];

	assert_eq!(surface.hit_test(160.0, 10.0), Some(root));
	assert_eq!(surface.hit_test(205.0, 155.0), Some(root));
	assert_matches_exhaustive(&surface);

	// A props-only commit invalidates the cached bounds.
	surface.set_root(&tree(false, true));
	assert_eq!(surface.hit_test(160.0, 10.0), Some(button));
	assert_eq!(surface.hit_test(205.0, 155.0), Some(indicator));
	assert_matches_exhaustive(&surface);

	surface.set_root(&tree(true, false));
	assert_eq!(surface.hit_test(160.0, 10.0), Some(root));
	assert_matches_exhaustive(&surface);
}

#[test]
fn pressable_decoration_never_widens_the_bounds() {
	let surface = surface_with(
		&View().class("w-full h-full").child(
			Pressable("Action")
				.class("w-20 h-10 overflow-visible")
				.child(Text("Overflow").class("absolute left-[150px] top-0 w-20 h-10")),
		),
	);
	let root = surface.root().unwrap();
	let pressable = surface.children(root)[0];

	assert_eq!(surface.hit_test(40.0, 5.0), Some(pressable));
	assert_eq!(surface.hit_test(160.0, 5.0), Some(root));
	assert_matches_exhaustive(&surface);
}

#[test]
fn reordered_and_removed_children_rebuild_the_bounds() {
	let positions = ["left-[10px]", "left-[200px]", "left-[100px] top-[150px]"];
	let tree = |order: &[usize]| {
		View()
			.class("w-20 h-20")
			.children(order.iter().map(|&index| {
				Button(format!("button {index}"))
					.class(format!("absolute {} w-10 h-10", positions[index]))
			}))
	};
	let mut surface = surface_with(&tree(&[0, 1, 2]));
	let root = surface.root().unwrap();
	assert_eq!(
		surface.hit_test(210.0, 5.0),
		Some(surface.children(root)[1])
	);
	assert_matches_exhaustive(&surface);

	surface.set_root(&tree(&[2, 0, 1]));
	assert_eq!(
		surface.hit_test(210.0, 5.0),
		Some(surface.children(root)[2])
	);
	assert_matches_exhaustive(&surface);

	surface.set_root(&tree(&[2]));
	assert_eq!(surface.hit_test(210.0, 5.0), None);
	assert_eq!(
		surface.hit_test(110.0, 155.0),
		Some(surface.children(root)[0])
	);
	assert_matches_exhaustive(&surface);
}

#[test]
fn viewport_relayout_rebuilds_the_bounds() {
	let tree = View().class("w-full h-full").child(
		View()
			.class("w-1/2 h-1/2")
			.child(Button("edge").class("absolute left-full top-full w-10 h-10")),
	);
	let mut surface = surface_with(&tree);
	assert_matches_exhaustive(&surface);

	surface.set_viewport(200.0, 120.0);
	let root = surface.root().unwrap();
	let button = surface.children(surface.children(root)[0])[0];
	assert_eq!(surface.hit_test(105.0, 65.0), Some(button));
	assert_matches_exhaustive(&surface);
}

#[test]
fn kind_changes_rebuild_the_replaced_subtree_bounds() {
	let view = || {
		View().class("w-full h-full").child(
			View()
				.class("w-20 h-10")
				.child(Button("overflow").class("absolute left-[150px] w-10 h-10")),
		)
	};
	let mut surface = surface_with(&view());
	let root = surface.root().unwrap();
	let container = surface.children(root)[0];
	let button = surface.children(container)[0];
	assert_eq!(surface.hit_test(160.0, 5.0), Some(button));

	// A different kind in the same slot tears the old subtree down and builds
	// a replacement with fresh ids in the same commit.
	surface.set_root(
		&View().class("w-full h-full").child(
			Pressable("Action")
				.class("w-20 h-10")
				.child(Text("Overflow").class("absolute left-[150px] w-10 h-10")),
		),
	);
	let pressable = surface.children(root)[0];
	assert_ne!(pressable, container);
	assert_eq!(surface.kind(pressable), NativeKind::Pressable);
	assert_eq!(surface.hit_test(40.0, 5.0), Some(pressable));
	assert_eq!(surface.hit_test(160.0, 5.0), Some(root));
	assert_matches_exhaustive(&surface);

	surface.set_root(&view());
	let rebuilt = surface.children(surface.children(root)[0])[0];
	assert_ne!(rebuilt, button);
	assert_eq!(surface.hit_test(160.0, 5.0), Some(rebuilt));
	assert_matches_exhaustive(&surface);
}

#[test]
fn refreshed_text_metrics_rebuild_the_bounds() {
	let mut surface = surface_with(
		&View()
			.class("w-full h-full items-start")
			.child(Element::text("hello")),
	);
	let root = surface.root().unwrap();
	let text = surface.children(root)[0];
	assert_eq!(surface.frame(text).unwrap().width, 40.0);
	assert_eq!(surface.hit_test(50.0, 5.0), Some(root));

	let backend = std::mem::take(surface.backend_mut());
	*surface.backend_mut() = backend.with_text_measurer(Arc::new(ApproxTextMeasurer {
		char_width: 12.0,
		line_height: 24.0,
	}));
	surface.refresh_text_measurer();
	assert_eq!(surface.frame(text).unwrap().width, 60.0);
	assert_eq!(surface.hit_test(50.0, 5.0), Some(text));
	assert_matches_exhaustive(&surface);
}

#[test]
fn queries_before_the_first_commit_find_nothing() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	assert_eq!(surface.hit_test(0.0, 0.0), None);

	surface.set_viewport(VIEWPORT.0, VIEWPORT.1);
	assert_eq!(surface.hit_test(0.0, 0.0), None);
}

#[test]
fn queries_outside_the_tree_find_nothing() {
	let surface = surface_with(&View().class("w-20 h-20"));

	assert_eq!(surface.hit_test(-1.0, 10.0), None);
	assert_eq!(surface.hit_test(f32::NAN, 10.0), None);
	assert_eq!(surface.hit_test(10.0, f32::INFINITY), None);
	assert!(surface.hit_test(10.0, 10.0).is_some());
}
