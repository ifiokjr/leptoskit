use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_native::Element;
use leptoskit_native::NativeKind;
use leptoskit_native::RadioGroup;
use leptoskit_native::SliderState;
use leptoskit_native::components::*;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;

#[test]
fn rich_pressables_activate_from_their_label_or_decorative_content() {
	let count = Arc::new(AtomicUsize::new(0));
	let mut tester = WidgetTester::new(
		move || {
			let action = Arc::clone(&count);
			Pressable("Increase")
				.class("w-60 h-20 p-4 justify-center")
				.child(Text(format!("Count: {}", count.load(Ordering::Relaxed))))
				.on_press(move || {
					action.fetch_add(1, Ordering::Relaxed);
				})
		},
		320.0,
		240.0,
	)
	.unwrap();

	tester
		.tap(&Finder::role(NativeKind::Pressable, "Increase"))
		.unwrap();
	tester.find(&Finder::text("Count: 1")).unwrap();
	tester.tap(&Finder::text("Count: 1")).unwrap();
	tester.find(&Finder::text("Count: 2")).unwrap();
}

#[test]
fn disabled_or_hidden_pressables_cannot_activate() {
	for hidden in [false, true] {
		let mut tester = WidgetTester::new(
			move || {
				View()
					.enabled(false)
					.with_props(|props| props.hidden = Some(hidden))
					.child(
						Pressable("Blocked")
							.class("w-60 h-20")
							.child(Text("Decoration"))
							.on_press(|| panic!("blocked action")),
					)
			},
			320.0,
			240.0,
		)
		.unwrap();
		assert!(tester.tap(&Finder::label("Blocked")).is_err());
	}
}

#[test]
fn tapping_boolean_controls_proposes_typed_changes_and_renders_owned_state() {
	for kind in [NativeKind::Switch, NativeKind::Checkbox] {
		let checked = Arc::new(AtomicBool::new(false));
		let mut tester = WidgetTester::new(
			move || {
				let state = Arc::clone(&checked);
				let current = checked.load(Ordering::Relaxed);
				let control = match kind {
					NativeKind::Switch => Switch(current),
					NativeKind::Checkbox => Checkbox(current),
					_ => unreachable!(),
				};
				control
					.accessibility_label("Choice")
					.on_toggle(move |value| state.store(value, Ordering::Relaxed))
			},
			320.0,
			240.0,
		)
		.unwrap();
		let finder = Finder::label("Choice");
		tester.tap(&finder).unwrap();
		assert_eq!(tester.find(&finder).unwrap().props.checked, Some(true));
		tester.tap(&finder).unwrap();
		assert_eq!(tester.find(&finder).unwrap().props.checked, Some(false));
		tester.toggle(&finder, true).unwrap();
		assert_eq!(tester.find(&finder).unwrap().props.checked, Some(true));
	}
}

#[test]
fn owners_can_reject_boolean_and_slider_proposals() {
	let boolean_proposals = Arc::new(Mutex::new(Vec::new()));
	let slider_proposals = Arc::new(Mutex::new(Vec::new()));
	let booleans = Arc::clone(&boolean_proposals);
	let sliders = Arc::clone(&slider_proposals);
	let mut tester = WidgetTester::new(
		move || {
			let booleans = Arc::clone(&booleans);
			let sliders = Arc::clone(&sliders);
			View().children([
				Checkbox(false)
					.accessibility_label("Consent")
					.on_toggle(move |value| booleans.lock().unwrap().push(value)),
				Slider(SliderState::new(0.0, 1.0, 0.0, Some(0.25)).unwrap())
					.accessibility_label("Volume")
					.on_value_change(move |value| sliders.lock().unwrap().push(value)),
			])
		},
		320.0,
		240.0,
	)
	.unwrap();
	tester.tap(&Finder::label("Consent")).unwrap();
	tester
		.set_slider_value(&Finder::label("Volume"), 0.7)
		.unwrap();
	assert_eq!(*boolean_proposals.lock().unwrap(), [true]);
	assert_eq!(*slider_proposals.lock().unwrap(), [0.75]);
	assert_eq!(
		tester
			.find(&Finder::label("Consent"))
			.unwrap()
			.props
			.checked,
		Some(false)
	);
	assert!(
		tester
			.find(&Finder::label("Volume"))
			.unwrap()
			.props
			.slider
			.unwrap()
			.value()
			.abs()
			< f64::EPSILON
	);
}

#[test]
fn radio_selection_is_owned_by_the_explicit_group() {
	let selected = Arc::new(AtomicBool::new(false));
	let mut tester = WidgetTester::new(
		move || {
			let choose_first = Arc::clone(&selected);
			let choose_second = Arc::clone(&selected);
			let second = selected.load(Ordering::Relaxed);
			let group = RadioGroup::new("theme").unwrap();
			View().children([
				Radio(group.clone(), !second)
					.accessibility_label("Light")
					.on_toggle(move |_| choose_first.store(false, Ordering::Relaxed)),
				Radio(group, second)
					.accessibility_label("Dark")
					.on_toggle(move |_| choose_second.store(true, Ordering::Relaxed)),
			])
		},
		320.0,
		240.0,
	)
	.unwrap();
	tester.tap(&Finder::label("Dark")).unwrap();
	assert_eq!(
		tester.find(&Finder::label("Light")).unwrap().props.checked,
		Some(false)
	);
	assert_eq!(
		tester.find(&Finder::label("Dark")).unwrap().props.checked,
		Some(true)
	);
	assert!(
		tester
			.toggle(&Finder::label("Dark"), false)
			.unwrap_err()
			.reason
			.contains("only propose selection")
	);
	tester.toggle(&Finder::label("Light"), true).unwrap();
	assert_eq!(
		tester.find(&Finder::label("Light")).unwrap().props.checked,
		Some(true)
	);
}

#[test]
fn sliders_snap_clamp_and_reject_nonfinite_proposals_without_rerendering() {
	let value = Arc::new(Mutex::new(0.0));
	let mut tester = WidgetTester::new(
		move || {
			let state = Arc::clone(&value);
			Slider(SliderState::new(0.0, 10.0, *value.lock().unwrap(), Some(3.0)).unwrap())
				.accessibility_label("Amount")
				.on_value_change(move |proposed| *state.lock().unwrap() = proposed)
		},
		320.0,
		240.0,
	)
	.unwrap();
	let finder = Finder::label("Amount");

	for (proposal, expected) in [(4.4, 3.0), (20.0, 9.0), (-10.0, 0.0)] {
		tester.set_slider_value(&finder, proposal).unwrap();
		let actual = tester.find(&finder).unwrap().props.slider.unwrap().value();
		assert!((actual - expected).abs() < f64::EPSILON);
	}

	let before = tester.tree();
	for proposal in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
		assert!(
			tester
				.set_slider_value(&finder, proposal)
				.unwrap_err()
				.reason
				.contains("finite")
		);
		assert_eq!(tester.tree(), before);
	}
}

#[test]
fn password_entry_updates_the_controlled_secure_field() {
	let value = Arc::new(Mutex::new(String::new()));
	let mut tester = WidgetTester::new(
		move || {
			let state = Arc::clone(&value);
			PasswordInput()
				.class("w-40 h-8")
				.accessibility_label("Password")
				.value(value.lock().unwrap().clone())
				.on_change(move |text| *state.lock().unwrap() = text.to_owned())
		},
		320.0,
		240.0,
	)
	.unwrap();
	tester
		.enter_text(&Finder::label("Password"), "test-only password")
		.unwrap();
	let password = tester.find(&Finder::label("Password")).unwrap();
	assert_eq!(password.kind, NativeKind::PasswordInput);
	assert_eq!(password.props.value.as_deref(), Some("test-only password"));
	assert!(tester.tree().contains("<redacted>"));
	assert!(!tester.tree().contains("test-only password"));
}

#[test]
fn blur_reaches_a_text_input_blur_handler_and_rejects_other_kinds() {
	let left = Arc::new(AtomicUsize::new(0));
	let mut tester = WidgetTester::new(
		{
			let left = Arc::clone(&left);
			move || {
				let left = Arc::clone(&left);
				View()
					.child(
						TextInput()
							.class("w-40 h-8")
							.accessibility_label("Email")
							.on(leptoskit_native::NativeEventSlot::Blur, move |_| {
								left.fetch_add(1, Ordering::Relaxed);
								leptoskit_native::EventOutcome::Handled
							}),
					)
					.child(Button("Send").class("w-20 h-8").on_press(|| {}))
			}
		},
		320.0,
		240.0,
	)
	.unwrap();

	tester.blur(&Finder::label("Email")).unwrap();
	assert_eq!(left.load(Ordering::Relaxed), 1);

	let error = tester.blur(&Finder::text("Send")).unwrap_err();
	assert!(error.reason.contains("is not a text input"));
	assert_eq!(left.load(Ordering::Relaxed), 1);
}

#[test]
fn control_actions_require_the_correct_kind_and_enabled_visible_ancestors() {
	let mut tester = WidgetTester::new(
		|| Button("Action").class("w-20 h-8").on_press(|| {}),
		320.0,
		240.0,
	)
	.unwrap();
	let action = Finder::text("Action");
	assert!(
		tester
			.toggle(&action, true)
			.unwrap_err()
			.reason
			.contains("boolean control")
	);
	assert!(
		tester
			.set_slider_value(&action, 1.0)
			.unwrap_err()
			.reason
			.contains("not a slider")
	);
	tester.tap(&action).unwrap();

	for hidden in [false, true] {
		let mut tester = WidgetTester::new(
			move || {
				View()
					.with_props(|props| {
						if hidden {
							props.hidden = Some(true);
						} else {
							props.enabled = Some(false);
						}
					})
					.children([
						Switch(false)
							.accessibility_label("Switch")
							.on_toggle(|_| panic!("blocked toggle")),
						Slider(SliderState::default())
							.accessibility_label("Slider")
							.on_value_change(|_| panic!("blocked slider")),
						PasswordInput()
							.class("w-40 h-8")
							.accessibility_label("Password")
							.on_change(|_| panic!("blocked password")),
					])
			},
			320.0,
			240.0,
		)
		.unwrap();
		assert!(tester.tap(&Finder::label("Switch")).is_err());
		assert!(tester.toggle(&Finder::label("Switch"), true).is_err());
		assert!(
			tester
				.set_slider_value(&Finder::label("Slider"), 0.5)
				.is_err()
		);
		assert!(
			tester
				.enter_text(&Finder::label("Password"), "blocked")
				.is_err()
		);
	}
}

#[test]
fn unhandled_control_proposals_fail_with_typed_diagnostics() {
	let mut tester = WidgetTester::new(
		|| {
			Element::view().children([
				Checkbox(false).accessibility_label("Checkbox"),
				Slider(SliderState::default()).accessibility_label("Slider"),
			])
		},
		320.0,
		240.0,
	)
	.unwrap();
	assert!(
		tester
			.tap(&Finder::label("Checkbox"))
			.unwrap_err()
			.reason
			.contains("Toggle")
	);
	assert!(
		tester
			.set_slider_value(&Finder::label("Slider"), 0.5)
			.unwrap_err()
			.reason
			.contains("ValueChange")
	);

	let mut tester = WidgetTester::new(
		|| {
			PasswordInput()
				.class("w-40 h-8")
				.accessibility_label("Password")
		},
		320.0,
		240.0,
	)
	.unwrap();
	let error = tester
		.enter_text(&Finder::label("Password"), "rejected test-only secret")
		.unwrap_err();
	assert!(error.reason.contains("password input event"));
	assert!(!error.to_string().contains("rejected test-only secret"));
}

#[test]
fn stopped_activity_is_not_a_visible_match_but_preserves_following_layout() {
	let tester = WidgetTester::new(
		|| {
			View().children([
				ActivityIndicator(false).accessibility_label("Loading"),
				Text("Following"),
			])
		},
		100.0,
		100.0,
	)
	.unwrap();
	assert!(tester.find_all(&Finder::label("Loading")).is_empty());
	assert!((tester.find(&Finder::text("Following")).unwrap().frame.y - 20.0).abs() < f32::EPSILON);
}
