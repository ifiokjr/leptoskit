fn main() {
	let mut args = libtest_mimic::Arguments::from_args();
	args.test_threads = Some(1);
	#[cfg(target_os = "linux")]
	let trials = linux::trials();
	#[cfg(not(target_os = "linux"))]
	let trials = Vec::new();
	libtest_mimic::run(&args, trials).exit();
}

#[cfg(target_os = "linux")]
mod linux {
	use std::cell::RefCell;
	use std::process::Command;
	use std::rc::Rc;
	use std::time::Duration;

	use gtk::prelude::*;
	use leptoskit_gtk::GtkBackend;
	use leptoskit_gtk::GtkTextMeasurer;
	use leptoskit_gtk::gtk;
	use leptoskit_native::Element;
	use leptoskit_native::NativeBackend;
	use leptoskit_native::NativeEvent;
	use leptoskit_native::NativeKind;
	use leptoskit_native::NativeSurface;
	use leptoskit_native::TextMeasurer;
	use leptoskit_native::TextStyle;
	use leptoskit_native::components::*;
	use leptoskit_yoga::AvailableSpace;
	use leptoskit_yoga::NodeId;
	use leptoskit_yoga::Size;
	use libtest_mimic::Trial;

	pub fn trials() -> Vec<Trial> {
		gtk::init().expect("GTK needs a display; run under dbus-run-session and xvfb-run");
		let texture = gtk::gdk::MemoryTexture::new(
			2,
			2,
			gtk::gdk::MemoryFormat::R8g8b8a8,
			&gtk::glib::Bytes::from_owned(vec![
				255, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 255,
			]),
			8,
		);
		std::fs::write(artifact_path("image.png"), texture.save_to_png_bytes()).unwrap();
		[
			(
				"all_widgets_have_exact_yoga_frames",
				all_widgets_have_exact_yoga_frames as fn(),
			),
			(
				"controlled_actions_and_reentrant_removal",
				controlled_actions_and_reentrant_removal,
			),
			("radio_and_slider_keyboard", radio_and_slider_keyboard),
			(
				"empty_host_binds_late_and_replaced_roots",
				empty_host_binds_late_and_replaced_roots,
			),
			(
				"default_switch_size_contains_native_chrome",
				default_switch_size_contains_native_chrome,
			),
			(
				"mount_notifications_do_not_reenter_surface_borrows",
				mount_notifications_do_not_reenter_surface_borrows,
			),
			(
				"keyboard_callback_keeps_application_focus",
				keyboard_callback_keeps_application_focus,
			),
			("focus_events_bubble_once", focus_events_bubble_once),
			(
				"button_alignment_positions_native_label",
				button_alignment_positions_native_label,
			),
			(
				"rtl_text_and_inputs_follow_resolved_direction",
				rtl_text_and_inputs_follow_resolved_direction,
			),
			(
				"native_order_matches_paint_pick_and_tab",
				native_order_matches_paint_pick_and_tab,
			),
			(
				"slider_extremes_and_coarse_wheel",
				slider_extremes_and_coarse_wheel,
			),
			(
				"resize_and_root_replacement_are_hosted",
				resize_and_root_replacement_are_hosted,
			),
			(
				"readable_native_widget_catalog",
				readable_native_widget_catalog,
			),
			(
				"atspi_values_and_decorative_children",
				atspi_values_and_decorative_children,
			),
			(
				"root_frames_respect_fixed_and_zero_dimensions",
				root_frames_respect_fixed_and_zero_dimensions,
			),
			(
				"rounded_picking_and_input_paint",
				rounded_picking_and_input_paint,
			),
			(
				"password_editing_focus_and_removal",
				password_editing_focus_and_removal,
			),
			(
				"scroll_and_disabled_hidden_ancestors",
				scroll_and_disabled_hidden_ancestors,
			),
			(
				"pango_metrics_and_worker_measurement",
				pango_metrics_and_worker_measurement,
			),
			(
				"paint_animation_and_pressable",
				paint_animation_and_pressable,
			),
			(
				"overflow_visible_and_hidden_have_distinct_pixels",
				overflow_visible_and_hidden_have_distinct_pixels,
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

	fn artifact_path(name: &str) -> std::path::PathBuf {
		let directory = std::env::var_os("LEPTOSKIT_GTK_ARTIFACTS")
			.map_or_else(
				|| {
					std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
						.join("../../target/native-widgets/gtk")
				},
				std::path::PathBuf::from,
			)
			.join(std::process::id().to_string());
		std::fs::create_dir_all(&directory).expect("create native test artifact directory");
		directory.join(name)
	}

	fn pump() {
		let context = gtk::glib::MainContext::default();

		for _ in 0..20 {
			while context.pending() {
				context.iteration(false);
			}
			std::thread::sleep(Duration::from_millis(5));
		}
	}

	/// X input is delivered asynchronously; wait for its native effect
	/// instead of assuming one pump suffices on a loaded display server.
	fn pump_until(condition: impl Fn() -> bool) {
		let deadline = std::time::Instant::now() + Duration::from_secs(5);

		while !condition() {
			assert!(
				std::time::Instant::now() < deadline,
				"native input had no effect within five seconds"
			);
			pump();
		}
	}

	struct Mounted {
		surface: Rc<RefCell<NativeSurface<GtkBackend>>>,
		window: gtk::Window,
	}

	impl Drop for Mounted {
		fn drop(&mut self) {
			self.surface.borrow().backend().clear_event_listener();
			self.window.destroy();
			pump();
		}
	}

	fn mounted(tree: &Element) -> Mounted {
		let mut surface = NativeSurface::new(GtkBackend::new());
		surface.set_dark_mode(false);
		surface.set_viewport(480.0, 640.0);
		surface.set_root(tree);
		let window = surface
			.backend()
			.open_window(surface.root().unwrap(), "Leptoskit GTK Tests", 480, 640)
			.unwrap();
		pump();
		Mounted {
			surface: Rc::new(RefCell::new(surface)),
			window,
		}
	}

	fn child(mounted: &Mounted, index: usize) -> NodeId {
		let surface = mounted.surface.borrow();
		surface.children(surface.root().unwrap())[index]
	}

	fn native<T: IsA<gtk::Widget> + gtk::glib::object::IsClass>(
		mounted: &Mounted,
		id: NodeId,
	) -> T {
		mounted
			.surface
			.borrow()
			.backend()
			.control(id)
			.unwrap()
			.downcast::<T>()
			.expect("native widget type")
	}

	fn key(name: &str) {
		assert!(
			Command::new("xdotool")
				.args(["key", "--clearmodifiers", name])
				.status()
				.expect("xdotool installed")
				.success()
		);
		pump();
	}

	fn click(x: i32, y: i32) {
		let output = Command::new("xdotool")
			.args(["search", "--name", "^Leptoskit GTK Tests$"])
			.output()
			.unwrap();
		assert!(output.status.success());
		let window = String::from_utf8(output.stdout).unwrap();
		let window = window.lines().last().unwrap();
		assert!(
			Command::new("xdotool")
				.args([
					"mousemove",
					"--window",
					window,
					&x.to_string(),
					&y.to_string(),
					"click",
					"1"
				])
				.status()
				.unwrap()
				.success()
		);
		pump();
	}

	fn all_widgets_have_exact_yoga_frames() {
		let tree = View().class("w-full h-full bg-white gap-2 p-4").children([
			Text("GTK native primitives"),
			Button("A button with a very long label").class("w-1 h-1"),
			Switch(false).class("w-1 h-1"),
			Checkbox(false).class("w-1 h-1"),
			Radio(RadioGroup::new("all").unwrap(), false).class("w-1 h-1"),
			Slider(SliderState::new(0.0, 100.0, 40.0, Some(5.0)).unwrap()).class("w-1 h-1"),
			TextInput().class("w-1 h-1"),
			PasswordInput().class("w-1 h-1"),
			LinearProgressIndicator(Progress::Determinate(ProgressValue::new(0.4).unwrap()))
				.class("h-2"),
			ActivityIndicator(true).class("w-6 h-6"),
			Pressable("Add")
				.class("h-10 w-32 bg-blue-600 rounded-lg")
				.child(Text("Add preview").class("text-white")),
			Image(artifact_path("image.png").to_string_lossy()).class("h-4 w-4"),
			ScrollView()
				.class("h-10")
				.child(Text("Scrollable document")),
			Element::new(NativeKind::TextRun)
				.props(leptoskit_native::NativeProps::text("run"))
				.class("h-5"),
		]);
		let mounted = mounted(&tree);
		let surface = mounted.surface.borrow();
		let root = surface.root().unwrap();

		for id in surface.children(root) {
			let view = surface.backend().view(*id).unwrap();
			let layout = surface.layout(*id);
			let bounds = view
				.compute_bounds(&surface.backend().view(root).unwrap())
				.unwrap();
			assert_eq!(
				(view.width(), view.height()),
				(layout.width.round() as i32, layout.height.round() as i32),
				"{} allocation",
				surface.kind(*id)
			);
			assert!((bounds.x() - layout.x).abs() <= 0.5);
			assert!((bounds.y() - layout.y).abs() <= 0.5);
		}
		for index in 1..8 {
			let view = surface
				.backend()
				.view(surface.children(root)[index])
				.unwrap();
			assert_eq!(
				(view.width(), view.height()),
				(4, 4),
				"native minimum must not enlarge Yoga"
			);
		}
		std::fs::write(
			artifact_path("primitives.png"),
			surface.backend().snapshot_png(root).unwrap(),
		)
		.unwrap();
	}

	fn controlled_actions_and_reentrant_removal() {
		let mounted = mounted(&View().class("gap-4 p-4").children([
			Switch(false).class("w-16 h-8"),
			Checkbox(false).class("w-8 h-8"),
			Button("Press").class("h-10"),
		]));
		let switch_id = child(&mounted, 0);
		let checkbox_id = child(&mounted, 1);
		let switch = native::<gtk::Switch>(&mounted, switch_id);
		let checkbox = native::<gtk::CheckButton>(&mounted, checkbox_id);
		let events = Rc::new(RefCell::new(Vec::new()));
		mounted.surface.borrow().backend().set_event_listener({
			let events = events.clone();
			move |id, event| events.borrow_mut().push((id, event))
		});
		click(36, 30);
		assert!(!switch.is_active());
		assert!(
			events
				.borrow()
				.contains(&(switch_id, NativeEvent::Toggle { checked: true }))
		);
		checkbox.set_active(true);
		assert!(!checkbox.is_active());
		assert!(
			events
				.borrow()
				.contains(&(checkbox_id, NativeEvent::Toggle { checked: true }))
		);
		let weak = Rc::downgrade(&mounted.surface);
		mounted
			.surface
			.borrow()
			.backend()
			.set_event_listener(move |id, event| {
				if event == (NativeEvent::Toggle { checked: true }) {
					let surface = weak.upgrade().unwrap();
					let mut surface = surface.borrow_mut();
					let mut props = surface.props(id).clone();
					props.checked = Some(true);
					surface.backend_mut().update_props(id, &props);
				}
			});
		checkbox.set_active(true);
		assert!(checkbox.is_active());
		assert_eq!(native::<gtk::CheckButton>(&mounted, checkbox_id), checkbox);
		let weak = Rc::downgrade(&mounted.surface);
		mounted
			.surface
			.borrow()
			.backend()
			.set_event_listener(move |_, event| {
				if event == NativeEvent::Press {
					weak.upgrade().unwrap().borrow_mut().set_root(&View());
				}
			});
		let button = native::<gtk::Button>(&mounted, child(&mounted, 2));
		button.emit_clicked();
		assert!(!button.is_sensitive());
	}

	fn radio_and_slider_keyboard() {
		let group = RadioGroup::new("arrows").unwrap();
		let mounted = mounted(&View().class("p-4 gap-2").children([
			Radio(group.clone(), true).class("w-8 h-8"),
			View().child(Radio(group.clone(), false).enabled(false).class("w-8 h-8")),
			View().child(Radio(group, false).class("w-8 h-8")),
			Slider(SliderState::new(-10.0, 10.0, 0.0, Some(2.0)).unwrap()).class("w-64 h-8"),
		]));
		let first = child(&mounted, 0);
		let third = mounted.surface.borrow().children(child(&mounted, 2))[0];
		let slider_id = child(&mounted, 3);
		let events = Rc::new(RefCell::new(Vec::new()));
		mounted.surface.borrow().backend().set_event_listener({
			let events = events.clone();
			move |id, event| events.borrow_mut().push((id, event))
		});
		let radio = native::<gtk::CheckButton>(&mounted, first);
		assert_eq!(radio.accessible_role(), gtk::AccessibleRole::Radio);
		let other = native::<gtk::CheckButton>(&mounted, third);
		other.set_active(true);
		assert!(
			radio.is_active(),
			"rejected native activation restores prior selected sibling"
		);
		assert!(!other.is_active());
		radio.grab_focus();
		pump();
		key("Right");
		assert!(
			events
				.borrow()
				.contains(&(third, NativeEvent::Toggle { checked: true }))
		);
		let scale = native::<gtk::Scale>(&mounted, slider_id);
		scale.grab_focus();
		pump();
		key("Right");
		assert!(
			events
				.borrow()
				.contains(&(slider_id, NativeEvent::ValueChange { value: 2.0 }))
		);
		assert!((scale.value() - 0.5).abs() < f64::EPSILON);
		key("End");
		assert!(
			events
				.borrow()
				.contains(&(slider_id, NativeEvent::ValueChange { value: 10.0 }))
		);
		for index in 0..20 {
			let mut props = mounted.surface.borrow().props(third).clone();
			props.radio_group = Some(
				RadioGroup::new(if index % 2 == 0 {
					"replacement group"
				} else {
					"arrows"
				})
				.unwrap(),
			);
			mounted
				.surface
				.borrow_mut()
				.backend_mut()
				.update_props(third, &props);
			assert!(radio.is_active());
		}
	}

	fn empty_host_binds_late_and_replaced_roots() {
		let mut surface = NativeSurface::new(GtkBackend::new());
		surface.set_viewport(480.0, 640.0);
		let window = surface
			.backend()
			.create_window("Leptoskit GTK Tests", 480, 640);
		pump();
		assert!(surface.root().is_none());
		surface.set_root(&Button("Late guest root").class("w-40 h-10"));
		pump();
		let root = surface.root().unwrap();
		let first = surface.backend().view(root).unwrap();
		assert!(first.is_mapped());
		assert_eq!((first.width(), first.height()), (160, 40));
		surface.set_root(&View().class("w-20 h-20 bg-blue-600"));
		pump();
		let root = surface.root().unwrap();
		assert!(first.parent().is_none());
		assert!(surface.backend().view(root).unwrap().is_mapped());
		window.destroy();
		pump();
	}

	fn default_switch_size_contains_native_chrome() {
		let mounted = mounted(
			&View()
				.class("w-full h-full p-4 gap-4 bg-white")
				.children([Switch(false), Switch(true)]),
		);
		for index in 0..2 {
			let control = native::<gtk::Switch>(&mounted, child(&mounted, index));
			let view = mounted
				.surface
				.borrow()
				.backend()
				.view(child(&mounted, index))
				.unwrap();
			assert_eq!((view.width(), view.height()), (40, 24));
			let bounds = control.compute_bounds(&view).unwrap();
			assert!(
				bounds.width() <= 40.0 && bounds.height() <= 24.0,
				"normal shared switch size contains native chrome: {bounds:?}"
			);
		}
		let events = Rc::new(RefCell::new(Vec::new()));
		mounted.surface.borrow().backend().set_event_listener({
			let events = events.clone();
			move |id, event| events.borrow_mut().push((id, event))
		});
		let toggled = |index, checked| {
			events
				.borrow()
				.contains(&(child(&mounted, index), NativeEvent::Toggle { checked }))
		};
		click(36, 28);
		pump_until(|| toggled(0, true));
		click(36, 68);
		pump_until(|| toggled(1, false));
		let surface = mounted.surface.borrow();
		std::fs::write(
			artifact_path("switch-defaults.png"),
			surface
				.backend()
				.snapshot_png(surface.root().unwrap())
				.unwrap(),
		)
		.unwrap();
	}

	fn mount_notifications_do_not_reenter_surface_borrows() {
		let mut surface = NativeSurface::new(GtkBackend::new());
		surface.set_viewport(480.0, 640.0);
		surface.set_root(&View().class("gap-4 p-4").children([
			TextInput().class("h-10"),
			ActivityIndicator(true).class("w-8 h-8"),
		]));
		let surface = Rc::new(RefCell::new(surface));
		let weak = Rc::downgrade(&surface);
		let callbacks = Rc::new(std::cell::Cell::new(0));
		surface.borrow().backend().set_event_listener({
			let callbacks = callbacks.clone();
			move |_, _| {
				let surface = weak.upgrade().unwrap();
				let _borrow = surface.borrow_mut();
				callbacks.set(callbacks.get() + 1);
			}
		});
		let window = {
			let surface = surface.borrow();
			surface
				.backend()
				.open_window(surface.root().unwrap(), "Leptoskit GTK Tests", 480, 640)
				.unwrap()
		};
		assert_eq!(
			callbacks.get(),
			0,
			"initial platform focus is a mount notification"
		);
		pump();
		let root = surface.borrow().root().unwrap();
		let spinner = surface
			.borrow()
			.backend()
			.control(surface.borrow().children(root)[1])
			.unwrap()
			.downcast::<gtk::Spinner>()
			.unwrap();
		assert!(
			spinner.is_spinning(),
			"mount suppression still starts mapped animations"
		);
		surface.borrow_mut().set_dark_mode(true);
		surface.borrow_mut().set_dark_mode(false);
		surface.borrow().backend().clear_event_listener();
		window.destroy();
		pump();
	}

	fn keyboard_callback_keeps_application_focus() {
		let mounted = mounted(&View().class("p-4 gap-4").children([
			Slider(SliderState::default()).class("w-64 h-8"),
			TextInput().class("h-10"),
		]));
		let editor = native::<gtk::Entry>(&mounted, child(&mounted, 1));
		mounted.surface.borrow().backend().set_event_listener({
			let editor = editor.clone();
			move |_, event| {
				if matches!(event, NativeEvent::ValueChange { .. }) {
					editor.grab_focus();
				}
			}
		});
		native::<gtk::Scale>(&mounted, child(&mounted, 0)).grab_focus();
		pump();
		key("Right");
		assert!(
			GtkWindowExt::focus(&mounted.window)
				.is_some_and(|focus| focus == editor || focus.is_ancestor(&editor)),
			"native callback's chosen focus is retained"
		);
	}

	fn password_editing_focus_and_removal() {
		let mounted = mounted(
			&View().class("p-4 gap-2").children([
				PasswordInput()
					.value("demo")
					.placeholder("Password")
					.class("h-10"),
				TextInput().value("hello").class("h-10"),
			]),
		);
		let id = child(&mounted, 0);
		let entry = native::<gtk::Entry>(&mounted, id);
		assert!(!entry.property::<bool>("visibility"));
		assert_eq!(entry.input_purpose(), gtk::InputPurpose::Password);
		let events = Rc::new(RefCell::new(Vec::new()));
		mounted.surface.borrow().backend().set_event_listener({
			let events = events.clone();
			move |id, event| events.borrow_mut().push((id, event))
		});
		entry.grab_focus();
		pump();
		key("End");
		key("x");
		assert_eq!(entry.text(), "demo");
		assert!(events.borrow().contains(&(
			id,
			NativeEvent::Change {
				value: "demox".into()
			}
		)));
		key("Return");
		assert!(events.borrow().contains(&(
			id,
			NativeEvent::Submit {
				value: "demo".into()
			}
		)));
		mounted.surface.borrow_mut().set_dark_mode(true);
		assert!(
			GtkWindowExt::focus(&mounted.window)
				.is_some_and(|focus| focus == entry || focus.is_ancestor(&entry)),
			"native editor focus survives appearance update"
		);
		assert_eq!(entry.text(), "demo");
		let weak = Rc::downgrade(&mounted.surface);
		mounted
			.surface
			.borrow()
			.backend()
			.set_event_listener(move |_, event| {
				if matches!(event, NativeEvent::Change { .. }) {
					weak.upgrade()
						.unwrap()
						.borrow_mut()
						.set_root(&View().child(Button("Replaced")));
				}
			});
		key("x");
		assert_eq!(
			mounted.surface.borrow().kind(child(&mounted, 0)),
			NativeKind::Button
		);
		assert!(!entry.is_sensitive());
	}

	fn scroll_and_disabled_hidden_ancestors() {
		let mounted = mounted(
			&View().class("gap-4").children([
				ScrollView()
					.class("w-40 h-20")
					.child(View().class("w-80 h-96 bg-blue-600")),
				View()
					.props(leptoskit_native::NativeProps {
						hidden: Some(true),
						..Default::default()
					})
					.child(Checkbox(false).class("w-8 h-8")),
				View()
					.enabled(false)
					.child(Button("Disabled").class("w-32 h-10")),
			]),
		);
		let scroll_id = child(&mounted, 0);
		let scroll = native::<gtk::ScrolledWindow>(&mounted, scroll_id);
		assert!(scroll.hadjustment().upper() >= 320.0);
		assert!(scroll.vadjustment().upper() >= 384.0);
		let events = Rc::new(RefCell::new(Vec::new()));
		mounted.surface.borrow().backend().set_event_listener({
			let events = events.clone();
			move |id, event| events.borrow_mut().push((id, event))
		});
		scroll.vadjustment().set_value(40.0);
		pump();
		assert!(events.borrow().iter().any(|(id, event)| *id == scroll_id && matches!(event, NativeEvent::Scroll { offset_y, .. } if (*offset_y - 40.0).abs() < f32::EPSILON)));
		let hidden = mounted.surface.borrow().children(child(&mounted, 1))[0];
		native::<gtk::CheckButton>(&mounted, hidden).set_active(true);
		assert!(!events.borrow().iter().any(|(id, _)| *id == hidden));
		let disabled = mounted.surface.borrow().children(child(&mounted, 2))[0];
		let button = native::<gtk::Button>(&mounted, disabled);
		button.emit_clicked();
		assert!(!button.is_sensitive());
		assert!(!events.borrow().iter().any(|(id, _)| *id == disabled));
	}

	fn pango_metrics_and_worker_measurement() {
		let mounted = mounted(
			&View()
				.class("w-40")
				.child(Text("Wrapping longestwordwithoutspaces correctly").class("text-xl")),
		);
		let id = child(&mounted, 0);
		let label = native::<gtk::Label>(&mounted, id);
		let surface = mounted.surface.borrow();
		let layout = surface.layout(id);
		let measured = GtkTextMeasurer.measure(
			label.text().as_str(),
			surface.props(id).text_style(),
			Size::new(
				AvailableSpace::Definite(layout.width),
				AvailableSpace::MaxContent,
			),
		);
		assert_eq!(
			label.layout().pixel_size(),
			(measured.width as i32, measured.height as i32)
		);
		let result = std::thread::spawn(|| {
			GtkTextMeasurer.measure(
				"Worker fonts",
				TextStyle::default(),
				Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
			)
		})
		.join()
		.unwrap();
		assert!(result.width > 0.0 && result.height > 0.0);
	}

	fn paint_animation_and_pressable() {
		let mounted = mounted(
			&View().class("w-full h-full bg-white p-4 gap-4").children([
				View()
					.class(
						"w-20 h-20 rounded-xl overflow-hidden border-4 border-blue-600 bg-red-500 \
						 opacity-50",
					)
					.child(View().class("w-40 h-40 bg-green-500")),
				ActivityIndicator(true).class("w-8 h-8"),
				LinearProgressIndicator(Progress::Indeterminate).class("w-64 h-4"),
				Pressable("Decorative action")
					.class("w-48 h-12 bg-blue-600 rounded-xl")
					.child(Text("Activate").class("text-white text-lg")),
				Image(
					gtk::gio::File::for_path(artifact_path("image.png"))
						.uri()
						.to_string(),
				)
				.class("w-8 h-8"),
			]),
		);
		let spinner_id = child(&mounted, 1);
		let spinner = native::<gtk::Spinner>(&mounted, spinner_id);
		assert!(spinner.is_spinning());
		let pressable_id = child(&mounted, 3);
		let pressable = native::<gtk::Button>(&mounted, pressable_id);
		assert_eq!(pressable.accessible_role(), gtk::AccessibleRole::Button);
		let events = Rc::new(RefCell::new(Vec::new()));
		mounted.surface.borrow().backend().set_event_listener({
			let events = events.clone();
			move |id, event| events.borrow_mut().push((id, event))
		});
		pressable.grab_focus();
		pump();
		key("space");
		assert!(
			events
				.borrow()
				.contains(&(pressable_id, NativeEvent::Press))
		);
		let root = mounted.surface.borrow().root().unwrap();
		let png = mounted
			.surface
			.borrow()
			.backend()
			.snapshot_png(root)
			.unwrap();
		std::fs::write(artifact_path("paint.png"), &png).unwrap();
		assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
		let texture = gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from_owned(png)).unwrap();
		let mut pixels = vec![0; texture.width() as usize * texture.height() as usize * 4];
		texture.download(&mut pixels, texture.width() as usize * 4);
		let pixel = |x: usize, y: usize| {
			let offset = (y * texture.width() as usize + x) * 4;
			&pixels[offset..offset + 4]
		};
		// Texture.download uses native-endian Cairo ARGB32 (BGRA on Linux).
		assert_eq!(
			pixel(16, 16),
			&[255, 255, 255, 255],
			"rounded corner preserves white outside"
		);
		let green = pixel(50, 50);
		assert!(
			green[1] > green[0] && green[1] > green[2] && green[2] > 100,
			"green child is composited at half opacity: {green:?}"
		);
		assert_eq!(pixel(110, 50), &[255, 255, 255, 255], "overflow is clipped");
		let image = native::<gtk::Picture>(&mounted, child(&mounted, 4));
		assert!(
			image.paintable().is_some(),
			"local file URL decoded to native paintable"
		);
		mounted.window.set_visible(false);
		pump();
		assert!(
			!spinner.is_spinning(),
			"unmapped native windows stop animation"
		);
		mounted.window.present();
		pump();
		assert!(spinner.is_spinning());
		let mut props = mounted.surface.borrow().props(spinner_id).clone();
		props.hidden = Some(true);
		mounted
			.surface
			.borrow_mut()
			.backend_mut()
			.update_props(spinner_id, &props);
		assert!(!spinner.is_spinning());
		props.hidden = Some(false);
		props.running = Some(false);
		mounted
			.surface
			.borrow_mut()
			.backend_mut()
			.update_props(spinner_id, &props);
		assert!(!spinner.is_spinning());
		mounted.surface.borrow_mut().set_root(&View());
		assert!(!spinner.is_spinning());
	}

	fn rtl_text_and_inputs_follow_resolved_direction() {
		let rtl = leptoskit_yoga::Style {
			direction: leptoskit_yoga::Direction::Rtl,
			..Default::default()
		};
		let mounted = mounted(
			&View().class("w-full h-full p-4 gap-4 bg-white").children([
				Text("مرحبا abc\nx")
					.style(rtl.clone())
					.class("w-40 text-xl"),
				TextInput()
					.style(rtl.clone())
					.value("مرحبا abc")
					.class("w-40 h-10 bg-white text-black text-xl"),
				Slider(SliderState::new(0.0, 100.0, 50.0, Some(10.0)).unwrap())
					.style(rtl.clone())
					.class("w-40 h-8"),
				TextInput()
					.style(rtl)
					.value("abc")
					.class("w-40 h-10 bg-white text-black text-xl"),
			]),
		);
		let label = native::<gtk::Label>(&mounted, child(&mounted, 0));
		let entry = native::<gtk::Entry>(&mounted, child(&mounted, 1));
		assert_eq!(label.direction(), gtk::TextDirection::Rtl);
		assert_eq!(label.layout().alignment(), gtk::pango::Alignment::Right);
		let short_line = label.layout().index_to_pos((label.text().len() - 1) as i32);
		let right_edge =
			label.layout_offsets().0 + (short_line.x() + short_line.width()) / gtk::pango::SCALE;
		assert!(
			(label.width() - right_edge).abs() <= 2,
			"RTL short line aligns to the physical right edge: {short_line:?}"
		);
		assert_eq!(entry.direction(), gtk::TextDirection::Rtl);
		assert_eq!(label.pango_context().base_dir(), gtk::pango::Direction::Rtl);
		let surface = mounted.surface.borrow();
		let id = child(&mounted, 0);
		let measured = GtkTextMeasurer.measure(
			label.text().as_str(),
			surface.props(id).text_style(),
			Size::new(
				AvailableSpace::Definite(surface.layout(id).width),
				AvailableSpace::MaxContent,
			),
		);
		assert_eq!(
			label.layout().pixel_size(),
			(measured.width as i32, measured.height as i32)
		);
		drop(surface);
		let slider = child(&mounted, 2);
		let events = Rc::new(RefCell::new(Vec::new()));
		mounted.surface.borrow().backend().set_event_listener({
			let events = events.clone();
			move |id, event| events.borrow_mut().push((id, event))
		});
		native::<gtk::Scale>(&mounted, slider).grab_focus();
		pump();
		key("Right");
		assert!(
			events
				.borrow()
				.contains(&(slider, NativeEvent::ValueChange { value: 40.0 })),
			"RTL horizontal arrow follows native visual scale direction"
		);
		let surface = mounted.surface.borrow();
		let png = surface
			.backend()
			.snapshot_png(surface.root().unwrap())
			.unwrap();
		std::fs::write(artifact_path("rtl.png"), &png).unwrap();
		let texture = gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from_owned(png)).unwrap();
		let mut pixels = vec![0; texture.width() as usize * texture.height() as usize * 4];
		texture.download(&mut pixels, texture.width() as usize * 4);
		for index in [1, 3] {
			let layout = surface.layout(child(&mounted, index));
			let mut min_x = layout.width as usize;
			let mut max_x = 0;
			for y in 2..layout.height as usize - 2 {
				for x in 2..layout.width as usize - 2 {
					let offset = ((layout.y as usize + y) * texture.width() as usize
						+ layout.x as usize
						+ x)
						* 4;
					if pixels[offset + 3] == 255
						&& pixels[offset..offset + 3]
							.iter()
							.all(|channel| *channel < 100)
					{
						min_x = min_x.min(x);
						max_x = max_x.max(x);
					}
				}
			}
			assert!(
				min_x > 50 && max_x > 140,
				"declared RTL start positions Arabic and Latin at right: {min_x}..{max_x}"
			);
		}
	}

	fn focus_events_bubble_once() {
		use std::sync::Arc;
		use std::sync::atomic::AtomicUsize;
		use std::sync::atomic::Ordering;
		let focus = Arc::new(AtomicUsize::new(0));
		let blur = Arc::new(AtomicUsize::new(0));
		let mounted = mounted(
			&View()
				.class("p-4 gap-4")
				.on(leptoskit_native::NativeEventSlot::Focus, {
					let focus = focus.clone();
					move |_| {
						focus.fetch_add(1, Ordering::SeqCst);
						leptoskit_native::EventOutcome::Handled
					}
				})
				.on(leptoskit_native::NativeEventSlot::Blur, {
					let blur = blur.clone();
					move |_| {
						blur.fetch_add(1, Ordering::SeqCst);
						leptoskit_native::EventOutcome::Handled
					}
				})
				.children([
					TextInput().class("h-10"),
					TextInput().class("h-10"),
					Button("Focus sentinel").class("h-10"),
				]),
		);
		let weak = Rc::downgrade(&mounted.surface);
		mounted
			.surface
			.borrow()
			.backend()
			.set_event_listener(move |id, event| {
				weak.upgrade().unwrap().borrow().dispatch_event(id, &event);
			});
		native::<gtk::Button>(&mounted, child(&mounted, 2)).grab_focus();
		pump();
		focus.store(0, Ordering::SeqCst);
		blur.store(0, Ordering::SeqCst);
		native::<gtk::Entry>(&mounted, child(&mounted, 0)).grab_focus();
		pump();
		assert_eq!(focus.load(Ordering::SeqCst), 1);
		native::<gtk::Entry>(&mounted, child(&mounted, 1)).grab_focus();
		pump();
		assert_eq!(focus.load(Ordering::SeqCst), 2);
		assert_eq!(
			blur.load(Ordering::SeqCst),
			2,
			"sentinel and first entry each blur once"
		);
	}

	fn button_alignment_positions_native_label() {
		let mounted = mounted(
			&View().class("p-4 gap-4").children([
				Button("Go")
					.text_align(leptoskit_native::TextAlign::Start)
					.class("w-64 h-10"),
				Button("Go")
					.text_align(leptoskit_native::TextAlign::End)
					.class("w-64 h-10"),
			]),
		);
		let start = native::<gtk::Button>(&mounted, child(&mounted, 0))
			.child()
			.and_downcast::<gtk::Label>()
			.unwrap();
		let end = native::<gtk::Button>(&mounted, child(&mounted, 1))
			.child()
			.and_downcast::<gtk::Label>()
			.unwrap();
		assert!(
			end.layout_offsets().0 - start.layout_offsets().0 > 100,
			"native title physically follows shared start/end alignment"
		);
	}

	fn native_order_matches_paint_pick_and_tab() {
		let mounted = mounted(&View().class("w-full h-full bg-white relative").children([
			Button("Back").class("absolute left-4 top-4 w-20 h-20 bg-red-500"),
			Button("Front").class("absolute left-4 top-4 w-20 h-20 bg-blue-600"),
		]));
		let root = mounted.surface.borrow().root().unwrap();
		let back = child(&mounted, 0);
		let front = child(&mounted, 1);
		mounted
			.surface
			.borrow_mut()
			.backend_mut()
			.remove_child(root, back);
		mounted
			.surface
			.borrow_mut()
			.backend_mut()
			.insert_child(root, back, 0);
		pump();
		let events = Rc::new(RefCell::new(Vec::new()));
		mounted.surface.borrow().backend().set_event_listener({
			let events = events.clone();
			move |id, event| events.borrow_mut().push((id, event))
		});
		click(56, 56);
		assert!(
			events.borrow().contains(&(front, NativeEvent::Press)),
			"GTK picking uses painted sibling order"
		);
		let button = native::<gtk::Button>(&mounted, back);
		button.grab_focus();
		pump();
		key("Tab");
		assert!(
			native::<gtk::Button>(&mounted, front).has_focus(),
			"native Tab order follows insertion order"
		);
		let png = mounted
			.surface
			.borrow()
			.backend()
			.snapshot_png(root)
			.unwrap();
		let texture = gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from_owned(png)).unwrap();
		let mut pixels = vec![0; texture.width() as usize * texture.height() as usize * 4];
		texture.download(&mut pixels, texture.width() as usize * 4);
		let offset = (30 * texture.width() as usize + 30) * 4;
		assert!(
			pixels[offset] > pixels[offset + 2],
			"blue sibling is painted above red"
		);
	}

	#[allow(clippy::float_cmp)]
	fn slider_extremes_and_coarse_wheel() {
		let states = [
			SliderState::new(1.0e200, 1.0001e200, 1.0e200, Some(1.0e190)).unwrap(),
			SliderState::new(1.0e-200, 2.0e-200, 1.0e-200, Some(1.0e-210)).unwrap(),
			SliderState::new(
				-f64::MAX / 4.0,
				f64::MAX / 4.0,
				-f64::MAX / 4.0,
				Some(f64::MAX / 4096.0),
			)
			.unwrap(),
			SliderState::new(3.0, 109.0, 3.0, Some(7.0)).unwrap(),
		];
		let mounted = mounted(
			&View()
				.class("p-4")
				.child(Slider(states[0]).class("w-64 h-8")),
		);
		let id = child(&mounted, 0);
		let scale = native::<gtk::Scale>(&mounted, id);
		let events = Rc::new(RefCell::new(Vec::new()));
		mounted.surface.borrow().backend().set_event_listener({
			let events = events.clone();
			move |id, event| events.borrow_mut().push((id, event))
		});

		for state in states {
			mounted
				.surface
				.borrow_mut()
				.set_root(&View().class("p-4").child(Slider(state).class("w-64 h-8")));
			pump();
			scale.set_value(0.64321);
			let expected = state
				.proposed_value(0.64321f64.mul_add(state.max() - state.min(), state.min()))
				.unwrap();
			assert!(
				events
					.borrow()
					.contains(&(id, NativeEvent::ValueChange { value: expected }))
			);
			assert_eq!(
				scale.value(),
				0.0,
				"rejected native proposal restores normalized minimum"
			);
			scale.grab_focus();
			pump();
			key("Right");
			assert!(
				events.borrow().contains(&(
					id,
					NativeEvent::ValueChange {
						value: state
							.proposed_value(state.min() + state.step().unwrap())
							.unwrap()
					}
				))
			);
			key("End");
			assert!(events.borrow().contains(&(
				id,
				NativeEvent::ValueChange {
					value: state.proposed_value(state.max()).unwrap()
				}
			)));
			key("Home");
			assert!(
				events
					.borrow()
					.contains(&(id, NativeEvent::ValueChange { value: state.min() }))
			);
			events.borrow_mut().clear();
		}
		mounted
			.surface
			.borrow_mut()
			.set_root(&View().class("p-4").child(
				Slider(SliderState::new(0.0, 100.0, 50.0, Some(50.0)).unwrap()).class("w-64 h-8"),
			));
		pump();
		let output = Command::new("xdotool")
			.args(["search", "--name", "^Leptoskit GTK Tests$"])
			.output()
			.unwrap();
		let window = String::from_utf8(output.stdout).unwrap();
		assert!(
			Command::new("xdotool")
				.args([
					"mousemove",
					"--window",
					window.lines().last().unwrap(),
					"128",
					"30",
					"click",
					"4"
				])
				.status()
				.unwrap()
				.success()
		);
		pump();
		assert!(
			events.borrow().iter().any(|(event_id, event)| {
				*event_id == id
					&& matches!(event, NativeEvent::ValueChange { value } if *value == 0.0 || *value == 100.0)
			}),
			"coarse native wheel increments move to a neighboring valid grid value"
		);
		assert_eq!(scale.value(), 0.5);
	}

	fn atspi_values_and_decorative_children() {
		let group = RadioGroup::new("accessible options").unwrap();
		let mounted = mounted(
			&View().class("w-full h-full gap-4 p-4").children([
				Slider(SliderState::new(-10.0, 10.0, 0.0, Some(2.0)).unwrap())
					.accessibility_label("AT volume")
					.class("w-64 h-8"),
				Pressable("AT action")
					.class("h-12")
					.child(Text("Decorative text")),
				Button("+")
					.accessibility_label("AT add preview")
					.class("h-10"),
				Radio(group.clone(), true)
					.accessibility_label("AT first")
					.class("w-8 h-8"),
				View().child(
					Radio(group, false)
						.accessibility_label("AT second")
						.class("w-8 h-8"),
				),
			]),
		);
		let slider_id = child(&mounted, 0);
		let events = Rc::new(RefCell::new(Vec::new()));
		mounted.surface.borrow().backend().set_event_listener({
			let events = events.clone();
			move |id, event| events.borrow_mut().push((id, event))
		});
		let (sender, receiver) = std::sync::mpsc::channel();
		let worker = std::thread::spawn(move || {
			sender.send(atspi_probe()).unwrap();
		});
		let deadline = std::time::Instant::now() + Duration::from_secs(20);
		let result = loop {
			pump();
			if let Ok(result) = receiver.try_recv() {
				break result;
			}
			assert!(
				std::time::Instant::now() < deadline,
				"AT-SPI probe timed out"
			);
		};
		worker.join().unwrap();
		result.expect("AT-SPI native interface proof");
		assert!(
			events
				.borrow()
				.contains(&(slider_id, NativeEvent::ValueChange { value: 6.0 }))
		);
	}

	fn atspi_probe() -> Result<(), String> {
		use gtk::gio;
		use gtk::glib;
		type Ref = (String, glib::variant::ObjectPath);
		fn call(
			connection: &gio::DBusConnection,
			target: &Ref,
			interface: &str,
			method: &str,
			parameters: Option<&glib::Variant>,
		) -> Result<glib::Variant, String> {
			connection
				.call_sync(
					Some(&target.0),
					target.1.as_str(),
					interface,
					method,
					parameters,
					None,
					gio::DBusCallFlags::NONE,
					5000,
					None::<&gio::Cancellable>,
				)
				.map_err(|error| error.to_string())
		}
		fn property(
			connection: &gio::DBusConnection,
			target: &Ref,
			interface: &str,
			name: &str,
		) -> Result<glib::Variant, String> {
			Ok(call(
				connection,
				target,
				"org.freedesktop.DBus.Properties",
				"Get",
				Some(&(interface, name).to_variant()),
			)?
			.child_value(0)
			.as_variant()
			.ok_or("variant property")?)
		}
		fn walk(
			connection: &gio::DBusConnection,
			target: &Ref,
			found: &mut Vec<(String, Ref)>,
			depth: usize,
		) -> Result<(), String> {
			if depth > 30 {
				return Err("AT-SPI tree depth".into());
			}
			let name = property(connection, target, "org.a11y.atspi.Accessible", "Name")?
				.get::<String>()
				.ok_or("accessible name")?;
			found.push((name, target.clone()));
			let children = call(
				connection,
				target,
				"org.a11y.atspi.Accessible",
				"GetChildren",
				None,
			)?
			.get::<(Vec<Ref>,)>()
			.ok_or("accessible children")?
			.0;
			for child in children {
				walk(connection, &child, found, depth + 1)?;
			}
			Ok(())
		}
		let session = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>)
			.map_err(|error| error.to_string())?;
		let address = session
			.call_sync(
				Some("org.a11y.Bus"),
				"/org/a11y/bus",
				"org.a11y.Bus",
				"GetAddress",
				None,
				None,
				gio::DBusCallFlags::NONE,
				5000,
				None::<&gio::Cancellable>,
			)
			.map_err(|error| error.to_string())?
			.get::<(String,)>()
			.ok_or("AT bus address")?
			.0;
		let connection = gio::DBusConnection::for_address_sync(
			&address,
			gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
				| gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
			None,
			None::<&gio::Cancellable>,
		)
		.map_err(|error| error.to_string())?;
		let registry = (
			"org.a11y.atspi.Registry".to_owned(),
			glib::variant::ObjectPath::try_from("/org/a11y/atspi/accessible/root").unwrap(),
		);
		let mut found = Vec::new();
		walk(&connection, &registry, &mut found, 0)?;
		let volume = found
			.iter()
			.find(|(name, _)| name == "AT volume")
			.ok_or_else(|| format!("volume missing from AT-SPI tree: {found:?}"))?
			.1
			.clone();
		let spoken_value = property(
			&connection,
			&volume,
			"org.a11y.atspi.Accessible",
			"Description",
		)?
		.get::<String>()
		.ok_or("slider spoken value")?;
		if spoken_value != "0 (range -10 to 10)" {
			return Err(format!(
				"shared numeric domain missing from AT-SPI: {spoken_value}"
			));
		}
		let current = property(&connection, &volume, "org.a11y.atspi.Value", "CurrentValue")?
			.get::<f64>()
			.ok_or("current value")?;
		let min = property(&connection, &volume, "org.a11y.atspi.Value", "MinimumValue")?
			.get::<f64>()
			.ok_or("minimum")?;
		let max = property(&connection, &volume, "org.a11y.atspi.Value", "MaximumValue")?
			.get::<f64>()
			.ok_or("maximum")?;
		if (current - 0.5).abs() > f64::EPSILON
			|| min.abs() > f64::EPSILON
			|| (max - 1.0).abs() > f64::EPSILON
		{
			return Err(format!("normalized native AT range: {min} {current} {max}"));
		}
		call(
			&connection,
			&volume,
			"org.freedesktop.DBus.Properties",
			"Set",
			Some(&("org.a11y.atspi.Value", "CurrentValue", 0.8f64.to_variant()).to_variant()),
		)?;
		let restored = property(&connection, &volume, "org.a11y.atspi.Value", "CurrentValue")?
			.get::<f64>()
			.ok_or("restored current")?;
		if (restored - 0.5).abs() > f64::EPSILON {
			return Err(format!("controlled AT rejection: {restored}"));
		}
		if found.iter().any(|(name, _)| name == "Decorative text") {
			return Err(format!("decorative child appears in AT tree: {found:?}"));
		}
		if !found.iter().any(|(name, _)| name == "AT add preview") {
			return Err("explicit native Button accessibility label was overwritten".into());
		}
		for (name, target) in found
			.iter()
			.filter(|(name, _)| name == "AT first" || name == "AT second")
		{
			let description = property(
				&connection,
				target,
				"org.a11y.atspi.Accessible",
				"Description",
			)?
			.get::<String>()
			.ok_or("radio description")?;
			let expected = if name == "AT first" {
				"Option 1 of 2"
			} else {
				"Option 2 of 2"
			};
			if description != expected {
				return Err(format!("radio group position: {description} != {expected}"));
			}
		}
		Ok(())
	}

	fn readable_native_widget_catalog() {
		let group = RadioGroup::new("density").unwrap();
		let row = |label, control: Element| {
			View()
				.class("flex-row items-center gap-4 h-10")
				.children([Text(label).class("w-48 text-slate-700"), control])
		};
		let mounted = mounted(
			&View()
				.class("w-full h-full p-6 gap-3 bg-slate-100")
				.children([
					Text("Linux native widgets").class("text-2xl font-bold text-slate-900"),
					Text("One shared tree, real GTK4 controls").class("text-slate-600"),
					row(
						"Switch",
						Switch(true).accessibility_label("Wi-Fi").class("w-16 h-8"),
					),
					row(
						"Checkbox",
						Checkbox(true)
							.accessibility_label("Enable previews")
							.class("w-8 h-8"),
					),
					row(
						"Radio",
						View().class("flex-row gap-3 w-32 h-8").children([
							Radio(group.clone(), true)
								.accessibility_label("Comfortable")
								.class("w-8 h-8"),
							Radio(group, false)
								.accessibility_label("Compact")
								.class("w-8 h-8"),
						]),
					),
					row(
						"Slider",
						Slider(SliderState::new(0.0, 100.0, 64.0, Some(1.0)).unwrap())
							.accessibility_label("Volume")
							.class("w-40 h-8"),
					),
					row(
						"Text input",
						TextInput().value("Hello from GTK").class(
							"w-48 h-9 bg-white border border-slate-300 rounded-md text-slate-900",
						),
					),
					row(
						"Password",
						PasswordInput().value("demo-fixture").class(
							"w-48 h-9 bg-white border border-slate-300 rounded-md text-slate-900",
						),
					),
					row(
						"Progress",
						LinearProgressIndicator(Progress::Determinate(
							ProgressValue::new(0.64).unwrap(),
						))
						.class("w-48 h-4"),
					),
					row(
						"Loading",
						View()
							.class("flex-row items-center gap-3 w-48 h-8")
							.children([
								ActivityIndicator(true).class("w-8 h-8"),
								LinearProgressIndicator(Progress::Indeterminate).class("w-32 h-4"),
							]),
					),
					View().class("flex-row gap-4 items-center h-10").children([
						Button("Native button").class("w-40 h-10"),
						Pressable("Create preview")
							.class("w-48 h-10 bg-blue-600 rounded-lg items-center justify-center")
							.child(Text("Create preview").class("text-white font-semibold")),
						Image(artifact_path("image.png").to_string_lossy()).class("w-8 h-8"),
					]),
					ScrollView()
						.class("w-full h-16 bg-white rounded-lg border border-slate-300")
						.child(View().class("p-3 gap-2 h-48").children([
							Text("A real scroll document"),
							Text("More shared content"),
							Text("Native scrolling on both axes"),
						])),
				]),
		);
		let surface = mounted.surface.borrow();
		std::fs::write(
			artifact_path("catalog.png"),
			surface
				.backend()
				.snapshot_png(surface.root().unwrap())
				.unwrap(),
		)
		.unwrap();
	}

	fn resize_and_root_replacement_are_hosted() {
		let mounted = mounted(&View().class("w-full h-full bg-blue-600"));
		let old_root = mounted.surface.borrow().root().unwrap();
		let old = mounted.surface.borrow().backend().view(old_root).unwrap();
		let sizes = Rc::new(RefCell::new(Vec::new()));
		let weak = Rc::downgrade(&mounted.surface);
		mounted.surface.borrow().backend().set_resize_listener({
			let sizes = sizes.clone();
			move |width, height| {
				sizes.borrow_mut().push((width, height));
				weak.upgrade()
					.unwrap()
					.borrow_mut()
					.set_viewport(width, height);
			}
		});
		mounted.window.set_default_size(600, 700);
		pump();
		assert!(sizes.borrow().contains(&(600.0, 700.0)));
		assert_eq!((old.width(), old.height()), (600, 700));
		mounted
			.surface
			.borrow_mut()
			.set_root(&Button("Replacement root").class("w-40 h-10"));
		pump();
		let new_root = mounted.surface.borrow().root().unwrap();
		assert_ne!(new_root, old_root);
		let new = mounted.surface.borrow().backend().view(new_root).unwrap();
		assert!(new.is_mapped());
		assert_eq!((new.width(), new.height()), (160, 40));
		assert!(old.parent().is_none());
	}

	fn root_frames_respect_fixed_and_zero_dimensions() {
		let mounted = mounted(&View().class("w-40 h-20 bg-red-500"));
		let id = mounted.surface.borrow().root().unwrap();
		let view = mounted.surface.borrow().backend().view(id).unwrap();
		assert_eq!((view.width(), view.height()), (160, 80));
		let png = mounted.surface.borrow().backend().snapshot_png(id).unwrap();
		let texture = gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from_owned(png)).unwrap();
		assert_eq!((texture.width(), texture.height()), (160, 80));
		mounted
			.surface
			.borrow_mut()
			.set_root(&View().class("w-0 h-0"));
		pump();
		assert_eq!((view.width(), view.height()), (0, 0));
	}

	fn rounded_picking_and_input_paint() {
		let mounted = mounted(
			&View().class("w-full h-full bg-white relative").children([
				Pressable("Round")
					.class(
						"absolute left-4 top-4 w-20 h-20 rounded-full overflow-hidden bg-blue-600",
					)
					.child(Text("Press")),
				View()
					.class("absolute left-32 top-4 w-20 h-20 rounded-full overflow-hidden")
					.child(Button("Child").class("w-20 h-20")),
				TextInput()
					.value("Demo")
					.class("absolute left-4 top-32 w-40 h-10 bg-red-500 rounded-lg text-center"),
				Text("Wide line\nx").class("absolute left-4 top-48 w-40 text-center"),
			]),
		);
		let events = Rc::new(RefCell::new(Vec::new()));
		mounted.surface.borrow().backend().set_event_listener({
			let events = events.clone();
			move |id, event| events.borrow_mut().push((id, event))
		});
		click(17, 17);
		assert!(
			!events
				.borrow()
				.iter()
				.any(|(_, event)| *event == NativeEvent::Press),
			"rounded Pressable corner rejects pointer"
		);
		click(56, 56);
		assert!(
			events
				.borrow()
				.contains(&(child(&mounted, 0), NativeEvent::Press))
		);
		events.borrow_mut().clear();
		click(129, 17);
		assert!(
			!events
				.borrow()
				.iter()
				.any(|(_, event)| *event == NativeEvent::Press),
			"rounded View corner rejects descendant pointer"
		);
		click(168, 56);
		assert!(
			events
				.borrow()
				.iter()
				.any(|(_, event)| *event == NativeEvent::Press)
		);
		let entry = native::<gtk::Entry>(&mounted, child(&mounted, 2));
		assert!((EntryExt::alignment(&entry) - 0.5).abs() < f32::EPSILON);
		let label = native::<gtk::Label>(&mounted, child(&mounted, 3));
		assert_eq!(label.justify(), gtk::Justification::Center);
		let root = mounted.surface.borrow().root().unwrap();
		let png = mounted
			.surface
			.borrow()
			.backend()
			.snapshot_png(root)
			.unwrap();
		std::fs::write(artifact_path("rounded.png"), &png).unwrap();
		let texture = gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from_owned(png)).unwrap();
		let mut pixels = vec![0; texture.width() as usize * texture.height() as usize * 4];
		texture.download(&mut pixels, texture.width() as usize * 4);
		let offset = (148 * texture.width() as usize + 24) * 4;
		let pixel = &pixels[offset..offset + 4];
		assert!(
			pixel[2] > 200 && pixel[1] < 150,
			"entry chrome reveals custom red paint: {pixel:?}"
		);
	}

	fn overflow_visible_and_hidden_have_distinct_pixels() {
		let mounted = mounted(
			&View().class("w-full h-full bg-white relative").children([
				View()
					.class("absolute left-4 top-4 w-10 h-10")
					.child(View().class("w-20 h-20 bg-green-500")),
				View()
					.class("absolute left-32 top-4 w-10 h-10 overflow-hidden")
					.child(View().class("w-20 h-20 bg-green-500")),
			]),
		);
		let surface = mounted.surface.borrow();
		let png = surface
			.backend()
			.snapshot_png(surface.root().unwrap())
			.unwrap();
		std::fs::write(artifact_path("overflow.png"), &png).unwrap();
		let texture = gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from_owned(png)).unwrap();
		let mut pixels = vec![0; texture.width() as usize * texture.height() as usize * 4];
		texture.download(&mut pixels, texture.width() as usize * 4);
		let pixel = |x: usize, y: usize| {
			let offset = (y * texture.width() as usize + x) * 4;
			&pixels[offset..offset + 4]
		};
		assert!(
			pixel(80, 40)[1] > pixel(80, 40)[2],
			"ordinary View paints overflow"
		);
		assert_eq!(
			pixel(192, 40),
			&[255, 255, 255, 255],
			"clipped View hides overflow"
		);
	}
}
