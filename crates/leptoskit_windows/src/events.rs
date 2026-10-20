//! Retained subclass callbacks, controlled proposals, and primitive painting.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;
use std::rc::Weak;

use leptoskit_native::Color;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_yoga::NodeId;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Accessibility::IRawElementProviderSimple;
use windows::Win32::UI::Accessibility::UiaReturnRawElementProvider;
use windows::Win32::UI::Accessibility::UiaRootObjectId;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::DefSubclassProc;
use windows::Win32::UI::Shell::RemoveWindowSubclass;
use windows::Win32::UI::Shell::SetWindowSubclass;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::accessibility::DecorativeProvider;
use crate::accessibility::SliderProvider;
use crate::backend::SLIDER_TICKS;
use crate::backend::Widget;
use crate::backend::window_text;
use crate::drawing::Brush;
use crate::drawing::rgb;
use crate::drawing::text_flags;

/// A UI-thread callback receiving a native user proposal.
pub type EventListener = Rc<dyn Fn(NodeId, NativeEvent)>;

#[derive(Default)]
pub(crate) struct EventSink {
	pub listener: RefCell<Option<EventListener>>,
	suppressed: Cell<usize>,
}

pub(crate) struct Suppression(Rc<EventSink>);

impl EventSink {
	pub fn suppress(self: &Rc<Self>) -> Suppression {
		self.suppressed.set(self.suppressed.get() + 1);
		Suppression(Rc::clone(self))
	}

	pub fn emit(&self, widget: &Widget, event: NativeEvent) {
		if self.suppressed.get() != 0 || !widget.interactive() {
			return;
		}

		let listener = self.listener.borrow().clone();

		if let Some(listener) = listener {
			listener(widget.id, event);
		}
	}
}

impl Drop for Suppression {
	fn drop(&mut self) {
		self.0.suppressed.set(self.0.suppressed.get() - 1);
	}
}

struct Context {
	widget: Weak<Widget>,
	control: bool,
}

pub(crate) fn install_subclass(widget: &Rc<Widget>, hwnd: HWND, control: bool) {
	let context = Rc::new(Context {
		widget: Rc::downgrade(widget),
		control,
	});
	let retained = Rc::into_raw(context);
	// SAFETY: The subclass owns this one Rc until WM_NCDESTROY. Callbacks take
	// another Rc before dispatch, so reentrant destruction cannot free it
	// early.
	if !unsafe { SetWindowSubclass(hwnd, Some(subclass), 1, retained as usize) }.as_bool() {
		// SAFETY: Installation failed, so no callback owns the transferred Rc.
		unsafe {
			drop(Rc::from_raw(retained));
		}
		panic!("Windows could not install a native widget subclass");
	}
}

unsafe extern "system" fn subclass(
	hwnd: HWND,
	message: u32,
	wparam: WPARAM,
	lparam: LPARAM,
	_id: usize,
	data: usize,
) -> LRESULT {
	let pointer = data as *const Context;
	// SAFETY: SetWindowSubclass installed an Rc pointer. Keep an extra
	// reference across callbacks, which may synchronously destroy this
	// exact HWND.
	let context = unsafe {
		Rc::increment_strong_count(pointer);
		Rc::from_raw(pointer)
	};

	if message == WM_NCDESTROY {
		if let Some(widget) = context.widget.upgrade() {
			widget.alive.set(false);
			widget.animating.set(false);
			if hwnd == widget.frame {
				widget.frame_alive.set(false);
			}
			if hwnd == widget.composition {
				widget.composition_alive.set(false);
			}
			if Some(hwnd) == widget.control {
				widget.control_alive.set(false);
			}
			if Some(hwnd) == widget.document {
				widget.document_alive.set(false);
			}
		}
		// SAFETY: This is the subclass's final message; release its installed
		// Rc exactly once, while the local reference keeps the context
		// alive.
		unsafe {
			let _ = RemoveWindowSubclass(hwnd, Some(subclass), 1);
			drop(Rc::from_raw(pointer));
			return DefSubclassProc(hwnd, message, wparam, lparam);
		}
	}

	let Some(widget) = context.widget.upgrade() else {
		// SAFETY: Forwarding untouched arguments to the next native procedure.
		return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
	};

	if !widget.alive.get() {
		// SAFETY: Destruction messages continue through the original procedure.
		return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
	}

	if message == WM_NCHITTEST && widget.decorative() {
		return LRESULT(HTTRANSPARENT as isize);
	}

	if message == WM_GETOBJECT
		&& lparam.0 as i32 == UiaRootObjectId
		&& (widget.decorative() || hwnd == widget.composition)
	{
		let provider: IRawElementProviderSimple = DecorativeProvider {
			widget: Rc::downgrade(&widget),
			hwnd,
		}
		.into();
		// SAFETY: UI Automation retains the COM provider for as long as needed;
		// the provider holds only a weak widget and rejects calls after
		// teardown.
		return unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &provider) };
	}

	if message == WM_GETOBJECT
		&& lparam.0 as i32 == UiaRootObjectId
		&& context.control
		&& widget.kind == NativeKind::Slider
	{
		let provider: IRawElementProviderSimple = SliderProvider {
			widget: Rc::downgrade(&widget),
			hwnd,
		}
		.into();
		// SAFETY: UIA retains the provider; proposals use the shared f64 state,
		// never the trackbar's internal normalized integer coordinate range.
		return unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &provider) };
	}

	if hwnd == widget.composition {
		if message == WM_PAINT {
			let mut paint = PAINTSTRUCT::default();
			// SAFETY: Group pixels are covered by the union of its frame and
			// child groups. Validate without adding an opaque
			// composition backdrop.
			unsafe {
				BeginPaint(hwnd, &raw mut paint);
				let _ = EndPaint(hwnd, &raw const paint);
			}
			return LRESULT(0);
		}

		// SAFETY: Actual widget input/paint belongs to the Yoga-sized frame.
		return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
	}

	if context.control {
		if let Some(result) = control_message(&widget, hwnd, message, wparam, lparam) {
			return result;
		}
	} else if let Some(result) = frame_message(&widget, hwnd, message, wparam, lparam) {
		return result;
	}

	// SAFETY: No borrowed widget state crosses the native default procedure.
	unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn control_message(
	widget: &Rc<Widget>,
	hwnd: HWND,
	message: u32,
	wparam: WPARAM,
	lparam: LPARAM,
) -> Option<LRESULT> {
	if matches!(message, WM_SETFOCUS | WM_KILLFOCUS) {
		// SAFETY: Complete native focus processing before the callback can
		// remove the control. No HWND is accessed after the listener
		// runs.
		let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
		widget.sink.emit(
			widget,
			if message == WM_SETFOCUS {
				NativeEvent::Focus
			} else {
				NativeEvent::Blur
			},
		);
		return Some(result);
	}

	if message == WM_GETDLGCODE {
		// SAFETY: Querying native keyboard preferences with the original
		// message.
		let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
		let wants_return = wparam.0 == usize::from(VK_RETURN.0)
			&& matches!(
				widget.kind,
				NativeKind::TextInput
					| NativeKind::PasswordInput
					| NativeKind::Button
					| NativeKind::Pressable
			);
		let selective_message = if wants_return { DLGC_WANTMESSAGE } else { 0 };
		return Some(LRESULT(
			result.0
				| DLGC_WANTARROWS as isize
				| DLGC_WANTCHARS as isize
				| selective_message as isize,
		));
	}

	if message != WM_KEYDOWN {
		return None;
	}

	let key = wparam.0 as u16;

	if matches!(widget.kind, NativeKind::Button | NativeKind::Pressable) && key == VK_RETURN.0 {
		if lparam.0 & (1 << 30) == 0 {
			propose(widget, NativeEvent::Press);
		}
		return Some(LRESULT(0));
	}

	if matches!(
		widget.kind,
		NativeKind::TextInput | NativeKind::PasswordInput
	) && key == VK_RETURN.0
	{
		propose(
			widget,
			NativeEvent::Submit {
				value: window_text(hwnd),
			},
		);
		return Some(LRESULT(0));
	}

	if widget.kind == NativeKind::Radio
		&& [VK_LEFT.0, VK_RIGHT.0, VK_UP.0, VK_DOWN.0].contains(&key)
	{
		radio_arrow(
			widget,
			matches!(key, key if key == VK_LEFT.0 || key == VK_UP.0),
		);
		return Some(LRESULT(0));
	}

	if widget.kind != NativeKind::Slider {
		return None;
	}

	let state = widget.props.borrow().slider.unwrap_or_default();
	let increment = state.step().unwrap_or((state.max() - state.min()) / 100.0);
	let next = match key {
		key if key == VK_HOME.0 => state.min(),
		key if key == VK_END.0 => state.max(),
		key if key == VK_LEFT.0 || key == VK_DOWN.0 => (-increment).mul_add(1.0, state.value()),
		key if key == VK_RIGHT.0 || key == VK_UP.0 => increment.mul_add(1.0, state.value()),
		key if key == VK_PRIOR.0 => increment.mul_add(10.0, state.value()),
		key if key == VK_NEXT.0 => (-increment).mul_add(10.0, state.value()),
		_ => return None,
	};

	if let Some(value) = state.proposed_value(next) {
		propose(widget, NativeEvent::ValueChange { value });
	}

	Some(LRESULT(0))
}

fn propose(widget: &Rc<Widget>, event: NativeEvent) {
	widget.sink.emit(widget, event);
	widget.restore_control();
}

fn radio_arrow(widget: &Rc<Widget>, backward: bool) {
	if !widget.interactive() {
		return;
	}

	let group = widget.props.borrow().radio_group.clone();
	let mut root = Rc::clone(widget);

	loop {
		let parent = root.parent.borrow().upgrade();
		let Some(parent) = parent else { break };
		root = parent;
	}

	fn collect(
		widget: &Rc<Widget>,
		group: Option<&leptoskit_native::RadioGroup>,
		members: &mut Vec<Rc<Widget>>,
	) {
		if widget.kind == NativeKind::Radio
			&& widget.props.borrow().radio_group.as_ref() == group
			&& widget.interactive()
		{
			members.push(Rc::clone(widget));
		}

		for child in widget.children.borrow().iter() {
			collect(child, group, members);
		}
	}

	let mut members = Vec::new();
	collect(&root, group.as_ref(), &mut members);
	let Some(index) = members.iter().position(|member| member.id == widget.id) else {
		return;
	};
	let next = if backward {
		(index + members.len() - 1) % members.len()
	} else {
		(index + 1) % members.len()
	};
	let target = Rc::clone(&members[next]);

	if let Some(control) = target.control {
		// SAFETY: The member remains retained during focus callbacks. Those may
		// remove it, which propose's live-state check handles before any
		// access.
		unsafe {
			let _ = SetFocus(Some(control));
		}
	}

	propose(&target, NativeEvent::Toggle { checked: true });
}

fn frame_message(
	widget: &Rc<Widget>,
	hwnd: HWND,
	message: u32,
	wparam: WPARAM,
	lparam: LPARAM,
) -> Option<LRESULT> {
	match message {
		WM_NOTIFY if lparam.0 != 0 => {
			// SAFETY: WM_NOTIFY carries an NMHDR with a lifetime spanning this
			// synchronous call; NM_CUSTOMDRAW extends it with NMCUSTOMDRAW.
			let header = unsafe { &*(lparam.0 as *const NMHDR) };

			if widget.control == Some(header.hwndFrom)
				&& header.code == NM_CUSTOMDRAW
				&& matches!(
					widget.kind,
					NativeKind::Button
						| NativeKind::Pressable
						| NativeKind::Switch
						| NativeKind::Checkbox
						| NativeKind::Radio
				) {
				// SAFETY: The notification code establishes the concrete
				// payload.
				let draw = unsafe { &*(lparam.0 as *const NMCUSTOMDRAW) };

				if draw.dwDrawStage == CDDS_PREPAINT {
					let native_button = widget.kind == NativeKind::Button && {
						let props = widget.props.borrow();
						props.background.is_none()
							&& props.color.is_none()
							&& props.border_width.is_none()
							&& props.corner_radius.is_none()
					};

					if native_button {
						return None;
					}

					paint_button(widget, draw);
					return Some(LRESULT(CDRF_SKIPDEFAULT as isize));
				}
			}

			None
		}
		WM_COMMAND
			if widget
				.control
				.is_some_and(|control| control.0 as isize == lparam.0) =>
		{
			let notification = ((wparam.0 >> 16) & 0xffff) as u32;
			let control = widget.control.expect("matched control");
			let event = match (widget.kind, notification) {
				(NativeKind::Button | NativeKind::Pressable, BN_CLICKED) => {
					Some(NativeEvent::Press)
				}
				(NativeKind::Switch | NativeKind::Checkbox, BN_CLICKED) => {
					Some(NativeEvent::Toggle {
						checked: !widget.props.borrow().checked.unwrap_or(false),
					})
				}
				(NativeKind::Radio, BN_CLICKED) => Some(NativeEvent::Toggle { checked: true }),
				(NativeKind::TextInput | NativeKind::PasswordInput, EN_CHANGE) => {
					Some(NativeEvent::Change {
						value: window_text(control),
					})
				}
				_ => None,
			};

			if let Some(event) = event {
				propose(widget, event);
			}

			Some(LRESULT(0))
		}
		WM_HSCROLL if widget.kind == NativeKind::Slider && lparam.0 != 0 => {
			let state = widget.props.borrow().slider.unwrap_or_default();
			// SAFETY: This is a normalized integer coordinate, never the shared
			// f64 value. Snapping always goes through SliderState.
			let coordinate = unsafe {
				SendMessageW(widget.control.expect("slider control"), WM_USER, None, None).0
			};
			let fraction = coordinate as f64 / f64::from(SLIDER_TICKS);
			let proposal = fraction.mul_add(state.max() - state.min(), state.min());

			if let Some(value) = state.proposed_value(proposal) {
				propose(widget, NativeEvent::ValueChange { value });
			}

			Some(LRESULT(0))
		}
		WM_VSCROLL | WM_HSCROLL | WM_MOUSEWHEEL | WM_MOUSEHWHEEL if widget.document.is_some() => {
			scroll(widget, message, wparam);
			Some(LRESULT(0))
		}
		WM_TIMER if wparam.0 == 1 => {
			widget.phase.set(widget.phase.get().wrapping_add(1));

			if widget.kind == NativeKind::LinearProgressIndicator && widget.animating.get() {
				// SAFETY: The native progress control is advanced by our one
				// owned timer so hiding or deletion stops all
				// animation work.
				unsafe {
					SendMessageW(
						widget.control.expect("progress control"),
						PBM_SETPOS,
						Some(WPARAM((widget.phase.get() as usize * 300) % 10_000)),
						None,
					);
				}
			}

			// SAFETY: Invalidation schedules painting without borrowed state.
			unsafe {
				let _ = InvalidateRect(Some(hwnd), None, false);
			}
			Some(LRESULT(0))
		}
		WM_ERASEBKGND => Some(LRESULT(1)),
		WM_PAINT => {
			paint(widget, hwnd);
			Some(LRESULT(0))
		}
		WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN => {
			let props = widget.props.borrow();
			let foreground = props.color.unwrap_or(if widget.dark.get() {
				Color::rgba(245, 245, 245, 255)
			} else {
				Color::rgba(20, 20, 20, 255)
			});
			let background = widget.background();
			// SAFETY: Windows supplied this DC for the synchronous color query.
			unsafe {
				SetTextColor(HDC(wparam.0 as *mut _), rgb(foreground));
				SetBkColor(HDC(wparam.0 as *mut _), rgb(background));
			}
			Some(LRESULT(widget.brush.borrow().0.0 as isize))
		}
		_ => None,
	}
}

fn scroll(widget: &Rc<Widget>, message: u32, wparam: WPARAM) {
	if !widget.interactive() {
		return;
	}

	let horizontal = message == WM_HSCROLL || message == WM_MOUSEHWHEEL;
	let axis = usize::from(!horizontal);
	let mut offset = widget.scroll.get();
	let previous = offset;
	let client = widget.viewport();
	let viewport = if horizontal {
		client.right - client.left
	} else {
		client.bottom - client.top
	};
	let maximum = (widget.extent.get()[axis] - viewport).max(0);

	if matches!(message, WM_MOUSEWHEEL | WM_MOUSEHWHEEL) {
		let delta = ((wparam.0 >> 16) as u16) as i16;
		offset[axis] += if horizontal {
			i32::from(delta)
		} else {
			-i32::from(delta)
		} * 48
			/ 120;
	} else {
		let command = (wparam.0 & 0xffff) as u32;
		offset[axis] = match command {
			value if value == SB_LINEUP.0 as u32 => offset[axis] - 20,
			value if value == SB_LINEDOWN.0 as u32 => offset[axis] + 20,
			value if value == SB_PAGEUP.0 as u32 => offset[axis] - viewport,
			value if value == SB_PAGEDOWN.0 as u32 => offset[axis] + viewport,
			value if value == SB_TOP.0 as u32 => 0,
			value if value == SB_BOTTOM.0 as u32 => maximum,
			value if value == SB_THUMBTRACK.0 as u32 || value == SB_THUMBPOSITION.0 as u32 => {
				let mut info = SCROLLINFO {
					cbSize: size_of::<SCROLLINFO>() as u32,
					fMask: SIF_TRACKPOS,
					..Default::default()
				};
				// SAFETY: The output buffer matches SCROLLINFO's declared size.
				unsafe {
					GetScrollInfo(
						widget.frame,
						if horizontal { SB_HORZ } else { SB_VERT },
						&raw mut info,
					)
					.expect("owned scroll window has scroll information");
				}
				info.nTrackPos
			}
			_ => return,
		};
	}

	offset[axis] = offset[axis].clamp(0, maximum);
	widget.scroll.set(offset);
	widget.refresh_scroll();

	if offset != previous {
		widget.sink.emit(
			widget,
			NativeEvent::Scroll {
				offset_x: offset[0] as f32 / widget.scale(),
				offset_y: offset[1] as f32 / widget.scale(),
			},
		);
	}
}

fn paint(widget: &Widget, hwnd: HWND) {
	let props = widget.props.borrow();
	let mut paint = PAINTSTRUCT::default();
	let mut bounds = RECT::default();
	// SAFETY: Begin/EndPaint bracket all use of this thread-local paint DC.
	unsafe {
		let dc = BeginPaint(hwnd, &raw mut paint);
		GetClientRect(hwnd, &raw mut bounds).expect("owned frame has a client rectangle");
		FillRect(dc, &raw const bounds, widget.brush.borrow().0);
		let previous_font = SelectObject(dc, widget.font.borrow().0.into());
		let foreground = props.color.unwrap_or(if widget.dark.get() {
			Color::rgba(245, 245, 245, 255)
		} else {
			Color::rgba(20, 20, 20, 255)
		});
		SetTextColor(dc, rgb(foreground));
		SetBkMode(dc, TRANSPARENT);

		if matches!(widget.kind, NativeKind::Text | NativeKind::TextRun) {
			let mut text: Vec<u16> = props.text.as_deref().unwrap_or("").encode_utf16().collect();
			DrawTextW(
				dc,
				&mut text,
				&raw mut bounds,
				text_flags(props.text_style()),
			);
		}

		if let Some(image) = widget.image.borrow().as_ref() {
			paint_image(dc, image, bounds.right, bounds.bottom);
		}

		if widget.kind == NativeKind::ActivityIndicator
			&& widget.props.borrow().running.unwrap_or(true)
		{
			let radius = f64::from(bounds.right.min(bounds.bottom).max(0)) * 0.34;

			for index in 0..12 {
				let angle = f64::from(index) * std::f64::consts::TAU / 12.0;
				let alpha = ((index + 12 - widget.phase.get() % 12) % 12 + 1) as u8;
				let background = widget.background();
				let channel = |value, backdrop| {
					((u32::from(value) * u32::from(alpha)
						+ u32::from(backdrop) * u32::from(12 - alpha))
						/ 12) as u8
				};
				let brush = Brush::new(Color::rgba(
					channel(foreground.red(), background.red()),
					channel(foreground.green(), background.green()),
					channel(foreground.blue(), background.blue()),
					255,
				));
				let previous = SelectObject(dc, brush.0.into());
				let x = (f64::from(bounds.right) / 2.0 + angle.cos() * radius).round() as i32;
				let y = (f64::from(bounds.bottom) / 2.0 + angle.sin() * radius).round() as i32;
				let _ = Ellipse(dc, x - 2, y - 2, x + 3, y + 3);
				SelectObject(dc, previous);
			}
		}

		let border = if widget.document == Some(hwnd) {
			0
		} else {
			(props.border_width.unwrap_or(0.0) * widget.scale())
				.round()
				.max(0.0) as i32
		};

		if border > 0 {
			let brush = Brush::new(props.border_color.unwrap_or(foreground));

			for inset in 0..border.min(bounds.right.min(bounds.bottom) / 2) {
				let rect = RECT {
					left: inset,
					top: inset,
					right: bounds.right - inset,
					bottom: bounds.bottom - inset,
				};
				FrameRect(dc, &raw const rect, brush.0);
			}
		}

		SelectObject(dc, previous_font);
		let _ = EndPaint(hwnd, &raw const paint);
	}
}

fn paint_button(widget: &Widget, draw: &NMCUSTOMDRAW) {
	let props = widget.props.borrow();
	let mut bounds = draw.rc;
	let foreground = props.color.unwrap_or(if widget.dark.get() {
		Color::rgba(245, 245, 245, 255)
	} else {
		Color::rgba(20, 20, 20, 255)
	});
	// SAFETY: The native custom-draw DC is borrowed for this synchronous call.
	unsafe {
		FillRect(draw.hdc, &raw const bounds, widget.brush.borrow().0);

		if matches!(widget.kind, NativeKind::Button | NativeKind::Pressable)
			&& (draw.uItemState & CDIS_SELECTED).0 != 0
		{
			let background = widget.background();
			let channel = |value| (u16::from(value) * 7 / 8) as u8;
			let pressed = Brush::new(Color::rgba(
				channel(background.red()),
				channel(background.green()),
				channel(background.blue()),
				255,
			));
			FillRect(draw.hdc, &raw const bounds, pressed.0);
		}
		let previous = SelectObject(draw.hdc, widget.font.borrow().0.into());
		SetTextColor(draw.hdc, rgb(foreground));
		SetBkMode(draw.hdc, TRANSPARENT);

		if matches!(
			widget.kind,
			NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio
		) {
			let edge = (bounds.bottom - bounds.top).min(20);
			let mut glyph = RECT {
				left: 0,
				top: (bounds.bottom - edge) / 2,
				right: edge,
				bottom: (bounds.bottom + edge) / 2,
			};
			let mut flags = if widget.kind == NativeKind::Radio {
				DFCS_BUTTONRADIO
			} else {
				DFCS_BUTTONCHECK
			};

			if props.checked.unwrap_or(false) {
				flags |= DFCS_CHECKED;
			}
			if !widget.interactive() {
				flags |= DFCS_INACTIVE;
			}

			let _ = DrawFrameControl(draw.hdc, &raw mut glyph, DFC_BUTTON, flags);
			bounds.left = edge + 5;
		} else if widget.kind == NativeKind::Button && props.border_width.unwrap_or(0.0) == 0.0 {
			FrameRect(draw.hdc, &raw const bounds, widget.brush.borrow().0);
		}

		if widget.kind != NativeKind::Pressable {
			let text = props.text.as_deref().unwrap_or("");
			let mut text: Vec<u16> = text.encode_utf16().collect();
			let mut style = props.text_style();

			if widget.kind == NativeKind::Button {
				style.text_align = props
					.text_align
					.unwrap_or(leptoskit_native::TextAlign::Center);
			}
			let align = text_flags(style) & !DT_WORDBREAK;
			DrawTextW(
				draw.hdc,
				&mut text,
				&raw mut bounds,
				align | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
			);
		}

		if (draw.uItemState & CDIS_FOCUS).0 != 0 {
			let mut focus = draw.rc;
			focus.left += 2;
			focus.top += 2;
			focus.right -= 2;
			focus.bottom -= 2;
			let _ = DrawFocusRect(draw.hdc, &raw const focus);
		}

		SelectObject(draw.hdc, previous);
	}
}

fn paint_image(dc: HDC, image: &image::RgbaImage, width: i32, height: i32) {
	let info = BITMAPINFO {
		bmiHeader: BITMAPINFOHEADER {
			biSize: size_of::<BITMAPINFOHEADER>() as u32,
			biWidth: image.width() as i32,
			biHeight: -(image.height() as i32),
			biPlanes: 1,
			biBitCount: 32,
			biCompression: BI_RGB.0,
			..Default::default()
		},
		..Default::default()
	};
	let mut bits = std::ptr::null_mut();
	// SAFETY: CreateDIBSection allocates exactly width*height BGRA pixels. The
	// bitmap is deselected before deletion, and premultiplication matches GDI's
	// AC_SRC_ALPHA contract rather than discarding source transparency.
	unsafe {
		let bitmap = CreateDIBSection(
			Some(dc),
			&raw const info,
			DIB_RGB_COLORS,
			&raw mut bits,
			None,
			0,
		)
		.expect("Windows could not allocate an image bitmap");
		let destination = std::slice::from_raw_parts_mut(bits.cast::<u8>(), image.as_raw().len());

		for (source, destination) in image
			.as_raw()
			.chunks_exact(4)
			.zip(destination.chunks_exact_mut(4))
		{
			let alpha = u16::from(source[3]);
			destination[0] = ((u16::from(source[2]) * alpha + 127) / 255) as u8;
			destination[1] = ((u16::from(source[1]) * alpha + 127) / 255) as u8;
			destination[2] = ((u16::from(source[0]) * alpha + 127) / 255) as u8;
			destination[3] = source[3];
		}

		let memory = CreateCompatibleDC(Some(dc));
		let previous = SelectObject(memory, bitmap.into());
		assert!(
			AlphaBlend(
				dc,
				0,
				0,
				width,
				height,
				memory,
				0,
				0,
				image.width() as i32,
				image.height() as i32,
				BLENDFUNCTION {
					BlendOp: AC_SRC_OVER as u8,
					BlendFlags: 0,
					SourceConstantAlpha: 255,
					AlphaFormat: AC_SRC_ALPHA as u8
				}
			)
			.as_bool(),
			"Windows could not paint an image"
		);
		SelectObject(memory, previous);
		let _ = DeleteDC(memory);
		let _ = DeleteObject(bitmap.into());
	}
}
