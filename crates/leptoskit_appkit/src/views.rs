//! The Objective-C classes the backend defines: a flipped container view and
//! the target that turns control actions into leptoskit events.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use leptoskit_native::NativeEvent;
use leptoskit_native::NativeProps;
use leptoskit_native::RadioGroup;
use leptoskit_native::SliderState;
use leptoskit_yoga::NodeId;
use objc2::DefinedClass;
use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::Message;
use objc2::define_class;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::rc::Weak;
use objc2::runtime::AnyObject;
use objc2::runtime::NSObject;
use objc2::runtime::NSObjectProtocol;
use objc2_app_kit::NSButton;
use objc2_app_kit::NSClipView;
use objc2_app_kit::NSControl;
use objc2_app_kit::NSControlTextEditingDelegate;
use objc2_app_kit::NSEvent;
use objc2_app_kit::NSResponder;
use objc2_app_kit::NSSlider;
use objc2_app_kit::NSSwitch;
use objc2_app_kit::NSTextField;
use objc2_app_kit::NSTextFieldDelegate;
use objc2_app_kit::NSView;
use objc2_foundation::NSNotification;
use objc2_foundation::NSRect;
use objc2_foundation::NSString;

define_class!(
	// SAFETY:
	// - `NSView` has no subclassing requirements beyond being initialized
	//   through its designated initializer, which `FlippedView::new` does.
	// - The class does not implement `Drop`.
	#[unsafe(super(NSView, NSResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	/// A container view whose origin is the top-left corner, matching the
	/// layout engine's coordinate system.
	pub(crate) struct FlippedView;

	impl FlippedView {
		#[unsafe(method(isFlipped))]
		fn is_flipped(&self) -> bool {
			true
		}
	}
);

impl FlippedView {
	/// A layer-backed flipped view with an empty frame.
	pub(crate) fn new(mtm: MainThreadMarker) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(());
		// SAFETY: `initWithFrame:` is `NSView`'s designated initializer and
		// returns an initialized instance of the receiver's class.
		let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
		view.setWantsLayer(true);
		view
	}
}

define_class!(
	// SAFETY: NSButton is initialized through its designated initializer and
	// every non-arrow key is passed to the platform button implementation.
	#[unsafe(super(NSButton, NSControl, NSView, NSResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = Weak<ActionTarget>]
	/// A native radio button whose arrows follow the explicit shared group.
	pub(crate) struct RadioButton;

	impl RadioButton {
		#[unsafe(method(keyDown:))]
		fn key_down(&self, event: &NSEvent) {
			let _keep_alive = self.retain();
			let character = event
				.charactersIgnoringModifiers()
				.map(|value| value.to_string());
			let forward = match character.as_deref() {
				Some("\u{f700}" | "\u{f702}") => Some(false),
				Some("\u{f701}" | "\u{f703}") => Some(true),
				_ => None,
			};

			if let (Some(forward), Some(target)) = (forward, self.ivars().load()) {
				target.move_radio(forward);
			} else {
				// SAFETY: NSButton implements keyDown: with this exact signature.
				unsafe {
					let _: () = msg_send![super(self), keyDown: event];
				}
			}
		}
	}
);

impl RadioButton {
	pub(crate) fn new(mtm: MainThreadMarker, target: &ActionTarget) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(Weak::new(target));
		// SAFETY: initWithFrame: is NSButton's designated initializer.
		unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
	}
}

define_class!(
	// SAFETY: NSButton is initialized through its designated initializer, and
	// mouse tracking still runs through NSButton's own mouseDown:.
	#[unsafe(super(NSButton, NSControl, NSView, NSResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = Weak<ActionTarget>]
	/// A push button that reports when the pointer holds it down.
	pub(crate) struct PressButton;

	impl PressButton {
		#[unsafe(method(mouseDown:))]
		fn mouse_down(&self, event: &NSEvent) {
			track_press(self, self.ivars().load(), || {
				// SAFETY: NSButton implements mouseDown: with this exact
				// signature.
				unsafe {
					let _: () = msg_send![super(self), mouseDown: event];
				}
			});
		}
	}
);

impl PressButton {
	pub(crate) fn new(mtm: MainThreadMarker, target: &ActionTarget) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(Weak::new(target));
		// SAFETY: initWithFrame: is NSButton's designated initializer.
		unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
	}
}

/// Brackets a button's mouse tracking with press phases.
///
/// `NSButton`'s `mouseDown:` runs its tracking loop and returns only once
/// the button is released, so the control shows its pressed style for the
/// whole hold, as CSS `:active` does, even while the pointer strays outside
/// it. The action, and so any [`NativeEvent::Press`], fires inside `track`.
/// A target invalidated by that press's re-render reports nothing more.
pub(crate) fn track_press(
	button: &NSButton,
	target: Option<Retained<ActionTarget>>,
	track: impl FnOnce(),
) {
	let _keep_alive = button.retain();
	let target = target.filter(|_| button.isEnabled());

	if let Some(target) = &target {
		target.emit(&NativeEvent::PressIn);
	}

	track();

	if let Some(target) = &target {
		target.emit(&NativeEvent::PressOut);
	}
}

define_class!(
	// SAFETY: NSSlider is initialized through its designated initializer;
	// non-arrow keys retain the native slider's keyboard implementation.
	#[unsafe(super(NSSlider, NSControl, NSView, NSResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = Weak<ActionTarget>]
	/// A slider whose keyboard increments honor the shared step grid.
	pub(crate) struct ControlledSlider;

	impl ControlledSlider {
		#[unsafe(method(keyDown:))]
		fn key_down(&self, event: &NSEvent) {
			let _keep_alive = self.retain();
			let character = event
				.charactersIgnoringModifiers()
				.map(|value| value.to_string());
			let forward = match character.as_deref() {
				Some("\u{f700}" | "\u{f703}") => Some(true),
				Some("\u{f701}" | "\u{f702}") => Some(false),
				_ => None,
			};

			if let (Some(forward), Some(target)) = (forward, self.ivars().load()) {
				let state = target.ivars().slider.get();
				let increment = state.step().unwrap_or((state.max() - state.min()) / 100.0);
				let value = if forward {
					state.value() + increment.min(state.max() - state.value())
				} else {
					state.value() - increment.min(state.value() - state.min())
				};

				target.propose_slider(self, value);
			} else {
				// SAFETY: NSSlider implements keyDown: with this exact signature.
				unsafe {
					let _: () = msg_send![super(self), keyDown: event];
				}
			}
		}
	}
);

impl ControlledSlider {
	pub(crate) fn new(mtm: MainThreadMarker, target: &ActionTarget) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(Weak::new(target));
		// SAFETY: initWithFrame: is NSSlider's designated initializer.
		unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
	}
}

/// Receives the events native widgets emit, as `(node, event)` pairs.
pub type EventListener = Rc<dyn Fn(NodeId, &NativeEvent)>;

/// Where widget actions deliver their events: the listener the app
/// installed, if any.
#[derive(Default)]
pub(crate) struct EventSink {
	listener: RefCell<Option<EventListener>>,
	radios: RefCell<Vec<RadioMember>>,
	suppressed: Cell<bool>,
}

/// Keeps synchronous AppKit focus/submit notifications inside a mutation
/// batch from reentering the application while its surface is borrowed.
pub(crate) struct EventSuppression {
	sink: Rc<EventSink>,
	previous: bool,
}

impl Drop for EventSuppression {
	fn drop(&mut self) {
		self.sink.suppressed.set(self.previous);
	}
}

/// Weak references avoid a sink → widget → target → sink ownership cycle.
pub(crate) struct RadioMember {
	pub(crate) button: Weak<NSButton>,
	pub(crate) target: Weak<ActionTarget>,
}

impl EventSink {
	pub(crate) fn suppress(self: &Rc<Self>) -> EventSuppression {
		EventSuppression {
			sink: Rc::clone(self),
			previous: self.suppressed.replace(true),
		}
	}

	pub(crate) fn set_listener(&self, listener: Option<EventListener>) {
		*self.listener.borrow_mut() = listener;
	}

	pub(crate) fn set_radios(&self, radios: Vec<RadioMember>) {
		*self.radios.borrow_mut() = radios;
	}

	fn restore_radios(&self) {
		let radios: Vec<_> = self
			.radios
			.borrow()
			.iter()
			.filter_map(|member| Some((member.button.load()?, member.target.load()?)))
			.collect();

		for (button, target) in radios {
			if target.ivars().active.get() {
				button.setState(isize::from(target.ivars().checked.get()));
			}
		}
	}

	fn move_radio(&self, node: NodeId, group: &RadioGroup, forward: bool) {
		let radios: Vec<_> = self
			.radios
			.borrow()
			.iter()
			.filter_map(|member| {
				let button = member.button.load()?;
				let target = member.target.load()?;

				if !target.ivars().active.get()
					|| !button.isEnabled()
					|| button.isHiddenOrHasHiddenAncestor()
					|| target.ivars().radio_group.borrow().as_ref() != Some(group)
				{
					return None;
				}

				Some((button, target))
			})
			.collect();
		let Some(index) = radios
			.iter()
			.position(|(_, target)| target.ivars().node == node)
		else {
			return;
		};
		let next = if forward {
			(index + 1) % radios.len()
		} else {
			(index + radios.len() - 1) % radios.len()
		};

		let (button, target) = &radios[next];

		if let Some(window) = button.window() {
			window.makeFirstResponder(Some(button));
		}

		target.propose_checked(button, true);
	}

	/// Delivers an event to the listener.
	///
	/// The listener is cloned out before it runs, so it may replace itself or
	/// rebuild the whole interface — deleting the very widget that fired —
	/// without a `RefCell` conflict.
	fn emit(&self, node: NodeId, event: &NativeEvent) {
		if self.suppressed.get() {
			return;
		}

		let listener = self.listener.borrow().clone();

		if let Some(listener) = listener {
			listener(node, event);
		}
	}
}

/// The state of an [`ActionTarget`]: which node it speaks for and where its
/// events go.
pub(crate) struct ActionTargetIvars {
	node: NodeId,
	sink: Rc<EventSink>,
	active: Cell<bool>,
	checked: Cell<bool>,
	radio_group: RefCell<Option<RadioGroup>>,
	slider: Cell<SliderState>,
	value: RefCell<String>,
}

define_class!(
	// SAFETY:
	// - `NSObject` has no subclassing requirements.
	// - The class does not implement `Drop`; its ivars are dropped by objc2.
	#[unsafe(super(NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = ActionTargetIvars]
	/// The target and delegate of one control, translating Objective-C
	/// actions into [`NativeEvent`]s for its node.
	pub(crate) struct ActionTarget;

	impl ActionTarget {
		/// AppKit proposes a new boolean value before application state commits it.
		#[unsafe(method(toggle:))]
		fn toggle(&self, sender: Option<&AnyObject>) {
			let _keep_alive = self.retain();

			if let Some(button) = sender.and_then(|sender| sender.downcast_ref::<NSButton>()) {
				let button = button.retain();
				let checked = self.ivars().radio_group.borrow().is_some() || button.state() != 0;
				self.propose_checked(&button, checked);
			} else if let Some(switch) = sender.and_then(|sender| sender.downcast_ref::<NSSwitch>()) {
				let switch = switch.retain();

				if switch.isEnabled() && !switch.isHiddenOrHasHiddenAncestor() {
					self.emit(&NativeEvent::Toggle { checked: switch.state() != 0 });
				}

				if self.ivars().active.get() {
					switch.setState(isize::from(self.ivars().checked.get()));
				}
			}
		}

		/// A slider reports a finite value on the shared step grid.
		#[unsafe(method(slide:))]
		fn slide(&self, sender: Option<&AnyObject>) {
			let _keep_alive = self.retain();
			let Some(slider) = sender
				.and_then(|sender| sender.downcast_ref::<NSSlider>())
				.map(Message::retain)
			else {
				return;
			};

			self.propose_slider(&slider, slider.doubleValue());
		}

		/// A button was clicked.
		#[unsafe(method(press:))]
		fn press(&self, _sender: Option<&AnyObject>) {
			let _keep_alive = self.retain();
			self.emit(&NativeEvent::Press);
		}

		/// A scroll view's clip view moved its bounds: the user or the
		/// system scrolled. The document view is flipped, so the bounds
		/// origin is the content offset from the top-left corner.
		#[unsafe(method(boundsDidChange:))]
		fn bounds_did_change(&self, notification: &NSNotification) {
			let _keep_alive = self.retain();
			let Some(clip) = notification
				.object()
				.and_then(|object| object.downcast::<NSClipView>().ok())
			else {
				return;
			};
			let origin = clip.bounds().origin;
			self.emit(&NativeEvent::Scroll {
				offset_x: origin.x as f32,
				offset_y: origin.y as f32,
			});
		}

		/// A text field's action fired: the user pressed return.
		#[unsafe(method(submit:))]
		fn submit(&self, sender: Option<&AnyObject>) {
			let value = sender
				.and_then(|sender| sender.downcast_ref::<NSTextField>())
				.map(|field| field.stringValue().to_string())
				.unwrap_or_default();
			self.emit(&NativeEvent::Submit { value });
		}
	}

	unsafe impl NSObjectProtocol for ActionTarget {}

	unsafe impl NSControlTextEditingDelegate for ActionTarget {
		#[unsafe(method(controlTextDidChange:))]
		fn control_text_did_change(&self, notification: &NSNotification) {
			let _keep_alive = self.retain();
			let Some(field) = notification
				.object()
				.and_then(|object| object.downcast::<NSTextField>().ok())
			else {
				return;
			};
			let value = field.stringValue().to_string();

			if field.isEnabled() && !field.isHiddenOrHasHiddenAncestor() {
				self.emit(&NativeEvent::Change { value });
			}

			if self.ivars().active.get()
				&& field.stringValue().to_string() != *self.ivars().value.borrow()
			{
				field.setStringValue(&NSString::from_str(&self.ivars().value.borrow()));
			}
		}

		#[unsafe(method(controlTextDidBeginEditing:))]
		fn control_text_did_begin_editing(&self, _notification: &NSNotification) {
			self.emit(&NativeEvent::Focus);
		}

		#[unsafe(method(controlTextDidEndEditing:))]
		fn control_text_did_end_editing(&self, _notification: &NSNotification) {
			self.emit(&NativeEvent::Blur);
		}
	}

	unsafe impl NSTextFieldDelegate for ActionTarget {}
);

impl ActionTarget {
	pub(crate) fn new(mtm: MainThreadMarker, node: NodeId, sink: Rc<EventSink>) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(ActionTargetIvars {
			node,
			sink,
			active: Cell::new(true),
			checked: Cell::new(false),
			radio_group: RefCell::new(None),
			slider: Cell::new(SliderState::default()),
			value: RefCell::new(String::new()),
		});
		// SAFETY: `init` is `NSObject`'s designated initializer.
		unsafe { msg_send![super(this), init] }
	}

	pub(crate) fn emit(&self, event: &NativeEvent) {
		let _keep_alive = self.retain();
		let ivars = self.ivars();

		if ivars.active.get() {
			ivars.sink.emit(ivars.node, event);
		}
	}

	pub(crate) fn sync_props(&self, props: &NativeProps) {
		self.ivars().checked.set(props.checked.unwrap_or(false));
		self.ivars().radio_group.replace(props.radio_group.clone());
		self.ivars().slider.set(props.slider.unwrap_or_default());
		self.ivars().value.replace(
			props
				.value
				.as_ref()
				.or(props.text.as_ref())
				.cloned()
				.unwrap_or_default(),
		);
	}

	pub(crate) fn invalidate(&self) {
		self.ivars().active.set(false);
	}

	pub(crate) fn move_radio(&self, forward: bool) {
		let _keep_alive = self.retain();
		let group = self.ivars().radio_group.borrow().clone();

		if let Some(group) = group {
			self.ivars()
				.sink
				.move_radio(self.ivars().node, &group, forward);
		}
	}

	fn propose_checked(&self, button: &NSButton, checked: bool) {
		let _keep_alive = self.retain();
		let button = button.retain();

		if button.isEnabled()
			&& !button.isHiddenOrHasHiddenAncestor()
			&& checked != self.ivars().checked.get()
		{
			self.emit(&NativeEvent::Toggle { checked });
		}

		if self.ivars().active.get() {
			button.setState(isize::from(self.ivars().checked.get()));
		}

		self.ivars().sink.restore_radios();
	}

	fn propose_slider(&self, slider: &NSSlider, value: f64) {
		let _keep_alive = self.retain();
		let slider = slider.retain();

		if slider.isEnabled()
			&& !slider.isHiddenOrHasHiddenAncestor()
			&& let Some(value) = self.ivars().slider.get().proposed_value(value)
		{
			self.emit(&NativeEvent::ValueChange { value });
		}

		if self.ivars().active.get() {
			slider.setDoubleValue(self.ivars().slider.get().value());
		}
	}
}
