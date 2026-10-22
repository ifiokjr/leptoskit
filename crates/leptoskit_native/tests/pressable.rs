//! Rich activation controls through the production renderer and validation.

use leptoskit_native::ControlTreeError;
use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::*;

#[test]
fn decorative_content_determines_layout_and_hits_target_the_pressable() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	let tree = Pressable("Open profile")
		.class("w-60 h-20 rounded-xl p-4 justify-center")
		.child(View().class("gap-1").children([
			Text("Ifiok Jr.").class("font-semibold"),
			Text("View profile"),
		]));

	surface.set_viewport(240.0, 80.0);
	surface.set_root(&tree);
	let root = surface.root().unwrap();
	let content = surface.children(root)[0];
	let frame = surface.frame(root).unwrap();
	assert!((frame.width - 240.0).abs() < 0.01);
	assert!((frame.height - 80.0).abs() < 0.01);
	assert!(surface.frame(content).unwrap().height > 0.0);
	assert_eq!(surface.props(root).text, None);
	assert_eq!(surface.hit_test(120.0, 40.0), Some(root));
	assert_eq!(surface.hit_test(1.0, 1.0), None);
}

#[test]
fn pressables_require_an_explicit_non_whitespace_accessible_name() {
	for element in [
		Element::new(NativeKind::Pressable),
		Pressable(""),
		Pressable(" \t\n\u{a0}"),
	] {
		assert_eq!(
			element.validate(),
			Err(ControlTreeError::MissingPressableLabel)
		);
	}
	assert!(Pressable("  Open profile  ").validate().is_ok());
}

#[test]
fn overflowing_decoration_never_extends_the_activation_bounds() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(
		&View().child(
			Pressable("Action")
				.class("w-20 h-10 overflow-visible")
				.child(Text("Overflow").class("absolute left-24 top-0 w-20 h-10")),
		),
	);
	let root = surface.root().unwrap();
	let pressable = surface.children(root)[0];
	assert_eq!(surface.hit_test(40.0, 20.0), Some(pressable));
	assert_eq!(surface.hit_test(120.0, 20.0), None);
}

#[test]
fn decoration_rejects_nested_controls_handlers_and_scrolling_in_all_variants() {
	let group = RadioGroup::new("choices").unwrap();

	for child in [
		Button("Nested"),
		Pressable("Nested"),
		TextInput(),
		PasswordInput(),
		Switch(false),
		Checkbox(false),
		Radio(group, false),
		Slider(SliderState::default()),
		ScrollView(),
		View().class("md:overflow-scroll"),
		View().with_style(|style| style.overflow = leptoskit_yoga::Overflow::Scroll),
		View().on_press(|| {}),
		Text("Handler").on_change(|_| {}),
		Element::new(NativeKind::Custom("platform-widget")),
	] {
		let descendant = child.kind;
		assert_eq!(
			Pressable("Action").child(View().child(child)).validate(),
			Err(ControlTreeError::InteractiveDescendant {
				container: NativeKind::Pressable,
				descendant,
			})
		);
	}

	assert!(
		Element::new(NativeKind::Custom("platform-widget"))
			.validate()
			.is_ok()
	);
	assert!(matches!(
		Button("Outer").child(Pressable("Inner")).validate(),
		Err(ControlTreeError::InteractiveDescendant {
			container: NativeKind::Button,
			..
		})
	));
}

#[test]
fn invalid_pressables_preserve_backend_and_retained_source_atomically() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(240.0, 80.0);
	surface.set_root(&Text("Previous interface"));
	let previous = surface.root().unwrap();
	surface.backend_mut().clear();

	for invalid in [Pressable(" "), Pressable("Invalid").child(Checkbox(false))] {
		assert!(surface.try_set_root(&invalid).is_err());
		assert_eq!(surface.root(), Some(previous));
		assert!(surface.backend().log().is_empty());
	}

	surface.set_viewport(300.0, 100.0);
	surface.set_dark_mode(true);
	assert_eq!(surface.root(), Some(previous));
	assert_eq!(
		surface.props(previous).text.as_deref(),
		Some("Previous interface")
	);
}

#[test]
fn an_href_makes_a_pressable_a_link_without_changing_native_rendering() {
	let link = Pressable("Open note").href("/notes/42");
	assert_eq!(link.props.href(), Some("/notes/42"));
	assert_eq!(Pressable("Plain").props.href(), None);

	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(200.0, 60.0);
	surface.set_root(&link.child(Text("Open")));
	let root = surface.root().unwrap();
	assert_eq!(surface.props(root).href(), Some("/notes/42"));
}
