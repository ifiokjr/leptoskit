//! A compiled guest that uses `leptoskit::hooks` must instantiate and render.
//!
//! Hooks link `reactive_graph`, which links wasm-bindgen runtime imports on
//! `wasm32-unknown-unknown`. A dependency bump that adds an import outside
//! the interpreter's tolerated shim set makes `WasmHost::headless` fail here,
//! naming the import, instead of on a device.

use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_wasm::WasmHost;

#[test]
#[ignore = "run test:plugins:guest to compile and load the Rust wasm fixture"]
fn compiled_hooks_guest_instantiates_and_renders_reactive_state() {
	let path = std::env::var("LEPTOSKIT_HOOKS_GUEST").expect("fixture wasm path");
	let mut host = WasmHost::headless(&std::fs::read(path).expect("fixture wasm"))
		.expect("a guest linking reactive_graph instantiates");
	host.start(320.0, 480.0).expect("the hooks guest mounts");
	assert_eq!(texts(&host), ["Off", "Changes: 0"]);

	press(&mut host);
	assert_eq!(texts(&host), ["On", "Changes: 1"]);

	press(&mut host);
	assert_eq!(texts(&host), ["Off", "Changes: 2"]);
}

/// The text of each child of the root, in order.
fn texts(host: &WasmHost<HeadlessBackend>) -> Vec<String> {
	let surface = host.surface();
	let root = surface.root().expect("a root node");
	surface
		.children(root)
		.iter()
		.map(|child| surface.props(*child).text.clone().unwrap_or_default())
		.collect()
}

/// Presses the switch the guest mounts first.
fn press(host: &mut WasmHost<HeadlessBackend>) {
	let surface = host.surface();
	let root = surface.root().expect("a root node");
	let button = surface.children(root)[0];
	assert_eq!(surface.kind(button), NativeKind::Button);
	host.dispatch(button, &NativeEvent::Press)
		.expect("the press reaches the guest");
}
