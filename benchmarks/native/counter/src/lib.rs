//! The counter screen `leptoskit new` generates, as a benchmark guest: the
//! same components, classes, and handler, built with the same guest runtime,
//! so a tap here costs what a tap costs in a new app.
#![cfg(target_arch = "wasm32")]
// The rendering ABI is a set of exported, unmangled entry points.
#![allow(unsafe_code)]

use leptoskit::prelude::Button;
use leptoskit::prelude::Element;
use leptoskit::prelude::Get;
use leptoskit::prelude::RwSignal;
use leptoskit::prelude::Set;
use leptoskit::prelude::Text;
use leptoskit::prelude::View;

fn screen(count: RwSignal<i32>) -> Element {
	View()
		.class("min-h-screen items-center justify-center gap-6 p-6 bg-slate-950 text-white")
		.child(Text("Hello, leptoskit").class("text-3xl font-bold"))
		.child(Text(move || format!("Count: {}", count.get())).class("text-xl"))
		.child(
			Button("Add one")
				.class("rounded-lg bg-indigo-500 px-6 py-3 font-semibold")
				.on_press(move || count.set(count.get().saturating_add(1))),
		)
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	let count = RwSignal::new(0);
	leptoskit::guest::mount(move || screen(count));
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	leptoskit::guest::dispatch(node, kind, ptr, len)
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_poll() {
	leptoskit::guest::poll();
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_flush() {
	leptoskit::guest::flush();
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(len: i32) -> i32 {
	leptoskit::guest::alloc(len)
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	leptoskit::guest::ABI_VERSION as i32
}
