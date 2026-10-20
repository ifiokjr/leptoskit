//! Main-thread integration tests against real native control actions.
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
	use objc2::rc::Retained;
	use objc2_app_kit::NSAccessibility;
	use objc2_app_kit::NSApplication;
	use objc2_app_kit::NSButton;
	use objc2_app_kit::NSControl;
	use objc2_app_kit::NSControlTextEditingDelegate;
	use objc2_app_kit::NSEvent;
	use objc2_app_kit::NSEventModifierFlags;
	use objc2_app_kit::NSEventType;
	use objc2_app_kit::NSProgressIndicator;
	use objc2_app_kit::NSProgressIndicatorStyle;
	use objc2_app_kit::NSSecureTextField;
	use objc2_app_kit::NSSlider;
	use objc2_app_kit::NSSwitch;
	use objc2_app_kit::NSTextField;
	use objc2_foundation::NSNotification;
	use objc2_foundation::NSPoint;
	use objc2_foundation::NSString;

	pub fn trials() -> Vec<Trial> {
		[
			(
				"boolean_controls_are_controlled",
				boolean_controls_are_controlled as fn(),
			),
			(
				"accepted_and_deleted_actions_are_reentrant",
				accepted_and_deleted_actions_are_reentrant,
			),
			(
				"slider_proposals_snap_and_remain_controlled",
				slider_proposals_snap_and_remain_controlled,
			),
			(
				"radio_arrows_follow_explicit_groups",
				radio_arrows_follow_explicit_groups,
			),
			(
				"progress_and_activity_have_animation_lifecycles",
				progress_and_activity_have_animation_lifecycles,
			),
			(
				"password_fields_are_secure_and_controlled",
				password_fields_are_secure_and_controlled,
			),
			(
				"focused_field_removal_does_not_emit_inside_commit",
				focused_field_removal_does_not_emit_inside_commit,
			),
			("controls_render_offscreen", controls_render_offscreen),
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

	fn mounted(root: &Element) -> NativeSurface<AppKitBackend> {
		let mtm = MainThreadMarker::new().expect("the harness runs on the main thread");
		NSApplication::sharedApplication(mtm);
		let mut surface = NativeSurface::new(AppKitBackend::new(mtm));
		surface.set_viewport(400.0, 300.0);
		surface.set_root(root);
		surface
	}

	fn child(surface: &NativeSurface<AppKitBackend>, index: usize) -> NodeId {
		surface.children(surface.root().unwrap())[index]
	}

	fn native<T: objc2::DowncastTarget>(
		surface: &NativeSurface<AppKitBackend>,
		id: NodeId,
	) -> Retained<T> {
		surface
			.backend()
			.view(id)
			.unwrap()
			.downcast_ref::<T>()
			.expect("expected native widget")
			.retain()
	}

	fn action(control: &NSControl) {
		// SAFETY: the control's action and target were installed by this
		// backend.
		unsafe {
			control.sendAction_to(control.action(), control.target().as_deref());
		}
	}

	fn click(button: &NSButton) {
		// SAFETY: the native button accepts an optional action sender.
		unsafe {
			button.performClick(None);
		}
	}

	fn boolean_controls_are_controlled() {
		let tree = View().children([
			Switch(false).accessibility_label("Wi-Fi"),
			Checkbox(false).accessibility_label("Accept terms"),
		]);
		let mut surface = mounted(&tree);
		let switch_id = child(&surface, 0);
		let checkbox_id = child(&surface, 1);
		let switch = native::<NSSwitch>(&surface, switch_id);
		let checkbox = native::<NSButton>(&surface, checkbox_id);
		let events = Rc::new(RefCell::new(Vec::new()));
		surface.backend().set_event_listener({
			let events = Rc::clone(&events);
			move |id, event| {
				events.borrow_mut().push((id, event.clone()));
			}
		});

		switch.setState(1);
		action(&switch);
		click(&checkbox);
		assert_eq!(switch.state(), 0, "rejected switch proposal restores state");
		assert_eq!(
			checkbox.state(),
			0,
			"rejected checkbox proposal restores state"
		);
		assert_eq!(
			*events.borrow(),
			vec![
				(switch_id, NativeEvent::Toggle { checked: true }),
				(checkbox_id, NativeEvent::Toggle { checked: true }),
			]
		);
		assert_eq!(switch.accessibilityLabel().unwrap().to_string(), "Wi-Fi");
		assert_eq!(
			checkbox.accessibilityLabel().unwrap().to_string(),
			"Accept terms"
		);

		// Equal props produce no mutation; the controls must already be
		// restored.
		surface.set_root(&tree);
		assert_eq!(switch.state(), 0);
		assert_eq!(checkbox.state(), 0);
		surface.set_root(
			&View().children([Switch(true).enabled(false), Checkbox(true).enabled(false)]),
		);
		assert!(!switch.isEnabled());
		assert!(!checkbox.isEnabled());
		let count = events.borrow().len();
		switch.setState(0);
		action(&switch);
		click(&checkbox);
		assert_eq!(
			events.borrow().len(),
			count,
			"disabled controls propose nothing"
		);
		assert_eq!(switch.state(), 1);
		assert_eq!(checkbox.state(), 1);
	}

	fn accepted_and_deleted_actions_are_reentrant() {
		let surface = Rc::new(RefCell::new(mounted(&View().child(Checkbox(false)))));
		let id = child(&surface.borrow(), 0);
		let button = native::<NSButton>(&surface.borrow(), id);
		surface.borrow().backend().set_event_listener({
			let surface = Rc::downgrade(&surface);
			move |_, event| {
				if let (Some(surface), NativeEvent::Toggle { checked }) = (surface.upgrade(), event)
				{
					surface
						.borrow_mut()
						.set_root(&View().child(Checkbox(*checked)));
				}
			}
		});
		click(&button);
		assert_eq!(
			button.state(),
			1,
			"restoration reads the newly committed state"
		);
		click(&button);
		assert_eq!(button.state(), 0);

		surface.borrow().backend().set_event_listener({
			let surface = Rc::downgrade(&surface);
			move |_, _| {
				if let Some(surface) = surface.upgrade() {
					surface.borrow_mut().set_root(&View());
				}
			}
		});
		click(&button);
		assert!(
			surface.borrow().backend().view(id).is_none(),
			"the firing widget can be deleted safely"
		);
		click(&button);
		assert!(
			surface
				.borrow()
				.children(surface.borrow().root().unwrap())
				.is_empty(),
			"retained deleted controls have inert targets"
		);
	}

	fn slider_proposals_snap_and_remain_controlled() {
		let state = SliderState::new(5.0, 15.0, 7.0, Some(2.0)).unwrap();
		let surface = Rc::new(RefCell::new(mounted(
			&Slider(state).accessibility_label("Volume"),
		)));
		let id = surface.borrow().root().unwrap();
		let slider = native::<NSSlider>(&surface.borrow(), id);
		let events = Rc::new(RefCell::new(Vec::new()));
		surface.borrow().backend().set_event_listener({
			let events = Rc::clone(&events);
			move |_, event| {
				events.borrow_mut().push(event.clone());
			}
		});
		assert_eq!(
			(slider.minValue(), slider.maxValue(), slider.doubleValue()),
			(5.0, 15.0, 7.0)
		);
		assert!(slider.isContinuous());
		slider.setDoubleValue(10.8);
		action(&slider);
		assert_eq!(
			*events.borrow(),
			vec![NativeEvent::ValueChange { value: 11.0 }]
		);
		assert_eq!(slider.doubleValue(), 7.0);
		let window = surface
			.borrow()
			.backend()
			.open_window(id, "Slider control test")
			.unwrap();
		slider.keyDown(&key(&window, "\u{f703}", 124));
		assert_eq!(
			events.borrow().last(),
			Some(&NativeEvent::ValueChange { value: 9.0 }),
			"arrow keys move by the declared step rather than snapping back"
		);
		assert_eq!(slider.doubleValue(), 7.0);

		surface.borrow().backend().set_event_listener({
			let surface = Rc::downgrade(&surface);
			move |_, event| {
				if let (Some(surface), NativeEvent::ValueChange { value }) =
					(surface.upgrade(), event)
				{
					surface.borrow_mut().set_root(&Slider(
						SliderState::new(5.0, 15.0, *value, Some(2.0)).unwrap(),
					));
				}
			}
		});
		slider.setDoubleValue(10.8);
		action(&slider);
		assert_eq!(slider.doubleValue(), 11.0);
		slider.keyDown(&key(&window, "\u{f702}", 123));
		assert_eq!(slider.doubleValue(), 9.0);

		surface.borrow_mut().set_root(&Slider(state).enabled(false));
		slider.setDoubleValue(13.0);
		action(&slider);
		assert_eq!(slider.doubleValue(), 7.0);
		window.close();
	}

	fn radio_arrows_follow_explicit_groups() {
		let group = RadioGroup::new("delivery").unwrap();
		let other = RadioGroup::new("payment").unwrap();
		let tree = |selected: usize| {
			View().children([
				Radio(group.clone(), selected == 0).accessibility_label("Standard"),
				Radio(other.clone(), true).accessibility_label("Card"),
				Radio(group.clone(), false).enabled(false),
				Radio(group.clone(), false).with_props(|props| props.hidden = Some(true)),
				View().child(Radio(group.clone(), selected == 1).accessibility_label("Express")),
			])
		};
		let mut surface = mounted(&tree(0));
		let first_id = child(&surface, 0);
		let other_id = child(&surface, 1);
		let last_id = surface.children(child(&surface, 4))[0];
		let first = native::<NSButton>(&surface, first_id);
		let last = native::<NSButton>(&surface, last_id);
		let card = native::<NSButton>(&surface, other_id);
		let window = surface
			.backend()
			.open_window(surface.root().unwrap(), "Radio control test")
			.unwrap();
		let events = Rc::new(RefCell::new(Vec::new()));
		surface.backend().set_event_listener({
			let events = Rc::clone(&events);
			move |id, event| {
				events.borrow_mut().push((id, event.clone()));
			}
		});
		window.makeFirstResponder(Some(&first));
		let right = key(&window, "\u{f703}", 124);
		first.keyDown(&right);
		assert_eq!(
			*events.borrow(),
			vec![(last_id, NativeEvent::Toggle { checked: true })]
		);
		assert_eq!(
			window.firstResponder().unwrap().downcast_ref::<NSButton>(),
			Some(&*last)
		);
		assert_eq!(
			(first.state(), last.state(), card.state()),
			(1, 0, 1),
			"rejected proposals restore the whole group without changing other groups"
		);
		surface.set_root(&tree(1));
		last.keyDown(&right);
		assert_eq!(
			events.borrow().last(),
			Some(&(first_id, NativeEvent::Toggle { checked: true })),
			"arrows wrap through enabled visible members"
		);
		assert_eq!(
			window.firstResponder().unwrap().downcast_ref::<NSButton>(),
			Some(&*first)
		);
		window.close();
	}

	fn key(window: &objc2_app_kit::NSWindow, character: &str, code: u16) -> Retained<NSEvent> {
		let character = NSString::from_str(character);
		NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
			NSEventType::KeyDown, NSPoint::ZERO, NSEventModifierFlags::empty(), 0.0,
			window.windowNumber(), None, &character, &character, false, code,
		).unwrap()
	}

	fn focused_field_removal_does_not_emit_inside_commit() {
		for field in [TextInput().class("w-48 h-8"), PasswordInput()] {
			let surface = Rc::new(RefCell::new(mounted(
				&View().child(field.value("committed")),
			)));
			let id = child(&surface.borrow(), 0);
			let field = native::<NSTextField>(&surface.borrow(), id);
			let window = surface
				.borrow()
				.backend()
				.open_window(surface.borrow().root().unwrap(), "Editor removal probe")
				.unwrap();
			let events = Rc::new(RefCell::new(Vec::new()));
			surface.borrow().backend().set_event_listener({
				let surface = Rc::downgrade(&surface);
				let events = Rc::clone(&events);
				move |_, event| {
					let borrowed = surface
						.upgrade()
						.is_some_and(|surface| surface.try_borrow_mut().is_err());
					events.borrow_mut().push((event.clone(), borrowed));
				}
			});
			// SAFETY: this is a real editable field mounted in the live test
			// window.
			unsafe {
				field.selectText(None);
			}
			assert!(
				field.currentEditor().is_some(),
				"actual AppKit field editor is active"
			);
			assert!(window.makeFirstResponder(None));
			assert!(
				events
					.borrow()
					.iter()
					.any(|(event, _)| *event == NativeEvent::Blur),
				"baseline editor resignation must emit Blur"
			);
			assert!(events.borrow().iter().all(|(_, borrowed)| !borrowed));
			// SAFETY: the live field may start a second actual editing session.
			unsafe {
				field.selectText(None);
			}
			assert!(field.currentEditor().is_some());
			events.borrow_mut().clear();
			surface.borrow_mut().set_root(&View());
			assert!(
				events.borrow().is_empty(),
				"backend emitted an event during a surface commit: {:?}",
				events.borrow()
			);
			surface
				.borrow_mut()
				.set_root(&View().child(TextInput().class("w-48 h-8")));
			let next = native::<NSTextField>(&surface.borrow(), child(&surface.borrow(), 0));
			// SAFETY: the new live field starts editing in the same test
			// window.
			unsafe {
				next.selectText(None);
			}
			assert!(next.currentEditor().is_some());
			assert!(window.makeFirstResponder(None));
			assert!(
				events
					.borrow()
					.iter()
					.any(|(event, _)| *event == NativeEvent::Blur),
				"callbacks resume after the mutation guard drops"
			);
			assert!(events.borrow().iter().all(|(_, borrowed)| !borrowed));
			window.close();
		}
	}

	fn progress_and_activity_have_animation_lifecycles() {
		let tree = |running, hidden| {
			View()
				.with_props(|props| props.hidden = Some(hidden))
				.children([
					ActivityIndicator(running),
					LinearProgressIndicator(Progress::Indeterminate),
					LinearProgressIndicator(Progress::Determinate(
						ProgressValue::new(0.4).unwrap(),
					)),
				])
		};
		let mut surface = mounted(&tree(true, false));
		let activity_id = child(&surface, 0);
		let progress_id = child(&surface, 1);
		let determinate_id = child(&surface, 2);
		let activity = native::<NSProgressIndicator>(&surface, activity_id);
		let determinate = native::<NSProgressIndicator>(&surface, determinate_id);
		assert_eq!(activity.style(), NSProgressIndicatorStyle::Spinning);
		assert!(activity.isIndeterminate());
		assert!(!activity.isDisplayedWhenStopped());
		assert_eq!(surface.backend().is_animating(activity_id), Some(true));
		assert_eq!(surface.backend().is_animating(progress_id), Some(true));
		assert_eq!(surface.backend().is_animating(determinate_id), Some(false));
		assert!(!determinate.isIndeterminate());
		assert_eq!(determinate.doubleValue(), 0.4);
		surface.set_root(&tree(true, true));
		assert_eq!(surface.backend().is_animating(activity_id), Some(false));
		assert_eq!(surface.backend().is_animating(progress_id), Some(false));
		surface.set_root(&tree(false, false));
		assert_eq!(surface.backend().is_animating(activity_id), Some(false));
		assert_eq!(surface.backend().is_animating(progress_id), Some(true));
		assert!(
			activity.isHidden(),
			"stopped activity indicators are not static glyphs"
		);
		surface.set_root(&tree(true, false));
		assert_eq!(surface.backend().is_animating(activity_id), Some(true));
		surface.set_root(&View());
		assert_eq!(surface.backend().is_animating(activity_id), None);
		// SAFETY: the retained indicator remains valid after its node is
		// deleted.
		assert!(unsafe { activity.superview() }.is_none());
	}

	fn password_fields_are_secure_and_controlled() {
		let surface = Rc::new(RefCell::new(mounted(
			&PasswordInput()
				.value("committed")
				.placeholder("Password")
				.accessibility_label("Account password"),
		)));
		let id = surface.borrow().root().unwrap();
		let secure = native::<NSSecureTextField>(&surface.borrow(), id);
		assert_eq!(secure.stringValue().to_string(), "committed");
		assert_eq!(
			secure.accessibilityLabel().unwrap().to_string(),
			"Account password"
		);
		let events = Rc::new(RefCell::new(Vec::new()));
		surface.borrow().backend().set_event_listener({
			let events = Rc::clone(&events);
			move |_, event| {
				events.borrow_mut().push(event.clone());
			}
		});
		edit(&secure, "rejected");
		assert_eq!(
			*events.borrow(),
			vec![NativeEvent::Change {
				value: "rejected".to_owned()
			}]
		);
		assert_eq!(secure.stringValue().to_string(), "committed");

		surface.borrow().backend().set_event_listener({
			let surface = Rc::downgrade(&surface);
			move |_, event| {
				if let (Some(surface), NativeEvent::Change { value }) = (surface.upgrade(), event) {
					surface.borrow_mut().set_root(&PasswordInput().value(value));
				}
			}
		});
		edit(&secure, "accepted");
		assert_eq!(secure.stringValue().to_string(), "accepted");

		surface.borrow().backend().set_event_listener({
			let surface = Rc::downgrade(&surface);
			move |_, _| {
				if let Some(surface) = surface.upgrade() {
					surface.borrow_mut().set_root(&View());
				}
			}
		});
		edit(&secure, "delete while editing");
		assert!(surface.borrow().backend().view(id).is_none());
	}

	fn controls_render_offscreen() {
		let group = RadioGroup::new("preview").unwrap();
		let row = |label: &str, control: Element| {
			View()
				.class("flex-row items-center gap-3 h-8")
				.child(Text(label).class("w-32 text-sm"))
				.child(control.accessibility_label(label))
		};
		let tree = View()
			.class("w-full h-full bg-slate-900 text-white p-4 gap-1")
			.children([
				row("Wi-Fi", Switch(true)),
				row("Remember me", Checkbox(true)),
				row("Standard", Radio(group.clone(), true)),
				row("Express", Radio(group, false)),
				row(
					"Volume",
					Slider(SliderState::new(0.0, 100.0, 60.0, Some(5.0)).unwrap()),
				),
				row(
					"Downloading",
					LinearProgressIndicator(Progress::Determinate(
						ProgressValue::new(0.6).unwrap(),
					)),
				),
				row("Loading", ActivityIndicator(true)),
				row(
					"Password",
					PasswordInput().value("example only").class("w-44 h-7"),
				),
			]);
		let surface = mounted(&tree);
		let png = surface
			.backend()
			.snapshot_png(surface.root().unwrap(), 2.0)
			.unwrap();
		assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));

		if let Some(directory) = std::env::var_os("LEPTOSKIT_TEST_ARTIFACTS") {
			std::fs::create_dir_all(&directory).unwrap();
			std::fs::write(
				std::path::PathBuf::from(directory).join("native-controls.png"),
				png,
			)
			.unwrap();
		}
	}

	fn edit(field: &NSTextField, value: &str) {
		field.setStringValue(&NSString::from_str(value));
		// SAFETY: the notification retains this actual field as its object;
		// the installed delegate implements the native edit notification.
		let notification = unsafe {
			NSNotification::notificationWithName_object(
				&NSString::from_str("NSControlTextDidChangeNotification"),
				Some(field),
			)
		};
		field
			.delegate()
			.unwrap()
			.controlTextDidChange(&notification);
	}
}
