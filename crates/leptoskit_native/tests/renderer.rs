//! Renderer integration tests: diffing, mutation batches, events, hit testing.

// Frames are exact floats produced by the same arithmetic the engine runs.
#![allow(clippy::float_cmp)]

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_native::ApproxTextMeasurer;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::FontWeight;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeEventSlot;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::Align;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::Rect;
use leptoskit_yoga::Size;

fn counter_app(count: u32, presses: Arc<AtomicUsize>) -> Element {
	Element::view()
		.with_style(|style| {
			style.flex_direction = FlexDirection::Column;
			style.width = Dimension::Percent(1.0);
			style.height = Dimension::Percent(1.0);
		})
		.child(Element::text(format!("Count: {count}")))
		.child(
			Element::button("Increment").on(NativeEventSlot::Press, move |_| {
				presses.fetch_add(1, Ordering::SeqCst);
				EventOutcome::Handled
			}),
		)
}

#[test]
fn first_commit_creates_the_whole_tree() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(&counter_app(0, Arc::new(AtomicUsize::new(0))));

	let log = surface.backend().log();
	assert!(log.iter().any(|mutation| {
		matches!(
			mutation,
			Mutation::Create { kind, .. } if kind.to_string() == "text"
		)
	}));
	assert!(
		log.iter()
			.any(|mutation| matches!(mutation, Mutation::InsertChild { index: 1, .. }))
	);
	assert!(log.iter().any(|mutation| {
		matches!(
			mutation,
			Mutation::UpdateLayout { width, height, .. } if *width == 320.0 && *height == 240.0
		)
	}));
}

#[test]
fn prop_change_updates_only_the_changed_node() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	let presses = Arc::new(AtomicUsize::new(0));
	surface.set_root(&counter_app(0, Arc::clone(&presses)));
	surface.backend_mut().clear();

	surface.set_root(&counter_app(1, Arc::clone(&presses)));

	let log = surface.backend().log();
	let creates = log
		.iter()
		.filter(|mutation| matches!(mutation, Mutation::Create { .. }))
		.count();
	let updates = log
		.iter()
		.filter(|mutation| matches!(mutation, Mutation::UpdateProps { .. }))
		.count();
	// The count text is the only changed widget; the button keeps its props.
	assert_eq!(updates, 1);
	assert_eq!(creates, 0);
	assert!(log.iter().any(|mutation| {
		matches!(
			mutation,
			Mutation::UpdateProps { props, .. } if props.text.as_deref() == Some("Count: 1")
		)
	}));
}

#[test]
fn identical_commit_is_a_no_op() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	let presses = Arc::new(AtomicUsize::new(0));
	surface.set_root(&counter_app(5, Arc::clone(&presses)));
	surface.backend_mut().clear();

	surface.set_root(&counter_app(5, Arc::clone(&presses)));

	assert_eq!(surface.backend().log().len(), 0);
}

#[test]
fn removed_children_are_deleted() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(
		&Element::view()
			.child(Element::text("one"))
			.child(Element::text("two"))
			.child(Element::text("three")),
	);
	surface.backend_mut().clear();

	surface.set_root(&Element::view().child(Element::text("one")));

	let log = surface.backend().log();
	assert!(
		log.iter()
			.any(|mutation| matches!(mutation, Mutation::RemoveChild { .. }))
	);
	assert_eq!(
		log.iter()
			.filter(|mutation| matches!(mutation, Mutation::Delete { .. }))
			.count(),
		2
	);
}

#[test]
fn press_events_reach_handlers_and_bubble() {
	let presses = Arc::new(AtomicUsize::new(0));
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(&Element::view().child(Element::button("Increment").on(
		NativeEventSlot::Press,
		{
			let presses = Arc::clone(&presses);
			move |_| {
				presses.fetch_add(1, Ordering::SeqCst);
				EventOutcome::Handled
			}
		},
	)));

	let root = surface.root().expect("root exists");
	let button_id = surface.children(root)[0];
	let outcome = surface.dispatch_event(button_id, &NativeEvent::Press);

	assert_eq!(outcome, EventOutcome::Handled);
	assert_eq!(presses.load(Ordering::SeqCst), 1);
}

#[test]
fn unhandled_events_keep_bubbling() {
	let presses = Arc::new(AtomicUsize::new(0));
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(
		&Element::view()
			// The parent handles presses that the child does not.
			.on(NativeEventSlot::Press, {
				let presses = Arc::clone(&presses);
				move |_| {
					presses.fetch_add(1, Ordering::SeqCst);
					EventOutcome::Handled
				}
			})
			.child(Element::text("plain")),
	);

	let root = surface.root().expect("root exists");
	let text_id = surface.children(root)[0];
	let outcome = surface.dispatch_event(text_id, &NativeEvent::Press);

	assert_eq!(outcome, EventOutcome::Handled);
	assert_eq!(presses.load(Ordering::SeqCst), 1);
}

#[test]
fn hit_test_finds_the_deepest_node() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(
		&Element::view()
			.with_style(|style| {
				style.width = Dimension::Points(320.0);
				style.height = Dimension::Points(240.0);
			})
			.child(
				Element::text("hello").with_style(|style| style.width = Dimension::Points(50.0)),
			),
	);

	let root = surface.root().expect("root exists");
	let text_id = surface.children(root)[0];
	assert_eq!(surface.hit_test(10.0, 5.0), Some(text_id));
	assert_eq!(surface.hit_test(10.0, 200.0), Some(root));
	assert_eq!(surface.hit_test(-5.0, 5.0), None);
}

#[test]
fn text_is_measured_and_laid_out() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(&Element::view().child(
		Element::text("hello world").with_style(|style| style.width = Dimension::Points(200.0)),
	));

	let root = surface.root().expect("root exists");
	let text_id = surface.children(root)[0];
	let frame = surface.frame(text_id).expect("text frame");
	// The explicit width wins; the height comes from the approximate measurer:
	// one 16px line for 11 characters bounded to 200px.
	assert_eq!((frame.width, frame.height), (200.0, 16.0));
}

#[test]
fn kind_change_replaces_the_subtree() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(&Element::view().child(Element::text("label")));
	surface.backend_mut().clear();

	surface.set_root(&Element::view().child(Element::image("cover.png")));

	let log = surface.backend().log();
	assert!(
		log.iter()
			.any(|mutation| matches!(mutation, Mutation::Delete { .. }))
	);
	assert!(log.iter().any(|mutation| {
		matches!(
			mutation,
			Mutation::Create { kind, .. } if kind.to_string() == "image"
		)
	}));
}

#[test]
fn viewport_change_relayouts() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(&Element::view().with_style(|style| {
		style.width = Dimension::Percent(1.0);
		style.height = Dimension::Percent(1.0);
	}));
	surface.backend_mut().clear();

	surface.set_viewport(640.0, 480.0);

	let root = surface.root().expect("root exists");
	let frame = surface.frame(root).expect("root frame");
	assert_eq!((frame.width, frame.height), (640.0, 480.0));
}

#[test]
fn container_roots_fill_the_viewport_width_like_a_web_div() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(
		&Element::view()
			.class("items-center")
			.child(Element::text("Hi")),
	);
	let root = surface.root().expect("root exists");
	let text = surface.children(root)[0];
	assert_eq!(surface.frame(root).expect("root frame").width, 320.0);
	// The approximation sets "Hi" in two 8-unit cells, centred in 320.
	let label = surface.frame(text).expect("text frame");
	assert_eq!((label.x, label.width), (152.0, 16.0));

	// Margins come off the filled width, and a max width still caps it.
	surface.set_root(&Element::view().class("mx-10").child(Element::text("Hi")));
	assert_eq!(surface.frame(root).expect("root frame").width, 240.0);
	surface.set_root(&Element::view().class("max-w-40").child(Element::text("Hi")));
	assert_eq!(surface.frame(root).expect("root frame").width, 160.0);

	// A text root keeps its intrinsic width, as an inline `<span>` does.
	surface.set_root(&Element::text("Hi"));
	let root = surface.root().expect("root exists");
	assert_eq!(surface.frame(root).expect("root frame").width, 16.0);
}

#[test]
fn safe_area_change_relayouts_safe_padding_only_when_it_changes() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 640.0);
	surface.set_root(
		&Element::view()
			.class("w-full h-full pt-safe-offset-4 pb-safe")
			.child(Element::view().class("flex-1")),
	);
	let root = surface.root().expect("root exists");
	let content = surface.children(root)[0];
	assert_eq!(surface.frame(content).expect("content frame").y, 16.0);

	surface.set_safe_area(Rect::all_edges(24.0, 0.0, 48.0, 0.0));
	let frame = surface.frame(content).expect("content frame");
	assert_eq!((frame.y, frame.height), (40.0, 552.0));

	surface.backend_mut().clear();
	surface.set_safe_area(Rect::all_edges(24.0, 0.0, 48.0, 0.0));
	assert_eq!(surface.backend().log(), []);
}

fn non_layout_mutations(surface: &NativeSurface<HeadlessBackend>) -> Vec<Mutation> {
	surface
		.backend()
		.log()
		.iter()
		.filter(|mutation| !matches!(mutation, Mutation::UpdateLayout { .. }))
		.cloned()
		.collect()
}

#[test]
fn kind_change_replaces_the_node_once_in_its_slot() {
	let image_element =
		|| Element::image("x").with_style(|style| style.height = Dimension::Points(10.0));
	let tree = |middle: Element| {
		Element::view()
			.child(Element::text("a"))
			.child(middle)
			.child(Element::text("c"))
	};
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(&tree(Element::text("b")));
	let root = surface.root().expect("root exists");
	let before = surface.children(root).to_vec();
	surface.backend_mut().clear();

	surface.set_root(&tree(image_element()));

	let after = surface.children(root).to_vec();
	assert_eq!(after.len(), 3);
	assert_eq!((after[0], after[2]), (before[0], before[2]));
	let image = after[1];
	assert_eq!(surface.kind(image), NativeKind::Image);
	assert_eq!(surface.parent(image), Some(root));
	assert_eq!(
		non_layout_mutations(&surface),
		vec![
			Mutation::RemoveChild {
				parent: root,
				child: before[1],
			},
			Mutation::Delete { id: before[1] },
			Mutation::Create {
				id: image,
				kind: NativeKind::Image,
				props: surface.props(image).clone(),
			},
			Mutation::InsertChild {
				parent: root,
				child: image,
				index: 1,
			},
		]
	);
	// Layout order matches the shadow order: the image sits between a and c.
	let ys: Vec<f32> = after.iter().map(|&id| surface.layout(id).y).collect();

	assert!(ys[0] < ys[1] && ys[1] < ys[2], "layout order {ys:?}");

	surface.backend_mut().clear();
	surface.set_root(&tree(image_element()));
	assert_eq!(surface.backend().log(), []);
	assert_eq!(surface.children(root), after.as_slice());
}

#[test]
fn text_change_remeasures_the_node() {
	let tree = |content: &str| {
		Element::view()
			.with_style(|style| style.align_items = Align::FlexStart)
			.child(Element::text(content))
	};
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(&tree("Count: 9"));
	let root = surface.root().expect("root exists");
	let text_id = surface.children(root)[0];
	assert_eq!(surface.frame(text_id).expect("frame").width, 64.0);

	surface.set_root(&tree("Count: 10"));

	assert_eq!(surface.frame(text_id).expect("frame").width, 72.0);
}

#[test]
fn clearing_text_removes_its_measurement() {
	let tree = |text: Element| {
		Element::view()
			.with_style(|style| style.align_items = Align::FlexStart)
			.child(text)
	};
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(&tree(Element::text("hello world")));
	let root = surface.root().expect("root exists");
	let text_id = surface.children(root)[0];
	assert_eq!(surface.frame(text_id).expect("frame").width, 88.0);

	surface.set_root(&tree(
		Element::text("hello world").with_props(|props| props.text = None),
	));

	let frame = surface.frame(text_id).expect("frame");
	assert_eq!((frame.width, frame.height), (0.0, 0.0));
}

#[test]
fn deleted_nodes_no_longer_receive_or_bubble_events() {
	let presses = Arc::new(AtomicUsize::new(0));
	let counting = |presses: &Arc<AtomicUsize>| {
		let presses = Arc::clone(presses);
		move |_: &NativeEvent| {
			presses.fetch_add(1, Ordering::SeqCst);
			EventOutcome::Handled
		}
	};
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(
		&Element::view()
			.on(NativeEventSlot::Press, counting(&presses))
			.child(Element::button("Go").on(NativeEventSlot::Press, counting(&presses))),
	);
	let root = surface.root().expect("root exists");
	let button_id = surface.children(root)[0];

	surface.set_root(&Element::view().on(NativeEventSlot::Press, counting(&presses)));

	assert_eq!(surface.parent(button_id), None);
	assert_eq!(
		surface.dispatch_event(button_id, &NativeEvent::Press),
		EventOutcome::Unhandled
	);
	assert_eq!(presses.load(Ordering::SeqCst), 0);
}

fn measure(text: &str, width: f32) -> (f32, f32) {
	let size = ApproxTextMeasurer::new().measure(
		text,
		TextStyle::default(),
		Size::new(AvailableSpace::Definite(width), AvailableSpace::MaxContent),
	);
	(size.width, size.height)
}

#[test]
fn approx_measurer_reports_the_widest_line() {
	// "aaaaaaaaaa" fills the 80px line; "b" wraps onto a narrower second line.
	assert_eq!(measure("aaaaaaaaaa b", 80.0), (80.0, 32.0));
	assert_eq!(measure("hello world", 48.0), (40.0, 32.0));
	assert_eq!(measure("hello world", 200.0), (88.0, 16.0));
}

#[test]
fn approx_measurer_clamps_overlong_words_consistently() {
	// A 96px word is clamped to the 80px line whether it comes first or later.
	assert_eq!(measure("aaaaaaaaaaaa", 80.0), (80.0, 16.0));
	assert_eq!(measure("b aaaaaaaaaaaa", 80.0), (80.0, 32.0));
}

#[test]
fn approx_measurer_scales_with_font_size() {
	let measurer = ApproxTextMeasurer::new();
	let unbounded = Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent);
	let body = measurer.measure("hello", TextStyle::default(), unbounded);
	let title = measurer.measure(
		"hello",
		TextStyle::new(TextStyle::DEFAULT_FONT_SIZE * 2.0, FontWeight::Bold),
		unbounded,
	);
	assert_eq!((body.width, body.height), (40.0, 16.0));
	assert_eq!((title.width, title.height), (80.0, 32.0));
}

#[test]
fn font_change_remeasures_the_node() {
	let tree = |size: f32| {
		Element::view()
			.with_style(|style| style.align_items = Align::FlexStart)
			.child(Element::text("hello").font(size, FontWeight::Regular))
	};
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(&tree(TextStyle::DEFAULT_FONT_SIZE));
	let root = surface.root().expect("root exists");
	let text_id = surface.children(root)[0];
	let before = surface.frame(text_id).expect("frame");
	assert_eq!((before.width, before.height), (40.0, 16.0));

	surface.set_root(&tree(TextStyle::DEFAULT_FONT_SIZE * 1.5));

	let after = surface.frame(text_id).expect("frame");
	assert_eq!((after.width, after.height), (60.0, 24.0));
}

/// Counts measurements, delegating to the approximation.
struct CountingMeasurer(Arc<AtomicUsize>);

impl TextMeasurer for CountingMeasurer {
	fn measure(&self, text: &str, style: TextStyle, available: Size<AvailableSpace>) -> Size<f32> {
		self.0.fetch_add(1, Ordering::SeqCst);
		ApproxTextMeasurer::new().measure(text, style, available)
	}
}

#[test]
fn color_change_does_not_remeasure_text() {
	let tree = |color: Color| {
		Element::view()
			.with_style(|style| style.align_items = Align::FlexStart)
			.child(Element::text("hello").color(color))
	};
	let measurements = Arc::new(AtomicUsize::new(0));
	let backend = HeadlessBackend::new()
		.with_text_measurer(Arc::new(CountingMeasurer(Arc::clone(&measurements))));
	let mut surface = NativeSurface::new(backend);
	surface.set_viewport(320.0, 240.0);
	surface.set_root(&tree(Color::rgba(0, 0, 0, 255)));
	let measured = measurements.load(Ordering::SeqCst);
	assert!(measured > 0);

	surface.backend_mut().clear();

	surface.set_root(&tree(Color::rgba(255, 0, 0, 255)));

	assert_eq!(measurements.load(Ordering::SeqCst), measured);
	assert!(
		surface
			.backend()
			.log()
			.iter()
			.any(|mutation| matches!(mutation, Mutation::UpdateProps { .. }))
	);
}

#[test]
fn refreshing_font_metrics_reflows_without_replacing_widgets_or_handlers() {
	let presses = Arc::new(AtomicUsize::new(0));
	let counter = Arc::clone(&presses);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.refresh_text_measurer();
	surface.set_viewport(320.0, 240.0);
	surface.set_root(
		&Element::view()
			.with_style(|style| style.align_items = Align::FlexStart)
			.child(Element::text("hello").on_press(move || {
				counter.fetch_add(1, Ordering::SeqCst);
			})),
	);
	let root = surface.root().unwrap();
	let text = surface.children(root)[0];

	assert_eq!(surface.frame(text).unwrap().width, 40.0);
	surface.backend_mut().clear();
	let backend = std::mem::take(surface.backend_mut());
	*surface.backend_mut() = backend.with_text_measurer(Arc::new(ApproxTextMeasurer {
		char_width: 12.0,
		line_height: 24.0,
	}));
	surface.refresh_text_measurer();
	assert_eq!(surface.root(), Some(root));
	assert_eq!(surface.children(root), &[text]);
	assert_eq!(surface.frame(text).unwrap().width, 60.0);
	assert_eq!(surface.frame(text).unwrap().height, 24.0);

	assert!(
		surface
			.backend()
			.log()
			.iter()
			.all(|mutation| matches!(mutation, Mutation::UpdateLayout { .. }))
	);
	assert_eq!(
		surface.dispatch_event(text, &NativeEvent::Press),
		EventOutcome::Handled
	);
	assert_eq!(presses.load(Ordering::SeqCst), 1);
	surface.set_root(&Element::view());
	surface.backend_mut().clear();
	surface.refresh_text_measurer();
	assert_eq!(surface.backend().log(), []);
}

#[test]
fn native_text_receives_the_nodes_layout_direction() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	let tree =
		Element::text("RTL").with_style(|style| style.direction = leptoskit_yoga::Direction::Rtl);
	surface.set_viewport(320.0, 240.0);
	surface.set_root(&tree);
	let root = surface.root().unwrap();
	assert_eq!(
		surface.props(root).text_style().direction,
		leptoskit_yoga::Direction::Rtl
	);
	surface.set_root(&Element::text("LTR"));

	assert_eq!(
		surface.props(root).text_style().direction,
		leptoskit_yoga::Direction::Ltr
	);
}
