//! Disclosure: a control that expands and collapses a region.

use leptoskit_native::Element;
use leptoskit_native::Role;
use leptoskit_yoga::Display;

/// The id of a [`DisclosurePanel`](fn@DisclosurePanel) for a disclosure id.
fn panel_id(id: &str) -> String {
	format!("{id}-panel")
}

/// The id of a [`DisclosureButton`](fn@DisclosureButton) for a disclosure id.
fn button_id(id: &str) -> String {
	format!("{id}-button")
}

/// The control that expands and collapses one disclosure region.
///
/// The button names the action, carries `aria-expanded`, and references its
/// panel with `aria-controls`; `on_toggle` receives the state the press
/// proposes. Style it with Tailwind classes; activation comes from the
/// platform, including Enter and Space on the web.
///
/// ```
/// use leptoskit_aria::DisclosureButton;
///
/// let button = DisclosureButton("faq", "Frequently asked questions", false, |_| ());
/// assert_eq!(button.props.id.as_deref(), Some("faq-button"));
/// assert_eq!(button.props.aria().expanded, Some(false));
/// assert_eq!(button.props.aria().controls, ["faq-panel"]);
/// ```
#[track_caller]
pub fn DisclosureButton(
	id: &str,
	label: impl Into<String>,
	open: bool,
	on_toggle: impl Fn(bool) + Send + Sync + 'static,
) -> Element {
	Element::button(label)
		.id(button_id(id))
		.aria_expanded(open)
		.aria_controls([panel_id(id)])
		.on_press(move || on_toggle(!open))
}

/// The region a [`DisclosureButton`](fn@DisclosureButton) shows and hides.
///
/// The panel is a named region — labelled by its button — that stays mounted
/// while collapsed, so the button's `aria-controls` always resolves and the
/// collapsed content is unreachable by assistive technology instead of absent.
/// Give it Tailwind classes for spacing; content comes from `child` calls.
///
/// ```
/// use leptoskit_aria::DisclosurePanel;
/// use leptoskit_native::components::Text;
///
/// let panel = DisclosurePanel("faq", false).child(Text("Only visible once expanded"));
/// assert_eq!(panel.props.id.as_deref(), Some("faq-panel"));
/// assert_eq!(panel.props.aria().labelled_by, ["faq-button"]);
/// assert!(panel.children.len() == 1);
/// ```
#[track_caller]
pub fn DisclosurePanel(id: &str, open: bool) -> Element {
	let mut panel = Element::view()
		.id(panel_id(id))
		.role(Role::Region)
		.aria_labelled_by([button_id(id)]);

	if !open {
		panel = panel.with_style(|style| style.display = Display::None);
	}

	panel
}
