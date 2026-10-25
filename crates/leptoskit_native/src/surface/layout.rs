//! Commits changed frames and rebuilds the existing hit-area cache.

use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::NodeId;
use leptoskit_yoga::Size;

use super::Frame;
use super::HitArea;
use super::HitBounds;
use super::NativeSurface;
use crate::backend::Mutation;
use crate::backend::MutationList;
use crate::backend::NativeBackend;
use crate::props::NativeKind;

impl<B: NativeBackend> NativeSurface<B> {
	pub(super) fn commit_layout(&mut self) {
		let Some(root) = self.root else {
			return;
		};
		self.yoga.calculate_layout(
			root,
			Size::new(
				AvailableSpace::Definite(self.viewport.width.max(0.0)),
				AvailableSpace::Definite(self.viewport.height.max(0.0)),
			),
		);
		let mut frames = MutationList::new();
		self.collect_frames(root, 0.0, 0.0, &mut frames);
		self.backend.apply(&frames);
	}

	/// Pushes changed frames for a subtree and rebuilds its hit areas.
	/// `origin_x` and `origin_y` are the parent's viewport position, and the
	/// returned bounds are the subtree's [`HitBounds`].
	pub(super) fn collect_frames(
		&mut self,
		id: NodeId,
		origin_x: f32,
		origin_y: f32,
		out: &mut MutationList,
	) -> HitBounds {
		let layout = *self.yoga.layout(id);
		let frame = Frame {
			x: layout.x,
			y: layout.y,
			width: layout.width,
			height: layout.height,
		};

		self.push_changed_frame(id, frame, out);

		let left = origin_x + layout.x;
		let top = origin_y + layout.y;
		let mut subtree = HitBounds {
			left,
			top,
			right: left + layout.width,
			bottom: top + layout.height,
		};
		let shadow = &self.shadows[id.index()];
		let inert = shadow.is_inert();
		// Clipped overflow is only hit inside the node's own frame, and a
		// pressable's children are decorative, so neither widens the bounds.
		let overflows = shadow.props.clip != Some(true) && shadow.kind != NativeKind::Pressable;

		for index in 0..shadow.children.len() {
			let child = self.shadows[id.index()].children[index];
			let child_bounds = self.collect_frames(child, left, top, out);

			if overflows {
				subtree = subtree.union(child_bounds);
			}
		}

		if inert {
			subtree = HitBounds::EMPTY;
		}

		self.hit_areas[id.index()] = HitArea {
			left,
			top,
			width: layout.width,
			height: layout.height,
			subtree,
		};
		subtree
	}

	/// Emits each changed frame once, preserving the existing allocated node
	/// slots.
	fn push_changed_frame(&mut self, id: NodeId, frame: Frame, out: &mut MutationList) {
		if self.frames.get(id.index()).copied().flatten() != Some(frame) {
			if self.frames.len() <= id.index() {
				self.frames.resize(id.index() + 1, None);
			}

			self.frames[id.index()] = Some(frame);
			out.push(Mutation::UpdateLayout {
				id,
				x: frame.x,
				y: frame.y,
				width: frame.width,
				height: frame.height,
			});
		}
	}
}
