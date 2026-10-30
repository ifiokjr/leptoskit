//! Compiled guest fixture running `leptoskit::hooks` inside the native
//! interpreter.
//!
//! Hooks bring `reactive_graph`, whose `wasm32-unknown-unknown` build links
//! wasm-bindgen runtime imports, into the guest. The interpreter must still
//! instantiate the module, and signals, memos, and synchronous watchers must
//! drive the tree the host renders.
#![cfg(target_arch = "wasm32")]
#![allow(unsafe_code)] // Exported C names form the interpreter's entry points.

use leptoskit::components::Button;
use leptoskit::components::Text;
use leptoskit::components::View;
use leptoskit::hooks::use_not;
use leptoskit::hooks::use_toggle;
use leptoskit::hooks::watch_with_options;
use leptoskit::view;
use reactive_graph::computed::Memo;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;
use reactive_graph::signal::RwSignal;

/// Mounts a switch whose label, memoized summary, and change count all come
/// from hooks.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	// The owner scopes the hooks; the mounted view holds it for the app's
	// lifetime.
	let owner = Owner::new();
	let (switch, label, changes) = owner.with(|| {
		let switch = use_toggle(false);
		let off = use_not(switch.value);
		let label = Memo::new(move |_| if off.get() { "Off" } else { "On" });
		let changes = RwSignal::new(0_u32);
		watch_with_options(
			move || switch.value.get(),
			move |_, _| changes.update(|count| *count += 1),
			false,
		);
		(switch, label, changes)
	});
	leptoskit::guest::mount(move || {
		let _scope = &owner;
		let toggle = switch.toggle.clone();
		view! {
			<View>
				<Button on:press=move || toggle()>{label.get()}</Button>
				<Text>"Changes: " {changes.get()}</Text>
			</View>
		}
	});
}

/// Forwards platform-neutral events to the shared tree's handlers.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	leptoskit::guest::dispatch(node, kind, ptr, len)
}

/// Allocates the interpreter's event payload buffer.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(len: i32) -> i32 {
	leptoskit::guest::alloc(len)
}

/// Reports the native UI bridge ABI this guest implements.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	leptoskit::guest::ABI_VERSION as i32
}
