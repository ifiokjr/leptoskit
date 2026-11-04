---
"release": minor
---

# Ship a self-hosted deployment service

Add one optional service for the LeptosKit dashboard, durable deployment jobs, scoped agent API, and fleet analytics history. Reuse the existing OpenTofu and release engine, bind apply requests to reviewed plans, and retain uncertain runs for explicit reconciliation after interruption. Connect ordinary CLI commands without storing credentials in application repositories.

Add paired management database migrations, consistent backups, offline restore with token revocation, bounded retention, automatic app ingestion credentials, platform-scoped host samples and optional OTLP/HTTP metrics export. Build Git commit releases in temporary checkouts while preserving registered infra policy.
