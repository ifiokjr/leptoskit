//! View flattening must not change what the user sees: every element that
//! draws gets a native view at the same place on screen, with the same
//! properties, in the same order among its siblings.

use std::collections::HashMap;

use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::Button;
use leptoskit_native::components::ScrollView;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_native::flatten::Flattener;
use leptoskit_native::flatten::is_layout_only;
use leptoskit_yoga::NodeId;

/// A tree built by applying a mutation stream, as a platform would.
#[derive(Default)]
struct Model {
	nodes: HashMap<NodeId, ModelNode>,
}

struct ModelNode {
	kind: NativeKind,
	props: NativeProps,
	parent: Option<NodeId>,
	children: Vec<NodeId>,
	frame: [f32; 4],
}

impl Model {
	fn apply(&mut self, mutations: &[Mutation]) {
		for mutation in mutations {
			match mutation {
				Mutation::Create { id, kind, props } => {
					assert!(!self.nodes.contains_key(id), "{id:?} created twice");
					self.nodes.insert(
						*id,
						ModelNode {
							kind: *kind,
							props: props.clone(),
							parent: None,
							children: Vec::new(),
							frame: [0.0; 4],
						},
					);
				}
				Mutation::UpdateProps { id, props } => {
					self.nodes.get_mut(id).unwrap().props = props.clone();
				}
				Mutation::UpdateLayout {
					id,
					x,
					y,
					width,
					height,
				} => self.nodes.get_mut(id).unwrap().frame = [*x, *y, *width, *height],
				Mutation::InsertChild {
					parent,
					child,
					index,
				} => {
					assert!(
						self.nodes[child].parent.is_none(),
						"{child:?} inserted while attached"
					);
					let siblings = &mut self.nodes.get_mut(parent).unwrap().children;
					assert!(
						*index <= siblings.len(),
						"insert past the end of {parent:?}"
					);
					siblings.insert(*index, *child);
					self.nodes.get_mut(child).unwrap().parent = Some(*parent);
				}
				Mutation::RemoveChild { parent, child } => {
					assert_eq!(
						self.nodes[child].parent,
						Some(*parent),
						"{child:?} removed from a stranger"
					);
					self.nodes
						.get_mut(parent)
						.unwrap()
						.children
						.retain(|id| id != child);
					self.nodes.get_mut(child).unwrap().parent = None;
				}
				Mutation::Delete { id } => {
					let node = self.nodes.remove(id).unwrap();
					assert!(
						node.parent.is_none() && node.children.is_empty(),
						"{id:?} deleted while attached"
					);
				}
			}
		}
	}

	fn absolute(&self, mut id: NodeId) -> (f32, f32) {
		let mut position = (0.0, 0.0);

		loop {
			let node = &self.nodes[&id];
			position.0 += node.frame[0];
			position.1 += node.frame[1];

			match node.parent {
				Some(parent) => id = parent,
				None => return position,
			}
		}
	}

	fn root(&self) -> Option<NodeId> {
		let mut roots = self
			.nodes
			.iter()
			.filter(|(_, node)| node.parent.is_none())
			.map(|(id, _)| *id);
		let root = roots.next();
		assert!(roots.next().is_none(), "more than one root");
		root
	}

	/// The native descendants of `id` in document order, seen through the
	/// flattened containers below it.
	fn native_children(&self, id: NodeId, flattener: &Flattener, out: &mut Vec<NodeId>) {
		for &child in &self.nodes[&id].children {
			if flattener.is_native(child) {
				out.push(child);
			} else {
				self.native_children(child, flattener, out);
			}
		}
	}
}

/// A small fixed-seed generator, so a failure reproduces.
struct Random(u64);

impl Random {
	fn below(&mut self, bound: u64) -> u64 {
		self.0 = self
			.0
			.wrapping_mul(6_364_136_223_846_793_005)
			.wrapping_add(1_442_695_040_888_963_407);
		(self.0 >> 33) % bound
	}
}

const CLASSES: &[&str] = &[
	"",
	"p-2",
	"flex-row",
	"gap-2",
	"items-center",
	"w-16",
	"h-8",
	"flex-1",
	"m-1",
	"bg-red-500",
	"rounded",
	"opacity-50",
	"border",
	"text-white",
	"hidden",
	"overflow-hidden",
	"overflow-scroll",
];

fn element(random: &mut Random, depth: u32) -> Element {
	match random.below(if depth < 4 { 6 } else { 3 }) {
		0 => Text(format!("label {}", random.below(5))),
		1 => Button(format!("press {}", random.below(3))),
		_ => {
			let classes: Vec<&str> = (0..random.below(3))
				.map(|_| CLASSES[random.below(CLASSES.len() as u64) as usize])
				.collect();
			View()
				.class(classes.join(" "))
				.children((0..random.below(4)).map(|_| element(random, depth + 1)))
		}
	}
}

#[test]
fn flattening_keeps_every_drawn_element_in_place() {
	let mut promotions = 0;
	let mut flattened_checked = 0;

	for seed in 0..300 {
		let mut surface = NativeSurface::new(HeadlessBackend::new());
		surface.set_viewport(390.0, 844.0);
		let mut flattener = Flattener::default();
		let mut logical = Model::default();
		let mut native = Model::default();

		let mut random = Random(seed);

		for step in 0..6 {
			let screen = View()
				.class("p-4")
				.children((0..=random.below(3)).map(|_| element(&mut random, 0)));
			surface.set_root(&screen);
			let mutations = surface.backend_mut().take_log();
			let translated = flattener.translate(mutations.clone());
			promotions += translated
				.iter()
				.filter(
					|mutation| matches!(mutation, Mutation::Create { id, .. } if logical.nodes.contains_key(id)),
				)
				.count();
			logical.apply(&mutations);
			native.apply(&translated);

			let context = format!("seed {seed}, step {step}");
			assert_eq!(native.root(), logical.root(), "{context}: the root");

			for (id, node) in &logical.nodes {
				let is_native = flattener.is_native(*id);
				assert_eq!(
					native.nodes.contains_key(id),
					is_native,
					"{context}: {id:?}"
				);

				if !is_native {
					assert!(
						is_layout_only(node.kind, &node.props),
						"{context}: {id:?} draws but has no view"
					);
					assert!(
						node.parent.is_none_or(|parent| {
							logical.nodes[&parent].kind != NativeKind::ScrollView
						}),
						"{context}: {id:?} is a scroll view's content, which sizes its scrolling"
					);

					flattened_checked += 1;
					continue;
				}

				let view = &native.nodes[id];
				assert_eq!(view.kind, node.kind, "{context}: {id:?}");
				assert_eq!(view.props, node.props, "{context}: {id:?}");
				assert_eq!(view.frame[2..], node.frame[2..], "{context}: {id:?} size");
				let (expected, actual) = (logical.absolute(*id), native.absolute(*id));
				assert!(
					(expected.0 - actual.0).abs() < 1e-3 && (expected.1 - actual.1).abs() < 1e-3,
					"{context}: {id:?} at {actual:?}, expected {expected:?}"
				);
				let mut children = Vec::new();
				logical.native_children(*id, &flattener, &mut children);

				assert_eq!(view.children, children, "{context}: children of {id:?}");
			}
		}
	}

	// Both paths must have run for the comparison to mean anything.
	assert!(
		flattened_checked > 1000,
		"only {flattened_checked} flattened containers checked"
	);
	assert!(promotions > 20, "only {promotions} promotions checked");
}

/// A scroll view's direct content keeps its view and its size, even when
/// its children cover only part of it, as a virtual list's window does.
#[test]
fn a_scroll_views_content_keeps_its_view_and_size() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(390.0, 844.0);
	let screen = ScrollView().class("h-[400px]").child(
		View().class("h-[10000px] w-full").child(
			View()
				.class("absolute top-[200px] h-[72px] w-full")
				.child(Text("Item")),
		),
	);
	surface.set_root(&screen);
	let mut flattener = Flattener::default();
	let mut native = Model::default();
	native.apply(&flattener.translate(surface.backend_mut().take_log()));

	let root = native.root().expect("the scroll view is the root");
	assert_eq!(native.nodes[&root].kind, NativeKind::ScrollView);
	let content = native.nodes[&root].children[0];
	assert_eq!(native.nodes[&content].kind, NativeKind::View);
	assert!((native.nodes[&content].frame[3] - 10_000.0).abs() < f32::EPSILON);
}
