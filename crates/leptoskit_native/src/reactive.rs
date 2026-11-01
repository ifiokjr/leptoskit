//! Values that follow a signal instead of being rebuilt with the tree.
//!
//! An [`Element`](crate::Element) is a description. Rebuilding it is the right
//! shape for a screen replaced wholesale, and the wrong shape for a value that
//! changes in place: rebuilding a thousand labels to change a hundred of them
//! costs the interpreter work of all thousand. A binding
//! ([`Text`](crate::components::Text) with a closure, [`Element::background`])
//! rewrites that one node. [`For`] and [`Show`] rebuild only their own
//! children when the list or the condition changes.
//!
//! [`Element::text_from`](crate::Element::text_from) and
//! [`Element::background_from`](crate::Element::background_from) bind one
//! element's value to a closure that reads a [`RwSignal`]. The host keeps the
//! node; a refresh reads each binding, sends the nodes whose value actually
//! changed, and commits once. The change is proportional to the nodes that
//! changed, and the application writes its state the way it does on the web.

use std::cell::Cell;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;

pub use reactive_graph::signal::RwSignal;
pub use reactive_graph::traits::Get;
pub use reactive_graph::traits::Set;

use crate::Element;
use crate::props::Color;

thread_local! {
	/// How many guest renders are on the stack. A binding read inside one does
	/// not subscribe the screen: the refresh visits bindings on its own, and a
	/// subscription here would rebuild the screen to change a label.
	static GUEST_RENDER: Cell<u32> = const { Cell::new(0) };
}

/// Runs `build` as a guest render, so binding reads stay untracked.
///
/// Only the guest runtime calls it. A non-guest build has no guest modules, so
/// the function looks unused there.
#[cfg_attr(
	not(all(feature = "guest", any(target_arch = "wasm32", test))),
	allow(dead_code, reason = "only the wasm guest runtime calls this")
)]
pub(crate) fn guest_render_scope<T>(build: impl FnOnce() -> T) -> T {
	GUEST_RENDER.with(|depth| depth.set(depth.get().saturating_add(1)));
	let value = build();
	GUEST_RENDER.with(|depth| depth.set(depth.get().saturating_sub(1)));
	value
}

/// The current value of a binding.
///
/// Outside a guest render the read is tracked, which is how the web renderer
/// hears that the screen depends on the signal. Inside one it is not: the
/// guest refreshes bindings itself, and the screen subscribes only to signals
/// it reads directly.
pub(crate) fn read_binding<T>(binding: &Binding<T>) -> T {
	if GUEST_RENDER.with(Cell::get) > 0 {
		reactive_graph::graph::untrack(|| binding.read())
	} else {
		binding.read()
	}
}

/// A value read from application state on demand.
///
/// The closure reads the signals it needs, so a node bound to it changes when
/// those signals do, without the element tree being rebuilt.
pub struct Binding<T>(Arc<dyn Fn() -> T + Send + Sync>);

impl<T> Binding<T> {
	/// Binds a value to `read`.
	pub fn new(read: impl Fn() -> T + Send + Sync + 'static) -> Self {
		Self(Arc::new(read))
	}

	/// The current value.
	pub fn read(&self) -> T {
		(self.0)()
	}
}

impl<T> Clone for Binding<T> {
	fn clone(&self) -> Self {
		Self(Arc::clone(&self.0))
	}
}

impl<T> fmt::Debug for Binding<T>
where
	T: fmt::Debug,
{
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_tuple("Binding").field(&self.read()).finish()
	}
}

/// What an element reads from application state instead of from the tree it
/// was built in.
#[derive(Clone, Debug, Default)]
pub struct Bindings {
	/// The text the element shows.
	pub text: Option<Binding<String>>,
	/// The element's background colour.
	pub background: Option<Binding<Color>>,
}

impl Bindings {
	/// Whether nothing is bound.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.text.is_none() && self.background.is_none()
	}
}

/// A container whose children are rebuilt when the signals `render` reads
/// change. The guest patches that container; the rest of the screen stays.
#[derive(Clone)]
pub(crate) struct Region {
	render: Arc<dyn Fn() -> Vec<(u64, Element)> + Send + Sync>,
}

impl Region {
	fn new(render: impl Fn() -> Vec<(u64, Element)> + Send + Sync + 'static) -> Self {
		Self {
			render: Arc::new(render),
		}
	}

	/// The container's current children, each with the key it is reused by.
	pub(crate) fn render(&self) -> Vec<(u64, Element)> {
		(self.render)()
	}

	/// The closure, for the guest subscription that re-runs it.
	///
	/// Only the guest runtime calls it. A non-guest build has no guest modules,
	/// so the method looks unused there.
	#[cfg_attr(
		not(all(feature = "guest", any(target_arch = "wasm32", test))),
		allow(dead_code, reason = "only the wasm guest runtime calls this")
	)]
	pub(crate) fn into_render(self) -> Arc<dyn Fn() -> Vec<(u64, Element)> + Send + Sync> {
		self.render
	}
}

/// A stable identity for one child of [`For`].
pub(crate) fn hash_key<K: Hash + ?Sized>(key: &K) -> u64 {
	let mut hasher = std::collections::hash_map::DefaultHasher::new();
	key.hash(&mut hasher);
	hasher.finish()
}

/// An element whose children come from `render`, filled once up front so a
/// renderer that never re-runs regions still shows the first value.
fn region_element(render: impl Fn() -> Vec<(u64, Element)> + Send + Sync + 'static) -> Element {
	let region = Region::new(render);
	let mut element = Element::view();
	let initial = reactive_graph::graph::untrack(|| region.render());
	element.children = initial.into_iter().map(|(_, child)| child).collect();
	element.region = Some(region);
	element
}

/// Renders one child per item, and rebuilds only this list when `each` changes.
///
/// `key` names an item across updates, so an item that stays keeps its node:
/// appending a row does not recreate the rows above it. The view closure runs
/// again for the items whose identity changed. A value that changes inside an
/// item — a label bound with [`Text`](crate::components::Text) — does not
/// rebuild the list at all.
///
/// ```
/// use leptoskit_native::Element;
/// use leptoskit_native::reactive::For;
///
/// let list = For(
/// 	|| vec!["red".to_owned(), "blue".to_owned()],
/// 	|colour| colour.clone(),
/// 	|colour| Element::text(colour),
/// );
/// assert_eq!(list.children.len(), 2);
/// ```
#[allow(non_snake_case)]
#[must_use]
pub fn For<T, K, KF, VF>(
	each: impl Fn() -> Vec<T> + Send + Sync + 'static,
	key: KF,
	view: VF,
) -> Element
where
	T: Clone + Send + Sync + 'static,
	K: Hash + 'static,
	KF: Fn(&T) -> K + Send + Sync + 'static,
	VF: Fn(T) -> Element + Send + Sync + 'static,
{
	let each = Arc::new(each);
	let key = Arc::new(key);

	let view = Arc::new(view);
	region_element(move || {
		each()
			.into_iter()
			.map(|item| {
				let id = hash_key(&key(&item));
				(id, view(item))
			})
			.collect()
	})
}

/// Renders `view` while `when` is true, and nothing otherwise.
///
/// The condition is read as a signal. Changing it replaces this container's
/// child and leaves the rest of the screen where it was. A screen that reads
/// the same condition itself, and returns a different tree, still rebuilds
/// the whole screen.
///
/// ```
/// use leptoskit_native::Element;
/// use leptoskit_native::reactive::Show;
///
/// assert_eq!(Show(|| true, || Element::text("on")).children.len(), 1);
/// assert!(Show(|| false, || Element::text("on")).children.is_empty());
/// ```
#[allow(non_snake_case)]
#[must_use]
pub fn Show<W, VF>(when: W, view: VF) -> Element
where
	W: Fn() -> bool + Send + Sync + 'static,
	VF: Fn() -> Element + Send + Sync + 'static,
{
	let when = Arc::new(when);
	let view = Arc::new(view);
	region_element(move || {
		if when() {
			vec![(1, view())]
		} else {
			Vec::new()
		}
	})
}
