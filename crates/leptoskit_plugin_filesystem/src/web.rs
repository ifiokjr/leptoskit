//! Origin-private storage. Requires a secure context and OPFS support.
//!
//! Each call opens only handles beneath the origin's `leptoskit` directory.
//! Writes use a browser writable-file transaction and commit on `close`.
//! Rename requires the browser's `FileSystemHandle.move` implementation.
use js_sys::Array;
use js_sys::Function;
use js_sys::Promise;
use js_sys::Reflect;
use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use super::*;

// Consumes the platform error at the intentionally small bridge error boundary.
#[allow(clippy::needless_pass_by_value)]
fn failure(error: JsValue) -> Error {
	match error
		.dyn_ref::<web_sys::DomException>()
		.map(web_sys::DomException::name)
		.as_deref()
	{
		Some("NotAllowedError" | "SecurityError") => Error::PermissionDenied,
		Some("NotSupportedError") => Error::Unavailable,
		Some("QuotaExceededError") => Error::TooLarge,
		Some("TypeMismatchError" | "InvalidModificationError") => Error::InvalidArgument,
		_ => Error::Native,
	}
}

fn get(object: &JsValue, key: &str) -> Result<JsValue, Error> {
	Reflect::get(object, &key.into()).map_err(failure)
}

async fn call(object: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
	let method = Reflect::get(object, &name.into())?;
	let function = method.dyn_into::<Function>().map_err(|_| {
		web_sys::DomException::new_with_message_and_name(
			"OPFS method unavailable",
			"NotSupportedError",
		)
		.map_or(JsValue::UNDEFINED, Into::into)
	})?;
	let arguments = Array::new();

	for arg in args {
		arguments.push(arg);
	}

	let promise = function.apply(object, &arguments)?.dyn_into::<Promise>()?;
	JsFuture::from(promise).await
}

fn options(create: bool) -> Result<JsValue, Error> {
	let options = js_sys::Object::new();
	Reflect::set(&options, &"create".into(), &create.into()).map_err(failure)?;
	Ok(options.into())
}

async fn root() -> Result<JsValue, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;

	if !window.is_secure_context() {
		return Err(Error::PermissionDenied);
	}

	let storage = get(window.navigator().as_ref(), "storage")?;

	if storage.is_undefined() {
		return Err(Error::Unavailable);
	}

	let root = call(&storage, "getDirectory", &[]).await.map_err(failure)?;
	call(
		&root,
		"getDirectoryHandle",
		&["leptoskit".into(), options(true)?],
	)
	.await
	.map_err(failure)
}

async fn directory(path: &str, create: bool) -> Result<JsValue, Error> {
	validate_path(path, true)?;
	let mut handle = root().await?;

	if !path.is_empty() {
		for name in path.split('/') {
			handle = call(
				&handle,
				"getDirectoryHandle",
				&[name.into(), options(create)?],
			)
			.await
			.map_err(failure)?;
		}
	}

	Ok(handle)
}

async fn parent(path: &str) -> Result<(JsValue, &str), Error> {
	validate_path(path, false)?;
	let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
	Ok((directory(parent, false).await?, name))
}

async fn file(path: &str, create: bool) -> Result<JsValue, Error> {
	let (parent, name) = parent(path).await?;
	call(&parent, "getFileHandle", &[name.into(), options(create)?])
		.await
		.map_err(failure)
}

fn byte_length(value: &JsValue) -> Result<u64, Error> {
	let value = get(value, "size")?.as_f64().ok_or(Error::Native)?;

	if !(0.0..=js_sys::Number::MAX_SAFE_INTEGER).contains(&value) || value.fract() != 0.0 {
		return Err(Error::Native);
	}

	// Validated nonnegative safe integer, representable by u64.
	#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
	Ok(value as u64)
}

/// Reads at most [`MAX_FILE_BYTES`] bytes from an origin-private regular file.
pub async fn read(path: &str) -> Result<Bytes, Error> {
	let snapshot = call(&file(path, false).await?, "getFile", &[])
		.await
		.map_err(failure)?;

	if byte_length(&snapshot)? > MAX_FILE_BYTES as u64 {
		return Err(Error::TooLarge);
	}

	let buffer = call(&snapshot, "arrayBuffer", &[]).await.map_err(failure)?;
	let bytes = Uint8Array::new(&buffer);

	if bytes.length() as usize > MAX_FILE_BYTES {
		return Err(Error::TooLarge);
	}

	Ok(bytes.to_vec().into())
}

/// Commits a file replacement through an OPFS writable transaction. The browser
/// may reject quota exhaustion or concurrent writer contention.
pub async fn write(path: &str, bytes: &[u8]) -> Result<(), Error> {
	validate_path(path, false)?;

	if bytes.len() > MAX_FILE_BYTES {
		return Err(Error::TooLarge);
	}

	let stream = call(&file(path, true).await?, "createWritable", &[])
		.await
		.map_err(failure)?;
	let result = call(&stream, "write", &[Uint8Array::from(bytes).into()]).await;

	if let Err(error) = result {
		// Abort releases the browser transaction. Preserve the original write
		// error.
		let _ = call(&stream, "abort", &[]).await;

		return Err(failure(error));
	}

	call(&stream, "close", &[]).await.map_err(failure)?;
	Ok(())
}

/// Lists sorted immediate names, failing rather than truncating large results.
pub async fn list(path: &str) -> Result<Vec<String>, Error> {
	let dir = directory(path, false).await?;
	let method = get(&dir, "keys")?
		.dyn_into::<Function>()
		.map_err(|_| Error::Unavailable)?;
	let iterator = method.call0(&dir).map_err(failure)?;
	let mut names = Vec::new();
	let mut size = 4;

	loop {
		let next = call(&iterator, "next", &[]).await.map_err(failure)?;

		if get(&next, "done")?.as_bool() == Some(true) {
			break;
		}

		let name = get(&next, "value")?.as_string().ok_or(Error::Native)?;
		validate_path(&name, false)?;

		if name.contains('/') {
			return Err(Error::Native);
		}

		size += name.len() + 4;

		if names.len() >= MAX_ENTRIES || size > MAX_LIST_BYTES {
			return Err(Error::TooLarge);
		}

		names.push(name);
	}

	names.sort_unstable();
	Ok(names)
}

/// Creates directories recursively. Existing directories succeed.
pub async fn mkdir(path: &str) -> Result<(), Error> {
	validate_path(path, false)?;
	directory(path, true).await?;
	Ok(())
}

/// Removes a file or empty directory, never recursively.
pub async fn remove(path: &str) -> Result<(), Error> {
	let (parent, name) = parent(path).await?;
	call(&parent, "removeEntry", &[name.into()])
		.await
		.map_err(failure)?;
	Ok(())
}

/// Queries regular-file length or directory type.
pub async fn stat(path: &str) -> Result<Metadata, Error> {
	validate_path(path, true)?;

	if path.is_empty() {
		root().await?;
		return Ok(Metadata {
			kind: Kind::Directory,
			len: 0,
		});
	}

	let (parent, name) = parent(path).await?;

	match call(&parent, "getFileHandle", &[name.into()]).await {
		Ok(handle) => {
			let file = call(&handle, "getFile", &[]).await.map_err(failure)?;
			Ok(Metadata {
				kind: Kind::File,
				len: byte_length(&file)?,
			})
		}
		Err(error)
			if error
				.dyn_ref::<web_sys::DomException>()
				.is_some_and(|error| error.name() == "TypeMismatchError") =>
		{
			call(&parent, "getDirectoryHandle", &[name.into()])
				.await
				.map_err(failure)?;
			Ok(Metadata {
				kind: Kind::Directory,
				len: 0,
			})
		}
		Err(error) => Err(failure(error)),
	}
}

/// Moves a regular file within OPFS when `FileSystemFileHandle.move` is
/// available. Directory moves and browsers lacking this method return
/// [`Error::Unavailable`]; no non-atomic copy/delete fallback is performed.
pub async fn rename(source: &str, destination: &str) -> Result<(), Error> {
	validate_path(source, false)?;
	validate_path(destination, false)?;

	if stat(source).await?.kind != Kind::File {
		return Err(Error::Unavailable);
	}

	let file = file(source, false).await?;
	let (parent, name) = parent(destination).await?;
	call(&file, "move", &[parent, name.into()])
		.await
		.map_err(failure)?;
	Ok(())
}
