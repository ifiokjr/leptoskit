use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error as BridgeError;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use leptoskit_bridge::async_host::PluginFuture;
use serde::Deserialize;

use crate::Client;
use crate::Config;
use crate::Error;
use crate::protocol;

/// Scoped HTTP implementation installed by generated native registrants.
pub struct HttpPlugin {
	client: Client,
	reply_limit: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
	origins: Vec<String>,
	#[serde(default = "body_limit")]
	max_body_bytes: usize,
	#[serde(default = "header_limit")]
	max_header_bytes: usize,
	#[serde(default = "timeout")]
	timeout_ms: u64,
}
fn body_limit() -> usize {
	4 * 1024 * 1024
}
fn header_limit() -> usize {
	32 * 1024
}
fn timeout() -> u64 {
	30_000
}

/// Validates host-only JSON options before any request can run. Exact origins
/// are required. Unsupported options and encoded responses above 8 MiB fail.
pub fn from_config(options_json: &str) -> Result<HttpPlugin, BridgeError> {
	let options: Options =
		serde_json::from_str(options_json).map_err(|_| BridgeError::InvalidArgument)?;
	let mut config = Config::new(options.origins).map_err(BridgeError::from)?;
	config.max_body_bytes = options.max_body_bytes;
	config.max_header_bytes = options.max_header_bytes;
	config.timeout = Duration::from_millis(options.timeout_ms);
	let reply_limit = config
		.max_body_bytes
		.checked_add(config.max_header_bytes)
		.and_then(|n| n.checked_add(protocol::MAX_HEADERS * 8 + protocol::MAX_URL + 16))
		.filter(|n| *n <= 8 * 1024 * 1024)
		.ok_or(BridgeError::TooLarge)?;
	Ok(HttpPlugin {
		client: Client::new(config).map_err(BridgeError::from)?,
		reply_limit,
	})
}

impl AsyncPlugin for HttpPlugin {
	fn name(&self) -> &'static str {
		"http"
	}

	fn methods(&self) -> &'static [MethodId] {
		&[MethodId(0)]
	}

	fn reply_limit(&self, method: MethodId) -> Result<usize, BridgeError> {
		if method == MethodId(0) {
			Ok(self.reply_limit)
		} else {
			Err(BridgeError::Unavailable)
		}
	}

	fn invoke(
		&self,
		method: MethodId,
		payload: Bytes,
		cancellation: CancellationToken,
	) -> PluginFuture<'_> {
		Box::pin(async move {
			self.reply_limit(method)?;
			let request = protocol::decode_request(&payload)?;
			let response = tokio::select! { biased;
				() = cancellation.cancelled() => return Err(BridgeError::Cancelled),
				response = self.client.execute(request) => response.map_err(BridgeError::from)?,
			};
			Ok(protocol::encode_response(&response)?.into())
		})
	}

	fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
		Box::pin(async {})
	}
}
impl From<Error> for BridgeError {
	fn from(error: Error) -> Self {
		match error {
			Error::Invalid(_) => BridgeError::InvalidArgument,
			Error::DestinationDenied => BridgeError::PermissionDenied,
			Error::TooLarge => BridgeError::TooLarge,
			Error::Backpressure => BridgeError::Busy,
			Error::Unavailable => BridgeError::Unavailable,
			Error::Timeout | Error::Transport(_) => BridgeError::Native,
		}
	}
}
