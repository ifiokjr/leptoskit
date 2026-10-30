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
		#[cfg(feature = "rive")]
		NativeKind::Rive,
		NativeKind::TextInput,
		NativeKind::PasswordInput,
		NativeKind::Switch,
		NativeKind::Checkbox,
		NativeKind::Radio,
		NativeKind::Slider,
		NativeKind::LinearProgressIndicator,
		NativeKind::ActivityIndicator,
	] {
		let mut invalid = Element::new(kind).child(Text("invalid"));
		if kind == NativeKind::Rive {
			invalid = invalid
				.rive_playback(leptoskit_rive::Playback::new(
					leptoskit_rive::Asset::new(b"RIVE\x07\x00\x00\x00").unwrap(),
				))
				.aria_label("Illustration");
		}
		assert_eq!(
			surface.try_set_root(&invalid),
			Err(ControlTreeError::Children { kind })
		);
		assert_eq!(surface.root(), Some(root));
		assert_eq!(surface.props(root).text.as_deref(), Some("previous"));
	}
}

#[cfg(not(feature = "rive"))]
#[test]
fn rive_requires_the_runtime_feature() {
	let content =
		leptoskit_rive::Playback::new(leptoskit_rive::Asset::new(b"RIVE\x07\x00\x00\x00").unwrap());
	let error = Element::rive(content)
		.aria_label("Illustration")
		.validate()
		.unwrap_err();
	assert_eq!(error, ControlTreeError::RiveNotEnabled);
	assert!(
		error
			.to_string()
			.contains("enable the leptoskit rive feature")
	);
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
	surface.set_root(
		&PasswordInput()
			.aria_label("Password")
			.class("w-auto h-auto")
			.value("abcd"),
	);
	let root = surface.root().unwrap();
	assert_eq!(surface.layout(root).width, 32.0);
	assert_eq!(surface.layout(root).height, 28.0);
	surface.set_root(
		&PasswordInput()
			.aria_label("Password")
			.class("w-auto h-auto")
			.value("abcdefgh"),
	);
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

#[cfg(feature = "serde")]
#[test]
fn native_kinds_round_trip_as_wire_names_and_accept_legacy_serialization() {
	for kind in [
		NativeKind::View,
		NativeKind::Text,
		NativeKind::TextRun,
		NativeKind::Image,
		NativeKind::Rive,
		NativeKind::ScrollView,
		NativeKind::TextInput,
		NativeKind::Button,
		NativeKind::Switch,
		NativeKind::Checkbox,
		NativeKind::Radio,
		NativeKind::Slider,
		NativeKind::LinearProgressIndicator,
		NativeKind::ActivityIndicator,
		NativeKind::PasswordInput,
		NativeKind::Pressable,
		NativeKind::Custom("canvas"),
	] {
		let encoded = serde_json::to_string(&kind).unwrap();
		assert_eq!(encoded, format!("\"{}\"", kind.wire_name()));
		assert_eq!(serde_json::from_str::<NativeKind>(&encoded).unwrap(), kind);
	}

	assert_eq!(
		serde_json::from_str::<NativeKind>(r#""TextInput""#).unwrap(),
		NativeKind::TextInput
	);
	assert_eq!(
		serde_json::from_str::<NativeKind>(r#"{"Custom":"canvas"}"#).unwrap(),
		NativeKind::Custom("canvas")
	);
}

#[test]
fn stopped_activity_hides_but_retains_layout_space_and_ignores_input() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(100.0, 100.0);
	surface.set_root(&View().children([
		ActivityIndicator(false).on_press(|| panic!("stopped")),
		Text("next"),
	]));
	let root = surface.root().unwrap();
	let indicator = surface.children(root)[0];
	let next = surface.children(root)[1];
	assert_eq!(surface.layout(indicator).height, 20.0);
	assert_eq!(surface.layout(next).y, 20.0);
	assert_ne!(surface.hit_test(10.0, 10.0), Some(indicator));
	assert_eq!(
		surface.dispatch_event(indicator, &NativeEvent::Press),
		EventOutcome::Unhandled
	);
}

#[test]
fn slider_proposals_reconstruct_and_quantize_idempotently_across_float_scales() {
	let configurations = [
		(0.1, 100_000_000.0, 0.0001),
		(10_000_000_000.0, 10_000_000_001.0, 0.0001),
		(-10_000_000_001.0, -10_000_000_000.0, 0.0001),
		(-10_000_000_000.0, 10_000_000_000.0, 0.001),
		(0.0, 1.0, 1e-12),
		(1e100, 1e100 + 1e90, 1e87),
		(-1e100, 1e100, 1e86),
		(0.0, 1e-320, f64::from_bits(32)),
		(-f64::MAX / 2.0, f64::MAX / 2.0, f64::MAX / 16.0),
		(f64::MAX / 4.0, f64::MAX, f64::MAX * 0.4),
	];
	let mut random = 123_456_789_u64;

	for (min, max, step) in configurations {
		let state = SliderState::new(min, max, min, Some(step)).unwrap();

		for proposed in [300_000.099_7, 10_000_000_000.001, min, max] {
			let value = state.proposed_value(proposed).unwrap();
			let accepted = SliderState::new(min, max, value, Some(step)).unwrap();
			assert_eq!(accepted.value().to_bits(), value.to_bits());
			#[cfg(feature = "serde")]
			{
				let json = serde_json::to_string(&accepted).unwrap();
				assert_eq!(
					serde_json::from_str::<SliderState>(&json)
						.unwrap_or_else(|error| panic!("{json}: {error}")),
					accepted
				);
			}
			assert_eq!(
				accepted.proposed_value(value).unwrap().to_bits(),
				value.to_bits()
			);
		}

		for _ in 0..1024 {
			random = random
				.wrapping_mul(6_364_136_223_846_793_005)
				.wrapping_add(1);
			let fraction = (random >> 11) as f64 / (1_u64 << 53) as f64;
			let value = state
				.proposed_value((max - min).mul_add(fraction, min))
				.unwrap();
			let accepted = SliderState::new(min, max, value, Some(step)).unwrap();
			assert_eq!(accepted.value().to_bits(), value.to_bits());
			assert_eq!(
				accepted.proposed_value(value).unwrap().to_bits(),
				value.to_bits()
			);
		}
	}
}

#[test]
fn radio_group_names_reject_control_characters_and_preserve_unicode_identity() {
	for invalid in [
		"a\0b", "a\rb", "a\nb", "a\r\nb", "a\tb", "a\u{7f}b", "a\u{85}b",
	] {
		assert!(RadioGroup::new(invalid).is_err(), "accepted {invalid:?}");
		#[cfg(feature = "serde")]
		assert!(
			serde_json::from_str::<RadioGroup>(&serde_json::to_string(invalid).unwrap()).is_err()
		);
	}

	for valid in ["a�b", "école 🍇", "plan standard", "delivery"] {
		let group = RadioGroup::new(valid).unwrap();
		assert_eq!(group.name(), valid);
		#[cfg(feature = "serde")]
		assert_eq!(
			serde_json::from_str::<RadioGroup>(&serde_json::to_string(&group).unwrap()).unwrap(),
			group
		);
	}
}

#[test]
fn accepted_slider_grids_preserve_value_index_mapping_distribution() {
	fn random(state: &mut u64) -> f64 {
		*state = state
			.wrapping_mul(6_364_136_223_846_793_005)
			.wrapping_add(1_442_695_040_888_963_407);
		(*state >> 11) as f64 / (1_u64 << 53) as f64
	}

	let mut accepted = [0_usize; 6];
	let mut failures = [0_usize; 6];
	let mut check = |min: f64, max: f64, step: f64, proposal: f64| {
		let Ok(state) = SliderState::new(min, max, min, Some(step)) else {
			return;
		};
		let magnitude = min.abs().max(max.abs());
		let resolution = magnitude.next_up() - magnitude;
		let separation = step / resolution;
		let bucket = [1.0, 2.0, 4.0, 8.0, 16.0]
			.into_iter()
			.position(|limit| separation < limit)
			.unwrap_or(5);
		accepted[bucket] += 1;
		let value = state.proposed_value(proposal).unwrap();
		let stable = SliderState::new(min, max, value, Some(step)).is_ok_and(|updated| {
			updated.value().to_bits() == value.to_bits()
				&& updated.proposed_value(value).unwrap().to_bits() == value.to_bits()
		});
		if !stable {
			failures[bucket] += 1;
		}
	};
	check(
		-23_239.375_495_939_74,
		90_347.117_153_181_72,
		2.184_017_578_735_988_2e-11,
		52_378.746_521_646_24,
	);
	check(
		359_384.563_708_404_2,
		1_022_202.021_653_870_5,
		1.141_506_715_633_459e-10,
		835_289.438_859_453_5,
	);
	let mut seed = 65_754_638_u64;

	for _ in 0..200_000 {
		let scale = 10.0_f64.powf(random(&mut seed).mul_add(500.0, -250.0));
		let min = (random(&mut seed) - 0.5) * scale;
		let max = min + (0.1 + random(&mut seed)) * scale;
		let step = (max - min) / 2.0_f64.powf(random(&mut seed) * 53.0);
		let proposal = (max - min).mul_add(random(&mut seed), min);
		check(min, max, step, proposal);
	}

	eprintln!("step/ULP bins <1,<2,<4,<8,<16,>=16: accepted={accepted:?}, failures={failures:?}");
	assert!(
		accepted[5] > 100_000,
		"the accepted domain must remain substantial"
	);
	assert_eq!(
		failures, [0; 6],
		"every accepted grid must reconstruct its proposals without drift"
	);
}

#[test]
fn slider_rejects_grids_without_global_float_precision_headroom() {
	for (min, max, step) in [
		(
			-23_239.375_495_939_74,
			90_347.117_153_181_72,
			2.184_017_578_735_988_2e-11,
		),
		(
			359_384.563_708_404_2,
			1_022_202.021_653_870_5,
			1.141_506_715_633_459e-10,
		),
		(10_000_000_000.0, 10_000_000_001.0, 0.00001),
	] {
		assert!(SliderState::new(min, max, min, Some(step)).is_err());
	}

	let boundary = 16.0 * f64::EPSILON;
	assert!(SliderState::new(0.0, 1.0, 0.0, Some(boundary.next_down())).is_err());
	assert!(SliderState::new(0.0, 1.0, 0.0, Some(boundary)).is_ok());
	assert!(SliderState::new(0.1, 100_000_000.0, 0.1, Some(0.0001)).is_ok());
}

#[test]
fn slider_precision_boundary_round_trips_across_binary_exponents() {
	let mut accepted = 0;

	for exponent in -1070_i32..=1023 {
		let power = if exponent >= -1022 {
			f64::from_bits(((exponent + 1023) as u64) << 52)
		} else {
			f64::from_bits(1_u64 << (exponent + 1074))
		};

		for factor in [1.0, 1.234_567_89, 2.0_f64.next_down()] {
			let magnitude = power * factor;

			for (min, max) in [
				(-magnitude * 0.7, magnitude * 0.3),
				(-magnitude, 0.0),
				(0.0, magnitude),
				(magnitude * 0.5, magnitude),
			] {
				let endpoint = min.abs().max(max.abs());
				let next = endpoint.next_up();
				let resolution = if next.is_finite() {
					next - endpoint
				} else {
					endpoint - endpoint.next_down()
				};
				let step = 16.0 * resolution;
				let Ok(state) = SliderState::new(min, max, min, Some(step)) else {
					continue;
				};
				accepted += 1;

				for proposal in [min, min.next_up(), max, max.next_down(), 0.0, step, -step]
					.into_iter()
					.chain((0..=64).map(|index| (max - min).mul_add(f64::from(index) / 64.0, min)))
				{
					let value = state.proposed_value(proposal).unwrap();
					let reconstructed = SliderState::new(min, max, value, Some(step)).unwrap();
					assert_eq!(reconstructed.value().to_bits(), value.to_bits());
					assert_eq!(
						reconstructed.proposed_value(value).unwrap().to_bits(),
						value.to_bits()
					);
				}
			}
		}
	}

	assert!(
		accepted > 25_000,
		"boundary coverage must include a substantial accepted domain"
	);
}
