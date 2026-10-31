//! The client in a native app's Wasm guest: requests go through the HTTP
//! plugin, signed by the credentials the app configured, such as
//! `leptoskit_auth`'s bearer tokens.

use leptoskit_query::BoxFuture;

use super::protocol::Http;
use super::protocol::HttpRequest;
use super::protocol::HttpResponse;
use crate::FnError;

/// Sends requests through `leptoskit_plugin_http`.
pub(crate) struct Plugin;

impl Http for Plugin {
	fn send(&self, request: HttpRequest) -> BoxFuture<'static, Result<HttpResponse, FnError>> {
		use leptoskit_plugin_http::Header;
		use leptoskit_plugin_http::Method;
		use leptoskit_plugin_http::guest::HttpClient;

		Box::pin(async move {
			if request.url.starts_with('/') {
				return Err(FnError::Other(
					"a native app needs the server's origin: call \
					 `leptoskit::rpc::client::configure` with it when the app starts"
						.to_owned(),
				));
			}

			let headers = request
				.headers()
				.into_iter()
				.map(|(name, value)| {
					Header {
						name: name.to_owned(),
						value: value.as_bytes().to_vec(),
					}
				})
				.collect();
			let plugin = HttpClient::connect().map_err(http_failed)?;
			let response = plugin
				.execute(leptoskit_plugin_http::Request {
					method: Method::Post,
					url: request.url,
					headers,
					body: request.body.into_bytes(),
				})
				.await
				.map_err(http_failed)?;

			Ok(HttpResponse {
				status: response.status,
				body: response.body,
			})
		})
	}
}

fn http_failed(failure: leptoskit_bridge::Error) -> FnError {
	use leptoskit_bridge::Error;

	match failure {
		Error::Unavailable => {
			FnError::Other(
				"calling the server needs the HTTP plugin: add \"http\" to `[native] plugins`"
					.to_owned(),
			)
		}
		Error::PermissionDenied => {
			FnError::Other(
				"the HTTP plugin doesn't allow the server's origin: add it to \
				 `[native.plugin-options.http] origins`"
					.to_owned(),
			)
		}
		_ => FnError::Network("couldn't reach the server; try again".to_owned()),
	}
}
