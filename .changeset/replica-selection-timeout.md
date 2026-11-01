---
"leptoskit_db": patch
---

# Bound the wait for an unavailable read replica

Include connection attempts and position queries in the replica replay wait, so a stalled standby falls back to the primary within the configured budget.
