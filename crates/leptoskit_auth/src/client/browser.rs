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

/// The browser's passkeys and page: `navigator.credentials`, and
/// `location`.
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub(crate) struct Browser;

#[cfg(all(feature = "web", target_arch = "wasm32"))]
thread_local! {
	/// Stops the passkey request that's waiting, if any. The browser allows
	/// one at a time, so starting another stops this one first.
	static WAITING: std::cell::RefCell<Option<web_sys::AbortController>> =
		const { std::cell::RefCell::new(None) };
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
impl Browser {
	/// A signal for a new passkey request, after stopping the one waiting.
	fn signal() -> Result<web_sys::AbortSignal, ErrorResponse> {
		let controller = web_sys::AbortController::new().map_err(|_| unavailable())?;
		let signal = controller.signal();
		WAITING.with(|waiting| {
			if let Some(previous) = waiting.borrow_mut().replace(controller) {
				previous.abort();
			}
		});
		Ok(signal)
	}

	fn credentials() -> Result<web_sys::CredentialsContainer, ErrorResponse> {
		let window = web_sys::window().ok_or_else(unavailable)?;
		Ok(window.navigator().credentials())
	}
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn unavailable() -> ErrorResponse {
	super::protocol::error("unsupported", "this browser can't use passkeys")
}

/// What a refused WebAuthn request means: the person said no, or the
/// browser can't.
#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn refused(error: &wasm_bindgen::JsValue) -> ErrorResponse {
	let name = js_sys::Reflect::get(error, &"name".into())
		.ok()
		.and_then(|name| name.as_string())
		.unwrap_or_default();

	match name.as_str() {
		"NotAllowedError" | "AbortError" => {
			super::protocol::error("cancelled", "the passkey request was cancelled")
		}
		"NotSupportedError" | "SecurityError" => unavailable(),
		_ => super::protocol::error("passkey_failed", "that passkey didn't work; try again"),
	}
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
impl super::protocol::Platform for Browser {
	fn create(
		&self,
		options: crate::CreationChallengeResponse,
	) -> BoxFuture<'static, Result<crate::RegisterPublicKeyCredential, ErrorResponse>> {
		Box::pin(send_wrapper::SendWrapper::new(async move {
			use wasm_bindgen::JsCast;

			let credentials = Self::credentials()?;
			let options = web_sys::CredentialCreationOptions::from(options);
			let signal: wasm_bindgen::JsValue = Self::signal()?.into();
			js_sys::Reflect::set(&options, &"signal".into(), &signal).map_err(|_| unavailable())?;
			let promise = credentials
				.create_with_options(&options)
				.map_err(|error| refused(&error))?;
			let credential = wasm_bindgen_futures::JsFuture::from(promise)
				.await
				.map_err(|error| refused(&error))?
				.dyn_into::<web_sys::PublicKeyCredential>()
				.map_err(|_| unavailable())?;
			Ok(crate::RegisterPublicKeyCredential::from(credential))
		}))
	}

	fn get(
		&self,
		options: crate::RequestChallengeResponse,
		autofill: bool,
	) -> BoxFuture<'static, Result<crate::PublicKeyCredential, ErrorResponse>> {
		Box::pin(send_wrapper::SendWrapper::new(async move {
			use wasm_bindgen::JsCast;

			if autofill && !conditional_mediation().await {
				return Err(unavailable());
			}

			let credentials = Self::credentials()?;
			let options = web_sys::CredentialRequestOptions::from(options);
			let signal: wasm_bindgen::JsValue = Self::signal()?.into();
			js_sys::Reflect::set(&options, &"signal".into(), &signal).map_err(|_| unavailable())?;
			let promise = credentials
				.get_with_options(&options)
				.map_err(|error| refused(&error))?;
			let credential = wasm_bindgen_futures::JsFuture::from(promise)
				.await
				.map_err(|error| refused(&error))?
				.dyn_into::<web_sys::PublicKeyCredential>()
				.map_err(|_| unavailable())?;
			Ok(crate::PublicKeyCredential::from(credential))
		}))
	}

	fn cancel(&self) {
		WAITING.with(|waiting| {
			if let Some(controller) = waiting.borrow_mut().take() {
				controller.abort();
			}
		});
	}

	fn open(&self, url: &str) -> Result<(), ErrorResponse> {
		web_sys::window()
			.ok_or_else(unavailable)?
			.location()
			.assign(url)
			.map_err(|_| super::protocol::error("network", "couldn't open the sign-in page"))
	}

	fn query(&self) -> Option<String> {
		web_sys::window()?.location().search().ok()
	}
}

/// Whether the browser offers passkeys in autofill:
/// `PublicKeyCredential.isConditionalMediationAvailable()`.
#[cfg(all(feature = "web", target_arch = "wasm32"))]
async fn conditional_mediation() -> bool {
	let Some(window) = web_sys::window() else {
		return false;
	};
	let Ok(class) = js_sys::Reflect::get(&window, &"PublicKeyCredential".into()) else {
		return false;
	};
	let Ok(check) = js_sys::Reflect::get(&class, &"isConditionalMediationAvailable".into()) else {
		return false;
	};
	let Ok(check) = wasm_bindgen::JsCast::dyn_into::<js_sys::Function>(check) else {
		return false;
	};
	let Ok(promise) = check.call0(&class) else {
		return false;
	};
	let Ok(promise) = wasm_bindgen::JsCast::dyn_into::<js_sys::Promise>(promise) else {
		return false;
	};
	wasm_bindgen_futures::JsFuture::from(promise)
		.await
		.ok()
		.and_then(|available| available.as_bool())
		.unwrap_or(false)
}
