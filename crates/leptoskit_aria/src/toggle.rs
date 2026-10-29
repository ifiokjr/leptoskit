//! A button that carries on/off state.

use leptoskit_native::Element;
use leptoskit_native::Tristate;

/// A pressed-state button: `aria-pressed` on a plain button.
///
/// The proposed state follows the WAI-ARIA checkbox convention for mixed
/// state: pressing an unpressed or mixed button proposes `true`, and pressing
/// a pressed one proposes `false`. `on_toggle` receives the proposal; keeping
/// state is the caller's, like every controlled control here.
///
/// ```
/// use leptoskit_aria::ToggleButton;
/// use leptoskit_native::Tristate;
///
/// let muted = ToggleButton("Mute", Tristate::Mixed, |_| ());
/// assert_eq!(muted.props.aria().pressed, Some(Tristate::Mixed));
/// ```
pub fn ToggleButton(
	label: impl Into<String>,
	pressed: impl Into<Tristate>,
	on_toggle: impl Fn(bool) + Send + Sync + 'static,
) -> Element {
	let pressed = pressed.into();
	Element::button(label)
		.aria_pressed(pressed)
		.on_press(move || on_toggle(!matches!(pressed, Tristate::True)))
}
