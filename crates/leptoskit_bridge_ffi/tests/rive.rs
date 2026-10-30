#![cfg(all(feature = "app", not(target_arch = "wasm32")))]
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::fmt::Write as _;
use std::mem::MaybeUninit;
use std::ptr;

use leptoskit_bridge_ffi::Buffer;
use leptoskit_bridge_ffi::app::*;
use leptoskit_bridge_ffi::lk_buffer_release;
use leptoskit_rive::Asset;
use leptoskit_rive::Playback;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Style;

fn guest() -> String {
	let playback = Playback::new(
		Asset::new(include_bytes!(
			"../../leptoskit_rive/tests/fixtures/scene/build/leptoskit-rive-test.riv"
		))
		.unwrap(),
	);
	let json = serde_json::to_vec(&serde_json::json!({
		"rive": playback, "running": true, "accessibility_label": "Triangle",
		"style": Style { width: Dimension::Points(128.0), height: Dimension::Points(128.0), ..Style::default() },
	})).unwrap();
	let mut data = String::with_capacity(json.len() * 3);
	for byte in &json {
		write!(&mut data, "\\{byte:02x}").unwrap();
	}
	format!(
		r#"(module
        (import "leptoskit" "node_create" (func $create (param i64 i32)))
        (import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
        (import "leptoskit" "commit" (func $commit))
        (memory (export "memory") 1)
        (data (i32.const 0) "{data}")
        (func (export "leptoskit_main")
            (call $create (i64.const 1) (i32.const 16))
            (call $props (i64.const 1) (i32.const 0) (i32.const {length}))
            (call $commit))
        (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) unreachable)
        (func (export "leptoskit_poll") unreachable))"#,
		length = json.len()
	)
}

fn take(buffer: Buffer) -> Vec<u8> {
	// SAFETY: The ABI returns one live buffer, copied then released exactly
	// once.
	unsafe {
		let bytes = std::slice::from_raw_parts(buffer.data, buffer.len).to_vec();
		lk_buffer_release(buffer);
		bytes
	}
}

#[derive(Default)]
struct Images {
	frames: Vec<Vec<u8>>,
	hidden: bool,
	invalid_visibility: bool,
	fail: bool,
}

unsafe extern "C" fn visible(context: *mut c_void, _node: usize) -> i32 {
	// SAFETY: Each tick exclusively lends this stack-owned Images to callbacks.
	let images = unsafe { &mut *context.cast::<Images>() };
	if images.invalid_visibility {
		-1
	} else {
		i32::from(!images.hidden)
	}
}

unsafe extern "C" fn present(
	context: *mut c_void,
	_node: usize,
	width: u32,
	height: u32,
	pixels: *const u8,
	length: usize,
) -> i32 {
	// SAFETY: The callback exclusively borrows Images and the ABI lends exactly
	// length readable bytes for this synchronous upload, never retained.
	let images = unsafe { &mut *context.cast::<Images>() };
	assert_eq!((width, height, length), (128, 128, 128 * 128 * 4));
	if images.fail {
		return -1;
	}
	images
		.frames
		.push(unsafe { std::slice::from_raw_parts(pixels, length) }.to_vec());
	1
}

#[test]
fn native_frames_bypass_guest_polling_and_json_and_copy_before_returning() {
	let wasm = guest();
	let mut diagnostic = MaybeUninit::uninit();
	// SAFETY: App and callback borrows remain exclusive on this thread. Destroy
	// consumes the live app exactly once after its final callback.
	unsafe {
		let app = create(wasm.as_ptr(), wasm.len(), diagnostic.as_mut_ptr(), |_| {
			Ok(())
		});
		assert!(!app.is_null());
		assert_eq!(lk_app_start(app, 128.0, 128.0), 0);
		assert_eq!(lk_app_has_rive(app), 1);
		let render = take(lk_app_render(app));
		let json: serde_json::Value = serde_json::from_slice(&render).unwrap();
		let created = json["mutations"]
			.as_array()
			.unwrap()
			.iter()
			.find_map(|mutation| mutation.get("Create"))
			.unwrap();
		assert_eq!(created["kind"], "rive");
		assert!(!String::from_utf8(render).unwrap().contains("\"rive\":"));
		let mut images = Images::default();
		let context = ptr::from_mut(&mut images).cast();
		assert_eq!(
			lk_app_rive_tick(app, 0.0, 1.0, 1, context, Some(visible), Some(present)),
			1
		);
		let initial = images.frames[0].clone();
		assert_eq!(
			lk_app_rive_tick(app, 0.1, 1.0, 1, context, Some(visible), Some(present)),
			1
		);
		assert_ne!(initial, images.frames[1]);
		assert_eq!(
			initial, images.frames[0],
			"the upload must own pixels before the next frame overwrites them"
		);
		let empty: serde_json::Value = serde_json::from_slice(&take(lk_app_render(app))).unwrap();
		assert_eq!(empty["mutations"], serde_json::json!([]));
		images.hidden = true;
		assert_eq!(
			lk_app_rive_tick(app, 0.1, 1.0, 1, context, Some(visible), Some(present)),
			0
		);
		assert_eq!(images.frames.len(), 2);
		images.invalid_visibility = true;
		assert_eq!(
			lk_app_rive_tick(app, 0.0, 1.0, 1, context, Some(visible), Some(present)),
			-1
		);
		assert_eq!(images.frames.len(), 2);
		images.invalid_visibility = false;
		assert_eq!(
			lk_app_rive_tick(app, f64::NAN, 1.0, 1, context, Some(visible), Some(present)),
			-1
		);
		assert_eq!(
			lk_app_rive_tick(app, 0.0, 1.0, 4, context, Some(visible), Some(present)),
			-1
		);
		assert_eq!(
			lk_app_rive_tick(app, 0.0, 1.0, 1, context, None, Some(present)),
			-1
		);
		assert_ne!(take(lk_app_error(app)), [] as [u8; 0]);
		images.hidden = false;
		images.fail = true;
		assert_eq!(
			lk_app_rive_tick(app, 0.1, 1.0, 1, context, Some(visible), Some(present)),
			-1
		);
		assert_eq!(
			lk_app_rive_tick(app, 0.1, 1.0, 1, context, Some(visible), Some(present)),
			0
		);
		lk_app_destroy(app);
	}
}
