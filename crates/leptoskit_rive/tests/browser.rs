#![cfg(all(feature = "web", target_arch = "wasm32", target_os = "unknown"))]

use std::cell::OnceCell;
use std::io::Read;

use js_sys::Array;
use js_sys::Function;
use js_sys::Uint8Array;
use leptoskit_rive::Asset;
use leptoskit_rive::Input;
use leptoskit_rive::Name;
use leptoskit_rive::Scene;
use leptoskit_rive::web::Options;
use leptoskit_rive::web::Player;
use leptoskit_rive::web::Runtime;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::Blob;
use web_sys::BlobPropertyBag;
use web_sys::CanvasRenderingContext2d;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(
	inline_js = "export async function frames(n) { for(let i=0; i<n; ++i) await new \
	             Promise(requestAnimationFrame); }"
)]
extern "C" {
	fn frames(count: u32) -> js_sys::Promise;
}

/// One actual upstream runtime, loaded offline from version-pinned test files.
fn runtime() -> Runtime {
	thread_local! {
		static RUNTIME: OnceCell<Runtime> = const { OnceCell::new() };
	}

	RUNTIME.with(|cell| {
		cell.get_or_init(|| {
			let mut source = String::new();
			flate2::read::GzDecoder::new(include_bytes!("fixtures/rive.js.gz").as_slice())
				.read_to_string(&mut source)
				.unwrap();
			source.push_str("\nreturn this.rive;");
			let module = Function::new_no_args(&source)
				.call0(&JsValue::NULL)
				.unwrap();
			let bytes = Uint8Array::from(include_bytes!("fixtures/rive.wasm").as_slice());
			let parts = Array::of1(&bytes);
			let options = BlobPropertyBag::new();
			options.set_type("application/wasm");
			let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &options).unwrap();
			let url = web_sys::Url::create_object_url_with_blob(&blob).unwrap();

			let runtime = Runtime::from_global(&url).unwrap();
			assert!(Runtime::new(JsValue::UNDEFINED, &url).is_err());
			assert!(Runtime::new(module, "https://outside.test/rive.wasm").is_err());

			runtime
		})
		.clone()
	})
}

/// A visible, explicitly sized, accessible canvas attached to the real DOM.
fn canvas() -> HtmlCanvasElement {
	let document = web_sys::window().unwrap().document().unwrap();
	let canvas: HtmlCanvasElement = document
		.create_element("canvas")
		.unwrap()
		.dyn_into()
		.unwrap();
	canvas
		.set_attribute("aria-label", "Rotating blue triangle")
		.unwrap();
	canvas
		.set_attribute(
			"style",
			"position:fixed;top:0;left:0;width:128px;height:128px",
		)
		.unwrap();
	document.body().unwrap().append_child(&canvas).unwrap();

	canvas
}

fn asset() -> Asset {
	Asset::new(include_bytes!(
		"fixtures/scene/build/leptoskit-rive-test.riv"
	))
	.unwrap()
}

/// Read the actual rendered pixels, rather than trusting an on-load event.
fn pixels(canvas: &HtmlCanvasElement) -> Vec<u8> {
	let context: CanvasRenderingContext2d = canvas
		.get_context("2d")
		.unwrap()
		.unwrap()
		.dyn_into()
		.unwrap();
	context
		.get_image_data(
			0.0,
			0.0,
			f64::from(canvas.width()),
			f64::from(canvas.height()),
		)
		.unwrap()
		.data()
		.0
}

async fn advance(count: u32) {
	JsFuture::from(frames(count)).await.unwrap();
}

#[wasm_bindgen_test]
async fn renders_moves_pauses_and_releases_the_real_runtime() {
	let runtime = runtime();
	let canvas = canvas();
	let options = Options {
		scene: Scene::Animation(Name::new("Spin").unwrap()),
		..Options::default()
	};
	let player = Player::new(&runtime, &canvas, &asset(), &options).unwrap();
	player.ready().await.unwrap();
	advance(3).await;
	let first = pixels(&canvas);
	assert!(
		first.chunks_exact(4).any(|pixel| pixel[3] != 0),
		"the first pose must be visible while paused"
	);
	player.play().unwrap();
	advance(8).await;
	assert_ne!(
		first,
		pixels(&canvas),
		"the animation must change actual pixels"
	);
	player.pause().unwrap();
	advance(3).await;
	let paused = pixels(&canvas);
	advance(5).await;
	assert_eq!(paused, pixels(&canvas));
	assert!(Player::new(&runtime, &canvas, &asset(), &options).is_err());
	player.close().unwrap();
	assert_eq!(canvas.width(), 300);
	assert_eq!(canvas.height(), 150);
	let replacement = Player::new(&runtime, &canvas, &asset(), &options).unwrap();
	replacement.ready().await.unwrap();
	drop(replacement);
	canvas.remove();
}

#[wasm_bindgen_test]
async fn validates_inputs_and_rejects_broken_files_and_scene_names() {
	let runtime = runtime();
	let canvas = canvas();
	let options = Options {
		scene: Scene::StateMachine(Name::new("Machine").unwrap()),
		..Options::default()
	};
	let player = Player::new(&runtime, &canvas, &asset(), &options).unwrap();
	assert!(
		player
			.set_input(&Name::new("amount").unwrap(), Input::Number(1.0))
			.is_err()
	);
	player.ready().await.unwrap();
	for (name, input) in [
		("amount", Input::Number(2.0)),
		("enabled", Input::Boolean(true)),
		("activate", Input::Trigger),
	] {
		player
			.validate_input(&Name::new(name).unwrap(), input)
			.unwrap();
	}
	assert!(
		player
			.validate_input(&Name::new("amount").unwrap(), Input::Trigger)
			.is_err()
	);
	player
		.set_input(&Name::new("amount").unwrap(), Input::Number(2.0))
		.unwrap();
	player
		.set_input(&Name::new("enabled").unwrap(), Input::Boolean(true))
		.unwrap();
	player
		.set_input(&Name::new("activate").unwrap(), Input::Trigger)
		.unwrap();
	assert!(
		player
			.set_input(&Name::new("amount").unwrap(), Input::Boolean(true))
			.is_err()
	);
	assert!(
		player
			.set_input(&Name::new("missing").unwrap(), Input::Trigger)
			.is_err()
	);
	assert!(
		player
			.set_input(&Name::new("amount").unwrap(), Input::Number(f64::NAN))
			.is_err()
	);
	drop(player);
	let broken = Asset::new(b"RIVE\x07\x00\x00\x00").unwrap();
	let player = Player::new(&runtime, &canvas, &broken, &options).unwrap();
	assert!(player.ready().await.is_err());
	drop(player);
	let options = Options {
		scene: Scene::Animation(Name::new("missing").unwrap()),
		..Options::default()
	};
	let player = Player::new(&runtime, &canvas, &asset(), &options).unwrap();
	assert!(player.ready().await.is_err());
	drop(player);
	canvas.remove();
}

#[wasm_bindgen_test]
async fn dropping_pending_imports_and_large_canvases_is_bounded() {
	let runtime = runtime();
	let canvas = canvas();
	canvas
		.set_attribute(
			"style",
			"position:fixed;top:0;left:0;width:100000px;height:100000px",
		)
		.unwrap();
	let player = Player::new(&runtime, &canvas, &asset(), &Options::default()).unwrap();
	player.ready().await.unwrap();
	assert!(canvas.width() <= 4096 && canvas.height() <= 4096);
	assert!(u64::from(canvas.width()) * u64::from(canvas.height()) <= 4_194_304);
	drop(player);
	let pending = Player::new(&runtime, &canvas, &asset(), &Options::default()).unwrap();
	drop(pending);
	advance(3).await;
	let player = Player::new(&runtime, &canvas, &asset(), &Options::default()).unwrap();
	player.ready().await.unwrap();
	drop(player);
	canvas.remove();
}
