//! The Objective-C classes the backend defines: a flipped container view and
//! the target that turns control actions into leptoskit events.

use std::cell::RefCell;
use std::rc::Rc;

use leptoskit_native::NativeEvent;
use leptoskit_yoga::NodeId;
use objc2::DefinedClass;
use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::define_class;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::runtime::NSObject;
use objc2::runtime::NSObjectProtocol;
use objc2_app_kit::NSControlTextEditingDelegate;
use objc2_app_kit::NSResponder;
use objc2_app_kit::NSTextField;
use objc2_app_kit::NSTextFieldDelegate;
use objc2_app_kit::NSView;
use objc2_foundation::NSNotification;
use objc2_foundation::NSRect;

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

/// Receives the events native widgets emit, as `(node, event)` pairs.
pub type EventListener = Rc<dyn Fn(NodeId, &NativeEvent)>;

/// Where widget actions deliver their events: the listener the app
/// installed, if any.
#[derive(Default)]
pub(crate) struct EventSink {
	listener: RefCell<Option<EventListener>>,
}

impl EventSink {
	pub(crate) fn set_listener(&self, listener: Option<EventListener>) {
		*self.listener.borrow_mut() = listener;
	}

	/// Delivers an event to the listener.
	///
	/// The listener is cloned out before it runs, so it may replace itself or
	/// rebuild the whole interface — deleting the very widget that fired —
	/// without a `RefCell` conflict.
	fn emit(&self, node: NodeId, event: &NativeEvent) {
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
		/// A button was clicked.
		#[unsafe(method(press:))]
		fn press(&self, _sender: Option<&AnyObject>) {
			self.emit(&NativeEvent::Press);
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
			let value = notification
				.object()
				.and_then(|object| object.downcast::<NSTextField>().ok())
				.map(|field| field.stringValue().to_string())
				.unwrap_or_default();
			self.emit(&NativeEvent::Change { value });
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
		let this = Self::alloc(mtm).set_ivars(ActionTargetIvars { node, sink });
		// SAFETY: `init` is `NSObject`'s designated initializer.
		unsafe { msg_send![super(this), init] }
	}

	fn emit(&self, event: &NativeEvent) {
		let ivars = self.ivars();
		ivars.sink.emit(ivars.node, event);
	}
}
