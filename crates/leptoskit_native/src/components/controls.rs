//! Controlled native widgets with semantic browser equivalents.

use leptoskit_yoga::Dimension;

use crate::Element;
use crate::NativeKind;
pub use crate::controls::ControlTreeError;
pub use crate::controls::ControlValueError;
pub use crate::controls::Progress;
pub use crate::controls::ProgressValue;
pub use crate::controls::RadioGroup;
pub use crate::controls::SliderState;

fn sized(kind: NativeKind, width: f32, height: f32) -> Element {
	Element::new(kind).with_style(|style| {
		style.width = Dimension::Points(width);
		style.height = Dimension::Points(height);
	})
}

/// A controlled on/off switch; `on_toggle` proposes a new value.
pub fn Switch(checked: bool) -> Element {
	sized(NativeKind::Switch, 40.0, 24.0).with_props(|props| props.checked = Some(checked))
}

/// A controlled boolean checkbox; mixed state is not currently supported.
pub fn Checkbox(checked: bool) -> Element {
	sized(NativeKind::Checkbox, 20.0, 20.0).with_props(|props| props.checked = Some(checked))
}

/// A controlled radio option. Related options share an explicit group and
/// application-owned selection; label each option with `accessibility_label`.
/// Arrow keys move between enabled options in the group and propose selection.
pub fn Radio(group: RadioGroup, checked: bool) -> Element {
	sized(NativeKind::Radio, 20.0, 20.0).with_props(|props| {
		props.radio_group = Some(group);
		props.checked = Some(checked);
	})
}

/// A controlled horizontal slider over a validated finite step grid.
pub fn Slider(state: SliderState) -> Element {
	sized(NativeKind::Slider, 180.0, 24.0).with_props(|props| props.slider = Some(state))
}

/// A horizontal progress bar, including genuine indeterminate animation.
pub fn LinearProgressIndicator(progress: Progress) -> Element {
	sized(NativeKind::LinearProgressIndicator, 180.0, 8.0)
		.with_props(|props| props.progress = Some(progress))
}

/// An animated platform loading spinner. Stopped indicators hide while keeping
/// their layout space. Hidden and stopped indicators do not animate; disabled
/// interaction does not interrupt loading. Use a linear bar for known
/// fractions.
pub fn ActivityIndicator(running: bool) -> Element {
	sized(NativeKind::ActivityIndicator, 20.0, 20.0)
		.with_props(|props| props.running = Some(running))
}

/// An editable single-line password field backed by a secure native text field.
pub fn PasswordInput() -> Element {
	sized(NativeKind::PasswordInput, 180.0, 28.0)
}

/// An accessible button containing arbitrary decorative component children.
///
/// `label` names the action for assistive technology; visible content comes
/// from children. The label must contain a non-whitespace character. Children
/// cannot contain interactive widgets, event handlers, or scrolling content.
/// Register activation with `on_press`; press phases and long presses are not
/// part of this control's contract.
pub fn Pressable(label: impl Into<String>) -> Element {
	Element::new(NativeKind::Pressable).accessibility_label(label)
}
