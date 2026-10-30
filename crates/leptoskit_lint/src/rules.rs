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
}

impl Rule {
	/// Every rule, in catalog order.
	pub const ALL: [Self; 2] = [Self::ComponentAttribute, Self::MarkupStyle];

	/// The rule's key under `[lint.rules]`.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::ComponentAttribute => "component_attribute",
			Self::MarkupStyle => "markup_style",
		}
	}

	/// The level a project's rules give this rule.
	#[must_use]
	pub const fn level(self, rules: &LintRules) -> LintLevel {
		match self {
			Self::ComponentAttribute => rules.component_attribute,
			Self::MarkupStyle => rules.markup_style,
		}
	}
}

impl fmt::Display for Rule {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.name())
	}
}
