//! Direct Web Share API. Call `share` synchronously from a trusted user action.
//! The browser controls its UI and completion timing; fulfillment never proves
//! delivery. Dropping `ShareRequest` retires the caller, but admission remains
//! busy until the browser promise settles because its UI cannot be dismissed.
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use futures::channel::oneshot;
use js_sys::Function;
use js_sys::Object;
use js_sys::Promise;
use js_sys::Reflect;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use crate::Content;
use crate::Error;

thread_local! { static BUSY: Cell<bool> = const { Cell::new(false) }; }
struct Admission;
impl Admission {
	fn acquire() -> Result<Self, Error> {
		if BUSY.replace(true) {
			Err(Error::Busy)
		} else {
			Ok(Self)
		}
	}
}
impl Drop for Admission {
	fn drop(&mut self) {
		BUSY.set(false);
	}
}

/// One browser request; dropping it cannot dismiss the OS share UI.
pub struct ShareRequest(oneshot::Receiver<Result<(), Error>>);
impl Future for ShareRequest {
	type Output = Result<(), Error>;

	fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
		Pin::new(&mut self.0)
			.poll(context)
			.map(|reply| reply.unwrap_or(Err(Error::Cancelled)))
	}
}

/// Checks secure context and API presence without opening UI. Permissions
/// policy, activation and content support may still reject a subsequent call.
pub fn is_available() -> bool {
	let Some(window) = web_sys::window() else {
		return false;
	};
	window.is_secure_context() && share_function(window.navigator().as_ref()).is_ok()
}

/// Immediately requests native browser share UI, preserving user activation.
/// Await the returned request to observe API completion, never delivery.
pub fn share(content: &Content) -> Result<ShareRequest, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	if !window.is_secure_context() {
		return Err(Error::PermissionDenied);
	}
	begin(window.navigator().as_ref(), content)
}

fn begin(target: &JsValue, content: &Content) -> Result<ShareRequest, Error> {
	let function = share_function(target)?;
	let admission = Admission::acquire()?;
	let data = Object::new();
	let field = if content.is_url() { "url" } else { "text" };
	if !Reflect::set(&data, &field.into(), &content.as_str().into())
		.map_err(|error| failure(&error))?
	{
		return Err(Error::Native);
	}
	let promise = function
		.call1(target, &data)
		.map_err(|error| failure(&error))?
		.dyn_into::<Promise>()
		.map_err(|_| Error::Native)?;
	let (sender, receiver) = oneshot::channel();
	wasm_bindgen_futures::spawn_local(async move {
		let result = JsFuture::from(promise)
			.await
			.map(|_| ())
			.map_err(|error| failure(&error));
		drop(admission);
		let _ = sender.send(result);
	});
	Ok(ShareRequest(receiver))
}

fn share_function(target: &JsValue) -> Result<Function, Error> {
	Reflect::get(target, &"share".into())
		.map_err(|error| failure(&error))?
		.dyn_into::<Function>()
		.map_err(|_| Error::Unavailable)
}

fn failure(error: &JsValue) -> Error {
	match error
		.dyn_ref::<web_sys::DomException>()
		.map(web_sys::DomException::name)
		.as_deref()
	{
		Some("AbortError") => Error::Cancelled,
		Some("NotAllowedError" | "SecurityError") => Error::PermissionDenied,
		Some("InvalidStateError") => Error::Busy,
		Some("NotSupportedError") => Error::Unavailable,
		_ if error.is_instance_of::<js_sys::TypeError>() => Error::InvalidArgument,
		_ => Error::Native,
	}
}

#[cfg(test)]
mod tests {
	use wasm_bindgen::closure::Closure;
	use wasm_bindgen_test::wasm_bindgen_test;

	use super::*;
	wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

	#[wasm_bindgen_test]
	async fn dropped_caller_keeps_browser_admission_until_promise_settles() {
		let mut resolve = None;
		let promise = Promise::new(&mut |yes, _| resolve = Some(yes));
		let fake = Object::new();
		let function = Closure::<dyn FnMut(JsValue) -> Promise>::new(move |data: JsValue| {
			assert_eq!(
				Reflect::get(&data, &"text".into())
					.unwrap()
					.as_string()
					.unwrap(),
				"synthetic fixture"
			);
			promise.clone()
		});
		Reflect::set(&fake, &"share".into(), function.as_ref()).unwrap();
		let content = Content::text("synthetic fixture").unwrap();
		drop(begin(&fake, &content).unwrap());
		assert!(matches!(begin(&fake, &content), Err(Error::Busy)));
		resolve.unwrap().call0(&JsValue::NULL).unwrap();
		JsFuture::from(Promise::resolve(&JsValue::NULL))
			.await
			.unwrap();
		// Poll through the spawned promise waiter before checking admission.
		JsFuture::from(Promise::resolve(&JsValue::NULL))
			.await
			.unwrap();
		assert!(!BUSY.get());
	}

	#[wasm_bindgen_test]
	fn browser_errors_are_not_delivery_results() {
		for (name, expected) in [
			("AbortError", Error::Cancelled),
			("InvalidStateError", Error::Busy),
			("NotAllowedError", Error::PermissionDenied),
		] {
			let error =
				web_sys::DomException::new_with_message_and_name("synthetic", name).unwrap();
			assert_eq!(failure(error.as_ref()), expected);
		}
		assert!(matches!(
			share_function(&Object::new()),
			Err(Error::Unavailable)
		));
	}
}
