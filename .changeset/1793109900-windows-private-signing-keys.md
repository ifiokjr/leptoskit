---
"release": patch
---

# Keep code push signing keys private on Windows

`leptoskit codepush keygen` now creates Windows signing keys with owner-only access before writing the secret, even inside a shared directory. It refuses to write a key when the filesystem cannot enforce those permissions.
