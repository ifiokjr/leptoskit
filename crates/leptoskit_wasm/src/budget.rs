//! Host allocation limits survive guest refueling; only work is replenished.

use super::host_trap;

/// Maximum live wire nodes, including detached and implicitly created nodes.
pub const MAX_RENDER_NODES: usize = 4096;
/// Maximum cumulative native shadow slots before the guest must be reloaded.
/// The renderer preserves deleted IDs as inert slots rather than reusing them.
pub const MAX_RENDER_ALLOCATED_NODES: usize = 65536;
/// Maximum JSON bytes accepted by one `node_set_props` call.
pub const MAX_NODE_PROPERTY_BYTES: usize = 64 * 1024;
/// Maximum sum of retained property payload bounds for all live nodes.
/// Compact Rive updates also retain the original scene's payload bound.
/// Counting input conservatively covers unknown fields and whitespace.
pub const MAX_RENDER_PROPERTY_BYTES: usize = 4 * 1024 * 1024;
/// Host work per entry point, in bytes plus a fixed cost per node operation.
/// Assembly pays for the entire retained forest before cloning any properties.
pub const MAX_RENDER_WORK: usize = 16 * 1024 * 1024;
/// Maximum JSON bytes retained in an undrained headless mutation queue.
pub const MAX_RENDER_MUTATION_BYTES: usize = 32 * 1024 * 1024;
/// Maximum property input plus text expanded during accessibility resolution.
pub const MAX_RENDER_COMMIT_BYTES: usize = 16 * 1024 * 1024;

/// Count derived text before the renderer copies subtree names or joins ID
/// references. Raw property quotas alone do not bound those expansions.
pub(super) fn commit_bytes(
	tree: &leptoskit_native::Element,
	property_bytes: usize,
) -> Option<usize> {
	fn collect<'a>(
		tree: &'a leptoskit_native::Element,
		nodes: &mut Vec<(&'a leptoskit_native::Element, usize)>,
	) -> usize {
		let mut content = tree
			.props
			.text
			.as_deref()
			.map_or(0, |text| text.trim().len());

		for child in &tree.children {
			let size = collect(child, nodes);
			content = content.saturating_add(size).saturating_add(1);
		}

		if let Some(label) = tree
			.props
			.accessibility_label
			.as_deref()
			.filter(|label| !label.trim().is_empty())
		{
			content = label.trim().len();
		}

		nodes.push((tree, content));
		content
	}

	let mut nodes = Vec::new();
	collect(tree, &mut nodes);
	let mut names = std::collections::BTreeMap::new();
	let mut bytes = property_bytes;

	for (node, content) in &nodes {
		if let Some(id) = node.props.id.as_deref() {
			// Repeated IDs must not make the estimate depend on traversal
			// order: the renderer can select a longer name for that ID.
			let size = names.entry(id).or_insert(0usize);
			*size = (*size).max(*content);
			bytes = bytes.saturating_add(*content);
		}
	}

	for (node, content) in nodes {
		let references = |ids: &[String]| {
			ids.iter()
				.filter_map(|id| names.get(id.as_str()))
				.fold(0usize, |bytes, size| {
					bytes.saturating_add(*size).saturating_add(1)
				})
		};
		let label = references(&node.props.aria().labelled_by);
		let description = references(&node.props.aria().described_by);
		bytes = bytes.saturating_add(label.saturating_add(description).saturating_mul(2));

		if node.props.aria().live.is_some() || node.props.aria().role.is_some() {
			bytes = bytes.saturating_add(content.max(label));
		}

		if bytes > MAX_RENDER_COMMIT_BYTES {
			return None;
		}
	}

	Some(bytes)
}

#[derive(Debug)]
pub(super) struct RenderBudget {
	pub property_bytes: usize,
	remaining_work: usize,
	failure: Option<&'static str>,
}

impl Default for RenderBudget {
	fn default() -> Self {
		Self {
			property_bytes: 0,
			remaining_work: MAX_RENDER_WORK,
			failure: None,
		}
	}
}

impl RenderBudget {
	pub fn error(&self) -> Option<&'static str> {
		self.failure
	}

	pub fn stop(&mut self, message: &'static str) {
		self.failure.get_or_insert(message);
	}

	pub fn reject(&mut self, message: &'static str) -> wasmi::Error {
		self.stop(message);
		host_trap(message)
	}

	pub fn refuel(&mut self) -> Result<(), wasmi::Error> {
		if let Some(error) = self.failure {
			return Err(host_trap(error));
		}

		self.remaining_work = MAX_RENDER_WORK;
		Ok(())
	}

	pub fn charge(&mut self, bytes: usize) -> Result<(), wasmi::Error> {
		if let Some(error) = self.failure {
			return Err(host_trap(error));
		}

		if bytes > self.remaining_work {
			return Err(self.reject("render work limit exceeded; reload the guest"));
		}

		self.remaining_work -= bytes;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_native::Element;

	use super::commit_bytes;

	#[test]
	fn duplicate_ids_cannot_hide_accessibility_text_expansion() {
		let tree = Element::view()
			.id("duplicate")
			.aria_label("short")
			.child(Element::text("x".repeat(60000)).id("duplicate"))
			.child(Element::view().aria_labelled_by(["duplicate"; 512]));
		assert!(commit_bytes(&tree, 65536).is_none());
	}
}
