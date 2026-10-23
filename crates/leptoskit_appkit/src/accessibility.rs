//! Maps shared ARIA properties onto `NSAccessibility`.
//!
//! Native controls already describe themselves to `VoiceOver`; this module
//! carries what apps declare: names and descriptions, explicit roles, the
//! states AppKit models (selected, expanded, required), range values, and
//! live-region announcements. Plain containers become one accessibility
//! element when named, like React Native's `accessible`, and `aria-hidden`
//! removes an element from the accessibility tree while it stays visible.

use leptoskit_native::Live;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::Role;
use leptoskit_native::Tristate;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSAccessibility;
use objc2_app_kit::NSAccessibilityAnnouncementKey;
use objc2_app_kit::NSAccessibilityAnnouncementRequestedNotification;
use objc2_app_kit::NSAccessibilityButtonRole;
use objc2_app_kit::NSAccessibilityCellRole;
use objc2_app_kit::NSAccessibilityCheckBoxRole;
use objc2_app_kit::NSAccessibilityComboBoxRole;
use objc2_app_kit::NSAccessibilityGridRole;
use objc2_app_kit::NSAccessibilityGroupRole;
use objc2_app_kit::NSAccessibilityHeadingRole;
use objc2_app_kit::NSAccessibilityHelpTagRole;
use objc2_app_kit::NSAccessibilityImageRole;
use objc2_app_kit::NSAccessibilityIncrementorRole;
use objc2_app_kit::NSAccessibilityLevelIndicatorRole;
use objc2_app_kit::NSAccessibilityLinkRole;
use objc2_app_kit::NSAccessibilityListRole;
use objc2_app_kit::NSAccessibilityMenuBarRole;
use objc2_app_kit::NSAccessibilityMenuItemRole;
use objc2_app_kit::NSAccessibilityMenuRole;
use objc2_app_kit::NSAccessibilityOrientation;
use objc2_app_kit::NSAccessibilityOutlineRole;
use objc2_app_kit::NSAccessibilityPostNotificationWithUserInfo;
use objc2_app_kit::NSAccessibilityPriorityKey;
use objc2_app_kit::NSAccessibilityPriorityLevel;
use objc2_app_kit::NSAccessibilityProgressIndicatorRole;
use objc2_app_kit::NSAccessibilityRadioButtonRole;
use objc2_app_kit::NSAccessibilityRadioGroupRole;
use objc2_app_kit::NSAccessibilityRole;
use objc2_app_kit::NSAccessibilityRowRole;
use objc2_app_kit::NSAccessibilityScrollBarRole;
use objc2_app_kit::NSAccessibilitySliderRole;
use objc2_app_kit::NSAccessibilitySplitterRole;
use objc2_app_kit::NSAccessibilityTabGroupRole;
use objc2_app_kit::NSAccessibilityTableRole;
use objc2_app_kit::NSAccessibilityTextFieldRole;
use objc2_app_kit::NSAccessibilityToolbarRole;
use objc2_app_kit::NSView;
use objc2_foundation::NSArray;
use objc2_foundation::NSDictionary;
use objc2_foundation::NSString;

/// The AppKit role an ARIA role maps onto, if a close one exists; otherwise
/// the ARIA name becomes the role description.
fn ns_role(role: Role) -> Option<&'static NSAccessibilityRole> {
	// SAFETY: AppKit exports these immutable role-name statics.
	unsafe {
		match role {
			Role::Button => Some(NSAccessibilityButtonRole),
			Role::Checkbox | Role::MenuItemCheckbox => Some(NSAccessibilityCheckBoxRole),
			Role::MenuItemRadio | Role::Radio => Some(NSAccessibilityRadioButtonRole),
			Role::RadioGroup => Some(NSAccessibilityRadioGroupRole),
			Role::Slider => Some(NSAccessibilitySliderRole),
			Role::SpinButton => Some(NSAccessibilityIncrementorRole),
			Role::TextBox | Role::SearchBox => Some(NSAccessibilityTextFieldRole),
			Role::ComboBox => Some(NSAccessibilityComboBoxRole),
			Role::ScrollBar => Some(NSAccessibilityScrollBarRole),
			Role::ProgressBar => Some(NSAccessibilityProgressIndicatorRole),
			Role::Meter => Some(NSAccessibilityLevelIndicatorRole),
			Role::Img => Some(NSAccessibilityImageRole),
			Role::Link => Some(NSAccessibilityLinkRole),
			Role::Heading => Some(NSAccessibilityHeadingRole),
			Role::List => Some(NSAccessibilityListRole),
			Role::Menu => Some(NSAccessibilityMenuRole),
			Role::MenuBar => Some(NSAccessibilityMenuBarRole),
			Role::MenuItem => Some(NSAccessibilityMenuItemRole),
			Role::Toolbar => Some(NSAccessibilityToolbarRole),
			Role::Table => Some(NSAccessibilityTableRole),
			Role::Row => Some(NSAccessibilityRowRole),
			Role::Cell | Role::ColumnHeader | Role::GridCell | Role::RowHeader => {
				Some(NSAccessibilityCellRole)
			}
			Role::Grid | Role::TreeGrid => Some(NSAccessibilityGridRole),
			Role::Tree => Some(NSAccessibilityOutlineRole),
			Role::Group => Some(NSAccessibilityGroupRole),
			Role::Separator => Some(NSAccessibilitySplitterRole),
			Role::Tooltip => Some(NSAccessibilityHelpTagRole),
			Role::TabList | Role::Tab => Some(NSAccessibilityTabGroupRole),
			_ => None,
		}
	}
}

/// Whether the element's content is one announced unit, so children leave
/// the accessibility tree.
fn absorbs_children(role: Option<Role>) -> bool {
	match role {
		None
		| Some(
			Role::Alert
			| Role::AlertDialog
			| Role::Figure
			| Role::Heading
			| Role::Img
			| Role::Log
			| Role::Marquee
			| Role::Note
			| Role::Paragraph
			| Role::Status
			| Role::Term
			| Role::Timer
			| Role::Tooltip,
		) => true,
		Some(role) => role.is_interactive(),
	}
}

/// Applies the accessibility properties of one widget.
pub(super) fn apply(view: &NSView, props: &NativeProps, kind: NativeKind) {
	let aria = props.aria();
	let label = props
		.accessibility_label
		.as_deref()
		.map(NSString::from_str)
		.or_else(|| (kind == NativeKind::ActivityIndicator).then(|| NSString::from_str("Loading")));
	view.setAccessibilityLabel(label.as_deref());

	if aria.is_hidden() {
		view.setAccessibilityElement(false);
		return;
	}

	let help = aria.description.as_deref().map(NSString::from_str);
	view.setAccessibilityHelp(help.as_deref());

	if let Some(role) = aria.role {
		if let Some(ns_role) = ns_role(role) {
			view.setAccessibilityRole(Some(ns_role));
		} else {
			// A bare custom view reports itself as unknown; unmapped roles
			// read best as a named group.
			let description = aria.role_description.as_deref().unwrap_or(role.wire_name());
			// SAFETY: AppKit exports this immutable role-name static.
			view.setAccessibilityRole(Some(unsafe { NSAccessibilityGroupRole }));
			view.setAccessibilityRoleDescription(Some(&NSString::from_str(description)));
		}
	} else if let Some(description) = aria.role_description.as_deref() {
		view.setAccessibilityRoleDescription(Some(&NSString::from_str(description)));
	}

	let value = aria
		.value_text
		.as_deref()
		.map(NSString::from_str)
		.or_else(|| checked_value(aria.role, aria.checked).map(NSString::from_str));
	if let Some(value) = value.as_deref() {
		// SAFETY: a string is a valid accessibility value object for any
		// role that accepts one.
		unsafe {
			let _: () = msg_send![view, setAccessibilityValue: value];
		}
	}

	view.setAccessibilitySelected(aria.selected == Some(true));
	view.setAccessibilityExpanded(aria.expanded == Some(true));
	view.setAccessibilityRequired(aria.required == Some(true));
	if let Some(orientation) = aria.orientation {
		view.setAccessibilityOrientation(match orientation {
			leptoskit_native::Orientation::Horizontal => NSAccessibilityOrientation::Horizontal,
			leptoskit_native::Orientation::Vertical => NSAccessibilityOrientation::Vertical,
		});
	}

	// Controls describe themselves; plain containers and text become
	// accessibility elements exactly when they carry semantics.
	if matches!(
		kind,
		NativeKind::View | NativeKind::ScrollView | NativeKind::Text | NativeKind::TextRun
	) {
		view.setAccessibilityEnabled(props.enabled != Some(false));
		let element = aria.role.is_some()
			|| label.is_some()
			|| help.is_some()
			|| aria.live.is_some()
			|| aria.live_text.is_some();
		view.setAccessibilityElement(element);
		if element && absorbs_children(aria.role) {
			let children = NSArray::new();
			// SAFETY: an empty array is always a valid children list.
			unsafe { view.setAccessibilityChildren(Some(&children)) };
		}
	}
}

/// The announced value of a custom checkbox-style control.
fn checked_value(role: Option<Role>, checked: Option<Tristate>) -> Option<&'static str> {
	let custom = matches!(
		role,
		Some(Role::Checkbox | Role::MenuItemCheckbox | Role::Switch)
	);

	if !custom {
		return None;
	}

	checked.map(|state| {
		match state {
			Tristate::True => "checked",
			Tristate::False => "unchecked",
			Tristate::Mixed => "mixed",
		}
	})
}

/// Announces live-region text as a screen-reader notification.
///
/// Polite announcements use AppKit's low priority so they wait for current
/// speech; assertive ones use high priority and interrupt it.
pub(super) fn announce(view: &NSView, text: &str, live: Live) {
	let priority = match live {
		Live::Assertive => NSAccessibilityPriorityLevel::High,
		_ => NSAccessibilityPriorityLevel::Low,
	};
	let announcement: Retained<AnyObject> = NSString::from_str(text).into();
	let priority: Retained<AnyObject> = objc2_foundation::NSNumber::new_isize(priority.0).into();
	let info;
	// SAFETY: AppKit exports these immutable user-info keys, and the
	// dictionary holds exactly the announcement string and its priority.
	unsafe {
		let keys = [NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey];
		let values = [&*announcement, &*priority];
		info = NSDictionary::from_slices(&keys, &values);
		NSAccessibilityPostNotificationWithUserInfo(
			view,
			NSAccessibilityAnnouncementRequestedNotification,
			Some(&info),
		);
	}
}
