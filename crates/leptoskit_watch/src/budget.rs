//! Hard limits that keep watch trees small.

use leptoskit_native::Element;
use leptoskit_native::NativeKind;

use crate::WatchError;

/// Layout budget for one watch surface.
///
/// Budgets exist so a phone screen cannot be pasted onto a watch unnoticed.
/// Hosts validate content against the budget before presenting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchBudget {
	/// Maximum nesting depth, counting the root as 1.
	pub max_depth: u8,
	/// Maximum number of element nodes.
	pub max_nodes: u16,
	/// Maximum total characters across text props.
	pub max_text_chars: u16,
}

impl WatchBudget {
	/// Interactive watch app page: still small, but room for a few controls.
	#[must_use]
	pub const fn page() -> Self {
		Self {
			max_depth: 5,
			max_nodes: 32,
			max_text_chars: 160,
		}
	}

	/// Wear OS tile / watchOS accessory widget.
	#[must_use]
	pub const fn tile() -> Self {
		Self {
			max_depth: 3,
			max_nodes: 12,
			max_text_chars: 64,
		}
	}

	/// Smart Stack / glance preview.
	#[must_use]
	pub const fn glance() -> Self {
		Self {
			max_depth: 3,
			max_nodes: 8,
			max_text_chars: 48,
		}
	}

	/// Face complication slot.
	#[must_use]
	pub const fn complication() -> Self {
		Self {
			max_depth: 2,
			max_nodes: 4,
			max_text_chars: 24,
		}
	}
}

impl Default for WatchBudget {
	fn default() -> Self {
		Self::page()
	}
}

/// Counts measured against a [`WatchBudget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WatchMeasure {
	/// Deepest nesting level.
	pub depth: u8,
	/// Total element nodes.
	pub nodes: u16,
	/// Total characters in text props.
	pub text_chars: u16,
}

/// Walks `root` and returns its measure, or the first budget violation.
pub fn measure(root: &Element, budget: WatchBudget) -> Result<WatchMeasure, WatchError> {
	let mut measure = WatchMeasure::default();
	walk(root, 1, budget, &mut measure)?;
	Ok(measure)
}

fn walk(
	element: &Element,
	depth: u8,
	budget: WatchBudget,
	measure: &mut WatchMeasure,
) -> Result<(), WatchError> {
	refuse_kind(element.kind)?;

	if depth > budget.max_depth {
		return Err(WatchError::TooDeep {
			depth,
			max: budget.max_depth,
		});
	}
	measure.depth = measure.depth.max(depth);

	let next_nodes = measure
		.nodes
		.checked_add(1)
		.ok_or(WatchError::TooManyNodes {
			nodes: u16::MAX,
			max: budget.max_nodes,
		})?;
	if next_nodes > budget.max_nodes {
		return Err(WatchError::TooManyNodes {
			nodes: next_nodes,
			max: budget.max_nodes,
		});
	}
	measure.nodes = next_nodes;

	if let Some(text) = element.props.text.as_deref() {
		let added = u16::try_from(text.chars().count()).unwrap_or(u16::MAX);
		let next = measure.text_chars.saturating_add(added);
		if next > budget.max_text_chars {
			return Err(WatchError::TooMuchText {
				chars: next,
				max: budget.max_text_chars,
			});
		}
		measure.text_chars = next;
	}

	for child in &element.children {
		walk(child, depth.saturating_add(1), budget, measure)?;
	}
	Ok(())
}

fn refuse_kind(kind: NativeKind) -> Result<(), WatchError> {
	match kind {
		NativeKind::View
		| NativeKind::Text
		| NativeKind::TextRun
		| NativeKind::Image
		| NativeKind::Button
		| NativeKind::Pressable
		| NativeKind::Switch
		| NativeKind::ActivityIndicator
		| NativeKind::LinearProgressIndicator
		| NativeKind::Vector => Ok(()),
		NativeKind::ScrollView => Err(WatchError::UnsupportedKind("ScrollView")),
		NativeKind::TextInput => Err(WatchError::UnsupportedKind("TextInput")),
		NativeKind::PasswordInput => Err(WatchError::UnsupportedKind("PasswordInput")),
		NativeKind::Checkbox => Err(WatchError::UnsupportedKind("Checkbox")),
		NativeKind::Radio => Err(WatchError::UnsupportedKind("Radio")),
		NativeKind::Slider => Err(WatchError::UnsupportedKind("Slider")),
		NativeKind::Rive => Err(WatchError::UnsupportedKind("Rive")),
		NativeKind::Custom(name) => Err(WatchError::UnsupportedKind(name)),
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_native::Element;
	use leptoskit_native::components::Button;
	use leptoskit_native::components::Text;
	use leptoskit_native::components::View;

	use super::*;

	#[test]
	fn page_budget_accepts_a_small_tree() {
		let tree = View().child(Text("Hello")).child(Button("Go"));
		assert!(measure(&tree, WatchBudget::page()).is_ok());
	}

	#[test]
	fn complication_budget_rejects_a_deep_tree() {
		let tree = View().child(View().child(View().child(Text("too deep"))));
		let error = measure(&tree, WatchBudget::complication()).unwrap_err();
		assert!(matches!(error, WatchError::TooDeep { .. }));
	}

	#[test]
	fn scroll_views_are_refused() {
		let tree = Element::scroll_view().child(Text("nope"));
		let error = measure(&tree, WatchBudget::page()).unwrap_err();
		assert_eq!(error, WatchError::UnsupportedKind("ScrollView"));
	}
}
