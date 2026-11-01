//! Binary plugin imports, versioned separately from the renderer.
//!
//! Import module: `leptoskit_plugins_v1`.
//! `resolve(ptr,len) -> i64`: host-assigned ID or encoded error.
//! `submit(plugin,method,ptr,len) -> i64`: positive call ID or encoded error.
//! `result_len(id) -> i64`: bytes, -1 pending, or encoded error.
//! `take_result(id,ptr,capacity) -> i64`: copies and consumes; same status.
//! `cancel(id) -> i32`: 0 or encoded error.
//! `event_info(ptr) -> i32`: writes 12 LE bytes (plugin,kind,len), 1 if
//! present, 0 if empty. `take_event(ptr,capacity) -> i64`: copies/consumes.
//! Errors encode as `-(code + 1)`, preserving -1 for pending/empty.
//! Invalid memory ranges trap without consuming the response/event.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::CallId;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::PluginId;
use wasmi::Caller;
use wasmi::Linker;
use wasmi::Memory;

use crate::HostState;
use crate::caller_memory;
use crate::host_trap;

const MODULE: &str = "leptoskit_plugins_v1";

pub(super) fn link(linker: &mut Linker<HostState>) -> Result<(), wasmi::errors::LinkerError> {
	linker.func_wrap(MODULE, "version", || leptoskit_bridge::VERSION as i32)?;
	linker.func_wrap(MODULE, "resolve", resolve)?;
	linker.func_wrap(MODULE, "submit", submit)?;
	linker.func_wrap(MODULE, "result_len", result_len)?;
	linker.func_wrap(MODULE, "take_result", take_result)?;
	linker.func_wrap(MODULE, "cancel", cancel)?;
	linker.func_wrap(MODULE, "event_info", event_info)?;
	linker.func_wrap(MODULE, "take_event", take_event)?;

	Ok(())
}

fn status(error: Error) -> i64 {
	-i64::from(error as i32) - 1
}

fn memory(caller: &mut Caller<'_, HostState>) -> Result<Memory, wasmi::Error> {
	caller_memory(caller).ok_or_else(|| host_trap("plugin guest must export memory"))
}

fn resolve(mut caller: Caller<'_, HostState>, ptr: i32, len: i32) -> Result<i64, wasmi::Error> {
	// Plugin names are setup metadata; limit work even for a hostile guest.
	if len as u32 > 256 {
		return Ok(status(Error::TooLarge));
	}

	let memory = memory(&mut caller)?;
	let start = ptr as u32 as usize;
	let bytes = memory
		.data(&caller)
		.get(start..start.saturating_add(len as u32 as usize))
		.ok_or_else(|| host_trap("plugin name outside guest memory"))?;
	let Ok(name) = std::str::from_utf8(bytes) else {
		return Ok(status(Error::InvalidArgument));
	};
	Ok(caller
		.data()
		.plugins
		.resolve(name)
		.map_or_else(|| status(Error::Unavailable), |id| i64::from(id.0)))
}

fn submit(
	mut caller: Caller<'_, HostState>,
	plugin: i32,
	method: i32,
	ptr: i32,
	len: i32,
) -> Result<i64, wasmi::Error> {
	let plugin = PluginId(plugin as u32);
	let method = MethodId(method as u32);
	let len = len as u32 as usize;

	if let Err(error) = caller.data().plugins.check_call(plugin, method, len) {
		return Ok(status(error));
	}

	let memory = memory(&mut caller)?;
	let start = ptr as u32 as usize;
	let input = memory
		.data(&caller)
		.get(start..start.saturating_add(len))
		.ok_or_else(|| host_trap("plugin payload outside guest memory"))?;
	// The sole guest -> host payload copy. Ownership then moves to native code.
	let payload = Bytes::copy_from_slice(input);
	Ok(caller
		.data_mut()
		.plugins
		.submit(plugin, method, payload)
		.map_or_else(status, |id| id.0 as i64))
}

#[allow(clippy::needless_pass_by_value)] // wasmi imports own their Caller.
fn result_len(caller: Caller<'_, HostState>, id: i64) -> i64 {
	match caller.data().plugins.result(CallId(id as u64)) {
		Ok(Some(Ok(bytes))) => bytes.len() as i64,
		Ok(None) => -1,
		Ok(Some(Err(error))) => status(*error),
		Err(error) => status(error),
	}
}

fn take_result(
	mut caller: Caller<'_, HostState>,
	id: i64,
	ptr: i32,
	capacity: i32,
) -> Result<i64, wasmi::Error> {
	let id = CallId(id as u64);
	let memory = memory(&mut caller)?;
	// Split the interpreter's memory and host state so output can borrow the
	// original Bytes without cloning a handle or promoting its shared owner.
	let (memory, state) = memory.data_and_store_mut(&mut caller);

	let bytes = match state.plugins.result(id) {
		Ok(Some(Ok(bytes))) => bytes,
		Ok(None) => return Ok(-1),
		Ok(Some(Err(error))) => {
			let code = status(*error);
			let _ = state.plugins.take_result(id);
			return Ok(code);
		}
		Err(error) => return Ok(status(error)),
	};

	if bytes.len() > capacity as u32 as usize {
		return Ok(status(Error::TooLarge));
	}

	let len = bytes.len();
	let start = ptr as u32 as usize;
	let output = memory
		.get_mut(start..start.saturating_add(len))
		.ok_or_else(|| host_trap("plugin response destination outside guest memory"))?;
	output.copy_from_slice(bytes);
	state
		.plugins
		.take_result(id)
		.map_err(|error| host_trap(error.to_string()))?;
	Ok(len as i64)
}

fn cancel(mut caller: Caller<'_, HostState>, id: i64) -> i32 {
	caller
		.data_mut()
		.plugins
		.cancel(CallId(id as u64))
		.map_or_else(|error| status(error) as i32, |()| 0)
}

fn event_info(mut caller: Caller<'_, HostState>, ptr: i32) -> Result<i32, wasmi::Error> {
	let Some(event) = caller.data().plugins.event() else {
		return Ok(0);
	};
	let mut info = [0_u8; 12];
	info[..4].copy_from_slice(&event.plugin.0.to_le_bytes());
	info[4..8].copy_from_slice(&event.kind.to_le_bytes());
	info[8..].copy_from_slice(&(event.payload.len() as u32).to_le_bytes());
	let memory = memory(&mut caller)?;
	memory.write(&mut caller, ptr as u32 as usize, &info)?;

	Ok(1)
}

fn take_event(
	mut caller: Caller<'_, HostState>,
	ptr: i32,
	capacity: i32,
) -> Result<i64, wasmi::Error> {
	let memory = memory(&mut caller)?;
	let (memory, state) = memory.data_and_store_mut(&mut caller);
	let Some(event) = state.plugins.event() else {
		return Ok(-1);
	};

	if event.payload.len() > capacity as u32 as usize {
		return Ok(status(Error::TooLarge));
	}

	let len = event.payload.len();
	let start = ptr as u32 as usize;
	let output = memory
		.get_mut(start..start.saturating_add(len))
		.ok_or_else(|| host_trap("plugin event destination outside guest memory"))?;
	output.copy_from_slice(&event.payload);
	state.plugins.next_event();
	Ok(len as i64)
}
