---
"release": minor
---

# Release markers for database migrations

`leptoskit db release record <version>` writes a marker into `migrations/registry.txt` that freezes every migration above it as published, pinning the SHA-256 of the schema the release ends at. Editing a frozen migration's snapshot is refused the next time the migrations are read, and a marker with a typo in it never silently unfreezes a release. `leptoskit db status` names the release that froze what, and how many migrations came after it; those are the drafts the next release will consolidate.
