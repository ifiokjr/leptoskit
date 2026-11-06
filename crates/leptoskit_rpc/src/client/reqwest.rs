//! HTTP from a server, to another service: `reqwest` on the server's own
//! tokio runtime.

use std::sync::OnceLock;
use std::time::Duration;

use leptoskit_query::BoxFuture;

use super::protocol::Http;
use super::protocol::HttpRequest;
use super::protocol::HttpResponse;
use crate::FnError;

/// How long a call to another service may take, connecting included.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Sends requests with one `reqwest` client the whole process shares, so
/// connections to a service are reused.
#[derive(Clone)]
pub(crate) struct Reqwest(reqwest::Client);

impl Reqwest {
	/// The process's client.
	pub(crate) fn shared() -> Self {
		static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
		Self(
			CLIENT
				.get_or_init(|| {
					reqwest::Client::builder()
						.timeout(TIMEOUT)
						.build()
						.unwrap_or_default()
				})
				.clone(),
		)
	}
}

impl Http for Reqwest {
	fn send(&self, request: HttpRequest) -> BoxFuture<'static, Result<HttpResponse, FnError>> {
		let client = self.0.clone();
		Box::pin(async move {
			let unreachable = |error: reqwest::Error| {
				FnError::Network(format!("couldn't reach the service: {error}"))
			};
			let mut builder = client.post(&request.url);

			for (name, value) in request.headers() {
				builder = builder.header(name, value);
			}

			let response = builder
				.body(request.body.clone())
				.send()
				.await
				.map_err(unreachable)?;
			let status = response.status().as_u16();
			let body = response.bytes().await.map_err(unreachable)?.to_vec();
			Ok(HttpResponse { status, body })
		})
	}
}
