//! The installed macOS recording host uses the same metrics as its native Text
//! renderer.
#![cfg(all(feature = "app", target_os = "macos"))]

use std::fmt::Write as _;

use leptoskit_appkit::AppKitTextMeasurer;
use leptoskit_bridge_ffi::app::App;
use leptoskit_native::FontWeight;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Size;

const WIDTH: f32 = 712.0;
const HEIGHT: f32 = 480.0;
const NARROW_WIDTH: f32 = 64.0;
const DEFAULT_FONT: f32 = 14.0;
const SCALED_FONT: f32 = 28.0;
const JOURNEY: &str =
	"Solana client journey\nready\ndenied\nbalance: 500000000000000000 lamports, slot 277";

#[test]
fn application_text_layout_uses_native_metrics() {
	for (text, font_size, width) in [
		(JOURNEY, DEFAULT_FONT, WIDTH),
		("", DEFAULT_FONT, WIDTH),
		("ready\n", DEFAULT_FONT, WIDTH),
		("ready\n\ndenied\n", DEFAULT_FONT, WIDTH),
		("gypq ÅÉç\nfaçade naïve", DEFAULT_FONT, WIDTH),
		("gypq ÅÉç\nfaçade naïve", SCALED_FONT, WIDTH),
		("alpha beta gamma delta", DEFAULT_FONT, NARROW_WIDTH),
	] {
		let guest = guest(text, font_size);
		let mut app = App::new(guest.as_bytes()).unwrap();
		app.with_host(|host| host.start(width, HEIGHT)).unwrap();
		let frame = app.with_host(|host| {
			let surface = host.surface();
			let root = surface.root().unwrap();
			let label = surface.children(root)[0];
			surface.frame(label).unwrap()
		});
		let measured = AppKitTextMeasurer::new().measure(
			text,
			TextStyle::new(font_size, FontWeight::Regular),
			Size::new(
				AvailableSpace::Definite(frame.width),
				AvailableSpace::MaxContent,
			),
		);
		assert_eq!(
			frame.height.to_bits(),
			measured.height.to_bits(),
			"{text:?} at {font_size}"
		);
	}
}

fn guest(text: &str, font_size: f32) -> String {
	let props = serde_json::json!({ "text": text, "font_size": font_size }).to_string();
	let data = escape(&props);
	let root = r#"{"class":"flex flex-col"}"#;
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
		  (call $create (i64.const 2) (i32.const 1))
		  (call $props (i64.const 1) (i32.const 0) (i32.const {root_length}))
		  (call $props (i64.const 2) (i32.const 256) (i32.const {length}))
		  (call $insert (i64.const 1) (i64.const 2) (i32.const 0))
		  (call $commit))
		(func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0))"#,
		root_length = root.len(),
		length = props.len()
	)
}

fn escape(text: &str) -> String {
	let mut escaped = String::new();

	for byte in text.bytes() {
		write!(escaped, "\\{byte:02x}").unwrap();
	}

	escaped
}
