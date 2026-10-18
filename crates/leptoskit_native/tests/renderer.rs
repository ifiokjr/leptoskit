//! Renderer integration tests: diffing, mutation batches, events, hit testing.

// Frames are exact floats produced by the same arithmetic the engine runs.
#![allow(clippy::float_cmp)]

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeEventSlot;
use leptoskit_native::NativeSurface;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::FlexDirection;

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
