//! The tree a native host reports, kept by replaying the mutations its
//! renderer applies, so the agent never walks the host's surface.

use std::collections::HashMap;

use leptoskit_devtools_protocol::Frame;
use leptoskit_devtools_protocol::MatchKey;
use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::SourceLocation;
use leptoskit_devtools_protocol::StyleRule;
use leptoskit_devtools_protocol::Tree;
use leptoskit_native::Mutation;
use leptoskit_native::NativeKind;
use leptoskit_native::devtools as keys;
use leptoskit_native::devtools::Origin;
use leptoskit_native::tailwind::StyleContext;
use leptoskit_native::tailwind::Tailwind;

/// One change to a native tree: the part of a renderer [`Mutation`] the
/// mirror keeps, or what the guest said about a node that no mutation
/// carries.
///
/// Ids are the surface's node ids, which the protocol's
/// [`NodeRef::Native`] and the platform's widgets use too.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
	/// A widget was created.
	Create {
		/// The widget.
		id: u64,
		/// Its kind.
		kind: NativeKind,
		/// The text it shows, if any.
		text: Option<String>,
	},
	/// A widget's properties changed; the mirror keeps only its text.
	Text {
		/// The widget.
		id: u64,
		/// The text it shows now, if any.
		text: Option<String>,
	},
	/// A widget was laid out.
	Layout {
		/// The widget.
		id: u64,
		/// Its frame, relative to its parent.
		frame: Frame,
	},
	/// A child was inserted into a parent.
	Insert {
		/// The parent.
		parent: u64,
		/// The child.
		child: u64,
		/// Where among the parent's children.
		index: usize,
	},
	/// A child was removed from its parent.
	Remove {
		/// The parent.
		parent: u64,
		/// The child.
		child: u64,
	},
	/// A widget was deleted.
	Delete {
		/// The widget.
		id: u64,
	},
	/// The classes and source location the guest sent for a widget.
	Details {
		/// The widget.
		id: u64,
		/// Its Tailwind classes, separated by spaces.
		classes: String,
		/// Where the guest created it, as `file:line:column`, when the guest
		/// was built with `devtools`.
		source: Option<String>,
		/// The `#[component]` whose body created it, when the guest was built
		/// with `devtools`.
		component: Option<String>,
	},
	/// The platform draws the widget with a view of its own, rather than
	/// leaving it out as a layout-only container. The root always has one.
	Drawn {
		/// The widget.
		id: u64,
	},
	/// What the surface resolves classes against changed: the viewport,
	/// appearance, or safe area, or the control held down.
	Styling {
		/// The viewport, appearance, and safe area; `active` is ignored.
		context: StyleContext,
		/// The control held down, whose `active:` classes apply.
		pressed: Option<u64>,
	},
}

impl Change {
	/// What `mutation` changes in the mirror: everything but the widget
	/// properties other than text, which stay with the renderer.
	#[must_use]
	pub fn of(mutation: &Mutation) -> Self {
		match mutation {
			Mutation::Create { id, kind, props } => {
				Self::Create {
					id: id.index() as u64,
					kind: *kind,
					text: props.text.clone(),
				}
			}
			Mutation::UpdateProps { id, props } => {
				Self::Text {
					id: id.index() as u64,
					text: props.text.clone(),
				}
			}
			Mutation::UpdateLayout {
				id,
				x,
				y,
				width,
				height,
			} => {
				Self::Layout {
					id: id.index() as u64,
					frame: Frame {
						x: *x,
						y: *y,
						width: *width,
						height: *height,
					},
				}
			}
			Mutation::InsertChild {
				parent,
				child,
				index,
			} => {
				Self::Insert {
					parent: parent.index() as u64,
					child: child.index() as u64,
					index: *index,
				}
			}
			Mutation::RemoveChild { parent, child } => {
				Self::Remove {
					parent: parent.index() as u64,
					child: child.index() as u64,
				}
			}
			Mutation::Delete { id } => {
				Self::Delete {
					id: id.index() as u64,
				}
			}
		}
	}
}

/// One mirrored widget.
#[derive(Clone, Debug)]
struct Entry {
	kind: NativeKind,
	text: Option<String>,
	/// The frame relative to the parent, once laid out.
	frame: Option<Frame>,
	parent: Option<u64>,
	children: Vec<u64>,
	classes: String,
	source: Option<String>,
	component: Option<String>,
	/// Whether the platform draws a view for it.
	drawn: bool,
	/// Its active classes and their declarations, explained when its
	/// classes or the styling change rather than on every report.
	styles: Vec<StyleRule>,
}

/// A native host's committed tree, kept by applying the same changes its
/// renderer applies, in the same order.
#[derive(Clone, Debug, Default)]
pub struct Mirror {
	nodes: HashMap<u64, Entry>,
	root: Option<u64>,
	context: StyleContext,
	pressed: Option<u64>,
}

impl Mirror {
	/// An empty mirror.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Applies one change. Changes that name a widget the mirror does not
	/// have are ignored, as the renderer would never send them.
	pub fn apply(&mut self, change: Change) {
		match change {
			Change::Create { id, kind, text } => {
				self.nodes.insert(
					id,
					Entry {
						kind,
						text,
						frame: None,
						parent: None,
						children: Vec::new(),
						classes: String::new(),
						source: None,
						component: None,
						drawn: false,
						styles: Vec::new(),
					},
				);
			}
			Change::Text { id, text } => {
				if let Some(entry) = self.nodes.get_mut(&id) {
					entry.text = text;
				}
			}
			Change::Layout { id, frame } => {
				if let Some(entry) = self.nodes.get_mut(&id) {
					entry.frame = Some(frame);
				}
			}
			Change::Insert {
				parent,
				child,
				index,
			} => self.insert(parent, child, index),
			Change::Remove { parent, child } => self.detach(parent, child),
			Change::Delete { id } => self.delete(id),
			Change::Details {
				id,
				classes,
				source,
				component,
			} => {
				let context = self.context_of(id);

				if let Some(entry) = self.nodes.get_mut(&id) {
					if entry.classes != classes {
						entry.styles = explain(&classes, &context);
						entry.classes = classes;
					}

					entry.source = source;
					entry.component = component;
				}
			}
			Change::Drawn { id } => {
				if let Some(entry) = self.nodes.get_mut(&id) {
					entry.drawn = true;
				}
			}
			Change::Styling { context, pressed } => self.restyle(context, pressed),
		}
	}

	/// Explains every node's classes again after what they resolve against
	/// changed: all of them for a new viewport, appearance, or safe area, and
	/// the controls let go and held down for a new press.
	fn restyle(&mut self, context: StyleContext, pressed: Option<u64>) {
		let context = StyleContext {
			active: false,
			..context
		};
		let all = context != self.context;
		let released = self.pressed;
		self.context = context;
		self.pressed = pressed;

		let ids: Vec<u64> = if all {
			self.nodes.keys().copied().collect()
		} else if released == pressed {
			return;
		} else {
			released.into_iter().chain(pressed).collect()
		};

		for id in ids {
			let context = self.context_of(id);

			if let Some(entry) = self.nodes.get_mut(&id)
				&& !entry.classes.is_empty()
			{
				entry.styles = explain(&entry.classes, &context);
			}
		}
	}

	/// What `id`'s classes resolve against.
	fn context_of(&self, id: u64) -> StyleContext {
		StyleContext {
			active: self.pressed == Some(id),
			..self.context
		}
	}

	/// Names the root the renderer shows, or none.
	pub fn set_root(&mut self, root: Option<u64>) {
		self.root = root;
	}

	/// The root the renderer shows, if any.
	#[must_use]
	pub fn root(&self) -> Option<u64> {
		self.root.filter(|root| self.nodes.contains_key(root))
	}

	/// The number of widgets mirrored, attached or not.
	#[must_use]
	pub fn len(&self) -> usize {
		self.nodes.len()
	}

	/// Whether the mirror has no widgets.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.nodes.is_empty()
	}

	/// The tree under the root in the protocol's shape, depth first: each
	/// node's match key, its frame in viewport space, and its children in
	/// order. `None` before the renderer shows a root.
	#[must_use]
	pub fn tree(&self) -> Option<Tree> {
		let root = self.root()?;
		let mut nodes = Vec::with_capacity(self.nodes.len());
		let mut stack = vec![(root, keys::ROOT, None, 0.0, 0.0)];

		// The renderer keeps a tree, so this visits each node once; the bound
		// only stops a malformed stream from looping.
		while let Some((id, key, parent, left, top)) = stack.pop() {
			if nodes.len() == self.nodes.len() {
				break;
			}

			let Some(entry) = self.nodes.get(&id) else {
				continue;
			};
			let frame = entry.frame.map(|frame| {
				Frame {
					x: left + frame.x,
					y: top + frame.y,
					width: frame.width,
					height: frame.height,
				}
			});
			let (left, top) = frame.map_or((left, top), |frame| (frame.x, frame.y));
			let origins: Vec<Origin<'_>> = entry
				.children
				.iter()
				.map(|child| self.origin(*child))
				.collect();
			let ordinals = keys::ordinals(origins.iter().copied());

			for ((child, origin), ordinal) in entry.children.iter().zip(origins).zip(ordinals).rev()
			{
				stack.push((
					*child,
					keys::child(key, origin, ordinal),
					Some(id),
					left,
					top,
				));
			}

			nodes.push(Node {
				id: NodeRef::Native(id),
				key: MatchKey::new(keys::format(key)),
				parent: parent.map(NodeRef::Native),
				kind: format!("{:?}", entry.kind),
				source: entry
					.source
					.as_deref()
					.and_then(source)
					.map(|(file, line, column)| {
						SourceLocation {
							file: file.to_owned(),
							line,
							column,
						}
					}),
				frame,
				classes: entry
					.classes
					.split_whitespace()
					.map(str::to_owned)
					.collect(),
				text: entry.text.clone(),
				children: entry
					.children
					.iter()
					.map(|child| NodeRef::Native(*child))
					.collect(),
				component: entry.component.clone(),
				styles: entry.styles.clone(),
			});
		}

		Some(Tree {
			root: NodeRef::Native(root),
			nodes,
		})
	}

	/// Where the platform draws `id`: the nearest view it draws, `id`'s own
	/// or an ancestor's, with `id`'s frame in the coordinates that view lays
	/// its children out in. That is the view's own for itself, at the origin,
	/// and its content's for a descendant, which for a scroll view moves as
	/// it scrolls. The root is always drawn, so only a widget that is not
	/// laid out or not under the root has none.
	#[must_use]
	pub fn anchor(&self, id: u64) -> Option<(u64, Frame)> {
		let entry = self.nodes.get(&id)?;
		let frame = entry.frame?;
		let (mut x, mut y) = (0.0, 0.0);
		let mut current = id;

		for _ in 0..=self.nodes.len() {
			let entry = self.nodes.get(&current)?;

			if entry.drawn || Some(current) == self.root {
				return Some((
					current,
					Frame {
						x,
						y,
						width: frame.width,
						height: frame.height,
					},
				));
			}

			let offset = entry.frame?;
			x += offset.x;
			y += offset.y;
			current = entry.parent?;
		}

		None
	}

	/// The widget a tap at `(x, y)` lands on, given in the coordinates the
	/// drawn view `view` lays its children out in: the deepest widget under
	/// it whose frame holds the point, a later sibling winning over an
	/// earlier one as it is drawn above, as the surface's hit test decides.
	/// `view` itself when none does. A `view` the mirror does not know is
	/// read as the root, with the point in viewport space.
	#[must_use]
	pub fn hit(&self, view: u64, x: f32, y: f32) -> Option<u64> {
		let (start, x, y) = match self.viewport_origin(view) {
			Some((left, top)) => (view, left + x, top + y),
			None => (self.root()?, x, y),
		};

		let (left, top) = self.viewport_origin(start)?;
		let mut best = start;
		let mut stack = vec![(start, left, top)];

		// Depth first, in drawing order: each node holding the point replaces
		// the one before. The bound only stops a malformed stream looping.
		for _ in 0..self.nodes.len() {
			let Some((id, left, top)) = stack.pop() else {
				break;
			};
			let Some(entry) = self.nodes.get(&id) else {
				continue;
			};

			if id != start
				&& let Some(frame) = entry.frame
				&& frame.width > 0.0
				&& frame.height > 0.0
				&& x >= left
				&& x <= left + frame.width
				&& y >= top
				&& y <= top + frame.height
			{
				best = id;
			}

			for child in entry.children.iter().rev() {
				let offset = self
					.nodes
					.get(child)
					.and_then(|child| child.frame)
					.map_or((0.0, 0.0), |frame| (frame.x, frame.y));
				stack.push((*child, left + offset.0, top + offset.1));
			}
		}

		Some(best)
	}

	/// Where `id`'s frame starts in viewport space, when it is under the
	/// root.
	fn viewport_origin(&self, id: u64) -> Option<(f32, f32)> {
		let root = self.root()?;
		let (mut left, mut top) = (0.0, 0.0);
		let mut current = id;

		for _ in 0..=self.nodes.len() {
			let entry = self.nodes.get(&current)?;
			let frame = entry.frame?;
			left += frame.x;
			top += frame.y;

			if current == root {
				return Some((left, top));
			}

			current = entry.parent?;
		}

		None
	}

	/// What tells a node apart from its siblings: where the guest created
	/// it, or its kind when the guest sent no location.
	fn origin(&self, id: u64) -> Origin<'_> {
		let Some(entry) = self.nodes.get(&id) else {
			return Origin::Kind(NativeKind::View);
		};

		match entry.source.as_deref().and_then(source) {
			Some((file, line, column)) => Origin::Source(file, line, column),
			None => Origin::Kind(entry.kind),
		}
	}

	fn insert(&mut self, parent: u64, child: u64, index: usize) {
		if parent == child || !self.nodes.contains_key(&parent) || !self.nodes.contains_key(&child)
		{
			return;
		}

		if let Some(previous) = self.nodes[&child].parent {
			self.detach(previous, child);
		}

		if let Some(entry) = self.nodes.get_mut(&parent) {
			let index = index.min(entry.children.len());
			entry.children.insert(index, child);
		}

		if let Some(entry) = self.nodes.get_mut(&child) {
			entry.parent = Some(parent);
		}
	}

	fn detach(&mut self, parent: u64, child: u64) {
		if let Some(entry) = self.nodes.get_mut(&parent) {
			entry.children.retain(|existing| *existing != child);
		}

		if let Some(entry) = self.nodes.get_mut(&child)
			&& entry.parent == Some(parent)
		{
			entry.parent = None;
		}
	}

	fn delete(&mut self, id: u64) {
		let Some(entry) = self.nodes.remove(&id) else {
			return;
		};

		if let Some(parent) = entry.parent
			&& let Some(parent) = self.nodes.get_mut(&parent)
		{
			parent.children.retain(|existing| *existing != id);
		}

		for child in entry.children {
			if let Some(child) = self.nodes.get_mut(&child) {
				child.parent = None;
			}
		}
	}
}

/// The active classes in `classes` and their declarations. A class the
/// compiler does not know is left out, rather than losing the rest.
fn explain(classes: &str, context: &StyleContext) -> Vec<StyleRule> {
	if classes.trim().is_empty() {
		return Vec::new();
	}

	let explained = match Tailwind::parse(classes) {
		Ok(tailwind) => tailwind.explain(context),
		Err(_) => {
			classes
				.split_whitespace()
				.filter_map(|class| Tailwind::parse(class).ok())
				.flat_map(|tailwind| tailwind.explain(context))
				.collect()
		}
	};

	explained
		.into_iter()
		.map(|explained| {
			StyleRule {
				class: explained.class,
				declarations: explained.declarations,
				variant: explained.variant,
			}
		})
		.collect()
}

/// A `file:line:column` location, as the guest and the web renderer write
/// it.
fn source(text: &str) -> Option<(&str, u32, u32)> {
	let mut parts = text.rsplitn(3, ':');
	let column = parts.next()?.parse().ok()?;
	let line = parts.next()?.parse().ok()?;
	let file = parts.next().filter(|file| !file.is_empty())?;
	Some((file, line, column))
}

#[cfg(test)]
mod tests {
	use leptoskit_native::Element;
	use leptoskit_native::HeadlessBackend;
	use leptoskit_native::NativeSurface;
	use leptoskit_native::flatten::Flattener;

	use super::*;

	/// A surface that has committed `element`, and a mirror fed the same
	/// mutations, with each node's classes and source as a guest sends them
	/// and the views a platform draws for them.
	struct Mirrored {
		surface: NativeSurface<HeadlessBackend>,
		mirror: Mirror,
		flattener: Flattener,
	}

	fn mirrored(element: &Element) -> Mirrored {
		let mut surface = NativeSurface::new(HeadlessBackend::new());
		surface.set_viewport(320.0, 480.0);
		surface.set_root(element);
		let mut mirrored = Mirrored {
			surface,
			mirror: Mirror::new(),
			flattener: Flattener::default(),
		};
		mirrored.feed();
		mirrored
	}

	impl Mirrored {
		fn set_root(&mut self, element: &Element) {
			self.surface.set_root(element);
			self.feed();
		}

		fn feed(&mut self) {
			let Self {
				surface,
				mirror,
				flattener,
			} = self;
			feed(surface, mirror, flattener);
		}
	}

	fn feed(
		surface: &mut NativeSurface<HeadlessBackend>,
		mirror: &mut Mirror,
		flattener: &mut Flattener,
	) {
		mirror.apply(Change::Styling {
			context: surface.style_context(),
			pressed: surface.pressed().map(|id| id.index() as u64),
		});
		let log = surface.backend_mut().take_log();

		for mutation in &log {
			mirror.apply(Change::of(mutation));
		}

		for mutation in flattener.translate(log) {
			if let Mutation::Create { id, .. } = mutation {
				mirror.apply(Change::Drawn {
					id: id.index() as u64,
				});
			}
		}

		for node in surface.devtools_tree().nodes {
			let NodeRef::Native(id) = node.id else {
				unreachable!("a native tree names native nodes");
			};
			mirror.apply(Change::Details {
				id,
				classes: node.classes.join(" "),
				source: node
					.source
					.map(|source| format!("{}:{}:{}", source.file, source.line, source.column)),
				component: node.component,
			});
		}

		mirror.set_root(surface.root().map(|root| root.index() as u64));
	}

	fn screen(rows: usize) -> Element {
		Element::view()
			.class("flex-col gap-2 p-4")
			.child(Element::text("Title").class("text-lg"))
			.children((0..rows).map(|row| {
				Element::view()
					.class("flex-row gap-1")
					.child(Element::text(format!("Row {row}")))
					.child(Element::button("Open"))
			}))
	}

	#[test]
	fn the_mirror_matches_the_surface_it_follows() {
		let Mirrored {
			surface, mirror, ..
		} = mirrored(&screen(3));
		let expected = surface.devtools_tree();
		let tree = mirror.tree().unwrap();
		assert_eq!(tree.root, expected.root);
		assert_eq!(tree.nodes.len(), expected.nodes.len());

		for (node, expected) in tree.nodes.iter().zip(&expected.nodes) {
			assert_eq!(node.id, expected.id);
			assert_eq!(node.key, expected.key, "{} keys", node.kind);
			assert_eq!(node.parent, expected.parent);
			assert_eq!(node.kind, expected.kind);
			assert_eq!(node.source, expected.source);
			assert_eq!(node.classes, expected.classes);
			assert_eq!(node.text, expected.text);

			assert_eq!(node.children, expected.children);
			assert_eq!(node.frame, expected.frame, "{} frames", node.kind);
			assert_eq!(node.styles, expected.styles, "{} styles", node.kind);
		}

		assert!(
			tree.nodes[0]
				.styles
				.iter()
				.any(|rule| rule.class == "p-4" && rule.declarations.contains("padding")),
			"{:?}",
			tree.nodes[0].styles
		);

		let highlights: HashMap<usize, leptoskit_native::surface::Frame> =
			surface.highlight_rects().collect();

		for node in &tree.nodes {
			let NodeRef::Native(id) = node.id else {
				unreachable!("a native tree names native nodes");
			};
			let frame = node.frame.unwrap();
			let expected = highlights[&(id as usize)];
			assert_eq!(
				(frame.x, frame.y, frame.width, frame.height),
				(expected.x, expected.y, expected.width, expected.height),
				"{} frames are in viewport space",
				node.kind
			);
		}
	}

	#[test]
	fn the_mirror_follows_later_commits() {
		let mut mirrored = mirrored(&screen(3));
		mirrored.set_root(&screen(1));
		mirrored.set_root(&screen(4).class("p-8"));
		let Mirrored {
			surface, mirror, ..
		} = mirrored;
		let tree = mirror.tree().unwrap();
		let expected = surface.devtools_tree();
		let ids = |tree: &Tree| {
			tree.nodes
				.iter()
				.map(|node| (node.id.clone(), node.key.clone(), node.children.clone()))
				.collect::<Vec<_>>()
		};
		assert_eq!(ids(&tree), ids(&expected));

		assert_eq!(tree.nodes[0].classes, ["flex-col", "gap-2", "p-4", "p-8"]);
		assert_eq!(mirror.len(), tree.nodes.len(), "deleted widgets are gone");
	}

	#[test]
	fn styles_follow_the_viewport_and_appearance() {
		let element = Element::view().class("p-4 md:p-8 dark:bg-black");
		let mut mirrored = mirrored(&element);
		let classes = |mirror: &Mirror| {
			mirror.tree().unwrap().nodes[0]
				.styles
				.iter()
				.map(|rule| rule.class.clone())
				.collect::<Vec<_>>()
		};
		assert_eq!(classes(&mirrored.mirror), ["p-4"]);

		mirrored.surface.set_viewport(800.0, 480.0);
		mirrored.surface.set_dark_mode(true);
		mirrored.feed();
		let expected = mirrored.surface.devtools_tree().nodes[0].styles.clone();
		assert_eq!(mirrored.mirror.tree().unwrap().nodes[0].styles, expected);
		assert!(classes(&mirrored.mirror).contains(&"md:p-8".to_owned()));
		assert!(classes(&mirrored.mirror).contains(&"dark:bg-black".to_owned()));

		// A class the compiler does not know leaves the rest explained.
		mirrored.mirror.apply(Change::Details {
			id: mirrored.surface.root().unwrap().index() as u64,
			classes: "p-4 not-a-class".to_owned(),
			source: None,
			component: None,
		});
		assert_eq!(classes(&mirrored.mirror), ["p-4"]);
	}

	/// A screen with a layout-only container, which no platform draws, in a
	/// scroll view: `root > scroll > content > [card > (title, spacer),
	/// footer]`.
	fn nested() -> Element {
		Element::view().class("p-4").child(
			Element::scroll_view().class("h-40").child(
				Element::view()
					.class("bg-white p-2")
					.child(
						Element::view()
							.class("p-3")
							.child(Element::text("Title").class("w-20 h-6"))
							.child(Element::view().class("w-20 h-4")),
					)
					.child(Element::text("Footer").class("w-20 h-6")),
			),
		)
	}

	/// The mirrored node ids of `nested()`, depth first.
	fn ids(mirror: &Mirror) -> Vec<u64> {
		mirror
			.tree()
			.unwrap()
			.nodes
			.iter()
			.map(|node| {
				let NodeRef::Native(id) = node.id else {
					unreachable!("a native tree names native nodes");
				};
				id
			})
			.collect()
	}

	#[test]
	fn a_highlight_is_drawn_over_the_nearest_view_the_platform_draws() {
		let Mirrored { mirror, .. } = mirrored(&nested());
		let [root, scroll, content, card, title, spacer, footer] = ids(&mirror)[..] else {
			panic!("{:?}", ids(&mirror));
		};
		let tree = mirror.tree().unwrap();
		let viewport = |id| {
			tree.nodes
				.iter()
				.find(|node| node.id == NodeRef::Native(id))
				.and_then(|node| node.frame)
				.unwrap()
		};
		let within = |id, view| {
			let (frame, origin) = (viewport(id), viewport(view));
			Frame {
				x: frame.x - origin.x,
				y: frame.y - origin.y,
				..frame
			}
		};

		// Drawn widgets anchor to themselves.
		assert_eq!(
			mirror
				.anchor(title)
				.map(|(view, frame)| (view, frame.x, frame.y)),
			Some((title, 0.0, 0.0))
		);
		assert_eq!(mirror.anchor(root).unwrap().0, root);
		assert_eq!(mirror.anchor(scroll).unwrap().0, scroll);
		// The scroll view's content keeps its view; the card inside it is a
		// layout-only container, drawn as part of the content's view.
		assert_eq!(mirror.anchor(content).unwrap().0, content);
		assert_eq!(mirror.anchor(card), Some((content, within(card, content))));
		assert_eq!(
			(within(card, content).x, within(card, content).y),
			(8.0, 8.0)
		);
		// The spacer is layout-only too, so it is placed through the card.
		assert_eq!(
			mirror.anchor(spacer),
			Some((content, within(spacer, content)))
		);
		assert_eq!(
			(within(spacer, content).x, within(spacer, content).y),
			(20.0, 44.0)
		);

		assert_eq!(mirror.anchor(footer).unwrap().0, footer);
	}

	#[test]
	fn a_tap_lands_on_the_deepest_widget_under_it() {
		let Mirrored { mirror, .. } = mirrored(&nested());
		let [root, scroll, content, card, title, spacer, footer] = ids(&mirror)[..] else {
			panic!("{:?}", ids(&mirror));
		};

		// In the content's coordinates, as a platform's view reports them.
		assert_eq!(mirror.hit(content, 25.0, 25.0), Some(title));
		assert_eq!(mirror.hit(content, 25.0, 50.0), Some(spacer));
		// The card's padding is the card, though it has no view of its own.
		assert_eq!(mirror.hit(content, 9.0, 9.0), Some(card));
		assert_eq!(mirror.hit(content, 10.0, 75.0), Some(footer));
		assert_eq!(mirror.hit(content, 150.0, 2.0), Some(content));
		// A tap on a drawn widget with nothing under it is the widget.
		assert_eq!(mirror.hit(title, 1.0, 1.0), Some(title));
		assert_eq!(mirror.hit(root, 2.0, 2.0), Some(root));
		// An unknown view reads the point in viewport space from the root.
		assert_eq!(mirror.hit(9999, 16.0 + 25.0, 16.0 + 25.0), Some(title));

		assert_ne!(scroll, content);
	}

	#[test]
	fn text_and_layout_changes_reach_the_mirror() {
		let mut mirror = Mirror::new();
		mirror.apply(Change::Create {
			id: 1,
			kind: NativeKind::View,
			text: None,
		});
		mirror.apply(Change::Create {
			id: 2,
			kind: NativeKind::Text,
			text: Some("before".into()),
		});
		mirror.apply(Change::Insert {
			parent: 1,
			child: 2,
			index: 0,
		});
		let frame = |x, y| {
			Frame {
				x,
				y,
				width: 10.0,
				height: 5.0,
			}
		};
		mirror.apply(Change::Layout {
			id: 1,
			frame: frame(4.0, 8.0),
		});
		mirror.apply(Change::Layout {
			id: 2,
			frame: frame(1.0, 2.0),
		});
		mirror.apply(Change::Text {
			id: 2,
			text: Some("after".into()),
		});

		assert!(mirror.tree().is_none(), "nothing is shown before a root");
		mirror.set_root(Some(1));
		let tree = mirror.tree().unwrap();
		assert_eq!(tree.nodes[1].text.as_deref(), Some("after"));
		assert_eq!(tree.nodes[1].frame, Some(frame(5.0, 10.0)));
		// A root away from the origin: a tap in its child coordinates, and
		// one from a view the mirror doesn't know, in viewport space.
		assert_eq!(mirror.hit(1, 2.0, 3.0), Some(2));
		assert_eq!(mirror.hit(99, 6.0, 11.0), Some(2));
		assert_eq!(mirror.hit(99, 4.5, 8.5), Some(1));
		assert_eq!(mirror.anchor(2), Some((1, frame(1.0, 2.0))));
		assert_eq!(tree.nodes[1].kind, "Text");

		mirror.apply(Change::Remove {
			parent: 1,
			child: 2,
		});
		mirror.apply(Change::Delete { id: 2 });
		let tree = mirror.tree().unwrap();
		assert_eq!(tree.nodes.len(), 1);
		assert_eq!(tree.nodes[0].children, [] as [NodeRef; 0]);
	}

	#[test]
	fn a_source_without_a_location_falls_back_to_the_kind() {
		assert_eq!(source("src/app.rs:12:5"), Some(("src/app.rs", 12, 5)));
		assert_eq!(
			source("C:\\app\\src\\a.rs:3:1"),
			Some(("C:\\app\\src\\a.rs", 3, 1))
		);
		assert_eq!(source("src/app.rs"), None);
		assert_eq!(source(":1:2"), None);
	}
}
