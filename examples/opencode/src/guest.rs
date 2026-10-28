//! The native host's entry points into the app, when it runs as a wasm
//! guest. The host finds them by name, so they are unmangled exports.
#![allow(unsafe_code)]

/// Mounts the interface in the native host's wasm runtime.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	let navigation = crate::routes::Navigation::new(crate::routes::Route::Home);
	leptoskit::guest::mount(move || crate::screen(&navigation));
}

/// Forwards platform-neutral events to the tree's handlers.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	leptoskit::guest::dispatch(node, kind, ptr, len)
}

/// Allocates the native runtime's event payload buffer.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(len: i32) -> i32 {
	leptoskit::guest::alloc(len)
}

/// Records the host's viewport, which sizes the palette's list.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_viewport(width: f32, height: f32) {
	crate::Store::current().resize((width / 8.0) as u16, (height / 16.0) as u16);
	leptoskit::guest::set_viewport(width, height);
}

/// Reports the native UI bridge ABI implemented by this app.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	leptoskit::guest::ABI_VERSION as i32
}
