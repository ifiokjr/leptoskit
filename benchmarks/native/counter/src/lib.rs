//! The counter screen `leptoskit new` generates, as a benchmark guest: the
//! same components, classes, and handler, built with the same guest runtime,
//! so a tap here costs what a tap costs in a new app.
#![cfg(target_arch = "wasm32")]
// The rendering ABI is a set of exported, unmangled entry points.
#![allow(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::Ordering;

use leptoskit::prelude::Button;
use leptoskit::prelude::Element;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;

fn screen(count: &Arc<AtomicI32>) -> Element {
	let pressed = Arc::clone(count);

	View()
		.class("min-h-screen items-center justify-center gap-6 p-6 bg-slate-950 text-white")
		.child(Text("Hello, leptoskit").class("text-3xl font-bold"))
		.child(Text(format!("Count: {}", count.load(Ordering::Relaxed))).class("text-xl"))
		.child(
			Button("Add one")
				.class("rounded-lg bg-indigo-500 px-6 py-3 font-semibold")
				.on_press(move || {
					pressed.fetch_add(1, Ordering::Relaxed);
				}),
		)
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	let count = Arc::new(AtomicI32::new(0));
	leptoskit::guest::mount(move || screen(&count));
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	leptoskit::guest::dispatch(node, kind, ptr, len)
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(len: i32) -> i32 {
	leptoskit::guest::alloc(len)
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	leptoskit::guest::ABI_VERSION as i32
}
