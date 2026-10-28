//! Rendering imports preserve the versioned guest ABI and tree invariants.

use leptoskit_native::NativeKind;
use wasmi::Caller;
use wasmi::Linker;

use super::HostState;
use super::MAX_TREE_DEPTH;
use super::WireNode;
use super::caller_memory;
use super::host_trap;

const NATIVE_KINDS: &[NativeKind] = &[
	NativeKind::View,
	NativeKind::Text,
	NativeKind::TextRun,
	NativeKind::Image,
	NativeKind::ScrollView,
	NativeKind::TextInput,
	NativeKind::Button,
	NativeKind::Switch,
	NativeKind::Checkbox,
	NativeKind::Radio,
	NativeKind::Slider,
	NativeKind::LinearProgressIndicator,
	NativeKind::ActivityIndicator,
	NativeKind::PasswordInput,
	NativeKind::Pressable,
];

/// The import modules wasm-bindgen leaves in a module it has not processed.
const WASM_BINDGEN_MODULES: [&str; 2] =
	["__wbindgen_placeholder__", "__wbindgen_externref_xform__"];

/// Links every function a guest imports from wasm-bindgen's placeholder
/// modules to a stub that traps when called.
///
/// A guest can link a browser binding without ever calling it: a crate that
/// warns through `console.warn` only when it is misused, say. Those imports
/// would otherwise keep the whole module from loading; with the stubs it
/// loads, and a guest that does call one stops with a trap naming it.
pub(super) fn link_browser_stubs(
	linker: &mut Linker<HostState>,
	module: &wasmi::Module,
) -> Result<(), wasmi::errors::LinkerError> {
	for import in module.imports() {
		let wasmi::ExternType::Func(ty) = import.ty() else {
			continue;
		};

		if !WASM_BINDGEN_MODULES.contains(&import.module()) {
			continue;
		}

		let name = import.name().to_owned();
		linker.func_new(
			import.module(),
			import.name(),
			ty.clone(),
			move |_, _, _| {
				Err(wasmi::Error::new(format!(
					"the guest called the browser binding `{name}`, which native hosts do not \
					 provide"
				)))
			},
		)?;
	}

	Ok(())
}

pub(super) fn link(linker: &mut Linker<HostState>) -> Result<(), wasmi::errors::LinkerError> {
	linker.func_wrap("leptoskit", "node_create", node_create)?;
	linker.func_wrap("leptoskit", "node_set_props", node_set_props)?;
	linker.func_wrap("leptoskit", "node_insert", node_insert)?;
	linker.func_wrap("leptoskit", "node_remove", node_remove)?;
	linker.func_wrap("leptoskit", "node_delete", node_delete)?;
	linker.func_wrap("leptoskit", "commit", commit)?;
	Ok(())
}

fn node_create(mut caller: Caller<'_, HostState>, id: i64, kind: i32) {
	let kind = NATIVE_KINDS
		.get(kind as u32 as usize)
		.copied()
		.unwrap_or(NativeKind::View);
	caller.data_mut().nodes.entry(id).or_default();
	caller.data_mut().kinds.insert(id, kind);
}

fn node_set_props(
	mut caller: Caller<'_, HostState>,
	id: i64,
	ptr: i32,
	len: i32,
) -> Result<(), wasmi::Error> {
	let bytes = read_memory(&mut caller, ptr, len)?;
	let wire: WireNode =
		serde_json::from_slice(&bytes).map_err(|error| host_trap(error.to_string()))?;
	caller.data_mut().nodes.insert(id, wire);
	Ok(())
}

fn node_insert(
	mut caller: Caller<'_, HostState>,
	parent: i64,
	child: i64,
	index: i32,
) -> Result<(), wasmi::Error> {
	let state = caller.data_mut();
	// Walk up from the parent: finding the child means the insert would close
	// a cycle. The walk is bounded, so a hostile chain cannot stall the host.
	let mut ancestor = Some(parent);
	let mut depth = 0usize;

	while let Some(id) = ancestor {
		if id == child {
			return Err(host_trap(format!(
				"node_insert({parent}, {child}) would make node {child} its own ancestor"
			)));
		}

		depth += 1;

		if depth >= MAX_TREE_DEPTH {
			return Err(host_trap(format!(
				"node_insert({parent}, {child}) would nest deeper than {MAX_TREE_DEPTH} levels"
			)));
		}

		ancestor = state.parents.get(&id).copied();
	}

	state.detach(child);
	state.nodes.entry(parent).or_default();
	state.nodes.entry(child).or_default();
	let siblings = state.children.entry(parent).or_default();
	let index = (index as u32 as usize).min(siblings.len());
	siblings.insert(index, child);
	state.parents.insert(child, parent);
	Ok(())
}

fn node_remove(mut caller: Caller<'_, HostState>, parent: i64, child: i64) {
	let state = caller.data_mut();

	if state.parents.get(&child) == Some(&parent) {
		state.detach(child);
	}
}

fn node_delete(mut caller: Caller<'_, HostState>, id: i64) {
	let state = caller.data_mut();
	state.detach(id);

	for child in state.children.remove(&id).into_iter().flatten() {
		state.parents.remove(&child);
	}

	state.nodes.remove(&id);
	state.kinds.remove(&id);
}

fn commit(mut caller: Caller<'_, HostState>) -> Result<(), wasmi::Error> {
	let state = caller.data();
	let parentless: Vec<i64> = state
		.nodes
		.keys()
		.copied()
		.filter(|id| !state.parents.contains_key(id))
		.collect();

	let root = match parentless.as_slice() {
		[] => return Ok(()),
		[root] => *root,
		many => return Err(excess_roots(many)),
	};

	let pending = state.assemble(root)?;
	let state = caller.data_mut();
	state.pending = Some(pending);
	state.commits += 1;
	Ok(())
}

fn excess_roots(roots: &[i64]) -> wasmi::Error {
	let shown: Vec<String> = roots.iter().take(8).map(i64::to_string).collect();
	let more = if roots.len() > shown.len() {
		", …"
	} else {
		""
	};

	host_trap(format!(
		"commit found {} parentless nodes ({}{more}); exactly one root may be committed, so \
		 attach or delete the others first",
		roots.len(),
		shown.join(", ")
	))
}

/// Copies `len` bytes at `ptr` out of the module's memory, trapping when the
/// range is out of bounds. Both values are unsigned.
fn read_memory(
	caller: &mut Caller<'_, HostState>,
	ptr: i32,
	len: i32,
) -> Result<Vec<u8>, wasmi::Error> {
	let start = ptr as u32 as usize;
	let len = len as u32 as usize;

	if len == 0 {
		return Ok(Vec::new());
	}

	let memory =
		caller_memory(caller).ok_or_else(|| host_trap("the module does not export memory"))?;
	start
		.checked_add(len)
		.and_then(|end| memory.data(&*caller).get(start..end))
		.map(<[u8]>::to_vec)
		.ok_or_else(|| host_trap(format!("props range {start}+{len} is out of bounds")))
}
