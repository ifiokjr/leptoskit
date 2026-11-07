//! Serializable contracts and domain types shared by the app and backend.
//!
//! Each derives `JsonSchema` too, so the published RPC describes it and
//! `leptoskit rpc generate` writes it into every generated client.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// The response returned by `GET /api/health`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Health {
	/// Whether the backend is ready to receive requests.
	pub ready: bool,
}
