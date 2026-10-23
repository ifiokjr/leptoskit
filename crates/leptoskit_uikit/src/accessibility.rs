//! Maps shared ARIA properties onto `UIAccessibility`.
//!
//! Native controls report their own traits and values; this module carries
//! what apps declare: hints from descriptions, values from human-readable
//! range text, traits for declared roles on plain views, modal isolation,
//! and hidden semantics. Live regions announce their resolved text through
//! the announcement notification when it changes.

use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::Role;
use leptoskit_native::Tristate;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::NSString;
use objc2_ui_kit::NSObjectUIAccessibility;
use objc2_ui_kit::UIAccessibilityAnnouncementNotification;
use objc2_ui_kit::UIAccessibilityPostNotification;
use objc2_ui_kit::UIAccessibilityTraitAdjustable;
use objc2_ui_kit::UIAccessibilityTraitButton;
use objc2_ui_kit::UIAccessibilityTraitHeader;
use objc2_ui_kit::UIAccessibilityTraitImage;
use objc2_ui_kit::UIAccessibilityTraitLink;
use objc2_ui_kit::UIAccessibilityTraitSearchField;
use objc2_ui_kit::UIAccessibilityTraitSelected;
use objc2_ui_kit::UIAccessibilityTraitStaticText;
use objc2_ui_kit::UIAccessibilityTraitUpdatesFrequently;
use objc2_ui_kit::UIAccessibilityTraits;
use objc2_ui_kit::UIView;

/// The traits a declared role adds to a plain view.
pub(crate) fn traits(role: Role) -> UIAccessibilityTraits {
	// SAFETY: UIKit exports these immutable trait bit flags.
	unsafe {
		match role {
			Role::Button
			| Role::MenuItem
			| Role::MenuItemCheckbox
			| Role::MenuItemRadio
			| Role::Option
			| Role::Tab
			| Role::TreeItem => UIAccessibilityTraitButton,
			Role::Link => UIAccessibilityTraitLink,
			Role::Heading => UIAccessibilityTraitHeader,
			Role::Img => UIAccessibilityTraitImage,
			Role::Search | Role::SearchBox => UIAccessibilityTraitSearchField,
			Role::Slider | Role::SpinButton | Role::ScrollBar | Role::Meter => {
				UIAccessibilityTraitAdjustable
			}
			Role::Alert | Role::Log | Role::Marquee | Role::Status | Role::Timer | Role::Feed => {
				UIAccessibilityTraitUpdatesFrequently
			}
			Role::Article
			| Role::Code
			| Role::Definition
			| Role::Emphasis
			| Role::Note
			| Role::Paragraph
			| Role::Strong
			| Role::Term
			| Role::Time
			| Role::Tooltip => UIAccessibilityTraitStaticText,
			_ => 0,
		}
	}
}

/// Applies the accessibility properties of one widget beyond its label.
pub(crate) fn apply(view: &UIView, props: &NativeProps, kind: NativeKind, mtm: MainThreadMarker) {
	let aria = props.aria();

	let hint = aria.description.as_deref().map(NSString::from_str);
	view.setAccessibilityHint(hint.as_deref(), mtm);

	let value = aria
		.value_text
		.as_deref()
		.map(NSString::from_str)
		.or_else(|| {
			let custom = matches!(
				aria.role,
				Some(Role::Checkbox | Role::MenuItemCheckbox | Role::Switch)
			);
			custom
				.then_some(aria.checked)
				.flatten()
				.map(|checked| {
					match checked {
						Tristate::True => "1",
						Tristate::False => "0",
						Tristate::Mixed => "mixed",
					}
				})
				.map(NSString::from_str)
		});
	view.setAccessibilityValue(value.as_deref(), mtm);

	view.setAccessibilityElementsHidden(aria.is_hidden(), mtm);
	if aria.modal == Some(true) {
		view.setAccessibilityViewIsModal(true, mtm);
	}

	// Controls report their own traits; declared roles restyle plain views.
	if matches!(
		kind,
		NativeKind::View | NativeKind::ScrollView | NativeKind::Text | NativeKind::TextRun
	) && let Some(role) = aria.role
	{
		let mut role_traits = traits(role);
		if aria.selected == Some(true) {
			// SAFETY: as above.
			role_traits |= unsafe { UIAccessibilityTraitSelected };
		}
		view.setAccessibilityTraits(role_traits, mtm);
	}
}

/// Whether a plain view becomes a single accessibility element, like React
/// Native's `accessible`.
pub(crate) fn is_element(props: &NativeProps) -> bool {
	props.aria().role.is_some()
		|| props.accessibility_label.is_some()
		|| props.aria().description.is_some()
		|| props.aria().live.is_some()
}

/// Announces live-region text to assistive technology.
pub(crate) fn announce(text: &str) {
	let argument: Retained<AnyObject> = NSString::from_str(text).into();
	// SAFETY: the announcement notification takes a string argument, and
	// UIKit exports this immutable notification constant.
	unsafe {
		UIAccessibilityPostNotification(UIAccessibilityAnnouncementNotification, Some(&argument));
	}
}
