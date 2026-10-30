//! How requests reach the server. Each host sends them its own way, behind
//! one trait, so the API client is the same code everywhere.

use std::future::Future;
use std::pin::Pin;

/// A request to Commonplace's server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
	pub method: &'static str,
	/// The path and query, such as `/api/projects`.
	pub path: String,
	/// A JSON body.
	pub body: Option<Vec<u8>>,
	/// The session token, for hosts that send it as a bearer token.
	pub bearer: Option<String>,
}

/// The server's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
	pub status: u16,
	pub body: Vec<u8>,
}

/// A transport's future.
pub type Sending = Pin<Box<dyn Future<Output = Result<Response, String>>>>;

/// Sends requests to the server.
pub trait Transport {
	/// Sends one request. A failure is a sentence for people: the request
	/// never got an answer.
	fn send(&self, request: Request) -> Sending;

	/// Whether this host signs in with a bearer token it keeps itself, rather
	/// than the browser's session cookie.
	fn uses_bearer(&self) -> bool;
}

/// The website: `fetch` on the page's own origin, sending the session
/// cookie the server set at sign-in. Scripts can't read that cookie, so the
/// website never handles the token at all.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub struct Fetch;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
impl Transport for Fetch {
	fn send(&self, request: Request) -> Sending {
		Box::pin(async move {
			use wasm_bindgen::JsCast;
			use wasm_bindgen_futures::JsFuture;

			let failed = |_| "Commonplace couldn't reach its server.".to_owned();
			let init = web_sys::RequestInit::new();
			init.set_method(request.method);
			init.set_credentials(web_sys::RequestCredentials::SameOrigin);
			let headers = web_sys::Headers::new().map_err(failed)?;
			if let Some(body) = &request.body {
				headers
					.set("content-type", "application/json")
					.map_err(failed)?;
				init.set_body(&js_sys::Uint8Array::from(body.as_slice()).into());
			}
			init.set_headers(&headers.into());
			let window = web_sys::window().ok_or("no window")?;
			let response = JsFuture::from(window.fetch_with_str_and_init(&request.path, &init))
				.await
				.map_err(failed)?
				.dyn_into::<web_sys::Response>()
				.map_err(failed)?;
			let body = JsFuture::from(response.array_buffer().map_err(failed)?)
				.await
				.map_err(failed)?;
			Ok(Response {
				status: response.status(),
				body: js_sys::Uint8Array::new(&body).to_vec(),
			})
		})
	}

	fn uses_bearer(&self) -> bool {
		false
	}
}

/// The native apps: the HTTP plugin, to the server named at build time.
#[cfg(all(target_arch = "wasm32", feature = "native", not(feature = "web")))]
pub struct Plugin {
	pub server: String,
}

#[cfg(all(target_arch = "wasm32", feature = "native", not(feature = "web")))]
impl Transport for Plugin {
	fn send(&self, request: Request) -> Sending {
		use leptoskit_plugin_http::Header;
		use leptoskit_plugin_http::Method;

		let url = format!("{}{}", self.server, request.path);
		Box::pin(async move {
			let client = leptoskit_plugin_http::guest::HttpClient::connect()
				.map_err(|error| format!("The HTTP plugin isn't available: {error}"))?;
			let mut headers = vec![Header {
				name: "accept".into(),
				value: b"application/json".to_vec(),
			}];
			if let Some(token) = &request.bearer {
				headers.push(Header {
					name: "authorization".into(),
					value: format!("Bearer {token}").into_bytes(),
				});
			}
			if request.body.is_some() {
				headers.push(Header {
					name: "content-type".into(),
					value: b"application/json".to_vec(),
				});
			}
			let method = match request.method {
				"POST" => Method::Post,
				"PATCH" => Method::Patch,
				"DELETE" => Method::Delete,
				_ => Method::Get,
			};
			let response = client
				.execute(leptoskit_plugin_http::Request {
					method,
					url,
					headers,
					body: request.body.unwrap_or_default(),
				})
				.await
				.map_err(|error| format!("Commonplace couldn't reach its server: {error}"))?;
			Ok(Response {
				status: response.status,
				body: response.body,
			})
		})
	}

	fn uses_bearer(&self) -> bool {
		true
	}
}
