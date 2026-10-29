//! Held controls through the production renderer: `active:` utilities, the
//! default pressed dim, and how a hold ends.
#![allow(clippy::float_cmp)]

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeProps;
use leptoskit_native::NativeSurface;
use leptoskit_native::Role;
use leptoskit_native::StyleContext;
use leptoskit_native::Tailwind;
use leptoskit_native::components::*;
use leptoskit_yoga::NodeId;
use leptoskit_yoga::Style;

fn color(class: &str) -> Option<Color> {
	let mut props = NativeProps::default();
	Tailwind::parse(class).unwrap().apply(
		&mut Style::default(),
		&mut props,
		&StyleContext::default(),
	);
	props.background
}

fn mounted(tree: &Element) -> NativeSurface<HeadlessBackend> {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 480.0);
	surface.set_root(tree);
	surface.backend_mut().clear();
	surface
}

fn child(surface: &NativeSurface<HeadlessBackend>, index: usize) -> NodeId {
	surface.children(surface.root().unwrap())[index]
}

fn counter() -> Element {
	View().children([
		Text("Count: 0"),
		Button("Add one").class("rounded-lg bg-indigo-500 px-6 py-3 active:bg-indigo-700"),
	])
}

#[test]
fn a_held_button_takes_its_active_utilities_until_released() {
	let mut surface = mounted(&counter());
	let button = child(&surface, 1);
	let resting = surface.props(button).clone();
	assert_eq!(resting.background, color("bg-indigo-500"));

	surface.press_in(button);
	assert_eq!(surface.pressed(), Some(button));
	assert_eq!(surface.props(button).background, color("bg-indigo-700"));
	// Only the held control changes, in place: no node is rebuilt and
	// assistive technology keeps the same name and role.
	assert_eq!(
		surface.backend().log(),
		[Mutation::UpdateProps {
			id: button,
			props: surface.props(button).clone(),
		}]
	);
	assert_eq!(surface.props(button).accessible_name(), Some("Add one"));
	assert_eq!(surface.props(button).aria(), resting.aria());

	surface.backend_mut().clear();
	surface.press_out(button);
	assert_eq!(surface.pressed(), None);
	assert_eq!(surface.props(button), &resting);
	assert_eq!(
		surface.backend().log(),
		[Mutation::UpdateProps {
			id: button,
			props: resting,
		}]
	);
}

#[test]
fn repeated_and_mismatched_phases_change_nothing() {
	let mut surface = mounted(&counter());
	let button = child(&surface, 1);
	surface.press_out(button);
	surface.press_in(button);
	surface.backend_mut().clear();
	surface.press_in(button);
	surface.press_out(child(&surface, 0));
	assert_eq!(surface.backend().log(), []);
	assert_eq!(surface.pressed(), Some(button));
}

#[test]
fn only_enabled_visible_buttons_and_pressables_can_be_held() {
	let mut surface = mounted(
		&View().children([
			Text("Label"),
			Button("Disabled")
				.class("bg-black active:bg-white")
				.enabled(false),
			View()
				.with_props(|props| props.hidden = Some(true))
				.child(Button("Hidden").class("bg-black active:bg-white")),
			View()
				.enabled(false)
				.child(Pressable("Inside a disabled view").class("bg-black")),
			Checkbox(false),
		]),
	);
	let hidden = surface.children(child(&surface, 2))[0];
	let inside = surface.children(child(&surface, 3))[0];

	for target in [
		child(&surface, 0),
		child(&surface, 1),
		hidden,
		inside,
		child(&surface, 4),
	] {
		surface.press_in(target);
		assert_eq!(surface.pressed(), None, "{target:?} cannot be held");
	}
	assert_eq!(surface.backend().log(), []);
}

#[test]
fn holding_another_control_releases_the_first() {
	let mut surface = mounted(&View().children([
		Button("First").class("bg-black active:bg-white"),
		Button("Second").class("bg-black active:bg-white"),
	]));
	let (first, second) = (child(&surface, 0), child(&surface, 1));
	surface.press_in(first);
	surface.press_in(second);
	assert_eq!(surface.pressed(), Some(second));
	assert_eq!(surface.props(first).background, color("bg-black"));
	assert_eq!(surface.props(second).background, color("bg-white"));
}

#[test]
fn renders_during_a_hold_keep_it_and_renders_that_end_it_restore_the_slot() {
	let mut surface = mounted(&counter());
	let button = child(&surface, 1);
	surface.press_in(button);

	// A press handler usually re-renders while the control is still held.
	surface.set_root(&View().children([
		Text("Count: 1"),
		Button("Add one").class("rounded-lg bg-indigo-500 px-6 py-3 active:bg-indigo-700"),
	]));
	assert_eq!(surface.pressed(), Some(button));
	assert_eq!(surface.props(button).background, color("bg-indigo-700"));

	// Disabling the held control ends the hold: it cannot be pressed.
	surface.set_root(
		&View().children([
			Text("Count: 1"),
			Button("Add one")
				.class("rounded-lg bg-indigo-500 px-6 py-3 active:bg-indigo-700")
				.enabled(false),
		]),
	);
	assert_eq!(surface.pressed(), None);
	assert_eq!(surface.props(button).background, color("bg-indigo-500"));

	// Replacing the held control builds an unpressed widget in its slot,
	// which no release will ever reach.
	surface.set_root(&counter());
	surface.press_in(button);
	surface.set_root(&View().children([
		Text("Count: 1"),
		Pressable("Replacement").class("bg-indigo-500 active:bg-indigo-700"),
	]));
	let replacement = child(&surface, 1);
	assert_ne!(replacement, button);
	assert_eq!(surface.pressed(), None);
	assert_eq!(
		surface.props(replacement).background,
		color("bg-indigo-500")
	);

	// Removing the held control ends the hold as well.
	surface.press_in(replacement);
	surface.set_root(&View().child(Text("Count: 1")));
	assert_eq!(surface.pressed(), None);
	surface.press_out(replacement);
	assert_eq!(surface.pressed(), None);
}

#[test]
fn controls_without_active_utilities_dim_unless_they_keep_native_chrome() {
	let mut surface = mounted(&View().children([
		Button("Native chrome"),
		Button("Drawn").class("bg-indigo-500"),
		Button("Bordered").class("border-2 border-black"),
		Pressable("Card").class("opacity-50"),
		Pressable("Styled").class("active:opacity-90"),
	]));
	let held = |surface: &mut NativeSurface<HeadlessBackend>, index| {
		let id = child(surface, index);
		surface.press_in(id);
		let opacity = surface.props(id).opacity;
		surface.press_out(id);
		opacity
	};
	// The platform draws its own highlight on a native-chrome button.
	assert_eq!(held(&mut surface, 0), None);
	assert_eq!(held(&mut surface, 1), Some(0.7));
	assert_eq!(held(&mut surface, 2), Some(0.7));
	// The dim multiplies the control's own opacity.
	assert_eq!(held(&mut surface, 3), Some(0.5 * 0.7));
	// Any `active:` utility replaces the default.
	assert_eq!(held(&mut surface, 4), Some(0.9));
	assert_eq!(surface.props(child(&surface, 3)).opacity, Some(0.5));
}

#[test]
fn press_phases_reach_no_event_handler() {
	let presses = Arc::new(AtomicUsize::new(0));
	let counted = Arc::clone(&presses);
	let surface = mounted(&View().child(Button("Add").on_press(move || {
		counted.fetch_add(1, Ordering::SeqCst);
	})));
	let button = child(&surface, 0);

	for phase in [NativeEvent::PressIn, NativeEvent::PressOut] {
		assert_eq!(
			surface.dispatch_event(button, &phase),
			EventOutcome::Unhandled
		);
	}
	assert_eq!(presses.load(Ordering::SeqCst), 0);
	assert_eq!(
		surface.dispatch_event(button, &NativeEvent::Press),
		EventOutcome::Handled
	);
}

#[test]
fn active_utilities_style_only_controls_with_a_pressed_state() {
	for element in [
		View(),
		Text("Label"),
		Image("photo.png"),
		TextInput(),
		Switch(false),
	] {
		let kind = element.kind;
		let error = element.try_class("p-2 active:bg-black").unwrap_err();
		assert_eq!(error.class, "active:bg-black");
		assert!(error.reason.contains(kind.wire_name()), "{error}");
	}
	assert!(Button("Go").try_class("active:bg-black").is_ok());
	assert!(Pressable("Go").try_class("active:bg-black").is_ok());
	// A role does not add a pressed state the platforms could report.
	assert!(
		View()
			.role(Role::Button)
			.try_class("active:bg-black")
			.is_err()
	);
}

#[test]
fn resolving_with_an_active_context_shows_every_control_pressed() {
	let tree = View().class("bg-white").children([
		Button("One").class("bg-black active:bg-red-500"),
		Pressable("Two").class("bg-black"),
	]);
	let pressed = tree.resolved(&StyleContext {
		active: true,
		..StyleContext::default()
	});
	assert_eq!(pressed.props.background, color("bg-white"));
	assert_eq!(pressed.children[0].props.background, color("bg-red-500"));
	assert_eq!(pressed.children[1].props.opacity, Some(0.7));
	let resting = tree.resolved(&StyleContext::default());
	assert_eq!(resting.children[0].props.background, color("bg-black"));
	assert_eq!(resting.children[1].props.opacity, None);
}
