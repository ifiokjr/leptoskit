#![cfg(all(
	feature = "web",
	feature = "rive",
	target_arch = "wasm32",
	target_os = "unknown"
))]

use std::io::Read;

use js_sys::Array;
use js_sys::Function;
use js_sys::Uint8Array;
use leptos::mount::mount_to;
use leptos::prelude::*;
use leptoskit_native::Element;
use leptoskit_native::web;
use leptoskit_rive::Asset;
use leptoskit_rive::Input;
use leptoskit_rive::Name;
use leptoskit_rive::Playback;
use leptoskit_rive::Scene;
use leptoskit_rive::web::Runtime;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::Blob;
use web_sys::BlobPropertyBag;
use web_sys::CanvasRenderingContext2d;
use web_sys::HtmlCanvasElement;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(
	inline_js = "export function nextFrame() { return new Promise(requestAnimationFrame); }"
)]
extern "C" {
	fn nextFrame() -> js_sys::Promise;
}

fn runtime() -> (Runtime, String) {
	let mut source = String::new();
	flate2::read::GzDecoder::new(
		include_bytes!("../../leptoskit_rive/tests/fixtures/rive.js.gz").as_slice(),
	)
	.read_to_string(&mut source)
	.unwrap();
	source.push_str("\nreturn this.rive;");
	let module = Function::new_no_args(&source)
		.call0(&JsValue::NULL)
		.unwrap();
	let bytes = Uint8Array::from(
		include_bytes!("../../leptoskit_rive/tests/fixtures/rive.wasm").as_slice(),
	);
	let options = BlobPropertyBag::new();
	options.set_type("application/wasm");
	let blob = Blob::new_with_u8_array_sequence_and_options(&Array::of1(&bytes), &options).unwrap();

	let url = web_sys::Url::create_object_url_with_blob(&blob).unwrap();
	(Runtime::new(module, &url).unwrap(), url)
}

fn pixels(canvas: &HtmlCanvasElement) -> Vec<u8> {
	let context = canvas
		.get_context("2d")
		.unwrap()
		.unwrap()
		.dyn_into::<CanvasRenderingContext2d>()
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
		.to_vec()
}

async fn wait_for_pixels(canvas: &HtmlCanvasElement, different_from: Option<&[u8]>) -> Vec<u8> {
	let deadline = js_sys::Date::now() + 15_000.0;

	loop {
		JsFuture::from(nextFrame()).await.unwrap();
		assert_eq!(canvas.get_attribute("data-lk-rive-error"), None);
		let current = pixels(canvas);

		if current.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0)
			&& different_from.is_none_or(|previous| previous != current)
		{
			return current;
		}

		assert!(
			js_sys::Date::now() < deadline,
			"real browser Rive pixels did not reach the expected pose"
		);
	}
}

#[wasm_bindgen_test]
async fn shared_canvas_retains_its_pose_through_updates_and_disposes_on_removal() {
	let (runtime, wasm_url) = runtime();
	let document = web_sys::window().unwrap().document().unwrap();
	let parent = document
		.create_element("div")
		.unwrap()
		.dyn_into::<web_sys::HtmlElement>()
		.unwrap();
	parent
		.set_attribute(
			"style",
			"position:fixed;left:0;top:0;width:128px;height:128px",
		)
		.unwrap();
	document.body().unwrap().append_child(&parent).unwrap();
	let revision = ArcRwSignal::new(0);
	let running = ArcRwSignal::new(false);
	let show = ArcRwSignal::new(true);

	let state = (revision.clone(), running.clone(), show.clone());
	let playback = Playback::new(
		Asset::new(include_bytes!(
			"../../leptoskit_rive/tests/fixtures/scene/build/leptoskit-rive-test.riv"
		))
		.unwrap(),
	);
	let mounted = mount_to(parent.clone(), move || {
		web::provide_rive_runtime(runtime);
		move || {
			if state.2.get() {
				web::render(
					Element::rive(playback.clone())
						.class("w-32 h-32")
						.aria_label(format!("Triangle {}", state.0.get()))
						.running(state.1.get()),
				)
			} else {
				web::render(Element::view())
			}
		}
	});
	let canvas = parent
		.query_selector("canvas")
		.unwrap()
		.unwrap()
		.dyn_into::<HtmlCanvasElement>()
		.unwrap();
	let initial = wait_for_pixels(&canvas, None).await;
	assert_eq!(canvas.get_attribute("role").as_deref(), Some("img"));
	revision.set(1);
	leptos::task::tick().await;

	assert_eq!(
		parent.query_selector("canvas").unwrap().unwrap(),
		canvas.clone().into()
	);
	assert_eq!(
		canvas.get_attribute("aria-label").as_deref(),
		Some("Triangle 1")
	);
	assert_eq!(pixels(&canvas), initial);
	running.set(true);
	wait_for_pixels(&canvas, Some(&initial)).await;
	running.set(false);
	leptos::task::tick().await;
	let paused = pixels(&canvas);
	JsFuture::from(nextFrame()).await.unwrap();
	JsFuture::from(nextFrame()).await.unwrap();
	assert_eq!(pixels(&canvas), paused);
	show.set(false);
	leptos::task::tick().await;
	assert!(!canvas.is_connected());
	assert_eq!(
		(canvas.width(), canvas.height()),
		(300, 150),
		"drop restores canvas dimensions and disposes the player"
	);
	drop(mounted);
	parent.remove();
	web_sys::Url::revoke_object_url(&wasm_url).unwrap();
}

async fn wait_for_diagnostic(canvas: &HtmlCanvasElement, expected: &str) {
	let deadline = js_sys::Date::now() + 15_000.0;

	loop {
		JsFuture::from(nextFrame()).await.unwrap();

		if let Some(message) = canvas.get_attribute("data-lk-rive-error") {
			assert!(message.contains(expected), "{message}");

			return;
		}

		assert!(
			js_sys::Date::now() < deadline,
			"Rive failure did not reach the canvas diagnostic"
		);
	}
}

#[wasm_bindgen_test]
async fn controlled_inputs_and_scene_replacement_recover_from_browser_failures() {
	let (runtime, wasm_url) = runtime();
	let document = web_sys::window().unwrap().document().unwrap();
	let parent = document
		.create_element("div")
		.unwrap()
		.dyn_into::<web_sys::HtmlElement>()
		.unwrap();
	parent
		.set_attribute(
			"style",
			"position:fixed;left:0;top:0;width:128px;height:128px",
		)
		.unwrap();
	document.body().unwrap().append_child(&parent).unwrap();
	let asset = Asset::new(include_bytes!(
		"../../leptoskit_rive/tests/fixtures/scene/build/leptoskit-rive-test.riv"
	))
	.unwrap();
	let mut initial =
		Playback::new(asset.clone()).scene(Scene::StateMachine(Name::new("Machine").unwrap()));

	initial
		.set_input(Name::new("amount").unwrap(), Input::Number(0.0), 0)
		.unwrap();
	let content = ArcRwSignal::new(initial.clone());
	let state = content.clone();
	let mounted = mount_to(parent.clone(), move || {
		web::provide_rive_runtime(runtime);
		move || {
			web::render(
				Element::rive(state.get())
					.class("w-32 h-32")
					.aria_hidden(true),
			)
		}
	});
	let canvas = parent
		.query_selector("canvas")
		.unwrap()
		.unwrap()
		.dyn_into::<HtmlCanvasElement>()
		.unwrap();
	wait_for_pixels(&canvas, None).await;
	initial
		.set_input(Name::new("amount").unwrap(), Input::Number(1.0), 1)
		.unwrap();
	initial
		.set_input(Name::new("enabled").unwrap(), Input::Boolean(true), 1)
		.unwrap();

	initial
		.set_input(Name::new("activate").unwrap(), Input::Trigger, 1)
		.unwrap();
	content.set(initial.clone());
	leptos::task::tick().await;
	JsFuture::from(nextFrame()).await.unwrap();
	assert_eq!(canvas.get_attribute("data-lk-rive-error"), None);
	assert_eq!(
		parent.query_selector("canvas").unwrap().unwrap(),
		canvas.clone().into()
	);
	initial
		.set_input(Name::new("amount").unwrap(), Input::Boolean(true), 2)
		.unwrap();
	content.set(initial);
	wait_for_diagnostic(&canvas, "input type mismatch").await;
	content
		.set(Playback::new(asset.clone()).scene(Scene::Animation(Name::new("missing").unwrap())));
	wait_for_diagnostic(&canvas, "unknown Rive scene").await;
	content.set(Playback::new(asset));
	wait_for_pixels(&canvas, None).await;
	assert_eq!(
		parent.query_selector("canvas").unwrap().unwrap(),
		canvas.clone().into()
	);
	drop(mounted);
	assert_eq!((canvas.width(), canvas.height()), (300, 150));
	parent.remove();
	web_sys::Url::revoke_object_url(&wasm_url).unwrap();
}

#[wasm_bindgen_test]
async fn missing_runtime_reports_a_diagnostic_and_removal_cancels_pending_mounts() {
	let document = web_sys::window().unwrap().document().unwrap();
	let parent = document
		.create_element("div")
		.unwrap()
		.dyn_into::<web_sys::HtmlElement>()
		.unwrap();
	document.body().unwrap().append_child(&parent).unwrap();
	let content = Playback::new(
		Asset::new(include_bytes!(
			"../../leptoskit_rive/tests/fixtures/scene/build/leptoskit-rive-test.riv"
		))
		.unwrap(),
	);
	let mounted = mount_to(parent.clone(), {
		let content = content.clone();
		move || web::render(Element::rive(content).aria_hidden(true))
	});
	let canvas = parent
		.query_selector("canvas")
		.unwrap()
		.unwrap()
		.dyn_into::<HtmlCanvasElement>()
		.unwrap();
	wait_for_diagnostic(&canvas, "provide_rive_runtime").await;
	drop(mounted);

	let (runtime, wasm_url) = runtime();
	let mounted = mount_to(parent.clone(), move || {
		web::provide_rive_runtime(runtime);
		web::render(Element::rive(content).aria_hidden(true))
	});
	let pending = parent.query_selector("canvas").unwrap().unwrap();
	drop(mounted);
	leptos::task::tick().await;
	JsFuture::from(nextFrame()).await.unwrap();
	assert!(!pending.is_connected());
	assert_eq!(pending.get_attribute("data-lk-rive-error"), None);

	parent.remove();
	web_sys::Url::revoke_object_url(&wasm_url).unwrap();
}
