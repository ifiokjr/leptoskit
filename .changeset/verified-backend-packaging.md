---
"release": patch
---

# Package reproduced backends and retain image evidence

Add an opt-in backend-only image packaging command that reproduces independently pinned bundles, copies the verified executable without application compilation, checks extracted image bytes and configuration, and writes an operator-pinned descriptor. Managed releases retain this evidence across rollback and recovery. Website and service-bearing bundles remain unsupported.
