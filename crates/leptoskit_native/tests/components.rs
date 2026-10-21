//! Shared component behavior through the actual native reconciliation pipeline.
#![allow(clippy::float_cmp)]

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_native::Color;
use leptoskit_native::EventOutcome;
use leptoskit_native::FontWeight;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::Button;
use leptoskit_native::components::Image;
use leptoskit_native::components::ScrollView;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::View;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Size;

#[test]
fn primitives_use_the_shared_widget_vocabulary() {
	let tree = View().children([
		Text("title"),
		Button("press"),
		Image("avatar.png").accessibility_label("An avatar"),
		ScrollView(),
		TextInput()
			.value("hello")
			.placeholder("Search")
			.enabled(false),
	]);
	assert_eq!(tree.kind, NativeKind::View);
	assert_eq!(
		tree.children
			.iter()
			.map(|node| node.kind)
			.collect::<Vec<_>>(),
		[
			NativeKind::Text,
			NativeKind::Button,
			NativeKind::Image,
			NativeKind::ScrollView,
			NativeKind::TextInput,
		]
	);
	assert_eq!(
		tree.children[2].props.aria.label.as_deref(),
		Some("An avatar")
	);
	assert_eq!(tree.children[4].props.value.as_deref(), Some("hello"));
}

#[test]
fn viewport_changes_reapply_classes_and_preserve_committed_nodes() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	let screen = View()
		.class("w-full p-4 gap-2 md:flex-row")
		.children([View().class("w-20 h-10"), View().class("w-20 h-10")]);
	surface.set_viewport(320.0, 200.0);
	surface.set_root(&screen);
	let root = surface.root().expect("root");
	let first = surface.children(root)[0];
	let second = surface.children(root)[1];
	assert_eq!(
		(surface.layout(first).x, surface.layout(first).y),
		(16.0, 16.0)
	);
	assert_eq!(
		(surface.layout(second).x, surface.layout(second).y),
		(16.0, 64.0)
	);

	surface.set_viewport(900.0, 200.0);
	assert_eq!(surface.children(root), [first, second]);
	assert_eq!(
		(surface.layout(second).x, surface.layout(second).y),
		(104.0, 16.0)
	);
	surface.set_viewport(320.0, 200.0);
	assert_eq!(
		(surface.layout(second).x, surface.layout(second).y),
		(16.0, 64.0)
	);
	assert_eq!(
		screen.style.width,
		Dimension::Auto,
		"source styles stay unresolved"
	);
}

#[test]
fn classes_override_typed_bases_independently_of_builder_order() {
	let screen = View()
		.class("w-20")
		.with_style(|style| style.width = Dimension::Points(15.0))
		.class("h-10");
	let resolved = screen.resolved(&leptoskit_native::tailwind::StyleContext::default());
	assert_eq!(resolved.style.width, Dimension::Points(80.0));
	assert_eq!(resolved.style.height, Dimension::Points(40.0));
	assert!(resolved.classes().classes().is_empty());
	assert!(!screen.classes().classes().is_empty());
}

#[test]
fn dark_classes_update_inherited_text_without_a_new_screen() {
	let screen = View()
		.class("text-black text-xl font-bold dark:text-white")
		.child(Text("inherited"))
		.child(Text("explicit").color(Color::rgba(255, 0, 0, 255)));
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 200.0);
	surface.set_root(&screen);
	let root = surface.root().expect("root");
	let child = surface.children(root)[0];
	let explicit = surface.children(root)[1];
	assert_eq!(surface.props(child).color, Some(Color::rgba(0, 0, 0, 255)));
	assert_eq!(surface.props(child).font_size, Some(20.0));
	assert_eq!(surface.props(child).font_weight, Some(FontWeight::Bold));
	surface.set_dark_mode(true);
	assert_eq!(
		surface.props(child).color,
		Some(Color::rgba(255, 255, 255, 255))
	);
	assert_eq!(
		surface.props(explicit).color,
		Some(Color::rgba(255, 0, 0, 255))
	);
}

#[test]
fn viewport_units_are_resolved_again_after_resize() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(200.0, 100.0);
	surface.set_root(&View().class("w-[50vw] h-[50vh]"));
	let root = surface.root().expect("root");
	assert_eq!(
		(surface.layout(root).width, surface.layout(root).height),
		(100.0, 50.0)
	);
	surface.set_viewport(400.0, 300.0);
	assert_eq!(
		(surface.layout(root).width, surface.layout(root).height),
		(200.0, 150.0)
	);
}

#[test]
fn hidden_toggles_remove_hit_targets_and_restore_the_same_node() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 200.0);
	surface.set_root(
		&View()
			.class("w-full h-20 md:hidden")
			.child(Button("press").class("w-20 h-10")),
	);
	let root = surface.root().expect("root");
	let button = surface.children(root)[0];
	assert_eq!(surface.hit_test(5.0, 5.0), Some(button));
	surface.set_viewport(900.0, 200.0);
	assert_eq!(surface.props(root).hidden, Some(true));
	assert_eq!(surface.hit_test(5.0, 5.0), None);
	assert_eq!(
		surface.dispatch_event(button, &NativeEvent::Press),
		EventOutcome::Unhandled
	);
	surface.set_viewport(320.0, 200.0);
	assert_eq!(surface.children(root), [button]);
	assert_eq!(surface.props(root).hidden, None);
	assert_eq!(surface.hit_test(5.0, 5.0), Some(button));
}

#[test]
fn overflow_hit_testing_follows_clipping() {
	let child = || Button("outside").class("absolute left-[100px] w-10 h-10");
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(300.0, 200.0);
	surface.set_root(&View().class("w-20 h-20").child(child()));
	let root = surface.root().expect("root");
	let button = surface.children(root)[0];
	assert_eq!(surface.hit_test(110.0, 10.0), Some(button));

	surface.set_root(&View().class("w-20 h-20 overflow-hidden").child(child()));
	assert_eq!(surface.hit_test(110.0, 10.0), None);
	assert_eq!(surface.props(root).clip, Some(true));
}

#[test]
fn disabled_controls_do_not_dispatch_handlers() {
	let presses = Arc::new(AtomicUsize::new(0));
	let counted = Arc::clone(&presses);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&Button("disabled").enabled(false).on_press(move || {
		counted.fetch_add(1, Ordering::SeqCst);
	}));
	let root = surface.root().expect("root");
	assert_eq!(
		surface.dispatch_event(root, &NativeEvent::Press),
		EventOutcome::Unhandled
	);
	assert_eq!(presses.load(Ordering::SeqCst), 0);
}

#[test]
fn repeated_environment_values_emit_no_mutations() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 200.0);
	surface.set_root(&View().class("p-4 md:p-8").child(Text("hello")));
	surface.backend_mut().clear();
	surface.set_viewport(320.0, 200.0);
	surface.set_dark_mode(false);
	assert!(surface.backend().log().is_empty());
}

#[test]
fn invalid_dynamic_classes_leave_the_committed_tree_untouched() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&Text("committed"));
	let root = surface.root().expect("root");
	let invalid = View().try_class("md:hover:totally-unknown");
	assert!(invalid.is_err());
	assert_eq!(surface.props(root).text.as_deref(), Some("committed"));
}

#[test]
fn resolution_inherits_before_resolving_current_color_borders() {
	let tree = View().class("text-white").child(View().class("border p-2"));
	let resolved = tree.resolved(&leptoskit_native::tailwind::StyleContext {
		viewport: Size::new(320.0, 200.0),
		dark: false,
	});
	assert_eq!(
		resolved.children[0].props.border_color,
		Some(Color::rgba(255, 255, 255, 255))
	);
}

#[test]
fn scrolling_utilities_lower_containers_to_real_scroll_widgets() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 200.0);
	surface.set_root(
		&View()
			.class("w-20 h-20 overflow-scroll")
			.child(View().class("w-40 h-40")),
	);
	let root = surface.root().expect("root");
	assert_eq!(surface.kind(root), NativeKind::ScrollView);
	assert_eq!(surface.props(root).clip, Some(true));
}

#[test]
fn text_input_measures_and_remeasures_the_displayed_value() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 200.0);
	surface.set_root(&TextInput().aria_label("Name").value("hello"));
	let root = surface.root().expect("root");
	assert_eq!(surface.layout(root).width, 40.0);
	assert_eq!(surface.layout(root).height, 16.0);

	surface.set_root(&TextInput().aria_label("Name").value("hello world"));
	assert_eq!(surface.layout(root).width, 88.0);

	surface.set_root(&TextInput().value("").placeholder("Search"));
	assert_eq!(surface.layout(root).width, 48.0);
}

#[test]
fn explicit_display_and_overflow_classes_override_typed_visual_flags() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 200.0);
	surface.set_root(
		&View()
			.with_props(|props| {
				props.hidden = Some(true);
				props.clip = Some(true);
			})
			.class("flex overflow-visible w-20 h-20"),
	);
	let root = surface.root().expect("root");
	assert_eq!(surface.props(root).hidden, Some(false));
	assert_eq!(surface.props(root).clip, Some(false));
	assert_eq!(surface.hit_test(10.0, 10.0), Some(root));
}

#[test]
fn disabled_ancestors_suppress_descendant_events() {
	let presses = Arc::new(AtomicUsize::new(0));
	let counted = Arc::clone(&presses);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(
		&View()
			.enabled(false)
			.child(Button("child").on_press(move || {
				counted.fetch_add(1, Ordering::SeqCst);
			})),
	);
	let root = surface.root().expect("root");
	let child = surface.children(root)[0];
	assert_eq!(surface.props(child).enabled, Some(false));
	assert_eq!(
		surface.dispatch_event(child, &NativeEvent::Press),
		EventOutcome::Unhandled
	);
	assert_eq!(presses.load(Ordering::SeqCst), 0);
}

#[test]
fn rounded_containers_clip_hit_targets_like_the_platform_paints_them() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 200.0);
	surface.set_root(
		&View()
			.class("w-20 h-20 rounded-lg")
			.child(Button("outside").class("absolute left-[100px] w-10 h-10")),
	);
	assert_eq!(surface.hit_test(110.0, 10.0), None);

	surface.set_root(
		&View()
			.class("w-20 h-20 rounded-full")
			.child(Button("inside").class("w-20 h-20")),
	);
	let root = surface.root().expect("root");
	let child = surface.children(root)[0];
	for (x, y) in [(1.0, 1.0), (79.0, 1.0), (1.0, 79.0), (79.0, 79.0)] {
		assert_eq!(surface.hit_test(x, y), None);
	}
	assert_eq!(surface.hit_test(40.0, 40.0), Some(child));
	assert_eq!(surface.hit_test(40.0, 0.0), Some(child));
}

#[test]
fn single_line_input_height_is_independent_of_constrained_width() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 200.0);
	for screen in [
		TextInput()
			.aria_label("Name")
			.class("w-10")
			.value("hello world"),
		TextInput().class("w-10").placeholder("a long placeholder"),
		TextInput()
			.aria_label("Name")
			.class("w-10")
			.value("updated longer text"),
	] {
		surface.set_root(&screen);
		let root = surface.root().expect("root");
		assert_eq!(surface.layout(root).width, 40.0);
		assert_eq!(surface.layout(root).height, 16.0);
	}
}

#[test]
fn scroll_primitive_overflow_overrides_use_matching_native_widgets() {
	let context = leptoskit_native::tailwind::StyleContext::default();
	let scrolling = ScrollView().resolved(&context);
	assert_eq!(scrolling.kind, NativeKind::ScrollView);
	assert_eq!(scrolling.props.clip, Some(true));

	let visible = ScrollView().class("overflow-visible").resolved(&context);
	assert_eq!(visible.kind, NativeKind::View);
	assert_eq!(visible.props.clip, Some(false));

	let clipped = ScrollView().class("overflow-hidden").resolved(&context);
	assert_eq!(clipped.kind, NativeKind::View);
	assert_eq!(clipped.props.clip, Some(true));
}
