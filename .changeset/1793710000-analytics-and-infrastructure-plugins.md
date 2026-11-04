---
"release": minor
---

# Add analytics and versioned infrastructure plugins

Add the optional `leptoskit_analytics` package and facade feature: bounded aggregate windows, W3C request correlation, Axum instrumentation, explicit backend timers, privacy-conscious browser measurements and a protected JSON evidence API. Keep browser reports separate from backend findings and make missing capacity evidence explicit.

Add `[infra.plugins.analytics]`, versioned built-in plugin manifests included in saved-plan fingerprints, managed runtime/CI bindings, release identity, `lk infra plugins` and read-only `lk infra analytics`. The embedded plugin adds no containers or cloud resources. Document the implementation and the remaining OTLP, storage, host-metrics and dashboard adapters.
