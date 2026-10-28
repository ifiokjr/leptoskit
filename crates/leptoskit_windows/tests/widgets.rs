//! Native execution only: real HWND state, proposals, lifecycle, and pixels.
#![cfg(target_os = "windows")]
#![allow(unsafe_code, clippy::float_cmp)]

use std::cell::RefCell;
use std::rc::Rc;

use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::*;
use leptoskit_windows::WindowsBackend;
use leptoskit_windows::WindowsWindow;
use leptoskit_yoga::NodeId;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::CLSCTX_INPROC_SERVER;
use windows::Win32::System::Com::COINIT_MULTITHREADED;
use windows::Win32::System::Com::CoCreateInstance;
use windows::Win32::System::Com::CoInitializeEx;
use windows::Win32::System::Com::CoUninitialize;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Interface;
use windows::core::w;

#[expect(
	clippy::needless_pass_by_value,
	reason = "the test fixture consumes its builder at the mount boundary"
)]
fn mount(root: Element) -> (WindowsWindow, NativeSurface<WindowsBackend>) {
	let window = WindowsWindow::new("Leptoskit native widget verification", 520, 680).unwrap();
	let mut backend = WindowsBackend::new().unwrap();
	backend.set_dpi(window.dpi());
	let mut surface = NativeSurface::new(backend);
	surface.set_viewport(520.0, 680.0);
	surface.set_root(&root);
	window
		.mount(surface.backend(), surface.root().unwrap())
		.unwrap();
	window.show();
	// Pixel captures and WindowFromPoint read the screen, so keep the fixture
	// above any console or runner window that would otherwise cover it.
	// SAFETY: The fixture owns this live top-level window.
	unsafe {
		SetWindowPos(
			window.handle(),
			Some(HWND_TOPMOST),
			0,
			0,
			0,
			0,
			SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
		)
		.unwrap();
	}
	window.pump_messages();
	(window, surface)
}

fn child(surface: &NativeSurface<WindowsBackend>, index: usize) -> NodeId {
	surface.children(surface.root().unwrap())[index]
}

fn class(hwnd: HWND) -> String {
	let mut buffer = [0; 128];
	// SAFETY: The fixed output buffer is passed with its exact slice length.
	let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
	String::from_utf16_lossy(&buffer[..length as usize])
}

fn text(hwnd: HWND) -> String {
	// SAFETY: Standard edit text is copied into a buffer sized for its
	// terminator.
	unsafe {
		let mut buffer = vec![0; GetWindowTextLengthW(hwnd) as usize + 1];
		let length = GetWindowTextW(hwnd, &mut buffer);
		String::from_utf16_lossy(&buffer[..length as usize])
	}
}

fn click(hwnd: HWND) {
	// SAFETY: The test only passes a BUTTON HWND owned by its local backend.
	unsafe {
		SendMessageW(hwnd, BM_CLICK, None, None);
	}
}

fn key(hwnd: HWND, key: VIRTUAL_KEY) {
	// SAFETY: This is a synchronous keyboard message to the real native
	// control.
	unsafe {
		SendMessageW(
			hwnd,
			WM_KEYDOWN,
			Some(WPARAM(key.0 as usize)),
			Some(LPARAM(1)),
		);
	}
}

fn queued_key(window: &WindowsWindow, hwnd: HWND, key: VIRTUAL_KEY) {
	// SAFETY: Queue input through the same IsDialogMessage/TranslateMessage
	// route as the production host, including dialog handling of Return/Tab.
	unsafe {
		let _ = SetFocus(Some(hwnd));
		PostMessageW(
			Some(hwnd),
			WM_KEYDOWN,
			WPARAM(usize::from(key.0)),
			LPARAM(1),
		)
		.unwrap();
		PostMessageW(
			Some(hwnd),
			WM_KEYUP,
			WPARAM(usize::from(key.0)),
			LPARAM((1_u32 | (1 << 30) | (1 << 31)) as isize),
		)
		.unwrap();
	}
	window.pump_messages();
}

fn accessible(hwnd: HWND) -> IAccessible {
	let mut pointer = std::ptr::null_mut();
	// SAFETY: AccessibleObjectFromWindow returns one owned COM reference with
	// the requested IID; from_raw assumes that exact reference.
	unsafe {
		AccessibleObjectFromWindow(
			hwnd,
			OBJID_CLIENT.0 as u32,
			&IAccessible::IID,
			&raw mut pointer,
		)
		.unwrap();
		IAccessible::from_raw(pointer)
	}
}

fn pixel(window: &WindowsWindow, hwnd: HWND, x: f32, y: f32) -> [u8; 4] {
	window.pump_messages();
	std::thread::sleep(std::time::Duration::from_millis(50));
	let image = image::load_from_memory(&window.snapshot_png().unwrap())
		.unwrap()
		.into_rgba8();
	let mut frame = RECT::default();
	let mut origin = POINT::default();
	// SAFETY: Both rectangles refer to the live fixture window's own client
	// area.
	unsafe {
		GetWindowRect(hwnd, &raw mut frame).unwrap();
		let _ = ClientToScreen(window.handle(), &raw mut origin);
	}
	let scale = window.dpi() as f32 / 96.0;
	image
		.get_pixel(
			(frame.left - origin.x + (x * scale).round() as i32) as u32,
			(frame.top - origin.y + (y * scale).round() as i32) as u32,
		)
		.0
}

fn near(actual: [u8; 4], expected: [u8; 4]) {
	for (actual, expected) in actual.into_iter().zip(expected) {
		assert!(
			actual.abs_diff(expected) <= 3,
			"native pixel channel {actual} differs from {expected}"
		);
	}
}

#[test]
fn overflow_visible_and_clip_hidden_preserve_pixels_and_native_hit_targets() {
	fn app(clip: bool) -> Element {
		View().class("w-full h-full bg-white p-4").child(
			View()
				.class("relative w-10 h-10 bg-blue-500")
				.with_props(|props| props.clip = Some(clip))
				.child(
					Button("")
						.accessibility_label("Overflow action")
						.class("absolute left-12 top-0 w-8 h-8 bg-red-500"),
				),
		)
	}

	let (window, mut surface) = mount(app(false));
	let parent = child(&surface, 0);
	let id = surface.children(parent)[0];
	let frame = surface.backend().view(id).unwrap();
	let control = surface.backend().control(id).unwrap();
	near(pixel(&window, frame, 10.0, 10.0), [251, 44, 54, 255]);
	let mut point = POINT { x: 10, y: 10 };
	// SAFETY: Hit-testing the genuine native overflow target uses screen
	// coords.
	unsafe {
		let _ = ClientToScreen(control, &raw mut point);
		assert_eq!(WindowFromPoint(point), control);
	}
	surface.set_root(&app(true));
	assert_eq!(surface.backend().control(id), Some(control));
	near(pixel(&window, frame, 10.0, 10.0), [255, 255, 255, 255]);
	// SAFETY: Clipping must also remove the overflow from native pointer hit
	// tests.
	unsafe {
		assert_ne!(WindowFromPoint(point), control);
	}
}

#[test]
fn group_opacity_composites_overlapping_content_and_visible_overflow_once() {
	let (window, surface) = mount(
		View().class("w-full h-full bg-white p-4").child(
			View()
				.class("relative w-20 h-20 bg-blue-500 opacity-50")
				.child(View().class("absolute left-10 top-0 w-20 h-10 bg-red-500")),
		),
	);
	let parent = child(&surface, 0);
	let child = surface.children(parent)[0];
	let frame = surface.backend().view(child).unwrap();
	// The red child overlaps the blue parent at x=10 and overflows at x=60.
	// Both pixels must blend red once over the white host, not red over an
	// independently faded blue parent. The child's native alpha remains 1.0.
	near(pixel(&window, frame, 10.0, 10.0), [253, 149, 154, 255]);
	near(pixel(&window, frame, 60.0, 10.0), [253, 149, 154, 255]);
	// SAFETY: Only the ancestor group owns the local alpha, preserving nesting.
	unsafe {
		assert_eq!(
			GetWindowLongW(
				surface.backend().composition_window(child).unwrap(),
				GWL_EXSTYLE
			) as u32
				& WS_EX_LAYERED.0,
			0
		);
	}
}

#[test]
fn transparent_and_translucent_containers_reveal_overlapping_siblings() {
	let (window, surface) = mount(
		View().class("relative w-full h-full bg-white").children([
			View().class("absolute left-4 top-4 w-20 h-20 bg-red-500"),
			// A transparent container covers the red sibling; only its own
			// child paints.
			View()
				.class("absolute left-4 top-4 w-20 h-20")
				.child(View().class("absolute left-2 top-10 w-4 h-4 bg-blue-500")),
			// A scrim half covers the red sibling and half covers the white
			// root.
			View().class("absolute left-14 top-4 w-20 h-20 bg-black/50"),
		]),
	);
	let red = surface.backend().view(child(&surface, 0)).unwrap();
	near(pixel(&window, red, 4.0, 4.0), [251, 44, 54, 255]);
	near(pixel(&window, red, 12.0, 44.0), [43, 127, 255, 255]);
	near(pixel(&window, red, 60.0, 4.0), [125, 22, 27, 255]);
	near(pixel(&window, red, 90.0, 4.0), [127, 127, 127, 255]);
	let mut point = POINT { x: 4, y: 4 };
	// SAFETY: Hit-testing uses screen coordinates of the live red frame.
	unsafe {
		let _ = ClientToScreen(red, &raw mut point);
		assert_eq!(WindowFromPoint(point), red);
	}
}

#[test]
fn percent_encoded_file_urls_and_alpha_images_render_real_pixels() {
	let path = std::env::temp_dir().join(format!(
		"leptoskit alpha fixture {}.png",
		std::process::id()
	));
	image::RgbaImage::from_pixel(16, 16, image::Rgba([255, 0, 0, 128]))
		.save(&path)
		.unwrap();
	let source = format!("file:///{}", path.display())
		.replace('\\', "/")
		.replace(' ', "%20");
	let (window, surface) = mount(
		View()
			.class("w-full h-full bg-blue-500 p-4")
			.child(Image(source).class("w-16 h-16")),
	);
	let id = child(&surface, 0);
	assert!(surface.backend().image_error(id).is_none());
	near(
		pixel(&window, surface.backend().view(id).unwrap(), 10.0, 10.0),
		[149, 63, 127, 255],
	);
	std::fs::remove_file(path).unwrap();
}

#[test]
fn all_builtin_kinds_create_real_native_windows_and_exact_frames() {
	let group = RadioGroup::new("options").unwrap();
	let nodes = [
		View().class("w-20 h-8"),
		Text("Native text"),
		Element::new(NativeKind::TextRun)
			.with_props(|props| props.text = Some("Text run".to_owned())),
		Image("missing-image.png").class("w-8 h-8"),
		ScrollView().class("w-20 h-8"),
		TextInput().aria_label("Editor").value("text"),
		Button("Action"),
		Switch(false),
		Checkbox(true),
		Radio(group, false),
		Slider(SliderState::new(-10.0, 30.0, 0.0, Some(2.0)).unwrap()),
		LinearProgressIndicator(Progress::Determinate(ProgressValue::new(0.5).unwrap())),
		ActivityIndicator(true),
		PasswordInput().aria_label("Password").value("fixture only"),
		Pressable("Open preview")
			.class("w-20 h-8")
			.child(Text("Open")),
		Vector(ViewBox::UNIT).class("w-8 h-8"),
	];
	let (window, surface) = mount(View().class("w-full h-full p-4 gap-2").children(nodes));
	let classes = [
		None,
		Some("Static"),
		Some("Static"),
		None,
		None,
		Some("Edit"),
		Some("Button"),
		Some("Button"),
		Some("Button"),
		Some("Button"),
		Some("msctls_trackbar32"),
		Some("msctls_progress32"),
		None,
		Some("Edit"),
		Some("Button"),
		None,
	];

	for (index, expected) in classes.into_iter().enumerate() {
		let id = child(&surface, index);
		let hwnd = surface.backend().view(id).unwrap();
		let layout = surface.layout(id);
		let mut rect = RECT::default();
		// SAFETY: Every HWND was returned by this still-live backend.
		unsafe {
			assert!(IsWindow(Some(hwnd)).as_bool());
			GetWindowRect(hwnd, &raw mut rect).unwrap();
		}
		let scale = window.dpi() as f32 / 96.0;
		assert_eq!(
			rect.right - rect.left,
			(layout.width * scale).round() as i32
		);
		assert_eq!(
			rect.bottom - rect.top,
			(layout.height * scale).round() as i32
		);

		if let Some(expected) = expected {
			assert!(class(surface.backend().control(id).unwrap()).eq_ignore_ascii_case(expected));
		}
	}

	assert!(
		surface
			.backend()
			.scroll_document(child(&surface, 4))
			.is_some()
	);
	assert!(surface.backend().image_error(child(&surface, 3)).is_some());
}

#[test]
fn native_controls_reject_proposals_without_losing_identity() {
	let (_window, mut surface) =
		mount(
			View().children([
				Checkbox(false).accessibility_label("Enable previews"),
				Slider(SliderState::new(0.0, 1.0, 0.5, Some(0.1)).unwrap())
					.accessibility_label("Volume"),
				TextInput().value("committed").accessibility_label("Name"),
			]),
		);
	let events = Rc::new(RefCell::new(Vec::new()));
	let recorded = Rc::clone(&events);
	surface
		.backend_mut()
		.set_event_listener(Rc::new(move |id, event| {
			recorded.borrow_mut().push((id, event));
		}));
	let checkbox = surface.backend().control(child(&surface, 0)).unwrap();
	let slider = surface.backend().control(child(&surface, 1)).unwrap();
	let input = surface.backend().control(child(&surface, 2)).unwrap();
	click(checkbox);
	key(slider, VK_RIGHT);
	// SAFETY: The EDIT class copies this terminated text synchronously.
	unsafe {
		SetWindowTextW(input, w!("proposal")).unwrap();
	}
	let snapped = SliderState::new(0.0, 1.0, 0.5, Some(0.1))
		.unwrap()
		.proposed_value(0.6)
		.unwrap();
	assert_eq!(
		events
			.borrow()
			.iter()
			// A foreground window may also report genuine focus changes.
			.filter(|(_, event)| !matches!(event, NativeEvent::Focus | NativeEvent::Blur))
			.map(|(_, event)| event.clone())
			.collect::<Vec<_>>(),
		vec![
			NativeEvent::Toggle { checked: true },
			NativeEvent::ValueChange { value: snapped },
			NativeEvent::Change {
				value: "proposal".to_owned()
			}
		]
	);
	assert_eq!(text(input), "committed");
	// SAFETY: Querying standard control state does not mutate the renderer.
	unsafe {
		assert_eq!(
			SendMessageW(checkbox, BM_GETCHECK, None, None).0,
			BST_UNCHECKED.0 as isize
		);
		assert_eq!(SendMessageW(slider, WM_USER, None, None).0, 500_000);
	}
	assert_eq!(
		surface.backend().control(child(&surface, 0)),
		Some(checkbox)
	);
	assert_eq!(surface.backend().control(child(&surface, 1)), Some(slider));
}

#[test]
fn accepted_actions_and_synchronous_removal_are_reentrant() {
	let (_window, surface) = mount(View().child(Checkbox(false).accessibility_label("Accepted")));
	let surface = Rc::new(RefCell::new(surface));
	let weak = Rc::downgrade(&surface);
	surface
		.borrow_mut()
		.backend_mut()
		.set_event_listener(Rc::new(move |_, event| {
			if event == (NativeEvent::Toggle { checked: true }) {
				weak.upgrade()
					.unwrap()
					.borrow_mut()
					.set_root(&View().child(Checkbox(true).accessibility_label("Accepted")));
			}
		}));
	let id = child(&surface.borrow(), 0);
	let control = surface.borrow().backend().control(id).unwrap();
	click(control);
	assert_eq!(surface.borrow().backend().control(id), Some(control));
	// SAFETY: The live BUTTON's native state reflects the accepted commit.
	unsafe {
		assert_eq!(
			SendMessageW(control, BM_GETCHECK, None, None).0,
			BST_CHECKED.0 as isize
		);
	}
	let weak = Rc::downgrade(&surface);
	surface
		.borrow_mut()
		.backend_mut()
		.set_event_listener(Rc::new(move |_, _| {
			weak.upgrade().unwrap().borrow_mut().set_root(&View());
		}));
	click(control);
	// SAFETY: IsWindow explicitly permits testing a formerly valid HWND.
	unsafe {
		assert!(!IsWindow(Some(control)).as_bool());
	}
}

#[test]
fn radio_arrows_skip_hidden_and_disabled_members_across_wrappers() {
	let group = RadioGroup::new("density").unwrap();
	let (_window, mut surface) = mount(
		View().children([
			Radio(group.clone(), true).accessibility_label("First"),
			View().child(
				Radio(group.clone(), false)
					.enabled(false)
					.accessibility_label("Disabled"),
			),
			View()
				.with_props(|props| props.hidden = Some(true))
				.child(Radio(group.clone(), false).accessibility_label("Hidden")),
			Radio(group, false).accessibility_label("Last"),
		]),
	);
	let events = Rc::new(RefCell::new(Vec::new()));
	let recorded = Rc::clone(&events);
	surface
		.backend_mut()
		.set_event_listener(Rc::new(move |id, event| {
			recorded.borrow_mut().push((id, event));
		}));
	let first = surface.backend().control(child(&surface, 0)).unwrap();
	let last_id = child(&surface, 3);
	let last = surface.backend().control(last_id).unwrap();
	key(first, VK_RIGHT);
	assert!(
		events
			.borrow()
			.contains(&(last_id, NativeEvent::Toggle { checked: true }))
	);
	// SAFETY: Focus and radio selection are read from the actual native
	// controls.
	unsafe {
		assert_eq!(GetFocus(), last);
		assert_eq!(
			SendMessageW(last, BM_GETCHECK, None, None).0,
			BST_UNCHECKED.0 as isize
		);
	}
}

#[test]
fn password_accessibility_names_preserve_values_and_keyboard_events() {
	let (window, mut surface) = mount(
		View().children([
			PasswordInput()
				.value("fixture only")
				.accessibility_label("Account password"),
			Slider(SliderState::default()).accessibility_label("Volume"),
			Pressable("Add preview")
				.class("w-32 h-10")
				.child(Text("Add")),
		]),
	);
	let password = surface.backend().control(child(&surface, 0)).unwrap();
	let slider = surface.backend().control(child(&surface, 1)).unwrap();
	let pressable = surface.backend().control(child(&surface, 2)).unwrap();
	// MSAA child ids are VT_I4; the strict UI Automation proxy rejects VT_UI4.
	let child_self = VARIANT::from(CHILDID_SELF as i32);
	// SAFETY: The controls expose standard MSAA objects with annotated names.
	unsafe {
		assert_ne!(
			GetWindowLongW(password, GWL_STYLE) as u32 & ES_PASSWORD as u32,
			0
		);
		assert_ne!(SendMessageW(password, EM_GETPASSWORDCHAR, None, None).0, 0);
		assert_eq!(
			accessible(password)
				.get_accName(&child_self)
				.unwrap()
				.to_string(),
			"Account password"
		);
		assert_eq!(
			accessible(slider)
				.get_accName(&child_self)
				.unwrap()
				.to_string(),
			"Volume"
		);
		assert_eq!(
			u32::try_from(&accessible(slider).get_accRole(&child_self).unwrap()).unwrap(),
			ROLE_SYSTEM_SLIDER
		);
		assert_eq!(
			accessible(pressable)
				.get_accName(&child_self)
				.unwrap()
				.to_string(),
			"Add preview"
		);
		assert_ne!(
			accessible(password)
				.get_accValue(&child_self)
				.ok()
				.map(|value| value.to_string()),
			Some("fixture only".to_owned())
		);
	}
	assert_eq!(text(password), "fixture only");
	let events = Rc::new(RefCell::new(Vec::new()));
	let recorded = Rc::clone(&events);
	surface
		.backend_mut()
		.set_event_listener(Rc::new(move |_, event| recorded.borrow_mut().push(event)));
	queued_key(&window, password, VK_RETURN);
	queued_key(&window, pressable, VK_RETURN);
	queued_key(&window, pressable, VK_SPACE);
	// SAFETY: Native accessibility default action invokes the real BUTTON.
	unsafe {
		accessible(pressable)
			.accDoDefaultAction(&child_self)
			.unwrap();
	}
	// The standard BUTTON proxy posts its default-action click.
	window.pump_messages();
	assert_eq!(
		events
			.borrow()
			.iter()
			.filter(|event| **event == NativeEvent::Press)
			.count(),
		3
	);
	assert!(events.borrow().contains(&NativeEvent::Submit {
		value: "fixture only".to_owned()
	}));
}

#[test]
fn scroll_document_moves_and_reports_native_offsets() {
	let (window, mut surface) = mount(
		View().child(
			ScrollView()
				.class("w-48 h-24")
				.child(View().class("w-96 h-96 bg-blue-500")),
		),
	);
	let id = child(&surface, 0);
	let view = surface.backend().view(id).unwrap();
	let document = surface.backend().scroll_document(id).unwrap();
	let events = Rc::new(RefCell::new(Vec::new()));
	let recorded = Rc::clone(&events);
	surface
		.backend_mut()
		.set_event_listener(Rc::new(move |_, event| recorded.borrow_mut().push(event)));
	// SAFETY: A standard wheel message scrolls the actual viewport
	// implementation.
	unsafe {
		SendMessageW(
			view,
			WM_MOUSEWHEEL,
			Some(WPARAM(((-120_i16) as u16 as usize) << 16)),
			None,
		);
	}
	window.pump_messages();
	assert!(
		events
			.borrow()
			.iter()
			.any(|event| matches!(event, NativeEvent::Scroll { offset_y, .. } if *offset_y > 0.0))
	);
	let mut origin = POINT::default();
	// SAFETY: Mapping from the document's origin into its viewport is valid.
	unsafe {
		MapWindowPoints(
			Some(document),
			Some(view),
			std::slice::from_mut(&mut origin),
		);
	}
	assert!(origin.y < 0);
	// SAFETY: Standard scrollbar commands move to the actual final positions.
	unsafe {
		SendMessageW(view, WM_HSCROLL, Some(WPARAM(SB_BOTTOM.0 as usize)), None);
		SendMessageW(view, WM_VSCROLL, Some(WPARAM(SB_BOTTOM.0 as usize)), None);
		let mut client = RECT::default();
		let mut document_bounds = RECT::default();
		GetClientRect(view, &raw mut client).unwrap();
		GetWindowRect(document, &raw mut document_bounds).unwrap();
		let mut corner = POINT {
			x: document_bounds.right,
			y: document_bounds.bottom,
		};
		let _ = ScreenToClient(view, &raw mut corner);
		assert_eq!(corner.x, client.right);
		assert_eq!(corner.y, client.bottom);
		assert_ne!(GetWindowLongW(view, GWL_STYLE) as u32 & WS_HSCROLL.0, 0);
		assert_ne!(GetWindowLongW(view, GWL_STYLE) as u32 & WS_VSCROLL.0, 0);
	}
}

#[test]
fn scroll_extent_includes_visible_overflow_of_descendants() {
	let (window, surface) = mount(
		View().child(
			ScrollView().class("w-48 h-24").child(
				View()
					.class("relative w-24 h-12")
					.child(View().class("absolute left-0 top-40 w-8 h-8 bg-blue-500")),
			),
		),
	);
	let view = surface.backend().view(child(&surface, 0)).unwrap();
	let mut info = SCROLLINFO {
		cbSize: size_of::<SCROLLINFO>() as u32,
		fMask: SIF_RANGE,
		..Default::default()
	};
	// SAFETY: The output buffer matches SCROLLINFO's declared size.
	unsafe {
		GetScrollInfo(view, SB_VERT, &raw mut info).unwrap();
		assert_ne!(GetWindowLongW(view, GWL_STYLE) as u32 & WS_VSCROLL.0, 0);
	}
	// The grandchild overflows its 48px parent and ends at 160 + 32.
	let scale = window.dpi() as f32 / 96.0;
	assert_eq!(info.nMax + 1, (192.0 * scale).round() as i32);
}

#[test]
fn host_close_tolerates_removing_destroyed_children() {
	let (window, mut surface) = mount(View().children([Button("First"), Button("Second")]));
	// SAFETY: Closing the real top-level window destroys all its native
	// children.
	unsafe {
		SendMessageW(window.handle(), WM_CLOSE, None, None);
	}
	surface.set_root(&View().child(Button("First")));
	assert_eq!(surface.children(surface.root().unwrap()).len(), 1);
	drop(surface);
}

#[test]
fn host_close_invalidates_widgets_before_later_reconciliation() {
	let (window, mut surface) = mount(View().children([
		TextInput().aria_label("Editor").value("before"),
		ActivityIndicator(true),
	]));
	let input_id = child(&surface, 0);
	let old_input = surface.backend().control(input_id).unwrap();
	// SAFETY: Closing the real top-level window destroys all its native
	// children.
	unsafe {
		SendMessageW(window.handle(), WM_CLOSE, None, None);
	}
	assert!(surface.backend().control(input_id).is_none());
	assert!(surface.backend().view(surface.root().unwrap()).is_none());
	// Changed props and a new child after closure must never send messages to a
	// stale or reused HWND. Fresh nodes may remain parked until a new host
	// mounts.
	surface.set_root(&View().children([
		TextInput().aria_label("Editor").value("after"),
		ActivityIndicator(false),
		Button("New parked control"),
	]));
	surface.set_dark_mode(true);
	drop(surface);
	// SAFETY: IsWindow accepts a formerly valid HWND without dereferencing it.
	unsafe {
		assert!(!IsWindow(Some(old_input)).as_bool());
	}
}

#[test]
fn monitor_dpi_changes_convert_frames_fonts_and_resize_to_logical_units() {
	let (window, surface) = mount(
		View().child(
			TextInput()
				.aria_label("Scaled")
				.value("Scaled")
				.class("w-40 h-10"),
		),
	);
	let id = child(&surface, 0);
	let frame = surface.backend().view(id).unwrap();
	let control = surface.backend().control(id).unwrap();
	let sizes = Rc::new(RefCell::new(Vec::new()));
	let recorded = Rc::clone(&sizes);
	window.set_resize_listener(Rc::new(move |width, height, dpi| {
		recorded.borrow_mut().push((width, height, dpi));
	}));
	let mut suggested = RECT::default();
	// WM_DPICHANGED carries the new horizontal DPI in its low word and the
	// vertical DPI in its high word.
	let dpi: usize = 144;
	// SAFETY: Exercise the production WM_DPICHANGED handler against the actual
	// HWND. Windows CI need not have a second high-DPI monitor attached.
	unsafe {
		GetWindowRect(window.handle(), &raw mut suggested).unwrap();
		suggested.right = suggested.left + 900;
		suggested.bottom = suggested.top + 1000;
		SendMessageW(
			window.handle(),
			WM_DPICHANGED,
			Some(WPARAM(dpi | (dpi << 16))),
			Some(LPARAM((&raw const suggested) as isize)),
		);
		let mut bounds = RECT::default();
		GetWindowRect(frame, &raw mut bounds).unwrap();
		assert_eq!(bounds.right - bounds.left, 240);
		assert_eq!(bounds.bottom - bounds.top, 60);
		let font = SendMessageW(control, WM_GETFONT, None, None);
		let mut description = LOGFONTW::default();
		assert_ne!(
			GetObjectW(
				HGDIOBJ(font.0 as *mut _),
				size_of::<LOGFONTW>() as i32,
				Some((&raw mut description).cast())
			),
			0
		);
		assert_eq!(description.lfHeight, -21);
	}
	assert_eq!(window.dpi(), 144);
	let (width, height) = window.client_size().unwrap();
	let measured = sizes.borrow().last().copied().unwrap();
	assert_eq!(measured, (width, height, 144));
}

#[test]
fn clipping_opacity_animation_and_teardown_use_real_native_state() {
	let (window, mut surface) = mount(
		View().class("w-full h-full").children([
			View()
				.class("w-24 h-24 rounded-xl overflow-hidden opacity-50 bg-blue-500")
				.child(View().class("w-48 h-48 bg-red-500")),
			ActivityIndicator(true),
			LinearProgressIndicator(Progress::Indeterminate),
		]),
	);
	let clipped = surface.backend().view(child(&surface, 0)).unwrap();
	let composite = surface
		.backend()
		.composition_window(child(&surface, 0))
		.unwrap();
	let activity = child(&surface, 1);
	let progress = child(&surface, 2);
	// SAFETY: Region and layered-window queries observe the actual HWND state.
	unsafe {
		let region = CreateRectRgn(0, 0, 0, 0);
		assert_ne!(GetWindowRgn(clipped, region), GDI_REGION_TYPE(0));
		assert!(!PtInRegion(region, 0, 0).as_bool());
		let _ = DeleteObject(region.into());
		assert_ne!(
			GetWindowLongW(composite, GWL_EXSTYLE) as u32 & WS_EX_LAYERED.0,
			0
		);
		let mut alpha = 0;
		GetLayeredWindowAttributes(composite, None, Some(&raw mut alpha), None).unwrap();
		assert!((126..=129).contains(&alpha));
	}
	assert!(surface.backend().is_animating(activity));
	assert!(surface.backend().is_animating(progress));
	surface.set_root(
		&View()
			.class("w-full h-full")
			.with_props(|props| props.hidden = Some(true))
			.children([
				View(),
				ActivityIndicator(true),
				LinearProgressIndicator(Progress::Indeterminate),
			]),
	);
	assert!(!surface.backend().is_animating(activity));
	assert!(!surface.backend().is_animating(progress));
	surface.set_root(&View().children([
		View(),
		ActivityIndicator(false),
		LinearProgressIndicator(Progress::Determinate(ProgressValue::new(0.4).unwrap())),
	]));
	assert!(!surface.backend().is_animating(activity));
	assert!(!surface.backend().is_animating(progress));
	let root = surface.backend().view(surface.root().unwrap()).unwrap();
	drop(surface);
	window.pump_messages();
	// SAFETY: IsWindow handles destroyed HWNDs without dereferencing them.
	unsafe {
		assert!(!IsWindow(Some(root)).as_bool());
	}
}

#[test]
fn appearance_and_focused_editor_survive_updates_without_commit_events() {
	let (window, mut surface) = mount(View().children([
		TextInput().value("committed").accessibility_label("Editor"),
		Checkbox(false).accessibility_label("Choice"),
	]));
	let id = child(&surface, 0);
	let input = surface.backend().control(id).unwrap();
	let events = Rc::new(RefCell::new(Vec::new()));
	let recorded = Rc::clone(&events);
	surface
		.backend_mut()
		.set_event_listener(Rc::new(move |_, event| recorded.borrow_mut().push(event)));
	// SAFETY: This explicitly focuses an owned editor before a programmatic
	// update.
	unsafe {
		let _ = SetFocus(Some(input));
	}
	events.borrow_mut().clear();
	surface.set_dark_mode(true);
	surface.set_dark_mode(false);
	assert_eq!(surface.backend().control(id), Some(input));
	assert_eq!(text(input), "committed");
	// SAFETY: Theme changes must preserve the native active editor.
	unsafe {
		assert_eq!(GetFocus(), input);
	}
	surface.set_root(&View());
	window.pump_messages();
	assert!(events.borrow().is_empty());
}

#[test]
fn uia_hides_decorative_content_and_uses_the_shared_slider_domain() {
	let (window, surface) = mount(
		View().children([
			Pressable("One action")
				.class("w-48 h-10")
				.child(Text("Decorative description")),
			Slider(SliderState::new(-10.0, 10.0, 0.0, Some(2.0)).unwrap())
				.accessibility_label("Exact volume"),
		]),
	);
	let surface = Rc::new(RefCell::new(surface));
	let weak = Rc::downgrade(&surface);
	let events = Rc::new(RefCell::new(Vec::new()));
	let recorded = Rc::clone(&events);
	surface
		.borrow_mut()
		.backend_mut()
		.set_event_listener(Rc::new(move |_, event| {
			let remove = event == (NativeEvent::ValueChange { value: 4.0 });
			recorded.borrow_mut().push(event);

			if remove {
				weak.upgrade().unwrap().borrow_mut().set_root(&View());
			}
		}));
	let slider = surface
		.borrow()
		.backend()
		.control(child(&surface.borrow(), 1))
		.unwrap();
	let host_handle = window.handle().0 as usize;
	let slider_handle = slider.0 as usize;
	let (sender, receiver) = std::sync::mpsc::channel();
	// UI Automation clients belong on a separate MTA thread. Only HWND values
	// cross this OS API boundary; no backend, Rc, font, or widget moves
	// threads.
	let client = std::thread::spawn(move || {
		// SAFETY: COM is initialized on this client thread and balanced only
		// after all its automation interfaces have been dropped by the closure.
		unsafe {
			CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap();
			let result = (|| -> windows::core::Result<(Vec<String>, [f64; 4], f64, bool)> {
				let automation: IUIAutomation =
					CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
				let root = automation.ElementFromHandle(HWND(host_handle as *mut _))?;
				let elements =
					root.FindAll(TreeScope_Descendants, &automation.ControlViewCondition()?)?;
				let mut names = Vec::new();

				for index in 0..elements.Length()? {
					names.push(elements.GetElement(index)?.CurrentName()?.to_string());
				}

				let slider = automation.ElementFromHandle(HWND(slider_handle as *mut _))?;
				let range: IUIAutomationRangeValuePattern =
					slider.GetCurrentPatternAs(UIA_RangeValuePatternId)?;
				let domain = [
					range.CurrentMinimum()?,
					range.CurrentMaximum()?,
					range.CurrentSmallChange()?,
					range.CurrentValue()?,
				];
				range.SetValue(-3.0)?;
				let rejected = range.CurrentValue()?;
				range.SetValue(3.0)?;
				let removed = range.CurrentValue().is_err();
				Ok((names, domain, rejected, removed))
			})();
			CoUninitialize();
			sender
				.send(result.map_err(|error| error.to_string()))
				.unwrap();
		}
	});
	let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
	let result = loop {
		window.pump_messages();

		match receiver.try_recv() {
			Ok(result) => break result.unwrap(),
			Err(std::sync::mpsc::TryRecvError::Disconnected) => {
				panic!("UI Automation client terminated before reporting")
			}
			Err(std::sync::mpsc::TryRecvError::Empty) => {
				assert!(
					std::time::Instant::now() < deadline,
					"UI Automation client timed out"
				);
			}
		}

		std::thread::sleep(std::time::Duration::from_millis(1));
	};
	client.join().unwrap();
	let (names, domain, rejected, removed) = result;
	assert_eq!(
		names
			.iter()
			.filter(|name| name.as_str() == "One action")
			.count(),
		1
	);
	assert!(!names.iter().any(|name| name == "Decorative description"));
	assert_eq!(domain, [-10.0, 10.0, 2.0, 0.0]);
	assert_eq!(rejected, 0.0);
	assert!(removed);
	assert!(
		events
			.borrow()
			.contains(&NativeEvent::ValueChange { value: -2.0 })
	);
	assert!(
		events
			.borrow()
			.contains(&NativeEvent::ValueChange { value: 4.0 })
	);
}

#[test]
fn text_property_initializes_both_editor_kinds_without_a_value_property() {
	let (_window, mut surface) = mount(
		View().children([
			TextInput()
				.aria_label("Editor")
				.with_props(|props| props.text = Some("initial".to_owned())),
			PasswordInput()
				.aria_label("Password")
				.with_props(|props| props.text = Some("initial".to_owned())),
		]),
	);
	surface.backend_mut().set_event_listener(Rc::new(|_, _| {}));

	for index in 0..2 {
		let control = surface.backend().control(child(&surface, index)).unwrap();
		assert_eq!(text(control), "initial");
		// SAFETY: A real edit proposal is rejected and restores the text
		// fallback.
		unsafe {
			SetWindowTextW(control, w!("proposal")).unwrap();
		}
		assert_eq!(text(control), "initial");
	}
}

#[test]
fn text_direction_and_logical_alignment_update_native_editors() {
	let app = |direction| {
		View().children([
			TextInput()
				.aria_label("Editor")
				.value("مرحبا")
				.with_style(|style| style.direction = direction),
			PasswordInput()
				.aria_label("Password")
				.value("fixture")
				.with_style(|style| style.direction = direction),
		])
	};
	let (_window, mut surface) = mount(app(leptoskit_yoga::Direction::Rtl));
	let handles: Vec<_> = (0..2)
		.map(|index| surface.backend().control(child(&surface, index)).unwrap())
		.collect();

	for handle in &handles {
		// SAFETY: Native styles expose the resolved writing direction without
		// mirroring Yoga's already-resolved physical frame coordinates.
		unsafe {
			assert_ne!(
				GetWindowLongW(*handle, GWL_EXSTYLE) as u32 & WS_EX_RTLREADING.0,
				0
			);
			assert_ne!(GetWindowLongW(*handle, GWL_STYLE) & ES_RIGHT, 0);
		}
	}

	surface.set_root(&app(leptoskit_yoga::Direction::Ltr));

	for (index, handle) in handles.into_iter().enumerate() {
		assert_eq!(
			surface.backend().control(child(&surface, index)),
			Some(handle)
		);
		// SAFETY: Replacing direction restores the native leading alignment.
		unsafe {
			assert_eq!(
				GetWindowLongW(handle, GWL_EXSTYLE) as u32 & WS_EX_RTLREADING.0,
				0
			);
			assert_eq!(GetWindowLongW(handle, GWL_STYLE) & ES_RIGHT, 0);
		}
	}
}

#[test]
fn button_text_centers_by_default_and_honors_explicit_alignment_in_pixels() {
	let app = |align| {
		View().class("w-full h-full bg-white p-4").child(
			Button("X")
				.class("w-40 h-10 bg-white text-black")
				.with_props(|props| props.text_align = align),
		)
	};
	let (window, mut surface) = mount(app(None));
	let id = child(&surface, 0);
	let frame = surface.backend().view(id).unwrap();

	let ink = || {
		window.pump_messages();
		std::thread::sleep(std::time::Duration::from_millis(50));
		let image = image::load_from_memory(&window.snapshot_png().unwrap())
			.unwrap()
			.into_rgba8();
		let mut bounds = RECT::default();
		let mut origin = POINT::default();
		// SAFETY: Crop only the native button inside the fixture's own window.
		unsafe {
			GetWindowRect(frame, &raw mut bounds).unwrap();
			let _ = ClientToScreen(window.handle(), &raw mut origin);
		}
		let mut horizontal = Vec::new();

		for y in bounds.top..bounds.bottom {
			for x in bounds.left..bounds.right {
				let color = image.get_pixel((x - origin.x) as u32, (y - origin.y) as u32);

				if color.0[..3].iter().all(|channel| *channel < 80) {
					horizontal.push(x - bounds.left);
				}
			}
		}

		let scale = window.dpi() as f32 / 96.0;
		(
			*horizontal.iter().min().expect("button paints its text") as f32 / scale,
			*horizontal.iter().max().unwrap() as f32 / scale,
		)
	};
	let (center_left, center_right) = ink();
	assert!((center_left.midpoint(center_right) - 80.0).abs() < 4.0);
	surface.set_root(&app(Some(leptoskit_native::TextAlign::Start)));
	let (start, _) = ink();
	assert!(start < 5.0);
	surface.set_root(&app(Some(leptoskit_native::TextAlign::End)));
	let (_, end) = ink();
	assert!(end > 150.0);
	assert_eq!(surface.backend().view(id), Some(frame));
}

#[test]
fn disabled_and_hidden_ancestors_block_native_control_proposals() {
	let app = |enabled, hidden| {
		View()
			.enabled(enabled)
			.with_props(|props| props.hidden = Some(hidden))
			.children([
				Checkbox(false),
				Slider(SliderState::default()),
				TextInput().aria_label("Editor").value("committed"),
			])
	};
	let (_window, mut surface) = mount(app(false, false));
	let controls: Vec<_> = (0..3)
		.map(|index| surface.backend().control(child(&surface, index)).unwrap())
		.collect();
	let events = Rc::new(RefCell::new(Vec::new()));
	let recorded = Rc::clone(&events);
	surface
		.backend_mut()
		.set_event_listener(Rc::new(move |_, event| recorded.borrow_mut().push(event)));

	for control in &controls {
		// SAFETY: Native enabled state must reflect the logical ancestor.
		unsafe {
			assert!(!IsWindowEnabled(*control).as_bool());
		}
	}
	click(controls[0]);
	key(controls[1], VK_RIGHT);
	// SAFETY: Even a programmatic native edit cannot escape disabled ancestry.
	unsafe {
		SetWindowTextW(controls[2], w!("proposal")).unwrap();
	}
	assert_eq!(text(controls[2]), "committed");
	assert!(events.borrow().is_empty());
	surface.set_root(&app(true, true));
	click(controls[0]);
	key(controls[1], VK_RIGHT);
	key(controls[2], VK_RETURN);
	assert!(events.borrow().is_empty());
	// SAFETY: Native ancestor visibility also removes the controls from
	// display.
	unsafe {
		assert!(!IsWindowVisible(controls[0]).as_bool());
	}
}

const RED: Color = Color::rgba(255, 0, 0, 255);
const BLUE: Color = Color::rgba(0, 0, 255, 255);

fn ten_by_ten() -> ViewBox {
	ViewBox::new(0.0, 0.0, 10.0, 10.0).unwrap()
}

fn path(data: &str) -> VectorPath {
	VectorPath::new(PathData::parse(data).unwrap())
}

fn square() -> VectorPath {
	path("M 0 0 L 10 0 L 10 10 L 0 10 Z")
}

#[test]
fn vector_fit_centres_and_stretch_fills_the_frame_in_real_pixels() {
	let (window, surface) = mount(
		View().class("w-full h-full bg-white p-4 gap-4").children([
			Vector(ten_by_ten())
				.class("w-20 h-10")
				.path(square().fill(RED)),
			Vector(ten_by_ten())
				.class("w-20 h-10")
				.scaling(VectorScaling::Stretch)
				.path(square().fill(RED)),
		]),
	);
	let fit = surface.backend().view(child(&surface, 0)).unwrap();
	let stretch = surface.backend().view(child(&surface, 1)).unwrap();
	// Fit scales the square by four and centres it: x 20 to 60 of 80.
	near(pixel(&window, fit, 40.0, 20.0), [255, 0, 0, 255]);
	near(pixel(&window, fit, 10.0, 20.0), [255, 255, 255, 255]);
	near(pixel(&window, fit, 70.0, 20.0), [255, 255, 255, 255]);
	near(pixel(&window, stretch, 10.0, 20.0), [255, 0, 0, 255]);
	near(pixel(&window, stretch, 70.0, 20.0), [255, 0, 0, 255]);
}

#[test]
fn vector_strokes_keep_layout_widths_and_paint_over_nonzero_fills() {
	let (window, surface) = mount(
		View().class("w-full h-full bg-white p-4 gap-4").children([
			// Stretched four times vertically; a scaled stroke would be 16
			// units tall rather than 4.
			Vector(ten_by_ten())
				.class("w-20 h-10")
				.scaling(VectorScaling::Stretch)
				.path(path("M 0 5 L 10 5").stroke(Stroke::new(BLUE, 4.0).unwrap())),
			Vector(ten_by_ten())
				.class("w-20 h-20")
				.path(square().fill(RED).stroke(Stroke::new(BLUE, 8.0).unwrap())),
			// Both squares wind the same way, so nonzero fills the inner one
			// where even-odd would leave a hole.
			Vector(ten_by_ten())
				.class("w-20 h-20")
				.path(path("M 0 0 L 10 0 L 10 10 L 0 10 Z M 2 2 L 8 2 L 8 8 L 2 8 Z").fill(RED)),
		]),
	);
	let line = surface.backend().view(child(&surface, 0)).unwrap();
	let outlined = surface.backend().view(child(&surface, 1)).unwrap();
	let nested = surface.backend().view(child(&surface, 2)).unwrap();
	near(pixel(&window, line, 40.0, 20.0), [0, 0, 255, 255]);
	near(pixel(&window, line, 40.0, 14.0), [255, 255, 255, 255]);
	near(pixel(&window, line, 40.0, 26.0), [255, 255, 255, 255]);
	// The stroke straddles the outline and paints over the fill.
	near(pixel(&window, outlined, 2.0, 40.0), [0, 0, 255, 255]);
	near(pixel(&window, outlined, 40.0, 40.0), [255, 0, 0, 255]);
	near(pixel(&window, nested, 40.0, 40.0), [255, 0, 0, 255]);
}

#[test]
fn vector_ink_overflows_its_frame_unless_the_element_clips() {
	fn app(clip: bool) -> Element {
		View().class("w-full h-full bg-white p-8").child(
			Vector(ten_by_ten())
				.class("w-20 h-20")
				.with_props(|props| props.clip = Some(clip))
				.path(path("M 0 0 L 10 0").stroke(Stroke::new(BLUE, 8.0).unwrap())),
		)
	}

	let (window, mut surface) = mount(app(false));
	let id = child(&surface, 0);
	let frame = surface.backend().view(id).unwrap();
	near(pixel(&window, frame, 40.0, 2.0), [0, 0, 255, 255]);
	near(pixel(&window, frame, 40.0, -2.0), [0, 0, 255, 255]);
	near(pixel(&window, frame, 40.0, 10.0), [255, 255, 255, 255]);
	let mut point = POINT { x: 40, y: -2 };
	// SAFETY: Hit-testing uses screen coordinates of the live frame. Ink past
	// the frame is not a hit target, as on the web.
	unsafe {
		let _ = ClientToScreen(frame, &raw mut point);
		assert_ne!(
			WindowFromPoint(point),
			surface.backend().composition_window(id).unwrap()
		);
	}
	surface.set_root(&app(true));
	assert_eq!(surface.backend().view(id), Some(frame));
	near(pixel(&window, frame, 40.0, 2.0), [0, 0, 255, 255]);
	near(pixel(&window, frame, 40.0, -2.0), [255, 255, 255, 255]);
}

#[test]
fn transparent_vectors_reveal_siblings_beneath_unpainted_areas() {
	let (window, surface) = mount(
		View().class("relative w-full h-full bg-white").children([
			View().class("absolute left-4 top-4 w-20 h-20 bg-red-500"),
			Vector(ten_by_ten())
				.class("absolute left-4 top-4 w-20 h-20")
				.path(path("M 0 0 L 5 0 L 5 5 L 0 5 Z").fill(BLUE)),
		]),
	);
	let vector = surface.backend().view(child(&surface, 1)).unwrap();
	near(pixel(&window, vector, 20.0, 20.0), [0, 0, 255, 255]);
	near(pixel(&window, vector, 60.0, 60.0), [251, 44, 54, 255]);
}

#[test]
fn labelled_vectors_are_graphics_and_unlabelled_vectors_are_hidden() {
	let (_window, mut surface) = mount(
		View().children([
			Vector(ten_by_ten())
				.class("w-6 h-6")
				.accessibility_label("Signature")
				.path(square().fill(RED)),
			Vector(ten_by_ten())
				.class("w-6 h-6")
				.path(square().fill(RED)),
		]),
	);
	let labelled = surface.backend().view(child(&surface, 0)).unwrap();
	let decorative = surface.backend().view(child(&surface, 1)).unwrap();
	let child_self = VARIANT::from(CHILDID_SELF as i32);
	let invisible = |hwnd| {
		// SAFETY: The frame exposes a standard MSAA object with an annotated
		// state.
		let state = unsafe { accessible(hwnd).get_accState(&child_self).unwrap() };
		i32::try_from(&state).unwrap() as u32 & STATE_SYSTEM_INVISIBLE.0 != 0
	};
	// SAFETY: The frame exposes a standard MSAA object with annotated
	// properties.
	unsafe {
		assert_eq!(
			accessible(labelled)
				.get_accName(&child_self)
				.unwrap()
				.to_string(),
			"Signature"
		);
		assert_eq!(
			u32::try_from(&accessible(labelled).get_accRole(&child_self).unwrap()).unwrap(),
			ROLE_SYSTEM_GRAPHIC
		);
	}
	assert!(!invisible(labelled));
	assert!(invisible(decorative));

	// Naming a drawing exposes it; removing the name hides it again.
	surface.set_root(
		&View().children([
			Vector(ten_by_ten())
				.class("w-6 h-6")
				.path(square().fill(RED)),
			Vector(ten_by_ten())
				.class("w-6 h-6")
				.accessibility_label("Initials")
				.path(square().fill(RED)),
		]),
	);
	assert!(invisible(labelled));
	assert!(!invisible(decorative));
}

#[test]
fn held_buttons_follow_the_native_pushed_state_with_active_styles() {
	let (window, surface) = mount(View().class("p-4 gap-4").children([
		Button("Hold").class("w-40 h-12 bg-blue-600 active:bg-blue-800"),
		Pressable("Hold card").class("w-40 h-12 bg-blue-600"),
	]));
	let surface = Rc::new(RefCell::new(surface));
	let events = Rc::new(RefCell::new(Vec::new()));
	let (weak, recorded) = (Rc::downgrade(&surface), Rc::clone(&events));
	surface
		.borrow_mut()
		.backend_mut()
		.set_event_listener(Rc::new(move |id, event| {
			recorded.borrow_mut().push((id, event.clone()));
			let surface = weak.upgrade().unwrap();
			match event {
				NativeEvent::PressIn => surface.borrow_mut().press_in(id),
				NativeEvent::PressOut => surface.borrow_mut().press_out(id),
				_ => {}
			}
		}));
	let scale = window.dpi() as f32 / 96.0;
	let center = LPARAM((((24.0 * scale) as isize) << 16) | (80.0 * scale) as isize);
	let hold = |id| {
		let control = surface.borrow().backend().control(id).unwrap();
		// SAFETY: Synchronous pointer messages reach the real BUTTON as a click
		// delivers them; MK_LBUTTON is 1. Only the listener borrows the surface
		// while they are processed.
		unsafe {
			SendMessageW(control, WM_LBUTTONDOWN, Some(WPARAM(1)), Some(center));
		}
		control
	};
	let release = |control| {
		// SAFETY: As above, releasing inside the button's client area.
		unsafe {
			SendMessageW(control, WM_LBUTTONUP, None, Some(center));
		}
	};
	let button_id = child(&surface.borrow(), 0);
	let resting = surface.borrow().props(button_id).background;
	let control = hold(button_id);
	assert_eq!(surface.borrow().pressed(), Some(button_id));
	let pressed = surface.borrow().props(button_id).background;
	assert!(
		pressed.is_some() && pressed != resting,
		"active:bg-blue-800 applies while held"
	);
	assert_eq!(
		surface.borrow().props(button_id).accessible_name(),
		Some("Hold")
	);
	assert_eq!(surface.borrow().backend().control(button_id), Some(control));
	release(control);
	assert_eq!(surface.borrow().pressed(), None);
	assert_eq!(surface.borrow().props(button_id).background, resting);
	assert!(events.borrow().contains(&(button_id, NativeEvent::Press)));

	let pressable_id = child(&surface.borrow(), 1);
	let control = hold(pressable_id);
	assert_eq!(surface.borrow().props(pressable_id).opacity, Some(0.7));
	release(control);
	assert_eq!(surface.borrow().props(pressable_id).opacity, None);
	let phases = events
		.borrow()
		.iter()
		.filter(|(_, event)| matches!(event, NativeEvent::PressIn | NativeEvent::PressOut))
		.map(|(id, event)| (*id, event.clone()))
		.collect::<Vec<_>>();
	assert_eq!(
		phases,
		[
			(button_id, NativeEvent::PressIn),
			(button_id, NativeEvent::PressOut),
			(pressable_id, NativeEvent::PressIn),
			(pressable_id, NativeEvent::PressOut),
		]
	);
}
