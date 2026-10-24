//! Tabs: a list of tabs and the panels they select.

use leptoskit_native::Element;
use leptoskit_native::Role;
use leptoskit_yoga::Display;

fn tab_id(list_id: &str, index: usize) -> String {
	format!("{list_id}-tab-{index}")
}

fn panel_id(list_id: &str, index: usize) -> String {
	format!("{list_id}-panel-{index}")
}

/// The group of tabs: a `tablist` container for [`Tab`](fn@Tab) children.
///
/// Lay tabs out with Tailwind classes; the list contributes the role only.
/// The web renderer activates tabs from Enter and Space; arrow-key motion
/// between tabs arrives with the renderer's focus work.
///
/// ```
/// use leptoskit_aria::Tab;
/// use leptoskit_aria::TabList;
///
/// let list = TabList().child(Tab("settings", 0, 2, true, "General", |_| ()));
/// assert_eq!(
/// 	list.props.aria().effective_role(list.kind),
/// 	Some(leptoskit_native::Role::TabList)
/// );
/// ```
pub fn TabList() -> Element {
	Element::view().role(Role::TabList)
}

/// One tab of a [`TabList`](fn@TabList).
///
/// The selected tab joins sequential focus with `tabindex=0` and the rest sit
/// at `-1` — the roving tabindex — while `aria-selected`, `aria-controls`, and
/// the position in the set round out the wiring. `on_select` receives this
/// tab's index.
///
/// ```
/// use leptoskit_aria::Tab;
///
/// let tab = Tab("settings", 1, 3, false, "Notifications", |_| ());
/// assert_eq!(tab.props.id.as_deref(), Some("settings-tab-1"));
/// assert_eq!(tab.props.aria().selected, Some(false));
/// assert_eq!(tab.props.aria().tab_index, Some(-1));
/// assert_eq!(tab.props.aria().controls, ["settings-panel-1"]);
/// assert_eq!(tab.props.aria().pos_in_set, Some(2));
/// assert_eq!(tab.props.aria().set_size, Some(3));
/// ```
pub fn Tab(
	list_id: &str,
	index: usize,
	set_size: usize,
	selected: bool,
	label: impl Into<String>,
	on_select: impl Fn(usize) + Send + Sync + 'static,
) -> Element {
	Element::button(label)
		.id(tab_id(list_id, index))
		.role(Role::Tab)
		.aria_selected(selected)
		.aria_controls([panel_id(list_id, index)])
		.aria_pos_in_set(index as u32 + 1)
		.aria_set_size(set_size as u32)
		.tab_index(i32::from(selected) - 1)
		.on_press(move || on_select(index))
}

/// The content one [`Tab`](fn@Tab) selects.
///
/// Panels stay mounted with the inactive ones collapsed, so every tab's
/// `aria-controls` resolves and switching panels keeps state. The panel is
/// named after its tab.
///
/// ```
/// use leptoskit_aria::TabPanel;
/// use leptoskit_native::components::Text;
///
/// let panel = TabPanel("settings", 1, false).child(Text("Notification choices"));
/// assert_eq!(panel.props.id.as_deref(), Some("settings-panel-1"));
/// assert_eq!(panel.props.aria().labelled_by, ["settings-tab-1"]);
/// ```
pub fn TabPanel(list_id: &str, index: usize, active: bool) -> Element {
	let mut panel = Element::view()
		.id(panel_id(list_id, index))
		.role(Role::TabPanel)
		.aria_labelled_by([tab_id(list_id, index)]);

	if !active {
		panel = panel.with_style(|style| style.display = Display::None);
	}

	panel
}
