//! Typed controls through the shared production renderer.
#![allow(clippy::float_cmp)]

use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_native::ControlTreeError;
use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeEventSlot;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_native::StyleContext;
use leptoskit_native::components::*;
use leptoskit_yoga::Dimension;

#[test]
fn validated_slider_supports_continuous_and_stepped_ranges() {
	let continuous = SliderState::new(-20.0, 80.0, 2.125, None).unwrap();
	assert_eq!(continuous.proposed_value(2.123), Some(2.123));
	assert_eq!(continuous.proposed_value(-99.0), Some(-20.0));
	assert_eq!(continuous.proposed_value(90.0), Some(80.0));
	assert_eq!(continuous.proposed_value(f64::NAN), None);

	let stepped = SliderState::new(-1.0, 1.0, -0.5, Some(0.25)).unwrap();
	assert_eq!(stepped.proposed_value(0.36), Some(0.25));
	assert_eq!(stepped.proposed_value(0.38), Some(0.5));
	assert_eq!(stepped.proposed_value(f64::INFINITY), None);

	let uneven = SliderState::new(0.0, 1.0, 0.3, Some(0.3)).unwrap();
	assert!((uneven.proposed_value(1.0).unwrap() - 0.9).abs() < 1e-12);
	let decimal = SliderState::new(0.0, 0.3, 0.3, Some(0.1)).unwrap();
	assert_eq!(decimal.proposed_value(0.3), Some(0.3));
}

#[test]
fn slider_rejects_invalid_and_unrepresentable_configuration() {
	for (min, max, value, step) in [
		(0.0, 0.0, 0.0, None),
		(2.0, 1.0, 1.0, None),
		(0.0, 1.0, 2.0, None),
		(0.0, f64::INFINITY, 0.0, None),
		(0.0, 1.0, f64::NAN, None),
		(-f64::MAX, f64::MAX, 0.0, None),
		(0.0, 1.0, 0.0, Some(0.0)),
		(0.0, 1.0, 0.0, Some(-1.0)),
		(0.0, 1.0, 0.0, Some(2.0)),
		(0.0, 1.0, 0.0, Some(f64::NAN)),
		(0.0, 1.0, 0.1, Some(0.25)),
		(0.0, 1.0, 0.0, Some(f64::MIN_POSITIVE)),
	] {
		assert!(SliderState::new(min, max, value, step).is_err());
	}
}

#[test]
fn progress_and_group_values_are_validated() {
	for invalid in [-0.1, 1.01, f64::NAN, f64::INFINITY] {
		assert!(ProgressValue::new(invalid).is_err());
	}
	assert_eq!(ProgressValue::new(0.0).unwrap().get(), 0.0);
	assert_eq!(ProgressValue::new(1.0).unwrap().get(), 1.0);
	assert!(RadioGroup::new("").is_err());
	assert!(RadioGroup::new("x".repeat(129)).is_err());
	assert_eq!(RadioGroup::new("plan").unwrap().name(), "plan");
}

#[test]
fn controls_have_measurable_defaults_and_classes_override_them() {
	let group = RadioGroup::new("plan").unwrap();
	let tree = View().class("gap-2").children([
		Switch(false),
		Checkbox(true),
		Radio(group, true),
		Slider(SliderState::default()),
		LinearProgressIndicator(Progress::Indeterminate),
		ActivityIndicator(true),
	]);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(400.0, 400.0);
	surface.set_root(&tree);

	for &id in surface.children(surface.root().unwrap()) {
		let layout = surface.layout(id);
		assert!(layout.width > 0.0 && layout.height > 0.0);
	}

	let slider = Slider(SliderState::default())
		.class("w-64 h-12")
		.resolved(&StyleContext::default());
	assert_eq!(slider.style.width, Dimension::Points(256.0));
	assert_eq!(slider.style.height, Dimension::Points(48.0));
}

#[test]
fn invalid_leaf_commit_is_atomic() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&Text("previous"));
	let root = surface.root().unwrap();

	for kind in [
		NativeKind::Image,
		NativeKind::TextInput,
		NativeKind::PasswordInput,
		NativeKind::Switch,
		NativeKind::Checkbox,
		NativeKind::Radio,
		NativeKind::Slider,
		NativeKind::LinearProgressIndicator,
		NativeKind::ActivityIndicator,
	] {
		let invalid = Element::new(kind).child(Text("invalid"));
		assert_eq!(
			surface.try_set_root(&invalid),
			Err(ControlTreeError::Children { kind })
		);
		assert_eq!(surface.root(), Some(root));
		assert_eq!(surface.props(root).text.as_deref(), Some("previous"));
	}
}

#[test]
fn buttons_reject_interactive_descendants_even_at_inactive_breakpoints() {
	for descendant in [
		TextInput(),
		Checkbox(false),
		ScrollView(),
		View().class("md:overflow-scroll"),
	] {
		assert!(matches!(
			Button("invalid").child(View().child(descendant)).validate(),
			Err(ControlTreeError::InteractiveDescendant {
				container: NativeKind::Button,
				..
			})
		));
	}
	assert!(
		Button("valid")
			.child(View().child(Text("decoration")))
			.validate()
			.is_ok()
	);
}

#[test]
fn radio_groups_are_explicit_and_reject_multiple_selected_options() {
	let group = RadioGroup::new("plan").unwrap();
	assert_eq!(
		Element::new(NativeKind::Radio).validate(),
		Err(ControlTreeError::MissingRadioGroup)
	);
	assert!(
		View()
			.children([Radio(group.clone(), true), Radio(group.clone(), false)])
			.validate()
			.is_ok()
	);
	assert_eq!(
		View()
			.children([Radio(group.clone(), true), Radio(group, true)])
			.validate(),
		Err(ControlTreeError::MultipleSelectedRadio {
			group: "plan".to_owned()
		})
	);
	assert!(
		View()
			.children([
				Radio(RadioGroup::new("one").unwrap(), true),
				Radio(RadioGroup::new("two").unwrap(), true)
			])
			.validate()
			.is_ok()
	);
}

#[test]
fn typed_events_bubble_once_and_keep_application_state_authoritative() {
	let proposals = Arc::new(Mutex::new(Vec::new()));
	let events = Arc::clone(&proposals);
	let tree = View()
		.on(NativeEventSlot::Toggle, move |event| {
			events.lock().unwrap().push(event.clone());
			EventOutcome::Handled
		})
		.child(Checkbox(false));
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(&tree);
	let root = surface.root().unwrap();
	let checkbox = surface.children(root)[0];
	assert_eq!(
		surface.dispatch_event(checkbox, &NativeEvent::Toggle { checked: true }),
		EventOutcome::Handled
	);
	assert_eq!(
		*proposals.lock().unwrap(),
		[NativeEvent::Toggle { checked: true }]
	);
	assert_eq!(surface.props(checkbox).checked, Some(false));
	surface.set_root(&tree);
	assert_eq!(surface.children(root), [checkbox]);
	assert_eq!(surface.props(checkbox).checked, Some(false));
	assert_eq!(
		surface.dispatch_event(root, &NativeEvent::Toggle { checked: true }),
		EventOutcome::Unhandled
	);
}

#[test]
fn slider_proposals_are_normalized_and_invalid_events_are_ignored() {
	let values = Arc::new(Mutex::new(Vec::new()));
	let proposals = Arc::clone(&values);
	let state = SliderState::new(0.0, 10.0, 0.0, Some(2.0)).unwrap();
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(
		&Slider(state).on_value_change(move |value| proposals.lock().unwrap().push(value)),
	);
	let root = surface.root().unwrap();
	assert_eq!(
		surface.dispatch_event(root, &NativeEvent::ValueChange { value: 7.1 }),
		EventOutcome::Handled
	);
	assert_eq!(
		surface.dispatch_event(root, &NativeEvent::ValueChange { value: 99.0 }),
		EventOutcome::Handled
	);
	assert_eq!(
		surface.dispatch_event(root, &NativeEvent::ValueChange { value: f64::NAN }),
		EventOutcome::Unhandled
	);
	assert_eq!(*values.lock().unwrap(), [8.0, 10.0]);
	assert_eq!(surface.props(root).slider, Some(state));
}

#[test]
fn radio_does_not_propose_deselection() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(
		&Radio(RadioGroup::new("plan").unwrap(), true).on_toggle(|_| panic!("no deselection")),
	);
	assert_eq!(
		surface.dispatch_event(
			surface.root().unwrap(),
			&NativeEvent::Toggle { checked: false }
		),
		EventOutcome::Unhandled
	);
}

#[test]
fn hidden_ancestry_is_materialized_for_animation_lifecycle_and_cannot_be_overridden() {
	let tree = View()
		.class("hidden")
		.child(View().class("flex").child(ActivityIndicator(true)));
	let resolved = tree.resolved(&StyleContext::default());
	assert_eq!(resolved.children[0].props.hidden, Some(true));
	assert_eq!(resolved.children[0].children[0].props.hidden, Some(true));
}

#[test]
fn disabled_ancestors_reject_typed_control_events() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(
		&View()
			.enabled(false)
			.child(Switch(false).on_toggle(|_| panic!("disabled"))),
	);
	let child = surface.children(surface.root().unwrap())[0];
	assert_eq!(
		surface.dispatch_event(child, &NativeEvent::Toggle { checked: true }),
		EventOutcome::Unhandled
	);
}

#[test]
fn secure_fields_measure_single_line_values_and_update_without_replacement() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(300.0, 100.0);
	surface.set_root(&PasswordInput().class("w-auto h-auto").value("abcd"));
	let root = surface.root().unwrap();
	assert_eq!(surface.layout(root).width, 32.0);
	assert_eq!(surface.layout(root).height, 16.0);
	surface.set_root(&PasswordInput().class("w-auto h-auto").value("abcdefgh"));
	assert_eq!(surface.root(), Some(root));
	assert_eq!(surface.layout(root).width, 64.0);
}

#[cfg(feature = "serde")]
#[test]
fn wire_values_round_trip_and_invalid_slider_state_is_rejected() {
	let props = Slider(SliderState::new(-1.0, 1.0, 0.0, Some(0.5)).unwrap()).props;
	let json = serde_json::to_string(&props).unwrap();
	assert_eq!(
		serde_json::from_str::<leptoskit_native::NativeProps>(&json).unwrap(),
		props
	);

	for json in [
		r#"{"slider":{"min":0,"max":10,"value":20,"step":null}}"#,
		r#"{"slider":{"min":0,"max":10,"value":5,"step":0}}"#,
		r#"{"slider":{"min":0,"max":10,"value":1,"step":2}}"#,
		r#"{"progress":{"determinate":2}}"#,
		r#"{"radio_group":""}"#,
	] {
		assert!(
			serde_json::from_str::<leptoskit_native::NativeProps>(json).is_err(),
			"{json}"
		);
	}
}
