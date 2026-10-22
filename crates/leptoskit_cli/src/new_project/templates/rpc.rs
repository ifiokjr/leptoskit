//! Server implementations of the shared RPC contracts.

use __CRATE___shared::Health;

/// Reports backend availability using the app's shared response model.
pub async fn health() -> axum::Json<Health> {
	axum::Json(Health { ready: true })
}
