//! The optional `leptoskit_clock` export: a guest has no clock of its own,
//! so the host tells it the time before every entry point.

use leptoskit_wasm::WasmError;
use leptoskit_wasm::WasmHost;

/// Counts the host's reports. `main` traps unless the time was reported
/// first, and an event is handled only when it is the second report: one
/// before `main`, and one before the event itself.
const CLOCK_WAT: &str = r#"
(module
  (global $reports (mut i32) (i32.const 0))
  (func (export "leptoskit_clock") (param i64)
    (global.set $reports (i32.add (global.get $reports) (i32.const 1))))
  (func (export "leptoskit_main")
    (if (i32.eqz (global.get $reports)) (then unreachable)))
  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32)
    (i32.eq (global.get $reports) (i32.const 2))))
"#;

#[test]
fn the_host_reports_the_time_before_every_entry_point() {
	let mut host = WasmHost::headless(CLOCK_WAT.as_bytes()).expect("compile wat");
	host.start(320.0, 480.0)
		.expect("main ran after the clock was reported");

	assert_eq!(
		host.dispatch_wire(1, leptoskit_wasm::EventKind::Press, None)
			.expect("first event"),
		leptoskit_native::EventOutcome::Handled
	);
	// A third report now: the event sees it and answers unhandled.
	assert_eq!(
		host.dispatch_wire(1, leptoskit_wasm::EventKind::Press, None)
			.expect("second event"),
		leptoskit_native::EventOutcome::Unhandled
	);
}

#[test]
fn an_event_sees_a_current_wall_clock() {
	// Answers `1` only when the reported time is after 2020.
	let guest = r#"
(module
  (global $now (mut i64) (i64.const 0))
  (func (export "leptoskit_clock") (param i64) (global.set $now (local.get 0)))
  (func (export "leptoskit_main"))
  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32)
    (i64.ge_s (global.get $now) (i64.const 1600000000000))))
"#;
	let mut host = WasmHost::headless(guest.as_bytes()).expect("compile wat");
	host.start(320.0, 480.0).expect("start");

	assert_eq!(
		host.dispatch_wire(1, leptoskit_wasm::EventKind::Press, None)
			.expect("event"),
		leptoskit_native::EventOutcome::Handled
	);
}

#[test]
fn a_clock_export_with_the_wrong_signature_is_refused() {
	let guest = r#"
(module
  (func (export "leptoskit_clock") (param i32))
  (func (export "leptoskit_main"))
  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) (i32.const 0)))
"#;
	let error = WasmHost::headless(guest.as_bytes())
		.err()
		.expect("refused at load");

	assert!(
		matches!(&error, WasmError::Compile(message) if message.contains("leptoskit_clock")),
		"{error}"
	);
}
