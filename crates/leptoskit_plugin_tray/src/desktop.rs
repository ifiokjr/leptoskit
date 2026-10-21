use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
#[cfg(not(target_os = "linux"))]
use std::sync::Once;
#[cfg(not(target_os = "linux"))]
use std::sync::OnceLock;
#[cfg(not(target_os = "linux"))]
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use leptoskit_plugin_menu::DesktopMenu;
use parking_lot::Mutex;
#[cfg(not(target_os = "linux"))]
use tray_icon::TrayIconEvent;

#[cfg(not(target_os = "linux"))]
#[path = "native.rs"]
mod backend;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod backend;
use backend::NativeTray;

#[cfg(not(target_os = "linux"))]
use crate::Button;
use crate::EVENT_CAPACITY;
use crate::Error;
use crate::Event;
use crate::EventBatch;
use crate::EventKind;
use crate::Options;
#[cfg(not(target_os = "linux"))]
use crate::Point;
use crate::Rect;
use crate::RgbaIcon;
use crate::TRAY_CAPACITY;
use crate::TrayId;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
#[cfg(not(target_os = "linux"))]
static HANDLER: Once = Once::new();
#[cfg(not(target_os = "linux"))]
static ROUTER: OnceLock<Mutex<Weak<Events>>> = OnceLock::new();

struct Exclusive;
impl Exclusive {
	fn claim() -> Result<Self, Error> {
		ACTIVE
			.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
			.map(|_| Self)
			.map_err(|_| Error::AlreadyOwned)
	}
}
impl Drop for Exclusive {
	fn drop(&mut self) {
		ACTIVE.store(false, Ordering::Release);
	}
}

struct Queue {
	live: bool,
	ids: BTreeMap<String, TrayId>,
	events: VecDeque<Event>,
	lost: u64,
	coalesced: u64,
	wake_pending: bool,
	wake_failed: bool,
}
impl Default for Queue {
	fn default() -> Self {
		Self {
			live: true,
			ids: BTreeMap::new(),
			events: VecDeque::with_capacity(EVENT_CAPACITY),
			lost: 0,
			coalesced: 0,
			wake_pending: false,
			wake_failed: false,
		}
	}
}
impl Queue {
	fn push(&mut self, event: Event) -> bool {
		if !self.live || !self.ids.values().any(|id| *id == event.id) {
			return false;
		}
		if event.kind == EventKind::Move
			&& self
				.events
				.back()
				.is_some_and(|last| last.id == event.id && last.kind == EventKind::Move)
		{
			if let Some(last) = self.events.back_mut() {
				*last = event;
			}
			self.coalesced = self.coalesced.saturating_add(1);
		} else if self.events.len() == EVENT_CAPACITY {
			self.lost = self.lost.saturating_add(1);
		} else {
			self.events.push_back(event);
		}
		!std::mem::replace(&mut self.wake_pending, true)
	}

	fn remove(&mut self, id: TrayId) {
		self.ids.retain(|_, value| *value != id);
		self.events.retain(|event| event.id != id);
		if self.events.is_empty() {
			self.wake_pending = false;
		}
	}

	fn close(&mut self) {
		self.live = false;
		self.ids.clear();
		self.events.clear();
	}

	fn drain(&mut self) -> EventBatch {
		self.wake_pending = false;
		EventBatch {
			events: self.events.drain(..).collect(),
			lost: std::mem::take(&mut self.lost),
			coalesced: std::mem::take(&mut self.coalesced),
			wake_failed: std::mem::take(&mut self.wake_failed),
		}
	}
}
struct Events {
	queue: Mutex<Queue>,
	wake: Box<dyn Fn() + Send + Sync>,
}
impl Events {
	#[cfg(not(target_os = "linux"))]
	fn receive(&self, native: &TrayIconEvent) {
		let mut queue = self.queue.lock();
		let Some(id) = queue.ids.get(native.id().as_ref()).copied() else {
			return;
		};
		let Some(event) = convert(native, id) else {
			return;
		};
		if queue.push(event)
			&& std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.wake)())).is_err()
		{
			queue.wake_failed = true;
		}
	}

	#[cfg(target_os = "linux")]
	fn push(&self, event: Event) {
		let mut queue = self.queue.lock();
		if queue.push(event)
			&& std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.wake)())).is_err()
		{
			queue.wake_failed = true;
		}
	}
}

#[cfg(not(target_os = "linux"))]
fn button(value: tray_icon::MouseButton) -> Button {
	match value {
		tray_icon::MouseButton::Left => Button::Left,
		tray_icon::MouseButton::Right => Button::Right,
		tray_icon::MouseButton::Middle => Button::Middle,
	}
}
#[cfg(not(target_os = "linux"))]
fn convert(native: &TrayIconEvent, id: TrayId) -> Option<Event> {
	let (kind, position, rect) = match native {
		TrayIconEvent::Click {
			button: value,
			button_state,
			position,
			rect,
			..
		} => {
			let kind = if *button_state == tray_icon::MouseButtonState::Down {
				EventKind::Pressed(button(*value))
			} else {
				EventKind::Released(button(*value))
			};
			(kind, position, rect)
		}
		TrayIconEvent::DoubleClick {
			button: value,
			position,
			rect,
			..
		} => (EventKind::DoubleClick(button(*value)), position, rect),
		TrayIconEvent::Enter { position, rect, .. } => (EventKind::Enter, position, rect),
		TrayIconEvent::Move { position, rect, .. } => (EventKind::Move, position, rect),
		TrayIconEvent::Leave { position, rect, .. } => (EventKind::Leave, position, rect),
		_ => return None,
	};
	let position = (position.x.is_finite() && position.y.is_finite()).then_some(Point {
		x: position.x,
		y: position.y,
	});
	let bounds = (rect.position.x.is_finite() && rect.position.y.is_finite()).then_some(Rect {
		position: Point {
			x: rect.position.x,
			y: rect.position.y,
		},
		width: rect.size.width,
		height: rect.size.height,
	});
	Some(Event {
		id,
		kind,
		position,
		bounds,
	})
}

struct OwnedTray {
	// Native tray must drop before the menu wrapper that owns its event mapping.
	native: NativeTray,
	menu: Option<Rc<DesktopMenu>>,
}

/// Explicit process-wide tray owner, confined to its creating thread.
///
/// Windows requires its native message loop on that thread. macOS additionally
/// checks that the main-thread AppKit loop is running. Linux KSNI owns its
/// D-Bus worker thread and needs a session bus plus a compatible desktop tray
/// host.
///
/// This owner exclusively uses tray-icon's process-global callback. Install it
/// before creating any tray-icon objects or calling its event APIs elsewhere;
/// the upstream callback is write-once and cannot detect an earlier third-party
/// installation. This crate never touches muda's separate menu-event callback.
pub struct TrayHost {
	trays: BTreeMap<TrayId, OwnedTray>,
	events: Arc<Events>,
	_thread: PhantomData<Rc<()>>,
	_exclusive: Exclusive,
}
impl TrayHost {
	/// Installs this application's explicit event owner. No icon is created.
	///
	/// `wake` must only enqueue a nonblocking host event. It must not reenter
	/// this owner, drain events, block, access UI, or invoke a guest. Wakeups
	/// coalesce until drain. Drop synchronizes with active callbacks so no new
	/// wake runs after it returns. Callback panics are reported in
	/// `EventBatch`.
	pub fn install(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, Error> {
		#[cfg(target_os = "macos")]
		{
			let marker = objc2::MainThreadMarker::new().ok_or(Error::WrongThread)?;
			if !objc2_app_kit::NSApplication::sharedApplication(marker).isRunning() {
				return Err(Error::EventLoopNotRunning);
			}
		}
		let exclusive = Exclusive::claim()?;
		let events = Arc::new(Events {
			queue: Mutex::new(Queue::default()),
			wake: Box::new(wake),
		});
		#[cfg(not(target_os = "linux"))]
		{
			let router = ROUTER.get_or_init(|| Mutex::new(Weak::new()));
			HANDLER.call_once(|| {
				TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
					let events = ROUTER.get().and_then(|router| router.lock().upgrade());
					if let Some(events) = events {
						events.receive(&event);
					}
				}));
			});
			*router.lock() = Arc::downgrade(&events);
		}
		Ok(Self {
			trays: BTreeMap::new(),
			events,
			_thread: PhantomData,
			_exclusive: exclusive,
		})
	}

	/// Creates one native tray and retains the optional menu wrapper. Menus use
	/// their own explicit event routing; keep pumping that channel too.
	pub fn create(
		&mut self,
		icon: RgbaIcon,
		options: &Options,
		menu: Option<Rc<DesktopMenu>>,
	) -> Result<TrayId, Error> {
		options.validate()?;
		if self.trays.len() >= TRAY_CAPACITY {
			return Err(Error::Capacity);
		}
		let id = TrayId(
			NEXT_ID
				.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
				.map_err(|_| Error::Capacity)?,
		);
		let native_id = format!("leptoskit-tray-{}", id.0);
		let native = NativeTray::new(
			id,
			&native_id,
			icon,
			options,
			menu.as_deref(),
			Arc::clone(&self.events),
		)?;
		self.events.queue.lock().ids.insert(native_id, id);
		self.trays.insert(id, OwnedTray { native, menu });
		Ok(id)
	}

	/// Removes callbacks and queued events before dropping the native resource.
	pub fn remove(&mut self, id: TrayId) -> Result<(), Error> {
		if !self.trays.contains_key(&id) {
			return Err(Error::UnknownTray);
		}
		self.events.queue.lock().remove(id);
		self.trays.remove(&id);
		Ok(())
	}

	/// Changes the RGBA icon. Template rendering is macOS-only.
	pub fn set_icon(&mut self, id: TrayId, icon: RgbaIcon, template: bool) -> Result<(), Error> {
		if template && !cfg!(target_os = "macos") {
			return Err(Error::Unavailable);
		}
		let tray = self.tray(id)?;
		tray.native.set_icon(icon, template)
	}

	/// Changes/clears hover text. Success acknowledges the native request, not
	/// whether a desktop chose to display a tooltip.
	pub fn set_tooltip(&mut self, id: TrayId, text: Option<&str>) -> Result<(), Error> {
		crate::validate_tooltip(text)?;
		self.tray(id)?.native.set_tooltip(text)
	}

	/// Changes/clears macOS title or Linux SNI Title. Windows is unsupported.
	pub fn set_title(&mut self, id: TrayId, title: Option<&str>) -> Result<(), Error> {
		crate::validate_title(title)?;
		if cfg!(target_os = "windows") {
			return Err(Error::Unavailable);
		}
		self.tray(id)?.native.set_title(title)?;
		Ok(())
	}

	/// Requests native visibility; the desktop may lack a tray host or suppress
	/// an icon. This is not a visibility-observation API.
	pub fn set_visible(&mut self, id: TrayId, visible: bool) -> Result<(), Error> {
		self.tray(id)?.native.set_visible(visible)
	}

	/// Replaces or removes the context menu while retaining its Rust owner.
	pub fn set_menu(&mut self, id: TrayId, menu: Option<Rc<DesktopMenu>>) -> Result<(), Error> {
		let tray = self.trays.get_mut(&id).ok_or(Error::UnknownTray)?;
		tray.native.set_menu(menu.as_deref())?;
		tray.menu = menu;
		Ok(())
	}

	/// Drains at most `EVENT_CAPACITY` owned events and resets overflow
	/// counters.
	#[must_use]
	pub fn drain(&mut self) -> EventBatch {
		self.events.queue.lock().drain()
	}

	/// Native icon bounds, or None when absent/unsupported (including Linux).
	pub fn bounds(&self, id: TrayId) -> Result<Option<Rect>, Error> {
		self.tray(id)?.native.bounds()
	}

	/// Notify Linux of mutations made through the retained menu wrapper.
	/// Native macOS/Windows menus update directly; this is a no-op there.
	pub fn refresh_menu(&mut self, id: TrayId) -> Result<(), Error> {
		self.tray(id)?.native.refresh_menu()
	}

	fn tray(&self, id: TrayId) -> Result<&OwnedTray, Error> {
		self.trays.get(&id).ok_or(Error::UnknownTray)
	}
}
impl Drop for TrayHost {
	fn drop(&mut self) {
		self.events.queue.lock().close();
		self.trays.clear();
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	use crate::Button;
	use crate::Point;
	fn sample(id: TrayId, kind: EventKind, x: f64) -> Event {
		Event {
			id,
			kind,
			position: Some(Point { x, y: 0.0 }),
			bounds: None,
		}
	}
	#[test]
	fn coalescing_preserves_click_order_and_bounds_overflow() {
		let id = TrayId(1);
		let mut queue = Queue::default();
		queue.ids.insert("owned".into(), id);
		assert!(queue.push(sample(id, EventKind::Move, 1.0)));
		assert!(!queue.push(sample(id, EventKind::Move, 2.0)));
		queue.push(sample(id, EventKind::Pressed(Button::Left), 3.0));
		queue.push(sample(id, EventKind::Move, 4.0));
		let batch = queue.drain();
		assert_eq!(batch.events.len(), 3);
		assert_eq!(batch.coalesced, 1);
		assert_eq!(batch.events[0].position, Some(Point { x: 2.0, y: 0.0 }));
		for _ in 0..EVENT_CAPACITY + 9 {
			queue.push(sample(id, EventKind::Pressed(Button::Left), 0.0));
		}
		let batch = queue.drain();
		assert_eq!(batch.events.len(), EVENT_CAPACITY);
		assert_eq!(batch.lost, 9);
	}
	#[test]
	fn removal_and_close_reject_queued_and_late_events() {
		let id = TrayId(1);
		let mut queue = Queue::default();
		queue.ids.insert("owned".into(), id);
		queue.push(sample(id, EventKind::Move, 1.0));
		queue.remove(id);
		assert!(queue.drain().events.is_empty());
		assert!(!queue.push(sample(id, EventKind::Move, 2.0)));
		queue.ids.insert("new".into(), TrayId(2));
		queue.close();
		assert!(!queue.push(sample(TrayId(2), EventKind::Move, 3.0)));
	}
	#[test]
	#[cfg(not(target_os = "linux"))]
	fn native_events_keep_their_button_transition_and_finite_geometry() {
		use tray_icon::MouseButton;
		use tray_icon::MouseButtonState;
		use tray_icon::dpi::PhysicalPosition;
		use tray_icon::dpi::PhysicalSize;
		let id = TrayId(7);
		let rect = tray_icon::Rect {
			position: PhysicalPosition::new(10.0, 20.0),
			size: PhysicalSize::new(22, 24),
		};
		let click = |button, button_state| {
			TrayIconEvent::Click {
				id: "owned".into(),
				position: PhysicalPosition::new(1.5, 2.5),
				rect,
				button,
				button_state,
			}
		};
		let pressed = convert(&click(MouseButton::Left, MouseButtonState::Down), id).unwrap();
		assert_eq!(pressed.kind, EventKind::Pressed(Button::Left));
		assert_eq!(pressed.position, Some(Point { x: 1.5, y: 2.5 }));
		assert_eq!(
			pressed.bounds,
			Some(Rect {
				position: Point { x: 10.0, y: 20.0 },
				width: 22,
				height: 24,
			})
		);
		assert_eq!(
			convert(&click(MouseButton::Right, MouseButtonState::Up), id)
				.unwrap()
				.kind,
			EventKind::Released(Button::Right)
		);
		let double = TrayIconEvent::DoubleClick {
			id: "owned".into(),
			position: PhysicalPosition::new(f64::NAN, 0.0),
			rect: tray_icon::Rect {
				position: PhysicalPosition::new(f64::INFINITY, 0.0),
				size: PhysicalSize::new(1, 1),
			},
			button: MouseButton::Middle,
		};
		let double = convert(&double, id).unwrap();
		assert_eq!(double.kind, EventKind::DoubleClick(Button::Middle));
		assert_eq!((double.position, double.bounds), (None, None));
		for (native, kind) in [
			(
				TrayIconEvent::Enter {
					id: "owned".into(),
					position: PhysicalPosition::default(),
					rect,
				},
				EventKind::Enter,
			),
			(
				TrayIconEvent::Leave {
					id: "owned".into(),
					position: PhysicalPosition::default(),
					rect,
				},
				EventKind::Leave,
			),
		] {
			assert_eq!(convert(&native, id).unwrap().kind, kind);
		}
	}
	#[test]
	#[cfg(not(target_os = "linux"))]
	fn callback_panics_are_reported_without_unwinding_native_code() {
		let mut queue = Queue::default();
		queue.ids.insert("owned".into(), TrayId(1));
		let events = Events {
			queue: Mutex::new(queue),
			wake: Box::new(|| panic!("synthetic wake failure")),
		};
		let native = TrayIconEvent::Move {
			id: "owned".into(),
			position: tray_icon::dpi::PhysicalPosition::new(0.0, 0.0),
			rect: tray_icon::Rect::default(),
		};
		events.receive(&native);
		assert!(events.queue.lock().drain().wake_failed);
		events.queue.lock().close();
		events.receive(&native);
		assert!(!events.queue.lock().drain().wake_failed);
	}
}
