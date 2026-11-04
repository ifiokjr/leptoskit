//! Versioned built-in capabilities. A plugin describes app integration,
//! runtime bindings and resource ownership; it does not load executable
//! extensions into a credential-bearing deployment runner.

use leptoskit_config::infra::InfraConfig;
use serde_json::Value;
use serde_json::json;

pub(super) fn manifest(c: &InfraConfig) -> Value {
	json!({
		"format_version": 1,
		"project": c.name,
		"plugins": [
			{
				"id": "leptoskit.analytics", "version": 1,
				"enabled": c.plugins.analytics.is_some(),
				"configuration": c.plugins.analytics,
				"configuration_table": "infra.plugins.analytics",
				"application_features": ["analytics", "server"],
				"integration": "LeptosKit serve_listener_with auto-mounts from bindings; custom Axum servers mount explicitly. Browser SDK install is explicit.",
				"runtime": "embedded", "additional_containers": 0,
				"provider_resources": [], "persistent_data": false,
				"bindings": ["LEPTOSKIT_ANALYTICS", "LEPTOSKIT_ANALYTICS_SERVICE", "LEPTOSKIT_ANALYTICS_ENVIRONMENT", "LEPTOSKIT_ANALYTICS_ORIGIN", "LEPTOSKIT_ANALYTICS_WINDOWS", "LEPTOSKIT_ANALYTICS_MAX_SERIES", "LEPTOSKIT_ANALYTICS_ROUTES", "LEPTOSKIT_ANALYTICS_EXPORT_ENDPOINT", "LEPTOSKIT_ANALYTICS_EXPORT_PROTOCOL", "LEPTOSKIT_ANALYTICS_WRITE_KEY", "LEPTOSKIT_RELEASE"],
				"required_secrets": ["LEPTOSKIT_ANALYTICS_READ_KEY"],
				"read_api": "/_leptoskit/analytics",
				"durable_history": "Connected deployment service retains deduplicated fleet measurements in its existing SQLite store.",
				"remove_policy": "Disabling collection leaves shared history to expire under the deployment service's retention policy.",
				"metrics_export": "Optional OTLP/HTTP JSON delta histograms",
				"host_metrics": "Platform-dependent process, cgroup and filesystem samples; no separate host agent.",
				"limitations": ["In-process buffers reset on restart; exported history survives in the deployment service.", "Automatic DB/RPC hooks, complete trace export, native device instrumentation and Kubernetes autoscaling require separate adapters."]
			},
			{
				"id": "leptoskit.cache", "version": 1, "enabled": c.services.cache,
				"configuration_table": "infra.services.cache", "runtime": "valkey",
				"additional_containers": 1, "provider_resources": [],
				"bindings": ["VALKEY_URL"], "required_secrets": ["VALKEY_PASSWORD"],
				"persistent_data": false, "remove_policy": "Cache contents may be discarded; never store authoritative application data."
			},
			{
				"id": "leptoskit.live-relay", "version": 1, "enabled": c.services.relay,
				"configuration_table": "infra.services.relay", "runtime": "nats_jetstream",
				"application_features": ["rpc-relay"],
				"additional_containers": 1, "provider_resources": [],
				"bindings": ["rpc.live.relay_url_env"], "required_secrets": ["NATS_TOKEN"],
				"persistent_data": true, "remove_policy": "Preserve the host data directory; reconcile live state from the application database."
			}
		],
		"infrastructure_engine": "OpenTofu",
		"extension_files": ["*.tf", "*.tofu", "*.tf.json", "*.tofu.json"],
		"write_policy": "Review a new saved plan before applying provider changes. Analytics evidence does not authorize infrastructure changes."
	})
}
