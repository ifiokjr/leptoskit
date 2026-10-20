//! Browser file selection through `showOpenFilePicker`.
//!
//! This API is not supported by every browser. Unsupported browsers return
//! [`Error::Unavailable`]; applications can offer an HTML file input
//! themselves. Call from a user gesture in a secure context. Files are
//! capabilities provided by the browser, not fabricated local filesystem paths.

use js_sys::Array;
use js_sys::Function;
use js_sys::Object;
use js_sys::Promise;
use js_sys::Reflect;
use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use crate::Error;
use crate::FileFilter;

/// Selects files without reading their contents.
///
/// `None` means the picker was aborted, either by the user or by the browser
/// rejecting sensitive files. All other browser errors are returned explicitly.
/// Dropping the future does not dismiss a picker already opened by the browser.
pub async fn open_files(
	multiple: bool,
	filters: &[FileFilter],
) -> Result<Option<Vec<web_sys::File>>, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	if !window.is_secure_context() {
		return Err(Error::PermissionDenied);
	}
	let picker = function(window.as_ref(), "showOpenFilePicker")?;
	let options = options(multiple, filters)?;
	let result = picker
		.call1(window.as_ref(), &options)
		.map_err(|error| failure(&error))?;
	let promise = result.dyn_into::<Promise>().map_err(|_| Error::Native)?;
	let result = match JsFuture::from(promise).await {
		Ok(result) => result,
		Err(error) if exception_name(&error).as_deref() == Some("AbortError") => return Ok(None),
		Err(error) => return Err(failure(&error)),
	};
	let handles = result.dyn_into::<Array>().map_err(|_| Error::Native)?;
	if handles.length() == 0 || (!multiple && handles.length() > 1) {
		return Err(Error::Native);
	}
	let mut files = Vec::with_capacity(handles.length() as usize);
	for handle in handles.iter() {
		let promise = function(&handle, "getFile")?
			.call0(&handle)
			.map_err(|error| failure(&error))?
			.dyn_into::<Promise>()
			.map_err(|_| Error::Native)?;
		let file = JsFuture::from(promise)
			.await
			.map_err(|error| failure(&error))?
			.dyn_into::<web_sys::File>()
			.map_err(|_| Error::Native)?;
		files.push(file);
	}
	Ok(Some(files))
}

/// Reads a selected browser file only when its size fits the caller's limit.
///
/// Browser I/O rejection is propagated. At most `max_bytes` bytes are copied to
/// wasm memory. For large files, use the browser's streaming APIs instead.
pub async fn read_file(file: &web_sys::File, max_bytes: u32) -> Result<Vec<u8>, Error> {
	let size = file.size();
	if !size.is_finite() || size < 0.0 {
		return Err(Error::Native);
	}
	if size > f64::from(max_bytes) {
		return Err(Error::TooLarge);
	}
	let buffer = JsFuture::from(file.array_buffer())
		.await
		.map_err(|error| failure(&error))?
		.dyn_into::<js_sys::ArrayBuffer>()
		.map_err(|_| Error::Native)?;
	let bytes = Uint8Array::new(&buffer);
	if bytes.length() > max_bytes {
		return Err(Error::TooLarge);
	}
	Ok(bytes.to_vec())
}

fn options(multiple: bool, filters: &[FileFilter]) -> Result<Object, Error> {
	let options = Object::new();
	set(&options, "multiple", &JsValue::from_bool(multiple))?;
	let types = Array::new();
	for filter in filters {
		let category = Object::new();
		set(&category, "description", &JsValue::from_str(filter.name()))?;
		let accept = Object::new();
		let extensions = Array::new();
		for extension in filter.extensions() {
			// The browser requires a leading dot and at most 16 characters.
			if extension.len() > 15 {
				return Err(Error::InvalidOption);
			}
			extensions.push(&JsValue::from_str(&format!(".{extension}")));
		}
		set(&accept, "application/octet-stream", &extensions)?;
		set(&category, "accept", &accept)?;
		types.push(&category);
	}
	if !filters.is_empty() {
		set(&options, "types", &types)?;
	}
	Ok(options)
}

fn set(object: &Object, key: &str, value: &JsValue) -> Result<(), Error> {
	if Reflect::set(object, &JsValue::from_str(key), value).map_err(|error| failure(&error))? {
		Ok(())
	} else {
		Err(Error::Native)
	}
}

fn function(object: &JsValue, name: &str) -> Result<Function, Error> {
	Reflect::get(object, &JsValue::from_str(name))
		.map_err(|error| failure(&error))?
		.dyn_into::<Function>()
		.map_err(|_| Error::Unavailable)
}

fn exception_name(value: &JsValue) -> Option<String> {
	value
		.dyn_ref::<web_sys::DomException>()
		.map(web_sys::DomException::name)
}

fn failure(value: &JsValue) -> Error {
	match exception_name(value).as_deref() {
		Some("NotAllowedError" | "SecurityError") => Error::PermissionDenied,
		Some("NotSupportedError") => Error::Unavailable,
		_ if value.is_instance_of::<js_sys::TypeError>() => Error::InvalidOption,
		_ => Error::Native,
	}
}

#[cfg(test)]
mod tests {
	use wasm_bindgen_test::wasm_bindgen_test;

	use super::*;

	wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

	#[wasm_bindgen_test]
	fn options_preserve_extensions_and_reject_browser_length_limit() {
		let filter = FileFilter::new("Images", ["png", "JPG"]).unwrap();
		let value = options(true, &[filter]).unwrap();
		assert_eq!(
			Reflect::get(&value, &"multiple".into()).unwrap(),
			JsValue::TRUE
		);
		let types = Reflect::get(&value, &"types".into())
			.unwrap()
			.dyn_into::<Array>()
			.unwrap();
		let accept = Reflect::get(&types.get(0), &"accept".into()).unwrap();
		let extensions = Reflect::get(&accept, &"application/octet-stream".into())
			.unwrap()
			.dyn_into::<Array>()
			.unwrap();
		assert_eq!(extensions.get(0), ".png");
		assert_eq!(extensions.get(1), ".JPG");
		let too_long = FileFilter::new("Invalid", ["sixteencharacter"]).unwrap();
		assert!(matches!(
			options(false, &[too_long]),
			Err(Error::InvalidOption)
		));
	}

	#[wasm_bindgen_test]
	fn permission_errors_are_not_reported_as_generic_native_failure() {
		for name in ["SecurityError", "NotAllowedError"] {
			let exception =
				web_sys::DomException::new_with_message_and_name("denied", name).unwrap();
			assert_eq!(failure(exception.as_ref()), Error::PermissionDenied);
		}
		let exception =
			web_sys::DomException::new_with_message_and_name("read failed", "NotReadableError")
				.unwrap();
		assert_eq!(failure(exception.as_ref()), Error::Native);
		assert_eq!(
			failure(js_sys::TypeError::new("bad option").as_ref()),
			Error::InvalidOption
		);
	}

	#[wasm_bindgen_test]
	async fn reading_a_file_enforces_the_byte_limit() {
		let parts = Array::new();
		parts.push(&Uint8Array::from(&b"abc"[..]));
		let file = web_sys::File::new_with_u8_array_sequence(&parts, "test.txt").unwrap();
		assert_eq!(read_file(&file, 2).await, Err(Error::TooLarge));
		assert_eq!(read_file(&file, 3).await.unwrap(), b"abc");
	}
}
