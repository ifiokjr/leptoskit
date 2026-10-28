//! A real Windows host and deterministic screenshot fixture for CI.

#[cfg(target_os = "windows")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
	use std::cell::Cell;
	use std::cell::RefCell;
	use std::rc::Rc;

	use leptoskit_native::Element;
	use leptoskit_native::NativeEvent;
	use leptoskit_native::NativeSurface;
	use leptoskit_native::components::*;
	use leptoskit_windows::WindowsBackend;
	use leptoskit_windows::WindowsWindow;

	fn app() -> Element {
		View()
			.class("w-full h-full bg-slate-100 p-6 gap-4")
			.children([
				Text("Leptoskit on Windows").class("text-2xl font-bold text-slate-900"),
				Text("One shared primitive tree, real Windows controls.").class("text-slate-600"),
				View()
					.class("bg-white rounded-xl border border-slate-300 p-4 gap-3")
					.children([
						TextInput()
							.aria_label("Editor")
							.value("Native text editor")
							.accessibility_label("Name")
							.class("w-full h-10 border border-slate-300"),
						PasswordInput()
							.aria_label("Password")
							.value("fixture only")
							.accessibility_label("Password")
							.class("w-full h-10 border border-slate-300"),
						Switch(true)
							.accessibility_label("Notifications")
							.class("w-48"),
						Checkbox(false)
							.accessibility_label("Enable previews")
							.class("w-48"),
						View().class("flex-row gap-3").children([
							Radio(RadioGroup::new("density").unwrap(), true)
								.accessibility_label("Comfortable")
								.class("w-40"),
							Radio(RadioGroup::new("density").unwrap(), false)
								.accessibility_label("Compact")
								.class("w-32"),
						]),
						Slider(SliderState::new(0.0, 100.0, 64.0, Some(1.0)).unwrap())
							.accessibility_label("Volume")
							.class("w-full"),
						LinearProgressIndicator(Progress::Determinate(
							ProgressValue::new(0.64).unwrap(),
						))
						.class("w-full"),
						View()
							.class("flex-row items-center gap-3")
							.children([ActivityIndicator(true), Text("Ready for native input")]),
						vector_row(),
						Button("Native action").class("h-10 bg-blue-600 text-white rounded-lg"),
						Pressable("Add preview")
							.class("h-12 p-3 bg-slate-900 rounded-lg")
							.child(Text("Add preview →").class("text-white font-semibold")),
					]),
				ScrollView()
					.class("h-28 w-full bg-white border border-slate-300 rounded-lg")
					.child(View().class("gap-2 p-3").children((0..12).map(|index| {
						Text(format!("Native scroll item {}", index + 1)).class("h-6")
					}))),
			])
	}

	let window = WindowsWindow::new("Leptoskit Windows primitives", 560, 800)?;
	let mut backend = WindowsBackend::new()?;
	backend.set_dpi(window.dpi());
	let surface = Rc::new(RefCell::new(NativeSurface::new(backend)));
	let (width, height) = window.client_size()?;
	surface.borrow_mut().set_viewport(width, height);
	surface.borrow_mut().set_root(&app());
	let weak = Rc::downgrade(&surface);
	surface
		.borrow_mut()
		.backend_mut()
		.set_event_listener(Rc::new(move |id, event| {
			let Some(surface) = weak.upgrade() else {
				return;
			};
			let mut surface = surface.borrow_mut();

			match event {
				NativeEvent::PressIn => surface.press_in(id),
				NativeEvent::PressOut => surface.press_out(id),
				_ => {
					surface.dispatch_event(id, &event);
					surface.set_root(&app());
				}
			}
		}));
	let weak = Rc::downgrade(&surface);
	let previous_dpi = Cell::new(window.dpi());
	window.set_resize_listener(Rc::new(move |width, height, dpi| {
		let Some(surface) = weak.upgrade() else {
			return;
		};
		let mut surface = surface.borrow_mut();
		if previous_dpi.replace(dpi) != dpi {
			surface.backend_mut().set_dpi(dpi);
			surface.refresh_text_measurer();
		}
		surface.set_viewport(width, height);
		surface.set_root(&app());
	}));
	window.mount(surface.borrow().backend(), surface.borrow().root().unwrap())?;

	if let Some(directory) = std::env::args_os().nth(1) {
		let directory = std::path::PathBuf::from(directory);
		std::fs::create_dir_all(&directory)?;
		window.show();
		window.pump_messages();
		std::thread::sleep(std::time::Duration::from_millis(120));
		window.pump_messages();
		std::fs::write(directory.join("widgets.png"), window.snapshot_png()?)?;
		std::fs::write(
			directory.join("report.json"),
			format!(
				"{{\"platform\":\"windows\",\"renderer\":\"Win32 \
				 HWND\",\"dpi\":{},\"screenshot\":\"widgets.png\"}}\n",
				window.dpi()
			),
		)?;
		return Ok(());
	}

	window.run()?;
	Ok(())
}

/// A filled, fitted disc with a round-capped tick, beside a hand-drawn
/// underline stretched across the rest of the row.
#[cfg(target_os = "windows")]
fn vector_row() -> leptoskit_native::Element {
	use leptoskit_native::Color;
	use leptoskit_native::components::*;

	View().class("flex-row items-center gap-3").children([
		// Fit: a filled disc under a round-capped tick,
		// centred in a wider frame.
		Vector(ViewBox::new(0.0, 0.0, 24.0, 24.0).unwrap())
			.class("w-12 h-8")
			.accessibility_label("Done")
			.path(
				VectorPath::new(
					PathData::parse(
						"M12 2C17.5 2 22 6.5 22 12S17.5 22 12 22 2 17.5 2 12 6.5 2 12 2Z",
					)
					.unwrap(),
				)
				.fill(Color::rgba(219, 234, 254, 255)),
			)
			.path(
				VectorPath::new(PathData::parse("M6 13Q8 15 10 17Q14 10 19 7").unwrap()).stroke(
					Stroke::new(Color::rgba(37, 99, 235, 255), 2.5)
						.unwrap()
						.round(),
				),
			),
		// Stretch: a hand-drawn underline filling its
		// frame, its stroke still 2 units wide.
		Vector(ViewBox::new(0.0, 0.0, 100.0, 10.0).unwrap())
			.class("flex-1 h-6")
			.scaling(VectorScaling::Stretch)
			.path(
				VectorPath::new(PathData::parse("M2 6C20 1 30 9 50 5S80 2 98 6").unwrap()).stroke(
					Stroke::new(Color::rgba(15, 23, 42, 255), 2.0)
						.unwrap()
						.round(),
				),
			),
	])
}

#[cfg(not(target_os = "windows"))]
fn main() {}
