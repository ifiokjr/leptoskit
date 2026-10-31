//! Tolerated wasm-bindgen runtime shims.
//!
//! A native guest's dependency graph can link wasm-bindgen runtime imports
//! even when the guest never calls a browser API: upstream `reactive_graph`,
//! which the file router brings into guests, keeps an `#[allow(unused)]`
//! console-warn helper behind an unconditional `wasm32-unknown-unknown`
//! dependency on web-sys, and rustc emits an import for every linked extern.
//! `leptos` itself turns on `any_spawner`'s wasm-bindgen executor and
//! `server_fn`'s browser client unconditionally, so a guest that uses its
//! signals links js-sys futures and stream glue too. Those imports are
//! reachable only through function tables and wasm-bindgen's own exports,
//! which JavaScript calls; nothing a native host calls reaches them.
//!
//! The interpreter resolves a small set of shims that do something harmless
//! (discard, report through the host, or grow the notional externref
//! table). Every other function import from wasm-bindgen's own modules links
//! to a trap that names it, because reaching one means the guest tried to do
//! browser work and the native runtime has no JavaScript to call into.
//! Imports from any other module still fail instantiation with wasmi's own
//! unresolved-import error.

use wasmi::Caller;
use wasmi::Linker;
use wasmi::Module;
use wasmi::Val;

use super::HostState;
use super::host_trap;

/// What the host does when the guest calls a shim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// What reaching any other wasm-bindgen import means.
const BROWSER_WORK: &str = "the guest called browser-only wasm-bindgen glue, which the native \
                            runtime has no JavaScript for";

/// Maps one wasm-bindgen import to its host behavior. `None` for imports
/// from other modules, which stay unresolved.
fn behavior(module: &str, name: &str) -> Option<Behavior> {
	if module == "__wbindgen_externref_xform__" {
		return Some(match name {
			"__wbindgen_externref_table_grow" => Behavior::Grow,
			"__wbindgen_externref_table_set_null" => Behavior::Discard,
			_ => Behavior::Trap(BROWSER_WORK),
		});
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
		_ => Some(Behavior::Trap(BROWSER_WORK)),
	}
}

/// Registers host implementations for the wasm-bindgen imports `module`
/// declares, keeping each import's own signature.
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
		Behavior::Trap(message) => return Err(host_trap(format!("{message}: {name}"))),
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn wasm_bindgen_imports_are_shimmed_or_trap() {
		assert_eq!(
			behavior("__wbindgen_placeholder__", "__wbg_warn_13abc63e0d4b3527"),
			Some(Behavior::Report)
		);
		assert_eq!(
			behavior("__wbindgen_placeholder__", "__wbindgen_describe"),
			Some(Behavior::Discard)
		);
		assert_eq!(
			behavior(
				"__wbindgen_placeholder__",
				"__wbg___wbindgen_throw_41e9ee4f547fc59a"
			),
			Some(Behavior::Trap(
				"the guest threw through wasm-bindgen; the panic message is in the browser-only \
				 ABI"
			))
		);
		assert!(matches!(
			behavior(
				"__wbindgen_placeholder__",
				"__wbg___wbindgen_jspi_spawn_poll_4e3f923bdfe46933"
			),
			Some(Behavior::Trap(_))
		));
		assert_eq!(
			behavior(
				"__wbindgen_externref_xform__",
				"__wbindgen_externref_table_grow"
			),
			Some(Behavior::Grow)
		);
		assert_eq!(
			behavior(
				"__wbindgen_externref_xform__",
				"__wbindgen_externref_table_set_null"
			),
			Some(Behavior::Discard)
		);

		// Every other wasm-bindgen import is browser work, and traps.
		assert_eq!(
			behavior("__wbindgen_placeholder__", "__wbg_performance_now_1"),
			Some(Behavior::Trap(BROWSER_WORK))
		);
		assert_eq!(
			behavior(
				"__wbindgen_externref_xform__",
				"__wbindgen_externref_shrink"
			),
			Some(Behavior::Trap(BROWSER_WORK))
		);

		// Other modules stay unresolved.
		assert_eq!(behavior("leptoskit", "commit"), None);
		assert_eq!(behavior("env", "performance_now"), None);
		assert_eq!(
			behavior("__wbindgen_describe__", "__wbindgen_describe"),
			None
		);
	}

	#[test]
	fn zero_fills_each_slot_with_its_own_type() {
		let zeroed = |slot: Val| {
			let restored = zero(&slot);
			restore(slot, restored)
		};
		assert!(zeroed(Val::I32(7)));
		assert!(zeroed(Val::I64(7)));
		assert!(zeroed(Val::F32(7f32.into())));
		assert!(zeroed(Val::F64(7f64.into())));
	}

	/// Wasmi's values carry no comparison, so round-trip them through their
	/// Rust types.
	fn restore(slot: Val, zeroed: Val) -> bool {
		match (slot, zeroed) {
			(Val::I32(_), Val::I32(0)) | (Val::I64(_), Val::I64(0)) => true,
			(Val::F32(_), Val::F32(zero)) => zero.to_bits() == 0,
			(Val::F64(_), Val::F64(zero)) => zero.to_bits() == 0,
			_ => false,
		}
	}
}
