//! The real recording App allocates native Button titles rather than label
//! boxes.
#![cfg(all(feature = "app", target_os = "macos"))]

use std::fmt::Write as _;

use leptoskit_bridge_ffi::app::App;

const WIDTH: f32 = 712.0;
const HEIGHT: f32 = 480.0;
const JOURNEY_WIDTH: f32 = 258.5;
const NATIVE_HEIGHT: f32 = 24.0;
const SCALED_TEXT_HEIGHT: f32 = 33.0;
const NARROW_WIDTH: f32 = 64.0;
const JOURNEY: &str = "Subscribe, transfer and unsubscribe";

#[test]
fn application_button_layout_reserves_the_native_control() {
	let (width, height) = allocated(&serde_json::json!({ "text": JOURNEY }));
	assert!(
		width >= JOURNEY_WIDTH,
		"native title needs {JOURNEY_WIDTH}, got {width}"
	);
	assert_eq!(height.to_bits(), NATIVE_HEIGHT.to_bits());
}

#[test]
fn explicit_narrow_button_allocation_preserves_native_truncation() {
	let (width, height) = allocated(&serde_json::json!({ "text": JOURNEY, "class": "w-16" }));
	assert_eq!(width.to_bits(), NARROW_WIDTH.to_bits());
	assert_eq!(height.to_bits(), NATIVE_HEIGHT.to_bits());
}

#[test]
fn large_native_fonts_keep_the_native_height_and_custom_chrome_uses_text_height() {
	let (_, native) = allocated(&serde_json::json!({ "text": JOURNEY, "font_size": 28 }));
	let (_, custom) = allocated(&serde_json::json!({
		"text": JOURNEY, "font_size": 28, "background": 255
	}));
	assert_eq!(native.to_bits(), NATIVE_HEIGHT.to_bits());
	assert_eq!(custom.to_bits(), SCALED_TEXT_HEIGHT.to_bits());
}

fn allocated(props: &serde_json::Value) -> (f32, f32) {
	let guest = guest(&props.to_string());
	let mut app = App::new(guest.as_bytes()).unwrap();
	app.with_host(|host| host.start(WIDTH, HEIGHT)).unwrap();
	app.with_host(|host| {
		let surface = host.surface();
		let root = surface.root().unwrap();
		let button = surface.children(root)[0];
		let frame = surface.frame(button).unwrap();
		(frame.width, frame.height)
	})
}

fn guest(props: &str) -> String {
	let data = escape(props);
	let root = r#"{"class":"flex flex-col items-start"}"#;
	let root_data = escape(root);
	format!(
		r#"(module
		(import "leptoskit" "node_create" (func $create (param i64 i32)))
		(import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
		(import "leptoskit" "node_insert" (func $insert (param i64 i64 i32)))
		(import "leptoskit" "commit" (func $commit))
		(memory (export "memory") 1)
		(data (i32.const 0) "{root_data}")
		(data (i32.const 256) "{data}")
		(func (export "leptoskit_main")
		  (call $create (i64.const 1) (i32.const 0))
		  (call $create (i64.const 2) (i32.const 6))
		  (call $props (i64.const 1) (i32.const 0) (i32.const {root_length}))
		  (call $props (i64.const 2) (i32.const 256) (i32.const {length}))
		  (call $insert (i64.const 1) (i64.const 2) (i32.const 0))
		  (call $commit))
		(func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0))"#,
		root_length = root.len(),
		length = props.len(),
	)
}

fn escape(text: &str) -> String {
	let mut escaped = String::new();

	for byte in text.bytes() {
		write!(escaped, "\\{byte:02x}").unwrap();
	}

	escaped
}
