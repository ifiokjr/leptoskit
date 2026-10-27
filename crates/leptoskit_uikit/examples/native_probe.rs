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

	use leptoskit_native::Color;
	use leptoskit_native::Drawing;
	use leptoskit_native::Element;
	use leptoskit_native::FillRule;
	use leptoskit_native::FontWeight;
	use leptoskit_native::LineCap;
	use leptoskit_native::NativeBackend;
	use leptoskit_native::NativeEvent;
	use leptoskit_native::NativeKind;
	use leptoskit_native::NativeProps;
	use leptoskit_native::NativeSurface;
	use leptoskit_native::PathData;
	use leptoskit_native::Progress;
	use leptoskit_native::ProgressValue;
	use leptoskit_native::RadioGroup;
	use leptoskit_native::SliderState;
	use leptoskit_native::Stroke;
	use leptoskit_native::StrokeScaling;
	use leptoskit_native::TextAlign;
	use leptoskit_native::TextMeasurer;
	use leptoskit_native::TextStyle;
	use leptoskit_native::VectorPath;
	use leptoskit_native::VectorScaling;
	use leptoskit_native::ViewBox;
	use leptoskit_native::vector::PathCommand;
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
	use objc2_core_foundation::CGRect;
	use objc2_core_foundation::CGSize;
	use objc2_core_graphics::CGPath;
	use objc2_foundation::NSHomeDirectory;
	use objc2_foundation::NSNumber;
	use objc2_foundation::NSObjectNSDelayedPerforming;
	use objc2_foundation::NSObjectNSKeyValueCoding;
	use objc2_foundation::NSString;
	use objc2_foundation::NSURL;
	use objc2_quartz_core::CAShapeLayer;
	use objc2_quartz_core::kCAFillRuleEvenOdd;
	use objc2_quartz_core::kCAFillRuleNonZero;
	use objc2_quartz_core::kCALineCapRound;
	use objc2_ui_kit::NSObjectUIAccessibility;
	use objc2_ui_kit::NSObjectUIAccessibilityAction;
	use objc2_ui_kit::UIAccessibilityTraitImage;
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

	/// A fitted, filled triangle beside a stretched square that is filled and
	/// stroked, so the probe sees both scalings.
	fn vector_fixtures() -> Element {
		Element::view()
			.class("flex-row items-center gap-4 h-10")
			.children([
				named(
					Element::vector(ViewBox::new(0.0, 0.0, 24.0, 24.0).expect("fixture box"))
						.path(
							VectorPath::new(
								PathData::parse("M2 22 L12 2 L22 22 Z").expect("fixture path"),
							)
							.fill(Color::rgba(79, 70, 229, 255)),
						)
						.class("w-24 h-10"),
					"vector-fit",
				),
				named(
					Element::vector(ViewBox::new(0.0, 0.0, 10.0, 10.0).expect("fixture box"))
						.scaling(VectorScaling::Stretch)
						.paths([
							VectorPath::new(
								PathData::parse("M1 1 H9 V9 H1 Z").expect("fixture path"),
							)
							.fill(Color::rgba(226, 232, 240, 255)),
							VectorPath::new(
								PathData::parse("M1 1 H9 V9 H1 Z").expect("fixture path"),
							)
							.stroke(
								Stroke::new(Color::rgba(15, 23, 42, 255), 2.0)
									.expect("fixture stroke")
									.cap(LineCap::Round),
							),
						])
						.class("w-24 h-10"),
					"vector-stretch",
				),
			])
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
				vector_fixtures(),
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

	fn rive_uploads(mtm: MainThreadMarker) {
		use leptoskit_native::RiveFrame;
		use leptoskit_yoga::Yoga;
		use objc2_core_graphics::CGDataProvider;
		use objc2_core_graphics::CGImage;
		let mut backend = UIKitBackend::new(mtm);
		let mut yoga = Yoga::new();
		let id = yoga.create_node();
		backend.create(id, NativeKind::Rive, &NativeProps::default());
		let view = backend
			.view(id)
			.unwrap()
			.downcast_ref::<UIImageView>()
			.unwrap()
			.retain();
		let mut pixels = [128, 0, 0, 128, 0, 0, 255, 255];
		assert!(backend.present_rive(id, &RiveFrame::new(2, 1, &pixels).unwrap()));
		let image = view.image().unwrap();
		pixels.fill(0);
		// SAFETY: UIKit retains the immutable two-pixel CGImage created by the
		// adapter.
		let native = unsafe { image.CGImage() }.unwrap();
		let provider = CGImage::data_provider(Some(&native)).unwrap();
		let data = CGDataProvider::data(Some(&provider)).unwrap();
		assert_eq!(data.length(), 8);
		// SAFETY: The retained CFData owns exactly eight initialized pixel
		// bytes.
		assert_eq!(
			unsafe { std::slice::from_raw_parts(data.byte_ptr(), 8) },
			[128, 0, 0, 128, 0, 0, 255, 255]
		);
		backend.update_props(id, &NativeProps::default());
		assert_eq!(
			view.image().unwrap(),
			image,
			"ordinary updates preserve the paused native image"
		);
		backend.delete(id);
		assert!(backend.view(id).is_none());
	}

	fn verify(surface: &Surface, window: &UIWindow) -> Vec<String> {
		let mtm = MainThreadMarker::new().expect("probe main thread");
		multiline_metrics(mtm);
		rive_uploads(mtm);
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
		passed.push(
			"PASS\tRive Core Graphics upload owns premultiplied pixels and preserves paused images"
				.into(),
		);
		assert!(window.isKeyWindow());
		let checks = Checks::new(surface, events, mtm);
		passed.push(checks.built_in_kinds().into());
		vectors(surface, mtm);
		passed.push(
			"PASS\tvector paths draw as frame-mapped shape layers with fill rules, dashes, and \
			 scaled strokes"
				.into(),
		);
		passed.push(checks.border_boxes().into());
		passed.push(checks.text_metrics().into());
		passed.push(checks.boolean_controls().into());
		passed.push(checks.radio_keyboard().into());
		passed.push(checks.accepted_controls().into());
		passed.push(checks.slider_grids().into());
		passed.push(checks.editors().into());
		passed.push(checks.disabled_and_hidden().into());
		passed.push(checks.button_press_phases().into());
		passed.push(checks.scrolling_and_hit_testing().into());
		passed.push(checks.animations().into());
		passed.push(checks.appearance().into());
		passed.push(checks.reentrant_removal().into());

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

	/// The probe screen, its recorded events, and its native controls, looked
	/// up once so later checks prove that updates keep the same UIKit objects.
	/// Each check returns the report line it proves.
	struct Checks<'a> {
		surface: &'a Surface,
		events: Rc<RefCell<Vec<(NodeId, NativeEvent)>>>,
		mtm: MainThreadMarker,
		root: NodeId,
		label: Retained<UILabel>,
		input: Retained<UITextField>,
		password: Retained<UITextField>,
		switch: Retained<UISwitch>,
		checkbox: Retained<UIButton>,
		radio_a: Retained<UIButton>,
		radio_b: Retained<UIButton>,
		slider: Retained<UISlider>,
		button: Retained<UIButton>,
		pressable: Retained<UIButton>,
		scroll: Retained<UIScrollView>,
		image: Retained<UIImageView>,
		activity: Retained<UIActivityIndicatorView>,
		/// The indeterminate bar's moving segment, which must stop animating
		/// once its node is removed.
		fill: Retained<UIView>,
	}

	impl<'a> Checks<'a> {
		fn new(
			surface: &'a Surface,
			events: Rc<RefCell<Vec<(NodeId, NativeEvent)>>>,
			mtm: MainThreadMarker,
		) -> Self {
			let checks = Self {
				surface,
				events,
				mtm,
				label: native(surface, "title"),
				input: native(surface, "input"),
				password: native(surface, "password"),
				switch: native(surface, "switch"),
				checkbox: native(surface, "checkbox"),
				radio_a: native(surface, "radio-a"),
				radio_b: native(surface, "radio-b"),
				slider: native(surface, "slider"),
				button: native(surface, "button"),
				pressable: native(surface, "pressable"),
				scroll: native(surface, "scroll"),
				image: native(surface, "image"),
				activity: native(surface, "activity"),
				root: surface.borrow().root().expect("mounted root"),
				fill: native::<UIView>(surface, "loading")
					.subviews()
					.objectAtIndex(1),
			};
			let _text_run = native::<UILabel>(surface, "run");
			checks
		}

		fn built_in_kinds(&self) -> &'static str {
			let surface = self.surface;
			let label = &*self.label;
			let image = &*self.image;
			let scroll = &*self.scroll;
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
			"PASS\tall 16 built-in kinds mount real UIKit views"
		}

		fn border_boxes(&self) -> &'static str {
			let surface = self.surface;
			let root = self.root;
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
			"PASS\tUIKit border boxes match Yoga allocations"
		}

		fn text_metrics(&self) -> &'static str {
			let surface = self.surface;
			let label = &*self.label;
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
			"PASS\tCore Text metrics match UILabel and measure off UI thread"
		}

		fn boolean_controls(&self) -> &'static str {
			let events = &self.events;
			let switch = &*self.switch;
			let checkbox = &*self.checkbox;
			let radio_a = &*self.radio_a;
			let radio_b = &*self.radio_b;
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
			"PASS\tboolean controls reject proposals without uncontrolled drift"
		}

		fn radio_keyboard(&self) -> &'static str {
			let surface = self.surface;
			let events = &self.events;
			let mtm = self.mtm;
			let radio_a = &*self.radio_a;
			let key = radio_a
				.keyCommands()
				.expect("radio keyboard commands")
				.objectAtIndex(0);
			let before = events.borrow().len();
			// SAFETY: nextRadio: is installed on this live native radio button
			// and receives its UIKeyCommand through UIApplication's responder
			// action API.
			assert!(unsafe {
				UIApplication::sharedApplication(mtm).sendAction_to_from_forEvent(
					sel!(nextRadio:),
					Some(radio_a),
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
					Some(radio_a),
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
			"PASS\tradio keyboard commands follow explicit groups and skip hidden members"
		}

		fn accepted_controls(&self) -> &'static str {
			let surface = self.surface;
			let events = &self.events;
			let switch = &*self.switch;
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
			let record = Rc::clone(events);
			surface
				.borrow_mut()
				.backend_mut()
				.set_event_listener(Some(Rc::new(move |node, event| {
					record.borrow_mut().push((node, event.clone()));
				})));
			"PASS\taccepted synchronous controls retain their native identity"
		}

		fn slider_grids(&self) -> &'static str {
			let surface = self.surface;
			let events = &self.events;
			let mtm = self.mtm;
			let slider = &*self.slider;
			slider.setValue(0.47);
			slider.sendActionsForControlEvents(UIControlEvents::ValueChanged);
			assert!((slider.value() - 0.4).abs() < 0.0001);
			assert!(events.borrow().iter().any(|(_, event)| matches!(event, NativeEvent::ValueChange { value } if (*value - 45.0).abs() < 0.0001)));
			let extreme = SliderState::new(0.0, 1.0e200, 0.0, Some(1.0e199))
				.expect("shared finite large grid");
			modify(surface, "slider", |props| props.slider = Some(extreme));
			slider.setValue(0.75);
			slider.sendActionsForControlEvents(UIControlEvents::ValueChanged);
			let expected = extreme
				.proposed_value(extreme.max() * 0.75)
				.expect("shared canonical proposal");
			assert!(events.borrow().iter().any(|(_, event)| matches!(event, NativeEvent::ValueChange { value } if (*value - expected).abs() / expected.abs() < 1.0e-12)));
			assert!((slider.value() - 0.0).abs() < 0.0001);
			let tiny = SliderState::new(0.0, 1.0e-200, 0.0, Some(1.0e-201))
				.expect("shared finite tiny grid");
			modify(surface, "slider", |props| props.slider = Some(tiny));
			slider.accessibilityIncrement(mtm);
			assert!(events.borrow().iter().any(|(_, event)| matches!(event, NativeEvent::ValueChange { value } if *value > 0.0 && (*value / 1.0e-201 - 1.0).abs() < 0.001)));
			for state in [
				SliderState::new(1.0, 1.0f64.next_up(), 1.0, None)
					.expect("adjacent continuous range"),
				SliderState::new(0.0, f64::from_bits(2), 0.0, None)
					.expect("subnormal continuous range"),
			] {
				modify(surface, "slider", |props| props.slider = Some(state));
				let before = events.borrow().len();
				slider.accessibilityIncrement(mtm);
				assert!(events.borrow()[before..].iter().any(|(_, event)| matches!(event, NativeEvent::ValueChange { value } if *value > state.min() && *value <= state.max())));
			}
			"PASS\tf64 slider grids survive normalized UIKit coordinates and accessibility steps"
		}

		fn editors(&self) -> &'static str {
			let surface = self.surface;
			let events = &self.events;
			let input = &*self.input;
			let password = &*self.password;
			// UIKit forwards this optional UITextInputTraits property. KVC
			// observes the real public property without objc2's direct-method
			// introspection. SAFETY: secureTextEntry is UITextField's
			// documented boolean property.
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
			// SAFETY: The delegate conforms to UITextFieldDelegate and this
			// field is live.
			unsafe {
				let _: bool = msg_send![&*delegate, textFieldShouldReturn: input];
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
			"PASS\tsecure controlled editor emits input submit focus and blur"
		}

		fn disabled_and_hidden(&self) -> &'static str {
			let surface = self.surface;
			let events = &self.events;
			let root = self.root;
			let button = &*self.button;
			let switch = &*self.switch;
			let input = &*self.input;
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
			"PASS\tdisabled and hidden native controls do not dispatch"
		}

		fn button_press_phases(&self) -> &'static str {
			let surface = self.surface;
			let events = &self.events;
			let mtm = self.mtm;
			let button = &*self.button;
			let before = events.borrow().len();
			let node = id(&surface.borrow(), "button");
			for event in [
				UIControlEvents::TouchDown,
				UIControlEvents::TouchDragExit,
				UIControlEvents::TouchDragEnter,
				UIControlEvents::TouchCancel,
			] {
				button.sendActionsForControlEvents(event);
			}
			assert_eq!(
				events.borrow()[before..],
				[
					(node, NativeEvent::PressIn),
					(node, NativeEvent::PressOut),
					(node, NativeEvent::PressIn),
					(node, NativeEvent::PressOut),
				]
			);
			surface.borrow_mut().press_in(node);
			let held = surface
				.borrow()
				.backend()
				.view(node)
				.expect("button")
				.retain();
			assert!(
				(held.alpha() - 0.7).abs() < 1e-6,
				"drawn buttons dim while held"
			);
			assert_eq!(
				held.accessibilityLabel(mtm).map(|label| label.to_string()),
				Some("button".to_owned())
			);
			surface.borrow_mut().press_out(node);
			assert!(
				(held.alpha() - 1.0).abs() < 1e-6,
				"releasing restores the opacity"
			);
			"PASS\tbutton highlight events report press phases and restyle in place"
		}

		fn scrolling_and_hit_testing(&self) -> &'static str {
			let events = &self.events;
			let mtm = self.mtm;
			let scroll = &*self.scroll;
			let button = &*self.button;
			let checkbox = &*self.checkbox;
			let pressable = &*self.pressable;
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
			assert!(std::ptr::eq(&raw const *hit, pressable as &UIView));
			assert!(
				pressable
					.hitTest_withEvent(CGPoint::new(-1.0, 20.0), None)
					.is_none()
			);
			// A view that stops being modal releases VoiceOver again.
			modify(self.surface, "button", |props| {
				props.aria_mut().modal = Some(true)
			});
			assert!(button.accessibilityViewIsModal(mtm));
			modify(self.surface, "button", |props| {
				props.aria_mut().modal = None
			});
			assert!(!button.accessibilityViewIsModal(mtm));
			"PASS\tnative scrolling accessibility activation, composed hit testing, and modal reset"
		}

		fn animations(&self) -> &'static str {
			let surface = self.surface;
			let mtm = self.mtm;
			let root = self.root;
			let activity = &*self.activity;
			let fill = &*self.fill;
			let root_props = surface.borrow().props(root).clone();
			assert!(activity.isAnimating());
			assert!(fill.layer().animationKeys().is_some());
			// SAFETY: Core Animation's presentation layer is a live read-only
			// copy.
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
			"PASS\tindeterminate bar and spinner run and stop native animations"
		}

		fn appearance(&self) -> &'static str {
			let surface = self.surface;
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
			"PASS\texplicit appearance updates existing UIKit controls"
		}

		fn reentrant_removal(&self) -> &'static str {
			let surface = self.surface;
			let switch = &*self.switch;
			let activity = &*self.activity;
			let fill = &*self.fill;
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
			"PASS\treentrant removal invalidates retained actions and stops timers"
		}
	}

	/// The shape layers a `Vector` view paints, in painting order.
	fn shapes(view: &UIView) -> Vec<Retained<CAShapeLayer>> {
		// SAFETY: Reading a live view's sublayers on the main thread.
		unsafe { view.layer().sublayers() }
			.map(|layers| {
				layers
					.iter()
					.filter_map(|layer| layer.downcast::<CAShapeLayer>().ok())
					.collect()
			})
			.unwrap_or_default()
	}

	fn assert_box(layer: &CAShapeLayer, expected: CGRect, message: &str) {
		let actual = CGPath::path_bounding_box(layer.path().as_deref());
		assert!(
			(actual.origin.x - expected.origin.x).abs() < 0.01
				&& (actual.origin.y - expected.origin.y).abs() < 0.01
				&& (actual.size.width - expected.size.width).abs() < 0.01
				&& (actual.size.height - expected.size.height).abs() < 0.01,
			"{message}: path box {actual:?}, expected {expected:?}"
		);
	}

	fn vectors(surface: &Surface, mtm: MainThreadMarker) {
		let fit = native::<UIView>(surface, "vector-fit");
		let stretch = native::<UIView>(surface, "vector-stretch");
		let fit_node = id(&surface.borrow(), "vector-fit");
		let stretch_node = id(&surface.borrow(), "vector-stretch");

		// Every path becomes one shape layer whose geometry is the drawing
		// mapped onto the Yoga frame; strokes keep their layout-unit width.
		for (node, view) in [(fit_node, &fit), (stretch_node, &stretch)] {
			let drawing = surface
				.borrow()
				.props(node)
				.drawing
				.clone()
				.expect("fixture drawing");
			let size = view.bounds().size;
			let transform = drawing.transform(size.width as f32, size.height as f32);
			let layers = shapes(view);
			assert_eq!(layers.len(), drawing.paths().len(), "one layer per path");
			for (layer, path) in layers.iter().zip(drawing.paths()) {
				let points: Vec<_> = path
					.data()
					.transformed(transform)
					.filter_map(|command| {
						match command {
							PathCommand::MoveTo(point) | PathCommand::LineTo(point) => Some(point),
							_ => None,
						}
					})
					.collect();
				let min_x = points
					.iter()
					.map(|point| point.x)
					.fold(f32::INFINITY, f32::min);
				let min_y = points
					.iter()
					.map(|point| point.y)
					.fold(f32::INFINITY, f32::min);
				let max_x = points
					.iter()
					.map(|point| point.x)
					.fold(f32::NEG_INFINITY, f32::max);
				let max_y = points
					.iter()
					.map(|point| point.y)
					.fold(f32::NEG_INFINITY, f32::max);
				assert_box(
					layer,
					CGRect::new(
						CGPoint::new(f64::from(min_x), f64::from(min_y)),
						CGSize::new(f64::from(max_x - min_x), f64::from(max_y - min_y)),
					),
					"view box maps onto the frame",
				);
				assert_eq!(layer.fillColor().is_some(), path.fill_color().is_some());
				assert_eq!(layer.strokeColor().is_some(), path.stroke_style().is_some());
				if let Some(stroke) = path.stroke_style() {
					assert!(
						(layer.lineWidth() - f64::from(stroke.width())).abs() < 1.0e-6,
						"stroke width stays in layout units under a stretched view box"
					);
					assert!((layer.miterLimit() - 4.0).abs() < 1.0e-6);
					assert!(
						layer.lineDashPattern().is_none(),
						"solid strokes are undashed"
					);
				}
				// SAFETY: Core Animation exports this immutable constant.
				assert!(
					layer
						.fillRule()
						.isEqualToString(unsafe { kCAFillRuleNonZero })
				);
				assert!(!layer.masksToBounds());
			}
		}
		let stretched = shapes(&stretch);
		assert!(stretched[0].strokeColor().is_none() && stretched[1].fillColor().is_none());
		// SAFETY: Core Animation exports this immutable constant.
		assert!(
			stretched[1]
				.lineCap()
				.isEqualToString(unsafe { kCALineCapRound })
		);
		assert!(
			!fit.clipsToBounds(),
			"drawings paint outside unclipped frames"
		);

		// Fit letterboxes a square drawing inside a wider frame: a 48x40
		// frame scales by 40/24 and centres it 4 points in.
		surface
			.borrow_mut()
			.backend_mut()
			.update_layout(fit_node, 0.0, 0.0, 48.0, 40.0);
		assert_box(
			&shapes(&fit)[0],
			CGRect::new(
				CGPoint::new(4.0 + 2.0 * 40.0 / 24.0, 2.0 * 40.0 / 24.0),
				CGSize::new(20.0 * 40.0 / 24.0, 20.0 * 40.0 / 24.0),
			),
			"layout changes redraw",
		);
		let layout = surface.borrow().layout(fit_node);
		surface.borrow_mut().backend_mut().update_layout(
			fit_node,
			layout.x,
			layout.y,
			layout.width,
			layout.height,
		);

		// Prop updates add and remove layers to match the paths.
		modify(surface, "vector-stretch", |props| {
			let drawing = props.drawing.as_ref().expect("fixture drawing");
			props.drawing = Some(
				Drawing::new(drawing.view_box())
					.scaling(drawing.vector_scaling())
					.path(drawing.paths()[1].clone()),
			);
		});
		let remaining = shapes(&stretch);
		assert_eq!(remaining.len(), 1);
		assert!(remaining[0].fillColor().is_none() && remaining[0].strokeColor().is_some());
		modify(surface, "vector-stretch", |_| {});
		assert_eq!(shapes(&stretch).len(), 2);

		styled_vectors(surface, &fit, fit_node);

		// A named drawing is one image element; an unnamed one is decorative.
		assert!(fit.isAccessibilityElement(mtm));
		assert_eq!(
			fit.accessibilityLabel(mtm)
				.expect("vector name")
				.to_string(),
			"vector-fit"
		);
		// SAFETY: UIKit exports this immutable trait bit.
		assert_ne!(
			fit.accessibilityTraits(mtm) & unsafe { UIAccessibilityTraitImage },
			0
		);
		let mut unnamed = surface.borrow().props(fit_node).clone();
		unnamed.accessibility_label = None;
		surface
			.borrow_mut()
			.backend_mut()
			.update_props(fit_node, &unnamed);
		assert!(!fit.isAccessibilityElement(mtm));
		modify(surface, "vector-fit", |_| {});
		assert!(fit.isAccessibilityElement(mtm));
	}

	/// Fill rules, dashes, miter limits, and scaling strokes reach the shape
	/// layers in frame units, and restoring the plain drawing resets them on
	/// the same, reused layers.
	fn styled_vectors(surface: &Surface, fit: &UIView, fit_node: NodeId) {
		let ink = Color::rgba(15, 23, 42, 255);
		modify(surface, "vector-fit", |props| {
			let ring = PathData::parse("M2 2 H22 V22 H2 Z M8 8 H16 V16 H8 Z").expect("ring path");
			let line = PathData::parse("M2 12 H22").expect("line path");
			props.drawing = Some(
				Drawing::new(ViewBox::new(0.0, 0.0, 24.0, 24.0).expect("fixture box"))
					.path(VectorPath::new(ring).fill(ink).fill_rule(FillRule::Evenodd))
					.path(
						VectorPath::new(line).stroke(
							Stroke::new(ink, 2.0)
								.expect("fixture stroke")
								.miter_limit(8.0)
								.expect("fixture miter limit")
								.dashes(&[3.0, 1.0], 1.0)
								.expect("fixture dashes")
								.scaling(StrokeScaling::Drawing),
						),
					),
			);
		});
		let layers = shapes(fit);
		assert_eq!(layers.len(), 2, "a new drawing adds the missing layer");
		// SAFETY: Core Animation exports this immutable constant.
		assert!(
			layers[0]
				.fillRule()
				.isEqualToString(unsafe { kCAFillRuleEvenOdd }),
			"evenodd paths use Core Animation's even-odd rule"
		);
		// The 24-unit box fits the 40-point-tall frame at 40/24, and a
		// drawing-scaled stroke grows with it.
		let size = fit.bounds().size;
		let scale = (size.width / 24.0).min(size.height / 24.0);
		let close = |actual: f64, expected: f64| (actual - expected).abs() < 1.0e-4;
		let dashed = &layers[1];
		assert!(
			close(dashed.lineWidth(), 2.0 * scale),
			"scaling strokes take their frame width: {} at scale {scale}",
			dashed.lineWidth()
		);
		assert!(close(dashed.miterLimit(), 8.0));
		let pattern: Vec<f64> = dashed
			.lineDashPattern()
			.expect("a dashed stroke has a pattern")
			.iter()
			.map(|length| length.as_f64())
			.collect();
		assert!(
			pattern.len() == 2 && close(pattern[0], 3.0 * scale) && close(pattern[1], scale),
			"dash lengths scale into frame units: {pattern:?}"
		);
		assert!(close(dashed.lineDashPhase(), scale));

		modify(surface, "vector-fit", |_| {});
		let restored = shapes(fit);
		assert_eq!(restored.len(), 1, "the extra layer goes");
		assert!(
			Retained::as_ptr(&restored[0]) == Retained::as_ptr(&layers[0]),
			"the remaining layer is reused, not replaced"
		);
		// SAFETY: Core Animation exports this immutable constant.
		assert!(
			restored[0]
				.fillRule()
				.isEqualToString(unsafe { kCAFillRuleNonZero })
		);
		// A fill rule set behind the backend's back survives props with an
		// equal drawing, where a redraw would restore nonzero.
		// SAFETY: Core Animation exports this immutable constant.
		restored[0].setFillRule(unsafe { kCAFillRuleEvenOdd });
		let unchanged = surface.borrow().props(fit_node).clone();
		surface
			.borrow_mut()
			.backend_mut()
			.update_props(fit_node, &unchanged);
		// SAFETY: Core Animation exports this immutable constant.
		assert!(
			shapes(fit)[0]
				.fillRule()
				.isEqualToString(unsafe { kCAFillRuleEvenOdd }),
			"an unchanged drawing is not redrawn"
		);
		// SAFETY: Core Animation exports this immutable constant.
		restored[0].setFillRule(unsafe { kCAFillRuleNonZero });
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
		container.setFrame(CGRect::new(CGPoint::ZERO, CGSize::new(300.0, 400.0)));
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
