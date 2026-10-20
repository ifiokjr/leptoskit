use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::Event;
use leptoskit_bridge::MethodId;
use leptoskit_native::HeadlessBackend;
use leptoskit_wasm::WasmHost;

fn host(main: &str, ready: &str) -> WasmHost<HeadlessBackend> {
	let module = format!(
		r#"(module
			(import "leptoskit_plugins_v1" "submit" (func $submit (param i32 i32 i32 i32) (result i64)))
			(import "leptoskit_plugins_v1" "result_len" (func $len (param i64) (result i64)))
			(import "leptoskit_plugins_v1" "take_result" (func $take (param i64 i32 i32) (result i64)))
			(import "leptoskit_plugins_v1" "event_info" (func $info (param i32) (result i32)))
			(import "leptoskit_plugins_v1" "take_event" (func $event (param i32 i32) (result i64)))
			(memory (export "memory") 1)
			(data (i32.const 0) "hello")
			(global $id (mut i64) (i64.const 0))
			(func (export "leptoskit_main") {main})
			(func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
			(func (export "leptoskit_plugins_ready") {ready})
		)"#
	);
	WasmHost::headless(module.as_bytes()).unwrap()
}

#[test]
fn binary_call_round_trip_preserves_bytes_and_consumes_once() {
	let mut host = host(
		"(global.set $id (call $submit (i32.const 0) (i32.const 7) (i32.const 0) (i32.const 5)))",
		r"
		(if (i64.ne (call $len (global.get $id)) (i64.const 5)) (then unreachable))
		(if (i64.ne (call $take (global.get $id) (i32.const 16) (i32.const 4)) (i64.const -6))
			(then unreachable))
		(if (i64.ne (call $take (global.get $id) (i32.const 16) (i32.const 5)) (i64.const 5))
			(then unreachable))
		(if (i32.ne (i32.load8_u (i32.const 16)) (i32.const 104)) (then unreachable))
		(if (i64.ne (call $len (global.get $id)) (i64.const -9)) (then unreachable))
		",
	);
	let plugin = host.plugins().register("echo", [MethodId(7)]).unwrap();
	host.plugins().grant(plugin, MethodId(7), true).unwrap();
	host.start(100.0, 100.0).unwrap();
	let request = host.plugins().next_request().unwrap();
	assert_eq!(request.payload.as_ref(), b"hello");
	host.plugins()
		.complete(request.id, Ok(request.payload))
		.unwrap();
	host.wake_plugins().unwrap();
	assert_eq!(host.plugins().buffered_bytes(), 0);
}

#[test]
fn denied_calls_do_not_read_guest_memory_or_queue_work() {
	let mut host = host(
		r"(if (i64.ne
			(call $submit (i32.const 0) (i32.const 7) (i32.const -1) (i32.const 5))
			(i64.const -3)) (then unreachable))",
		"",
	);
	host.plugins().register("echo", [MethodId(7)]).unwrap();
	host.start(100.0, 100.0).unwrap();
	assert!(host.plugins().next_request().is_none());
}

#[test]
fn invalid_guest_pointer_traps_before_allocating_a_request() {
	let mut host = host(
		"(drop (call $submit (i32.const 0) (i32.const 7) (i32.const 65534) (i32.const 5)))",
		"",
	);
	let plugin = host.plugins().register("echo", [MethodId(7)]).unwrap();
	host.plugins().grant(plugin, MethodId(7), true).unwrap();
	assert!(host.start(100.0, 100.0).is_err());
	assert!(host.plugins().next_request().is_none());
	assert_eq!(host.plugins().buffered_bytes(), 0);
}

#[test]
fn invalid_destination_does_not_lose_reply() {
	let mut host = host(
		"(global.set $id (call $submit (i32.const 0) (i32.const 7) (i32.const 0) (i32.const 0)))",
		"(drop (call $take (global.get $id) (i32.const 65535) (i32.const 5)))",
	);
	let plugin = host.plugins().register("echo", [MethodId(7)]).unwrap();
	host.plugins().grant(plugin, MethodId(7), true).unwrap();
	host.start(100.0, 100.0).unwrap();
	let request = host.plugins().next_request().unwrap();
	host.plugins()
		.complete(request.id, Ok(Bytes::from_static(b"reply")))
		.unwrap();
	assert!(host.wake_plugins().is_err());
	assert_eq!(
		host.plugins().take_result(request.id).unwrap(),
		Some(Ok(Bytes::from_static(b"reply")))
	);
}

#[test]
fn native_failures_and_events_reach_the_guest() {
	let mut host = host(
		"(global.set $id (call $submit (i32.const 0) (i32.const 7) (i32.const 0) (i32.const 0)))",
		r"
		(if (i64.ne (call $take (global.get $id) (i32.const 0) (i32.const 0)) (i64.const -7))
			(then unreachable))
		(if (i32.ne (call $info (i32.const 32)) (i32.const 1)) (then unreachable))
		(if (i32.ne (i32.load (i32.const 36)) (i32.const 42)) (then unreachable))
		(if (i32.ne (i32.load (i32.const 40)) (i32.const 3)) (then unreachable))
		(if (i64.ne (call $event (i32.const 48) (i32.const 3)) (i64.const 3)) (then unreachable))
		(if (i32.ne (i32.load8_u (i32.const 48)) (i32.const 97)) (then unreachable))
		(if (i32.ne (call $info (i32.const 32)) (i32.const 0)) (then unreachable))
		",
	);
	let plugin = host.plugins().register("echo", [MethodId(7)]).unwrap();
	host.plugins().grant(plugin, MethodId(7), true).unwrap();
	host.start(100.0, 100.0).unwrap();
	let request = host.plugins().next_request().unwrap();
	host.plugins()
		.complete(request.id, Err(Error::Cancelled))
		.unwrap();
	host.plugins()
		.emit(Event {
			plugin,
			kind: 42,
			payload: Bytes::from_static(b"abc"),
		})
		.unwrap();
	host.wake_plugins().unwrap();
}

#[test]
#[ignore = "run test:plugins:guest to compile and load the Rust wasm fixture"]
fn compiled_rust_guest_handles_async_calls_events_and_cancellation() {
	let path = std::env::var("LEPTOSKIT_PLUGIN_GUEST").expect("fixture wasm path");
	let mut host = WasmHost::headless(&std::fs::read(path).unwrap()).unwrap();
	let plugin = host
		.plugins()
		.register("echo", [MethodId(0), MethodId(1), MethodId(2)])
		.unwrap();
	for method in [MethodId(0), MethodId(2)] {
		host.plugins().grant(plugin, method, true).unwrap();
	}
	host.start(100.0, 100.0).unwrap();
	let request = host.plugins().next_request().unwrap();
	assert_eq!(request.payload.as_ref(), b"echo");
	assert!(
		host.plugins().next_request().is_none(),
		"cancelled request leaked"
	);
	host.plugins()
		.complete(request.id, Ok(request.payload))
		.unwrap();
	host.wake_plugins().unwrap();
	host.dispatch_wire(0, leptoskit_wasm::EventKind::Press, None)
		.unwrap();
	host.plugins()
		.emit(Event {
			plugin,
			kind: 42,
			payload: Bytes::from_static(b"native event"),
		})
		.unwrap();
	host.wake_plugins().unwrap();
	host.dispatch_wire(0, leptoskit_wasm::EventKind::Press, None)
		.unwrap();
	let request = host.plugins().next_request().unwrap();
	assert_eq!(request.method, MethodId(2));
	assert_eq!(request.payload.as_ref(), b"passed");
	host.plugins()
		.complete(request.id, Ok(Bytes::new()))
		.unwrap();
	host.wake_plugins().unwrap();
	host.dispatch_wire(0, leptoskit_wasm::EventKind::Press, None)
		.unwrap();
	assert_eq!(host.plugins().buffered_bytes(), 0);
}
