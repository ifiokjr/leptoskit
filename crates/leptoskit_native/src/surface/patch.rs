//! Incremental commits: replace parts of the source tree and reconcile only
//! those parts, instead of resolving and diffing the whole tree.
//!
//! A host that knows which nodes changed since its last commit, such as the
//! wasm runtime, hands the surface one patch per changed node. Each patch is
//! resolved under its parent's committed props and reconciled in place, so a
//! commit that changes one label in a thousand rows costs one label.

use leptoskit_yoga::NodeId;

use super::NativeSurface;
use crate::backend::MutationList;
use crate::backend::NativeBackend;
use crate::controls::ControlTreeError;
use crate::element::Element;
use crate::props::NativeKind;
use crate::tailwind::StyleContext;

/// One change to a committed tree, for [`NativeSurface::try_patch`].
#[derive(Debug)]
pub enum SourcePatch {
	/// Replaces a node's kind, style, props, classes, and handlers, keeping
	/// its children. The element's own children are ignored.
	Node {
		/// The committed node.
		node: NodeId,
		/// Its replacement.
		element: Element,
	},
	/// Replaces a node and everything below it.
	Subtree {
		/// The committed node.
		node: NodeId,
		/// Its replacement, children included.
		element: Element,
	},
}

impl SourcePatch {
	fn node(&self) -> NodeId {
		match self {
			Self::Node { node, .. } | Self::Subtree { node, .. } => *node,
		}
	}

	fn is_subtree(&self) -> bool {
		matches!(self, Self::Subtree { .. })
	}
}

impl<B: NativeBackend> NativeSurface<B> {
	/// Commits changes to parts of the committed tree, resolving and diffing
	/// only those parts.
	///
	/// Returns the committed node at each patch's position afterwards, in the
	/// order given: a patch that changes a node's widget kind or role rebuilds
	/// it under a new id. Returns `Ok(None)`, leaving everything untouched,
	/// when the change cannot be applied in place and the caller should
	/// commit the whole tree with [`Self::try_set_owned_root`] instead: no
	/// tree is committed, a patched node is not committed, one patch lies
	/// inside another's subtree, or the tree declares accessibility
	/// relationships (ids, id references, radio groups, live regions), which
	/// resolve across the whole tree.
	///
	/// An invalid patch is rejected before anything changes, as
	/// [`Self::try_set_root`] rejects an invalid tree.
	pub fn try_patch(
		&mut self,
		patches: Vec<SourcePatch>,
	) -> Result<Option<Vec<NodeId>>, ControlTreeError> {
		if self.source.is_none() || self.root.is_none() || self.relationships {
			return Ok(None);
		}

		let mut planned = Vec::with_capacity(patches.len());

		for patch in patches {
			let node = patch.node();

			if !self.is_committed(node) {
				return Ok(None);
			}

			planned.push((self.path_to(node), patch));
		}

		// Ancestors first, so a parent's resolved props are settled before a
		// patch below it resolves against them.
		planned.sort_by(|(left, _), (right, _)| left.cmp(right));

		if planned.windows(2).any(|pair| {
			let (outer, patch) = &pair[0];
			patch.is_subtree() && pair[1].0.starts_with(outer)
		}) {
			return Ok(None);
		}

		let source = self.source.as_ref().expect("checked above");
		let mut checked = Vec::with_capacity(planned.len());

		for (path, patch) in planned {
			let current = source_at(source, &path);
			let patch = match patch {
				// A different kind changes what its children may be, so the
				// node is replaced along with them.
				SourcePatch::Node { element, .. } if element.kind != current.kind => {
					let mut element = element;
					element.children.clone_from(&current.children);
					(true, element)
				}
				SourcePatch::Node { mut element, .. } => {
					element.children = Vec::new();
					(false, element)
				}
				SourcePatch::Subtree { element, .. } => (true, element),
			};

			if !patch.1.is_self_contained() {
				return Ok(None);
			}

			patch.1.validate_within(button_ancestor(source, &path))?;
			checked.push((path, patch));
		}

		let source = self.source.as_mut().expect("checked above");

		for (path, (subtree, element)) in &mut checked {
			let target = source_at_mut(source, path);

			if *subtree {
				*target = std::mem::replace(element, Element::view());
			} else {
				target.replace_own(std::mem::replace(element, Element::view()));
			}
		}

		// A held control resolves along its path through the whole tree; a
		// full commit of the patched source keeps that one rule in one place.
		if self.pressed.is_some() {
			self.commit_source();
		} else {
			self.reconcile_patches(checked.iter().map(|(path, (subtree, _))| (path, *subtree)));
		}

		Ok(Some(
			checked.iter().map(|(path, _)| self.node_at(path)).collect(),
		))
	}

	/// Resolves and reconciles each patched position of the updated source.
	fn reconcile_patches<'a>(&mut self, patches: impl Iterator<Item = (&'a Vec<usize>, bool)>) {
		let context = StyleContext {
			viewport: self.viewport,
			dark: self.dark,
			safe_area: self.safe_area,
			active: false,
		};
		let mut mutations = MutationList::new();
		let mut resolved_subtrees: Vec<&Vec<usize>> = Vec::new();

		for (path, subtree) in patches {
			if resolved_subtrees
				.iter()
				.any(|resolved| path.starts_with(resolved))
			{
				continue;
			}

			let id = self.node_at(path);
			let parent = self.shadows[id.index()]
				.parent
				.map(|parent| self.shadows[parent.index()].props.clone())
				.unwrap_or_default();
			let source = source_at(self.source.as_ref().expect("patched"), path);
			let own = (!subtree).then(|| source.resolve_own(&context, &parent, None));
			let own = own.filter(|own| {
				self.keeps_identity(id, own)
					&& !Element::inherited_differs(&self.shadows[id.index()].props, &own.props)
			});

			let Some(own) = own else {
				let (element, _) = source.resolve_inherited(&context, &parent, None);
				let replaced = self.update_node(id, &element, &mut mutations);

				if self.root == Some(id) {
					self.root = Some(replaced);
				}

				resolved_subtrees.push(path);
				continue;
			};

			self.update_own(id, &own, &mut mutations);
		}

		self.apply_and_lay_out(mutations);
	}

	/// The committed node at a child-index path from the root.
	fn node_at(&self, path: &[usize]) -> NodeId {
		path.iter().fold(
			self.root.expect("a committed tree has a root"),
			|node, &index| self.shadows[node.index()].children[index],
		)
	}
}

fn source_at<'a>(root: &'a Element, path: &[usize]) -> &'a Element {
	path.iter()
		.fold(root, |element, &index| &element.children[index])
}

fn source_at_mut<'a>(root: &'a mut Element, path: &[usize]) -> &'a mut Element {
	path.iter()
		.fold(root, |element, &index| &mut element.children[index])
}

/// The nearest `Button` or `Pressable` above the element at `path`.
fn button_ancestor(root: &Element, path: &[usize]) -> Option<NativeKind> {
	let mut element = root;
	let mut found = None;

	for &index in path {
		if matches!(element.kind, NativeKind::Button | NativeKind::Pressable) {
			found = Some(element.kind);
		}

		element = &element.children[index];
	}

	found
}
