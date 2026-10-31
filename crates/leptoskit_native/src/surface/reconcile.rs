//! Reconciles source elements into the existing shadow tree and mutation
//! stream.

use leptoskit_yoga::NodeId;
use leptoskit_yoga::Style;

use super::HitArea;
use super::MeasuredText;
use super::NativeSurface;
use super::ShadowNode;
use super::has_native_chrome;
use super::measured_content;
use crate::backend::Mutation;
use crate::backend::MutationList;
use crate::backend::NativeBackend;
use crate::element::Element;
use crate::props::NativeKind;
use crate::props::NativeProps;
use crate::tailwind::StyleContext;

impl<B: NativeBackend> NativeSurface<B> {
	pub(super) fn commit_source(&mut self) {
		let Some(source) = &self.source else {
			return;
		};
		// Reconciliation is positional, so the held node's path in the
		// committed tree names its element in the new source.
		let held = self
			.pressed
			.filter(|&id| self.is_committed(id))
			.map(|id| self.path_to(id));
		let element = source.resolved_holding(
			&StyleContext {
				viewport: self.viewport,
				dark: self.dark,
				safe_area: self.safe_area,
				active: false,
			},
			held.as_deref(),
		);
		let mut mutations = MutationList::new();

		let new_root = match self.root {
			None => self.build_subtree(None, &element, &mut mutations),
			Some(root) => self.update_node(root, &element, &mut mutations),
		};

		self.root = Some(new_root);
		#[cfg(all(feature = "rive", not(target_arch = "wasm32")))]
		let mutations = self.sync_rive(mutations);
		#[cfg(feature = "devtools")]
		let source_start = std::time::Instant::now();
		self.backend.apply(&mutations);
		#[cfg(feature = "devtools")]
		self.devtools.source(mutations.len(), source_start);
		self.commit_layout();

		// A held control that was replaced, removed, disabled, or hidden by
		// this commit is no longer pressed, and no release will arrive for a
		// replacement built in its slot. Render that slot unpressed.
		if let Some(id) = self.pressed
			&& (!self.is_committed(id) || !self.event_path_is_active(id))
		{
			self.pressed = None;
			self.commit_source();
		}
	}

	pub(super) fn allocate(&mut self, kind: NativeKind) -> NodeId {
		let id = self.yoga.create_node();
		debug_assert_eq!(id.index(), self.shadows.len());
		self.shadows.push(ShadowNode {
			kind,
			props: NativeProps::default(),
			handlers: Vec::new(),
			children: Vec::new(),
			parent: None,
			text_cell: None,
			#[cfg(feature = "devtools")]
			source: None,
			#[cfg(feature = "devtools")]
			classes: Vec::new(),
		});
		self.frames.push(None);
		self.hit_areas.push(HitArea::DETACHED);
		id
	}

	/// Builds a fresh subtree for `element`.
	///
	/// When `slot` is `Some((parent, index))` the new root is inserted into
	/// `parent` at `index` — once, in the shadow tree, the layout tree, and the
	/// mutation stream — before its own children are built, so the batch stays
	/// top-down. With `None` the subtree is left detached.
	pub(super) fn build_subtree(
		&mut self,
		slot: Option<(NodeId, usize)>,
		element: &Element,
		out: &mut MutationList,
	) -> NodeId {
		let id = self.allocate(element.kind);
		self.shadows[id.index()].props = element.props.clone();
		self.shadows[id.index()].handlers = element.handlers().to_vec();
		#[cfg(feature = "devtools")]
		{
			self.shadows[id.index()].source = element.source_location();
			self.shadows[id.index()].classes = element
				.classes()
				.classes()
				.split_whitespace()
				.map(str::to_owned)
				.collect();
		}
		self.yoga.set_style(id, element.style.clone());

		if let Some(text) = measured_content(element) {
			self.attach_text_measure(
				id,
				MeasuredText {
					content: text.to_owned(),
					style: element.props.text_style(),
					native_chrome: has_native_chrome(element),
				},
			);
		}

		out.push(Mutation::Create {
			id,
			kind: element.kind,
			props: element.props.clone(),
		});

		if let Some((parent, index)) = slot {
			self.attach_child(parent, id, index, out);
		}

		for (index, child) in element.children.iter().enumerate() {
			self.build_subtree(Some((id, index)), child, out);
		}

		id
	}

	/// Inserts a detached node into `parent` at `index` (clamped to the
	/// current child count) and records the `InsertChild` mutation.
	pub(super) fn attach_child(
		&mut self,
		parent: NodeId,
		child: NodeId,
		index: usize,
		out: &mut MutationList,
	) {
		debug_assert!(self.shadows[child.index()].parent.is_none());
		let children = &mut self.shadows[parent.index()].children;
		let index = index.min(children.len());
		children.insert(index, child);
		self.shadows[child.index()].parent = Some(parent);
		self.yoga.insert_child(parent, child, index);
		out.push(Mutation::InsertChild {
			parent,
			child,
			index,
		});
	}

	pub(super) fn update_node(
		&mut self,
		id: NodeId,
		element: &Element,
		out: &mut MutationList,
	) -> NodeId {
		// A different widget kind is a different widget, and so is a different
		// accessibility role: platforms fix the role when the widget is
		// created (GTK's accessible role is construct-only), so a role change
		// tears the old subtree down and builds the replacement in the same
		// slot.
		let identity_changed = self.shadows[id.index()].kind != element.kind
			|| self.shadows[id.index()].props.aria().role != element.props.aria().role;

		if identity_changed {
			let slot = self.shadows[id.index()].parent.map(|parent| {
				let index = self.shadows[parent.index()]
					.children
					.iter()
					.position(|&child| child == id)
					.expect("a child is listed under its parent");
				(parent, index)
			});
			self.teardown(id, out);

			return self.build_subtree(slot, element, out);
		}

		if self.shadows[id.index()].props != element.props {
			self.shadows[id.index()].props = element.props.clone();
			self.update_text_measure(id, element);
			out.push(Mutation::UpdateProps {
				id,
				props: element.props.clone(),
			});
		}

		self.shadows[id.index()].handlers = element.handlers().to_vec();
		#[cfg(feature = "devtools")]
		{
			self.shadows[id.index()].source = element.source_location();
			self.shadows[id.index()].classes = element
				.classes()
				.classes()
				.split_whitespace()
				.map(str::to_owned)
				.collect();
		}
		self.yoga.set_style(id, element.style.clone());

		let old_children = self.shadows[id.index()].children.clone();
		self.reconcile_children(id, &old_children, &element.children, out);
		id
	}

	pub(super) fn reconcile_children(
		&mut self,
		parent: NodeId,
		old: &[NodeId],
		new: &[Element],
		out: &mut MutationList,
	) {
		for (index, element) in new.iter().enumerate() {
			match old.get(index) {
				Some(&child) => {
					self.update_node(child, element, out);
				}
				None => {
					self.build_subtree(Some((parent, index)), element, out);
				}
			}
		}

		// Extra old children are removed back to front so the indices in the
		// removal mutations stay valid as the backend applies them.
		for &child in old.iter().skip(new.len()).rev() {
			self.teardown(child, out);
		}
	}

	/// Removes a subtree, post-order: every node is detached from its parent
	/// (`RemoveChild`) and then deleted (`Delete`), so a widget never outlives
	/// its children and is always parentless when deleted.
	///
	/// The shadow slot stays allocated (ids are never reused), but it is left
	/// inert: no handlers, no parent, no children, and no measure function, so
	/// a stale id can neither receive events nor keep closures alive.
	pub(super) fn teardown(&mut self, id: NodeId, out: &mut MutationList) {
		let children = std::mem::take(&mut self.shadows[id.index()].children);

		for child in children {
			self.teardown(child, out);
		}

		if let Some(parent) = self.shadows[id.index()].parent.take() {
			self.shadows[parent.index()]
				.children
				.retain(|&child| child != id);
			self.yoga.detach(id);
			out.push(Mutation::RemoveChild { parent, child: id });
		}

		let shadow = &mut self.shadows[id.index()];

		shadow.handlers.clear();
		// Inert arena slots must not retain removed assets or other owned
		// props.
		shadow.props = NativeProps::default();
		self.yoga.set_style(id, Style::default());

		if shadow.text_cell.take().is_some() {
			self.yoga.set_measure_func(id, None);
		}

		self.frames[id.index()] = None;
		self.hit_areas[id.index()] = HitArea::DETACHED;
		out.push(Mutation::Delete { id });
	}
}
