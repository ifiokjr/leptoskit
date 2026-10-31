//! The client every platform shares: the `/oauth` routes, sent through the
//! auth client so they carry this device's session.

#![cfg_attr(
	all(feature = "ssr", not(target_arch = "wasm32"), not(test)),
	allow(dead_code, reason = "server rendering reads in process")
)]

use leptoskit_auth::ErrorResponse;
use leptoskit_auth::client::HttpResponse;
use leptoskit_auth::client::Method;
use leptoskit_query::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::Api;
use super::Pending;
use crate::AppChanges;
use crate::AppRef;
use crate::ClientInfo;
use crate::ConnectedApp;
use crate::ConsentDecision;
use crate::ConsentRedirect;
use crate::ConsentRequest;
use crate::DeviceDecision;
use crate::DeviceRequest;
use crate::DisconnectApp;
use crate::NewApp;
use crate::OAuthError;
use crate::RegisteredApp;

/// Sends a request: `leptoskit_auth::client::send`, or the tests' own.
pub(crate) type Sender =
	fn(Method, String, Option<String>) -> BoxFuture<'static, Result<HttpResponse, ErrorResponse>>;

/// The client over the `/oauth` routes.
pub(crate) struct Remote {
	send: Sender,
}

impl Remote {
	pub(crate) fn new(send: Sender) -> Self {
		Self { send }
	}

	async fn get<T: DeserializeOwned>(&self, path: String) -> Result<T, ErrorResponse> {
		let response = (self.send)(Method::Get, path, None).await?;
		decode(&response)
	}

	async fn post(&self, path: &str, body: &impl Serialize) -> Result<HttpResponse, ErrorResponse> {
		(self.send)(Method::Post, path.to_owned(), Some(json(body))).await
	}
}

impl Api for Remote {
	fn consent(&self, request: String) -> Pending<'_, ConsentRequest> {
		Box::pin(async move {
			self.get(format!("/oauth/consent?request={}", encode(&request)))
				.await
		})
	}

	fn decide_consent(&self, decision: ConsentDecision) -> Pending<'_, ConsentRedirect> {
		Box::pin(async move { decode(&self.post("/oauth/consent", &decision).await?) })
	}

	fn device(&self, user_code: String) -> Pending<'_, DeviceRequest> {
		Box::pin(async move {
			self.get(format!("/oauth/device?user_code={}", encode(&user_code)))
				.await
		})
	}

	fn decide_device(&self, decision: DeviceDecision) -> Pending<'_, ()> {
		Box::pin(async move { succeeded(&self.post("/oauth/device", &decision).await?) })
	}

	fn connected_apps(&self) -> Pending<'_, Vec<ConnectedApp>> {
		Box::pin(async move { self.get("/oauth/grants".to_owned()).await })
	}

	fn disconnect(&self, client_id: String) -> Pending<'_, bool> {
		Box::pin(async move {
			let response = self
				.post("/oauth/grants/revoke", &DisconnectApp { client_id })
				.await?;
			found(&response)
		})
	}

	fn my_apps(&self) -> Pending<'_, Vec<ClientInfo>> {
		Box::pin(async move { self.get("/oauth/apps".to_owned()).await })
	}

	fn register_app(&self, app: NewApp) -> Pending<'_, RegisteredApp> {
		Box::pin(async move { decode(&self.post("/oauth/apps", &app).await?) })
	}

	fn update_app(&self, changes: AppChanges) -> Pending<'_, ClientInfo> {
		Box::pin(async move { decode(&self.post("/oauth/apps/update", &changes).await?) })
	}

	fn rotate_app_secret(&self, client_id: String) -> Pending<'_, RegisteredApp> {
		Box::pin(async move {
			decode(
				&self
					.post("/oauth/apps/rotate-secret", &AppRef { client_id })
					.await?,
			)
		})
	}

	fn delete_app(&self, client_id: String) -> Pending<'_, bool> {
		Box::pin(async move {
			found(
				&self
					.post("/oauth/apps/delete", &AppRef { client_id })
					.await?,
			)
		})
	}
}

fn json(body: &impl Serialize) -> String {
	serde_json::to_string(body).expect("request bodies are plain structs, which always serialize")
}

/// A query value, percent-encoded: everything but the unreserved characters
/// (RFC 3986 §2.3).
fn encode(value: &str) -> String {
	use std::fmt::Write;

	let mut encoded = String::with_capacity(value.len());
	for byte in value.bytes() {
		if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
			encoded.push(char::from(byte));
		} else {
			let _ = write!(encoded, "%{byte:02X}");
		}
	}
	encoded
}

pub(crate) fn error(code: &str, message: &str) -> ErrorResponse {
	ErrorResponse {
		error: code.to_owned(),
		message: message.to_owned(),
		retry_after: None,
	}
}

/// A successful answer's body, or the error the server sent instead.
fn decode<T: DeserializeOwned>(response: &HttpResponse) -> Result<T, ErrorResponse> {
	succeeded(response)?;
	serde_json::from_slice(&response.body)
		.map_err(|_| error("server", "the server's answer didn't make sense; try again"))
}

/// Whether a change found what it changed: `204`, or `404` for nothing.
fn found(response: &HttpResponse) -> Result<bool, ErrorResponse> {
	if response.status == 404 && response.body.is_empty() {
		return Ok(false);
	}
	succeeded(response).map(|()| true)
}

/// The error the server sent, if it didn't succeed: the provider's own
/// code, with its description as the message.
fn succeeded(response: &HttpResponse) -> Result<(), ErrorResponse> {
	if (200..300).contains(&response.status) {
		return Ok(());
	}

	Err(
		serde_json::from_slice::<OAuthError>(&response.body).map_or_else(
			|_| error("server", "something went wrong; try again"),
			|answer| {
				let message = answer
					.error_description
					.unwrap_or_else(|| "something went wrong; try again".to_owned());
				error(&answer.error, &message)
			},
		),
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn query_values_are_percent_encoded() {
		assert_eq!(encode("WDJB-MJHT"), "WDJB-MJHT");
		assert_eq!(encode("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
	}

	#[test]
	fn answers_become_values_or_errors() {
		let answer = |status: u16, body: &str| {
			HttpResponse {
				status,
				body: body.as_bytes().to_vec(),
			}
		};

		assert_eq!(found(&answer(204, "")), Ok(true));
		assert_eq!(found(&answer(404, "")), Ok(false));
		let error = found(&answer(
			404,
			r#"{"error":"not_found","error_description":"that no longer exists"}"#,
		))
		.unwrap_err();
		assert_eq!(error.error, "not_found");
		assert_eq!(error.message, "that no longer exists");
		assert_eq!(
			succeeded(&answer(401, r#"{"error":"login_required"}"#))
				.unwrap_err()
				.error,
			"login_required"
		);
		assert_eq!(succeeded(&answer(500, "oops")).unwrap_err().error, "server");
		assert_eq!(
			decode::<Vec<ConnectedApp>>(&answer(200, "not json"))
				.unwrap_err()
				.error,
			"server"
		);
	}
}
