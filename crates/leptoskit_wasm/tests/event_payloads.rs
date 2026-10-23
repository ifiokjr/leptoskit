//! Native events reach the guest with their exact typed payload bytes.

use std::fmt::Write;

use leptoskit_native::EventOutcome;
use leptoskit_native::NativeEvent;
use leptoskit_wasm::EventKind;
use leptoskit_wasm::WasmHost;

const TEXT_INPUT: i32 = 5;
const TARGET: i64 = 42;
const WIDTH: f32 = 320.0;
const HEIGHT: f32 = 240.0;

fn wat_bytes(value: &str) -> String {
	let mut escaped = String::new();

	for byte in value.bytes() {
		write!(escaped, "\\{byte:02x}").unwrap();
	}

	escaped
}

/// The guest only handles the exact target, event code, and typed JSON bytes.
/// It commits an acknowledgement while retaining the control's owned value.
fn control_guest(kind: i32, props: &str, event: EventKind, payload: &str) -> String {
	let accepted = r#"{"text":"accepted"}"#;
	format!(
		r#"(module
(import "leptoskit" "node_create" (func $create (param i64 i32)))
(import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
(import "leptoskit" "node_insert" (func $insert (param i64 i64 i32)))
(import "leptoskit" "commit" (func $commit))
(memory (export "memory") 1)
(data (i32.const 256) "{props_bytes}")
(data (i32.const 1024) "{payload_bytes}")
(data (i32.const 2048) "{accepted_bytes}")
(func (export "leptoskit_alloc") (param i32) (result i32) (i32.const 4096))
(func (export "leptoskit_main")
(call $create (i64.const 1) (i32.const 0))
(call $create (i64.const 42) (i32.const {kind}))
(call $create (i64.const 43) (i32.const 1))
(call $props (i64.const 42) (i32.const 256) (i32.const {props_len}))
(call $insert (i64.const 1) (i64.const 42) (i32.const 0))
(call $insert (i64.const 1) (i64.const 43) (i32.const 1))
(call $commit))
(func (export "leptoskit_event")
(param $id i64) (param $kind i32) (param $ptr i32) (param $len i32) (result i32)
(local $index i32)
(if (i32.or (i64.ne (local.get $id) (i64.const 42))
(i32.ne (local.get $kind) (i32.const {event_code}))) (then (return (i32.const 0))))
(if (i32.ne (local.get $len) (i32.const {payload_len})) (then (return (i32.const 0))))
(block $done (loop $compare
(br_if $done (i32.eq (local.get $index) (local.get $len)))
(if (i32.ne
(i32.load8_u (i32.add (local.get $ptr) (local.get $index)))
(i32.load8_u (i32.add (i32.const 1024) (local.get $index))))
(then (return (i32.const 0))))
(local.set $index (i32.add (local.get $index) (i32.const 1)))
(br $compare)))
(call $props (i64.const 43) (i32.const 2048) (i32.const {accepted_len}))
(call $commit)
(i32.const 1)))"#,
		props_bytes = wat_bytes(props),
		payload_bytes = wat_bytes(payload),
		accepted_bytes = wat_bytes(accepted),
		props_len = props.len(),
		payload_len = payload.len(),
		event_code = event.code(),
		accepted_len = accepted.len(),
	)
}

#[test]
fn text_scroll_and_blur_events_deliver_exact_payloads_to_the_guest() {
	for (event, kind, payload) in [
		(NativeEvent::Blur, EventKind::Blur, ""),
		(
			NativeEvent::Scroll {
				offset_x: -12.5,
				offset_y: 32.0,
			},
			EventKind::Scroll,
			r#"{"x":-12.5,"y":32}"#,
		),
		(
			NativeEvent::Change {
				value: "quote\"\n\\雪".to_owned(),
			},
			EventKind::Change,
			r#"{"value":"quote\"\n\\雪"}"#,
		),
		(
			NativeEvent::Submit {
				value: String::new(),
			},
			EventKind::Submit,
			r#"{"value":""}"#,
		),
	] {
		let guest = control_guest(TEXT_INPUT, r#"{"placeholder":"Name"}"#, kind, payload);
		let mut host = WasmHost::headless(guest.as_bytes()).unwrap();
		host.start(WIDTH, HEIGHT).unwrap();
		let root = host.surface().root().unwrap();
		let target = host.surface().children(root)[0];

		assert_eq!(
			host.dispatch_wire(TARGET, kind, Some("wrong payload"))
				.unwrap(),
			EventOutcome::Unhandled
		);
		assert_eq!(host.commits(), 1);
		assert_eq!(
			host.dispatch(target, &event).unwrap(),
			EventOutcome::Handled
		);
		assert_eq!(host.commits(), 2);
		let status = host.surface().children(root)[1];
		assert_eq!(
			host.surface().props(status).text.as_deref(),
			Some("accepted")
		);
	}
}
