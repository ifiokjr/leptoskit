//! Browser input picker. Call/poll directly inside a trusted click handler.
//!
//! Requires `HTMLInputElement.showPicker` and the `cancel` event. Dropping the
//! future detaches listeners and removes the input; browsers do not expose a
//! method to forcibly dismiss their already-open system picker.
use std::cell::RefCell;
use std::rc::Rc;

use futures::channel::oneshot;
use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;

use crate::Document;
use crate::Error;
use crate::MAX_METADATA_BYTES;
use crate::Options;

struct InputGuard {
	input: web_sys::HtmlInputElement,
	change: Closure<dyn FnMut(web_sys::Event)>,
	cancel: Closure<dyn FnMut(web_sys::Event)>,
}
impl Drop for InputGuard {
	fn drop(&mut self) {
		self.input.set_onchange(None);
		let _ = self
			.input
			.remove_event_listener_with_callback("cancel", self.cancel.as_ref().unchecked_ref());
		self.input.remove();
	}
}

/// Presents the browser's file input, returning owned bytes bounded by options.
/// Only the direct browser API can preserve transient user activation; this
/// function is not installed as a delayed native-bridge operation.
pub async fn pick(options: Options) -> Result<Document, Error> {
	let document = web_sys::window()
		.and_then(|window| window.document())
		.ok_or(Error::Unavailable)?;
	let input = document
		.create_element("input")
		.map_err(|_| Error::Native)?
		.dyn_into::<web_sys::HtmlInputElement>()
		.map_err(|_| Error::Native)?;
	input.set_type("file");
	input.set_hidden(true);

	if !js_sys::Reflect::has(input.as_ref(), &"oncancel".into()).map_err(|_| Error::Native)?
		|| !js_sys::Reflect::get(input.as_ref(), &"showPicker".into())
			.map_err(|_| Error::Native)?
			.is_function()
	{
		return Err(Error::Unavailable);
	}
	let (sender, receiver) = oneshot::channel();

	let sender = Rc::new(RefCell::new(Some(sender)));
	let changed = sender.clone();
	let selected = input.clone();
	let change = Closure::wrap(Box::new(move |_: web_sys::Event| {
		if let Some(sender) = changed.borrow_mut().take() {
			let result = selected
				.files()
				.and_then(|files| files.get(0))
				.ok_or(Error::Cancelled);
			let _ = sender.send(result);
		}
	}) as Box<dyn FnMut(web_sys::Event)>);
	let cancel = Closure::wrap(Box::new(move |_: web_sys::Event| {
		if let Some(sender) = sender.borrow_mut().take() {
			let _ = sender.send(Err(Error::Cancelled));
		}
	}) as Box<dyn FnMut(web_sys::Event)>);
	let guard = InputGuard {
		input,
		change,
		cancel,
	};
	guard
		.input
		.set_onchange(Some(guard.change.as_ref().unchecked_ref()));
	guard
		.input
		.add_event_listener_with_callback("cancel", guard.cancel.as_ref().unchecked_ref())
		.map_err(|_| Error::Native)?;
	document
		.body()
		.ok_or(Error::Unavailable)?
		.append_child(&guard.input)
		.map_err(|_| Error::Native)?;
	guard.input.show_picker().map_err(|error| failure(&error))?;
	let file = receiver.await.map_err(|_| Error::Cancelled)??;
	read_file(&file, options).await
}

/// Reads an already selected browser File, without prompting again.
pub async fn read_file(file: &web_sys::File, options: Options) -> Result<Document, Error> {
	let size = file.size();

	if !size.is_finite() || size < 0.0 {
		return Err(Error::Native);
	}

	if size > f64::from(options.max_bytes()) {
		return Err(Error::TooLarge);
	}

	let name = file.name();
	let mime = file.type_();

	if name.is_empty() || name.len() > MAX_METADATA_BYTES || mime.len() > MAX_METADATA_BYTES {
		return Err(Error::TooLarge);
	}

	let buffer = JsFuture::from(file.array_buffer())
		.await
		.map_err(|error| failure(&error))?;
	let content = Uint8Array::new(&buffer);

	if content.length() > options.max_bytes() {
		return Err(Error::TooLarge);
	}

	Ok(Document {
		name,
		mime_type: (!mime.is_empty()).then_some(mime),
		bytes: content.to_vec().into(),
	})
}

fn failure(error: &JsValue) -> Error {
	match error
		.dyn_ref::<web_sys::DomException>()
		.map(web_sys::DomException::name)
		.as_deref()
	{
		Some("NotAllowedError" | "SecurityError") => Error::PermissionDenied,
		Some("AbortError") => Error::Cancelled,
		Some("NotSupportedError") => Error::Unavailable,
		_ => Error::Native,
	}
}

#[cfg(test)]
mod tests {
	use wasm_bindgen_test::wasm_bindgen_test;

	use super::*;
	wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

	#[wasm_bindgen_test]
	async fn synthetic_file_is_bounded_and_preserves_binary_content() {
		let parts = js_sys::Array::new();
		parts.push(&Uint8Array::from(&[0_u8, 128, 255][..]));
		let file = web_sys::File::new_with_u8_array_sequence(&parts, "owned-fixture.bin").unwrap();
		assert_eq!(
			read_file(&file, Options::new(2).unwrap()).await,
			Err(Error::TooLarge)
		);
		let result = read_file(&file, Options::new(3).unwrap()).await.unwrap();
		assert_eq!(result.name, "owned-fixture.bin");
		assert_eq!(result.bytes, [0_u8, 128, 255][..]);
	}

	#[wasm_bindgen_test]
	fn native_browser_failures_remain_distinct() {
		for (name, expected) in [
			("NotAllowedError", Error::PermissionDenied),
			("AbortError", Error::Cancelled),
			("NotReadableError", Error::Native),
		] {
			let error =
				web_sys::DomException::new_with_message_and_name("synthetic", name).unwrap();
			assert_eq!(failure(error.as_ref()), expected);
		}
	}
}
