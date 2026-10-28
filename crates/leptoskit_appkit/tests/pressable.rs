//! Main-thread tests of native rich button hit routing and activation.
#![allow(unsafe_code, clippy::float_cmp)]

fn main() {
	let mut args = libtest_mimic::Arguments::from_args();
	args.test_threads = Some(1);
	#[cfg(target_os = "macos")]
	let trials = macos::trials();
	#[cfg(not(target_os = "macos"))]
	let trials = Vec::new();
	libtest_mimic::run(&args, trials).exit();
}

#[cfg(target_os = "macos")]
mod macos {
	use std::cell::RefCell;
	use std::rc::Rc;

	use leptoskit_appkit::AppKitBackend;
	use leptoskit_appkit::MainThreadMarker;
	use leptoskit_native::Element;
	use leptoskit_native::NativeEvent;
	use leptoskit_native::NativeSurface;
	use leptoskit_native::components::*;
	use leptoskit_yoga::NodeId;
	use libtest_mimic::Trial;
	use objc2::Message;
	use objc2::rc::Retained;
	use objc2_app_kit::NSAccessibility;
	use objc2_app_kit::NSApplication;
	use objc2_app_kit::NSButton;
	use objc2_app_kit::NSEvent;
	use objc2_app_kit::NSEventModifierFlags;
	use objc2_app_kit::NSEventType;
	use objc2_app_kit::NSWindow;
	use objc2_foundation::NSPoint;
	use objc2_foundation::NSString;
	use objc2_quartz_core::kCACornerCurveCircular;

	pub fn trials() -> Vec<Trial> {
		[
			(
				"decorative_content_uses_flipped_coordinates_and_button_hits",
				decorative_content_uses_flipped_coordinates_and_button_hits as fn(),
			),
			(
				"pressable_exposes_one_accessible_action_and_clears_stale_targets",
				pressable_exposes_one_accessible_action_and_clears_stale_targets,
			),
			(
				"focused_pressables_activate_with_the_native_space_key",
				focused_pressables_activate_with_the_native_space_key,
			),
			(
				"native_press_can_remove_its_own_rich_content_tree",
				native_press_can_remove_its_own_rich_content_tree,
			),
			(
				"held_buttons_show_their_pressed_styles_until_the_mouse_is_released",
				held_buttons_show_their_pressed_styles_until_the_mouse_is_released,
			),
		]
		.into_iter()
		.map(|(name, test)| {
			Trial::test(name, move || {
				test();
				Ok(())
			})
		})
		.collect()
	}

	fn tree() -> Element {
		View().child(
			Pressable("Open profile")
				.class("w-60 h-20 p-4 rounded-xl")
				.child(View().child(Text("Visible content"))),
		)
	}

	fn mounted() -> NativeSurface<AppKitBackend> {
		let mtm = MainThreadMarker::new().expect("main-thread test harness");
		NSApplication::sharedApplication(mtm);
		let mut surface = NativeSurface::new(AppKitBackend::new(mtm));
		surface.set_viewport(320.0, 240.0);
		surface.set_root(&tree());
		surface
	}

	fn button(surface: &NativeSurface<AppKitBackend>) -> Retained<NSButton> {
		let id = surface.children(surface.root().unwrap())[0];
		surface
			.backend()
			.view(id)
			.unwrap()
			.downcast_ref::<NSButton>()
			.expect("native NSButton")
			.retain()
	}

	fn decorative_content_uses_flipped_coordinates_and_button_hits() {
		let mut surface = mounted();
		let button = button(&surface);
		assert!(button.isFlipped());
		assert!(!button.isBordered());
		assert!(button.title().is_empty());
		let content = button.subviews().objectAtIndex(0);
		assert!((content.frame().origin.y - 16.0).abs() < 0.01);
		let hit = button.hitTest(NSPoint::new(24.0, 24.0)).unwrap();
		assert_eq!(hit.downcast_ref::<NSButton>(), Some(&*button));
		assert!(button.hitTest(NSPoint::new(1.0, 1.0)).is_none());
		// SAFETY: QuartzCore exports this immutable corner-curve name.
		assert_eq!(&*button.layer().unwrap().cornerCurve(), unsafe {
			kCACornerCurveCircular
		});

		surface.set_root(
			&View().child(
				Pressable("Open profile")
					.class("w-60 h-20 rounded-none")
					.child(Text("Changed content")),
			),
		);
		assert!(button.hitTest(NSPoint::new(1.0, 1.0)).is_some());
		assert_eq!(button.subviews().len(), 1);
		surface.set_root(
			&View().child(
				Pressable("Open profile")
					.class("w-20 h-10 overflow-visible")
					.child(Text("Overflow").class("absolute left-24 top-0 w-20 h-10")),
			),
		);
		assert!(button.hitTest(NSPoint::new(120.0, 20.0)).is_none());
	}

	fn pressable_exposes_one_accessible_action_and_clears_stale_targets() {
		let mut surface = mounted();
		let button = button(&surface);
		let events = Rc::new(RefCell::new(Vec::new()));
		let received = Rc::clone(&events);
		surface
			.backend()
			.set_event_listener(move |id, event| received.borrow_mut().push((id, event.clone())));
		let id = surface.children(surface.root().unwrap())[0];
		assert_eq!(button.accessibilityRole().unwrap().to_string(), "AXButton");
		assert_eq!(
			button.accessibilityLabel().unwrap().to_string(),
			"Open profile"
		);
		assert!(
			button
				.accessibilityChildren()
				.is_none_or(|children| children.is_empty())
		);
		assert!(button.accessibilityPerformPress());
		assert_eq!(*events.borrow(), [(id, NativeEvent::Press)]);

		surface
			.set_root(&View().child(Pressable("Open profile").enabled(false).class("w-60 h-20")));
		assert!(!button.isEnabled());
		// SAFETY: NSButton accepts an optional sender for its standard action.
		unsafe { button.performClick(None) };
		assert_eq!(events.borrow().len(), 1);
		surface.set_root(&View());
		assert!(button.target().is_none());
		assert!(button.action().is_none());
		// SAFETY: retained detached NSButtons remain valid objects; the
		// backend has cleared their target and action before deletion.
		unsafe { button.performClick(None) };
		assert_eq!(events.borrow().len(), 1);
	}

	fn key(
		window: &NSWindow,
		kind: NSEventType,
		character: &str,
		code: u16,
		repeat: bool,
	) -> Retained<NSEvent> {
		let character = NSString::from_str(character);
		NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
			kind, NSPoint::ZERO, NSEventModifierFlags::empty(), 0.0,
			window.windowNumber(), None, &character, &character, repeat, code,
		).expect("native keyboard event")
	}

	fn mouse(window: &NSWindow, kind: NSEventType, x: f64, y: f64) -> Retained<NSEvent> {
		NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
			kind, NSPoint::new(x, y), NSEventModifierFlags::empty(), 0.0,
			window.windowNumber(), None, 0, 1, 1.0,
		).expect("native mouse event")
	}

	/// What the listener saw of a held control while AppKit tracked it.
	#[derive(Debug, Default)]
	struct Hold {
		phases: Vec<(NodeId, NativeEvent)>,
		backgrounds: Vec<Option<Retained<objc2_core_graphics::CGColor>>>,
		opacities: Vec<f64>,
		labels: Vec<String>,
	}

	/// `NSButton` tracks the mouse inside `mouseDown:` until it is released.
	/// The press phases bracket that loop, so the control is restyled while
	/// it is held, then restored, and the click still activates it.
	fn held_buttons_show_their_pressed_styles_until_the_mouse_is_released() {
		let mtm = MainThreadMarker::new().expect("main-thread test harness");
		let app = NSApplication::sharedApplication(mtm);
		let mut surface = NativeSurface::new(AppKitBackend::new(mtm));
		surface.set_viewport(320.0, 240.0);
		surface.set_root(
			&View().class("p-4 gap-4").children([
				Button("Add one").class("w-40 h-12 rounded-lg bg-indigo-500 active:bg-indigo-700"),
				Pressable("Open card")
					.class("w-40 h-12 bg-indigo-500")
					.child(Text("Card")),
			]),
		);
		let window = surface
			.backend()
			.open_window(surface.root().unwrap(), "Held button test")
			.unwrap();
		let surface = Rc::new(RefCell::new(surface));
		let hold = Rc::new(RefCell::new(Hold::default()));
		let (weak, seen) = (Rc::downgrade(&surface), Rc::clone(&hold));
		surface
			.borrow()
			.backend()
			.set_event_listener(move |id, event| {
				let surface = weak.upgrade().unwrap();
				match event {
					NativeEvent::PressIn => surface.borrow_mut().press_in(id),
					NativeEvent::PressOut => surface.borrow_mut().press_out(id),
					_ => {}
				}
				let surface = surface.borrow();
				let view = surface.backend().view(id).unwrap();
				let mut seen = seen.borrow_mut();
				seen.phases.push((id, event.clone()));
				seen.backgrounds
					.push(view.layer().unwrap().backgroundColor());
				seen.opacities.push(view.alphaValue());
				let button = view.downcast_ref::<NSButton>().unwrap();
				seen.labels.push(format!(
					"{:?} {:?} {}",
					view.accessibilityRole(),
					view.accessibilityLabel(),
					button.title()
				));
			});
		let ids = {
			let surface = surface.borrow();
			surface.children(surface.root().unwrap()).to_vec()
		};

		// The window fits the 144-point-tall root, and its coordinates start
		// at the bottom left: the button spans flipped y 16–64 and the
		// pressable y 80–128.
		for (id, y) in [(ids[0], 104.0), (ids[1], 40.0)] {
			hold.replace(Hold::default());
			// NSButton's own tracking loop, which runs synchronously inside
			// the mouse-down below, consumes the queued release.
			app.postEvent_atStart(&mouse(&window, NSEventType::LeftMouseUp, 96.0, y), false);
			window.sendEvent(&mouse(&window, NSEventType::LeftMouseDown, 96.0, y));
			let hold = hold.borrow();
			assert_eq!(
				hold.phases,
				[
					(id, NativeEvent::PressIn),
					(id, NativeEvent::Press),
					(id, NativeEvent::PressOut)
				],
			);
			assert_eq!(surface.borrow().pressed(), None);
			// The release's activation arrives while the control is still held.
			assert!(hold.labels.iter().all(|label| label == &hold.labels[0]));
			assert_eq!(hold.backgrounds[0], hold.backgrounds[1]);
			assert_eq!(hold.opacities[0], hold.opacities[1]);

			if id == ids[0] {
				assert_ne!(
					hold.backgrounds[1], hold.backgrounds[2],
					"active:bg-indigo-700 shows only while held"
				);
				assert_eq!(hold.opacities, [1.0, 1.0, 1.0]);
			} else {
				assert_eq!(hold.backgrounds[1], hold.backgrounds[2]);
				assert!(
					(hold.opacities[0] - 0.7).abs() < 1e-6,
					"{:?}",
					hold.opacities
				);
				assert_eq!(hold.opacities[2], 1.0);
			}
		}
		window.close();
	}

	fn native_press_can_remove_its_own_rich_content_tree() {
		let surface = Rc::new(RefCell::new(mounted()));
		let button = button(&surface.borrow());
		let weak = Rc::downgrade(&surface);
		surface.borrow().backend().set_event_listener(move |_, _| {
			if let Some(surface) = weak.upgrade() {
				surface.borrow_mut().set_root(&View());
			}
		});
		// SAFETY: the retained native button remains alive even when its
		// action removes the committed tree that originally owned it.
		unsafe { button.performClick(None) };
		let surface = surface.borrow();
		assert!(surface.children(surface.root().unwrap()).is_empty());
		assert!(button.target().is_none());
		assert!(button.action().is_none());
	}

	fn focused_pressables_activate_with_the_native_space_key() {
		let surface = mounted();
		let button = button(&surface);
		let root = surface.root().unwrap();
		let window = surface
			.backend()
			.open_window(root, "Pressable keyboard test")
			.unwrap();
		let presses = Rc::new(RefCell::new(0));
		let received = Rc::clone(&presses);
		surface.backend().set_event_listener(move |_, event| {
			if *event == NativeEvent::Press {
				*received.borrow_mut() += 1;
			}
		});
		assert!(window.makeFirstResponder(Some(&button)));
		assert_eq!(
			window.firstResponder().unwrap().downcast_ref::<NSButton>(),
			Some(&*button)
		);
		for (index, character, code) in [(1, " ", 49), (2, "\r", 36)] {
			window.sendEvent(&key(&window, NSEventType::KeyDown, character, code, false));
			window.sendEvent(&key(&window, NSEventType::KeyUp, character, code, false));
			assert_eq!(*presses.borrow(), index);
		}
		window.sendEvent(&key(&window, NSEventType::KeyDown, " ", 49, true));
		assert_eq!(*presses.borrow(), 2);
		button.setEnabled(false);
		window.sendEvent(&key(&window, NSEventType::KeyDown, " ", 49, false));
		assert_eq!(*presses.borrow(), 2);
		window.close();
	}
}
