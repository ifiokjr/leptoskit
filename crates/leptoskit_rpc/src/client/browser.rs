//! The client in the browser: a same-origin `fetch`, which carries the
//! session cookie, so the client holds nothing.
//!
//! `leptoskit_plugin_http`'s browser backend isn't used here: it omits
//! credentials, so it could never carry the cookie.

#[cfg(all(feature = "web", target_arch = "wasm32"))]
use leptoskit_query::BoxFuture;

#[cfg(all(feature = "web", target_arch = "wasm32"))]
use super::protocol::Http;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
use super::protocol::HttpRequest;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
use super::protocol::HttpResponse;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
use crate::FnError;

/// Sends requests with the browser's `fetch`.
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub(crate) struct Fetch;

#[cfg(all(feature = "web", target_arch = "wasm32"))]
impl Http for Fetch {
	fn send(&self, request: HttpRequest) -> BoxFuture<'static, Result<HttpResponse, FnError>> {
		// The browser runs one thread, so its futures needn't be `Send`.
		Box::pin(send_wrapper::SendWrapper::new(fetch(request)))
	}
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
async fn fetch(request: HttpRequest) -> Result<HttpResponse, FnError> {
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;
	use wasm_bindgen_futures::JsFuture;

	let offline = |_: JsValue| FnError::Network("couldn't reach the server; try again".to_owned());
	let init = web_sys::RequestInit::new();
	init.set_method("POST");
	// The cookie goes along, and never leaves the page's origin.
	init.set_credentials(web_sys::RequestCredentials::SameOrigin);
	let headers = web_sys::Headers::new().map_err(offline)?;

	for (name, value) in request.headers() {
		headers.set(name, value).map_err(offline)?;
	}

	init.set_headers(&headers);
	init.set_body(&JsValue::from_str(&request.body));

	let window = web_sys::window().ok_or_else(|| {
		FnError::Other("the browser client needs a window to fetch from".to_owned())
	})?;
	let response: web_sys::Response =
		JsFuture::from(window.fetch_with_str_and_init(&request.url, &init))
			.await
			.map_err(offline)?
			.dyn_into()
			.map_err(offline)?;
	let text = JsFuture::from(response.text().map_err(offline)?)
		.await
		.map_err(offline)?;

	Ok(HttpResponse {
		status: response.status(),
		body: text.as_string().unwrap_or_default().into_bytes(),
	})
}
