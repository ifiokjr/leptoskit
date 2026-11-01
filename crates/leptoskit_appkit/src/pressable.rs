//! Native button tracking for arbitrary decorative component content.

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
use objc2_app_kit::NSAccessibility;
use objc2_app_kit::NSAccessibilityButtonRole;
use objc2_app_kit::NSButton;
use objc2_app_kit::NSControl;
use objc2_app_kit::NSEvent;
use objc2_app_kit::NSEventModifierFlags;
use objc2_app_kit::NSResponder;
use objc2_app_kit::NSView;
use objc2_foundation::NSArray;
use objc2_foundation::NSPoint;
use objc2_foundation::NSRect;

use crate::views::ActionTarget;
use crate::views::track_press;

define_class!(
	// SAFETY: the class uses NSButton's designated initializer, keeps its
	// native tracking behavior, and maps accessible/key activation through
	// the same standard action as pointer tracking.
	#[unsafe(super(NSButton, NSControl, NSView, NSResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = Weak<ActionTarget>]
	struct ContentButton;

	impl ContentButton {
		#[unsafe(method(isFlipped))]
		fn is_flipped(&self) -> bool {
			true
		}

		#[unsafe(method(mouseDown:))]
		fn mouse_down(&self, event: &NSEvent) {
			track_press(self, self.ivars().load(), || {
				// SAFETY: NSButton implements mouseDown: with this exact
				// signature.
				unsafe { let _: () = msg_send![super(self), mouseDown: event]; }
			});
		}

		#[unsafe(method(keyDown:))]
		fn key_down(&self, event: &NSEvent) {
			let _keep_alive = self.retain();
			let character = event.charactersIgnoringModifiers().map(|value| value.to_string());

			let command = event.modifierFlags().intersects(
				NSEventModifierFlags::Command | NSEventModifierFlags::Control | NSEventModifierFlags::Option,
			);

			if !command && matches!(character.as_deref(), Some(" " | "\r" | "\u{3}")) {
				if !event.isARepeat() {
					self.activate();
				}
			} else {
				// SAFETY: NSButton implements keyDown: with this signature.
				unsafe { let _: () = msg_send![super(self), keyDown: event]; }
			}
		}

		#[unsafe(method(accessibilityPerformPress))]
		fn accessibility_perform_press(&self) -> bool {
			self.activate()
		}

		#[unsafe(method_id(hitTest:))]
		fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
			self.content_hit_test(point)
		}
	}

	unsafe impl NSObjectProtocol for ContentButton {}
);

impl ContentButton {
	fn activate(&self) -> bool {
		let _keep_alive = self.retain();
		let target = self.target();

		if !self.isEnabled()
			|| self.isHiddenOrHasHiddenAncestor()
			|| target.is_none()
			|| self.action().is_none()
		{
			return false;
		}

		// SAFETY: performClick: is NSButton's native tracking/action entry
		// point and accepts an absent sender. Keep the control and its weak
		// action target alive across a handler that may replace its whole tree.
		unsafe { self.performClick(None) };
		true
	}

	fn content_hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
		// SAFETY: this forwards the unchanged point to NSButton's hitTest:
		// implementation and observes its declared retained return type.
		let hit: Option<Retained<NSView>> = unsafe { msg_send![super(self), hitTest: point] };
		hit?;
		// SAFETY: hit testing runs synchronously on the main thread while
		// the backend/window owns the attached parent. Detached buttons have
		// no superview; no callback runs before this retained reference drops.
		let parent = unsafe { self.superview() };
		let local = self.convertPoint_fromView(point, parent.as_deref());
		let bounds = self.bounds();
		let radius = self
			.layer()
			.map_or(0.0, |layer| layer.cornerRadius())
			.max(0.0)
			.min(bounds.size.width.min(bounds.size.height) * 0.5);
		let right = bounds.origin.x + bounds.size.width;
		let bottom = bounds.origin.y + bounds.size.height;

		let dx = local.x - local.x.clamp(bounds.origin.x + radius, right - radius);
		let dy = local.y - local.y.clamp(bounds.origin.y + radius, bottom - radius);

		if dx * dx + dy * dy > radius * radius {
			return None;
		}

		// Descendants are decorative: shared-tree validation rejects
		// interactive children before they reach this native container.
		let retained: Retained<Self> = self.retain();
		let button = Retained::into_super(retained);
		Some(Retained::into_super(Retained::into_super(button)))
	}
}

/// Creates an unbordered `NSButton` whose children use Yoga's top-left origin
/// and whose holds report press phases to `target`.
pub(crate) fn content_button(mtm: MainThreadMarker, target: &ActionTarget) -> Retained<NSButton> {
	let allocated = ContentButton::alloc(mtm).set_ivars(Weak::new(target));
	// SAFETY: NSButton inherits NSView's designated initWithFrame: initializer.
	let button: Retained<ContentButton> =
		unsafe { msg_send![super(allocated), initWithFrame: NSRect::ZERO] };
	button.setBordered(false);
	button.setWantsLayer(true);
	button.setAccessibilityElement(true);
	// SAFETY: the empty accessibility array contains no objects with a
	// stronger protocol contract. The button's explicit label names its
	// complete action; decorative descendants are not separate controls.
	unsafe {
		button.setAccessibilityRole(Some(NSAccessibilityButtonRole));
		button.setAccessibilityChildren(Some(&NSArray::new()));
	}
	Retained::into_super(button)
}
