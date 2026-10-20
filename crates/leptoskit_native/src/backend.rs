//! The mutation stream the diff produces, and the backend contract that
//! consumes it.
//!
//! This is the React Native Fabric model: the renderer never touches widgets
//! directly. It commits a list of mutations — create, update props, update
//! layout, insert, remove, delete — and the platform backend applies them to
//! real widgets in one batched transaction.

use leptoskit_yoga::NodeId;

use crate::props::NativeKind;
use crate::props::NativeProps;
use crate::props::SharedTextMeasurer;

/// One change to the native widget tree.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Mutation {
	/// Creates a widget.
	Create {
		/// The id of the created widget.
		id: NodeId,
		/// The widget kind.
		kind: NativeKind,
		/// The initial properties.
		props: NativeProps,
	},
	/// Updates a widget's properties.
	UpdateProps {
		/// The id of the widget.
		id: NodeId,
		/// The full replacement properties.
		props: NativeProps,
	},
	/// Updates a widget's frame after layout.
	UpdateLayout {
		/// The id of the widget.
		id: NodeId,
		/// The x offset within the parent.
		x: f32,
		/// The y offset within the parent.
		y: f32,
		/// The border-box width.
		width: f32,
		/// The border-box height.
		height: f32,
	},
	/// Inserts a child into a parent at an index.
	InsertChild {
		/// The parent widget id.
		parent: NodeId,
		/// The child widget id.
		child: NodeId,
		/// The insertion index.
		index: usize,
	},
	/// Removes a child from its parent.
	RemoveChild {
		/// The parent widget id.
		parent: NodeId,
		/// The child widget id.
		child: NodeId,
	},
	/// Deletes a widget that no longer has a parent.
	Delete {
		/// The id of the deleted widget.
		id: NodeId,
	},
}

/// A batch of mutations produced by one commit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MutationList {
	mutations: Vec<Mutation>,
}

impl MutationList {
	/// An empty list.
	pub fn new() -> Self {
		Self::default()
	}

	/// Appends a mutation.
	pub fn push(&mut self, mutation: Mutation) {
		self.mutations.push(mutation);
	}

	/// Extends from another list.
	pub fn extend(&mut self, other: MutationList) {
		self.mutations.extend(other.mutations);
	}

	/// The mutations, in application order.
	pub fn mutations(&self) -> &[Mutation] {
		&self.mutations
	}

	/// The number of mutations in the batch.
	pub fn len(&self) -> usize {
		self.mutations.len()
	}

	/// Whether the batch is empty.
	pub fn is_empty(&self) -> bool {
		self.mutations.is_empty()
	}

	/// Consumes the list into its mutations.
	pub fn into_mutations(self) -> Vec<Mutation> {
		self.mutations
	}
}

/// The platform contract: one method per mutation, applied in batch order.
///
/// Implementations map each call onto real widgets — `UIView`s, Android
/// views, `NSView`s — and never render HTML.
pub trait NativeBackend {
	/// Applies a whole mutation batch.
	///
	/// The default forwards each mutation to the individual methods, which is
	/// correct for every backend; override it only to wrap the batch in a
	/// platform transaction.
	fn apply(&mut self, mutations: &MutationList) {
		for mutation in mutations.mutations() {
			self.apply_one(mutation);
		}
	}

	/// Applies a single mutation.
	fn apply_one(&mut self, mutation: &Mutation) {
		match mutation {
			Mutation::Create { id, kind, props } => self.create(*id, *kind, props),
			Mutation::UpdateProps { id, props } => self.update_props(*id, props),
			Mutation::UpdateLayout {
				id,
				x,
				y,
				width,
				height,
			} => self.update_layout(*id, *x, *y, *width, *height),
			Mutation::InsertChild {
				parent,
				child,
				index,
			} => self.insert_child(*parent, *child, *index),
			Mutation::RemoveChild { parent, child } => self.remove_child(*parent, *child),
			Mutation::Delete { id } => self.delete(*id),
		}
	}

	/// Creates a widget.
	fn create(&mut self, id: NodeId, kind: NativeKind, props: &NativeProps);

	/// Replaces a widget's properties.
	fn update_props(&mut self, id: NodeId, props: &NativeProps);

	/// Sets a widget's frame.
	fn update_layout(&mut self, id: NodeId, x: f32, y: f32, width: f32, height: f32);

	/// Inserts a child widget.
	fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize);

	/// Removes a child widget.
	fn remove_child(&mut self, parent: NodeId, child: NodeId);

	/// Deletes a widget.
	fn delete(&mut self, id: NodeId);

	/// The backend's immutable text-metric snapshot, used during layout.
	/// After changing the host font environment, call
	/// [`NativeSurface::refresh_text_measurer`](crate::NativeSurface::refresh_text_measurer)
	/// to replace existing snapshots and invalidate cached measurements.
	fn text_measurer(&self) -> SharedTextMeasurer;

	/// Applies the host's explicit color scheme to platform control chrome.
	/// Backends without native appearance support keep the default no-op.
	fn set_dark_mode(&mut self, _dark: bool) {}
}

/// A backend that records every mutation instead of touching widgets.
///
/// Used by tests, snapshot suites, and the wasm host's recording mode.
#[derive(Default)]
pub struct HeadlessBackend {
	log: Vec<Mutation>,
	measurer: Option<SharedTextMeasurer>,
}

impl std::fmt::Debug for HeadlessBackend {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("HeadlessBackend")
			.field("log", &self.log)
			.finish_non_exhaustive()
	}
}

impl HeadlessBackend {
	/// A backend that records mutations and measures text with the
	/// approximation.
	pub fn new() -> Self {
		Self::default()
	}

	/// Overrides the text measurer.
	#[must_use]
	pub fn with_text_measurer(mut self, measurer: SharedTextMeasurer) -> Self {
		self.measurer = Some(measurer);
		self
	}

	/// Every mutation applied so far.
	pub fn log(&self) -> &[Mutation] {
		&self.log
	}

	/// Clears the log.
	pub fn clear(&mut self) {
		self.log.clear();
	}
}

impl NativeBackend for HeadlessBackend {
	fn create(&mut self, id: NodeId, kind: NativeKind, props: &NativeProps) {
		self.log.push(Mutation::Create {
			id,
			kind,
			props: props.clone(),
		});
	}

	fn update_props(&mut self, id: NodeId, props: &NativeProps) {
		self.log.push(Mutation::UpdateProps {
			id,
			props: props.clone(),
		});
	}

	fn update_layout(&mut self, id: NodeId, x: f32, y: f32, width: f32, height: f32) {
		self.log.push(Mutation::UpdateLayout {
			id,
			x,
			y,
			width,
			height,
		});
	}

	fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
		self.log.push(Mutation::InsertChild {
			parent,
			child,
			index,
		});
	}

	fn remove_child(&mut self, parent: NodeId, child: NodeId) {
		self.log.push(Mutation::RemoveChild { parent, child });
	}

	fn delete(&mut self, id: NodeId) {
		self.log.push(Mutation::Delete { id });
	}

	fn text_measurer(&self) -> SharedTextMeasurer {
		self.measurer
			.clone()
			.unwrap_or_else(|| std::sync::Arc::new(crate::props::ApproxTextMeasurer::new()))
	}
}
