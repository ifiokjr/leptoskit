//! Tabs: a list of tabs and the panels they select.

use leptoskit_native::Element;
use leptoskit_native::Role;
use leptoskit_yoga::Display;

/// One tab set: how many tabs it has and which one is selected.
///
/// Every tab and panel comes from the same `Tabs`, so the wiring cannot
/// disagree: exactly one tab is selected, every tab reports the same set
/// size, and each tab's `aria-controls` names the panel built for the same
/// index. Build a fresh `Tabs` on every render from current state, the way
/// [`Field`](crate::Field) is built.
///
/// Every tab stays in sequential focus order. The WAI-ARIA roving tabindex
/// leaves only the selected tab focusable and moves between tabs with the
/// arrow keys; the renderer has no key events yet, so roving would strand
/// every unselected tab out of keyboard reach. Enter and Space activate the
/// focused tab.
///
/// ```
/// use leptoskit_aria::Tabs;
/// use leptoskit_native::components::Text;
/// use leptoskit_native::components::View;
///
/// let tabs = Tabs::new("settings", 2, 0);
/// let screen = View()
/// 	.child(
/// 		tabs.list()
/// 			.child(tabs.tab(0, "General", |_| ()))
/// 			.child(tabs.tab(1, "Notifications", |_| ())),
/// 	)
/// 	.child(tabs.panel(0).child(Text("General settings")))
/// 	.child(tabs.panel(1).child(Text("Notification choices")));
///
/// let general = &screen.children[0].children[0];
/// assert_eq!(general.props.id.as_deref(), Some("settings-tab-0"));
/// assert_eq!(general.props.aria().selected, Some(true));
/// assert_eq!(general.props.aria().controls, ["settings-panel-0"]);
/// assert_eq!(general.props.aria().set_size, Some(2));
/// assert!(screen.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tabs {
	id: String,
	count: usize,
	selected: usize,
}

impl Tabs {
	/// A tab set of `count` tabs under the id prefix `id`, with `selected`
	/// active.
	///
	/// # Panics
	///
	/// When `selected` is not below `count`: a tab set always has exactly
	/// one selected tab.
	pub fn new(id: impl Into<String>, count: usize, selected: usize) -> Self {
		assert!(
			selected < count,
			"selected tab {selected} is outside a set of {count}"
		);
		Self {
			id: id.into(),
			count,
			selected,
		}
	}

	/// The index of the selected tab.
	#[must_use]
	pub fn selected(&self) -> usize {
		self.selected
	}

	/// The `tablist` container for this set's tabs. Lay the tabs out with
	/// Tailwind classes; the list contributes the role only.
	#[must_use]
	pub fn list(&self) -> Element {
		Element::view().role(Role::TabList)
	}

	/// The tab at `index`, labelled `label`. `on_select` receives the index
	/// when the tab is pressed.
	///
	/// # Panics
	///
	/// When `index` is not below the set's count.
	#[must_use]
	pub fn tab(
		&self,
		index: usize,
		label: impl Into<String>,
		on_select: impl Fn(usize) + Send + Sync + 'static,
	) -> Element {
		self.check(index);
		Element::button(label)
			.id(self.tab_id(index))
			.role(Role::Tab)
			.aria_selected(index == self.selected)
			.aria_controls([self.panel_id(index)])
			.aria_pos_in_set(position(index))
			.aria_set_size(position(self.count - 1))
			.on_press(move || on_select(index))
	}

	/// The panel the tab at `index` selects, named after that tab.
	///
	/// Panels stay mounted with the inactive ones collapsed, so every tab's
	/// `aria-controls` resolves and switching tabs keeps panel state.
	///
	/// # Panics
	///
	/// When `index` is not below the set's count.
	#[must_use]
	pub fn panel(&self, index: usize) -> Element {
		self.check(index);
		let panel = Element::view()
			.id(self.panel_id(index))
			.role(Role::TabPanel)
			.aria_labelled_by([self.tab_id(index)]);

		if index == self.selected {
			panel
		} else {
			panel.with_style(|style| style.display = Display::None)
		}
	}

	fn check(&self, index: usize) {
		assert!(
			index < self.count,
			"tab {index} is outside a set of {}",
			self.count
		);
	}

	fn tab_id(&self, index: usize) -> String {
		format!("{}-tab-{index}", self.id)
	}

	fn panel_id(&self, index: usize) -> String {
		format!("{}-panel-{index}", self.id)
	}
}

/// The one-based `aria-posinset` of a zero-based index.
fn position(index: usize) -> u32 {
	u32::try_from(index + 1).expect("a tab set fits in u32 positions")
}
