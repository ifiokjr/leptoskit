//! Serializable contracts and domain types shared by the app and backend.

use serde::Deserialize;
use serde::Serialize;

/// The response returned by `GET /api/health`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
	/// Whether the backend is ready to receive requests.
	pub ready: bool,
}
