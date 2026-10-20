use futures::future::Either;
use futures::future::select;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use super::*;

/// Browser fetch client. CORS, browser forbidden headers, and TLS policy remain
/// enforced. Ambient cookies and automatic redirects are disabled.
#[derive(Debug, Clone)]
pub struct Client {
	config: Config,
}
struct Abort(web_sys::AbortController);
impl Drop for Abort {
	fn drop(&mut self) {
		self.0.abort();
	}
}

impl Client {
	/// Validates explicit destination scopes and resource limits.
	pub fn new(config: Config) -> Result<Self, Error> {
		config.validate()?;
		Ok(Self { config })
	}

	/// Fetches a bounded body incrementally. Dropping this future aborts fetch
	/// and body streaming rather than merely ignoring a later result.
	pub async fn execute(&self, request: Request) -> Result<Response, Error> {
		let url = self.config.request_url(&request)?;
		let window = web_sys::window().ok_or(Error::Unavailable)?;
		let abort = Abort(web_sys::AbortController::new().map_err(js_error)?);
		let init = web_sys::RequestInit::new();
		init.set_method(request.method.as_str());
		init.set_mode(web_sys::RequestMode::Cors);
		init.set_credentials(web_sys::RequestCredentials::Omit);
		init.set_redirect(web_sys::RequestRedirect::Error);
		init.set_signal(Some(&abort.0.signal()));
		if !request.body.is_empty() {
			init.set_body(&js_sys::Uint8Array::from(request.body.as_slice()));
		}
		let headers = web_sys::Headers::new().map_err(js_error)?;
		for header in request.headers {
			let name = header.name.to_ascii_lowercase();
			if name.starts_with("proxy-")
				|| name.starts_with("sec-")
				|| matches!(
					name.as_str(),
					"accept-charset"
						| "accept-encoding"
						| "access-control-request-headers"
						| "access-control-request-method"
						| "cookie"
						| "cookie2"
						| "date"
						| "dnt"
						| "expect"
						| "keep-alive"
						| "origin"
						| "permissions-policy"
						| "referer"
						| "te"
						| "trailer"
						| "user-agent"
						| "via"
				) {
				return Err(Error::Invalid("browser-owned request header".into()));
			}
			let value = std::str::from_utf8(&header.value)
				.map_err(|_| Error::Invalid("browser header values require UTF-8".into()))?;
			headers.append(&header.name, value).map_err(js_error)?;
		}
		init.set_headers(&headers);
		let request =
			web_sys::Request::new_with_str_and_init(url.as_str(), &init).map_err(js_error)?;
		let fetch = async {
			let response = JsFuture::from(window.fetch_with_request(&request))
				.await
				.map_err(js_error)?
				.dyn_into::<web_sys::Response>()
				.map_err(js_error)?;
			let mut headers = Vec::new();
			let mut header_bytes = 0_usize;
			let iterator = js_sys::try_iter(response.headers().as_ref())
				.map_err(js_error)?
				.ok_or_else(|| Error::Transport("headers are not iterable".into()))?;
			for entry in iterator {
				let pair = js_sys::Array::from(&entry.map_err(js_error)?);
				let name = pair
					.get(0)
					.as_string()
					.ok_or_else(|| Error::Transport("invalid response header".into()))?;
				let value = pair
					.get(1)
					.as_string()
					.ok_or_else(|| Error::Transport("invalid response header".into()))?;
				header_bytes = header_bytes
					.checked_add(name.len())
					.and_then(|size| size.checked_add(value.len()))
					.ok_or(Error::TooLarge)?;
				if header_bytes > self.config.max_header_bytes {
					return Err(Error::TooLarge);
				}
				headers.push(Header {
					name,
					value: value.into_bytes(),
				});
			}
			let mut result = Response {
				status: response.status(),
				url: response.url(),
				headers,
				body: Vec::new(),
			};
			if let Some(body) = response.body() {
				let reader = body
					.get_reader()
					.dyn_into::<web_sys::ReadableStreamDefaultReader>()
					.map_err(|value| js_error(value.into()))?;
				loop {
					let read = JsFuture::from(reader.read()).await.map_err(js_error)?;
					let done = js_sys::Reflect::get(&read, &JsValue::from_str("done"))
						.map_err(js_error)?;
					if done.as_bool() == Some(true) {
						break;
					}
					let value = js_sys::Reflect::get(&read, &JsValue::from_str("value"))
						.map_err(js_error)?;
					let chunk = value.dyn_into::<js_sys::Uint8Array>().map_err(js_error)?;
					if chunk.length() as usize
						> self.config.max_body_bytes.saturating_sub(result.body.len())
					{
						return Err(Error::TooLarge);
					}
					let start = result.body.len();
					result.body.resize(start + chunk.length() as usize, 0);
					chunk.copy_to(&mut result.body[start..]);
				}
				reader.release_lock();
			}
			Ok(result)
		};
		let timeout =
			gloo_timers::future::TimeoutFuture::new(self.config.timeout.as_millis() as u32);
		futures::pin_mut!(fetch, timeout);
		let result = match select(fetch, timeout).await {
			Either::Left((result, _)) => result,
			Either::Right(((), _)) => Err(Error::Timeout),
		};
		drop(abort);
		result
	}
}
fn js_error(value: JsValue) -> Error {
	Error::Transport(value.dyn_into::<js_sys::Error>().map_or_else(
		|_| "browser request failed".into(),
		|error| error.message().into(),
	))
}
