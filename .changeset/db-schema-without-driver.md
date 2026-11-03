---
"release": patch
---

# Fix database schema builds without a driver

Database schema export now compiles with the `server` feature alone, restoring CLI migration and studio workflows that inspect app tables before selecting a driver. Backend-independent change-image decoding stays available for read tracking, while SQL helpers and the Tokio dependency are enabled with database drivers.
