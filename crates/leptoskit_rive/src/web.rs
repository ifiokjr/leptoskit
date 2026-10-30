//! Browser playback using an app-supplied official Rive runtime module.
//!
//! The app pins and serves the JS/Wasm runtime itself. This crate never injects
//! scripts or chooses a CDN. Drop [`Player`] to release the canvas, observers,
//! pending load, animation loop, and runtime resources.

use js_sys::Uint8Array;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::HtmlCanvasElement;

use crate::Asset;
use crate::Error;
use crate::Input;
use crate::Name;
use crate::Scene;

#[wasm_bindgen(module = "/src/web.js")]
extern "C" {
	#[wasm_bindgen(catch, js_name = configure)]
	fn configure(module: &JsValue, wasm_url: &str) -> Result<(), JsValue>;
	#[wasm_bindgen(catch, js_name = mount)]
	fn mount(
		module: &JsValue,
		canvas: &HtmlCanvasElement,
		bytes: &Uint8Array,
		artboard: Option<&str>,
		scene_kind: &str,
		scene: Option<&str>,
		autoplay: bool,
	) -> Result<Handle, JsValue>;

	type Handle;
	#[wasm_bindgen(method, getter)]
	fn ready(this: &Handle) -> js_sys::Promise;
	#[wasm_bindgen(method, catch)]
	fn play(this: &Handle) -> Result<(), JsValue>;
	#[wasm_bindgen(method, catch)]
	fn pause(this: &Handle) -> Result<(), JsValue>;
	#[wasm_bindgen(method, catch, js_name = setInput)]
	fn set_input(this: &Handle, name: &str, kind: &str, value: JsValue) -> Result<(), JsValue>;
	#[wasm_bindgen(method, catch, js_name = validateInput)]
	fn validate_input(this: &Handle, name: &str, kind: &str) -> Result<(), JsValue>;
	#[wasm_bindgen(method, catch)]
	fn dispose(this: &Handle) -> Result<(), JsValue>;
}

/// A shared official runtime namespace, such as `@rive-app/canvas`.
///
/// Construct once at app startup. The Wasm URL must share the page's origin;
/// configuring the same namespace with another URL is rejected. The runtime
/// and its codecs remain trusted dependencies, outside leptoskit's Wasm guest.
#[derive(Debug, Clone)]
pub struct Runtime(JsValue);

impl Runtime {
	/// Uses the official self-hosted script's `globalThis.rive` namespace.
	/// Load that script in the page before calling this during browser startup.
	pub fn from_global(wasm_url: &str) -> Result<Self, Error> {
		let module = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("rive"))
			.map_err(|error| runtime_error(&error))?;

		Self::new(module, wasm_url)
	}

	/// Configures the app-supplied module before its first player is created.
	pub fn new(module: JsValue, wasm_url: &str) -> Result<Self, Error> {
		configure(&module, wasm_url).map_err(|error| runtime_error(&error))?;
		Ok(Self(module))
	}
}

/// The scene selected at import. Playback starts only when explicitly enabled.
#[derive(Debug, Clone, Default)]
pub struct Options {
	/// Use a named artboard, or the file's default artboard.
	pub artboard: Option<Name>,
	/// Run one timeline or one state machine.
	pub scene: Scene,
	/// Start when loaded and visible. Reduced motion still prevents animation.
	pub autoplay: bool,
}

/// One owned canvas player. It never copies encoded bytes on an animation
/// frame.
///
/// The upstream runtime owns frame scheduling. Rendering stops while paused,
/// offscreen, in a hidden document, or when reduced motion is requested. The
/// first pose stays visible. A canvas may have only one player at a time.
pub struct Player(Handle);

impl Player {
	/// Starts importing a trusted, self-contained file. Call [`Self::ready`] to
	/// observe import failures. Dropping during import cancels the owned
	/// player.
	pub fn new(
		runtime: &Runtime,
		canvas: &HtmlCanvasElement,
		asset: &Asset,
		options: &Options,
	) -> Result<Self, Error> {
		let bytes = Uint8Array::from(asset.bytes());
		let (kind, name) = match &options.scene {
			Scene::Default => ("default", None),
			Scene::Animation(name) => ("animation", Some(name.as_str())),
			Scene::StateMachine(name) => ("stateMachine", Some(name.as_str())),
		};
		let handle = mount(
			&runtime.0,
			canvas,
			&bytes,
			options.artboard.as_ref().map(Name::as_str),
			kind,
			name,
			options.autoplay,
		)
		.map_err(|error| runtime_error(&error))?;

		Ok(Self(handle))
	}

	/// Waits for import, with a 15-second load deadline. A cancelled wait does
	/// not relinquish ownership: dropping the player still performs cleanup.
	pub fn ready(&self) -> impl Future<Output = Result<(), Error>> + use<> {
		let ready = self.0.ready();
		async move {
			let result = JsFuture::from(ready)
				.await
				.map_err(|error| runtime_error(&error))?;

			if let Some(error) = result.as_string() {
				return Err(Error::Runtime(error));
			}

			Ok(())
		}
	}

	/// Requests playback, subject to visibility and reduced-motion preferences.
	pub fn play(&self) -> Result<(), Error> {
		self.0.play().map_err(|error| runtime_error(&error))
	}

	/// Pauses playback without discarding the current pose.
	pub fn pause(&self) -> Result<(), Error> {
		self.0.pause().map_err(|error| runtime_error(&error))
	}

	/// Releases the player explicitly, returning any upstream cleanup failure.
	/// Drop also releases resources and reports cleanup failures to the
	/// console.
	pub fn close(self) -> Result<(), Error> {
		self.0.dispose().map_err(|error| runtime_error(&error))
	}

	/// Updates one input, failing for a missing input or a mismatched type.
	pub fn set_input(&self, name: &Name, input: Input) -> Result<(), Error> {
		input.validate()?;
		let (kind, value) = match input {
			Input::Boolean(value) => ("boolean", JsValue::from_bool(value)),
			Input::Number(value) => ("number", JsValue::from_f64(value)),
			Input::Trigger => ("trigger", JsValue::UNDEFINED),
		};

		self.0
			.set_input(name.as_str(), kind, value)
			.map_err(|error| runtime_error(&error))
	}

	/// Checks a named input's type without updating it or firing a trigger.
	pub fn validate_input(&self, name: &Name, input: Input) -> Result<(), Error> {
		input.validate()?;
		let kind = match input {
			Input::Boolean(_) => "boolean",
			Input::Number(_) => "number",
			Input::Trigger => "trigger",
		};
		self.0
			.validate_input(name.as_str(), kind)
			.map_err(|error| runtime_error(&error))
	}
}

impl Drop for Player {
	fn drop(&mut self) {
		if let Err(error) = self.0.dispose() {
			web_sys::console::error_1(&error);
		}
	}
}

/// Preserve a useful upstream failure without panicking across the JS boundary.
fn runtime_error(value: &JsValue) -> Error {
	let message = value
		.as_string()
		.or_else(|| value.dyn_ref::<js_sys::Error>().map(|e| e.message().into()))
		.unwrap_or_else(|| "browser operation failed".into());

	Error::Runtime(message)
}
