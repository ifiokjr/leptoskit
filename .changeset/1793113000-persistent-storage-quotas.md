---
"release": patch
---

# Bound persistent plugin storage across repeated calls

Native filesystem storage defaults to 64 MiB and 4096 entries, with disk reconciliation and serialized reservations. SQLite enforces a host-selected page quota (64 MiB by default) and rolls back full-database failures. Secure-store writes share one encrypted record capped at 2560 encoded bytes and 64 entries across platforms. Hosts can configure budgets; quota failures return `TooLarge`, and deletion releases capacity. Legacy secure-store credentials remain readable and are migrated when rewritten.
