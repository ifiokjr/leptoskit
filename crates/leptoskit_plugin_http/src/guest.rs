use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::guest::Client as PluginHandle;

use crate::Request;
use crate::Response;
use crate::protocol;

/// Cached HTTP registration for a Wasm app hosted by `LeptosKit`.
#[derive(Debug, Clone, Copy)]
pub struct HttpClient(PluginHandle);
impl HttpClient {
	/// Resolves the host-installed scoped HTTP plugin once.
	pub fn connect() -> Result<Self, Error> {
		PluginHandle::new("http").map(Self)
	}

	/// Runs a request. Dropping this future cancels native I/O. Host origin and
	/// resource limits cannot be relaxed by the guest.
	pub async fn execute(&self, request: Request) -> Result<Response, Error> {
		let bytes = protocol::encode_request(&request)?;
		let response = self.0.call(MethodId(0), &bytes)?.await?;
		protocol::decode_response(&response)
	}
}
