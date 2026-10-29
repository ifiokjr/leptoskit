use std::fmt::Write;

use leptoskit_native::EventOutcome;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_wasm::EventKind;
use leptoskit_wasm::WasmHost;

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
fn widget_and_event_codes_remain_append_only() {
	for (code, expected, props) in [
		(0, NativeKind::View, "{}"),
		(1, NativeKind::Text, "{}"),
		(2, NativeKind::TextRun, "{}"),
		(3, NativeKind::Image, "{}"),
		(4, NativeKind::ScrollView, "{}"),
		(5, NativeKind::TextInput, r#"{"placeholder":"Name"}"#),
		(6, NativeKind::Button, "{}"),
		(7, NativeKind::Switch, r#"{"checked":true}"#),
		(8, NativeKind::Checkbox, r#"{"checked":false}"#),
		(
			9,
			NativeKind::Radio,
			r#"{"checked":true,"radio_group":"theme"}"#,
		),
		(
			10,
			NativeKind::Slider,
			r#"{"slider":{"min":0,"max":10,"value":3,"step":3}}"#,
		),
		(
			11,
			NativeKind::LinearProgressIndicator,
			r#"{"progress":{"determinate":0.5}}"#,
		),
		(12, NativeKind::ActivityIndicator, r#"{"running":true}"#),
		(
			13,
			NativeKind::PasswordInput,
			r#"{"value":"test-only","placeholder":"Secret"}"#,
		),
		(
			14,
			NativeKind::Pressable,
			r#"{"accessibility_label":"Action"}"#,
		),
	] {
		let guest = control_guest(code, props, EventKind::Press, "");
		let mut host = WasmHost::headless(guest.as_bytes()).unwrap();
		host.start(320.0, 240.0).unwrap();
		let surface = host.surface();
		let root = surface.root().unwrap();
		assert_eq!(
			surface.kind(surface.children(root)[0]),
			expected,
			"code {code}"
		);
	}

	for (code, kind) in [
		EventKind::Press,
		EventKind::Change,
		EventKind::Submit,
		EventKind::Focus,
		EventKind::Blur,
		EventKind::Scroll,
		EventKind::Toggle,
		EventKind::ValueChange,
		EventKind::Key,
	]
	.into_iter()
	.enumerate()
	{
		assert_eq!(kind.code(), i32::try_from(code).unwrap());
	}
}

#[test]
fn rich_pressable_content_routes_hits_to_its_wire_action() {
	let props = r#"{"accessibility_label":"Activate card","class":"w-60 h-20 p-4"}"#;
	let guest = control_guest(14, props, EventKind::Press, "").replace(
		"(call $insert (i64.const 1) (i64.const 43) (i32.const 1))",
		"(call $insert (i64.const 42) (i64.const 43) (i32.const 0))",
	);
	let mut host = WasmHost::headless(guest.as_bytes()).unwrap();
	host.start(320.0, 240.0).unwrap();
	let root = host.surface().root().unwrap();
	let pressable = host.surface().children(root)[0];
	let hit = host.surface().hit_test(20.0, 20.0).unwrap();
	assert_eq!(hit, pressable);
	assert_eq!(
		host.dispatch(hit, &NativeEvent::Press).unwrap(),
		EventOutcome::Handled
	);
	let status = host.surface().children(pressable)[0];
	assert_eq!(
		host.surface().props(status).text.as_deref(),
		Some("accepted")
	);
}

#[test]
fn press_phases_restyle_held_buttons_on_the_host_without_running_the_guest() {
	let props = r#"{"text":"Add one","class":"rounded-lg bg-indigo-500 active:bg-indigo-700"}"#;
	let guest = control_guest(6, props, EventKind::Press, "");
	let mut host = WasmHost::headless(guest.as_bytes()).unwrap();
	host.start(320.0, 240.0).unwrap();
	let root = host.surface().root().unwrap();
	let button = host.surface().children(root)[0];
	let resting = host.surface().props(button).clone();
	host.take_mutations();

	assert_eq!(
		host.dispatch(button, &NativeEvent::PressIn).unwrap(),
		EventOutcome::Unhandled
	);
	assert_eq!(host.commits(), 1, "press phases never enter the guest");
	assert_eq!(host.surface().pressed(), Some(button));
	let held = host.surface().props(button).clone();
	assert_ne!(held.background, resting.background);
	assert_eq!(held.accessible_name(), Some("Add one"));
	assert_eq!(
		host.take_mutations(),
		[leptoskit_native::Mutation::UpdateProps {
			id: button,
			props: held.clone(),
		}]
	);

	// The release activates the button; the guest's re-render keeps the hold.
	assert_eq!(
		host.dispatch(button, &NativeEvent::Press).unwrap(),
		EventOutcome::Handled
	);
	assert_eq!(host.commits(), 2);
	assert_eq!(host.surface().props(button), &held);
	assert_eq!(
		host.dispatch(button, &NativeEvent::PressOut).unwrap(),
		EventOutcome::Unhandled
	);
	assert_eq!(host.surface().pressed(), None);
	assert_eq!(host.surface().props(button), &resting);
	assert_eq!(host.commits(), 2);
}

#[test]
fn boolean_proposals_use_json_booleans_and_preserve_controlled_state() {
	for (kind, props, checked) in [
		(7, r#"{"checked":false}"#, true),
		(8, r#"{"checked":true}"#, false),
		(9, r#"{"checked":false,"radio_group":"theme"}"#, true),
	] {
		let expected_payload = format!(r#"{{"checked":{checked}}}"#);
		let guest = control_guest(kind, props, EventKind::Toggle, &expected_payload);
		let mut host = WasmHost::headless(guest.as_bytes()).unwrap();
		host.start(320.0, 240.0).unwrap();
		let root = host.surface().root().unwrap();
		let target = host.surface().children(root)[0];
		assert_eq!(
			host.dispatch(target, &NativeEvent::Toggle { checked })
				.unwrap(),
			EventOutcome::Handled
		);
		assert_eq!(host.commits(), 2);
		assert_eq!(host.surface().props(target).checked, Some(!checked));
		let status = host.surface().children(root)[1];
		assert_eq!(
			host.surface().props(status).text.as_deref(),
			Some("accepted")
		);
	}
}

#[test]
fn numeric_proposals_use_json_numbers_and_snap_to_the_committed_grid() {
	let guest = control_guest(
		10,
		r#"{"slider":{"min":0,"max":1,"value":0,"step":0.25}}"#,
		EventKind::ValueChange,
		r#"{"value":0.75}"#,
	);
	let mut host = WasmHost::headless(guest.as_bytes()).unwrap();
	host.start(320.0, 240.0).unwrap();
	let root = host.surface().root().unwrap();
	let slider = host.surface().children(root)[0];
	assert_eq!(
		host.dispatch_wire(42, EventKind::ValueChange, Some(r#"{"value":"0.75"}"#))
			.unwrap(),
		EventOutcome::Unhandled
	);
	assert_eq!(
		host.dispatch_wire(43, EventKind::ValueChange, Some(r#"{"value":0.75}"#))
			.unwrap(),
		EventOutcome::Unhandled
	);
	assert_eq!(host.commits(), 1);
	assert_eq!(
		host.dispatch(slider, &NativeEvent::ValueChange { value: 0.7 })
			.unwrap(),
		EventOutcome::Handled
	);
	assert!(host.surface().props(slider).slider.unwrap().value().abs() < f64::EPSILON);
	assert_eq!(host.commits(), 2);

	for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
		assert_eq!(
			host.dispatch(slider, &NativeEvent::ValueChange { value })
				.unwrap(),
			EventOutcome::Unhandled
		);
		assert_eq!(host.commits(), 2);
	}
}

#[test]
fn hidden_and_disabled_ancestors_block_typed_control_proposals() {
	for container in [r#"{"hidden":true}"#, r#"{"enabled":false}"#] {
		let guest = control_guest(7, "{}", EventKind::Toggle, r#"{"checked":true}"#)
			.replace(
				"(func (export \"leptoskit_main\")",
				&format!(
					"(data (i32.const 8192) \"{}\")\n(func (export \"leptoskit_main\")",
					wat_bytes(container)
				),
			)
			.replace(
				"(call $commit))",
				&format!(
					"(call $props (i64.const 1) (i32.const 8192) (i32.const {})) (call $commit))",
					container.len()
				),
			);
		let mut host = WasmHost::headless(guest.as_bytes()).unwrap();
		host.start(320.0, 240.0).unwrap();
		let root = host.surface().root().unwrap();
		let control = host.surface().children(root)[0];
		assert_eq!(
			host.dispatch(control, &NativeEvent::Toggle { checked: true })
				.unwrap(),
			EventOutcome::Unhandled
		);
		assert_eq!(host.commits(), 1);
	}
}

#[test]
fn stopped_activity_does_not_dispatch_input_to_the_guest() {
	let guest = control_guest(12, r#"{"running":false}"#, EventKind::Press, "");
	let mut host = WasmHost::headless(guest.as_bytes()).unwrap();
	host.start(100.0, 100.0).unwrap();
	let indicator = host.surface().children(host.surface().root().unwrap())[0];
	assert_eq!(
		host.dispatch(indicator, &NativeEvent::Press).unwrap(),
		EventOutcome::Unhandled
	);
	assert_eq!(host.commits(), 1);
}

#[test]
fn typed_control_payloads_require_the_guest_allocator() {
	for (kind, event) in [
		(7, NativeEvent::Toggle { checked: true }),
		(10, NativeEvent::ValueChange { value: 0.5 }),
	] {
		let guest = control_guest(kind, "{}", EventKind::Toggle, "").replace(
			"(func (export \"leptoskit_alloc\") (param i32) (result i32) (i32.const 4096))",
			"",
		);
		let mut host = WasmHost::headless(guest.as_bytes()).unwrap();
		host.start(320.0, 240.0).unwrap();
		let root = host.surface().root().unwrap();
		let control = host.surface().children(root)[0];
		assert!(
			host.dispatch(control, &event)
				.unwrap_err()
				.to_string()
				.contains("leptoskit_alloc")
		);
		assert_eq!(host.commits(), 1);
	}
}

#[test]
fn typed_proposals_cannot_target_the_wrong_kind_or_deselect_a_radio() {
	for (kind, props, event) in [
		(6, "{}", NativeEvent::Toggle { checked: true }),
		(7, "{}", NativeEvent::ValueChange { value: 0.5 }),
		(
			9,
			r#"{"radio_group":"theme","checked":true}"#,
			NativeEvent::Toggle { checked: false },
		),
	] {
		let guest = control_guest(kind, props, EventKind::Toggle, r#"{"checked":true}"#);
		let mut host = WasmHost::headless(guest.as_bytes()).unwrap();
		host.start(320.0, 240.0).unwrap();
		let root = host.surface().root().unwrap();
		let target = host.surface().children(root)[0];
		assert_eq!(
			host.dispatch(target, &event).unwrap(),
			EventOutcome::Unhandled
		);
		assert_eq!(host.commits(), 1);
	}
}

fn updating_guest(kind: i32, initial: &str, update: &str, event_body: &str) -> String {
	let event_body = event_body.replace("$update_len", &update.len().to_string());
	format!(
		r#"(module
  (import "leptoskit" "node_create" (func $create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
  (import "leptoskit" "node_insert" (func $insert (param i64 i64 i32)))
  (import "leptoskit" "commit" (func $commit))
  (memory (export "memory") 1)
  (data (i32.const 256) "{initial_bytes}")
  (data (i32.const 1024) "{update_bytes}")
  (func (export "leptoskit_main")
    (call $create (i64.const 42) (i32.const {kind}))
    (call $props (i64.const 42) (i32.const 256) (i32.const {initial_len}))
    (call $commit))
  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32)
    (call $props (i64.const 42) (i32.const 1024) (i32.const {update_len}))
    {event_body}
    (call $commit)
    (i32.const 1)))"#,
		initial_bytes = wat_bytes(initial),
		update_bytes = wat_bytes(update),
		initial_len = initial.len(),
		update_len = update.len(),
	)
}

#[test]
fn invalid_external_control_configuration_preserves_the_committed_interface() {
	for (kind, initial, invalid) in [
		(
			10,
			"{}",
			r#"{"slider":{"min":1,"max":1,"value":1,"step":null}}"#,
		),
		(
			10,
			"{}",
			r#"{"slider":{"min":0,"max":1,"value":2,"step":null}}"#,
		),
		(
			10,
			"{}",
			r#"{"slider":{"min":0,"max":1,"value":0.5,"step":0}}"#,
		),
		(
			10,
			"{}",
			r#"{"slider":{"min":0,"max":1,"value":0.5,"step":0.3}}"#,
		),
		(
			10,
			"{}",
			r#"{"slider":{"min":10000000000,"max":10000000001,"value":10000000000,"step":0.00001}}"#,
		),
		(
			10,
			"{}",
			r#"{"slider":{"min":0,"max":1,"value":1e400,"step":null}}"#,
		),
		(
			10,
			"{}",
			r#"{"slider":{"min":0,"max":1,"value":"0.5","step":null}}"#,
		),
		(11, "{}", r#"{"progress":{"determinate":-0.1}}"#),
		(11, "{}", r#"{"progress":{"determinate":1.1}}"#),
		(9, r#"{"radio_group":"theme"}"#, r#"{"radio_group":""}"#),
		(
			9,
			r#"{"radio_group":"theme"}"#,
			r#"{"radio_group":"a\u0000b"}"#,
		),
		(
			9,
			r#"{"radio_group":"theme"}"#,
			r#"{"radio_group":"a\r\nb"}"#,
		),
		(8, "{}", r#"{"checked":"true"}"#),
	] {
		let guest = updating_guest(kind, initial, invalid, "");
		let mut host = WasmHost::headless(guest.as_bytes()).unwrap();
		host.start(320.0, 240.0).unwrap();
		let root = host.surface().root().unwrap();
		let props = host.surface().props(root).clone();
		let mutation_count = host.surface().backend().log().len();
		assert!(
			host.dispatch(root, &NativeEvent::Press).is_err(),
			"{invalid}"
		);
		assert_eq!(host.commits(), 1);
		assert_eq!(host.surface().root(), Some(root));
		assert_eq!(host.surface().props(root), &props);
		assert_eq!(host.surface().backend().log().len(), mutation_count);
	}
}

#[test]
fn invalid_control_trees_trap_before_mutating_widgets() {
	for (kind, initial, update, body, expected_error) in [
		(
			14,
			r#"{"accessibility_label":"Action"}"#,
			r#"{"accessibility_label":" "}"#,
			"",
			"accessibility label",
		),
		(
			14,
			r#"{"accessibility_label":"Action"}"#,
			r#"{"accessibility_label":"Action"}"#,
			"(call $create (i64.const 43) (i32.const 6)) (call $insert (i64.const 42) (i64.const \
			 43) (i32.const 0))",
			"interactive",
		),
		(
			14,
			r#"{"accessibility_label":"Action"}"#,
			r#"{"accessibility_label":"Action"}"#,
			"(call $create (i64.const 43) (i32.const 4)) (call $insert (i64.const 42) (i64.const \
			 43) (i32.const 0))",
			"interactive",
		),
		(9, r#"{"radio_group":"theme"}"#, "{}", "", "RadioGroup"),
		(
			8,
			"{}",
			"{}",
			"(call $create (i64.const 43) (i32.const 1)) (call $insert (i64.const 42) (i64.const \
			 43) (i32.const 0))",
			"children",
		),
		(
			0,
			"{}",
			r#"{"radio_group":"theme","checked":true}"#,
			"(call $create (i64.const 1) (i32.const 0)) (call $create (i64.const 42) (i32.const \
			 9)) (call $create (i64.const 43) (i32.const 9)) (call $props (i64.const 43) \
			 (i32.const 1024) (i32.const $update_len)) (call $insert (i64.const 1) (i64.const 42) \
			 (i32.const 0)) (call $insert (i64.const 1) (i64.const 43) (i32.const 1))",
			"selected",
		),
	] {
		let guest = updating_guest(kind, initial, update, body);
		let mut host = WasmHost::headless(guest.as_bytes()).unwrap();
		host.start(320.0, 240.0).unwrap();
		let root = host.surface().root().unwrap();
		let mutation_count = host.surface().backend().log().len();
		let error = host
			.dispatch(root, &NativeEvent::Press)
			.unwrap_err()
			.to_string();
		assert!(error.contains(expected_error), "{error}");
		assert_eq!(host.commits(), 1);
		assert_eq!(host.surface().root(), Some(root));
		assert_eq!(host.surface().backend().log().len(), mutation_count);
	}
}
