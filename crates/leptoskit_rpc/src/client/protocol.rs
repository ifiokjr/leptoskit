//! The client every remote platform shares: `POST /api/<name>` over
//! whatever sends HTTP there, signed by whatever keeps this device's
//! credentials.

#![allow(
	dead_code,
	reason = "each platform's build uses its own part of this, and the tests all of it"
)]

use std::sync::Arc;
use std::time::Duration;

use leptoskit_query::BoxFuture;
use serde_json::Value;

use super::Api;
use super::ClientConfig;
use super::Key;
use crate::FnError;

/// A request to an operation's route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HttpRequest {
	/// The origin and path, or the path alone for the page's own origin.
	pub url: String,
	/// `Bearer …`, for a native app.
	pub authorization: Option<String>,
	/// A mutation's idempotency key.
	pub idempotency_key: Option<String>,
	/// The arguments, as a JSON object.
	pub body: String,
}

impl HttpRequest {
	/// The headers every sender puts on the request.
	pub(crate) fn headers(&self) -> Vec<(&'static str, &str)> {
		let mut headers = vec![
			("accept", "application/json"),
			("content-type", "application/json"),
		];

		if let Some(authorization) = &self.authorization {
			headers.push(("authorization", authorization));
		}

		if let Some(key) = &self.idempotency_key {
			headers.push(("idempotency-key", key));
		}

		headers
	}
}

/// What came back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HttpResponse {
	pub status: u16,
	pub body: Vec<u8>,
}

/// Sends requests: `fetch` in the browser, the HTTP plugin in a native app.
pub(crate) trait Http: Send + Sync + 'static {
	/// Sends `request` as a `POST`.
	fn send(&self, request: HttpRequest) -> BoxFuture<'static, Result<HttpResponse, FnError>>;
}

/// Signs requests with this device's credentials, such as a native app's
/// bearer token. `leptoskit_auth` implements it for its client.
pub trait Credentials: Send + Sync + 'static {
	/// The `Authorization` header for the next request, if any. Renew first
	/// when what it holds is about to run out.
	///
	/// # Errors
	///
	/// When the credentials can't be read, such as a secure store that
	/// refuses.
	fn authorization(&self) -> BoxFuture<'_, Result<Option<String>, FnError>>;

	/// After the server answered `401` to a request signed with `sent`:
	/// renew, and say whether the request is worth sending again.
	///
	/// # Errors
	///
	/// As [`authorization`](Self::authorization).
	fn refused(&self, sent: Option<String>) -> BoxFuture<'_, Result<bool, FnError>> {
		let _ = sent;
		Box::pin(std::future::ready(Ok(false)))
	}
}

/// How many times a mutation with an idempotency key is sent when the
/// network drops it, in all.
const KEYED_ATTEMPTS: u32 = 3;

/// The client over the routes, for one platform's HTTP.
pub(crate) struct Remote<H> {
	inner: Arc<Inner<H>>,
}

struct Inner<H> {
	origin: String,
	http: H,
	credentials: Option<Arc<dyn Credentials>>,
}

impl<H: Http> Remote<H> {
	pub(crate) fn new(config: ClientConfig, http: H) -> Self {
		Self {
			inner: Arc::new(Inner {
				origin: config
					.origin
					.unwrap_or_default()
					.trim_end_matches('/')
					.to_owned(),
				http,
				credentials: config.credentials,
			}),
		}
	}
}

impl<H: Http> Inner<H> {
	async fn authorization(&self) -> Result<Option<String>, FnError> {
		match &self.credentials {
			Some(credentials) => credentials.authorization().await,
			None => Ok(None),
		}
	}

	/// Sends one request, signed; when the server refuses the credentials,
	/// renews them and sends it once more.
	async fn send(
		&self,
		url: &str,
		body: &str,
		idempotency_key: Option<&String>,
	) -> Result<HttpResponse, FnError> {
		let request = |authorization| {
			HttpRequest {
				url: url.to_owned(),
				authorization,
				idempotency_key: idempotency_key.cloned(),
				body: body.to_owned(),
			}
		};
		let authorization = self.authorization().await?;
		let response = self.http.send(request(authorization.clone())).await?;

		if response.status != 401 {
			return Ok(response);
		}

		let renewed = match &self.credentials {
			Some(credentials) => credentials.refused(authorization).await?,
			None => false,
		};

		if !renewed {
			return Ok(response);
		}

		let authorization = self.authorization().await?;
		self.http.send(request(authorization)).await
	}

	async fn call(
		&self,
		name: &'static str,
		arguments: Value,
		idempotency_key: Option<String>,
	) -> Result<Value, FnError> {
		let url = format!("{}/api/{name}", self.origin);
		let body = arguments.to_string();
		// Only a keyed call is safe to send again: the server runs it once.
		let attempts = if idempotency_key.is_some() {
			KEYED_ATTEMPTS
		} else {
			1
		};

		let mut attempt = 0;

		let response = loop {
			attempt += 1;

			match self.send(&url, &body, idempotency_key.as_ref()).await {
				Err(FnError::Network(_)) if attempt < attempts => {
					leptoskit_query::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
				}

				outcome => break outcome?,
			}
		};

		decode(&response)
	}
}

impl<H: Http> Api for Remote<H> {
	fn call(
		&self,
		name: &'static str,
		arguments: Value,
		key: Key,
	) -> BoxFuture<'static, Result<Value, FnError>> {
		let inner = Arc::clone(&self.inner);
		Box::pin(async move { inner.call(name, arguments, key.resolve().await).await })
	}
}

/// A successful answer's value, or the error the server sent instead, as
/// [`FnError::from_wire`] classifies it.
pub(crate) fn decode(response: &HttpResponse) -> Result<Value, FnError> {
	let text = String::from_utf8_lossy(&response.body);

	if !(200..300).contains(&response.status) {
		if text.trim().is_empty() {
			return Err(FnError::Other(format!(
				"the server answered HTTP {}",
				response.status
			)));
		}

		return Err(FnError::from_wire(&text));
	}

	if text.trim().is_empty() {
		return Ok(Value::Null);
	}

	serde_json::from_str(&text)
		.map_err(|error| FnError::Decode(format!("the answer wasn't JSON: {error}")))
}

/// Where there's no way to reach the server.
pub(crate) struct Unreachable;

impl Http for Unreachable {
	fn send(&self, _request: HttpRequest) -> BoxFuture<'static, Result<HttpResponse, FnError>> {
		Box::pin(std::future::ready(Err(FnError::Other(
			"this build can't reach the server: turn on leptoskit_rpc's `web` feature for the \
			 browser, `guest` for a native app, or `server` with a database for server rendering"
				.to_owned(),
		))))
	}
}
