//! Executable UIKit verification inside a real simulator application bundle.
#![allow(unsafe_code)]

#[cfg(not(target_os = "ios"))]
fn main() {}

#[cfg(target_os = "ios")]
fn main() {
	probe::run();
}

#[cfg(target_os = "ios")]
mod probe {
	use std::cell::RefCell;
	use std::rc::Rc;
	use std::sync::Arc;
	use std::sync::atomic::AtomicBool;
	use std::sync::atomic::Ordering;

	use leptoskit_native::Element;
	use leptoskit_native::FontWeight;
	use leptoskit_native::NativeBackend;
	use leptoskit_native::NativeEvent;
	use leptoskit_native::NativeKind;
	use leptoskit_native::NativeProps;
	use leptoskit_native::NativeSurface;
	use leptoskit_native::Progress;
	use leptoskit_native::ProgressValue;
	use leptoskit_native::RadioGroup;
	use leptoskit_native::SliderState;
	use leptoskit_native::TextAlign;
	use leptoskit_native::TextMeasurer;
	use leptoskit_native::TextStyle;
	use leptoskit_uikit::MainThreadMarker;
	use leptoskit_uikit::UIKitBackend;
	use leptoskit_uikit::UIKitHost;
	use leptoskit_uikit::UIKitTextMeasurer;
	use leptoskit_uikit::run_application;
	use leptoskit_yoga::AvailableSpace;
	use leptoskit_yoga::NodeId;
	use leptoskit_yoga::Size;
	use objc2::DefinedClass;
	use objc2::MainThreadOnly;
	use objc2::Message;
	use objc2::define_class;
	use objc2::msg_send;
	use objc2::rc::Retained;
	use objc2::runtime::NSObject;
	use objc2::runtime::NSObjectProtocol;
	use objc2::sel;
	use objc2_core_foundation::CGPoint;
	use objc2_core_foundation::CGSize;
	use objc2_foundation::NSHomeDirectory;
	use objc2_foundation::NSNumber;
	use objc2_foundation::NSObjectNSDelayedPerforming;
	use objc2_foundation::NSObjectNSKeyValueCoding;
	use objc2_foundation::NSString;
	use objc2_foundation::NSURL;
	use objc2_ui_kit::NSObjectUIAccessibility;
	use objc2_ui_kit::NSObjectUIAccessibilityAction;
	use objc2_ui_kit::UIActivityIndicatorView;
	use objc2_ui_kit::UIApplication;
	use objc2_ui_kit::UIButton;
	use objc2_ui_kit::UIControlEvents;
	use objc2_ui_kit::UICoordinateSpace;
	use objc2_ui_kit::UIImageView;
	use objc2_ui_kit::UIKeyInput;
	use objc2_ui_kit::UILabel;
	use objc2_ui_kit::UIProgressView;
	use objc2_ui_kit::UIScrollView;
	use objc2_ui_kit::UISlider;
	use objc2_ui_kit::UISwitch;
	use objc2_ui_kit::UITextField;
	use objc2_ui_kit::UIUserInterfaceStyle;
	use objc2_ui_kit::UIView;
	use objc2_ui_kit::UIWindow;

	type Surface = Rc<RefCell<NativeSurface<UIKitBackend>>>;
	struct ProbeState {
		surface: Surface,
		window: Retained<UIWindow>,
	}
	thread_local! { static PROBE: RefCell<Option<Retained<Probe>>> = const { RefCell::new(None) }; }

	define_class!(
		// SAFETY: NSObject owns its ivars; the run loop retains a delayed target.
		#[unsafe(super(NSObject))]
		#[thread_kind = MainThreadOnly]
		#[ivars = ProbeState]
		struct Probe;
		impl Probe {
			#[unsafe(method(verify:))]
			fn verify(&self, _sender: Option<&NSObject>) {
				let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| verify(&self.ivars().surface, &self.ivars().window)));
				let report = match result {
					Ok(lines) => format!("VERIFIED\tUIKit native renderer\n{}\n", lines.join("\n")),
					Err(error) => format!("FAILED\t{}\n", error.downcast_ref::<String>().map(String::as_str).or_else(|| error.downcast_ref::<&str>().copied()).unwrap_or("native assertion panic")),
				};
				// The verifier stops the simulator once the report exists, so the
				// coverage profile must be complete before it is written.
				#[cfg(feature = "coverage")]
				super::flush_coverage();
				let path = format!("{}/Documents/native-report.tsv", NSHomeDirectory());
				std::fs::write(&path, &report).expect("write native verification report");
				eprintln!("{report}");
			}
		}
		unsafe impl NSObjectProtocol for Probe {}
	);

	pub fn run() {
		let mtm = MainThreadMarker::new().expect("UIKit application main thread");
		run_application(mtm, |mtm, scene| {
			let surface = Rc::new(RefCell::new(NativeSurface::new(UIKitBackend::new(mtm))));
			// SAFETY: UIWindowScene's coordinate space is valid after
			// connection.
			#[allow(deprecated)]
			let size = scene.coordinateSpace().bounds().size;
			surface
				.borrow_mut()
				.set_viewport(size.width as f32, size.height as f32);
			surface.borrow_mut().set_dark_mode(false);
			surface.borrow_mut().set_root(&screen());
			let root = surface.borrow().root().expect("mounted root");
			let window = surface
				.borrow()
				.backend()
				.open_window(root, scene)
				.expect("live root window");
			window.makeKeyAndVisible();
			let this = Probe::alloc(mtm).set_ivars(ProbeState {
				surface,
				window: window.clone(),
			});
			// SAFETY: NSObject's designated initializer initializes this
			// target.
			let probe: Retained<Probe> = unsafe { msg_send![super(this), init] };
			// SAFETY: verify: accepts the absent NSObject sender. The run loop
			// retains the target until the app has displayed its first frame.
			unsafe {
				probe.performSelector_withObject_afterDelay(sel!(verify:), None, 0.8);
			}
			PROBE.with(|slot| slot.replace(Some(probe)));
			window
		});
	}

	fn named(element: Element, name: &str) -> Element {
		element.with_props(|props| props.accessibility_label = Some(name.to_owned()))
	}

	fn screen() -> Element {
		let group = RadioGroup::new("density").expect("fixture group");
		Element::view()
			.class("w-full h-full flex-col gap-3 p-6 pt-16 bg-slate-50 text-slate-900")
			.children([
				named(
					Element::text("Native, all the way down").class("text-2xl font-bold h-9"),
					"title",
				),
				Element::text("Shared primitives. Real UIKit controls. One Yoga tree.")
					.class("text-sm text-slate-500 h-10"),
				named(
					Element::button("Continue with UIKit")
						.class("h-11 w-full rounded-xl bg-indigo-600 text-white"),
					"button",
				),
				Element::view()
					.class("flex-row items-center gap-4 h-10")
					.children([
						named(
							Element::new(NativeKind::Switch)
								.with_props(|props| props.checked = Some(false))
								.class("w-16 h-9 text-indigo-600"),
							"switch",
						),
						named(
							Element::new(NativeKind::Checkbox)
								.with_props(|props| props.checked = Some(false))
								.class("w-10 h-10 text-indigo-600"),
							"checkbox",
						),
						named(
							Element::new(NativeKind::Radio)
								.with_props(|props| {
									props.radio_group = Some(group.clone());
									props.checked = Some(true);
								})
								.class("w-10 h-10 text-indigo-600"),
							"radio-a",
						),
						named(
							Element::new(NativeKind::Radio)
								.with_props(|props| {
									props.radio_group = Some(group);
									props.checked = Some(false);
								})
								.class("w-10 h-10 text-indigo-600"),
							"radio-b",
						),
					]),
				named(
					Element::new(NativeKind::Slider)
						.with_props(|props| {
							props.slider = Some(
								SliderState::new(0.0, 100.0, 40.0, Some(5.0))
									.expect("fixture range"),
							);
						})
						.class("w-full h-8 text-indigo-600"),
					"slider",
				),
				named(
					Element::text_input()
						.with_props(|props| {
							props.value = Some("Shared editable text".into());
							props.placeholder = Some("Your name".into());
						})
						.class("h-11 border border-slate-300 rounded-lg bg-white"),
					"input",
				),
				named(
					Element::new(NativeKind::PasswordInput)
						.with_props(|props| {
							props.value = Some("dummy-fixture".into());
							props.placeholder = Some("Secure text".into());
						})
						.class("h-11 border border-slate-300 rounded-lg bg-white"),
					"password",
				),
				named(
					Element::new(NativeKind::LinearProgressIndicator)
						.with_props(|props| {
							props.progress = Some(Progress::Determinate(
								ProgressValue::new(0.64).expect("fixture progress"),
							));
						})
						.class("w-full h-2 bg-slate-200 text-indigo-600 rounded-full"),
					"progress",
				),
				named(
					Element::new(NativeKind::LinearProgressIndicator)
						.class("w-full h-2 bg-slate-200 text-indigo-600 rounded-full"),
					"loading",
				),
				named(
					Element::new(NativeKind::ActivityIndicator)
						.with_props(|props| props.running = Some(true))
						.class("w-8 h-8 text-indigo-600"),
					"activity",
				),
				named(
					Element::new(NativeKind::Pressable)
						.class("h-14 rounded-xl bg-slate-900 items-center justify-center")
						.child(
							Element::text("A composed pressable").class("text-white font-semibold"),
						),
					"pressable",
				),
				named(
					Element::scroll_view()
						.class("h-32 w-full bg-white border border-slate-200 rounded-xl")
						.children((0..12).map(|index| {
							Element::text(format!("Native scrolling row {}", index + 1))
								.class("h-8 px-3 text-sm")
						})),
					"scroll",
				),
				named(
					Element::image("resource://probe-image").class("w-16 h-8"),
					"image",
				),
				named(
					Element::new(NativeKind::TextRun)
						.with_props(|props| {
							props.text = Some("UIKit renderer verified on device".into());
						})
						.class("text-xs text-slate-500 h-5"),
					"run",
				),
			])
	}

	fn id(surface: &NativeSurface<UIKitBackend>, name: &str) -> NodeId {
		let mut nodes = vec![surface.root().expect("mounted tree")];
		while let Some(node) = nodes.pop() {
			if surface.props(node).accessibility_label.as_deref() == Some(name) {
				return node;
			}
			nodes.extend_from_slice(surface.children(node));
		}
		panic!("missing fixture node {name}")
	}

	fn native<T: objc2::DowncastTarget>(surface: &Surface, name: &str) -> Retained<T> {
		let surface = surface.borrow();
		let mut views = vec![
			surface
				.backend()
				.view(id(&surface, name))
				.expect("native view")
				.retain(),
		];
		while let Some(view) = views.pop() {
			if let Some(view) = view.downcast_ref::<T>() {
				return view.retain();
			}
			views.extend(view.subviews().iter());
		}
		panic!("concrete UIKit class for {name}")
	}

	fn modify(surface: &Surface, name: &str, change: impl FnOnce(&mut NativeProps)) {
		let node = id(&surface.borrow(), name);
		let mut props = surface.borrow().props(node).clone();
		change(&mut props);
		surface
			.borrow_mut()
			.backend_mut()
			.update_props(node, &props);
	}

	fn verify(surface: &Surface, window: &UIWindow) -> Vec<String> {
		let mtm = MainThreadMarker::new().expect("probe main thread");
		multiline_metrics(mtm);
		let events = Rc::new(RefCell::new(Vec::<(NodeId, NativeEvent)>::new()));
		let record = Rc::clone(&events);
		surface
			.borrow_mut()
			.backend_mut()
			.set_event_listener(Some(Rc::new(move |node, event| {
				record.borrow_mut().push((node, event.clone()));
			})));
		let mut passed = Vec::new();
		passed.push("PASS\twrapped gallery footnotes match native UILabel height".into());
		assert!(window.isKeyWindow());
		let label = native::<UILabel>(surface, "title");
		let input = native::<UITextField>(surface, "input");
		let password = native::<UITextField>(surface, "password");
		let switch = native::<UISwitch>(surface, "switch");
		let checkbox = native::<UIButton>(surface, "checkbox");
		let radio_a = native::<UIButton>(surface, "radio-a");
		let radio_b = native::<UIButton>(surface, "radio-b");
		let slider = native::<UISlider>(surface, "slider");
		let button = native::<UIButton>(surface, "button");
		let pressable = native::<UIButton>(surface, "pressable");
		let scroll = native::<UIScrollView>(surface, "scroll");
		let image = native::<UIImageView>(surface, "image");
		let activity = native::<UIActivityIndicatorView>(surface, "activity");
		let _text_run = native::<UILabel>(surface, "run");
		assert!(label.window().is_some());
		assert!(image.image().is_some(), "packaged resource loads");
		modify(surface, "image", |props| {
			props.source = Some("resource://probe-image.bmp".into());
		});
		assert!(image.image().is_some(), "gallery BMP decodes in UIKit");
		let local_image = format!("{}/Documents/a local image.png", NSHomeDirectory());
		std::fs::write(&local_image, include_bytes!("../probe/probe-image.png"))
			.expect("local image fixture");
		modify(surface, "image", |props| {
			props.source = Some(local_image.clone());
		});
		assert!(image.image().is_some(), "plain path with spaces loads");
		let file_url = NSURL::fileURLWithPath(&NSString::from_str(&local_image))
			.absoluteString()
			.expect("file URL")
			.to_string();
		assert!(file_url.contains("%20"));
		modify(surface, "image", |props| props.source = Some(file_url));
		assert!(image.image().is_some(), "encoded file URL loads");
		modify(surface, "image", |props| {
			props.source = Some("https://invalid.example/image.png".into());
		});
		assert!(
			image.image().is_none(),
			"unsupported network sources clear the image"
		);
		modify(surface, "image", |props| {
			props.source = Some("resource://probe-image".into());
		});
		assert!(scroll.contentSize().height > scroll.bounds().size.height);
		let progress = native::<UIView>(surface, "progress");
		assert!(
			progress
				.subviews()
				.objectAtIndex(0)
				.downcast_ref::<UIProgressView>()
				.is_some()
		);
		assert!(
			progress.bounds().size.height > 0.0,
			"progress outer frame {:?}",
			progress.frame()
		);
		assert!(
			(progress.layer().cornerRadius() - 4.0).abs() < 0.01,
			"full rounding clamps to the border box"
		);
		passed.push("PASS\tall 15 built-in kinds mount real UIKit views".into());

		let root = surface.borrow().root().expect("mounted root");
		let mut children = surface.borrow().children(root).to_vec();
		while let Some(child) = children.pop() {
			let surface = surface.borrow();
			children.extend_from_slice(surface.children(child));
			let layout = surface.layout(child);
			let frame = surface.backend().view(child).expect("native child").frame();
			assert!((frame.origin.x - f64::from(layout.x)).abs() < 0.01);
			assert!((frame.origin.y - f64::from(layout.y)).abs() < 0.01);
			assert!((frame.size.width - f64::from(layout.width)).abs() < 0.01);
			assert!((frame.size.height - f64::from(layout.height)).abs() < 0.01);
		}
		passed.push("PASS\tUIKit border boxes match Yoga allocations".into());

		let style = TextStyle::new(24.0, FontWeight::Bold);
		let measured = UIKitTextMeasurer::new().measure(
			"Native, all the way down",
			style,
			Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
		);
		let intrinsic = label.sizeThatFits(CGSize::new(1.0e7, 1.0e7));
		assert!(
			(f64::from(measured.width) - intrinsic.width).abs() <= 1.0,
			"Core Text width {measured:?} UIKit {intrinsic:?}"
		);
		assert!(
			(f64::from(measured.height) - intrinsic.height).abs() <= 1.0,
			"Core Text height {measured:?} UIKit {intrinsic:?}"
		);
		let wrapped = UIKitTextMeasurer::new().measure(
			"one two three four five",
			TextStyle::default(),
			Size::new(AvailableSpace::Definite(45.0), AvailableSpace::MaxContent),
		);
		assert!(wrapped.height > 30.0);
		assert!(
			!label.isOpaque(),
			"unpainted labels must remain transparent"
		);
		modify(surface, "title", |props| {
			props.text_align = Some(TextAlign::End);
			props.direction = Some(leptoskit_yoga::Direction::Rtl);
		});
		assert_eq!(label.textAlignment(), objc2_ui_kit::NSTextAlignment::Left);
		modify(surface, "title", |props| {
			props.text_align = Some(TextAlign::End);
			props.direction = Some(leptoskit_yoga::Direction::Ltr);
		});
		assert_eq!(label.textAlignment(), objc2_ui_kit::NSTextAlignment::Right);
		modify(surface, "title", |props| {
			props.text_align = Some(TextAlign::Start);
		});
		let measured_thread = std::thread::spawn(move || {
			UIKitTextMeasurer::new().measure(
				"thread safe",
				style,
				Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
			)
		})
		.join()
		.expect("Core Text background operation");
		assert!(measured_thread.width > 0.0);
		passed.push("PASS\tCore Text metrics match UILabel and measure off UI thread".into());

		switch.setOn(true);
		switch.sendActionsForControlEvents(UIControlEvents::ValueChanged);
		assert!(
			!switch.isOn(),
			"rejected switch proposal restores committed false"
		);
		checkbox.sendActionsForControlEvents(UIControlEvents::PrimaryActionTriggered);
		assert!(!checkbox.isSelected());
		radio_b.sendActionsForControlEvents(UIControlEvents::PrimaryActionTriggered);
		assert!(radio_a.isSelected());
		assert!(!radio_b.isSelected());
		assert!(
			events
				.borrow()
				.iter()
				.any(|(_, event)| matches!(event, NativeEvent::Toggle { checked: true }))
		);
		passed.push("PASS\tboolean controls reject proposals without uncontrolled drift".into());
		let key = radio_a
			.keyCommands()
			.expect("radio keyboard commands")
			.objectAtIndex(0);
		let before = events.borrow().len();
		// SAFETY: nextRadio: is installed on this live native radio button and
		// receives its UIKeyCommand through UIApplication's responder action
		// API.
		assert!(unsafe {
			UIApplication::sharedApplication(mtm).sendAction_to_from_forEvent(
				sel!(nextRadio:),
				Some(&radio_a),
				Some(&key),
				None,
			)
		});
		let radio_b_id = id(&surface.borrow(), "radio-b");
		assert!(events.borrow()[before..].iter().any(|(node, event)| {
			*node == radio_b_id && matches!(event, NativeEvent::Toggle { checked: true })
		}));
		modify(surface, "radio-b", |props| props.hidden = Some(true));
		let before = events.borrow().len();
		assert!(unsafe {
			UIApplication::sharedApplication(mtm).sendAction_to_from_forEvent(
				sel!(nextRadio:),
				Some(&radio_a),
				Some(&key),
				None,
			)
		});
		assert!(
			!events.borrow()[before..]
				.iter()
				.any(|(node, _)| *node == radio_b_id)
		);
		modify(surface, "radio-b", |props| props.hidden = Some(false));
		passed.push(
			"PASS\tradio keyboard commands follow explicit groups and skip hidden members".into(),
		);

		let weak = Rc::downgrade(surface);
		surface
			.borrow_mut()
			.backend_mut()
			.set_event_listener(Some(Rc::new(move |node, event| {
				let surface = weak.upgrade().expect("live probe surface");
				let mut props = surface.borrow().props(node).clone();
				if let NativeEvent::Toggle { checked } = event {
					props.checked = Some(*checked);
				}
				surface
					.borrow_mut()
					.backend_mut()
					.update_props(node, &props);
			})));
		switch.setOn(true);
		switch.sendActionsForControlEvents(UIControlEvents::ValueChanged);
		assert!(switch.isOn(), "accepted update is read after callback");
		assert!(std::ptr::eq(
			&raw const *switch,
			&raw const *native::<UISwitch>(surface, "switch")
		));
		passed.push("PASS\taccepted synchronous controls retain their native identity".into());
		let record = Rc::clone(&events);
		surface
			.borrow_mut()
			.backend_mut()
			.set_event_listener(Some(Rc::new(move |node, event| {
				record.borrow_mut().push((node, event.clone()));
			})));

		slider.setValue(0.47);
		slider.sendActionsForControlEvents(UIControlEvents::ValueChanged);
		assert!((slider.value() - 0.4).abs() < 0.0001);
		assert!(events.borrow().iter().any(|(_, event)| matches!(event, NativeEvent::ValueChange { value } if (*value - 45.0).abs() < 0.0001)));
		let extreme =
			SliderState::new(0.0, 1.0e200, 0.0, Some(1.0e199)).expect("shared finite large grid");
		modify(surface, "slider", |props| props.slider = Some(extreme));
		slider.setValue(0.75);
		slider.sendActionsForControlEvents(UIControlEvents::ValueChanged);
		let expected = extreme
			.proposed_value(extreme.max() * 0.75)
			.expect("shared canonical proposal");
		assert!(events.borrow().iter().any(|(_, event)| matches!(event, NativeEvent::ValueChange { value } if (*value - expected).abs() / expected.abs() < 1.0e-12)));
		assert!((slider.value() - 0.0).abs() < 0.0001);
		let tiny =
			SliderState::new(0.0, 1.0e-200, 0.0, Some(1.0e-201)).expect("shared finite tiny grid");
		modify(surface, "slider", |props| props.slider = Some(tiny));
		slider.accessibilityIncrement(mtm);
		assert!(events.borrow().iter().any(|(_, event)| matches!(event, NativeEvent::ValueChange { value } if *value > 0.0 && (*value / 1.0e-201 - 1.0).abs() < 0.001)));
		for state in [
			SliderState::new(1.0, 1.0f64.next_up(), 1.0, None).expect("adjacent continuous range"),
			SliderState::new(0.0, f64::from_bits(2), 0.0, None)
				.expect("subnormal continuous range"),
		] {
			modify(surface, "slider", |props| props.slider = Some(state));
			let before = events.borrow().len();
			slider.accessibilityIncrement(mtm);
			assert!(events.borrow()[before..].iter().any(|(_, event)| matches!(event, NativeEvent::ValueChange { value } if *value > state.min() && *value <= state.max())));
		}
		passed.push(
			"PASS\tf64 slider grids survive normalized UIKit coordinates and accessibility steps"
				.into(),
		);

		// UIKit forwards this optional UITextInputTraits property. KVC observes
		// the real public property without objc2's direct-method introspection.
		// SAFETY: secureTextEntry is UITextField's documented boolean property.
		let secure = password
			.valueForKey(&NSString::from_str("secureTextEntry"))
			.expect("secure property");
		assert!(
			secure
				.downcast_ref::<NSNumber>()
				.expect("boolean property")
				.boolValue()
		);
		let before = events.borrow().len();
		password.setText(Some(&NSString::from_str("dummy-proposal")));
		password.sendActionsForControlEvents(UIControlEvents::EditingChanged);
		assert_eq!(
			password.text().expect("secure value").to_string(),
			"dummy-fixture"
		);
		assert!(events.borrow()[before..].iter().any(
			|(_, event)| matches!(event, NativeEvent::Change { value } if value == "dummy-proposal")
		));
		input.setText(Some(&NSString::from_str("proposed")));
		input.sendActionsForControlEvents(UIControlEvents::EditingChanged);
		assert_eq!(
			input.text().expect("controlled text").to_string(),
			"Shared editable text"
		);
		modify(surface, "input", |props| {
			props.value = None;
			props.text = Some("text-only initial".into());
		});
		input.setText(Some(&NSString::from_str("rejected text-only edit")));
		input.sendActionsForControlEvents(UIControlEvents::EditingChanged);
		assert_eq!(
			input.text().expect("text fallback").to_string(),
			"text-only initial"
		);
		modify(surface, "input", |props| {
			props.value = Some("Shared editable text".into());
		});
		assert!(input.becomeFirstResponder());
		assert!(input.isFirstResponder());
		let before = events.borrow().len();
		input.insertText(&NSString::from_str("x"));
		assert!(
			events.borrow()[before..]
				.iter()
				.any(|(_, event)| matches!(event, NativeEvent::Change { .. })),
			"native key input emits editing changes"
		);
		assert_eq!(
			input.text().expect("rejected native key input").to_string(),
			"Shared editable text"
		);
		let delegate = input.delegate().expect("native text delegate");
		// SAFETY: The delegate conforms to UITextFieldDelegate and this field
		// is live.
		unsafe {
			let _: bool = msg_send![&*delegate, textFieldShouldReturn: &*input];
		}
		assert!(
			events
				.borrow()
				.iter()
				.any(|(_, event)| matches!(event, NativeEvent::Focus))
		);
		assert!(events.borrow().iter().any(
			|(_, event)| matches!(event, NativeEvent::Submit { value } if value == "Shared editable text")
		));
		input.resignFirstResponder();
		assert!(
			events
				.borrow()
				.iter()
				.any(|(_, event)| matches!(event, NativeEvent::Blur))
		);
		passed.push("PASS\tsecure controlled editor emits input submit focus and blur".into());

		let before = events.borrow().len();
		modify(surface, "button", |props| props.enabled = Some(false));
		button.sendActionsForControlEvents(UIControlEvents::PrimaryActionTriggered);
		assert_eq!(events.borrow().len(), before);
		modify(surface, "button", |props| {
			props.enabled = Some(true);
			props.hidden = Some(true);
		});
		button.sendActionsForControlEvents(UIControlEvents::PrimaryActionTriggered);
		assert_eq!(events.borrow().len(), before);
		modify(surface, "button", |props| props.hidden = Some(false));
		button.sendActionsForControlEvents(UIControlEvents::PrimaryActionTriggered);
		assert_eq!(events.borrow().len(), before + 1);
		let root_props = surface.borrow().props(root).clone();
		let mut disabled = root_props.clone();
		disabled.enabled = Some(false);
		surface
			.borrow_mut()
			.backend_mut()
			.update_props(root, &disabled);
		assert!(!button.isEnabled());
		assert!(!switch.isEnabled());
		assert!(!input.isEnabled());
		assert!(
			!input.becomeFirstResponder(),
			"disabled ancestor blocks editor focus"
		);
		button.sendActionsForControlEvents(UIControlEvents::PrimaryActionTriggered);
		assert_eq!(events.borrow().len(), before + 1);
		surface
			.borrow_mut()
			.backend_mut()
			.update_props(root, &root_props);
		assert!(button.isEnabled());
		assert!(switch.isEnabled());
		assert!(input.isEnabled());
		passed.push("PASS\tdisabled and hidden native controls do not dispatch".into());

		let before = events.borrow().len();
		scroll.setContentOffset(CGPoint::new(0.0, 60.0));
		assert!(scroll.contentOffset().y > 0.0);
		assert!(events.borrow()[before..].iter().any(
			|(_, event)| matches!(event, NativeEvent::Scroll { offset_y, .. } if *offset_y > 0.0)
		));
		assert_eq!(
			button
				.accessibilityLabel(mtm)
				.expect("button name")
				.to_string(),
			"button"
		);
		assert_eq!(
			checkbox
				.accessibilityValue(mtm)
				.expect("boolean value")
				.to_string(),
			"0"
		);
		assert!(pressable.accessibilityActivate(mtm));
		let hit = pressable
			.hitTest_withEvent(CGPoint::new(50.0, 20.0), None)
			.expect("pressable hit");
		assert!(std::ptr::eq(&raw const *hit, &*pressable as &UIView));
		assert!(
			pressable
				.hitTest_withEvent(CGPoint::new(-1.0, 20.0), None)
				.is_none()
		);
		passed.push(
			"PASS\tnative scrolling accessibility activation and composed hit testing".into(),
		);

		assert!(activity.isAnimating());
		let loading = native::<UIView>(surface, "loading");
		let fill = loading.subviews().objectAtIndex(1);
		assert!(fill.layer().animationKeys().is_some());
		// SAFETY: Core Animation's presentation layer is a live read-only copy.
		let presentation =
			unsafe { fill.layer().presentationLayer() }.expect("native presentation animation");
		assert!(
			presentation.transform().m41.abs() > 0.1,
			"indeterminate segment actually advances on Core Animation's timeline"
		);
		modify(surface, "activity", |props| props.running = Some(false));
		assert!(!activity.isAnimating());
		assert!(activity.isHidden());
		assert!(activity.accessibilityElementsHidden(mtm));
		modify(surface, "activity", |props| props.running = Some(true));
		assert!(activity.isAnimating());
		assert!(!activity.isHidden());
		let mut hidden = root_props.clone();
		hidden.hidden = Some(true);
		surface
			.borrow_mut()
			.backend_mut()
			.update_props(root, &hidden);
		assert!(!activity.isAnimating());
		assert!(fill.layer().animationKeys().is_none());
		surface
			.borrow_mut()
			.backend_mut()
			.update_props(root, &root_props);
		assert!(activity.isAnimating());
		assert!(fill.layer().animationKeys().is_some());
		modify(surface, "loading", |props| props.hidden = Some(true));
		assert!(fill.layer().animationKeys().is_none());
		modify(surface, "loading", |props| props.hidden = Some(false));
		assert!(fill.layer().animationKeys().is_some());
		passed.push("PASS\tindeterminate bar and spinner run and stop native animations".into());

		surface.borrow_mut().set_dark_mode(true);
		assert_eq!(
			native::<UIView>(surface, "switch").overrideUserInterfaceStyle(),
			UIUserInterfaceStyle::Dark
		);
		surface.borrow_mut().set_dark_mode(false);
		assert_eq!(
			native::<UIView>(surface, "switch").overrideUserInterfaceStyle(),
			UIUserInterfaceStyle::Light
		);
		passed.push("PASS\texplicit appearance updates existing UIKit controls".into());

		let weak = Rc::downgrade(surface);
		surface
			.borrow_mut()
			.backend_mut()
			.set_event_listener(Some(Rc::new(move |_, _| {
				let surface = weak.upgrade().expect("live surface");
				surface
					.borrow_mut()
					.set_root(&Element::text("Rebuilt during native callback"));
			})));
		switch.setOn(false);
		switch.sendActionsForControlEvents(UIControlEvents::ValueChanged);
		assert!(switch.superview().is_none());
		switch.sendActionsForControlEvents(UIControlEvents::ValueChanged);
		assert!(!activity.isAnimating());
		assert!(fill.layer().animationKeys().is_none());
		passed.push("PASS\treentrant removal invalidates retained actions and stops timers".into());

		surface.borrow_mut().backend_mut().set_event_listener(None);
		surface.borrow_mut().set_root(&screen());
		let root = surface.borrow().root().expect("restored fixture");
		let controller = window.rootViewController().expect("application controller");
		assert!(surface.borrow().backend().mount(root, &controller));
		window.layoutIfNeeded();
		assert!(
			surface
				.borrow()
				.backend()
				.view(root)
				.expect("restored native root")
				.window()
				.is_some()
		);
		host_lifecycle(window, mtm);
		passed.push(
			"PASS\tretained host preserves Yoga root frames and handles replacement and resize"
				.into(),
		);
		reentrant_lifecycle(window, mtm);
		passed.push(
			"PASS\tnative accessibility editor and scroll callbacks tolerate synchronous deletion"
				.into(),
		);
		assert!(surface.borrow().backend().mount(root, &controller));
		window.makeKeyAndVisible();
		passed
	}

	fn host_lifecycle(window: &UIWindow, mtm: MainThreadMarker) {
		let changed = Arc::new(AtomicBool::new(false));
		let render_state = Arc::clone(&changed);
		let host = UIKitHost::new(mtm, move || {
			if render_state.load(Ordering::SeqCst) {
				Element::text("Replaced native root").class("w-full h-8")
			} else {
				let changed = Arc::clone(&render_state);
				Element::button("Replace")
					.class("w-20 h-10")
					.on_press(move || {
						changed.store(true, Ordering::SeqCst);
					})
			}
		});
		host.set_dark_mode(false);
		let test_window = host.open_window(&window.windowScene().expect("connected scene"));
		test_window.makeKeyAndVisible();
		let controller = test_window
			.rootViewController()
			.expect("retained surface controller");
		controller.viewDidLayoutSubviews();
		let container = controller.view().expect("stable host view");
		let button_view = container.subviews().objectAtIndex(0);
		let button = button_view
			.downcast_ref::<UIButton>()
			.expect("initial button root");
		assert!((button.frame().size.width - 80.0).abs() < 0.01);
		assert!((button.frame().size.height - 40.0).abs() < 0.01);
		drop(host);
		button.sendActionsForControlEvents(UIControlEvents::PrimaryActionTriggered);
		assert!(
			changed.load(Ordering::SeqCst),
			"controller retains host after Rust handle drops"
		);
		assert!(button.superview().is_none());
		let replaced = container.subviews().objectAtIndex(0);
		assert_eq!(
			replaced
				.downcast_ref::<UILabel>()
				.expect("replacement label")
				.text()
				.expect("label text")
				.to_string(),
			"Replaced native root"
		);
		assert!(replaced.window().is_some());
		container.setFrame(objc2_core_foundation::CGRect::new(
			CGPoint::ZERO,
			CGSize::new(300.0, 400.0),
		));
		controller.viewDidLayoutSubviews();
		assert!((container.subviews().objectAtIndex(0).frame().size.width - 300.0).abs() < 0.01);
		test_window.setHidden(true);
		test_window.setRootViewController(None);

		let loading_host = UIKitHost::new(mtm, || {
			leptoskit_native::components::ActivityIndicator(true)
		});
		let scene = window.windowScene().expect("connected scene");
		let indicator = objc2::rc::autoreleasepool(|_| {
			let test_window = loading_host.open_window(&scene);
			let controller = test_window
				.rootViewController()
				.expect("loading controller");
			let root = controller
				.view()
				.expect("loading container")
				.subviews()
				.objectAtIndex(0);
			let indicator = root
				.downcast_ref::<UIActivityIndicatorView>()
				.expect("native loading root")
				.retain();
			assert!(indicator.isAnimating());
			test_window.setRootViewController(None);
			indicator
		});
		assert!(
			!indicator.isAnimating(),
			"controller teardown pauses retained host animations"
		);
		loading_host.set_dark_mode(true);
		assert!(
			!indicator.isAnimating(),
			"closed host does not restart animations on theme updates"
		);
		let reopened = loading_host.open_window(&scene);
		assert!(
			indicator.isAnimating(),
			"a new controller resumes requested animation"
		);
		reopened.setRootViewController(None);
	}

	fn mounted_fixture(element: &Element, window: &UIWindow, mtm: MainThreadMarker) -> Surface {
		let surface = Rc::new(RefCell::new(NativeSurface::new(UIKitBackend::new(mtm))));
		surface.borrow_mut().set_viewport(
			window.bounds().size.width as f32,
			window.bounds().size.height as f32,
		);
		surface.borrow_mut().set_root(element);
		let controller = window.rootViewController().expect("probe controller");
		let root = surface.borrow().root().expect("fixture root");
		assert!(surface.borrow().backend().mount(root, &controller));
		window.makeKeyAndVisible();
		surface
	}

	fn multiline_metrics(mtm: MainThreadMarker) {
		for (text, font_size, width) in [
			(
				"Low Power Mode reduces background activity like downloads and mail fetch until \
				 you charge your phone.",
				13.0,
				338.0,
			),
			("one two three four five", 14.0, 45.0),
			("A line\nA second line\n", 13.0, 120.0),
			("Shared text العربية עברית 😀 across platforms", 13.0, 120.0),
		] {
			let mut surface = NativeSurface::new(UIKitBackend::new(mtm));
			surface.set_viewport(width, 500.0);
			surface.set_root(
				&Element::text(text)
					.with_props(|props| props.font_size = Some(font_size))
					.with_style(|style| style.width = leptoskit_yoga::Dimension::Points(width)),
			);
			let root = surface.root().expect("wrapped label");
			let label = surface
				.backend()
				.view(root)
				.expect("native label")
				.downcast_ref::<UILabel>()
				.expect("UILabel");
			let fitting = label.sizeThatFits(CGSize::new(f64::from(width), 1.0e7));
			let measured = UIKitTextMeasurer::new().measure(
				text,
				TextStyle::new(font_size, FontWeight::Regular),
				Size::new(AvailableSpace::Definite(width), AvailableSpace::MaxContent),
			);
			assert!(
				f64::from(measured.height) >= fitting.height
					&& f64::from(measured.height) - fitting.height <= 1.0,
				"multiline metrics at {font_size}pt width {width}: Core Text {measured:?}, \
				 UILabel {fitting:?}"
			);
		}
	}

	fn reentrant_lifecycle(window: &UIWindow, mtm: MainThreadMarker) {
		let fixture = mounted_fixture(
			&Element::view().child(named(
				leptoskit_native::components::Switch(false),
				"default-switch",
			)),
			window,
			mtm,
		);
		let switch = native::<UISwitch>(&fixture, "default-switch");
		let outer = native::<UIView>(&fixture, "default-switch");
		assert!((outer.frame().size.width - 40.0).abs() < 0.01);
		assert!((outer.frame().size.height - 24.0).abs() < 0.01);
		assert!(switch.frame().origin.x >= -0.01 && switch.frame().origin.y >= -0.01);
		assert!(switch.frame().size.width <= 40.01 && switch.frame().size.height <= 24.01);
		fixture
			.borrow_mut()
			.set_root(&Element::text("Switch deleted"));
		assert!(outer.superview().is_none());
		assert!(switch.window().is_none());
		assert_eq!(switch.allTargets().count(), 0);

		let fixture = mounted_fixture(
			&Element::view().child(named(
				leptoskit_native::components::Slider(SliderState::default()),
				"deleting-slider",
			)),
			window,
			mtm,
		);
		let slider = native::<UISlider>(&fixture, "deleting-slider");
		let raw = &raw const *slider;
		drop(slider);
		let weak = Rc::downgrade(&fixture);
		fixture
			.borrow_mut()
			.backend_mut()
			.set_event_listener(Some(Rc::new(move |_, event| {
				if matches!(event, NativeEvent::ValueChange { .. }) {
					weak.upgrade()
						.expect("live slider surface")
						.borrow_mut()
						.set_root(&Element::text("Slider deleted during accessibility action"));
				}
			})));
		// SAFETY: The surface owns this receiver at method entry. Its override
		// retains itself before the callback synchronously removes its node.
		unsafe {
			(&*raw).accessibilityIncrement(mtm);
		}
		assert_eq!(
			fixture
				.borrow()
				.kind(fixture.borrow().root().expect("replacement")),
			NativeKind::Text
		);

		let fixture = mounted_fixture(
			&Element::view().child(named(
				Element::text_input().class("w-48 h-10"),
				"deleting-input",
			)),
			window,
			mtm,
		);
		let input = native::<UITextField>(&fixture, "deleting-input");
		assert!(input.becomeFirstResponder());
		let weak = Rc::downgrade(&fixture);
		let blurs = Rc::new(std::cell::Cell::new(0));
		let observed_blurs = Rc::clone(&blurs);
		fixture
			.borrow_mut()
			.backend_mut()
			.set_event_listener(Some(Rc::new(move |_, event| {
				if matches!(event, NativeEvent::Blur) {
					observed_blurs.set(observed_blurs.get() + 1);
				}
				if matches!(event, NativeEvent::Change { .. }) {
					weak.upgrade()
						.expect("live editor surface")
						.borrow_mut()
						.set_root(&Element::text("Focused editor deleted during input"));
				}
			})));
		input.insertText(&NSString::from_str("x"));
		assert!(input.superview().is_none());
		assert!(input.delegate().is_none());
		assert_eq!(
			blurs.get(),
			0,
			"commit never reenters through editor teardown"
		);

		let fixture = mounted_fixture(
			&named(
				Element::scroll_view()
					.class("w-48 h-16")
					.child(Element::text("Scrollable native child").class("h-96")),
				"deleting-scroll",
			),
			window,
			mtm,
		);
		let scroll = native::<UIScrollView>(&fixture, "deleting-scroll");
		let weak = Rc::downgrade(&fixture);
		fixture
			.borrow_mut()
			.backend_mut()
			.set_event_listener(Some(Rc::new(move |_, event| {
				if matches!(event, NativeEvent::Scroll { .. }) {
					weak.upgrade()
						.expect("live scroll surface")
						.borrow_mut()
						.set_root(&Element::text("Scroll deleted inside delegate"));
				}
			})));
		scroll.setContentOffset(CGPoint::new(0.0, 40.0));
		assert!(scroll.superview().is_none());
		assert!(scroll.delegate().is_none());
	}
}

/// Writes the LLVM coverage profile now. The probe never exits normally: the
/// verifier shuts its simulator down after reading the report.
#[cfg(all(target_os = "ios", feature = "coverage"))]
fn flush_coverage() {
	unsafe extern "C" {
		fn __llvm_profile_write_file() -> i32;
	}
	// SAFETY: The profiler runtime is linked by `-C instrument-coverage`;
	// writing the profile has no preconditions beyond that.
	let status = unsafe { __llvm_profile_write_file() };
	assert_eq!(status, 0, "write the LLVM coverage profile");
}
