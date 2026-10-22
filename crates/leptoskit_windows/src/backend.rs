//! The mutation consumer and the UI-thread-owned window tree.

use std::cell::Cell;
use std::cell::RefCell;
use std::os::windows::ffi::OsStringExt;
use std::rc::Rc;
use std::rc::Weak;
use std::sync::Arc;

use leptoskit_native::Color;
use leptoskit_native::MutationList;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::Progress;
use leptoskit_native::Role;
use leptoskit_native::SharedTextMeasurer;
use leptoskit_yoga::Direction;
use leptoskit_yoga::NodeId;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemServices::SS_CENTER;
use windows::Win32::System::SystemServices::SS_LEFT;
use windows::Win32::System::SystemServices::SS_NOPREFIX;
use windows::Win32::System::SystemServices::SS_RIGHT;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;
use windows::core::w;

use crate::drawing::Brush;
use crate::drawing::Font;
use crate::drawing::WindowsTextMeasurer;
use crate::events::EventListener;
use crate::events::EventSink;
use crate::events::install_subclass;

const FRAME_CLASS: PCWSTR = w!("Leptoskit.NativeFrame");
pub(crate) const SLIDER_TICKS: i32 = 1_000_000;

/// A Win32 renderer owned by the thread on which it was created.
///
/// Install an event listener before showing the tree. The listener runs outside
/// backend borrows and may synchronously reconcile or delete the firing widget.
/// Controls restore the latest committed value after each proposal.
pub struct WindowsBackend {
	pub(crate) widgets: Vec<Option<Rc<Widget>>>,
	pub(crate) sink: Rc<EventSink>,
	parking: HWND,
	accessibility: Rc<Accessibility>,
	dark: bool,
	host: Rc<Cell<HWND>>,
	dpi: Rc<Cell<u32>>,
	applying: bool,
}

impl std::fmt::Debug for WindowsBackend {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WindowsBackend")
			.field("widgets", &self.widgets.iter().flatten().count())
			.finish_non_exhaustive()
	}
}

pub(crate) struct Widget {
	pub id: NodeId,
	pub kind: NativeKind,
	pub frame: HWND,
	pub composition: HWND,
	pub control: Option<HWND>,
	pub document: Option<HWND>,
	pub props: RefCell<NativeProps>,
	pub parent: RefCell<Weak<Self>>,
	pub children: RefCell<Vec<Rc<Self>>>,
	pub layout: Cell<[f32; 4]>,
	pub composition_bounds: Cell<[f32; 4]>,
	pub scroll: Cell<[i32; 2]>,
	pub extent: Cell<[i32; 2]>,
	pub animating: Cell<bool>,
	pub phase: Cell<u32>,
	pub alive: Cell<bool>,
	pub frame_alive: Cell<bool>,
	pub control_alive: Cell<bool>,
	pub document_alive: Cell<bool>,
	pub composition_alive: Cell<bool>,
	destroying: Cell<bool>,
	detached: Cell<bool>,
	pub dark: Cell<bool>,
	pub font: RefCell<Font>,
	pub brush: RefCell<Brush>,
	pub image: RefCell<Option<image::RgbaImage>>,
	pub image_error: RefCell<Option<String>>,
	pub sink: Rc<EventSink>,
	pub host: Rc<Cell<HWND>>,
	pub dpi: Rc<Cell<u32>>,
	pub parking: HWND,
	decorative_annotation: Cell<bool>,
	/// Whether decorative annotations ever reached the frame and group. MSAA
	/// rejects clearing a window that was never annotated.
	group_annotated: Cell<bool>,
	/// Whether the group is layered and must repaint its placed subtree into
	/// its own redirection surface.
	repaint_group: Cell<bool>,
	accessibility: Rc<Accessibility>,
}

struct Accessibility(Option<IAccPropServices>);

impl Accessibility {
	fn services(&self) -> &IAccPropServices {
		self.0.as_ref().expect("COM apartment is live")
	}
}

impl Drop for Accessibility {
	fn drop(&mut self) {
		drop(self.0.take());
		// SAFETY: The last retained widget releases COM on its creating thread,
		// after the last accessibility interface, even during reentrant
		// teardown.
		unsafe {
			CoUninitialize();
		}
	}
}

unsafe extern "system" fn frame_proc(
	hwnd: HWND,
	message: u32,
	wparam: WPARAM,
	lparam: LPARAM,
) -> LRESULT {
	// SAFETY: The initial frame procedure forwards all messages unchanged.
	unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

impl WindowsBackend {
	/// Initializes common controls and COM on the calling UI thread.
	///
	/// The thread must use a single-threaded COM apartment. The executable must
	/// include `app.manifest` so Windows permits layered child-window opacity.
	pub fn new() -> windows::core::Result<Self> {
		// SAFETY: Initialization is balanced by Drop; all native ownership
		// remains on this thread because Rc makes the backend neither
		// Send nor Sync.
		unsafe {
			CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
			let result = Self::initialize();

			if result.is_err() {
				CoUninitialize();
			}

			result
		}
	}

	fn initialize() -> windows::core::Result<Self> {
		// SAFETY: The static class procedure never accesses uninitialized
		// state.
		unsafe {
			InitCommonControlsEx(&INITCOMMONCONTROLSEX {
				dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
				dwICC: ICC_STANDARD_CLASSES | ICC_BAR_CLASSES | ICC_PROGRESS_CLASS,
			})
			.ok()?;
			let instance = GetModuleHandleW(None)?;
			let class = WNDCLASSW {
				lpfnWndProc: Some(frame_proc),
				hInstance: instance.into(),
				lpszClassName: FRAME_CLASS,
				hCursor: LoadCursorW(None, IDC_ARROW)?,
				..Default::default()
			};

			if RegisterClassW(&raw const class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS
			{
				return Err(windows::core::Error::from_win32());
			}

			let services = CoCreateInstance(&CAccPropServices, None, CLSCTX_INPROC_SERVER)?;
			let parking = CreateWindowExW(
				WINDOW_EX_STYLE::default(),
				FRAME_CLASS,
				w!(""),
				WS_POPUP,
				0,
				0,
				0,
				0,
				None,
				None,
				Some(instance.into()),
				None,
			)?;

			Ok(Self {
				widgets: Vec::new(),
				sink: Rc::default(),
				parking,
				accessibility: Rc::new(Accessibility(Some(services))),
				dark: false,
				host: Rc::new(Cell::new(parking)),
				dpi: Rc::new(Cell::new(96)),
				applying: false,
			})
		}
	}

	/// Replaces the listener used by actual native input and accessibility
	/// actions.
	pub fn set_event_listener(&mut self, listener: EventListener) {
		*self.sink.listener.borrow_mut() = Some(listener);
	}

	/// The outer window whose frame is controlled by Yoga.
	pub fn view(&self, id: NodeId) -> Option<HWND> {
		self.widget(id)
			.filter(|widget| widget.alive.get())
			.map(|widget| widget.frame)
	}

	/// The real Windows control, when this primitive contains one.
	pub fn control(&self, id: NodeId) -> Option<HWND> {
		self.widget(id)
			.filter(|widget| widget.alive.get())
			.and_then(|widget| widget.control)
	}

	/// The native grouping window that composites opacity over the entire
	/// subtree, including visible overflow. Its child frame keeps Yoga's
	/// bounds.
	pub fn composition_window(&self, id: NodeId) -> Option<HWND> {
		self.widget(id)
			.filter(|widget| widget.alive.get())
			.map(|widget| widget.composition)
	}

	/// The document HWND used by a scroll viewport.
	pub fn scroll_document(&self, id: NodeId) -> Option<HWND> {
		self.widget(id)
			.filter(|widget| widget.alive.get())
			.and_then(|widget| widget.document)
	}

	/// Returns a source-loading diagnostic without printing potentially private
	/// paths.
	pub fn image_error(&self, id: NodeId) -> Option<String> {
		self.widget(id)
			.and_then(|widget| widget.image_error.borrow().clone())
	}

	/// Whether an indicator currently owns a live native animation timer.
	pub fn is_animating(&self, id: NodeId) -> bool {
		self.widget(id).is_some_and(|widget| widget.animating.get())
	}

	pub(crate) fn widget(&self, id: NodeId) -> Option<&Rc<Widget>> {
		self.widgets.get(id.index()).and_then(Option::as_ref)
	}

	pub(crate) fn mount(&self, root: NodeId, host: HWND, dpi: u32) {
		self.host.set(host);
		self.widget(root)
			.expect("mount requires a committed root")
			.refresh_dpi(dpi);
	}

	/// Updates native frames and fonts after a host DPI transition. Obtain a
	/// new immutable text measurer and invalidate surface text layout in
	/// the same host transaction before rendering the next frame.
	pub fn set_dpi(&mut self, dpi: u32) {
		assert!(dpi > 0, "Windows DPI must be positive");
		self.dpi.set(dpi);

		for widget in self
			.widgets
			.iter()
			.flatten()
			.filter(|widget| widget.parent.borrow().upgrade().is_none())
		{
			widget.refresh_dpi(dpi);
		}
	}

	fn refresh_tree(&self) {
		if self.applying {
			return;
		}
		let _suppressed = self.sink.suppress();

		for widget in self.widgets.iter().flatten() {
			if widget.parent.borrow().upgrade().is_none() {
				widget.update_composition_bounds();
				widget.place_tree();
			}
		}

		for widget in self
			.widgets
			.iter()
			.flatten()
			.filter(|widget| widget.document.is_some())
		{
			widget.refresh_scroll();
		}
	}
}

impl Widget {
	pub fn scale(&self) -> f32 {
		self.dpi.get() as f32 / 96.0
	}

	pub fn viewport(&self) -> RECT {
		let mut client = RECT::default();
		// SAFETY: This query only receives a live window owned by this widget.
		unsafe {
			GetClientRect(self.frame, &raw mut client)
				.expect("owned viewport has a client rectangle");
		}
		let border = (self.props.borrow().border_width.unwrap_or(0.0).max(0.0) * self.scale())
			.round() as i32;
		client.left = border.min(client.right / 2);
		client.top = border.min(client.bottom / 2);
		client.right = (client.right - border).max(client.left);
		client.bottom = (client.bottom - border).max(client.top);
		client
	}

	pub fn place_tree(&self) {
		if !self.alive.get() {
			return;
		}

		self.place();
		self.refresh_state();

		for child in self.children.borrow().iter() {
			child.place_tree();
		}

		self.update_composition_region();
		self.refresh_scroll();

		self.repaint_layer();
	}

	/// Paints a layered group's placed subtree into its own redirection
	/// surface. That surface only receives its children's paint while the
	/// group is visible, and repainting the host does not reach it, so content
	/// placed while the group or its host was hidden left it empty. Hidden
	/// groups stay pending until [`Self::repaint_layers`] runs after showing.
	fn repaint_layer(&self) {
		// SAFETY: The group is a live window owned by this widget.
		unsafe {
			if self.repaint_group.get() && IsWindowVisible(self.composition).as_bool() {
				self.repaint_group.set(false);
				let _ = RedrawWindow(
					Some(self.composition),
					None,
					None,
					RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN | RDW_UPDATENOW,
				);
			}
		}
	}

	/// Repaints every pending layered group in this subtree, after its host
	/// becomes visible.
	pub fn repaint_layers(&self) {
		if !self.alive.get() {
			return;
		}
		self.repaint_layer();

		for child in self.children.borrow().iter() {
			child.repaint_layers();
		}
	}

	pub fn refresh_dpi(&self, dpi: u32) {
		let _suppressed = self.sink.suppress();
		self.dpi.set(dpi);
		self.refresh_fonts();
		self.update_composition_bounds();
		self.place_tree();
	}

	fn refresh_fonts(&self) {
		if !self.alive.get() {
			return;
		}
		let props = self.props.borrow().clone();
		self.update_props(&props);

		for child in self.children.borrow().iter() {
			child.refresh_fonts();
		}
	}

	fn clips(&self) -> bool {
		let props = self.props.borrow();
		self.kind == NativeKind::ScrollView
			|| props.clip == Some(true)
			|| props.corner_radius.unwrap_or(0.0) > 0.0
	}

	/// Whether the Yoga-sized frame draws anything of its own. A transparent,
	/// unclipped container draws nothing, so its group's region excludes the
	/// frame: overlapping siblings beneath stay visible and receive input,
	/// as they do on every other backend. A root always paints the backend's
	/// default background over the host.
	fn paints_frame(&self) -> bool {
		let props = self.props.borrow();
		self.kind != NativeKind::View
			|| self.parent.borrow().upgrade().is_none()
			|| self.clips()
			|| props.background.is_some_and(|color| color.alpha() > 0)
			|| props.border_width.unwrap_or(0.0) > 0.0
	}

	/// The alpha of a translucent background on a childless, borderless
	/// container, such as a modal scrim. Its group window blends the
	/// unflattened color with this alpha, so the siblings beneath show
	/// through. A translucent background behind text, controls, or children
	/// is flattened over the nearest ancestor background instead, because a
	/// classic child window cannot blend its own content per pixel.
	fn backdrop_alpha(&self) -> Option<u8> {
		let props = self.props.borrow();
		let alpha = props.background?.alpha();
		(self.kind == NativeKind::View
			&& (1..u8::MAX).contains(&alpha)
			&& props.border_width.unwrap_or(0.0) <= 0.0
			&& self.children.borrow().is_empty())
		.then_some(alpha)
	}

	/// The color this widget's frame fills with.
	fn fill(&self) -> Color {
		match (self.backdrop_alpha(), self.props.borrow().background) {
			(Some(_), Some(color)) => Color::rgba(color.red(), color.green(), color.blue(), 255),
			_ => self.background(),
		}
	}

	fn update_composition_bounds(&self) {
		let [_, _, width, height] = self.layout.get();
		let mut bounds = [0.0_f32, 0.0_f32, width, height];

		for child in self.children.borrow().iter() {
			child.update_composition_bounds();

			if self.clips() || !child.visible() {
				continue;
			}

			let [x, y, _, _] = child.layout.get();
			let [left, top, right, bottom] = child.composition_bounds.get();
			bounds = [
				bounds[0].min(x + left),
				bounds[1].min(y + top),
				bounds[2].max(x + right),
				bounds[3].max(y + bottom),
			];
		}

		self.composition_bounds.set(bounds);
	}

	fn place(&self) {
		let [mut x, mut y, width, height] = self.layout.get();
		let [left, top, right, bottom] = self.composition_bounds.get();
		x += left;
		y += top;
		let physical_parent = self
			.parent
			.borrow()
			.upgrade()
			.map_or(self.host.get(), |parent| {
				if parent.clips() {
					if parent.document.is_some() {
						let border = parent.props.borrow().border_width.unwrap_or(0.0).max(0.0);
						x -= border;
						y -= border;
					}
					parent.content()
				} else {
					let [parent_left, parent_top, _, _] = parent.composition_bounds.get();
					x -= parent_left;
					y -= parent_top;
					parent.composition
				}
			});
		let parent_destroyed = self
			.parent
			.borrow()
			.upgrade()
			.is_some_and(|parent| !parent.alive.get());
		let physical_parent = if self.detached.get() || parent_destroyed {
			self.parking
		} else {
			physical_parent
		};
		let scale = self.scale();
		let opacity = self.props.borrow().opacity.unwrap_or(1.0).clamp(0.0, 1.0)
			* f32::from(self.backdrop_alpha().unwrap_or(u8::MAX))
			/ 255.0;
		// SAFETY: The group encloses the union of visible overflow while the
		// actual frame retains Yoga's dimensions. Native group alpha therefore
		// composites overlapping descendants together, rather than fading each
		// sibling independently. Explicit clipping still uses the
		// frame/document.
		unsafe {
			if GetParent(self.composition).ok() != Some(physical_parent) {
				SetParent(self.composition, Some(physical_parent))
					.expect("owned group can be reparented");
			}
			SetWindowPos(
				self.composition,
				Some(HWND_TOP),
				(x * scale).round() as i32,
				(y * scale).round() as i32,
				((right - left) * scale).round().max(0.0) as i32,
				((bottom - top) * scale).round().max(0.0) as i32,
				SWP_NOACTIVATE,
			)
			.expect("owned group accepts overflow bounds");
			SetWindowPos(
				self.frame,
				Some(HWND_BOTTOM),
				(-left * scale).round() as i32,
				(-top * scale).round() as i32,
				(width * scale).round().max(0.0) as i32,
				(height * scale).round().max(0.0) as i32,
				SWP_NOACTIVATE,
			)
			.expect("owned native frame accepts Yoga bounds");
			let style = GetWindowLongW(self.composition, GWL_EXSTYLE) as u32;

			if opacity < 1.0 {
				SetWindowLongW(
					self.composition,
					GWL_EXSTYLE,
					(style | WS_EX_LAYERED.0) as i32,
				);
				SetLayeredWindowAttributes(
					self.composition,
					COLORREF(0),
					(opacity * 255.0).round() as u8,
					LWA_ALPHA,
				)
				.expect("child opacity requires the documented application manifest");
				self.repaint_group.set(true);
			} else if style & WS_EX_LAYERED.0 != 0 {
				SetWindowLongW(
					self.composition,
					GWL_EXSTYLE,
					(style & !WS_EX_LAYERED.0) as i32,
				);
			}

			// Windows caches the extended style until a frame change.
			if (opacity < 1.0) != (style & WS_EX_LAYERED.0 != 0) {
				SetWindowPos(
					self.composition,
					None,
					0,
					0,
					0,
					0,
					SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
				)
				.expect("owned group accepts its changed extended style");
			}
		}
		self.layout_control();
		self.update_region();
	}

	fn update_composition_region(&self) {
		if !self.alive.get() {
			return;
		}
		let scale = self.scale();
		let [left, top, _, _] = self.composition_bounds.get();
		let [_, _, width, height] = self.layout.get();
		let radius = self
			.props
			.borrow()
			.corner_radius
			.unwrap_or(0.0)
			.max(0.0)
			.min(width.min(height) / 2.0)
			* scale;
		// SAFETY: Child regions are copied and offset before union, never
		// stolen. The final region's ownership transfers to Windows
		// only on success.
		unsafe {
			let region = if !self.paints_frame() {
				CreateRectRgn(0, 0, 0, 0)
			} else if radius > 0.0 {
				CreateRoundRectRgn(
					0,
					0,
					(width * scale).round() as i32 + 1,
					(height * scale).round() as i32 + 1,
					(radius * 2.0).round() as i32,
					(radius * 2.0).round() as i32,
				)
			} else {
				CreateRectRgn(
					0,
					0,
					(width * scale).round() as i32,
					(height * scale).round() as i32,
				)
			};
			OffsetRgn(
				region,
				(-left * scale).round() as i32,
				(-top * scale).round() as i32,
			);

			if !self.clips() {
				for child in self
					.children
					.borrow()
					.iter()
					.filter(|child| child.visible())
				{
					let child_region = CreateRectRgn(0, 0, 0, 0);
					assert_ne!(
						GetWindowRgn(child.composition, child_region),
						GDI_REGION_TYPE(0),
						"owned child group has a clipping region"
					);
					let [x, y, _, _] = child.layout.get();
					let [child_left, child_top, _, _] = child.composition_bounds.get();
					OffsetRgn(
						child_region,
						((x + child_left - left) * scale).round() as i32,
						((y + child_top - top) * scale).round() as i32,
					);
					CombineRgn(Some(region), Some(region), Some(child_region), RGN_OR);
					let _ = DeleteObject(child_region.into());
				}
			}

			if SetWindowRgn(self.composition, Some(region), true) == 0 {
				let _ = DeleteObject(region.into());
				panic!("Windows could not apply a group clipping region");
			}
		}
	}

	pub fn background(&self) -> Color {
		let parent = self.parent.borrow().upgrade().map_or(
			if self.dark.get() {
				Color::rgba(32, 32, 32, 255)
			} else {
				Color::rgba(255, 255, 255, 255)
			},
			|parent| parent.background(),
		);
		let Some(color) = self.props.borrow().background else {
			return parent;
		};
		let alpha = u32::from(color.alpha());
		let channel = |foreground, background| {
			((u32::from(foreground) * alpha + u32::from(background) * (255 - alpha) + 127) / 255)
				as u8
		};
		Color::rgba(
			channel(color.red(), parent.red()),
			channel(color.green(), parent.green()),
			channel(color.blue(), parent.blue()),
			255,
		)
	}

	pub fn content(&self) -> HWND {
		self.document.unwrap_or(self.frame)
	}

	pub fn interactive(&self) -> bool {
		if !self.alive.get() || self.detached.get() {
			return false;
		}

		let props = self.props.borrow();

		if props.enabled == Some(false) || props.hidden == Some(true) {
			return false;
		}

		self.parent
			.borrow()
			.upgrade()
			.is_none_or(|parent| parent.interactive())
	}

	pub fn visible(&self) -> bool {
		self.alive.get()
			&& !self.detached.get()
			&& self.props.borrow().hidden != Some(true)
			&& self
				.parent
				.borrow()
				.upgrade()
				.is_none_or(|parent| parent.visible())
	}

	pub fn decorative(&self) -> bool {
		self.parent
			.borrow()
			.upgrade()
			.is_some_and(|parent| parent.kind == NativeKind::Pressable || parent.decorative())
	}

	pub fn refresh_state(&self) {
		if !self.alive.get() {
			return;
		}
		let interactive = self.interactive();
		let visible = self.visible()
			&& !(self.kind == NativeKind::ActivityIndicator
				&& self.props.borrow().running == Some(false));
		let decorative = self.decorative();

		if decorative != self.decorative_annotation.replace(decorative) {
			if decorative {
				self.group_annotated.set(true);
			}
			// SAFETY: Native MSAA names/states remain separate from edit
			// values. UI Automation additionally receives a
			// non-control provider in the subclass; flattened
			// decorative HWNDs remain visually present only.
			unsafe {
				for hwnd in [Some(self.frame), Some(self.composition), self.control]
					.into_iter()
					.flatten()
				{
					if decorative {
						self.accessibility
							.services()
							.SetHwndProp(
								hwnd,
								OBJID_CLIENT.0 as u32,
								CHILDID_SELF,
								PROPID_ACC_STATE,
								&msaa_i4(STATE_SYSTEM_INVISIBLE.0 | STATE_SYSTEM_OFFSCREEN.0),
							)
							.expect(
								"decorative native content accepts invisible accessibility state",
							);
						self.accessibility
							.services()
							.SetHwndPropStr(
								hwnd,
								OBJID_CLIENT.0 as u32,
								CHILDID_SELF,
								PROPID_ACC_NAME,
								w!(""),
							)
							.expect(
								"decorative native content accepts an empty accessibility name",
							);
					} else {
						self.accessibility
							.services()
							.ClearHwndProps(
								hwnd,
								OBJID_CLIENT.0 as u32,
								CHILDID_SELF,
								&[PROPID_ACC_STATE],
							)
							.expect("native accessibility state can be restored");
					}
				}
			}
		}
		let props = self.props.borrow();
		*self.brush.borrow_mut() = Brush::new(self.fill());
		let animate = visible
			&& match self.kind {
				NativeKind::ActivityIndicator => props.running.unwrap_or(true),
				NativeKind::LinearProgressIndicator => {
					props.progress.unwrap_or(Progress::Indeterminate) == Progress::Indeterminate
				}
				_ => false,
			};
		// SAFETY: All handles belong to this widget and remain on its UI
		// thread.
		unsafe {
			let _ = EnableWindow(self.frame, interactive);
			let _ = EnableWindow(self.composition, interactive);
			let _ = ShowWindow(self.frame, if visible { SW_SHOWNA } else { SW_HIDE });
			let _ = ShowWindow(self.composition, if visible { SW_SHOWNA } else { SW_HIDE });

			if let Some(control) = self.control {
				let _ = EnableWindow(control, interactive);
				let _ = InvalidateRect(Some(control), None, true);
			}
			let _ = InvalidateRect(Some(self.frame), None, true);

			if animate != self.animating.replace(animate) {
				if self.kind == NativeKind::LinearProgressIndicator {
					SendMessageW(
						self.control.expect("native progress control"),
						PBM_SETMARQUEE,
						Some(WPARAM(usize::from(animate))),
						Some(LPARAM(40)),
					);
				} else if animate {
					assert_ne!(
						SetTimer(Some(self.frame), 1, 40, None),
						0,
						"Windows could not allocate an animation timer"
					);
				} else {
					let _ = KillTimer(Some(self.frame), 1);
				}
			}
		}
	}

	pub fn restore_control(&self) {
		let Some(control) = self.control else { return };

		if !self.alive.get() {
			return;
		}

		let _suppressed = self.sink.suppress();
		let props = self.props.borrow();
		// SAFETY: Message payloads match the standard native control class.
		unsafe {
			match self.kind {
				NativeKind::TextInput | NativeKind::PasswordInput => {
					let value = props
						.value
						.as_deref()
						.or(props.text.as_deref())
						.unwrap_or("");

					if window_text(control) != value {
						let mut start = 0_u32;
						let mut end = 0_u32;
						SendMessageW(
							control,
							EM_GETSEL,
							Some(WPARAM((&raw mut start) as usize)),
							Some(LPARAM((&raw mut end) as isize)),
						);
						SetWindowTextW(control, PCWSTR(wide(value).as_ptr()))
							.expect("owned edit window accepts text");
						SendMessageW(
							control,
							EM_SETSEL,
							Some(WPARAM(start as usize)),
							Some(LPARAM(end as isize)),
						);
					}
				}
				NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio => {
					SendMessageW(
						control,
						BM_SETCHECK,
						Some(WPARAM(if props.checked.unwrap_or(false) {
							BST_CHECKED.0
						} else {
							BST_UNCHECKED.0
						} as usize)),
						None,
					);
				}
				NativeKind::Slider => {
					let state = props.slider.unwrap_or_default();
					let position = ((state.value() - state.min()) / (state.max() - state.min())
						* f64::from(SLIDER_TICKS))
					.round() as i32;
					SendMessageW(
						control,
						TBM_SETPOS,
						Some(WPARAM(1)),
						Some(LPARAM(position as isize)),
					);
				}
				_ => {}
			}
		}
	}

	fn update_props(&self, props: &NativeProps) {
		if !self.alive.get() {
			return;
		}
		let old_source = self.props.borrow().source.clone();
		*self.props.borrow_mut() = props.clone();
		let font = Font::new(props.text_style(), self.scale());
		*self.brush.borrow_mut() = Brush::new(self.fill());
		// SAFETY: All message arguments remain alive throughout synchronous
		// calls.
		unsafe {
			if let Some(control) = self.control {
				SendMessageW(
					control,
					WM_SETFONT,
					Some(WPARAM(font.0.0 as usize)),
					Some(LPARAM(1)),
				);
				let rtl = props.text_style().direction == Direction::Rtl;
				let exstyle = GetWindowLongW(control, GWL_EXSTYLE) as u32;
				SetWindowLongW(
					control,
					GWL_EXSTYLE,
					if rtl {
						exstyle | WS_EX_RTLREADING.0
					} else {
						exstyle & !WS_EX_RTLREADING.0
					} as i32,
				);
				let label = if self.decorative() {
					""
				} else {
					props
						.accessibility_label
						.as_deref()
						.or(props.text.as_deref())
						.unwrap_or("")
				};
				self.accessibility
					.services()
					.SetHwndPropStr(
						control,
						OBJID_CLIENT.0 as u32,
						CHILDID_SELF,
						PROPID_ACC_NAME,
						PCWSTR(wide(label).as_ptr()),
					)
					.expect("owned control accepts accessibility name");

				if matches!(
					self.kind,
					NativeKind::Button
						| NativeKind::Switch
						| NativeKind::Checkbox
						| NativeKind::Radio
				) {
					SetWindowTextW(
						control,
						PCWSTR(wide(props.text.as_deref().unwrap_or("")).as_ptr()),
					)
					.expect("owned button accepts title");
				}

				if matches!(self.kind, NativeKind::Text | NativeKind::TextRun) {
					SetWindowTextW(
						control,
						PCWSTR(wide(props.text.as_deref().unwrap_or("")).as_ptr()),
					)
					.expect("owned label accepts text");
					let alignment = match (props.text_align.unwrap_or_default(), rtl) {
						(leptoskit_native::TextAlign::Start, false)
						| (leptoskit_native::TextAlign::End, true) => SS_LEFT.0,
						(leptoskit_native::TextAlign::Center, _) => SS_CENTER.0,
						(leptoskit_native::TextAlign::End, false)
						| (leptoskit_native::TextAlign::Start, true) => SS_RIGHT.0,
					};
					let style = GetWindowLongW(control, GWL_STYLE) as u32;
					SetWindowLongW(control, GWL_STYLE, ((style & !0x1f) | alignment) as i32);
				}

				if self.kind == NativeKind::Pressable {
					SetWindowTextW(control, w!(""))
						.expect("owned pressable accepts an empty visual title");
				}

				if self.kind == NativeKind::Slider {
					let value = props.slider.unwrap_or_default().value().to_string();
					self.accessibility
						.services()
						.SetHwndPropStr(
							control,
							OBJID_CLIENT.0 as u32,
							CHILDID_SELF,
							PROPID_ACC_VALUE,
							PCWSTR(wide(&value).as_ptr()),
						)
						.expect("native slider exposes its shared value to MSAA");
				}

				if matches!(self.kind, NativeKind::TextInput | NativeKind::PasswordInput) {
					let alignment = match (props.text_align.unwrap_or_default(), rtl) {
						(leptoskit_native::TextAlign::Start, false)
						| (leptoskit_native::TextAlign::End, true) => ES_LEFT,
						(leptoskit_native::TextAlign::Center, _) => ES_CENTER,
						(leptoskit_native::TextAlign::End, false)
						| (leptoskit_native::TextAlign::Start, true) => ES_RIGHT,
					};
					let style = GetWindowLongW(control, GWL_STYLE);
					SetWindowLongW(
						control,
						GWL_STYLE,
						(style & !(ES_CENTER | ES_RIGHT)) | alignment,
					);
					SendMessageW(
						control,
						EM_SETCUEBANNER,
						Some(WPARAM(1)),
						Some(LPARAM(
							wide(props.placeholder.as_deref().unwrap_or("")).as_ptr() as isize,
						)),
					);
				}

				if self.kind == NativeKind::LinearProgressIndicator {
					let progress = props.progress.unwrap_or(Progress::Indeterminate);
					let style = GetWindowLongW(control, GWL_STYLE) as u32;
					let style = if progress == Progress::Indeterminate {
						style | PBS_MARQUEE
					} else {
						style & !PBS_MARQUEE
					};
					SetWindowLongW(control, GWL_STYLE, style as i32);
					SendMessageW(
						control,
						PBM_SETRANGE32,
						Some(WPARAM(0)),
						Some(LPARAM(10_000)),
					);
					SendMessageW(control, PBM_SETMARQUEE, Some(WPARAM(0)), None);
					self.animating.set(false);

					if let Progress::Determinate(value) = progress {
						SendMessageW(
							control,
							PBM_SETPOS,
							Some(WPARAM((value.get() * 10_000.0).round() as usize)),
							None,
						);
					}
				}
			}

			if self.control.is_none() {
				let label = if self.decorative() {
					""
				} else {
					props
						.accessibility_label
						.as_deref()
						.or(props.text.as_deref())
						.unwrap_or("")
				};
				let role = match props.aria.role {
					Some(Role::Button | Role::MenuItem | Role::Tab | Role::TreeItem) => {
						ROLE_SYSTEM_PUSHBUTTON
					}
					Some(Role::Checkbox | Role::MenuItemCheckbox | Role::Switch) => {
						ROLE_SYSTEM_CHECKBUTTON
					}
					Some(Role::Radio | Role::MenuItemRadio) => ROLE_SYSTEM_RADIOBUTTON,
					Some(Role::Link) => ROLE_SYSTEM_LINK,
					Some(Role::List | Role::ListBox) => ROLE_SYSTEM_LIST,
					Some(Role::ListItem) => ROLE_SYSTEM_LISTITEM,
					Some(Role::Menu) => ROLE_SYSTEM_MENUPOPUP,
					Some(Role::MenuBar) => ROLE_SYSTEM_MENUBAR,
					Some(Role::Toolbar) => ROLE_SYSTEM_TOOLBAR,
					Some(Role::Alert) => ROLE_SYSTEM_ALERT,
					Some(Role::Status | Role::Log) => ROLE_SYSTEM_STATUSBAR,
					Some(Role::Dialog | Role::AlertDialog) => ROLE_SYSTEM_DIALOG,
					Some(Role::Table | Role::Grid | Role::TreeGrid) => ROLE_SYSTEM_TABLE,
					Some(Role::Row) => ROLE_SYSTEM_ROW,
					Some(Role::Cell | Role::GridCell) => ROLE_SYSTEM_CELL,
					Some(Role::ColumnHeader | Role::RowHeader) => ROLE_SYSTEM_COLUMNHEADER,
					Some(Role::Separator) => ROLE_SYSTEM_SEPARATOR,
					Some(Role::SpinButton) => ROLE_SYSTEM_SPINBUTTON,
					Some(Role::ComboBox) => ROLE_SYSTEM_COMBOBOX,
					Some(Role::ScrollBar) => ROLE_SYSTEM_SCROLLBAR,
					Some(Role::ProgressBar | Role::Meter) => ROLE_SYSTEM_PROGRESSBAR,
					Some(Role::Heading | Role::Paragraph | Role::Term | Role::Definition) => {
						ROLE_SYSTEM_STATICTEXT
					}
					Some(Role::Img) => ROLE_SYSTEM_GRAPHIC,
					_ => {
						match self.kind {
							NativeKind::Image => ROLE_SYSTEM_GRAPHIC,
							NativeKind::ActivityIndicator => ROLE_SYSTEM_ANIMATION,
							_ => ROLE_SYSTEM_GROUPING,
						}
					}
				};
				self.accessibility
					.services()
					.SetHwndPropStr(
						self.frame,
						OBJID_CLIENT.0 as u32,
						CHILDID_SELF,
						PROPID_ACC_NAME,
						PCWSTR(wide(label).as_ptr()),
					)
					.expect("owned frame accepts accessibility name");
				self.accessibility
					.services()
					.SetHwndProp(
						self.frame,
						OBJID_CLIENT.0 as u32,
						CHILDID_SELF,
						PROPID_ACC_ROLE,
						&msaa_i4(role),
					)
					.expect("owned frame accepts accessibility role");
				let description = props.aria.description.as_deref().unwrap_or("");
				self.accessibility
					.services()
					.SetHwndPropStr(
						self.frame,
						OBJID_CLIENT.0 as u32,
						CHILDID_SELF,
						PROPID_ACC_DESCRIPTION,
						PCWSTR(wide(description).as_ptr()),
					)
					.expect("owned frame accepts accessibility description");
				if let Some(value) = props.aria.value_text.as_deref() {
					self.accessibility
						.services()
						.SetHwndPropStr(
							self.frame,
							OBJID_CLIENT.0 as u32,
							CHILDID_SELF,
							PROPID_ACC_VALUE,
							PCWSTR(wide(value).as_ptr()),
						)
						.expect("owned frame accepts accessibility value");
				}
			}

			let _ = InvalidateRect(Some(self.frame), None, true);
		}
		*self.font.borrow_mut() = font;
		self.restore_control();

		if self.kind == NativeKind::Image && old_source != props.source {
			let image = props.source.as_deref().map(load_image).transpose();

			match image {
				Ok(image) => {
					*self.image.borrow_mut() = image;
					*self.image_error.borrow_mut() = None;
				}
				Err(error) => {
					*self.image.borrow_mut() = None;
					*self.image_error.borrow_mut() = Some(error);
				}
			}
		}

		self.update_region();
		self.layout_control();
	}

	fn layout_control(&self) {
		let Some(control) = self.control else { return };
		let [_, _, width, height] = self.layout.get();
		let scale = self.scale();
		let border = (self
			.props
			.borrow()
			.border_width
			.unwrap_or(0.0)
			.max(0.0)
			.min(width.min(height) / 2.0)
			* scale)
			.round() as i32;
		let [width, height] = [
			(width * scale).round() as i32,
			(height * scale).round() as i32,
		];
		// SAFETY: The control's client frame stays inside its Yoga-owned outer
		// box.
		unsafe {
			SetWindowPos(
				control,
				Some(HWND_BOTTOM),
				border,
				border,
				width - 2 * border,
				height - 2 * border,
				SWP_NOACTIVATE,
			)
			.expect("owned control accepts layout");
		}
	}

	pub fn update_region(&self) {
		if !self.alive.get() {
			return;
		}
		let [_, _, width, height] = self.layout.get();
		let scale = self.scale();
		let radius = (self
			.props
			.borrow()
			.corner_radius
			.unwrap_or(0.0)
			.max(0.0)
			.min(width.min(height) / 2.0)
			* scale)
			.round() as i32;
		let [width, height] = [
			(width * scale).round() as i32,
			(height * scale).round() as i32,
		];
		// SAFETY: The region is transferred to Windows on success, otherwise
		// deleted here. A parent region clips its entire HWND subtree.
		unsafe {
			if radius > 0 {
				let region =
					CreateRoundRectRgn(0, 0, width + 1, height + 1, radius * 2, radius * 2);

				if SetWindowRgn(self.frame, Some(region), true) == 0 {
					let _ = DeleteObject(region.into());
					panic!("Windows could not apply a widget clipping region");
				}
			} else {
				SetWindowRgn(self.frame, None, true);
			}
		}
	}

	pub fn refresh_scroll(&self) {
		let Some(document) = self.document else {
			return;
		};
		if !self.alive.get() {
			return;
		}
		let [_, _, width, height] = self.layout.get();
		let scale = self.scale();
		let [width, height] = [
			(width * scale).round() as i32,
			(height * scale).round() as i32,
		];
		let border =
			(self.props.borrow().border_width.unwrap_or(0.0).max(0.0) * scale).round() as i32;
		// Each child's group bounds already enclose its visible,
		// unclipped descendants, so overflow beyond a child's own frame
		// stays reachable by scrolling.
		let content_extent = self
			.children
			.borrow()
			.iter()
			.filter(|child| child.visible())
			.fold([0, 0], |extent, child| {
				let [x, y, _, _] = child.layout.get();
				let [_, _, right, bottom] = child.composition_bounds.get();
				[
					extent[0].max(((x + right) * scale).round() as i32 - border),
					extent[1].max(((y + bottom) * scale).round() as i32 - border),
				]
			});
		let full = [width - border * 2, height - border * 2];
		let mut bars = [false; 2];
		// SAFETY: System metrics use the same monitor DPI as native allocation.
		let scrollbar = unsafe {
			[
				GetSystemMetricsForDpi(SM_CXVSCROLL, self.dpi.get()),
				GetSystemMetricsForDpi(SM_CYHSCROLL, self.dpi.get()),
			]
		};

		for _ in 0..3 {
			bars = [
				content_extent[0] > full[0] - if bars[1] { scrollbar[0] } else { 0 },
				content_extent[1] > full[1] - if bars[0] { scrollbar[1] } else { 0 },
			];
		}

		// SAFETY: Showing a standard nonclient bar changes the client size
		// before the actual viewport is queried below; neither bar
		// hides the final strip.
		unsafe {
			let _ = ShowScrollBar(self.frame, SB_HORZ, bars[0]);
			let _ = ShowScrollBar(self.frame, SB_VERT, bars[1]);
		}
		let old = self.scroll.get();
		let client = self.viewport();
		let [page_width, page_height] = [
			(client.right - client.left).max(0),
			(client.bottom - client.top).max(0),
		];
		let extent = [
			content_extent[0].max(page_width),
			content_extent[1].max(page_height),
		];
		self.extent.set(extent);
		let offset = [
			old[0].clamp(0, (extent[0] - page_width).max(0)),
			old[1].clamp(0, (extent[1] - page_height).max(0)),
		];
		self.scroll.set(offset);
		// SAFETY: The viewport owns its document and standard scroll bars.
		unsafe {
			SetWindowPos(
				document,
				None,
				client.left - offset[0],
				client.top - offset[1],
				extent[0],
				extent[1],
				SWP_NOZORDER | SWP_NOACTIVATE,
			)
			.expect("owned scroll document accepts its extent");

			for (bar, page, maximum, position) in [
				(SB_HORZ, page_width, extent[0], offset[0]),
				(SB_VERT, page_height, extent[1], offset[1]),
			] {
				SetScrollInfo(
					self.frame,
					bar,
					&SCROLLINFO {
						cbSize: size_of::<SCROLLINFO>() as u32,
						fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
						nMin: 0,
						nMax: maximum.saturating_sub(1),
						nPage: page.max(0) as u32,
						nPos: position,
						..Default::default()
					},
					true,
				);
			}

			let region = CreateRectRgn(
				offset[0],
				offset[1],
				offset[0] + page_width,
				offset[1] + page_height,
			);

			if SetWindowRgn(document, Some(region), true) == 0 {
				let _ = DeleteObject(region.into());
				panic!("Windows could not clip a scroll document to its viewport");
			}
		}
	}

	pub fn destroy(&self) {
		if self.destroying.replace(true) {
			return;
		}
		self.alive.set(false);

		let _suppressed = self.sink.suppress();
		self.animating.set(false);

		for child in self.children.borrow().iter() {
			child.destroy();
		}
		// SAFETY: This destroys only the window owned by this widget.
		// Subclasses release their retained callback contexts on
		// WM_NCDESTROY.
		unsafe {
			if self.frame_alive.get() {
				let _ = KillTimer(Some(self.frame), 1);
			}

			if let Some(control) = self.control.filter(|_| self.control_alive.get()) {
				self.accessibility
					.services()
					.ClearHwndProps(
						control,
						OBJID_CLIENT.0 as u32,
						CHILDID_SELF,
						&[PROPID_ACC_NAME, PROPID_ACC_STATE, PROPID_ACC_VALUE],
					)
					.expect("owned control annotations can be cleared");
			}

			// Frames without a control always carry a name and role.
			if self.frame_alive.get() && (self.control.is_none() || self.group_annotated.get()) {
				self.accessibility
					.services()
					.ClearHwndProps(
						self.frame,
						OBJID_CLIENT.0 as u32,
						CHILDID_SELF,
						&[PROPID_ACC_NAME, PROPID_ACC_ROLE, PROPID_ACC_STATE],
					)
					.expect("owned frame annotations can be cleared");
			}

			if self.composition_alive.get() {
				if self.group_annotated.get() {
					self.accessibility
						.services()
						.ClearHwndProps(
							self.composition,
							OBJID_CLIENT.0 as u32,
							CHILDID_SELF,
							&[PROPID_ACC_NAME, PROPID_ACC_STATE],
						)
						.expect("owned group annotations can be cleared");
				}
				DestroyWindow(self.composition).expect("owned widget group can be destroyed");
			}
		}
	}
}

impl Drop for Widget {
	fn drop(&mut self) {
		self.destroy();
	}
}

impl NativeBackend for WindowsBackend {
	fn apply(&mut self, mutations: &MutationList) {
		let _suppressed = self.sink.suppress();
		self.applying = true;

		for mutation in mutations.mutations() {
			self.apply_one(mutation);
		}

		self.applying = false;
		self.refresh_tree();
	}

	fn create(&mut self, id: NodeId, kind: NativeKind, props: &NativeProps) {
		let _suppressed = self.sink.suppress();
		// SAFETY: All windows are created and owned on this backend's UI
		// thread.
		let (composition, frame, control, document) = unsafe {
			let instance = GetModuleHandleW(None).expect("process module exists");
			let composition = CreateWindowExW(
				WS_EX_CONTROLPARENT,
				FRAME_CLASS,
				w!(""),
				WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
				0,
				0,
				0,
				0,
				Some(self.parking),
				None,
				Some(instance.into()),
				None,
			)
			.expect("Windows could not create a native composition group");
			let frame = CreateWindowExW(
				WS_EX_CONTROLPARENT,
				FRAME_CLASS,
				w!(""),
				WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
				0,
				0,
				0,
				0,
				Some(composition),
				None,
				Some(instance.into()),
				None,
			)
			.expect("Windows could not create a native frame");
			let class_and_style = match kind {
				NativeKind::Text | NativeKind::TextRun => {
					Some((w!("STATIC"), SS_LEFT.0 | SS_NOPREFIX.0))
				}
				NativeKind::Button | NativeKind::Pressable => {
					Some((w!("BUTTON"), BS_PUSHBUTTON as u32))
				}
				NativeKind::Switch | NativeKind::Checkbox => {
					Some((w!("BUTTON"), BS_CHECKBOX as u32))
				}
				NativeKind::Radio => Some((w!("BUTTON"), BS_RADIOBUTTON as u32)),
				NativeKind::TextInput => Some((w!("EDIT"), ES_AUTOHSCROLL as u32)),
				NativeKind::PasswordInput => {
					Some((w!("EDIT"), (ES_AUTOHSCROLL | ES_PASSWORD) as u32))
				}
				NativeKind::Slider => Some((TRACKBAR_CLASSW, TBS_HORZ | TBS_NOTICKS)),
				NativeKind::LinearProgressIndicator => Some((PROGRESS_CLASSW, PBS_SMOOTH)),
				_ => None,
			};
			let control = class_and_style.map(|(class, style)| {
				let tab = if matches!(
					kind,
					NativeKind::Text | NativeKind::TextRun | NativeKind::LinearProgressIndicator
				) {
					WINDOW_STYLE::default()
				} else {
					WS_TABSTOP
				};
				CreateWindowExW(
					WINDOW_EX_STYLE::default(),
					class,
					w!(""),
					WS_CHILD | WS_VISIBLE | tab | WINDOW_STYLE(style),
					0,
					0,
					0,
					0,
					Some(frame),
					None,
					Some(instance.into()),
					None,
				)
				.expect("Windows could not create a standard native control")
			});

			if kind == NativeKind::Slider {
				let control = control.expect("slider has a trackbar");
				SendMessageW(control, TBM_SETRANGEMIN, Some(WPARAM(0)), Some(LPARAM(0)));
				SendMessageW(
					control,
					TBM_SETRANGEMAX,
					Some(WPARAM(1)),
					Some(LPARAM(SLIDER_TICKS as isize)),
				);
			}

			let document = (kind == NativeKind::ScrollView).then(|| {
				CreateWindowExW(
					WS_EX_CONTROLPARENT,
					FRAME_CLASS,
					w!(""),
					WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
					0,
					0,
					0,
					0,
					Some(frame),
					None,
					Some(instance.into()),
					None,
				)
				.expect("Windows could not create a scroll document")
			});
			(composition, frame, control, document)
		};
		let widget = Rc::new(Widget {
			id,
			kind,
			frame,
			composition,
			control,
			document,
			props: RefCell::default(),
			parent: RefCell::default(),
			children: RefCell::default(),
			layout: Cell::new([0.0; 4]),
			composition_bounds: Cell::new([0.0; 4]),
			scroll: Cell::new([0; 2]),
			extent: Cell::new([0; 2]),
			animating: Cell::new(false),
			phase: Cell::new(0),
			alive: Cell::new(true),
			frame_alive: Cell::new(true),
			control_alive: Cell::new(control.is_some()),
			document_alive: Cell::new(document.is_some()),
			composition_alive: Cell::new(true),
			destroying: Cell::new(false),
			detached: Cell::new(false),
			dark: Cell::new(self.dark),
			font: RefCell::new(Font::new(props.text_style(), self.dpi.get() as f32 / 96.0)),
			brush: RefCell::new(Brush::new(Color::rgba(255, 255, 255, 255))),
			image: RefCell::default(),
			image_error: RefCell::default(),
			sink: Rc::clone(&self.sink),
			host: Rc::clone(&self.host),
			dpi: Rc::clone(&self.dpi),
			parking: self.parking,
			decorative_annotation: Cell::new(false),
			group_annotated: Cell::new(false),
			repaint_group: Cell::new(false),
			accessibility: self.accessibility.clone(),
		});
		install_subclass(&widget, frame, false);
		install_subclass(&widget, composition, false);

		if let Some(control) = control {
			install_subclass(&widget, control, true);
		}
		if let Some(document) = document {
			install_subclass(&widget, document, false);
		}

		widget.update_props(props);
		self.widgets
			.resize_with(self.widgets.len().max(id.index() + 1), || None);
		assert!(
			self.widgets[id.index()].replace(widget).is_none(),
			"native node ids must be unique"
		);
		self.refresh_tree();
	}

	fn update_props(&mut self, id: NodeId, props: &NativeProps) {
		let _suppressed = self.sink.suppress();

		if let Some(widget) = self.widget(id) {
			crate::accessibility::announce_live(widget, props);
			widget.update_props(props);
		}

		self.refresh_tree();
	}

	fn update_layout(&mut self, id: NodeId, x: f32, y: f32, width: f32, height: f32) {
		if let Some(widget) = self.widget(id) {
			widget.layout.set([x, y, width, height]);
		}

		self.refresh_tree();
	}

	fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
		let parent = Rc::clone(self.widget(parent).expect("insert parent exists"));
		let child = Rc::clone(self.widget(child).expect("insert child exists"));
		*child.parent.borrow_mut() = Rc::downgrade(&parent);
		child.detached.set(false);
		let mut children = parent.children.borrow_mut();
		let index = index.min(children.len());
		children.insert(index, Rc::clone(&child));
		drop(children);
		self.refresh_tree();
	}

	fn remove_child(&mut self, parent: NodeId, child: NodeId) {
		let _suppressed = self.sink.suppress();

		if let Some(parent) = self.widget(parent) {
			parent
				.children
				.borrow_mut()
				.retain(|widget| widget.id != child);
		}

		if let Some(child) = self.widget(child) {
			*child.parent.borrow_mut() = Weak::new();
			child.detached.set(true);

			// Closing a host destroys its mounted HWNDs before the guest
			// commits the removal, so only a live group is parked.
			if child.composition_alive.get() {
				// SAFETY: Parking keeps detached windows owned until Delete
				// arrives.
				unsafe {
					SetParent(child.composition, Some(self.parking))
						.expect("owned child can be detached");
				}
			}
		}

		self.refresh_tree();
	}

	fn delete(&mut self, id: NodeId) {
		let _suppressed = self.sink.suppress();

		if let Some(widget) = self.widgets.get_mut(id.index()).and_then(Option::take) {
			widget.destroy();
		}
	}

	fn text_measurer(&self) -> SharedTextMeasurer {
		Arc::new(WindowsTextMeasurer::new(self.dpi.get()))
	}

	fn set_dark_mode(&mut self, dark: bool) {
		let _suppressed = self.sink.suppress();
		self.dark = dark;

		for widget in self.widgets.iter().flatten() {
			widget.dark.set(dark);
			let props = widget.props.borrow().clone();
			widget.update_props(&props);
		}
	}
}

impl Drop for WindowsBackend {
	fn drop(&mut self) {
		let _suppressed = self.sink.suppress();

		for widget in self.widgets.iter().flatten() {
			widget.destroy();
		}
		self.widgets.clear();
		// SAFETY: All child windows are gone. The shared apartment releases COM
		// only after any in-flight native callback also releases its widget.
		unsafe {
			let _ = DestroyWindow(self.parking);
		}
	}
}

/// MSAA role and state annotations must be `VT_I4`. The `windows` crate
/// declares those constants as `u32`, whose `VARIANT` is `VT_UI4`, which
/// `IAccPropServices` rejects with `E_INVALIDARG`.
fn msaa_i4(value: u32) -> windows::Win32::System::Variant::VARIANT {
	windows::Win32::System::Variant::VARIANT::from(
		i32::try_from(value).expect("MSAA roles and states fit in VT_I4"),
	)
}

pub(crate) fn wide(text: &str) -> Vec<u16> {
	text.encode_utf16().chain(Some(0)).collect()
}

pub(crate) fn window_text(hwnd: HWND) -> String {
	// SAFETY: The owned edit window writes at most the supplied buffer length.
	unsafe {
		let mut value = vec![0; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
		let length = GetWindowTextW(hwnd, &mut value).max(0) as usize;
		String::from_utf16_lossy(&value[..length])
	}
}

fn load_image(source: &str) -> Result<image::RgbaImage, String> {
	let path = if source.starts_with("file:") {
		let mut path = vec![0_u16; 32_768];
		let mut length = path.len() as u32;
		// SAFETY: Windows decodes percent escapes, drive paths, and UNC file
		// URLs into a bounded native path buffer without accessing the file.
		unsafe {
			windows::Win32::UI::Shell::PathCreateFromUrlW(
				PCWSTR(wide(source).as_ptr()),
				windows::core::PWSTR(path.as_mut_ptr()),
				&raw mut length,
				0,
			)
		}
		.map_err(|_| "invalid local file URL".to_owned())?;
		path.truncate(
			path.iter()
				.position(|value| *value == 0)
				.unwrap_or(length as usize),
		);
		std::path::PathBuf::from(std::ffi::OsString::from_wide(&path))
	} else {
		if source.contains("://") {
			return Err("image source must be a local file".to_owned());
		}
		std::path::PathBuf::from(source)
	};

	image::open(path)
		.map(image::DynamicImage::into_rgba8)
		.map_err(|_| "image could not be read or decoded".to_owned())
}
