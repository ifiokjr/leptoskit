//! Server implementations of the shared RPC contracts.

use leptoskit::server::axum::Json;
use tier_shared::Health;

/// Reports backend availability using the app's shared response model.
pub async fn health() -> Json<Health> {
	Json(Health { ready: true })
}
