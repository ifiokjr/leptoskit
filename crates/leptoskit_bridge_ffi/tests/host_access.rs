//! Scoped access drives the application's real interpreter and renderer.
#![cfg(feature = "app")]

use leptoskit_bridge_ffi::app::App;
use leptoskit_native::Mutation;
use leptoskit_wasm::WasmHost;

const VIEWPORT: (f32, f32) = (320.0, 480.0);

const GUEST: &[u8] = br#"(module
 (import "leptoskit" "node_create" (func $create (param i64 i32)))
 (import "leptoskit" "commit" (func $commit))
 (memory (export "memory") 1)
 (func (export "leptoskit_main")
   (call $create (i64.const 1) (i32.const 0)) (call $commit))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0))"#;

#[test]
fn scoped_access_drives_the_same_host_after_app_move() {
	let mut app = App::new(GUEST).unwrap();
	let identity = app.with_host(|host| host.identity());
	let mut moved = Box::new(app);

	moved
		.with_host(|host| host.start(VIEWPORT.0, VIEWPORT.1))
		.unwrap();

	assert_eq!(identity, moved.with_host(|host| host.identity()));
	assert_eq!(moved.with_host(|host| host.commits()), 1);

	let mutations = moved.with_host(WasmHost::take_mutations);
	let created = mutations
		.iter()
		.filter(|mutation| matches!(mutation, Mutation::Create { .. }))
		.count();

	assert_eq!(created, 1);
}
