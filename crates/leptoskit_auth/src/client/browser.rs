//! The client in the browser. The session lives in the `HttpOnly` cookie,
//! which scripts can't read, so the client holds nothing: each request is a
//! same-origin `fetch` the browser attaches the cookie to.
//!
//! `leptoskit_plugin_http`'s browser backend isn't used here: it omits
//! credentials, so it could never carry the cookie.

use leptoskit_query::BoxFuture;

use super::protocol::Credentials;
use super::protocol::Endpoint;
use super::protocol::Http;
use crate::ErrorResponse;
use crate::Tokens;
use crate::Transport;

/// The browser's credentials: the cookie, which only the browser touches.
pub(crate) struct Cookie;

impl Credentials for Cookie {
	const TRANSPORT: Transport = Transport::Cookie;

	fn authorization<'a, H: Http>(
		&'a self,
		_server: &'a Endpoint<H>,
	) -> BoxFuture<'a, Result<Option<String>, ErrorResponse>> {
		Box::pin(std::future::ready(Ok(None)))
	}

	fn refused<'a, H: Http>(
		&'a self,
		_server: &'a Endpoint<H>,
		_sent: Option<String>,
	) -> BoxFuture<'a, Result<bool, ErrorResponse>> {
		Box::pin(std::future::ready(Ok(false)))
	}

	fn keep(&self, _tokens: Option<Tokens>) -> BoxFuture<'_, Result<(), ErrorResponse>> {
		Box::pin(std::future::ready(Ok(())))
	}

	fn forget(&self) -> BoxFuture<'_, ()> {
		Box::pin(std::future::ready(()))
	}

	// The second step's token is in an `HttpOnly` cookie, too.
	fn hold_second_factor(&self, _token: Option<String>) {}

	fn second_factor(&self) -> Option<String> {
		None
	}
}

/// Sends requests with the browser's `fetch`.
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub(crate) struct Fetch;

#[cfg(all(feature = "web", target_arch = "wasm32"))]
impl Http for Fetch {
	fn send(
		&self,
		request: super::protocol::HttpRequest,
	) -> BoxFuture<'static, Result<super::protocol::HttpResponse, ErrorResponse>> {
		// The browser runs one thread, so its futures needn't be `Send`.
		Box::pin(send_wrapper::SendWrapper::new(fetch(request)))
	}
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
async fn fetch(
	request: super::protocol::HttpRequest,
) -> Result<super::protocol::HttpResponse, ErrorResponse> {
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;
	use wasm_bindgen_futures::JsFuture;

	let offline =
		|_: JsValue| super::protocol::error("network", "couldn't reach the server; try again");
	let init = web_sys::RequestInit::new();
	init.set_method(request.method.as_str());
	// The cookie goes along, and never leaves the page's origin.
	init.set_credentials(web_sys::RequestCredentials::SameOrigin);
	init.set_mode(web_sys::RequestMode::SameOrigin);
	let headers = web_sys::Headers::new().map_err(offline)?;

	for (name, value) in request.headers() {
		headers.set(name, value).map_err(offline)?;
	}

	init.set_headers(&headers);

	if let Some(body) = &request.body {
		init.set_body(&JsValue::from_str(body));
	}

	let window = web_sys::window().ok_or_else(|| {
		super::protocol::error("config", "the browser client needs a window to fetch from")
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

	Ok(super::protocol::HttpResponse {
		status: response.status(),
		body: text.as_string().unwrap_or_default().into_bytes(),
	})
}
