//! Navigation models that fit round and small screens.

/// How a watch app moves between its pages.
///
/// These models match crown, bezel, and swipe gestures. They are not the
/// phone file router: a watch app declares a short list of pages, not a
/// deep route tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WatchNavigation {
	/// One page only. The default for a glance-first companion.
	#[default]
	Single,
	/// Vertical stack driven by the digital crown or vertical swipe.
	Stack,
	/// Horizontal pager with page indicators.
	Pager,
	/// Short list that opens a detail page.
	ListDetail,
}
