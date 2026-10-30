//! The rule catalog: every rule, its name in `leptoskit.toml`, and what it
//! asks of a project.

use std::fmt;

use leptoskit_config::LintLevel;
use leptoskit_config::LintRules;

/// One leptoskit lint rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rule {
	/// A capitalized function that returns a view is a component, so it
	/// carries `#[component]`: `view!` can then call it with named props, and
	/// rustc stops asking for a snake case name.
	ComponentAttribute,
	/// Component trees are written the way `[lint] markup` says: markup in
	/// `view!` by default, or builder calls.
	MarkupStyle,
	/// A `View` with no classes, props, or handlers that wraps a single child
	/// adds a node and nothing else. Flutter's `avoid_unnecessary_containers`.
	RedundantView,
	/// An empty `View` sized only to push its siblings apart is better
	/// written as `gap-*` on the parent or a `Spacer()`. Flutter's
	/// `sized_box_for_whitespace`.
	SpacerView,
	/// A builder tree sets its properties before its children, so it reads
	/// top-down like markup. Flutter's `sort_child_properties_last`.
	ChildrenLast,
	/// An `Image` has an accessible name, or is marked decorative. The
	/// accessibility rules of React Native and Lynx apps.
	ImageLabel,
	/// Colors come from the theme rather than hand-written values. React
	/// Native's `no-color-literals`. Off by default.
	ColorLiteral,
}

impl Rule {
	/// Every rule, in catalog order.
	pub const ALL: [Self; 7] = [
		Self::ComponentAttribute,
		Self::MarkupStyle,
		Self::RedundantView,
		Self::SpacerView,
		Self::ChildrenLast,
		Self::ImageLabel,
		Self::ColorLiteral,
	];

	/// The rule's key under `[lint.rules]`.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::ComponentAttribute => "component_attribute",
			Self::MarkupStyle => "markup_style",
			Self::RedundantView => "redundant_view",
			Self::SpacerView => "spacer_view",
			Self::ChildrenLast => "children_last",
			Self::ImageLabel => "image_label",
			Self::ColorLiteral => "color_literal",
		}
	}

	/// The level a project's rules give this rule.
	#[must_use]
	pub const fn level(self, rules: &LintRules) -> LintLevel {
		match self {
			Self::ComponentAttribute => rules.component_attribute,
			Self::MarkupStyle => rules.markup_style,
			Self::RedundantView => rules.redundant_view,
			Self::SpacerView => rules.spacer_view,
			Self::ChildrenLast => rules.children_last,
			Self::ImageLabel => rules.image_label,
			Self::ColorLiteral => rules.color_literal,
		}
	}
}

impl fmt::Display for Rule {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.name())
	}
}
