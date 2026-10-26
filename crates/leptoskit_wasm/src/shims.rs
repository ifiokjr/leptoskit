//! Tolerated wasm-bindgen runtime shims.
//!
//! A native guest's dependency graph can link wasm-bindgen runtime imports
//! even when the guest never calls a browser API: upstream `reactive_graph`,
//! which the file router brings into guests, keeps an `#[allow(unused)]`
//! console-warn helper behind an unconditional `wasm32-unknown-unknown`
//! dependency on web-sys, and rustc emits an import for every linked extern.
//! The interpreter resolves this closed set of shims so instantiation
//! succeeds; shims that would only run while doing real browser work either
//! report through the host or trap loudly, because the native runtime has no
//! JavaScript to call into. Any import outside the set still fails
//! instantiation with wasmi's own unresolved-import error.

use wasmi::Caller;
use wasmi::Linker;
use wasmi::Module;
use wasmi::Val;

use super::HostState;
use super::host_trap;

/// What the host does when the guest calls a shim.
#[derive(Clone, Copy)]
enum Behavior {
	/// Records nothing: the shim's effect exists only in a browser.
	Discard,
	/// Reports the call through the host log; the browser value behind the
	/// argument is an opaque table index the native runtime never populated.
	Report,
	/// Grows the notional externref table and returns its previous length.
	Grow,
	/// Traps: reaching this shim means the guest tried to do browser work.
	Trap(&'static str),
}

/// Maps one wasm-bindgen import to its host behavior, if it is tolerated.
fn behavior(module: &str, name: &str) -> Option<Behavior> {
	if module == "__wbindgen_externref_xform__" {
		return match name {
			"__wbindgen_externref_table_grow" => Some(Behavior::Grow),
			"__wbindgen_externref_table_set_null" => Some(Behavior::Discard),
			_ => None,
		};
	}
	if module != "__wbindgen_placeholder__" {
		return None;
	}
	match name {
		"__wbindgen_describe"
		| "__wbindgen_describe_generic_import"
		| "__wbindgen_object_drop_ref" => Some(Behavior::Discard),
		_ if name.starts_with("__wbg_warn_") => Some(Behavior::Report),
		_ if name.starts_with("__wbg___wbindgen_throw") => {
			Some(Behavior::Trap(
				"the guest threw through wasm-bindgen; the panic message is in the browser-only \
				 ABI",
			))
		}
		_ if name.starts_with("__wbg___wbindgen_jspi_spawn_poll") => {
			Some(Behavior::Trap(
				"wasm-bindgen JSPI futures are unavailable in the native runtime",
			))
		}
		_ => None,
	}
}

/// Registers host implementations for the wasm-bindgen imports `module`
/// declares that [`behavior`] tolerates, keeping each import's own signature.
pub(super) fn link(
	linker: &mut Linker<HostState>,
	module: &Module,
) -> Result<(), wasmi::errors::LinkerError> {
	for import in module.imports() {
		let function = match import.ty().func() {
			Some(function) => function.clone(),
			None => continue,
		};
		let module_name = import.module().to_owned();
		let name = import.name().to_owned();
		let Some(behavior) = behavior(&module_name, &name) else {
			continue;
		};
		let shim_name = name.clone();
		linker.func_new(&module_name, &name, function, move |caller, _, results| {
			shim(caller, behavior, &shim_name, results)
		})?;
	}
	Ok(())
}

/// Runs one shim call, filling result slots with each type's zero value.
fn shim(
	mut caller: Caller<'_, HostState>,
	behavior: Behavior,
	name: &str,
	results: &mut [Val],
) -> Result<(), wasmi::Error> {
	match behavior {
		Behavior::Discard => {}
		Behavior::Report => {
			eprintln!("guest called {name}; the value lives in the browser-only ABI");
		}
		Behavior::Grow => {
			let previous = caller.data().externref_table;
			caller.data_mut().externref_table = previous.saturating_add(1);
			if let Some(slot) = results.first_mut() {
				*slot = Val::I32(previous);
			}
		}
		Behavior::Trap(message) => return Err(host_trap(message)),
	}
	for (index, slot) in results.iter_mut().enumerate() {
		if index == 0 && matches!(behavior, Behavior::Grow) {
			continue;
		}
		*slot = zero(slot);
	}
	Ok(())
}

/// The zero value of one result slot's own type.
fn zero(slot: &Val) -> Val {
	match slot {
		Val::I64(_) => Val::I64(0),
		Val::F32(_) => Val::F32(0f32.into()),
		Val::F64(_) => Val::F64(0f64.into()),
		_ => Val::I32(0),
	}
}
