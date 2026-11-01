//! Real-Activity fixture, built only by the explicit `probe` feature.
#![allow(
	clippy::missing_const_for_thread_local,
	reason = "Rust 1.90's Android TLS macro diagnoses its generated drop-state initializer; PROBE \
	          is already const"
)]

use std::cell::Cell;
use std::cell::RefCell;
use std::ffi::CString;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;

use jni::JNIEnv;
use jni::objects::JClass;
use jni::objects::JObject;
use jni::objects::JString;
use jni::sys::jfloat;
use jni::sys::jint;
use jni::sys::jlong;
use jni::sys::jstring;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FillRule;
use leptoskit_native::FontWeight;
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
use leptoskit_native::VectorPath;
use leptoskit_native::VectorScaling;
use leptoskit_native::ViewBox;
use leptoskit_native::components::*;
use leptoskit_yoga::NodeId;
use leptoskit_yoga::Yoga;

use crate::AndroidBackend;

#[derive(Default)]
#[allow(
	clippy::struct_excessive_bools,
	reason = "independent fixture toggles exercise the platform state matrix"
)]
struct State {
	accept: bool,
	checked: bool,
	radio: bool,
	value: String,
	slider: f64,
	removed: bool,
	hidden: bool,
	enabled: bool,
	events: [usize; 10],
	image: String,
	shared_paint: bool,
	running: bool,
	button_alignment: Option<TextAlign>,
	root_fixed: bool,
	editor_removed: bool,
	vector_recolored: bool,
}

struct Probe {
	surface: Rc<RefCell<NativeSurface<AndroidBackend>>>,
	state: Arc<Mutex<State>>,
}

/// A backend driven one `NativeBackend` method at a time. `NativeSurface`
/// batches every commit through `apply`, so the fixture above never reaches
/// the single-mutation methods.
struct Contract {
	backend: AndroidBackend,
	root: NodeId,
	button: NodeId,
	rive: NodeId,
	presses: Rc<Cell<usize>>,
}

thread_local! {
	static PROBE: RefCell<Option<Probe>> = const { RefCell::new(None) };
	static CONTRACT: RefCell<Option<Contract>> = const { RefCell::new(None) };
}

fn fixture(state: &State) -> Element {
	let group = RadioGroup::new("density").expect("valid fixture group");
	let mut root =
		View()
			.enabled(state.enabled)
			.with_props(|props| props.hidden = Some(state.hidden))
			.class(if state.root_fixed {
				"w-72 h-[800px] m-3 p-4 gap-1 bg-slate-50 text-slate-900"
			} else {
				"w-full h-full p-4 gap-1 bg-slate-50 text-slate-900"
			})
			.child(Text("Android native components").class("h-7 text-xl font-bold"))
			.children(
				[
					("Amara Okafor", 24.0, FontWeight::Semibold),
					("Following", 14.0, FontWeight::Regular),
					("Projects", 14.0, FontWeight::Regular),
					("Overview", 28.0, FontWeight::Bold),
				]
				.map(|(label, size, weight)| {
					View()
						.class("flex-row")
						.child(Text(label).font(size, weight))
				}),
			)
			.child(
				Element::new(NativeKind::TextRun)
					.accessibility_label("Wrapped text")
					.with_style(|style| style.direction = leptoskit_yoga::Direction::Rtl)
					.with_props(|p| {
						p.text = Some(
							"Platform widgets العربية हिन्दी 日本語 wrap consistently across lines."
								.into(),
						);
					})
					.class("w-48 text-sm"),
			)
			.child(
				Button("Activate")
					.accessibility_label("Action")
					.with_style(|style| style.direction = leptoskit_yoga::Direction::Rtl)
					.with_props(|props| props.text_align = state.button_alignment)
					.class(if state.shared_paint {
						"w-40 h-10 bg-blue-600 text-white rounded-lg"
					} else {
						"w-40 h-10"
					}),
			)
			.child(if state.editor_removed {
				View().class("w-full h-10")
			} else {
				TextInput()
					.accessibility_label("Editor")
					.with_props(|props| props.text = Some(state.value.clone()))
					.placeholder("Text input")
					.class(if state.shared_paint {
						"w-full h-10 bg-white border border-slate-300 rounded-lg"
					} else {
						"w-full h-10"
					})
			})
			.child(
				PasswordInput()
					.accessibility_label("Secure editor")
					.with_props(|props| props.text = Some("fixture-only".into()))
					.class("w-full h-10"),
			)
			.child(
				View()
					.accessibility_label("Control group")
					.enabled(state.enabled)
					.with_props(|p| p.hidden = Some(state.hidden))
					.class("gap-1")
					.child(
						Switch(state.checked)
							.accessibility_label("Notifications")
							.class("w-36 h-10"),
					)
					.child(
						Checkbox(state.checked)
							.accessibility_label("Preview")
							.class("w-36 h-9"),
					)
					.child(
						Radio(group.clone(), !state.radio)
							.accessibility_label("Comfortable")
							.class("w-36 h-9"),
					)
					.child(
						Radio(group, state.radio)
							.accessibility_label("Compact")
							.class("w-36 h-9"),
					),
			)
			.child(
				Slider(
					SliderState::new(1e100, 2e100, state.slider, Some(1e99))
						.expect("valid extreme grid"),
				)
				.accessibility_label("Volume")
				.class("w-full h-9"),
			)
			.child(
				LinearProgressIndicator(Progress::Determinate(
					ProgressValue::new(0.6).expect("valid progress"),
				))
				.accessibility_label("Completion")
				.class("w-full h-3"),
			)
			.child(
				ActivityIndicator(state.running)
					.accessibility_label("Loading")
					.class("w-8 h-8"),
			)
			.child(
				View()
					.class("flex-row gap-1")
					.child(
						Image(&state.image)
							.accessibility_label("Local image")
							.class("w-10 h-10 rounded-lg"),
					)
					.children(vectors(state)),
			)
			.child(
				ScrollView()
					.accessibility_label("Scroll region")
					.class("w-full h-24 rounded-lg border border-slate-300")
					.children((0..12).map(|index| {
						Text(format!("Native scroll row {index}")).class("h-8 shrink-0")
					})),
			)
			.child(
				Pressable("Rich action")
					.class("w-48 h-10 bg-blue-600 rounded-lg")
					.child(Text("Native pressable").class("text-white text-center")),
			);

	if !state.removed {
		root = root.child(
			Button("Remove this control")
				.accessibility_label("Remove")
				.class("w-48 h-10"),
		);
	}

	root
}

/// A labelled, filled square fitted into a wide frame, a decorative stroked
/// line stretched across one, and a square holding an even-odd ring over a
/// dashed line. The verifier reads their pixels: the fit square is centred,
/// the stretched stroke keeps its layout-unit width, the ring's inner square
/// is a hole, and the dashes scale with the drawing from their offset.
fn vectors(state: &State) -> [Element; 3] {
	let unit = ViewBox::new(0.0, 0.0, 10.0, 10.0).expect("valid fixture view box");
	let fill = if state.vector_recolored {
		Color::rgba(37, 99, 235, 255)
	} else {
		Color::rgba(220, 38, 38, 255)
	};

	[
		Vector(unit)
			.accessibility_label("Filled vector")
			.class("w-20 h-10")
			.path(
				VectorPath::new(
					PathData::parse("M0 0 H10 V10 H0 Z").expect("valid fixture square"),
				)
				.fill(fill),
			),
		Vector(unit)
			.scaling(VectorScaling::Stretch)
			.class("w-20 h-10")
			.path(
				VectorPath::new(PathData::parse("M0 5 H10").expect("valid fixture line")).stroke(
					Stroke::new(Color::rgba(22, 163, 74, 255), 4.0).expect("valid fixture stroke"),
				),
			),
		// Fitted 2 layout units per drawing unit into its 40 by 40 frame.
		Vector(ViewBox::new(0.0, 0.0, 20.0, 20.0).expect("valid fixture view box"))
			.accessibility_label("Path rules vector")
			.class("w-10 h-10")
			.path(
				// Both squares wind the same way, so only the even-odd rule
				// leaves the inner one empty.
				VectorPath::new(
					PathData::parse("M0 0 H10 V10 H0 Z M3 3 H7 V7 H3 Z")
						.expect("valid fixture ring"),
				)
				.fill(Color::rgba(220, 38, 38, 255))
				.fill_rule(FillRule::Evenodd),
			)
			.path(
				// 8 on, 4 off, starting 4 into the pattern, in layout units:
				// dashes at 0-4, 8-16, 20-28, and 32-40.
				VectorPath::new(PathData::parse("M0 15 H20").expect("valid fixture line")).stroke(
					Stroke::new(Color::rgba(22, 163, 74, 255), 2.0)
						.expect("valid fixture stroke")
						.scaling(StrokeScaling::Drawing)
						.dashes(&[4.0, 2.0], 2.0)
						.expect("valid fixture dashes"),
				),
			),
	]
}

// SAFETY: Each exported entry point matches ProbeActivity's external
// declaration and is invoked on its UI thread. No local Java reference outlives
// a call.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_widgets_ProbeActivity_nativeMount(
	mut env: JNIEnv<'_>,
	_class: JClass<'_>,
	host: JObject<'_>,
	width: jfloat,
	height: jfloat,
	image: JString<'_>,
) -> jlong {
	std::panic::set_hook(Box::new(|info| {
		let message = CString::new(info.to_string()).expect("panic message contains no nulls");
		// SAFETY: Both pointers are valid NUL-terminated strings for this call.
		unsafe {
			__android_log_write(6, c"LeptoskitWidgets".as_ptr(), message.as_ptr());
		}
	}));
	let image: String = env.get_string(&image).expect("fixture image path").into();
	let backend = AndroidBackend::new(&mut env, &host).expect("fixture Android host");
	let measurer = backend.text_measurer();
	let worker_size = std::thread::spawn(move || {
		measurer.measure(
			"Worker العربية हिन्दी",
			leptoskit_native::TextStyle::default(),
			leptoskit_yoga::Size::new(
				leptoskit_yoga::AvailableSpace::Definite(120.0),
				leptoskit_yoga::AvailableSpace::MaxContent,
			),
		)
	})
	.join()
	.expect("worker text measurement completes");
	assert!(worker_size.width > 0.0 && worker_size.height > 0.0);
	let surface = Rc::new(RefCell::new(NativeSurface::new(backend)));
	let state = Arc::new(Mutex::new(State {
		shared_paint: true,
		running: true,
		enabled: true,
		slider: 1e100,
		value: "kept".into(),
		image,
		..State::default()
	}));
	surface.borrow_mut().set_dark_mode(false);
	surface.borrow_mut().set_viewport(width, height);
	surface
		.borrow_mut()
		.set_root(&fixture(&state.lock().expect("fixture state")));
	let weak = Rc::downgrade(&surface);
	let data = Arc::clone(&state);
	surface
		.borrow()
		.backend()
		.set_event_listener(move |id, event| {
			let Some(surface) = weak.upgrade() else {
				return;
			};
			let label = surface.borrow().props(id).accessibility_label.clone();
			{
				let mut state = data.lock().expect("fixture state");
				match event {
					NativeEvent::Press => {
						state.events[0] += 1;
						if label.as_deref() == Some("Remove") {
							state.removed = true;
						}
					}
					NativeEvent::Change { value } => {
						state.events[1] += 1;
						if value == "remove-me" && label.as_deref() == Some("Editor") {
							state.editor_removed = true;
						} else if state.accept && label.as_deref() == Some("Editor") {
							state.value.clone_from(value);
						}
					}
					NativeEvent::Submit { .. } => state.events[2] += 1,
					NativeEvent::Focus => state.events[3] += 1,
					NativeEvent::Blur => state.events[4] += 1,
					NativeEvent::Toggle { checked } => {
						state.events[5] += 1;
						if state.accept {
							if label.as_deref() == Some("Compact") {
								state.radio = true;
							} else if label.as_deref() == Some("Comfortable") {
								state.radio = false;
							} else {
								state.checked = *checked;
							}
						}
					}
					NativeEvent::ValueChange { value } => {
						state.events[6] += 1;
						if state.accept {
							state.slider = *value;
						}
					}
					NativeEvent::Scroll { .. } => state.events[7] += 1,
					NativeEvent::PressIn => state.events[8] += 1,
					NativeEvent::PressOut => state.events[9] += 1,
					// Android widgets deliver no key presses yet.
					NativeEvent::Key(_) => {}
				}
			}
			match event {
				NativeEvent::PressIn => return surface.borrow_mut().press_in(id),
				NativeEvent::PressOut => return surface.borrow_mut().press_out(id),
				_ => {}
			}

			surface.borrow().dispatch_event(id, event);
			let element = fixture(&data.lock().expect("fixture state"));
			surface.borrow_mut().set_root(&element);
		});
	let root = surface.borrow().root().expect("mounted fixture").index() as i64;
	PROBE.with(|probe| *probe.borrow_mut() = Some(Probe { surface, state }));
	root
}

#[link(name = "log")]
unsafe extern "C" {
	fn __android_log_write(
		priority: i32,
		tag: *const std::ffi::c_char,
		text: *const std::ffi::c_char,
	) -> i32;
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_widgets_ProbeActivity_nativeConfigure(
	_env: JNIEnv<'_>,
	_class: JClass<'_>,
	mode: jint,
) {
	PROBE.with(|probe| {
		let probe = probe.borrow();
		let probe = probe.as_ref().expect("mounted fixture");
		let mut state = probe.state.lock().expect("fixture state");
		match mode {
			0 => state.accept = false,
			1 => state.accept = true,
			2 => state.enabled = false,
			3 => state.enabled = true,
			4 => state.hidden = true,
			5 => state.hidden = false,
			6 => probe.surface.borrow_mut().set_dark_mode(true),
			7 => probe.surface.borrow_mut().set_dark_mode(false),
			8 => state.shared_paint = false,
			9 => state.running = false,
			10 => state.running = true,
			11 => state.button_alignment = Some(TextAlign::Start),
			12 => state.button_alignment = Some(TextAlign::End),
			13 => state.button_alignment = None,
			14 => state.root_fixed = true,
			15 => state.root_fixed = false,
			16 => state.editor_removed = false,
			17 => {
				// The Yoga arena root has no parent positioning its margins.
				// Exercise the backend's independent root-origin mutation
				// contract.
				let mut surface = probe.surface.borrow_mut();
				let root = surface.root().expect("mounted fixture");
				surface
					.backend_mut()
					.update_layout(root, 12.0, 12.0, 288.0, 800.0);
				return;
			}
			18 => state.vector_recolored = true,
			_ => panic!("unknown fixture mode"),
		}

		probe.surface.borrow_mut().set_root(&fixture(&state));
	});
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_widgets_ProbeActivity_nativeResize(
	_env: JNIEnv<'_>,
	_class: JClass<'_>,
	width: jfloat,
	height: jfloat,
) {
	PROBE.with(|probe| {
		let probe = probe.borrow();
		probe
			.as_ref()
			.expect("mounted fixture")
			.surface
			.borrow_mut()
			.set_viewport(width, height);
	});
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_widgets_ProbeActivity_nativeStatus(
	env: JNIEnv<'_>,
	_class: JClass<'_>,
) -> jstring {
	let status = PROBE.with(|probe| {
		let probe = probe.borrow();
		let state = probe.as_ref().expect("mounted fixture").state.lock().expect("fixture state");
		serde_json::json!({ "events": state.events, "checked": state.checked, "radio": state.radio, "slider": state.slider, "removed": state.removed }).to_string()
	});
	env.new_string(status).expect("fixture status").into_raw()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_widgets_ProbeActivity_nativeDestroy(
	_env: JNIEnv<'_>,
	_class: JClass<'_>,
) {
	PROBE.with(|probe| {
		probe.borrow_mut().take();
	});
}

fn contract_button(text: &str) -> NativeProps {
	NativeProps {
		text: Some(text.into()),
		accessibility_label: Some("Contract".into()),
		..NativeProps::default()
	}
}

/// Builds a root view holding one button on `host` through `create`,
/// `insert_child` and `update_layout`, and returns the node ids, whether
/// `AndroidBackend::view` resolves the button, and the backend's debug form.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_widgets_ProbeActivity_nativeContractMount(
	mut env: JNIEnv<'_>,
	_class: JClass<'_>,
	host: JObject<'_>,
) -> jstring {
	let mut backend = AndroidBackend::new(&mut env, &host).expect("contract Android host");
	let mut arena = Yoga::new();
	let root = arena.create_node();
	let button = arena.create_node();
	let rive = arena.create_node();
	backend.create(root, NativeKind::View, &NativeProps::default());
	backend.create(button, NativeKind::Button, &contract_button("Contract"));
	backend.insert_child(root, button, 0);
	backend.update_layout(root, 0.0, 0.0, 200.0, 80.0);
	backend.update_layout(button, 10.0, 10.0, 120.0, 40.0);
	backend.create(rive, NativeKind::Rive, &NativeProps::default());
	backend.update_layout(rive, 0.0, 0.0, 2.0, 1.0);
	assert!(
		!backend.rive_visible(rive),
		"an unattached host must not schedule Rive frames"
	);
	let mut pixels = [128, 0, 0, 128, 0, 0, 255, 255];
	assert!(backend.present_rive(
		rive,
		&leptoskit_native::RiveFrame::new(2, 1, &pixels).unwrap()
	));
	pixels.fill(0);
	let presses = Rc::new(Cell::new(0));
	let counter = Rc::clone(&presses);
	backend.set_event_listener(move |_, event| {
		if matches!(event, NativeEvent::Press) {
			counter.set(counter.get() + 1);
		}
	});
	let view = backend
		.view(&mut env, button)
		.expect("contract button view");
	let status = serde_json::json!({
		"root": root.index(),
		"button": button.index(),
		"rive": rive.index(),
		"view": !view.is_null(),
		"debug": format!("{backend:?}"),
	})
	.to_string();
	CONTRACT.with(|contract| {
		*contract.borrow_mut() = Some(Contract {
			backend,
			root,
			button,
			rive,
			presses,
		});
	});
	env.new_string(status).expect("contract status").into_raw()
}

/// Applies one contract step and returns the presses the listener has seen.
/// Step 0 only reports; dropping the backend (step 5) closes its host.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_widgets_ProbeActivity_nativeContractStep(
	_env: JNIEnv<'_>,
	_class: JClass<'_>,
	step: jint,
) -> jint {
	CONTRACT.with(|contract| {
		let mut contract = contract.borrow_mut();
		if step == 5 {
			contract.take().expect("mounted contract");
			return 0;
		}

		let Contract {
			backend,
			root,
			button,
			rive,
			presses,
		} = contract.as_mut().expect("mounted contract");
		match step {
			0 => {}
			1 => backend.update_props(*button, &contract_button("Updated")),
			2 => backend.clear_event_listener(),
			3 => backend.remove_child(*root, *button),
			4 => backend.delete(*button),
			6 => backend.update_props(*rive, &NativeProps::default()),
			7 => {
				let pixels = [0, 128, 0, 128, 255, 0, 0, 255];
				assert!(backend.present_rive(
					*rive,
					&leptoskit_native::RiveFrame::new(2, 1, &pixels).unwrap()
				));
			}
			8 => backend.delete(*rive),
			_ => panic!("unknown contract step"),
		}

		jint::try_from(presses.get()).expect("press count fits a jint")
	})
}

/// Writes the LLVM coverage profile to `path`. The probe Activity calls this
/// just before reporting its verdict, because the verifier stops the app
/// rather than letting it exit. Builds without the `coverage` feature ignore
/// the request.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_leptoskit_widgets_ProbeActivity_nativeFlushCoverage(
	mut env: JNIEnv<'_>,
	_class: JClass<'_>,
	path: JString<'_>,
) {
	#[cfg(feature = "coverage")]
	{
		unsafe extern "C" {
			fn __llvm_profile_set_filename(name: *const std::ffi::c_char);
			fn __llvm_profile_write_file() -> i32;
		}
		let path: String = env.get_string(&path).expect("coverage path").into();
		let path = CString::new(path).expect("coverage path has no NUL");
		// SAFETY: `-C instrument-coverage` links the profiler runtime, which
		// copies the filename; `path` outlives both calls.
		let status = unsafe {
			__llvm_profile_set_filename(path.as_ptr());
			__llvm_profile_write_file()
		};
		assert_eq!(status, 0, "write the LLVM coverage profile");
	}
	#[cfg(not(feature = "coverage"))]
	let _ = (&mut env, path);
}
