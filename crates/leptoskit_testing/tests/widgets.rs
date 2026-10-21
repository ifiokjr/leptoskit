use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeEventSlot;
use leptoskit_native::NativeKind;
use leptoskit_testing::Finder;
use leptoskit_testing::WidgetTester;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Display;

fn box_sized(element: Element) -> Element {
	element.with_style(|style| {
		style.width = Dimension::Points(200.0);
		style.height = Dimension::Points(50.0);
	})
}

#[test]
fn obscured_and_offscreen_controls_do_not_receive_events() {
	use leptoskit_yoga::PositionType;
	let mut tester = WidgetTester::new(
		|| {
			box_sized(Element::view())
				.child(box_sized(Element::button("Covered")).on_press(|| panic!("covered")))
				.child(box_sized(Element::button("Overlay")).with_style(|style| {
					style.position = PositionType::Absolute;
					style.inset.left = Dimension::Points(0.0);
					style.inset.top = Dimension::Points(0.0);
				}))
		},
		390.0,
		844.0,
	)
	.unwrap();
	assert!(
		tester
			.tap(&Finder::text("Covered"))
			.unwrap_err()
			.reason
			.contains("obscured")
	);

	let tester = WidgetTester::new(
		|| {
			box_sized(Element::scroll_view()).child(box_sized(Element::button("Below")).with_style(
				|style| {
					style.position = PositionType::Absolute;
					style.inset.top = Dimension::Points(100.0);
				},
			))
		},
		390.0,
		844.0,
	)
	.unwrap();
	assert!(tester.find_all(&Finder::text("Below")).is_empty());
}

#[test]
fn native_hidden_and_clipping_properties_change_query_visibility() {
	use leptoskit_yoga::PositionType;

	for clipping in [false, true] {
		let mut renders = 0;
		let mut tester = WidgetTester::new(
			move || {
				renders += 1;
				box_sized(Element::view())
					.with_props(|props| {
						if clipping {
							props.clip = Some(renders == 1);
						} else {
							props.hidden = Some(renders == 1);
						}
					})
					.child(
						box_sized(Element::button("Reveal"))
							.with_style(|style| {
								if clipping {
									style.position = PositionType::Absolute;
									style.inset.top = Dimension::Points(100.0);
								}
							})
							.on_press(|| {}),
					)
			},
			390.0,
			844.0,
		)
		.unwrap();
		let finder = Finder::text("Reveal");
		assert!(tester.find_all(&finder).is_empty());
		assert!(tester.tap(&finder).is_err());
		tester.pump();
		tester.find(&finder).unwrap();
		tester.tap(&finder).unwrap();
	}
}

#[test]
fn tap_renders_the_new_count_and_requeries_after_replacement() {
	let count = Arc::new(AtomicUsize::new(0));
	let mut tester = WidgetTester::new(
		move || {
			let action = Arc::clone(&count);
			Element::view()
				.child(Element::text(format!(
					"Count: {}",
					count.load(Ordering::SeqCst)
				)))
				.child(box_sized(Element::button("Add")).on_press(move || {
					action.fetch_add(1, Ordering::SeqCst);
				}))
		},
		390.0,
		844.0,
	)
	.unwrap();
	tester
		.tap(&Finder::role(NativeKind::Button, "Add"))
		.unwrap();
	assert!(tester.find(&Finder::text("Count: 1")).is_ok());
	assert!(tester.find_all(&Finder::text("Count: 0")).is_empty());
	tester.tap(&Finder::text("Add")).unwrap();
	assert!(tester.find(&Finder::text("Count: 2")).is_ok());
}

#[test]
fn form_entry_updates_visible_feedback() {
	let value = Arc::new(Mutex::new(String::new()));
	let mut tester = WidgetTester::new(
		move || {
			let state = Arc::clone(&value);
			let current = value.lock().unwrap().clone();
			Element::view()
				.child(
					box_sized(Element::text_input())
						.with_props(|props| {
							props.aria.label = Some("Name".to_owned());
							props.value = Some(current.clone());
						})
						.on(NativeEventSlot::Change, move |event| {
							if let NativeEvent::Change { value } = event {
								state.lock().unwrap().clone_from(value);
							}
							EventOutcome::Handled
						}),
				)
				.child(Element::text(format!("Hello, {current}")))
		},
		390.0,
		844.0,
	)
	.unwrap();
	tester.enter_text(&Finder::label("Name"), "Ifiok").unwrap();
	assert!(tester.find(&Finder::text("Hello, Ifiok")).is_ok());
	assert_eq!(
		tester
			.find(&Finder::label("Name"))
			.unwrap()
			.props
			.value
			.as_deref(),
		Some("Ifiok")
	);
	assert!(
		tester
			.enter_text(&Finder::text("Hello, Ifiok"), "wrong")
			.is_err()
	);
}

#[test]
fn duplicate_controls_fail_with_diagnostics_instead_of_picking_the_first() {
	let tester = WidgetTester::new(
		|| {
			Element::view()
				.child(Element::button("Delete"))
				.child(Element::button("Delete"))
		},
		390.0,
		844.0,
	)
	.unwrap();
	let error = tester.find(&Finder::text("Delete")).unwrap_err();
	assert!(error.reason.contains("found 2"));
	assert_eq!(
		error.tree.matches("button text=Some(\"Delete\")").count(),
		2
	);
	assert_eq!(tester.find_all(&Finder::text("Delete")).len(), 2);
}

#[test]
fn labels_override_text_for_accessible_names_and_ids_are_explicit() {
	let tester = WidgetTester::new(
		|| {
			Element::button("+").with_props(|props| {
				props.aria.label = Some("Increase".to_owned());
				props.extra.insert("test_id".to_owned(), "increment".into());
			})
		},
		390.0,
		844.0,
	)
	.unwrap();
	assert!(
		tester
			.find(&Finder::role(NativeKind::Button, "Increase"))
			.is_ok()
	);
	assert!(tester.find(&Finder::role(NativeKind::Button, "+")).is_err());
	assert!(tester.find(&Finder::test_id("increment")).is_ok());
}

#[test]
fn disabled_ancestors_and_invisible_controls_cannot_be_activated() {
	for hidden in [false, true] {
		let mut tester = WidgetTester::new(
			move || {
				box_sized(Element::view())
					.with_props(|props| {
						props.enabled = Some(false);
						if hidden {
							props.opacity = Some(0.0);
						}
					})
					.child(Element::button("Danger").on_press(|| panic!("must not be activated")))
			},
			390.0,
			844.0,
		)
		.unwrap();
		assert!(tester.tap(&Finder::text("Danger")).is_err());
		assert_eq!(tester.find_all(&Finder::text("Danger")).is_empty(), hidden);
	}
	let mut tester = WidgetTester::new(
		|| {
			Element::view().child(
				Element::button("Hidden")
					.with_style(|style| style.display = Display::None)
					.on_press(|| panic!("hidden handler")),
			)
		},
		390.0,
		844.0,
	)
	.unwrap();
	assert!(tester.tap(&Finder::text("Hidden")).is_err());
}

#[test]
fn unhandled_actions_fail_and_missing_waits_have_a_finite_budget() {
	let mut renders = 0;
	let mut tester = WidgetTester::new(
		move || {
			renders += 1;
			Element::button(format!("Frame {renders}"))
		},
		390.0,
		844.0,
	)
	.unwrap();
	assert!(
		tester
			.tap(&Finder::text("Frame 1"))
			.unwrap_err()
			.reason
			.contains("did not handle")
	);
	assert!(tester.pump_until(&Finder::text("Frame 3"), 2).is_ok());
	let error = tester.pump_until(&Finder::text("Never"), 3).unwrap_err();
	assert!(error.reason.contains("within 3 pumps"));
	assert!(error.tree.contains("Frame 6"));
}

#[test]
fn removed_subtrees_do_not_match_and_state_is_isolated() {
	let mut renders = 0;
	let mut tester = WidgetTester::new(
		move || {
			renders += 1;
			if renders == 1 {
				Element::view().child(Element::text("Old"))
			} else {
				Element::button("New")
			}
		},
		390.0,
		844.0,
	)
	.unwrap();
	assert!(tester.find(&Finder::text("Old")).is_ok());
	tester.pump();
	assert!(tester.find_all(&Finder::text("Old")).is_empty());
	assert!(tester.find(&Finder::text("New")).is_ok());
	let other = WidgetTester::new(|| Element::text("Old"), 390.0, 844.0).unwrap();
	assert!(other.find_all(&Finder::text("New")).is_empty());
}

#[test]
fn viewport_changes_relayout_and_invalid_dimensions_leave_it_unchanged() {
	let mut tester = WidgetTester::new(
		|| {
			Element::button("Wide").with_style(|style| {
				style.width = Dimension::Percent(1.0);
			})
		},
		390.0,
		844.0,
	)
	.unwrap();
	let finder = Finder::text("Wide");
	assert!((tester.find(&finder).unwrap().frame.width - 390.0).abs() < 0.01);
	tester.resize(844.0, 390.0).unwrap();
	assert!((tester.find(&finder).unwrap().frame.width - 844.0).abs() < 0.01);
	assert!(tester.resize(f32::NAN, 20.0).is_err());
	assert!((tester.find(&finder).unwrap().frame.width - 844.0).abs() < 0.01);
}

#[test]
fn dropping_a_tester_releases_component_and_handler_state() {
	let state = Arc::new(AtomicUsize::new(0));
	let weak = Arc::downgrade(&state);
	let tester = WidgetTester::new(
		move || {
			let state = Arc::clone(&state);
			Element::button("Hold").on_press(move || {
				state.fetch_add(1, Ordering::Relaxed);
			})
		},
		390.0,
		844.0,
	)
	.unwrap();
	assert!(weak.upgrade().is_some());
	drop(tester);
	assert!(weak.upgrade().is_none());
}

#[test]
fn scroll_dispatches_to_the_component_and_renders_its_result() {
	let offset = Arc::new(Mutex::new(0.0));
	let mut tester = WidgetTester::new(
		move || {
			let state = Arc::clone(&offset);
			Element::view()
				.child(
					box_sized(Element::scroll_view())
						.with_props(|props| props.aria.label = Some("Feed".to_owned()))
						.on(NativeEventSlot::Scroll, move |event| {
							if let NativeEvent::Scroll { offset_y, .. } = event {
								*state.lock().unwrap() = *offset_y;
							}
							EventOutcome::Handled
						}),
				)
				.child(Element::text(format!("Offset {}", offset.lock().unwrap())))
		},
		390.0,
		844.0,
	)
	.unwrap();
	tester.scroll(&Finder::label("Feed"), 0.0, 42.0).unwrap();
	assert!(tester.find(&Finder::text("Offset 42")).is_ok());
	assert!(
		tester
			.scroll(&Finder::label("Feed"), f32::NAN, 0.0)
			.is_err()
	);
}
