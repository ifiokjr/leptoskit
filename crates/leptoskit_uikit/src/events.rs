//! Main-thread control targets and reentrant-safe controlled proposals.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
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
use objc2::runtime::NSObject;
use objc2::runtime::NSObjectProtocol;
use objc2_core_foundation::CGPoint;
use objc2_foundation::NSArray;
use objc2_foundation::NSString;
use objc2_ui_kit::UIButton;
use objc2_ui_kit::UIControl;
use objc2_ui_kit::UIControlEvents;
use objc2_ui_kit::UIEvent;
use objc2_ui_kit::UIScrollView;
use objc2_ui_kit::UIScrollViewDelegate;
use objc2_ui_kit::UISlider;
use objc2_ui_kit::UISwitch;
use objc2_ui_kit::UITextField;
use objc2_ui_kit::UITextFieldDelegate;
use objc2_ui_kit::UIView;

/// Receives native user proposals outside borrowed backend state.
pub type EventListener = Rc<dyn Fn(NodeId, &NativeEvent)>;

#[derive(Default)]
pub(crate) struct EventSink {
	listener: RefCell<Option<EventListener>>,
	radios: RefCell<Vec<RadioMember>>,
	suppressed: Cell<bool>,
}

pub(crate) struct RadioMember {
	pub(crate) button: Weak<UIButton>,
	pub(crate) target: Weak<ActionTarget>,
}

pub(crate) struct Suppression {
	sink: Rc<EventSink>,
	previous: bool,
}

impl Drop for Suppression {
	fn drop(&mut self) {
		self.sink.suppressed.set(self.previous);
	}
}

impl EventSink {
	pub(crate) fn suppress(self: &Rc<Self>) -> Suppression {
		Suppression {
			sink: Rc::clone(self),
			previous: self.suppressed.replace(true),
		}
	}

	pub(crate) fn set_listener(&self, listener: Option<EventListener>) {
		self.listener.replace(listener);
	}

	pub(crate) fn set_radios(&self, radios: Vec<RadioMember>) {
		self.radios.replace(radios);
	}

	fn move_radio(&self, node: NodeId, group: &leptoskit_native::RadioGroup, forward: bool) {
		let members: Vec<_> = self
			.radios
			.borrow()
			.iter()
			.filter_map(|member| {
				let button = member.button.load()?;
				let target = member.target.load()?;
				if !target.ivars().active.get()
					|| !button.isEnabled()
					|| !interactive(&button)
					|| target.ivars().props.borrow().radio_group.as_ref() != Some(group)
				{
					return None;
				}
				Some((button, target))
			})
			.collect();
		let Some(index) = members
			.iter()
			.position(|(_, target)| target.ivars().node == node)
		else {
			return;
		};
		let next = if forward {
			(index + 1) % members.len()
		} else {
			(index + members.len() - 1) % members.len()
		};
		let (button, _) = &members[next];
		button.becomeFirstResponder();
		button.sendActionsForControlEvents(UIControlEvents::PrimaryActionTriggered);
	}

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

pub(crate) fn interactive(view: &UIView) -> bool {
	if view.isHidden() || !view.isUserInteractionEnabled() || view.alpha() <= 0.01 {
		return false;
	}
	let mut parent = view.superview();
	while let Some(view) = parent {
		if view.isHidden() || !view.isUserInteractionEnabled() || view.alpha() <= 0.01 {
			return false;
		}
		parent = view.superview();
	}
	true
}

pub(crate) fn visible(view: &UIView) -> bool {
	if view.isHidden() || view.alpha() <= 0.01 {
		return false;
	}
	let mut parent = view.superview();
	while let Some(view) = parent {
		if view.isHidden() || view.alpha() <= 0.01 {
			return false;
		}
		parent = view.superview();
	}
	true
}

pub(crate) fn slider_coordinate(state: SliderState) -> f32 {
	((state.value() - state.min()) / (state.max() - state.min())) as f32
}

pub(crate) fn slider_proposal(state: SliderState, coordinate: f32) -> Option<f64> {
	if !coordinate.is_finite() {
		return None;
	}
	let proposal =
		f64::from(coordinate.clamp(0.0, 1.0)).mul_add(state.max() - state.min(), state.min());
	state.proposed_value(proposal)
}

pub(crate) fn input_value(props: &NativeProps) -> &str {
	props
		.value
		.as_deref()
		.or(props.text.as_deref())
		.unwrap_or_default()
}

pub(crate) struct TargetState {
	node: NodeId,
	kind: NativeKind,
	sink: Rc<EventSink>,
	active: Cell<bool>,
	props: RefCell<NativeProps>,
}

define_class!(
	// SAFETY: NSObject has no subclassing obligations. objc2 drops owned ivars.
	#[unsafe(super(NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = TargetState]
	pub(crate) struct ActionTarget;

	impl ActionTarget {
		#[unsafe(method(activate:))]
		fn activate(&self, sender: &UIControl) {
			let _keep_alive = self.retain();
			let sender = sender.retain();
			if !sender.isEnabled() || !interactive(&sender) { return; }
			let props = self.ivars().props.borrow().clone();
			match self.ivars().kind {
				NativeKind::Switch => {
					if let Some(switch) = sender.downcast_ref::<UISwitch>() {
						self.emit(&NativeEvent::Toggle { checked: switch.isOn() });
						if self.ivars().active.get() { switch.setOn(self.ivars().props.borrow().checked.unwrap_or(false)); }
					}
				}
				NativeKind::Checkbox | NativeKind::Radio => {
					self.emit(&NativeEvent::Toggle { checked: self.ivars().kind == NativeKind::Radio || !props.checked.unwrap_or(false) });
					if self.ivars().active.get() {
						sender.setSelected(self.ivars().props.borrow().checked.unwrap_or(false));
					}
				}
				NativeKind::Slider => {
					if let Some(slider) = sender.downcast_ref::<UISlider>()
						&& let Some(value) = slider_proposal(props.slider.unwrap_or_default(), slider.value()) {
							self.propose_slider(slider, value);
						}
				}
				_ => self.emit(&NativeEvent::Press),
			}
		}

		/// A touch began on the button or slid back onto it: UIKit
		/// highlights it.
		#[unsafe(method(pressIn:))]
		fn press_in(&self, sender: &UIControl) {
			let _keep_alive = self.retain();
			let sender = sender.retain();
			if sender.isEnabled() && interactive(&sender) {
				self.emit(&NativeEvent::PressIn);
			}
		}

		/// The touch ended, was cancelled, or slid away: UIKit removes the
		/// highlight.
		#[unsafe(method(pressOut:))]
		fn press_out(&self, _sender: &UIControl) {
			self.emit(&NativeEvent::PressOut);
		}

		#[unsafe(method(edit:))]
		fn edit(&self, field: &UITextField) {
			let _keep_alive = self.retain();
			let field = field.retain();
			if field.isEnabled() && interactive(&field) {
				self.emit(&NativeEvent::Change { value: field.text().map(|text| text.to_string()).unwrap_or_default() });
			}
			if self.ivars().active.get() {
				let value = {
					let props = self.ivars().props.borrow();
					input_value(&props).to_owned()
				};
				if field.text().map(|text| text.to_string()).unwrap_or_default() != value {
					field.setText(Some(&NSString::from_str(&value)));
				}
			}
		}
	}

	unsafe impl NSObjectProtocol for ActionTarget {}

	unsafe impl UITextFieldDelegate for ActionTarget {
		#[unsafe(method(textFieldDidBeginEditing:))]
		fn begin(&self, field: &UITextField) {
			let _keep_alive = self.retain();
			let field = field.retain();
			if interactive(&field) { self.emit(&NativeEvent::Focus); }
		}

		#[unsafe(method(textFieldDidEndEditing:))]
		fn end(&self, field: &UITextField) {
			let _keep_alive = self.retain();
			let _field = field.retain();
			self.emit(&NativeEvent::Blur);
		}

		#[unsafe(method(textFieldShouldReturn:))]
		fn submit(&self, field: &UITextField) -> bool {
			let _keep_alive = self.retain();
			let field = field.retain();
			if field.isEnabled() && interactive(&field) {
				self.emit(&NativeEvent::Submit { value: field.text().map(|text| text.to_string()).unwrap_or_default() });
			}
			false
		}
	}

	unsafe impl UIScrollViewDelegate for ActionTarget {
		#[unsafe(method(scrollViewDidScroll:))]
		fn scroll(&self, scroll: &UIScrollView) {
			let _keep_alive = self.retain();
			let scroll = scroll.retain();
			let offset = scroll.contentOffset();
			if interactive(&scroll) {
				self.emit(&NativeEvent::Scroll { offset_x: offset.x as f32, offset_y: offset.y as f32 });
			}
		}
	}
);

impl ActionTarget {
	pub(crate) fn new(
		mtm: MainThreadMarker,
		node: NodeId,
		kind: NativeKind,
		sink: Rc<EventSink>,
	) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(TargetState {
			node,
			kind,
			sink,
			active: Cell::new(true),
			props: RefCell::new(NativeProps::default()),
		});
		// SAFETY: NSObject's designated initializer initializes this owned
		// instance.
		unsafe { msg_send![super(this), init] }
	}

	fn emit(&self, event: &NativeEvent) {
		let _keep_alive = self.retain();
		if self.ivars().active.get() {
			self.ivars().sink.emit(self.ivars().node, event);
		}
	}

	pub(crate) fn sync(&self, props: &NativeProps) {
		self.ivars().props.replace(props.clone());
	}

	pub(crate) fn invalidate(&self) {
		self.ivars().active.set(false);
	}

	fn propose_slider(&self, slider: &UISlider, value: f64) {
		let _keep_alive = self.retain();
		let slider = slider.retain();
		if slider.isEnabled() && interactive(&slider) {
			self.emit(&NativeEvent::ValueChange { value });
		}
		if self.ivars().active.get() {
			slider.setValue(slider_coordinate(
				self.ivars().props.borrow().slider.unwrap_or_default(),
			));
		}
	}

	fn step_slider(&self, slider: &UISlider, increment: bool) {
		let state = self.ivars().props.borrow().slider.unwrap_or_default();
		let step = state.step().unwrap_or((state.max() - state.min()) / 100.0);
		let mut proposed = if increment {
			state.value() + step
		} else {
			state.value() - step
		};
		let moved = if increment {
			proposed > state.value()
		} else {
			proposed < state.value()
		};
		if !moved {
			proposed = if increment {
				state.value().next_up()
			} else {
				state.value().next_down()
			};
		}
		proposed = proposed.clamp(state.min(), state.max());
		if let Some(value) = state.proposed_value(proposed) {
			self.propose_slider(slider, value);
		}
	}
}

pub(crate) struct SliderTarget {
	target: RefCell<Option<Weak<ActionTarget>>>,
}

define_class!(
	// SAFETY: UISlider's designated initializer is called below. The target is weak.
	#[unsafe(super(UISlider, UIControl, UIView, objc2_ui_kit::UIResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = SliderTarget]
	pub(crate) struct ControlledSlider;

	impl ControlledSlider {
		#[unsafe(method(accessibilityIncrement))]
		fn increment(&self) {
			let _keep_alive = self.retain();
			self.accessible_step(true);
		}
		#[unsafe(method(accessibilityDecrement))]
		fn decrement(&self) {
			let _keep_alive = self.retain();
			self.accessible_step(false);
		}
	}
	unsafe impl NSObjectProtocol for ControlledSlider {}
);

impl ControlledSlider {
	fn accessible_step(&self, increment: bool) {
		let _keep_alive = self.retain();
		let target = self.ivars().target.borrow().as_ref().and_then(Weak::load);
		if let Some(target) = target {
			target.step_slider(self, increment);
		}
	}

	pub(crate) fn new(mtm: MainThreadMarker, target: &ActionTarget) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(SliderTarget {
			target: RefCell::new(Some(Weak::new(target))),
		});
		// SAFETY: UISlider initializes its inherited frame with an empty
		// rectangle.
		unsafe { msg_send![super(this), initWithFrame: objc2_core_foundation::CGRect::ZERO] }
	}
}

define_class!(
	// SAFETY: UIButton initialization is used, and no UIKit lifecycle is replaced.
	#[unsafe(super(UIButton, UIControl, UIView, objc2_ui_kit::UIResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	pub(crate) struct ContentButton;

	impl ContentButton {
		#[unsafe(method_id(hitTest:withEvent:))]
		fn hit_test(&self, point: CGPoint, event: Option<&UIEvent>) -> Option<Retained<UIView>> { self.content_hit_test(point, event) }

		#[unsafe(method(accessibilityActivate))]
		fn accessibility_activate(&self) -> bool { self.activate_accessibly() }
	}
	unsafe impl NSObjectProtocol for ContentButton {}
);

impl ContentButton {
	fn activate_accessibly(&self) -> bool {
		let _keep_alive = self.retain();
		if !self.isEnabled() || !interactive(self) {
			return false;
		}
		self.sendActionsForControlEvents(UIControlEvents::PrimaryActionTriggered);
		true
	}

	fn content_hit_test(
		&self,
		point: CGPoint,
		event: Option<&UIEvent>,
	) -> Option<Retained<UIView>> {
		if !interactive(self) || !self.isEnabled() || !self.pointInside_withEvent(point, event) {
			return None;
		}
		let radius = self
			.layer()
			.cornerRadius()
			.min(self.bounds().size.width / 2.0)
			.min(self.bounds().size.height / 2.0);
		let size = self.bounds().size;
		let center_x = point.x.clamp(radius, size.width - radius);
		let center_y = point.y.clamp(radius, size.height - radius);
		if (point.x - center_x).powi(2) + (point.y - center_y).powi(2) > radius.powi(2) {
			return None;
		}
		Some(self.retain().into_super().into_super().into_super())
	}
}

impl ContentButton {
	pub(crate) fn new(mtm: MainThreadMarker) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(());
		// SAFETY: UIButton initializes its inherited frame with an empty
		// rectangle.
		unsafe { msg_send![super(this), initWithFrame: objc2_core_foundation::CGRect::ZERO] }
	}
}

pub(crate) struct RadioTarget {
	target: Weak<ActionTarget>,
}

define_class!(
	// SAFETY: UIButton initialization is used. Keyboard commands retain no target
	// and radio registration holds only weak references.
	#[unsafe(super(UIButton, UIControl, UIView, objc2_ui_kit::UIResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = RadioTarget]
	pub(crate) struct RadioButton;
	impl RadioButton {
		#[unsafe(method(canBecomeFirstResponder))]
		fn can_become_first_responder(&self) -> bool { self.isEnabled() && interactive(self) }
		#[unsafe(method_id(keyCommands))]
		fn key_commands(&self) -> Option<Retained<NSArray<objc2_ui_kit::UIKeyCommand>>> { Some(self.commands()) }
		#[unsafe(method(nextRadio:))]
		fn next_radio(&self, _command: &objc2_ui_kit::UIKeyCommand) { self.move_selection(true); }
		#[unsafe(method(previousRadio:))]
		fn previous_radio(&self, _command: &objc2_ui_kit::UIKeyCommand) { self.move_selection(false); }
	}
	unsafe impl NSObjectProtocol for RadioButton {}
);

impl RadioButton {
	pub(crate) fn new(mtm: MainThreadMarker, target: &ActionTarget) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(RadioTarget {
			target: Weak::new(target),
		});
		// SAFETY: UIButton initializes its inherited empty frame.
		unsafe { msg_send![super(this), initWithFrame: objc2_core_foundation::CGRect::ZERO] }
	}

	fn move_selection(&self, forward: bool) {
		let _keep_alive = self.retain();
		let Some(target) = self.ivars().target.load() else {
			return;
		};
		let group = target.ivars().props.borrow().radio_group.clone();
		if let Some(group) = group {
			target
				.ivars()
				.sink
				.move_radio(target.ivars().node, &group, forward);
		}
	}

	fn commands(&self) -> Retained<NSArray<objc2_ui_kit::UIKeyCommand>> {
		use objc2_ui_kit::UIKeyCommand;
		use objc2_ui_kit::UIKeyInputDownArrow;
		use objc2_ui_kit::UIKeyInputLeftArrow;
		use objc2_ui_kit::UIKeyInputRightArrow;
		use objc2_ui_kit::UIKeyInputUpArrow;
		use objc2_ui_kit::UIKeyModifierFlags;
		// SAFETY: UIKit exports immutable key strings. Each selector is an
		// implemented RadioButton command and accepts its UIKeyCommand sender.
		unsafe {
			let commands: Vec<_> = [
				(UIKeyInputDownArrow, objc2::sel!(nextRadio:)),
				(UIKeyInputRightArrow, objc2::sel!(nextRadio:)),
				(UIKeyInputUpArrow, objc2::sel!(previousRadio:)),
				(UIKeyInputLeftArrow, objc2::sel!(previousRadio:)),
			]
			.into_iter()
			.map(|(input, action)| {
				let command = UIKeyCommand::keyCommandWithInput_modifierFlags_action(
					input,
					UIKeyModifierFlags::empty(),
					action,
					self.mtm(),
				);
				command.setWantsPriorityOverSystemBehavior(true);
				command
			})
			.collect();
			NSArray::from_retained_slice(&commands)
		}
	}
}
