#![allow(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;

use super::tree::Host;
use super::tree::Tree;
use super::tree::decode_event;
use super::tree::invoke;
use crate::Element;
use crate::EventOutcome;

#[link(wasm_import_module = "leptoskit")]
unsafe extern "C" {
	fn node_create(id: i64, kind: i32);
	fn node_set_props(id: i64, pointer: *const u8, length: usize);
	fn node_insert(parent: i64, child: i64, index: i32);
	fn node_delete(id: i64);
	fn commit();
}

struct Application {
	view: Rc<dyn Fn() -> Element>,
	tree: Tree,
}

thread_local! {
	static APPLICATION: RefCell<Option<Application>> = const { RefCell::new(None) };
	/// The reactive owner every application signal belongs to.
	static OWNER: RefCell<Option<reactive_graph::owner::Owner>> = const { RefCell::new(None) };
	static PAYLOAD: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
	static VIEWPORT: std::cell::Cell<Option<(f32, f32)>> = const { std::cell::Cell::new(None) };
}

/// The host's viewport, `(width, height)` in layout units, once the host has
/// reported it.
///
/// Layout and responsive classes already follow the viewport on the host;
/// read this when the application itself decides by size, such as how many
/// rows a list shows.
pub fn viewport() -> Option<(f32, f32)> {
	VIEWPORT.get()
}

/// Records the host's viewport and renders again.
///
/// Forward the optional `leptoskit_viewport(width: f32, height: f32)` export
/// here. Hosts call it before `leptoskit_main` and whenever the viewport
/// changes; hosts that predate it never call it, and [`viewport`] stays
/// `None`.
pub fn set_viewport(width: f32, height: f32) {
	if width.is_finite() && height.is_finite() {
		VIEWPORT.set(Some((width.max(0.0), height.max(0.0))));
		render();
	}
}

/// Mounts a portable application tree in the native Wasm host.
///
/// Create application state before this call and capture it in `view`; the
/// closure is called again after every dispatched event. The application must
/// export the ABI entry points that forward to this module's functions.
///
/// # Panics
/// Panics for an invalid control tree, unsupported custom widget, or a tree
/// deeper than the native ABI's 256-level limit.
pub fn mount(view: impl Fn() -> Element + 'static) {
	ensure_owner();
	APPLICATION.with_borrow_mut(|application| {
		let tree = application
			.take()
			.map_or_else(Tree::default, |app| app.tree);
		*application = Some(Application {
			view: Rc::new(view),
			tree,
		});
	});
	render();
}

/// Binds application signals to the renderer for this thread.
///
/// Signals need an owner to belong to and to be disposed with. The application
/// lives as long as the guest does, so one owner is created on first mount and
/// left current: a signal created in an event handler, a scenario, or a render
/// then lives for the application's lifetime.
fn ensure_owner() {
	OWNER.with_borrow_mut(|owner| {
		if owner.is_none() {
			let created = reactive_graph::owner::Owner::new();
			created.set();
			*owner = Some(created);
		}
	});
}

/// Sends the nodes whose bound value changed, without rebuilding the tree.
///
/// The native host calls this after every turn it gives the guest — the
/// plugin poll that runs application tasks, and the module's own start —
/// through the `leptoskit_flush` export, so an application never calls it
/// itself: writing a signal is enough.
///
/// Only the bound nodes are visited, and only the ones whose value differs
/// are encoded and sent, so a screen that changes a hundred rows out of a
/// thousand pays for a hundred. Events do not need it: [`dispatch`] renders
/// the view again after every handler, and that render reads the same
/// signals.
///
/// Call it before [`render`] when a change is both structural and bound:
/// a rebuild re-reads every element it builds, and sends the bindings it
/// finds changed as part of the same commit.
pub fn flush() {
	APPLICATION.with_borrow_mut(|application| {
		if let Some(application) = application.as_mut() {
			application.tree.refresh_reactive(&mut Imports);
		}
	});
}

/// Commits the mounted application after state changes outside a UI event.
///
/// Call this after polling an application executor or receiving a plugin
/// reply. Layout, responsive styles, and native text measurement stay on the
/// host. Calling it before [`mount`] has no effect.
pub fn render() {
	let view = APPLICATION.with_borrow(|application| {
		application
			.as_ref()
			.map(|application| Rc::clone(&application.view))
	});

	let Some(view) = view else {
		return;
	};
	let element = view();

	APPLICATION.with_borrow_mut(|application| {
		application
			.as_mut()
			.expect("the application remains mounted while rendering")
			.tree
			.update(element, &mut Imports);
	});
}

/// Allocates the initialized buffer used by the host for one payload event.
///
/// Forward the native ABI's `leptoskit_alloc` export here. The returned pointer
/// stays valid until the next call to [`alloc`] or [`dispatch`]. Allocation
/// failure returns zero; [`dispatch`] releases the buffer after consuming it.
pub fn alloc(length: i32) -> i32 {
	PAYLOAD.with_borrow_mut(|payload| {
		let mut next = Vec::new();

		if next.try_reserve_exact(length as u32 as usize).is_err() {
			return 0;
		}

		next.resize(length as u32 as usize, 0);
		let pointer = next.as_mut_ptr() as i32;
		*payload = next;
		pointer
	})
}

/// Dispatches one native ABI event, then renders the application again.
///
/// Forward `leptoskit_event` here. Payload pointers must identify the complete
/// buffer returned by [`alloc`]; arbitrary guest pointers are rejected without
/// dereferencing them. Returns one when a handler consumes the event, zero for
/// an unknown target, unsupported event, invalid payload, or unhandled event.
pub fn dispatch(node: i64, kind: i32, pointer: i32, length: i32) -> i32 {
	let mut payload = PAYLOAD.with_borrow_mut(std::mem::take);

	if length == 0 {
		if pointer != 0 {
			return 0;
		}

		payload.clear();
	} else if payload.as_ptr() as i32 != pointer || payload.len() != length as u32 as usize {
		return 0;
	}

	let Some(event) = decode_event(kind, &payload) else {
		return 0;
	};
	let handlers =
		APPLICATION.with_borrow(|application| application.as_ref()?.tree.handlers(node, event));

	let Some((event, handlers)) = handlers else {
		return 0;
	};
	let outcome = invoke(&event, &handlers);
	render();
	i32::from(outcome == EventOutcome::Handled)
}

/// Synchronous ABI imports; the host copies all borrowed property bytes.
struct Imports;

impl Host for Imports {
	fn create(&mut self, id: i64, kind: i32) {
		// SAFETY: This import accepts integer values and borrows no memory.
		unsafe { node_create(id, kind) };
	}

	fn props(&mut self, id: i64, bytes: &[u8]) {
		// SAFETY: The host copies this initialized slice during the call.
		unsafe { node_set_props(id, bytes.as_ptr(), bytes.len()) };
	}

	fn insert(&mut self, parent: i64, child: i64, index: i32) {
		// SAFETY: This import accepts integer values and borrows no memory.
		unsafe { node_insert(parent, child, index) };
	}

	fn delete(&mut self, id: i64) {
		// SAFETY: This import accepts an integer value and borrows no memory.
		unsafe { node_delete(id) };
	}

	fn commit(&mut self) {
		// SAFETY: This import accesses only the host's assembled tree.
		unsafe { commit() };
	}
}
