//! Interpreter and transport ownership have independent public identities.

use leptoskit_wasm::WasmHost;

const GUEST: &[u8] = br#"(module
 (memory (export "memory") 1)
 (func (export "leptoskit_main"))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0))"#;

#[test]
fn interpreter_identity_survives_moves_and_outlives_guest_state() {
	let host = WasmHost::headless(GUEST).unwrap();
	let identity = host.identity();
	let moved = Box::new(host);

	assert_eq!(identity, moved.identity());
	assert_eq!(identity, identity.clone());

	drop(moved);

	for _ in 0..16 {
		assert_ne!(identity, WasmHost::headless(GUEST).unwrap().identity());
	}
}

#[test]
fn transplant_moves_only_transport_identity() {
	let mut first = WasmHost::headless(GUEST).unwrap();
	let mut second = WasmHost::headless(GUEST).unwrap();
	let first_host = first.identity();
	let second_host = second.identity();
	let first_bridge = first.plugins().identity();
	let second_bridge = second.plugins().identity();

	std::mem::swap(first.plugins(), second.plugins());

	assert_ne!(first_host, second_host);
	assert_eq!(first_host, first.identity());
	assert_eq!(second_host, second.identity());
	assert_eq!(first_bridge, second.plugins().identity());
	assert_eq!(second_bridge, first.plugins().identity());
}
