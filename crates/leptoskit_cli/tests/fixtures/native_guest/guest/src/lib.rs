//! The smallest compilable guest: `native bundle` builds it for
//! wasm32-unknown-unknown and optimizes the module.

#[no_mangle]
pub extern "C" fn pushed_version() -> u32 {
	2
}
